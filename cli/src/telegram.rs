//! The phone is a remote (PLAN P8.0h): a person's own Telegram bot,
//! long-polled by their wallet running as a service. Nothing but Telegram is
//! reached, nothing listens, and no key ever leaves the machine the wallet
//! is on. One Telegram user is paired; everyone else is ignored.

use crate::serve::WalletService;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// `<folder>/<name>.wallet/telegram.json`: the bot token, who is paired, the
/// PIN's hash and the daily limit. Owner-only on disk. The token is a bot's
/// credential, not money: with it someone can talk as the bot, not spend —
/// spending needs the PIN as well, and the wallet's password to start.
#[derive(Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub token: String,
    pub user_id: Option<i64>,
    pub pairing_code: Option<String>,
    pub pin_salt: String,
    pub pin_hash: String,
    pub daily_limit_petals: u64,
    /// The chat the pairing happened in; the bot answers no other, so a
    /// command typed in a group cannot post a bearer code there.
    #[serde(default)]
    pub chat_id: Option<i64>,
    /// The chat 'backup telegram' posts to — a private group the bot is a
    /// member of, given once and kept.
    #[serde(default)]
    pub backup_chat_id: Option<i64>,
    /// "argon2" for a PIN hashed with Argon2id; absent for the first
    /// wallets' plain SHA-256, kept verifiable until the PIN is set again.
    #[serde(default)]
    pub pin_kdf: Option<String>,
    /// Unix seconds the pairing code was made; a code older than
    /// [`PAIRING_CODE_LIFETIME`] is dead.
    #[serde(default)]
    pub pairing_made: u64,
    /// Wrong pairing codes seen; five of them kill the code.
    #[serde(default)]
    pub pairing_failures: u32,
    /// Three wrong PINs. Kept on disk, so a restart does not unlock.
    #[serde(default)]
    pub locked: bool,
    /// The day (UTC) and the petals spent in it, kept on disk so a restart
    /// does not reset the daily limit.
    #[serde(default)]
    pub spent_day: u64,
    #[serde(default)]
    pub spent_petals: u64,
}

/// How long a pairing code stays valid.
pub const PAIRING_CODE_LIFETIME: Duration = Duration::from_secs(15 * 60);
/// Wrong pairing codes tolerated before the code is thrown away.
pub const PAIRING_FAILURES_ALLOWED: u32 = 5;

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub const DEFAULT_DAILY_LIMIT_PETALS: u64 = 100 * 100_000_000;

impl TelegramConfig {
    pub fn path(folder: &str, wallet_name: &str) -> PathBuf {
        let base = workflow_store::fs::resolve_path(folder).unwrap_or_else(|_| PathBuf::from(folder));
        base.join(kaspa_wallet_core::storage::local::wallet_dir_name(wallet_name)).join("telegram.json")
    }

    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        // Created owner-only from the first byte, beside the real file and
        // moved into place: no moment at the process umask, no half a file.
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&tmp)?.write_all(text.as_bytes())?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn new(token: String, pin: &str, daily_limit_petals: u64) -> Self {
        let salt: [u8; 16] = rand::random();
        let mut cfg = Self {
            token,
            user_id: None,
            pairing_code: None,
            pin_salt: hex::encode(salt),
            pin_hash: String::new(),
            daily_limit_petals,
            chat_id: None,
            backup_chat_id: None,
            pin_kdf: Some("argon2".to_string()),
            pairing_made: 0,
            pairing_failures: 0,
            locked: false,
            spent_day: 0,
            spent_petals: 0,
        };
        cfg.pin_hash = cfg.hash_pin(pin);
        cfg.new_pairing_code();
        cfg
    }

    /// A fresh six-digit pairing code, drawn uniformly, dated.
    pub fn new_pairing_code(&mut self) {
        use rand::Rng;
        let code: u32 = rand::thread_rng().gen_range(0..1_000_000);
        self.pairing_code = Some(format!("{code:06}"));
        self.pairing_made = now_unix();
        self.pairing_failures = 0;
    }

    /// Whether the pairing code can still be used.
    pub fn pairing_code_live(&self) -> bool {
        self.pairing_code.is_some() && now_unix().saturating_sub(self.pairing_made) < PAIRING_CODE_LIFETIME.as_secs()
    }

    /// Set (or reset) the PIN, hashed with Argon2id under a fresh salt.
    pub fn set_pin(&mut self, pin: &str) {
        let salt: [u8; 16] = rand::random();
        self.pin_salt = hex::encode(salt);
        self.pin_kdf = Some("argon2".to_string());
        self.pin_hash = self.hash_pin(pin);
    }

    /// The PIN's hash. Argon2id (memory-hard, so a four-digit PIN behind a
    /// leaked file is not instant) for every PIN set since 2026-09-20; the
    /// plain salted SHA-256 the first wallets used stays verifiable so
    /// nobody is locked out by the upgrade.
    fn hash_pin(&self, pin: &str) -> String {
        if self.pin_kdf.as_deref() == Some("argon2") {
            let salt = hex::decode(&self.pin_salt).unwrap_or_default();
            return kaspa_wallet_core::encryption::argon2_hash_with_salt(pin.trim().as_bytes(), &salt, 32)
                .map(|key| hex::encode(key.as_ref()))
                .unwrap_or_default();
        }
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(self.pin_salt.as_bytes());
        h.update(b":");
        h.update(pin.trim().as_bytes());
        hex::encode(h.finalize())
    }

    pub fn pin_matches(&self, pin: &str) -> bool {
        // Same length, compared in full: a hex digest is not a secret worth
        // a timing attack, but equal work is free.
        let candidate = self.hash_pin(pin);
        candidate.len() == self.pin_hash.len()
            && candidate.bytes().zip(self.pin_hash.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }
}

