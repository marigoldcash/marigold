//! Automatic backups to Telegram, checkpoint and delta (founder, 2026-09-24):
//! "like an Apple Cloud backup of an iPhone … always up to date", and "we never
//! have to forward more than a week's worth back to the wallet".
//!
//! After the first `telegram backup`, the wallet keeps a backup current by
//! itself while it is open — in the bot's own chat with its owner, the same
//! chat the payment codes arrive in (founder, 2026-09-24: "the person will
//! already have created the bot, so why not send the message direct"), or in
//! a group if one was given: a *checkpoint* — every file — once a day or when
//! the deltas have grown past half its size, and a *delta* — only the files
//! changed since the last post, plus the names of any removed — whenever a
//! vault has changed and been quiet for two minutes, at most every ten
//! minutes. A restore takes the latest checkpoint and the deltas after it,
//! forwarded to the bot in any order.
//!
//! EVERY WALLET, EACH UNDER ITS OWN WORDS (founder, 2026-10-06)
//!
//! A backup covers every wallet in the folder, not only the open one: a
//! reserve wallet that is never opened is exactly the one a lost disk would
//! take for good. Each wallet's files are sealed to a public key derived from
//! that wallet's 24 words (`backup::seal_to`), so one bundle holds them all
//! and each part opens with its own words alone — the open wallet's words
//! never stand in for another's. The public half sits in the clear in
//! `<name>.wallet/backup.pub`, written when a wallet is made or opened; a
//! wallet from before this has none until it is opened once or its words are
//! typed once (`cover`), and the status says which are waiting.
//!
//! What was last posted — paths and digests of every wallet's files — is
//! kept per destination chat in `<folder>/telegram-backup-<bot>-<chat>.json`,
//! which is how a delta knows what changed and how two wallets sharing one
//! bot keep one chat current between them. Whether a wallet's automatic
//! backups are on is the wallet's own setting, in `<name>.wallet/telegram-backup.json`.

use crate::backup as archive;
use crate::backup::ArchiveEntry;
use crate::bundle::*;
use crate::cli::KaspaCli;
use crate::imports::*;
use crate::telegram::{BACKUP_PART_BYTES, TelegramConfig, part_file_name, send_document, send_plain};
use kaspa_wallet_core::wallet::Wallet;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A checkpoint at least this often. Daily rather than weekly since
/// 2026-10-06: Telegram lets a bot delete its own messages for two days only
/// (measured), so keeping just the last two full copies in the chat means
/// the third-newest must be deleted while it is still under two days old —
/// which a daily copy allows and a weekly one never would.
pub const CHECKPOINT_EVERY_SECS: u64 = 23 * 3600;
/// How many full copies (with the changes after each) stay in the chat.
pub const KEEP_CHECKPOINTS: usize = 2;
/// A delta only when the vault has been quiet this long since its last change.
pub const QUIET_SECS: u64 = 120;
/// And at most this often.
pub const DELTA_EVERY_SECS: u64 = 600;
/// A new checkpoint once the deltas since the last one outweigh half of it —
/// once there is enough of it to matter: a small wallet's keys file alone
/// is more than half of its checkpoint, and below this floor a restore
/// forwards a few kilobytes either way (seen on the first live test).
const DELTA_WEIGHT_LIMIT: f64 = 0.5;
const DELTA_WEIGHT_FLOOR: u64 = 256 * 1024;
/// Posted before every full copy, so the instruction for a restore is one
/// sentence: forward everything from the last dashed line to the end
/// (founder, 2026-09-24).
pub const DIVIDER: &str = "────────────────────────────────";
/// Where backups go: the group if one was set, else the chat the bot was
/// paired in. A bot paired before the chat was recorded has the user and no
/// chat; a private chat's id is the user's id, so that is where its backups go.
/// A private chat that already answers for another wallet of this computer's
/// still takes the backups: they cover every wallet anyway.
pub fn target_chat(cfg: &TelegramConfig) -> Option<i64> {
    cfg.home_chat_id.or(cfg.backup_chat_id).or(cfg.chat_id).or(cfg.user_id)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A wallet's own say in the matter: `<name>.wallet/telegram-backup.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WalletBackupSettings {
    /// The one-time question ("back up automatically? highly recommended")
    /// has been put, whatever the answer.
    #[serde(default)]
    pub asked: bool,
    /// Set by 'telegram autobackup off'.
    #[serde(default)]
    pub paused: bool,
    /// The first 'telegram backup' has run: from then on the wallet keeps
    /// the chat current by itself while it is open.
    #[serde(default)]
    pub started: bool,
    /// What this file held before the posting state moved to the folder
    /// (2026-10-06): a checkpoint means the backups were running, and the
    /// posted groups are adopted by the chat's index so they are still
    /// taken down in their turn.
    #[serde(default, skip_serializing)]
    checkpoint: String,
    #[serde(default, skip_serializing)]
    posted: Vec<PostedGroup>,
}

