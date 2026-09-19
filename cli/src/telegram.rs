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
        std::fs::write(path, text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn new(token: String, pin: &str, daily_limit_petals: u64) -> Self {
        let salt: [u8; 16] = rand::random();
        let code: u32 = rand::random::<u32>() % 1_000_000;
        let mut cfg = Self {
            token,
            user_id: None,
            pairing_code: Some(format!("{code:06}")),
            pin_salt: hex::encode(salt),
            pin_hash: String::new(),
            daily_limit_petals,
        };
        cfg.pin_hash = cfg.hash_pin(pin);
        cfg
    }

    fn hash_pin(&self, pin: &str) -> String {
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
    let value: serde_json::Value = workflow_http::get_json(url).await.map_err(|e| format!("{method}: {e}"))?;
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
    let mut locked = false;
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
                if cfg.user_id != Some(from) || locked {
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
                                        log::warn!("telegram: locked after three wrong PINs");
                                        edit(
                                            &token,
                                            chat_id,
                                            message_id,
                                            "Wrong PIN, three times. Locked until the wallet is reopened.",
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
                                let outcome = match &share_key {
                                    Some(key) => service.pay_locked(petals, key, LOCK_SECONDS).await,
                                    None => service.pay(petals).await,
                                };
                                match outcome {
                                    Ok(paid) if share_key.is_some() => {
                                        service.note_spent(petals);
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
                                let line = if share_key.is_some() {
                                    pin_line_locked(petals, buf, ticker)
                                } else {
                                    pin_line(petals, buf, ticker)
                                };
                                edit(&token, chat_id, message_id, &line, Some(&keypad(false))).await;
                            }
                            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => {
                                if buf.len() < 12 {
                                    buf.push_str(k);
                                }
                                let line = if share_key.is_some() {
                                    pin_line_locked(petals, buf, ticker)
                                } else {
                                    pin_line(petals, buf, ticker)
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

            // Pairing: the one moment an unpaired user is listened to.
            if cfg.user_id.is_none() {
                let code = text.strip_prefix("/start").map(str::trim).unwrap_or("");
                if !code.is_empty() && cfg.pairing_code.as_deref() == Some(code) {
                    cfg.user_id = Some(from);
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
                    send(
                        &token,
                        chat_id,
                        "Not paired. In your wallet, 'mobile telegram' shows a code; send it here as /start &lt;code&gt;.",
                    )
                    .await;
                }
                continue;
            }
            if cfg.user_id != Some(from) {
                log::warn!("telegram: ignored a message from user {from}, not the paired one");
                continue;
            }
            if locked {
                send(&token, chat_id, "Locked after three wrong PINs. Close and reopen the wallet to unlock.").await;
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
            if text.starts_with("marigoldreq:") {
                send(&token, chat_id, "A request code: paying requests from the phone is not there yet. In your wallet: <code>pay &lt;that code&gt;</code>.").await;
                continue;
            }

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