// --- The Bot API, as GETs ----------------------------------------------------

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

async fn call(token: &str, method: &str, params: &[(&str, String)]) -> Result<serde_json::Value, String> {
    let query = params.iter().map(|(k, v)| format!("{k}={}", url_encode(v))).collect::<Vec<_>>().join("&");
    let url = format!("https://api.telegram.org/bot{token}/{method}?{query}");
    // The token is in the URL, and a transport error may quote the URL:
    // it must not reach the log.
    let value: serde_json::Value =
        workflow_http::get_json(url).await.map_err(|e| format!("{method}: {}", e.to_string().replace(token, "<token>")))?;
    if value.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!("{method}: {}", value.get("description").and_then(|d| d.as_str()).unwrap_or("not ok")));
    }
    Ok(value)
}

/// A PIN must not stay on the screen. Bots may delete a person's own message
/// in a private chat; if that fails the log says so and nothing else changes.
async fn delete(token: &str, chat_id: i64, message_id: i64) {
    let params = [("chat_id", chat_id.to_string()), ("message_id", message_id.to_string())];
    if let Err(e) = call(token, "deleteMessage", &params).await {
        log::warn!("telegram: could not delete the PIN message: {e}");
    }
}

/// The code as a QR picture, under the message that carries it as text.
/// Telegram takes the file as a multipart upload; a failure is logged and
/// the text, already sent, stands on its own.
async fn send_qr(token: &str, chat_id: i64, code: &str, caption: &str) {
    let Some(png) = crate::qrpng::qr_png(code) else { return };
    let part = match reqwest::multipart::Part::bytes(png).file_name("code.png").mime_str("image/png") {
        Ok(part) => part,
        Err(_) => return,
    };
    let form =
        reqwest::multipart::Form::new().text("chat_id", chat_id.to_string()).text("caption", caption.to_string()).part("photo", part);
    let url = format!("https://api.telegram.org/bot{token}/sendPhoto");
    match reqwest::Client::new().post(url).multipart(form).send().await {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => log::warn!("telegram: sendPhoto answered {}", resp.status()),
        Err(e) => log::warn!("telegram: sendPhoto: {e}"),
    }
}

/// Every plain reply carries the buttons: Telegram shows a reply keyboard
/// only with a message that brings it, and a person who never typed /help
/// would otherwise never see them.
async fn send(token: &str, chat_id: i64, html: &str) {
    let params = [
        ("chat_id", chat_id.to_string()),
        ("text", html.to_string()),
        ("parse_mode", "HTML".to_string()),
        ("reply_markup", main_keyboard()),
    ];
    if let Err(e) = call(token, "sendMessage", &params).await {
        log::warn!("telegram: could not send to {chat_id}: {e}");
    }
}

/// The scanner page: a Mini App that opens the phone's camera, reads a
/// Marigold code and hands it back to the bot. Opened from a keyboard
/// button, which is the one launch that can send data straight back.
pub const SCAN_URL: &str = "https://marigold.cash/app/scan/";

/// What the bot is waiting for from the paired user.
enum Pending {
    Nothing,
    /// Digits for an amount, on the keypad message `message_id`.
    Amount {
        purpose: Purpose,
        buf: String,
        message_id: i64,
    },
    /// PIN digits, on the keypad message `message_id`, to pay `petals` — to a
    /// share key under a lock when `key` is set, as a bearer code otherwise.
    Pin {
        petals: u64,
        buf: String,
        message_id: i64,
        key: Option<String>,
    },
}

/// A locked payment from the phone lasts this long: three days, the same
/// default the terminal uses.
const LOCK_SECONDS: u64 = 3 * 86_400;

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    Pay,
    Request,
}

const HELP: &str = "<b>Your wallet</b>\n\
Tap a button below, or type:\n\
/balance — what you hold\n\
/pay &lt;amount&gt; — a code to hand to someone (asks your PIN)\n\
/pay &lt;request code&gt; — pay a request you were given; pasting or scanning it works too\n\
/receive &lt;code&gt; — take a code you were given; pasting or scanning a code works too\n\
/request [amount] — a code for someone to pay you\n\
/history — the last payments\n\
/status — the network, the miner\n\
/mine start|stop|status — the miner, if this wallet mines\n\
/key [name] — a key of yours for someone to pay to; /pay &lt;amount&gt; &lt;key&gt; pays to one, locked three days\n\
/cancel — forget what was being asked";

/// The buttons under the message field: the everyday verbs, and the camera.
fn main_keyboard() -> String {
    serde_json::json!({
        "keyboard": [
            [{"text": "Balance"}, {"text": "Pay"}],
            [{"text": "Receive"}, {"text": "Request"}],
            [{"text": "History"}, {"text": "Status"}],
            [{"text": "My key"}],
            [{"text": "📷 Scan a code", "web_app": {"url": SCAN_URL}}]
        ],
        "resize_keyboard": true,
        "is_persistent": true
    })
    .to_string()
}

/// A keypad on the message itself. Every tap comes back as a callback,
/// never as a message in the chat, which is what a PIN wants.
fn keypad(with_dot: bool) -> String {
    let row = |keys: &[&str]| -> Vec<serde_json::Value> {
        keys.iter().map(|k| serde_json::json!({"text": k, "callback_data": format!("k:{k}")})).collect()
    };
    let fourth = if with_dot { row(&[".", "0", "⌫"]) } else { row(&["0", "⌫"]) };
    serde_json::json!({ "inline_keyboard": [row(&["1", "2", "3"]), row(&["4", "5", "6"]), row(&["7", "8", "9"]), fourth, row(&["Cancel", "OK"])] }).to_string()
}

