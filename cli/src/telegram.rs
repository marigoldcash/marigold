//! The phone is a remote (PLAN P8.0h): a person's own Telegram bot,
//! long-polled by their wallet running as a service. Nothing but Telegram is
//! reached, nothing listens, and no key ever leaves the machine the wallet
//! is on. One Telegram user is paired; everyone else is ignored.

use crate::serve::WalletService;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// The configuration, the homes registry, the Bot API call and the backup
// transport live in the wallet service crate (PH-M1b2); this module is the bot.
pub use kaspa_wallet_service::telegram::*;

// --- The Bot API, as GETs ----------------------------------------------------

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
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
/// Posts the code as a QR picture; the message id comes back so the picture
/// can be taken down again once the code has done its work.
async fn send_qr(token: &str, chat_id: i64, code: &str, caption: &str) -> Option<i64> {
    let png = crate::qrpng::qr_png(code)?;
    let part = reqwest::multipart::Part::bytes(png).file_name("code.png").mime_str("image/png").ok()?;
    let form =
        reqwest::multipart::Form::new().text("chat_id", chat_id.to_string()).text("caption", caption.to_string()).part("photo", part);
    let url = format!("https://api.telegram.org/bot{token}/sendPhoto");
    match reqwest::Client::new().post(url).multipart(form).send().await {
        Ok(resp) if resp.status().is_success() => {
            let body = resp.text().await.ok()?;
            let v: serde_json::Value = serde_json::from_str(&body).ok()?;
            v.get("result").and_then(|r| r.get("message_id")).and_then(|m| m.as_i64())
        }
        Ok(resp) => {
            log::warn!("telegram: sendPhoto answered {}", resp.status());
            None
        }
        Err(e) => {
            log::warn!("telegram: sendPhoto: {e}");
            None
        }
    }
}

/// Takes a code's messages down once it has done its work: a request when it
/// is paid or has lapsed, a hand-over when the notes have been taken (or came
/// back from a lock). Bearer value should not sit in the chat history
/// (founder, 2026-10-06). Within Telegram's two-day window only: a hand-over
/// still untaken after that keeps its code, which may be the only copy.
async fn tidy_code(token: &str, chat_id: i64, messages: &[Option<i64>]) {
    for id in messages.iter().flatten() {
        if let Err(e) = delete_message(token, chat_id, *id).await {
            log::info!("telegram: could not take a code message down: {e}");
        }
    }
}

const CODE_TIDY_WINDOW: Duration = Duration::from_secs(47 * 3600);

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
/// Watches a hand-over's notes and takes the code's messages down once they
/// have been taken (or came back from a lock), within the two-day window.
fn watch_handover(
    service: Arc<WalletService>,
    token: String,
    chat_id: i64,
    messages: [Option<i64>; 2],
    serials: Vec<kaspa_consensus_core::Hash>,
) {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        while started.elapsed() < CODE_TIDY_WINDOW {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if service.handover_settled(&serials).await {
                tidy_code(&token, chat_id, &messages).await;
                return;
            }
        }
    });
}

/// Whether the bot answers in this chat for this wallet: its private chat
/// (unless another wallet holds it) and its home group.
fn chat_allowed(cfg: &TelegramConfig, chat_id: i64) -> bool {
    if cfg.home_chat_id == Some(chat_id) {
        return true;
    }
    if cfg.private_chat_taken {
        return false;
    }
    cfg.chat_id.is_none_or(|paired| paired == chat_id)
}

/// When the bot loop last came round, as seconds since the epoch: a loop
/// that is alive polls every twenty seconds at most, so a stamp older than
/// a couple of minutes means a hung task, which housekeeping restarts.
static BOT_LAST_POLL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn note_bot_poll() {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    BOT_LAST_POLL.store(now, std::sync::atomic::Ordering::Relaxed);
}