impl WalletBackupSettings {
    pub fn path(wallet_dir: &Path) -> PathBuf {
        wallet_dir.join("telegram-backup.json")
    }

    pub fn load(wallet_dir: &Path) -> Self {
        let mut settings: Self =
            std::fs::read_to_string(Self::path(wallet_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if !settings.checkpoint.is_empty() {
            settings.started = true;
        }
        settings
    }

    pub fn save(&self, wallet_dir: &Path) -> Result<()> {
        // The legacy posting state stays in the file until a chat index has
        // adopted it, so saving a setting never loses the groups to tidy.
        let mut value = serde_json::to_value(self).map_err(|e| Error::custom(e.to_string()))?;
        if !self.posted.is_empty()
            && let Some(map) = value.as_object_mut()
        {
            map.insert("checkpoint".into(), serde_json::Value::String(self.checkpoint.clone()));
            map.insert("posted".into(), serde_json::to_value(&self.posted).map_err(|e| Error::custom(e.to_string()))?);
        }
        let text = serde_json::to_string_pretty(&value).map_err(|e| Error::custom(e.to_string()))?;
        write_json(&Self::path(wallet_dir), &text)
    }

    /// The legacy groups handed over to the chat's index; the file keeps
    /// only the settings from then on.
    fn take_legacy_posted(&mut self, wallet_dir: &Path, chat_id: i64) -> Vec<PostedGroup> {
        if self.posted.is_empty() {
            return Vec::new();
        }
        let (mine, _): (Vec<_>, Vec<_>) = std::mem::take(&mut self.posted).into_iter().partition(|g| g.chat_id == chat_id);
        self.checkpoint.clear();
        let _ = self.save(wallet_dir);
        mine
    }
}

fn write_json(path: &Path, text: &str) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    archive::write_owner_only(&tmp, text.as_bytes()).map_err(|e| Error::custom(format!("cannot write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::custom(format!("cannot write {}: {e}", path.display())))?;
    Ok(())
}

/// What was last posted to one chat: which checkpoint, how many deltas after
/// it, and every file's digest — of every wallet — as of the last post.
/// `<folder>/telegram-backup-<bot fingerprint>-<chat id>.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupIndex {
    /// The checkpoint's stamp (`c20260924T100000`); empty before the first.
    #[serde(default)]
    pub checkpoint: String,
    #[serde(default)]
    pub checkpoint_at: u64,
    #[serde(default)]
    pub checkpoint_bytes: u64,
    #[serde(default)]
    pub delta_seq: u32,
    #[serde(default)]
    pub delta_bytes: u64,
    #[serde(default)]
    pub last_post_at: u64,
    /// path → sha256 hex, as of the last post. Paths lead with the wallet
    /// (`reserve.wallet/…`), so one map covers the folder.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// The cheap change gate: every wallet's manifest and keys file, size
    /// and modification time, when the index was last brought up to date.
    #[serde(default)]
    pub gate: String,
    /// The messages posted per full copy — the dashed line, the sentence,
    /// the files, and the deltas' messages after it — so the oldest can be
    /// taken down once more than `KEEP_CHECKPOINTS` are in the chat.
    #[serde(default)]
    pub posted: Vec<PostedGroup>,
    /// The wallets the last checkpoint covered.
    #[serde(default)]
    pub wallets: Vec<String>,
    #[serde(skip)]
    path: PathBuf,
}

/// One full copy's messages in the chat, deltas included.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PostedGroup {
    pub checkpoint: String,
    pub chat_id: i64,
    pub message_ids: Vec<i64>,
}

impl BackupIndex {
    pub fn path(folder: &Path, token: &str, chat_id: i64) -> PathBuf {
        folder.join(format!("telegram-backup-{}-{chat_id}.json", crate::telegram::homes::fingerprint(token)))
    }

    /// The chat's index; a chat without one yet inherits what the open
    /// wallet posted there under the per-wallet scheme.
    pub fn load(folder: &Folder, token: &str, chat_id: i64) -> Self {
        let path = Self::path(&folder.path, token, chat_id);
        let mut index: Self = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        index.path = path;
        if index.posted.is_empty()
            && let Some(open) = folder.open_wallet()
        {
            let mut settings = WalletBackupSettings::load(&open.wallet_dir);
            index.posted = settings.take_legacy_posted(&open.wallet_dir, chat_id);
        }
        index
    }

    pub fn save(&self) -> Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(|e| Error::custom(e.to_string()))?;
        write_json(&self.path, &text)
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    faster_hex::hex_string(&h.finalize())
}

/// What the next post should be.
pub enum Plan {
    Checkpoint,
    Delta { changed: Vec<ArchiveEntry>, removed: Vec<String> },
    Nothing,
}

pub fn plan(index: &BackupIndex, entries: &[ArchiveEntry]) -> Plan {
    let now = now_secs();
    if index.checkpoint.is_empty() || now.saturating_sub(index.checkpoint_at) > CHECKPOINT_EVERY_SECS {
        return Plan::Checkpoint;
    }
    let current: BTreeMap<&str, String> = entries.iter().map(|e| (e.path.as_str(), sha256_hex(&e.data))).collect();
    let changed: Vec<ArchiveEntry> =
        entries.iter().filter(|e| index.files.get(&e.path) != current.get(e.path.as_str())).cloned().collect();
    let removed: Vec<String> = index.files.keys().filter(|p| !current.contains_key(p.as_str())).cloned().collect();
    if changed.is_empty() && removed.is_empty() {
        return Plan::Nothing;
    }
    let delta_bytes: u64 = changed.iter().map(|e| e.data.len() as u64).sum();
    if index.checkpoint_bytes > 0
        && index.delta_bytes + delta_bytes > DELTA_WEIGHT_FLOOR
        && (index.delta_bytes + delta_bytes) as f64 > index.checkpoint_bytes as f64 * DELTA_WEIGHT_LIMIT
    {
        return Plan::Checkpoint;
    }
    Plan::Delta { changed, removed }
}

/// Posts one sealed archive: a plain line saying what it is, then the file
/// (or its parts). No hashes, no markers — the archive is sealed and checked
/// on its own, and the chat should read like a person wrote it.
async fn post(
    token: &str,
    chat_id: i64,
    name: &str,
    what: &str,
    packed: &[u8],
    say: &(dyn Fn(String) + Send + Sync),
) -> Result<Vec<i64>> {
    let parts: Vec<&[u8]> = packed.chunks(BACKUP_PART_BYTES).collect();
    let count = parts.len();
    let in_parts = if count > 1 { format!(", in {count} files") } else { String::new() };
    let mut ids = Vec::with_capacity(count + 1);
    ids.push(
        send_plain(token, chat_id, &format!("{what}{in_parts}. To bring the wallet back on another computer, forward this bot everything from the last dashed line to the end of this chat; each wallet opens with its own 24 words."))
            .await
            .map_err(Error::custom)?,
    );
    for (i, chunk) in parts.iter().enumerate() {
        let index = i + 1;
        let caption = if count > 1 { format!("{what} (part {index} of {count})") } else { what.to_string() };
        ids.push(
            send_document(token, chat_id, &part_file_name(name, index, count), chunk.to_vec(), &caption)
                .await
                .map_err(Error::custom)?,
        );
        say(format!("part {index} of {count} sent ({})", archive::human_size(chunk.len())));
    }
    Ok(ids)
}

/// Keeps the newest `KEEP_CHECKPOINTS` full copies in the chat and takes the
/// older ones down. Telegram refuses deletions older than two days; a copy
/// that old — the wallet was closed for days — stays, and the owner is told.
async fn tidy_old_copies(token: &str, index: &mut BackupIndex, say: &(dyn Fn(String) + Send + Sync)) {
    while index.posted.len() > KEEP_CHECKPOINTS {
        let old = index.posted.remove(0);
        let mut refused = 0usize;
        for id in &old.message_ids {
            if crate::telegram::delete_message(token, old.chat_id, *id).await.is_err() {
                refused += 1;
            }
        }
        if refused == 0 {
            say(format!("the full copy of {} and its changes were taken down from the chat", checkpoint_moment(&old.checkpoint)));
        } else {
            say(format!(
                "the full copy of {} stays in the chat: Telegram lets a bot take messages down for two days only",
                checkpoint_moment(&old.checkpoint)
            ));
        }
    }
}

/// One backup run from the terminal: the open wallet's folder, its words to
/// check its own part by. Returns what was posted, for the line the caller prints.
pub async fn run(cli: &Arc<KaspaCli>, words: &str, force_checkpoint: bool, say: &(dyn Fn(String) + Send + Sync)) -> Result<String> {
    let files = WalletFiles::of(cli).await?;
    let cfg_path = telegram_config_path(cli)?;
    let cfg = TelegramConfig::load(&cfg_path).ok_or_else(|| Error::custom("no Telegram bot is set up for this wallet"))?;
    let folder = Folder::around(&files)?;
    run_folder(&folder, &cfg, Some((&files.name, words)), force_checkpoint, say).await
}

/// The backup run on a folder: the shared core behind the terminal command,
/// the housekeeping tick and the desktop wallet's screen. Decides checkpoint
/// or delta (or that nothing changed), posts it, and brings the chat's index
/// up to date. `force_checkpoint` is the manual command.
pub async fn run_folder(
    folder: &Folder,
    cfg: &TelegramConfig,
    check_words: Option<(&str, &str)>,
    force_checkpoint: bool,
    say: &(dyn Fn(String) + Send + Sync),
) -> Result<String> {
    let chat_id = target_chat(cfg).ok_or_else(|| Error::custom("nowhere to post: pair the bot first"))?;
    if let Some(open) = folder.open_wallet()
        && open.recipient.is_none()
    {
        return Err(Error::custom(format!("the wallet '{}' has no backup key yet — close and open it once", open.name)));
    }
    let mut index = BackupIndex::load(folder, &cfg.token, chat_id);
    let entries = folder.entries()?;
    if entries.is_empty() {
        return Err(Error::custom("no wallet in the folder has a backup key yet"));
    }
    let plan = if force_checkpoint { Plan::Checkpoint } else { plan(&index, &entries) };
    let stamp = chrono::Utc::now().format("c%Y%m%dT%H%M%S").to_string();
    let outcome = match plan {
        Plan::Nothing => return Ok("nothing has changed since the last backup".to_string()),
        Plan::Checkpoint => {
            let (packed, names) = seal_folder(folder, &entries, check_words, None)?;
            let name = checkpoint_name(&names_label(&names), &stamp);
            say(format!(
                "checkpoint {name}: {} files of {}, {}",
                entries.len(),
                wallets_phrase(&names),
                archive::human_size(packed.len())
            ));
            let what =
                format!("Marigold backup of {} on this computer: a full copy, {}", wallets_phrase(&names), checkpoint_moment(&stamp));
            let divider = send_plain(&cfg.token, chat_id, DIVIDER).await.map_err(Error::custom)?;
            let mut ids = vec![divider];
            ids.extend(post(&cfg.token, chat_id, &name, &what, &packed, say).await?);
            index.posted.push(PostedGroup { checkpoint: stamp.clone(), chat_id, message_ids: ids });
            tidy_old_copies(&cfg.token, &mut index, say).await;
            index.checkpoint = stamp;
            index.checkpoint_at = now_secs();
            index.checkpoint_bytes = packed.len() as u64;
            index.delta_seq = 0;
            index.delta_bytes = 0;
            index.files = entries.iter().map(|e| (e.path.clone(), sha256_hex(&e.data))).collect();
            index.wallets = names.clone();
            format!("full copy of {} posted: {} files, {}", wallets_phrase(&names), entries.len(), archive::human_size(packed.len()))
        }
        Plan::Delta { changed, removed } => {
            let seq = index.delta_seq + 1;
            let (packed, names) = seal_folder(folder, &changed, check_words, Some((&index.checkpoint, seq, &removed)))?;
            let label = if index.wallets.is_empty() { names_label(&names) } else { names_label(&index.wallets) };
            let name = delta_name(&label, &index.checkpoint, seq);
            say(format!(
                "delta {seq} of {}: {} file(s) changed, {} removed, {}",
                index.checkpoint,
                changed.len(),
                removed.len(),
                archive::human_size(packed.len())
            ));
            let what = format!(
                "Marigold backup of {}: change {seq} after the full copy of {}",
                wallets_phrase(&names),
                checkpoint_moment(&index.checkpoint)
            );
            let ids = post(&cfg.token, chat_id, &name, &what, &packed, say).await?;
            if let Some(group) = index.posted.last_mut() {
                group.message_ids.extend(ids);
            }
            index.delta_seq = seq;
            index.delta_bytes += packed.len() as u64;
            for e in &changed {
                index.files.insert(e.path.clone(), sha256_hex(&e.data));
            }
            for p in &removed {
                index.files.remove(p);
            }
            for n in names {
                if !index.wallets.contains(&n) {
                    index.wallets.push(n);
                }
            }
            format!("delta {seq} posted: {} file(s), {}", changed.len(), archive::human_size(packed.len()))
        }
    };
    index.last_post_at = now_secs();
    index.gate = folder.gate();
    index.save()?;
    Ok(outcome)
}

pub fn telegram_config_path(cli: &Arc<KaspaCli>) -> Result<PathBuf> {
    let descriptor = cli.store().descriptor().ok_or_else(|| Error::custom("no wallet is open"))?;
    let folder: String = cli
        .wallet()
        .settings()
        .get(WalletSettings::Folder)
        .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
    Ok(TelegramConfig::path(&folder, &descriptor.filename))
}

/// The state the housekeeping tick keeps between calls.
#[derive(Default)]
pub struct AutoState {
    last_check: u64,
    changed_since: u64,
    running: bool,
}

/// Called from housekeeping every few seconds; cheap unless a post is due.
/// A post runs on its own task so housekeeping never waits on Telegram.
pub async fn auto_tick(cli: &Arc<KaspaCli>, state: &Arc<Mutex<AutoState>>) {
    if !cli.wallet().is_open() {
        return;
    }
    let Some(secret) = cli.tidying_secret() else { return };
    let Some(descriptor) = cli.store().descriptor() else { return };
    let Ok(cfg_path) = telegram_config_path(cli) else { return };
    let cli_ = cli.clone();
    let say: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line: String| tprintln!(cli_, "{line}"));
    auto_tick_for(cli.wallet(), cfg_path, descriptor.filename.clone(), secret, state.clone(), say).await;
}

/// The tick for any front end: `cfg_path` is the wallet's Telegram settings
/// file, `say` receives the one line a post produces.
pub async fn auto_tick_for(
    wallet: Arc<Wallet>,
    cfg_path: PathBuf,
    name: String,
    secret: Secret,
    state: Arc<Mutex<AutoState>>,
    say: Arc<dyn Fn(String) + Send + Sync>,
) {
    let now = now_secs();
    {
        let mut st = state.lock().unwrap();
        if st.running || now.saturating_sub(st.last_check) < 60 {
            return;
        }
        st.last_check = now;
    }
    let Some(cfg) = TelegramConfig::load(&cfg_path) else { return };
    let Some(chat_id) = target_chat(&cfg) else { return };
    let Ok(files) = WalletFiles::of_wallet(&wallet, &name).await else { return };
    let Ok(folder) = Folder::around(&files) else { return };
    // Nothing goes anywhere until the owner has asked once: the first
    // 'telegram backup' posts the first checkpoint, and from then on the
    // wallet keeps it current.
    let settings = WalletBackupSettings::load(&files.wallet_dir);
    if settings.paused || !settings.started {
        return;
    }
    let index = BackupIndex::load(&folder, &cfg.token, chat_id);
    let gate = folder.gate();
    let due = {
        let mut st = state.lock().unwrap();
        if gate == index.gate {
            st.changed_since = 0;
            now.saturating_sub(index.checkpoint_at) > CHECKPOINT_EVERY_SECS
        } else {
            if st.changed_since == 0 {
                st.changed_since = now;
            }
            now.saturating_sub(st.changed_since) >= QUIET_SECS && now.saturating_sub(index.last_post_at) >= DELTA_EVERY_SECS
        }
    };
    if !due {
        return;
    }
    state.lock().unwrap().running = true;
    workflow_core::task::spawn(async move {
        let outcome = async {
            let words = words_for(&wallet, &secret).await?;
            let quiet = |_line: String| {};
            run_folder(&folder, &cfg, Some((&name, &words)), false, &quiet).await
        }
        .await;
        match outcome {
            Ok(line) if line.starts_with("nothing") => {}
            Ok(line) => say(crate::ui::dim(format!("Telegram backup: {line}."))),
            Err(err) => say(crate::ui::warn(format!("Telegram backup did not go out: {err}"))),
        }
        state.lock().unwrap().running = false;
    });
}

/// A backup run sealed per wallet, the open wallet's words — which the
/// wallet password unlocks — checking its own part.
pub async fn run_with_words(cli: &Arc<KaspaCli>, secret: &Secret, force_checkpoint: bool) -> Result<String> {
    let words = words_for(&cli.wallet(), secret).await?;
    let quiet = |_line: String| {};
    run(cli, &words, force_checkpoint, &quiet).await
}

/// What a screen or a status line shows about the backups.
#[derive(Debug, Clone, Serialize)]
pub struct BackupStatus {
    /// Where the backups go, in words: "the bot's chat with you", "the group -5181777138", "nowhere yet".
    pub destination: String,
    /// A bot is set up for this wallet.
    pub bot: bool,
    /// The bot has somewhere to post: the chat it was paired in.
    pub paired: bool,
    /// The pairing code to send the bot, while one is live and nobody has paired.
    pub pairing_code: Option<String>,
    /// "on", "off", or "not started".
    pub automatic: String,
    /// The bot is paired, nothing has been backed up, and the one-time
    /// question has not been put: the screen should ask it now.
    pub offer: bool,
    pub checkpoint_at: u64,
    pub checkpoint_bytes: u64,
    pub deltas: u32,
    pub last_post_at: u64,
    /// The wallets a backup covers, the open one among them.
    pub wallets: Vec<String>,
    /// Wallets in the folder still without a backup key: they need their
    /// words typed once, or one open with their password.
    pub uncovered: Vec<String>,
}

pub fn status(files: &WalletFiles, cfg: Option<&TelegramConfig>) -> BackupStatus {
    let settings = WalletBackupSettings::load(&files.wallet_dir);
    let folder = Folder::around(files).ok();
    let index = match (cfg, &folder) {
        (Some(cfg), Some(folder)) => target_chat(cfg).map(|chat| BackupIndex::load(folder, &cfg.token, chat)).unwrap_or_default(),
        _ => BackupIndex::default(),
    };
    let destination = match cfg.map(|c| (c.home_chat_id.or(c.backup_chat_id), target_chat(c))) {
        Some((Some(id), _)) => format!("the group {id}"),
        Some((None, Some(_))) => "the bot's chat with you".to_string(),
        _ => "nowhere yet".to_string(),
    };
    BackupStatus {
        destination,
        bot: cfg.is_some(),
        offer: should_offer(&settings, cfg),
        paired: cfg.is_some_and(|c| target_chat(c).is_some()),
        pairing_code: cfg.and_then(|c| if target_chat(c).is_none() && c.pairing_code_live() { c.pairing_code.clone() } else { None }),
        automatic: if settings.paused {
            "off"
        } else if !settings.started {
            "not started"
        } else {
            "on"
        }
        .to_string(),
        checkpoint_at: index.checkpoint_at,
        checkpoint_bytes: index.checkpoint_bytes,
        deltas: index.delta_seq,
        last_post_at: index.last_post_at,
        wallets: folder.as_ref().map(|f| f.covered().iter().map(|w| w.name.clone()).collect()).unwrap_or_default(),
        uncovered: folder.as_ref().map(|f| f.uncovered()).unwrap_or_default(),
    }
}

/// Whether to put the one-time question now (founder, 2026-09-27: "once the
/// integration is up: do you want to back up your notes automatically to
/// your Telegram robot chat? (highly recommended)").
pub fn should_offer(settings: &WalletBackupSettings, cfg: Option<&TelegramConfig>) -> bool {
    cfg.is_some_and(|c| target_chat(c).is_some()) && !settings.started && !settings.asked && !settings.paused
}

/// The question has been put; it is not put again.
pub fn mark_asked(files: &WalletFiles) -> Result<()> {
    let mut settings = WalletBackupSettings::load(&files.wallet_dir);
    settings.asked = true;
    settings.save(&files.wallet_dir)
}

/// The automatic backups are on from now (the first 'telegram backup').
pub fn mark_started(files: &WalletFiles) -> Result<()> {
    let mut settings = WalletBackupSettings::load(&files.wallet_dir);
    settings.started = true;
    settings.paused = false;
    settings.save(&files.wallet_dir)
}

/// At 'open' in the terminal: the one-time question, and the first checkpoint on a yes.
pub async fn offer_at_open(cli: &Arc<KaspaCli>) {
    let Some(secret) = cli.tidying_secret() else { return };
    let Some(descriptor) = cli.store().descriptor() else { return };
    let Ok(cfg_path) = telegram_config_path(cli) else { return };
    let cfg = TelegramConfig::load(&cfg_path);
    let Ok(files) = WalletFiles::of_wallet(&cli.wallet(), &descriptor.filename).await else { return };
    let settings = WalletBackupSettings::load(&files.wallet_dir);
    if !should_offer(&settings, cfg.as_ref()) {
        return;
    }
    tprintln!(cli, "");
    tprintln!(
        cli,
        "Your Telegram bot is paired. The wallet can keep an encrypted copy of every wallet on this computer in that chat by itself — a full copy every day, the changes within minutes of a payment, silently, the last two copies kept — each wallet sealed with its own 24 words."
    );
    let answer = match cli
        .term()
        .ask(false, "Back up your wallets automatically to your Telegram bot chat? (highly recommended) [Y/n]: ")
        .await
    {
        Ok(a) => a.trim().to_lowercase(),
        Err(_) => return,
    };
    let _ = mark_asked(&files);
    if answer.starts_with('n') {
        tprintln!(cli, "{}", crate::ui::dim("Run 'telegram backup' later if you change your mind."));
        tprintln!(cli, "");
        return;
    }
    tprintln!(cli, "Posting the first full copy…");
    let _ = mark_started(&files);
    match run_with_words(cli, &secret, true).await {
        Ok(line) => {
            tprintln!(
                cli,
                "{}",
                crate::ui::dim(format!("Telegram backup: {line}. From now on it stays current by itself while the wallet is open."))
            );
        }
        Err(err) => {
            tprintln!(cli, "{}", crate::ui::warn(format!("The backup did not go out: {err} — 'telegram backup' tries again.")))
        }
    }
    tprintln!(cli, "");
}

/// At 'open' in the terminal, after the offer: wallets in the folder that
/// have no backup key yet (made before 2026-10-06 and not opened since), and
/// what to do about them. Only when the backups have somewhere to go.
pub async fn cover_at_open(cli: &Arc<KaspaCli>) {
    let Some(descriptor) = cli.store().descriptor() else { return };
    let Ok(cfg_path) = telegram_config_path(cli) else { return };
    let Some(cfg) = TelegramConfig::load(&cfg_path) else { return };
    if target_chat(&cfg).is_none() {
        return;
    }
    let Ok(files) = WalletFiles::of_wallet(&cli.wallet(), &descriptor.filename).await else { return };
    let Ok(folder) = Folder::around(&files) else { return };
    if folder.uncovered().is_empty() {
        return;
    }
    cover_wizard(cli, &folder, false).await;
}

/// The wizard proper: explains the change once, then asks per wallet.
/// `all`: include the wallets marked 'never' ('telegram cover').
pub async fn cover_wizard(cli: &Arc<KaspaCli>, folder: &Folder, all: bool) {
    let waiting: Vec<&WalletFiles> = folder
        .wallets
        .iter()
        .filter(|w| w.recipient.is_none() && (all || !w.skipped()) && Some(&w.name) != folder.open.as_ref())
        .collect();
    if waiting.is_empty() {
        if all {
            tprintln!(cli, "Every wallet in {} is covered.", folder.path.display());
        }
        return;
    }
    let names: Vec<String> = waiting.iter().map(|w| w.name.clone()).collect();
    tprintln!(cli, "");
    tprintln!(
        cli,
        "The backups now cover every wallet on this computer, each sealed with its own 24 words. Since this change, {} no backup key yet: {}.",
        if names.len() == 1 { "one wallet has" } else { "these wallets have" },
        names.join(", ")
    );
    tprintln!(
        cli,
        "{}",
        crate::ui::dim(
            "A wallet gets its key the first time it is opened with its password — or type its 24 words now, once; they are checked against the wallet and nothing is kept but a public key. Enter skips it for now; 'never' leaves it out of the backups ('telegram cover' asks again)."
        )
    );
    for wallet in waiting {
        tprintln!(cli, "");
        let answer =
            match cli.term().ask(true, &format!("The 24 words of '{}' (Enter to skip, 'never' to leave it out): ", wallet.name)).await
            {
                Ok(a) => a.trim().to_string(),
                Err(_) => return,
            };
        if answer.is_empty() || matches!(answer.to_lowercase().as_str(), "n" | "no" | "skip" | "later") {
            tprintln!(cli, "{}", crate::ui::dim(format!("'{}' is covered once you open it.", wallet.name)));
            continue;
        }
        if answer.eq_ignore_ascii_case("never") {
            match skip(folder, &wallet.name) {
                Ok(()) => tprintln!(cli, "{}", crate::ui::dim(format!("'{}' is left out of the backups.", wallet.name))),
                Err(err) => tprintln!(cli, "{}", crate::ui::warn(err.to_string())),
            }
            continue;
        }
        match cover(folder, &wallet.name, &answer) {
            Ok(()) => tprintln!(cli, "{}", style(format!("'{}' is covered from the next backup on.", wallet.name)).green()),
            Err(err) => tprintln!(cli, "{}", crate::ui::warn(format!("'{}' is not covered: {err}", wallet.name))),
        }
    }
    tprintln!(cli, "");
}

pub fn set_paused(files: &WalletFiles, paused: bool) -> Result<()> {
    let mut settings = WalletBackupSettings::load(&files.wallet_dir);
    settings.paused = paused;
    settings.save(&files.wallet_dir)
}

/// At 'close': a change not yet posted goes out now rather than at the next open.
pub async fn flush_before_close(cli: &Arc<KaspaCli>) {
    let Some(secret) = cli.tidying_secret() else { return };
    let Some(descriptor) = cli.store().descriptor() else { return };
    let Ok(cfg_path) = telegram_config_path(cli) else { return };
    let say = |line: String| tprintln!(cli, "{line}");
    flush_for(&cli.wallet(), &cfg_path, &descriptor.filename, &secret, &say).await;
}

/// The flush for any front end; `say` receives what happened, or nothing
/// when there was nothing to post.
pub async fn flush_for(wallet: &Arc<Wallet>, cfg_path: &Path, name: &str, secret: &Secret, say: &(dyn Fn(String) + Send + Sync)) {
    let Some(cfg) = TelegramConfig::load(cfg_path) else { return };
    let Some(chat_id) = target_chat(&cfg) else { return };
    let Ok(files) = WalletFiles::of_wallet(wallet, name).await else { return };
    let Ok(folder) = Folder::around(&files) else { return };
    let settings = WalletBackupSettings::load(&files.wallet_dir);
    let index = BackupIndex::load(&folder, &cfg.token, chat_id);
    if settings.paused || !settings.started || index.gate == folder.gate() {
        return;
    }
    say(crate::ui::dim("Backing up the latest changes to Telegram before closing…"));
    let outcome = async {
        let words = words_for(wallet, secret).await?;
        let quiet = |_line: String| {};
        run_folder(&folder, &cfg, Some((name, &words)), false, &quiet).await
    }
    .await;
    match outcome {
        Ok(line) => say(crate::ui::dim(format!("Telegram backup: {line}."))),
        Err(err) => say(crate::ui::warn(format!("Telegram backup did not go out: {err}"))),
    }
}