async fn send_with_keyboard(token: &str, chat_id: i64, html: &str, markup: &str) -> Option<i64> {
    let params = [
        ("chat_id", chat_id.to_string()),
        ("text", html.to_string()),
        ("parse_mode", "HTML".to_string()),
        ("reply_markup", markup.to_string()),
    ];
    match call(token, "sendMessage", &params).await {
        Ok(v) => v.get("result").and_then(|r| r.get("message_id")).and_then(|m| m.as_i64()),
        Err(e) => {
            log::warn!("telegram: could not send to {chat_id}: {e}");
            None
        }
    }
}

async fn edit(token: &str, chat_id: i64, message_id: i64, html: &str, markup: Option<&str>) {
    let mut params = vec![
        ("chat_id", chat_id.to_string()),
        ("message_id", message_id.to_string()),
        ("text", html.to_string()),
        ("parse_mode", "HTML".to_string()),
    ];
    if let Some(markup) = markup {
        params.push(("reply_markup", markup.to_string()));
    }
    if let Err(e) = call(token, "editMessageText", &params).await {
        // Editing to the same text is refused; harmless.
        if !e.contains("not modified") {
            log::warn!("telegram: edit: {e}");
        }
    }
}

async fn answer_callback(token: &str, id: &str) {
    let _ = call(token, "answerCallbackQuery", &[("callback_query_id", id.to_string())]).await;
}

/// The commands menu next to the message field is only there once the bot
/// has told Telegram its commands.
async fn register_commands(token: &str) {
    let commands = serde_json::json!([
        {"command": "balance", "description": "What you hold"},
        {"command": "pay", "description": "A code to hand to someone"},
        {"command": "receive", "description": "Take a code you were given"},
        {"command": "request", "description": "A code for someone to pay you"},
        {"command": "history", "description": "The last payments"},
        {"command": "status", "description": "The network and the miner"},
        {"command": "help", "description": "The buttons and the commands"},
        {"command": "cancel", "description": "Forget what was being asked"}
    ])
    .to_string();
    if let Err(e) = call(token, "setMyCommands", &[("commands", commands)]).await {
        log::warn!("telegram: setMyCommands: {e}");
    }
}

fn amount_line(purpose: Purpose, buf: &str, ticker: &str) -> String {
    let what = match purpose {
        Purpose::Pay => "Pay how much?",
        Purpose::Request => "Request how much? (OK with nothing: the payer chooses)",
    };
    format!("{what}\n<b>{} {ticker}</b>", if buf.is_empty() { "_" } else { buf })
}

fn pin_line(petals: u64, buf: &str, ticker: &str) -> String {
    format!(
        "Pay {} {ticker} as a code.\nPIN: <b>{}</b>",
        sompi_to_kaspa_string(petals),
        if buf.is_empty() { "_".to_string() } else { "•".repeat(buf.len()) }
    )
}

fn pin_line_request(petals: u64, buf: &str, ticker: &str) -> String {
    format!(
        "Pay {} {ticker} to this request.\nPIN: <b>{}</b>",
        sompi_to_kaspa_string(petals),
        if buf.is_empty() { "_".to_string() } else { "•".repeat(buf.len()) }
    )
}

fn pin_line_locked(petals: u64, buf: &str, ticker: &str) -> String {
    format!(
        "Pay {} {ticker} to their key, locked three days.\nPIN: <b>{}</b>",
        sompi_to_kaspa_string(petals),
        if buf.is_empty() { "_".to_string() } else { "•".repeat(buf.len()) }
    )
}

