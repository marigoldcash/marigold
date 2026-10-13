//! Telegram, as far as the wallet service needs it: the configuration a wallet
//! keeps for its bot, the registry of which wallet owns which chat, the Bot API
//! call, and the transport the backups use to post themselves as messages and
//! to be collected again. The bot itself (the long-polling loop, the keyboards,
//! the pending states) stays in `kaspa-cli`, which imports all of this.
//! (docs/marigold/MOBILE-PLAN.md, PH-M1b2.)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
    /// member of, given once and kept. Superseded by `home_chat_id`; read for
    /// settings written before it.
    #[serde(default)]
    pub backup_chat_id: Option<i64>,
    /// This wallet's own chat with the bot, when the bot serves several
    /// wallets: a group the owner made for it ('telegram home <group id>').
    /// Its backups go there and the bot answers it there; the bot's private
    /// chat belongs to whichever wallet claimed it first (founder,
    /// 2026-10-06: one bot per wallet runs into Telegram's twenty-bot limit).
    #[serde(default)]
    pub home_chat_id: Option<i64>,
    /// The bot's private chat already keeps another wallet of this
    /// computer's, so it is not this wallet's to post into.
    #[serde(default)]
    pub private_chat_taken: bool,
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

/// Which wallet of this computer's a bot's chat belongs to — one chat, one
/// vault, so two wallets on one bot never mix their backups in one place
/// (founder, 2026-10-06). Kept beside the wallets, in
/// `<folder>/telegram-homes.json`, as `"<token fingerprint>/<chat id>": "<wallet>"`.
pub mod homes {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn path(folder: &Path) -> PathBuf {
        folder.join("telegram-homes.json")
    }