/// Seconds since the bot loop last came round; None before it ever did.
pub fn bot_poll_age() -> Option<u64> {
    let last = BOT_LAST_POLL.load(std::sync::atomic::Ordering::Relaxed);
    if last == 0 {
        return None;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    Some(now.saturating_sub(last))
}

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
        note_bot_poll();
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
                    log::warn!("telegram: the bot token is refused (401) — check 'telegram' in the wallet; trying again in a minute");
                    tokio::time::sleep(Duration::from_secs(60)).await;
                } else if e.contains("Conflict") || e.contains("terminated by other getUpdates") {
                    // Two pollers on one token: another wallet open with the
                    // same bot, here or on another machine. Only one answers.
                    service.say("telegram: another copy of this bot is answering — another wallet open with the same bot? Only one can at a time; trying again in a minute".to_string());
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
                if cfg.user_id != Some(from) || locked || !chat_allowed(&cfg, chat_id) {
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
                                                let qr_id = send_qr(&token, chat_id, &code, "The same request, to scan").await;
                                                let (service, token, sent_code) = (service.clone(), token.clone(), code);
                                                tokio::spawn(async move {
                                                    match service.await_request(&sent_code, Duration::from_secs(3600)).await {
                                                        Ok(Some(line)) => send(&token, chat_id, &html_escape(&line)).await,
                                                        Ok(None) => {}
                                                        Err(e) => log::warn!("telegram: request watch: {e}"),
                                                    }
                                                    // Paid, or lapsed (a request lives twenty minutes): the code
                                                    // is spent either way.
                                                    tidy_code(&token, chat_id, &[Some(message_id), qr_id]).await;
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
                                            "Wrong PIN, three times. Locked; 'telegram unlock' in the wallet clears it.",
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
                                        let _ = send_qr(&token, chat_id, &paid.code, "The receipt, to scan").await;
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
                                        let qr_id = send_qr(&token, chat_id, &paid.code, "The same code, to scan").await;
                                        watch_handover(
                                            service.clone(),
                                            token.clone(),
                                            chat_id,
                                            [Some(message_id), qr_id],
                                            paid.serials.clone(),
                                        );
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
                                        let qr_id = send_qr(&token, chat_id, &paid.code, "The same code, to scan").await;
                                        watch_handover(
                                            service.clone(),
                                            token.clone(),
                                            chat_id,
                                            [Some(message_id), qr_id],
                                            paid.serials.clone(),
                                        );
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
            // A pairing from before the chat was recorded learns it from the
            // first message its user sends, so backups have somewhere to go.
            if cfg.user_id == Some(from) && cfg.chat_id.is_none() {
                cfg.chat_id = Some(chat_id);
                if let Err(e) = cfg.save(&cfg_path) {
                    log::warn!("telegram: could not record the chat: {e}");
                }
            }
            // A backup part forwarded to a bot that is being answered by an
            // open wallet would be swallowed here, and the restore on the new
            // machine would then find nothing (seen 2026-09-24). Say so.
            if let Some(name) = message.get("document").and_then(|d| d.get("file_name")).and_then(|v| v.as_str())
                && parse_part_file_name(name).is_some()
            {
                send_with_keyboard(
                    &token,
                    chat_id,
                    "That is a backup part, and this wallet is open, so it went nowhere. To restore on another machine, run \
                     'wallet restore telegram' there first, close every other copy of this wallet, then forward the parts.",
                    &main_keyboard(),
                )
                .await;
                continue;
            }
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
                    // One chat, one vault: a private chat that already keeps
                    // another wallet of this computer's is not this one's.
                    let folder = cfg_path.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf());
                    let taken_by = folder
                        .as_deref()
                        .and_then(|f| homes::owner(f, &token, chat_id))
                        .filter(|owner| owner != service.wallet_name());
                    cfg.private_chat_taken = taken_by.is_some();
                    if let Err(e) = cfg.save(&cfg_path) {
                        log::error!("telegram: could not save the pairing: {e}");
                    }
                    if let Some(other) = &taken_by {
                        service.say(format!(
                            "telegram: paired with user {from}. This chat already answers for the wallet '{other}'; for the bot to answer this wallet too, make a group, add the bot, then 'telegram home <group id>'. The backups cover this wallet in that chat either way."
                        ));
                        send_with_keyboard(
                            &token,
                            chat_id,
                            &format!("Paired. This chat already answers for the wallet '{}'. For me to answer this wallet too, make a group, add me to it, and run <b>telegram home &lt;group id&gt;</b> in the wallet. The backups here cover this wallet either way.\n\n{HELP}", html_escape(other)),
                            &main_keyboard(),
                        )
                        .await;
                        continue;
                    }
                    service.say(format!(
                        "telegram: paired with user {from} — the wallet offers the automatic backup to this chat at its next open ('telegram backup' turns it on now)"
                    ));
                    send_with_keyboard(
                        &token,
                        chat_id,
                        &format!(
                            "Paired. This chat now moves money in your wallet: keep 2FA on your Telegram account.\n\n\
                             It can also hold your wallet's backup: 'telegram backup' in the wallet posts an encrypted copy here \
                             and keeps it current by itself.\n\n{HELP}"
                        ),
                        &main_keyboard(),
                    )
                    .await;
                } else {
                    if !code.is_empty() && cfg.pairing_code.is_some() {
                        cfg.pairing_failures += 1;
                        if cfg.pairing_failures >= PAIRING_FAILURES_ALLOWED {
                            cfg.pairing_code = None;
                            service.say(
                                "telegram: five wrong pairing codes — the code is dead; 'telegram code' in the wallet makes a new one"
                                    .to_string(),
                            );
                        }
                        if let Err(e) = cfg.save(&cfg_path) {
                            log::error!("telegram: could not save: {e}");
                        }
                    }
                    send(&token, chat_id, "This bot answers its owner. If that is you: in your wallet, 'telegram' shows a code; send it here as /start &lt;code&gt;.").await;
                }
                continue;
            }
            if cfg.user_id != Some(from) {
                log::warn!("telegram: ignored a message from user {from}, not the paired one");
                continue;
            }
            if !chat_allowed(&cfg, chat_id) {
                log::warn!("telegram: ignored a message from chat {chat_id}, not the paired chat");
                continue;
            }
            if locked {
                send(&token, chat_id, "Locked after three wrong PINs. In your wallet, 'telegram unlock' clears it.").await;
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