/// Long-poll the bot and act for the paired user. Runs until the service
/// stops; a Telegram hiccup is logged and retried, never fatal.
pub async fn run_bot(service: Arc<WalletService>, cfg_path: PathBuf, mut cfg: TelegramConfig) {
    let token = cfg.token.clone();
    let mut offset: i64 = 0;
    let mut pending = Pending::Nothing;
    let mut pin_failures = 0u32;
    let mut locked = cfg.locked;
    service.seed_spent(cfg.spent_day, cfg.spent_petals);
    log::info!(
        "telegram: bot running; {}",
        match cfg.user_id {
            Some(id) => format!("paired with user {id}"),
            None => "not paired yet — send /start <code> to the bot".to_string(),
        }
    );
    register_commands(&token).await;
    let ticker = service.ticker();
    loop {
        let params = [
            ("offset", offset.to_string()),
            ("timeout", "25".to_string()),
            ("allowed_updates", "[\"message\",\"callback_query\"]".to_string()),
        ];
        let updates = match call(&token, "getUpdates", &params).await {
            Ok(v) => v,
            Err(e) => {
                // A bad token is a configuration problem, not a hiccup: say
                // so, and do not hammer Telegram about it.
                if e.contains("401") {
                    log::warn!(
                        "telegram: the bot token is refused (401) — check 'mobile telegram' in the wallet; trying again in a minute"
                    );
                    tokio::time::sleep(Duration::from_secs(60)).await;
                } else {
                    log::warn!("telegram: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                continue;
            }
        };
        let Some(list) = updates.get("result").and_then(|r| r.as_array()) else { continue };
        for update in list {
            if let Some(id) = update.get("update_id").and_then(|v| v.as_i64()) {
                offset = offset.max(id + 1);
            }

            // --- Keypad taps -------------------------------------------------
            if let Some(cb) = update.get("callback_query") {
                let cb_id = cb.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                answer_callback(&token, &cb_id).await;
                let from = cb.get("from").and_then(|f| f.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
                let Some(msg) = cb.get("message") else { continue };
                let chat_id = msg.get("chat").and_then(|c| c.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
                let message_id = msg.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0);
                let key = cb.get("data").and_then(|d| d.as_str()).and_then(|d| d.strip_prefix("k:")).unwrap_or("").to_string();
                if cfg.user_id != Some(from) || locked || cfg.chat_id.is_some_and(|paired| paired != chat_id) {
                    continue;
                }
                match &mut pending {
                    Pending::Amount { purpose, buf, message_id: mid } if *mid == message_id => {
                        let purpose = *purpose;
                        match key.as_str() {
                            "Cancel" => {
                                pending = Pending::Nothing;
                                edit(&token, chat_id, message_id, "Cancelled.", None).await;
                            }
                            "OK" => {
                                let text = buf.clone();
                                let petals = crate::utils::try_parse_required_nonzero_kaspa_as_sompi_u64(Some(&text)).ok();
                                match (purpose, petals) {
                                    (Purpose::Pay, Some(petals)) => match service.spend_allowed(petals, cfg.daily_limit_petals) {
                                        Err(why) => {
                                            pending = Pending::Nothing;
                                            edit(&token, chat_id, message_id, &html_escape(&why), None).await;
                                        }
                                        Ok(()) => {
                                            pending = Pending::Pin { petals, buf: String::new(), message_id, key: None };
                                            edit(&token, chat_id, message_id, &pin_line(petals, "", ticker), Some(&keypad(false)))
                                                .await;
                                        }
                                    },
                                    (Purpose::Pay, None) => {
                                        edit(&token, chat_id, message_id, &amount_line(purpose, buf, ticker), Some(&keypad(true)))
                                            .await
                                    }
                                    (Purpose::Request, petals) => {
                                        pending = Pending::Nothing;
                                        match service.request(petals).await {
                                            Ok(code) => {
                                                let what = match petals {
                                                    Some(p) => format!("for {} {ticker}", sompi_to_kaspa_string(p)),
                                                    None => "for whatever the payer chooses".to_string(),
                                                };
                                                edit(&token, chat_id, message_id, &format!("A request {what}. Give them this code; they type <b>pay</b> and the code. I will say when it is paid.\n\n<code>{}</code>", html_escape(&code)), None).await;
                                                send_qr(&token, chat_id, &code, "The same request, to scan").await;
                                                let (service, token, sent_code) = (service.clone(), token.clone(), code);
                                                tokio::spawn(async move {
                                                    match service.await_request(&sent_code, Duration::from_secs(3600)).await {
                                                        Ok(Some(line)) => send(&token, chat_id, &html_escape(&line)).await,
                                                        Ok(None) => {}
                                                        Err(e) => log::warn!("telegram: request watch: {e}"),
                                                    }
                                                });
                                            }
                                            Err(e) => {
                                                edit(
                                                    &token,
                                                    chat_id,
                                                    message_id,
                                                    &format!("Could not make a request: {}", html_escape(&e)),
                                                    None,
                                                )
                                                .await
                                            }
                                        }
                                    }
                                }
                            }
                            "⌫" => {
                                buf.pop();
                                let line = amount_line(purpose, buf, ticker);
                                edit(&token, chat_id, message_id, &line, Some(&keypad(true))).await;
                            }
                            k if k.len() == 1 && (k.chars().all(|c| c.is_ascii_digit()) || (k == "." && !buf.contains('.'))) => {
                                if buf.len() < 12 {
                                    buf.push_str(k);
                                }
                                let line = amount_line(purpose, buf, ticker);
                                edit(&token, chat_id, message_id, &line, Some(&keypad(true))).await;
                            }
                            _ => {}
                        }
                    }
                    Pending::Pin { petals, buf, message_id: mid, key: share } if *mid == message_id => {
                        let petals = *petals;
                        let share_key = share.clone();
                        match key.as_str() {
                            "Cancel" => {
                                pending = Pending::Nothing;
                                edit(&token, chat_id, message_id, "Cancelled.", None).await;
                            }
                            "OK" => {
                                let pin = buf.clone();
                                pending = Pending::Nothing;
                                if !cfg.pin_matches(&pin) {
                                    pin_failures += 1;
                                    if pin_failures >= 3 {
                                        locked = true;
                                        cfg.locked = true;
                                        if let Err(e) = cfg.save(&cfg_path) {
                                            log::error!("telegram: could not save the lock: {e}");
                                        }
                                        log::warn!("telegram: locked after three wrong PINs");
                                        edit(
                                            &token,
                                            chat_id,
                                            message_id,
                                            "Wrong PIN, three times. Locked; 'mobile telegram unlock' in the wallet clears it.",
                                            None,
                                        )
                                        .await;
                                    } else {
                                        edit(&token, chat_id, message_id, "Wrong PIN. Tap Pay to try again.", None).await;
                                    }
                                    continue;
                                }
                                pin_failures = 0;
                                edit(&token, chat_id, message_id, "Paying…", None).await;
                                let is_request = share_key.as_deref().is_some_and(|k| k.starts_with("marigoldreq:"));
                                let outcome = match &share_key {
                                    Some(code) if is_request => service.pay_request(code).await,
                                    Some(key) => service.pay_locked(petals, key, LOCK_SECONDS).await,
                                    None => service.pay(petals).await,
                                };
                                match outcome {
                                    Ok(paid) if is_request => {
                                        service.note_spent(petals + paid.fee_petals);
                                        {
                                            let (day, spent) = service.spent_today();
                                            cfg.spent_day = day;
                                            cfg.spent_petals = spent;
                                            if let Err(e) = cfg.save(&cfg_path) {
                                                log::error!("telegram: could not save the day's total: {e}");
                                            }
                                        }
                                        service.say(format!(
                                            "telegram: paid a request of {} {ticker}",
                                            sompi_to_kaspa_string(paid.value_petals)
                                        ));
                                        edit(
                                            &token,
                                            chat_id,
                                            message_id,
                                            &format!(
                                                "Paid {} {ticker} to the request (fee {}). Your receipt, for whoever asked — it points them at the payment and holds nothing secret:\n<code>{}</code>",
                                                sompi_to_kaspa_string(paid.value_petals),
                                                sompi_to_kaspa_string(paid.fee_petals),
                                                html_escape(&paid.code)
                                            ),
                                            None,
                                        )
                                        .await;
                                        send_qr(&token, chat_id, &paid.code, "The receipt, to scan").await;
                                    }
                                    Ok(paid) if share_key.is_some() => {
                                        service.note_spent(petals);
                                        {
                                            let (day, spent) = service.spent_today();
                                            cfg.spent_day = day;
                                            cfg.spent_petals = spent;
                                            if let Err(e) = cfg.save(&cfg_path) {
                                                log::error!("telegram: could not save the day's total: {e}");
                                            }
                                        }
                                        service.say(format!(
                                            "telegram: offered {} {ticker} to a key, locked three days",
                                            sompi_to_kaspa_string(paid.value_petals)
                                        ));
                                        edit(
                                            &token,
                                            chat_id,
                                            message_id,
                                            &format!(
                                                "{} {ticker} offered to their key for three days, plus a 0.01 stamp (fee {}). Give them this code; they type <b>receive</b> and the code, or scan the picture. Only their key can take it; if they have not by then, it comes back to you.\n\n<code>{}</code>",
                                                sompi_to_kaspa_string(paid.value_petals),
                                                sompi_to_kaspa_string(paid.fee_petals),
                                                html_escape(&paid.code)
                                            ),
                                            None,
                                        )
                                        .await;
                                        send_qr(&token, chat_id, &paid.code, "The same code, to scan").await;
                                    }
                                    Ok(paid) => {
                                        service.note_spent(petals);
                                        {
                                            let (day, spent) = service.spent_today();
                                            cfg.spent_day = day;
                                            cfg.spent_petals = spent;
                                            if let Err(e) = cfg.save(&cfg_path) {
                                                log::error!("telegram: could not save the day's total: {e}");
                                            }
                                        }
                                        service.say(format!(
                                            "telegram: paid {} {ticker} as a code",
                                            sompi_to_kaspa_string(paid.value_petals)
                                        ));
                                        edit(
                                            &token,
                                            chat_id,
                                            message_id,
                                            &format!(
                                                "{} {ticker} in {} note{}, plus a 0.01 stamp for the receiver (fee {}). Give them this code; they type <b>receive</b> and the code, or scan the picture. Anyone who sees it can take the money.\n\n<code>{}</code>",
                                                sompi_to_kaspa_string(paid.value_petals),
                                                paid.notes,
                                                if paid.notes == 1 { "" } else { "s" },
                                                sompi_to_kaspa_string(paid.fee_petals),
                                                html_escape(&paid.code)
                                            ),
                                            None,
                                        )
                                        .await;
                                        send_qr(&token, chat_id, &paid.code, "The same code, to scan").await;
                                    }
                                    Err(e) => {
                                        edit(&token, chat_id, message_id, &format!("Could not pay: {}", html_escape(&e)), None).await
                                    }
                                }
                            }
                            "⌫" => {
                                buf.pop();
                                let line = match share_key.as_deref() {
                                    Some(k) if k.starts_with("marigoldreq:") => pin_line_request(petals, buf, ticker),
                                    Some(_) => pin_line_locked(petals, buf, ticker),
                                    None => pin_line(petals, buf, ticker),
                                };
                                edit(&token, chat_id, message_id, &line, Some(&keypad(false))).await;
                            }
                            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => {
                                if buf.len() < 12 {
                                    buf.push_str(k);
                                }
                                let line = match share_key.as_deref() {
                                    Some(k) if k.starts_with("marigoldreq:") => pin_line_request(petals, buf, ticker),
                                    Some(_) => pin_line_locked(petals, buf, ticker),
                                    None => pin_line(petals, buf, ticker),
                                };
                                edit(&token, chat_id, message_id, &line, Some(&keypad(false))).await;
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        // A keypad from an earlier question: dead now.
                        edit(&token, chat_id, message_id, "That question is over. Tap a button below.", None).await;
                    }
                }
                continue;
            }

            // --- Messages ------------------------------------------------------
            let Some(message) = update.get("message") else { continue };
            let Some(from) = message.get("from").and_then(|f| f.get("id")).and_then(|v| v.as_i64()) else { continue };
            let Some(chat_id) = message.get("chat").and_then(|c| c.get("id")).and_then(|v| v.as_i64()) else { continue };
            let message_id = message.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0);
            // A code from the scanner page arrives as web_app_data.
            let scanned =
                message.get("web_app_data").and_then(|w| w.get("data")).and_then(|d| d.as_str()).map(|s| s.trim().to_string());
            let text =
                scanned.clone().unwrap_or_else(|| message.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string());

            // Pairing: the one moment an unpaired user is listened to. Six
            // digits are guessable by anyone who finds the bot, so the code
            // lives fifteen minutes, dies after five wrong tries, and a miss
            // is answered the same way as anything else.
            if cfg.user_id.is_none() {
                let code = text.strip_prefix("/start").map(str::trim).unwrap_or("");
                if !code.is_empty() && cfg.pairing_code_live() && cfg.pairing_code.as_deref() == Some(code) {
                    cfg.user_id = Some(from);
                    cfg.chat_id = Some(chat_id);
                    cfg.pairing_code = None;
                    if let Err(e) = cfg.save(&cfg_path) {
                        log::error!("telegram: could not save the pairing: {e}");
                    }
                    service.say(format!("telegram: paired with user {from}"));
                    send_with_keyboard(
                        &token,
                        chat_id,
                        &format!("Paired. This chat now moves money in your wallet: keep 2FA on your Telegram account.\n\n{HELP}"),
                        &main_keyboard(),
                    )
                    .await;
                } else {
                    if !code.is_empty() && cfg.pairing_code.is_some() {
                        cfg.pairing_failures += 1;
                        if cfg.pairing_failures >= PAIRING_FAILURES_ALLOWED {
                            cfg.pairing_code = None;
                            service.say("telegram: five wrong pairing codes — the code is dead; 'mobile telegram' in the wallet makes a new one".to_string());
                        }
                        if let Err(e) = cfg.save(&cfg_path) {
                            log::error!("telegram: could not save: {e}");
                        }
                    }
                    send(&token, chat_id, "This bot answers its owner. If that is you: in your wallet, 'mobile telegram' shows a code; send it here as /start &lt;code&gt;.").await;
                }
                continue;
            }
            if cfg.user_id != Some(from) {
                log::warn!("telegram: ignored a message from user {from}, not the paired one");
                continue;
            }
            if cfg.chat_id.is_some_and(|paired| paired != chat_id) {
                log::warn!("telegram: ignored a message from chat {chat_id}, not the paired chat");
                continue;
            }
            if locked {
                send(&token, chat_id, "Locked after three wrong PINs. In your wallet, 'mobile telegram unlock' clears it.").await;
                continue;
            }

            // A typed PIN, if the old way is used while a keypad is up.
            if let Pending::Pin { .. } = pending {
                delete(&token, chat_id, message_id).await;
                send(&token, chat_id, "Use the keypad on the message above, or /cancel.").await;
                continue;
            }

            // Codes pasted or scanned are received, every one of them; a
            // request code pays it.
            let codes: Vec<&str> = text
                .split_whitespace()
                .filter(|w| w.starts_with("marigoldpay:") || w.starts_with("marigoldpay2:") || w.starts_with("marigoldnote:"))
                .collect();
            if !text.starts_with('/') && !codes.is_empty() {
                for code in codes {
                    let reply = match service.receive(code).await {
                        Ok(line) => {
                            service.say(format!("telegram: {line}"));
                            line
                        }
                        Err(e) => format!("Could not receive one code: {e}"),
                    };
                    send(&token, chat_id, &html_escape(&reply)).await;
                }
                continue;
            }
            // A pasted or scanned request code is '/pay' with that code.
            let text = if text.starts_with("marigoldreq:") { format!("/pay {text}") } else { text };

            // Buttons say words; commands say slashes. Both land here.
            let lowered = text.to_lowercase();
            let mut words = text.split_whitespace();
            let first = words.next().unwrap_or("").split('@').next().unwrap_or("").to_lowercase();
            let rest: Vec<&str> = words.collect();
            let command = match first.as_str() {
                "/balance" | "balance" => "balance",
                "/pay" | "pay" => "pay",
                "/receive" | "receive" => "receive",
                "/request" | "request" => "request",
                "/history" | "history" => "history",
                "/status" | "status" => "status",
                "/mine" | "mine" => "mine",
                "/cancel" | "cancel" => "cancel",
                "/key" | "my key" | "key" => "key",
                "/start" | "/help" | "help" => "help",
                _ if lowered.starts_with("📷") => "scan",
                _ => "unknown",
            };
            match command {
                "help" => {
                    send_with_keyboard(&token, chat_id, HELP, &main_keyboard()).await;
                }
                "balance" => send(&token, chat_id, &html_escape(&service.balance_text().await)).await,
                "status" => send(&token, chat_id, &html_escape(&service.status_text().await)).await,
                "history" => {
                    let n = rest.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(10);
                    send(&token, chat_id, &format!("<pre>{}</pre>", html_escape(&service.history_text(n)))).await
                }
                // '/pay marigoldreq:…': the request says the amount; the PIN
                // confirms it. It used to fall through to the amount keypad,
                // which read as the bot not knowing what it was handed
                // (tester, 2026-09-21).
                "pay" if rest.first().is_some_and(|c| c.starts_with("marigoldreq:")) => {
                    let code = rest[0].to_string();
                    match WalletService::request_amount(&code) {
                        Err(why) => send(&token, chat_id, &html_escape(&why)).await,
                        Ok(petals) => match service.spend_allowed(petals, cfg.daily_limit_petals) {
                            Err(why) => send(&token, chat_id, &html_escape(&why)).await,
                            Ok(()) => {
                                if let Some(mid) =
                                    send_with_keyboard(&token, chat_id, &pin_line_request(petals, "", ticker), &keypad(false)).await
                                {
                                    pending = Pending::Pin { petals, buf: String::new(), message_id: mid, key: Some(code) };
                                }
                            }
                        },
                    }
                }
                "pay" => match rest.first().and_then(|s| crate::utils::try_parse_required_nonzero_kaspa_as_sompi_u64(Some(s)).ok()) {
                    // '/pay 5' typed: straight to the PIN keypad; '/pay 5 marigoldkey:…'
                    // pays to that key under a lock.
                    Some(petals) => match service.spend_allowed(petals, cfg.daily_limit_petals) {
                        Err(why) => send(&token, chat_id, &html_escape(&why)).await,
                        Ok(()) => {
                            let key = rest.get(1).filter(|k| WalletService::is_share_key(k)).map(|k| k.to_string());
                            let line = if key.is_some() { pin_line_locked(petals, "", ticker) } else { pin_line(petals, "", ticker) };
                            if let Some(mid) = send_with_keyboard(&token, chat_id, &line, &keypad(false)).await {
                                pending = Pending::Pin { petals, buf: String::new(), message_id: mid, key };
                            }
                        }
                    },
                    // The button, or a bare '/pay': the amount keypad first.
                    None => {
                        if let Some(mid) =
                            send_with_keyboard(&token, chat_id, &amount_line(Purpose::Pay, "", ticker), &keypad(true)).await
                        {
                            pending = Pending::Amount { purpose: Purpose::Pay, buf: String::new(), message_id: mid };
                        }
                    }
                },
                "receive" => match rest.first() {
                    Some(code) => {
                        let reply = match service.receive(code).await {
                            Ok(line) => {
                                service.say(format!("telegram: {line}"));
                                line
                            }
                            Err(e) => format!("Could not receive: {e}"),
                        };
                        send(&token, chat_id, &html_escape(&reply)).await
                    }
                    None => {
                        send(
                            &token,
                            chat_id,
                            "Paste the code you were given here, or tap <b>📷 Scan a code</b> below to read it with the camera.",
                        )
                        .await
                    }
                },
                "request" => {
                    match rest.first().and_then(|s| crate::utils::try_parse_required_nonzero_kaspa_as_sompi_u64(Some(s)).ok()) {
                        Some(_) | None if !rest.is_empty() => {
                            let petals =
                                rest.first().and_then(|s| crate::utils::try_parse_required_nonzero_kaspa_as_sompi_u64(Some(s)).ok());
                            match service.request(petals).await {
                                Ok(code) => {
                                    let what = match petals {
                                        Some(p) => format!("for {} {ticker}", sompi_to_kaspa_string(p)),
                                        None => "for whatever the payer chooses".to_string(),
                                    };
                                    send(&token, chat_id, &format!("A request {what}. Give them this code; they type <b>pay</b> and the code. I will say when it is paid.\n\n<code>{}</code>", html_escape(&code))).await;
                                    send_qr(&token, chat_id, &code, "The same request, to scan").await;
                                    let (service, token, sent_code) = (service.clone(), token.clone(), code);
                                    tokio::spawn(async move {
                                        match service.await_request(&sent_code, Duration::from_secs(3600)).await {
                                            Ok(Some(line)) => send(&token, chat_id, &html_escape(&line)).await,
                                            Ok(None) => {}
                                            Err(e) => log::warn!("telegram: request watch: {e}"),
                                        }
                                    });
                                }
                                Err(e) => send(&token, chat_id, &format!("Could not make a request: {}", html_escape(&e))).await,
                            }
                        }
                        _ => {
                            if let Some(mid) =
                                send_with_keyboard(&token, chat_id, &amount_line(Purpose::Request, "", ticker), &keypad(true)).await
                            {
                                pending = Pending::Amount { purpose: Purpose::Request, buf: String::new(), message_id: mid };
                            }
                        }
                    }
                }
                "mine" => send(&token, chat_id, &html_escape(&service.mine(rest.first().copied()).await)).await,
                "key" => match service.share_key(&rest.join(" ")).await {
                    Ok(key) => {
                        send(&token, chat_id, &format!("A key of yours. Give it to whoever should pay you; they use <b>pay &lt;amount&gt;</b> and this key, and the money is theirs to send and yours to take.\n\n<code>{}</code>", html_escape(&key))).await;
                        send_qr(&token, chat_id, &key, "The same key, to scan").await;
                    }
                    Err(e) => send(&token, chat_id, &format!("Could not make a key: {}", html_escape(&e))).await,
                },
                "scan" => send(&token, chat_id, "The scan button opens the camera; a code it reads comes straight back here.").await,
                "cancel" => {
                    pending = Pending::Nothing;
                    send_with_keyboard(&token, chat_id, "Nothing pending.", &main_keyboard()).await;
                }
                _ => {
                    send_with_keyboard(&token, chat_id, HELP, &main_keyboard()).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new configuration hashes its PIN with Argon2id; one from before
    /// still verifies with its plain salted SHA-256; a reset moves it on.
    #[test]
    fn pins_verify_under_both_hashes() {
        let cfg = TelegramConfig::new("token".into(), "4321", DEFAULT_DAILY_LIMIT_PETALS);
        assert_eq!(cfg.pin_kdf.as_deref(), Some("argon2"));
        assert!(cfg.pin_matches("4321"));
        assert!(cfg.pin_matches(" 4321 "), "surrounding spaces are not part of a PIN");
        assert!(!cfg.pin_matches("1234"));
        assert!(cfg.pairing_code_live());
        assert_eq!(cfg.pairing_code.as_ref().map(|c| c.len()), Some(6));

        let mut old = cfg.clone();
        old.pin_kdf = None;
        old.pin_salt = "abcd".into();
        old.pin_hash = String::new();
        old.pin_hash = old.hash_pin("2468");
        assert!(old.pin_matches("2468"));
        assert!(!old.pin_matches("4321"));
        old.set_pin("1357");
        assert_eq!(old.pin_kdf.as_deref(), Some("argon2"));
        assert!(old.pin_matches("1357"));
        assert!(!old.pin_matches("2468"));
    }
}

// ---- Backups as messages (founder, 2026-09-23): the encrypted archive of
// 'wallet backup', posted to a private group in parts the bot can also read
// back. The bot API takes uploads to 50 MB but hands out files to 20 MB only,
// so parts are cut below 20 MB, and each carries in its caption what a
// restore needs to check it.

/// Telegram hands a bot files of at most 20 MB through getFile; the parts stay
/// under that with room for the container.
pub const BACKUP_PART_BYTES: usize = 19 * 1024 * 1024;

/// Whether the bot can see the chat: getChat answers for a chat it is in.
pub async fn chat_reachable(token: &str, chat_id: i64) -> bool {
    call(token, "getChat", &[("chat_id", chat_id.to_string())]).await.is_ok()
}

pub async fn send_plain(token: &str, chat_id: i64, text: &str) -> Result<(), String> {
    let params = [("chat_id", chat_id.to_string()), ("text", text.to_string())];
    call(token, "sendMessage", &params).await.map(|_| ())
}

/// Uploads one file as a document with a caption.
pub async fn send_document(token: &str, chat_id: i64, file_name: &str, bytes: Vec<u8>, caption: &str) -> Result<(), String> {
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .text("chat_id", chat_id.to_string())
        .text("caption", caption.to_string())
        .part("document", part);
    let url = format!("https://api.telegram.org/bot{token}/sendDocument");
    let response =
        reqwest::Client::new().post(url).multipart(form).send().await.map_err(|e| e.to_string().replace(token, "<token>"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(format!("sendDocument answered {status}: {}", body.chars().take(200).collect::<String>()))
    }
}

/// Downloads a file the bot has been told about (by file id).
pub async fn download_file(token: &str, file_id: &str) -> Result<Vec<u8>, String> {
    let params = [("file_id", file_id.to_string())];
    let info = call(token, "getFile", &params).await?;
    let path = info
        .get("result")
        .and_then(|r| r.get("file_path"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "getFile gave no file path (a file over 20 MB cannot be fetched by a bot)".to_string())?;
    let url = format!("https://api.telegram.org/file/bot{token}/{path}");
    let response = reqwest::Client::new().get(url).send().await.map_err(|e| e.to_string().replace(token, "<token>"))?;
    if !response.status().is_success() {
        return Err(format!("the file download answered {}", response.status()));
    }
    response.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string().replace(token, "<token>"))
}

/// A backup part as it arrives at the bot: which backup, which part of how
/// many, and the file to fetch.
#[derive(Debug, Clone)]
pub struct BackupPart {
    pub backup: String,
    pub index: usize,
    pub count: usize,
    pub file_id: String,
    pub size: u64,
}

/// The part file name: `<backup>.p<index>of<count>`.
pub fn part_file_name(backup: &str, index: usize, count: usize) -> String {
    format!("{backup}.p{index:03}of{count:03}")
}

pub fn parse_part_file_name(name: &str) -> Option<(String, usize, usize)> {
    let (backup, rest) = name.rsplit_once(".p")?;
    let (index, count) = rest.split_once("of")?;
    Some((backup.to_string(), index.parse().ok()?, count.parse().ok()?))
}

/// Waits for the parts of one backup to be sent (or forwarded) to the bot,
/// from anyone in `chat` if given, and hands them back once all are there.
/// `progress` is told about each part as it lands.
pub async fn collect_backup_parts(
    token: &str,
    chat: Option<i64>,
    wanted: Option<&str>,
    timeout: Duration,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Vec<BackupPart>, String> {
    let started = std::time::Instant::now();
    let mut offset: i64 = 0;
    // Skip whatever the bot had queued before this restore began.
    if let Ok(v) = call(token, "getUpdates", &[("offset", "-1".to_string()), ("timeout", "0".to_string())]).await
        && let Some(last) = v.get("result").and_then(|r| r.as_array()).and_then(|a| a.last())
        && let Some(id) = last.get("update_id").and_then(|v| v.as_i64())
    {
        offset = id + 1;
    }
    let mut parts: std::collections::BTreeMap<usize, BackupPart> = std::collections::BTreeMap::new();
    let mut backup: Option<String> = wanted.map(|w| w.to_string());
    let mut count: Option<usize> = None;
    while started.elapsed() < timeout {
        let params = [("offset", offset.to_string()), ("timeout", "20".to_string()), ("allowed_updates", "[\"message\"]".to_string())];
        let updates = match call(token, "getUpdates", &params).await {
            Ok(v) => v,
            Err(e) => {
                progress(format!("(telegram: {e}; trying again)"));
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let Some(list) = updates.get("result").and_then(|r| r.as_array()) else { continue };
        for update in list {
            if let Some(id) = update.get("update_id").and_then(|v| v.as_i64()) {
                offset = offset.max(id + 1);
            }
            let Some(message) = update.get("message") else { continue };
            if let Some(chat) = chat {
                let from_chat = message.get("chat").and_then(|c| c.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
                if from_chat != chat {
                    continue;
                }
            }
            let Some(document) = message.get("document") else { continue };
            let Some(name) = document.get("file_name").and_then(|v| v.as_str()) else { continue };
            let Some((of_backup, index, of_count)) = parse_part_file_name(name) else { continue };
            match &backup {
                Some(b) if *b != of_backup => {
                    progress(format!("(a part of another backup, {of_backup}, ignored)"));
                    continue;
                }
                None => backup = Some(of_backup.clone()),
                _ => {}
            }
            count = Some(of_count);
            let file_id = document.get("file_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let size = document.get("file_size").and_then(|v| v.as_u64()).unwrap_or(0);
            if let std::collections::btree_map::Entry::Vacant(slot) = parts.entry(index) {
                slot.insert(BackupPart { backup: of_backup, index, count: of_count, file_id, size });
                progress(format!("part {index} of {of_count} received"));
            }
        }
        if let Some(count) = count
            && parts.len() == count
            && (1..=count).all(|i| parts.contains_key(&i))
        {
            return Ok(parts.into_values().collect());
        }
    }
    Err(match (backup, count) {
        (Some(b), Some(c)) => format!("gave up waiting: {} of {c} parts of {b} arrived", parts.len()),
        _ => "gave up waiting: no backup part reached the bot".to_string(),
    })
}

#[cfg(test)]
mod backup_part_tests {
    use super::{parse_part_file_name, part_file_name};

    #[test]
    fn backup_part_names_round_trip() {
        let name = part_file_name("marigold-test10-2026-09-23T20-10-01.mgb", 7, 112);
        assert_eq!(name, "marigold-test10-2026-09-23T20-10-01.mgb.p007of112");
        assert_eq!(parse_part_file_name(&name), Some(("marigold-test10-2026-09-23T20-10-01.mgb".to_string(), 7, 112)));
        assert_eq!(parse_part_file_name("random.pdf"), None);
        assert_eq!(parse_part_file_name("x.p1of"), None);
    }
}