    /// The first bytes of the token's hash: enough to tell bots apart, and
    /// nothing a reader of the file could use.
    pub fn fingerprint(token: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"marigold-telegram-home");
        h.update(token.as_bytes());
        faster_hex::hex_string(&h.finalize()[..8])
    }

    fn load(folder: &Path) -> BTreeMap<String, String> {
        std::fs::read_to_string(path(folder)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    /// The wallet that owns this chat of this bot, if any.
    pub fn owner(folder: &Path, token: &str, chat_id: i64) -> Option<String> {
        load(folder).get(&format!("{}/{chat_id}", fingerprint(token))).cloned()
    }

    /// Records the chat as this wallet's. Refused when another wallet holds it.
    pub fn claim(folder: &Path, token: &str, chat_id: i64, wallet: &str) -> Result<(), String> {
        let mut map = load(folder);
        let key = format!("{}/{chat_id}", fingerprint(token));
        match map.get(&key) {
            Some(other) if other != wallet => return Err(format!("that chat already keeps the wallet '{other}'")),
            _ => {}
        }
        map.insert(key, wallet.to_string());
        let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
        crate::backup::write_owner_only(&path(folder), text.as_bytes()).map_err(|e| e.to_string())
    }

    /// Lets go of every chat this wallet holds on this bot ('telegram unlink').
    pub fn release(folder: &Path, token: &str, wallet: &str) {
        let mut map = load(folder);
        let prefix = format!("{}/", fingerprint(token));
        map.retain(|k, v| !(k.starts_with(&prefix) && v == wallet));
        if let Ok(text) = serde_json::to_string_pretty(&map) {
            let _ = crate::backup::write_owner_only(&path(folder), text.as_bytes());
        }
    }
}
/// Wrong pairing codes tolerated before the code is thrown away.
pub const PAIRING_FAILURES_ALLOWED: u32 = 5;

pub fn now_unix() -> u64 {
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
            home_chat_id: None,
            private_chat_taken: false,
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

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub async fn call(token: &str, method: &str, params: &[(&str, String)]) -> Result<serde_json::Value, String> {
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

/// A backup message: delivered silently — no sound, no badge — so a chat
/// that also carries payment codes stays quiet while the wallet keeps its
/// backup current (founder, 2026-09-24: "messages can be sent silent").
pub async fn send_plain(token: &str, chat_id: i64, text: &str) -> Result<i64, String> {
    let params = [("chat_id", chat_id.to_string()), ("text", text.to_string()), ("disable_notification", "true".to_string())];
    let v = call(token, "sendMessage", &params).await?;
    Ok(v.get("result").and_then(|r| r.get("message_id")).and_then(|m| m.as_i64()).unwrap_or(0))
}

/// Deletes one of the bot's own messages. Telegram allows it for two days
/// after posting and refuses afterwards ("message can't be deleted"), in a
/// private chat as much as anywhere — measured 2026-10-06 — so whatever
/// tidying the wallet does has to happen inside that window.
pub async fn delete_message(token: &str, chat_id: i64, message_id: i64) -> Result<(), String> {
    let params = [("chat_id", chat_id.to_string()), ("message_id", message_id.to_string())];
    call(token, "deleteMessage", &params).await.map(|_| ())
}

/// A client for a 19 MB part either way: ten minutes is generous on any
/// line that works at all, and a line that does not gets an answer.
fn slow_client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(600)).build().unwrap_or_else(|_| reqwest::Client::new())
}

/// Uploads one file as a document with a caption, silently (see `send_plain`).
pub async fn send_document(token: &str, chat_id: i64, file_name: &str, bytes: Vec<u8>, caption: &str) -> Result<i64, String> {
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .text("chat_id", chat_id.to_string())
        .text("caption", caption.to_string())
        .text("disable_notification", "true")
        .part("document", part);
    let url = format!("https://api.telegram.org/bot{token}/sendDocument");
    // A deadline: an upload that stalls must fail and say so, not hold
    // 'close' for half an hour (tester on Windows, 2026-10-07).
    let response = slow_client()
        .post(url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("the upload did not finish: {}", e.to_string().replace(token, "<token>")))?;
    if response.status().is_success() {
        let body = response.text().await.map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        Ok(v.get("result").and_then(|r| r.get("message_id")).and_then(|m| m.as_i64()).unwrap_or(0))
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
    let response = slow_client().get(url).send().await.map_err(|e| e.to_string().replace(token, "<token>"))?;
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
/// The file name of one part: the backup's own name when it fits in one
/// file, `… (part 2 of 3).mgb` when it does not.
pub fn part_file_name(backup: &str, index: usize, count: usize) -> String {
    if count <= 1 {
        return backup.to_string();
    }
    match backup.strip_suffix(".mgb") {
        Some(stem) => format!("{stem} (part {index} of {count}).mgb"),
        None => format!("{backup} (part {index} of {count})"),
    }
}

/// (backup name, part index, part count) from a file name: the readable
/// form above, the first scheme's `<backup>.p001of003`, or any other `.mgb`
/// file, which is a whole backup on its own.
pub fn parse_part_file_name(name: &str) -> Option<(String, usize, usize)> {
    if let Some(stem) = name.strip_suffix(".mgb") {
        if let Some((base, rest)) = stem.rsplit_once(" (part ")
            && let Some(inner) = rest.strip_suffix(')')
            && let Some((index, count)) = inner.split_once(" of ")
            && let (Ok(index), Ok(count)) = (index.parse(), count.parse())
        {
            return Some((format!("{base}.mgb"), index, count));
        }
        return Some((name.to_string(), 1, 1));
    }
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
    // Whatever the bot already has queued is read too: a part forwarded a
    // minute before the restore began is exactly what is wanted, and parts of
    // any other backup are ignored by name.
    let mut offset: i64 = 0;
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

/// Waits for the parts of any number of backups forwarded to the bot — a
/// checkpoint and the deltas after it, in any order — and hands them back
/// grouped by backup once every backup seen is complete and a poll has
/// brought nothing new. `progress` is told about each part as it lands.
pub async fn collect_backup_sets(
    token: &str,
    timeout: Duration,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<std::collections::BTreeMap<String, Vec<BackupPart>>, String> {
    use std::collections::BTreeMap;
    let started = std::time::Instant::now();
    let mut offset: i64 = 0;
    let mut sets: BTreeMap<String, BTreeMap<usize, BackupPart>> = BTreeMap::new();
    while started.elapsed() < timeout {
        // A long poll until the first part; short ones after, so a complete
        // set is handed back within seconds of its last part.
        let wait = if sets.is_empty() { "20" } else { "5" };
        let params = [("offset", offset.to_string()), ("timeout", wait.to_string()), ("allowed_updates", "[\"message\"]".to_string())];
        let updates = match call(token, "getUpdates", &params).await {
            Ok(v) => v,
            Err(e) => {
                progress(format!("(telegram: {e}; trying again)"));
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let Some(list) = updates.get("result").and_then(|r| r.as_array()) else { continue };
        let mut new_parts = false;
        for update in list {
            if let Some(id) = update.get("update_id").and_then(|v| v.as_i64()) {
                offset = offset.max(id + 1);
            }
            let Some(document) = update.get("message").and_then(|m| m.get("document")) else { continue };
            let Some(name) = document.get("file_name").and_then(|v| v.as_str()) else { continue };
            let Some((backup, index, count)) = parse_part_file_name(name) else { continue };
            let file_id = document.get("file_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let size = document.get("file_size").and_then(|v| v.as_u64()).unwrap_or(0);
            if let std::collections::btree_map::Entry::Vacant(slot) = sets.entry(backup.clone()).or_default().entry(index) {
                slot.insert(BackupPart { backup: backup.clone(), index, count, file_id, size });
                progress(format!("{backup}: part {index} of {count} received"));
                new_parts = true;
            }
        }
        let all_complete = !sets.is_empty()
            && sets.values().all(|parts| {
                let count = parts.values().next().map(|p| p.count).unwrap_or(0);
                count > 0 && parts.len() == count && (1..=count).all(|i| parts.contains_key(&i))
            });
        if all_complete && !new_parts {
            return Ok(sets.into_iter().map(|(name, parts)| (name, parts.into_values().collect())).collect());
        }
    }
    if sets.is_empty() {
        return Err("gave up waiting: no backup part reached the bot".to_string());
    }
    let missing: Vec<String> = sets
        .iter()
        .filter_map(|(name, parts)| {
            let count = parts.values().next().map(|p| p.count).unwrap_or(0);
            (parts.len() != count).then(|| format!("{name} ({} of {count} parts)", parts.len()))
        })
        .collect();
    Err(format!("gave up waiting: incomplete — {}", missing.join(", ")))
}

#[cfg(test)]
mod backup_part_tests {
    use super::{parse_part_file_name, part_file_name};

    #[test]
    fn backup_part_names_round_trip() {
        let name = part_file_name("Marigold backup - test10 - 2026-09-24 01.17.46 - full.mgb", 7, 112);
        assert_eq!(name, "Marigold backup - test10 - 2026-09-24 01.17.46 - full (part 7 of 112).mgb");
        assert_eq!(
            parse_part_file_name(&name),
            Some(("Marigold backup - test10 - 2026-09-24 01.17.46 - full.mgb".to_string(), 7, 112))
        );
        let single = part_file_name("Marigold backup - test10 - 2026-09-24 01.17.46 - change 2.mgb", 1, 1);
        assert_eq!(single, "Marigold backup - test10 - 2026-09-24 01.17.46 - change 2.mgb");
        assert_eq!(parse_part_file_name(&single), Some((single.clone(), 1, 1)));
        assert_eq!(
            parse_part_file_name("marigold-test10-2026-09-23T20-10-01.mgb.p007of112"),
            Some(("marigold-test10-2026-09-23T20-10-01.mgb".to_string(), 7, 112))
        );
        assert_eq!(parse_part_file_name("random.pdf"), None);
        assert_eq!(parse_part_file_name("x.p1of"), None);
    }
}
