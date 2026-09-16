//! The phone is a remote (FORK-PLAN P8.0h): a person's own Telegram bot,
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
        candidate.len() == self.pin_hash.len() && candidate.bytes().zip(self.pin_hash.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
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
    let form = reqwest::multipart::Form::new().text("chat_id", chat_id.to_string()).text("caption", caption.to_string()).part("photo", part);
    let url = format!("https://api.telegram.org/bot{token}/sendPhoto");
    match reqwest::Client::new().post(url).multipart(form).send().await {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => log::warn!("telegram: sendPhoto answered {}", resp.status()),
        Err(e) => log::warn!("telegram: sendPhoto: {e}"),
    }
}

async fn send(token: &str, chat_id: i64, html: &str) {
    let params = [("chat_id", chat_id.to_string()), ("text", html.to_string()), ("parse_mode", "HTML".to_string())];
    if let Err(e) = call(token, "sendMessage", &params).await {
        log::warn!("telegram: could not send to {chat_id}: {e}");
    }
}

/// What the bot is waiting for from the paired user.
enum Pending {
    Nothing,
    /// A PIN, to pay this many petals.
    PinToPay { petals: u64 },
}

const HELP: &str = "<b>Your wallet</b>\n\
/balance — what you hold\n\
/pay &lt;amount&gt; — a code to hand to someone (asks your PIN)\n\
/receive &lt;code&gt; — take a code you were given; pasting codes on their own works too\n\
/request [amount] — a code for someone to pay you\n\
/history — the last payments\n\
/status — the network, the miner\n\
/mine start|stop|status — the miner, if this wallet mines\n\
/cancel — forget a PIN question";

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
    loop {
        let params = [("offset", offset.to_string()), ("timeout", "25".to_string()), ("allowed_updates", "[\"message\"]".to_string())];
        let updates = match call(&token, "getUpdates", &params).await {
            Ok(v) => v,
            Err(e) => {
                // A bad token is a configuration problem, not a hiccup: say
                // so, and do not hammer Telegram about it.
                if e.contains("401") {
                    log::warn!("telegram: the bot token is refused (401) — check 'mobile telegram' in the wallet; trying again in a minute");
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
            let Some(message) = update.get("message") else { continue };
            let Some(from) = message.get("from").and_then(|f| f.get("id")).and_then(|v| v.as_i64()) else { continue };
            let Some(chat_id) = message.get("chat").and_then(|c| c.get("id")).and_then(|v| v.as_i64()) else { continue };
            let text = message.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
            let message_id = message.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0);

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
                    send(&token, chat_id, &format!("Paired. This chat now moves money in your wallet: keep 2FA on your Telegram account.\n\n{HELP}")).await;
                } else {
                    send(&token, chat_id, "Not paired. In your wallet, 'mobile telegram' shows a code; send it here as /start &lt;code&gt;.").await;
                }
                continue;
            }
            if cfg.user_id != Some(from) {
                log::warn!("telegram: ignored a message from user {from}, not the paired one");
                continue;
            }
            if locked {
                send(&token, chat_id, "Locked after three wrong PINs. Restart the wallet service to unlock.").await;
                continue;
            }

            // A PIN, if one was asked for.
            if let Pending::PinToPay { petals } = pending {
                pending = Pending::Nothing;
                if text.starts_with('/') && text != "/cancel" {
                    send(&token, chat_id, "The PIN question was dropped. Ask again when ready.").await;
                    // fall through to handle the command
                } else if text == "/cancel" {
                    send(&token, chat_id, "Cancelled.").await;
                    continue;
                } else if !cfg.pin_matches(&text) {
                    delete(&token, chat_id, message_id).await;
                    pin_failures += 1;
                    if pin_failures >= 3 {
                        locked = true;
                        log::warn!("telegram: locked after three wrong PINs");
                        send(&token, chat_id, "Wrong PIN, three times. Locked until the wallet service is restarted.").await;
                    } else {
                        send(&token, chat_id, "Wrong PIN. Ask again when ready.").await;
                    }
                    continue;
                } else {
                    delete(&token, chat_id, message_id).await;
                    pin_failures = 0;
                    match service.spend_allowed(petals, cfg.daily_limit_petals) {
                        Err(why) => send(&token, chat_id, &html_escape(&why)).await,
                        Ok(()) => match service.pay(petals).await {
                            Ok(paid) => {
                                service.note_spent(petals);
                                service.say(format!("telegram: paid {} {} as a code", sompi_to_kaspa_string(paid.value_petals), service.ticker()));
                                send(
                                    &token,
                                    chat_id,
                                    &format!(
                                        "{} {} in {} note{}, plus a 0.01 stamp for the receiver (fee {}). Give them this code; they type <b>receive</b> and the code. Anyone who sees it can take the money.\n\n<code>{}</code>",
                                        sompi_to_kaspa_string(paid.value_petals),
                                        service.ticker(),
                                        paid.notes,
                                        if paid.notes == 1 { "" } else { "s" },
                                        sompi_to_kaspa_string(paid.fee_petals),
                                        html_escape(&paid.code)
                                    ),
                                )
                                .await;
                                send_qr(&token, chat_id, &paid.code, "The same code, to scan").await;
                            }
                            Err(e) => send(&token, chat_id, &format!("Could not pay: {}", html_escape(&e))).await,
                        },
                    }
                    continue;
                }
            }

            // Codes pasted on their own — a forwarded faucet message, say —
            // are received, every one of them.
            let codes: Vec<&str> = text.split_whitespace().filter(|w| w.starts_with("marigoldpay:") || w.starts_with("marigoldnote:")).collect();
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

            let mut words = text.split_whitespace();
            let command = words.next().unwrap_or("").split('@').next().unwrap_or("");
            let rest: Vec<&str> = words.collect();
            match command {
                "/start" | "/help" => send(&token, chat_id, HELP).await,
                "/balance" => send(&token, chat_id, &html_escape(&service.balance_text().await)).await,
                "/status" => send(&token, chat_id, &html_escape(&service.status_text().await)).await,
                "/history" => {
                    let n = rest.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(10);
                    send(&token, chat_id, &format!("<pre>{}</pre>", html_escape(&service.history_text(n)))).await
                }
                "/pay" => match rest.first().and_then(|s| crate::utils::try_parse_required_nonzero_kaspa_as_sompi_u64(Some(s)).ok()) {
                    Some(petals) => match service.spend_allowed(petals, cfg.daily_limit_petals) {
                        Err(why) => send(&token, chat_id, &html_escape(&why)).await,
                        Ok(()) => {
                            pending = Pending::PinToPay { petals };
                            send(&token, chat_id, &format!("Pay {} {} as a code? Reply with your PIN, or /cancel.", sompi_to_kaspa_string(petals), service.ticker())).await;
                        }
                    },
                    None => send(&token, chat_id, "/pay &lt;amount&gt;, for example /pay 5").await,
                },
                "/receive" => match rest.first() {
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
                    None => send(&token, chat_id, "/receive &lt;code&gt; — or just paste the code.").await,
                },
                "/request" => {
                    let petals = rest.first().and_then(|s| crate::utils::try_parse_required_nonzero_kaspa_as_sompi_u64(Some(s)).ok());
                    match service.request(petals).await {
                        Ok(code) => {
                            let what = match petals {
                                Some(p) => format!("for {} {}", sompi_to_kaspa_string(p), service.ticker()),
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
                "/mine" => send(&token, chat_id, &html_escape(&service.mine(rest.first().copied()).await)).await,
                "/cancel" => send(&token, chat_id, "Nothing to cancel.").await,
                _ => send(&token, chat_id, HELP).await,
            }
        }
    }
}
