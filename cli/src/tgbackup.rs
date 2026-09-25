//! Automatic backups to Telegram, checkpoint and delta (founder, 2026-09-24):
//! "like an Apple Cloud backup of an iPhone … always up to date", and "we never
//! have to forward more than a week's worth back to the wallet".
//!
//! After the first `backup telegram`, the wallet keeps its backup current by
//! itself while it is open — in the bot's own chat with its owner, the same
//! chat the payment codes arrive in (founder, 2026-09-24: "the person will
//! already have created the bot, so why not send the message direct"), or in
//! a private group if one was given: a *checkpoint* — every file —
//! once a week or when the deltas have grown past half its size, and a *delta*
//! — only the files changed since the last post, plus the names of any removed
//! — whenever the vault has changed and been quiet for two minutes, at most
//! every ten minutes. Everything is encrypted under a key derived from the
//! wallet's 24 words, so nothing has to be asked and the words, which the owner
//! keeps anyway, open it all. A restore takes the latest checkpoint and the
//! deltas after it, forwarded to the bot in any order.
//!
//! The wallet remembers what it last posted in `telegram-backup.json` beside
//! the wallet (paths and digests), which is how a delta knows what changed.

use crate::backup::{self as archive, ArchiveEntry, key_from_words};
use crate::cli::KaspaCli;
use crate::imports::*;
use crate::telegram::{BACKUP_PART_BYTES, TelegramConfig, part_file_name, send_document, send_plain};
use kaspa_wallet_core::wallet::Wallet;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A checkpoint at least this often.
pub const CHECKPOINT_EVERY_SECS: u64 = 7 * 24 * 3600;
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
/// The entry inside a delta that says what it is.
pub const DELTA_NOTE: &str = "__marigold_delta__.json";

/// Where backups go: the group if one was set, else the chat the bot was
/// paired in.
pub fn target_chat(cfg: &TelegramConfig) -> Option<i64> {
    cfg.backup_chat_id.or(cfg.chat_id)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What the wallet last posted: which checkpoint, how many deltas after it,
/// and every file's digest as of the last post.
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
    /// path → sha256 hex, as of the last post.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// The cheap change gate: the manifest's and the keys file's size and
    /// modification time when the index was last brought up to date.
    #[serde(default)]
    pub gate: String,
    /// Set by 'backup telegram off'.
    #[serde(default)]
    pub paused: bool,
}

impl BackupIndex {
    pub fn path(wallet_dir: &Path) -> PathBuf {
        wallet_dir.join("telegram-backup.json")
    }

    pub fn load(wallet_dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(wallet_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, wallet_dir: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(|e| Error::custom(e.to_string()))?;
        let path = Self::path(wallet_dir);
        let tmp = path.with_extension("json.tmp");
        archive::write_owner_only(&tmp, text.as_bytes()).map_err(|e| Error::custom(format!("cannot write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| Error::custom(format!("cannot write {}: {e}", path.display())))?;
        Ok(())
    }
}

/// The wallet's whereabouts on disk, for one backup run.
pub struct WalletFiles {
    pub name: String,
    pub wallet_dir: PathBuf,
    pub wallet_file: PathBuf,
    pub vault_folder: PathBuf,
}

impl WalletFiles {
    /// The open wallet's files, for the terminal wallet.
    pub async fn of(cli: &Arc<KaspaCli>) -> Result<Self> {
        let descriptor = cli.store().descriptor().ok_or_else(|| Error::custom("no wallet is open"))?;
        Self::of_wallet(&cli.wallet(), &descriptor.filename).await
    }

    /// The open wallet's files, for whichever front end holds the wallet —
    /// the terminal, the desktop app, the bot's service.
    pub async fn of_wallet(wallet: &Arc<Wallet>, name: &str) -> Result<Self> {
        let vault_folder = wallet.store().as_note_key_store()?.vault_folder().await?;
        let wallet_dir = vault_folder.parent().ok_or_else(|| Error::custom("cannot work out the wallet folder"))?.to_path_buf();
        let wallet_file = wallet_dir.join(kaspa_wallet_core::storage::local::keys_file_name(name));
        if !wallet_file.exists() {
            return Err(Error::custom(format!("{} is missing — nothing to back up", wallet_file.display())));
        }
        Ok(Self { name: name.to_string(), wallet_dir, wallet_file, vault_folder })
    }

    /// Every file of the wallet as archive entries, paths as a backup names them.
    pub fn entries(&self) -> Result<Vec<ArchiveEntry>> {
        let dir = kaspa_wallet_core::storage::local::wallet_dir_name(&self.name);
        let mut entries = vec![ArchiveEntry {
            path: format!("{dir}/{}", kaspa_wallet_core::storage::local::keys_file_name(&self.name)),
            data: std::fs::read(&self.wallet_file).map_err(|e| Error::custom(format!("cannot read the wallet file: {e}")))?,
        }];
        if self.vault_folder.exists() {
            archive::collect_tree(&self.vault_folder, &format!("{dir}/notes"), &mut entries)?;
        }
        Ok(entries)
    }

    /// The cheap change gate: sizes and modification times of the two files
    /// that change whenever anything does.
    pub fn gate(&self) -> String {
        let stamp = |p: &Path| {
            std::fs::metadata(p)
                .ok()
                .map(|m| {
                    format!(
                        "{}:{}",
                        m.len(),
                        m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
                    )
                })
                .unwrap_or_default()
        };
        format!("{}|{}", stamp(&self.wallet_file), stamp(&self.vault_folder.join("manifest.tsv")))
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

#[derive(Serialize, Deserialize)]
struct DeltaNote {
    checkpoint: String,
    seq: u32,
    removed: Vec<String>,
}

/// The names: `marigold-<wallet>-<checkpoint>.full.mgb` and
/// `marigold-<wallet>-<checkpoint>.d<seq>.mgb`, the part suffix after.
pub fn checkpoint_name(wallet: &str, checkpoint: &str) -> String {
    format!("marigold-{wallet}-{checkpoint}.full.mgb")
}

pub fn delta_name(wallet: &str, checkpoint: &str, seq: u32) -> String {
    format!("marigold-{wallet}-{checkpoint}.d{seq:03}.mgb")
}

/// Which backup a name is: (checkpoint stamp, None for the checkpoint itself
/// or Some(seq) for a delta). A name from before checkpoints — one
/// passphrase-sealed archive, `marigold-<wallet>-<time>.mgb` — counts as a
/// checkpoint of its own, named by its whole stem.
pub fn parse_backup_name(name: &str) -> Option<(String, Option<u32>)> {
    let stem = name.strip_suffix(".mgb")?;
    let stamp_of = |prefix: &str| prefix.rsplit_once("-c").map(|(_, digits)| format!("c{digits}"));
    if let Some(prefix) = stem.strip_suffix(".full") {
        return Some((stamp_of(prefix)?, None));
    }
    if let Some((prefix, d)) = stem.rsplit_once(".d")
        && let Ok(seq) = d.parse::<u32>()
        && let Some(stamp) = stamp_of(prefix)
    {
        return Some((stamp, Some(seq)));
    }
    Some((stem.to_string(), None))
}

/// Newer checkpoints sort later; a pre-checkpoint archive sorts before any checkpoint.
fn checkpoint_order(stamp: &str) -> (bool, String) {
    (stamp.starts_with('c') && stamp[1..].chars().all(|c| c.is_ascii_digit() || c == 'T'), stamp.to_string())
}

/// Posts one sealed archive as start message, parts and end message.
async fn post(token: &str, chat_id: i64, name: &str, packed: &[u8], say: &(dyn Fn(String) + Send + Sync)) -> Result<()> {
    let digest = sha256_hex(packed);
    let parts: Vec<&[u8]> = packed.chunks(BACKUP_PART_BYTES).collect();
    let count = parts.len();
    send_plain(token, chat_id, &format!("----- Marigold backup {name}: {count} part(s), {} bytes, sha256 {digest}", packed.len()))
        .await
        .map_err(Error::custom)?;
    for (i, chunk) in parts.iter().enumerate() {
        let index = i + 1;
        send_document(
            token,
            chat_id,
            &part_file_name(name, index, count),
            chunk.to_vec(),
            &format!("Part {index} of {count} of {name} · sha256 {}…", &digest[..16]),
        )
        .await
        .map_err(Error::custom)?;
        say(format!("part {index} of {count} sent ({})", archive::human_size(chunk.len())));
    }
    send_plain(token, chat_id, &format!("----- End of Marigold backup {name}")).await.map_err(Error::custom)?;
    Ok(())
}

/// One backup run: decides checkpoint or delta (or that nothing changed),
/// posts it, and brings the index up to date. `force_checkpoint` is the manual
/// command. Returns what was posted, for the line the caller prints.
pub async fn run(cli: &Arc<KaspaCli>, key: &Secret, force_checkpoint: bool, say: &(dyn Fn(String) + Send + Sync)) -> Result<String> {
    let files = WalletFiles::of(cli).await?;
    let cfg_path = telegram_config_path(cli)?;
    let cfg = TelegramConfig::load(&cfg_path).ok_or_else(|| Error::custom("no Telegram bot is set up for this wallet"))?;
    run_files(&files, &cfg, key, force_checkpoint, say).await
}

/// The backup run on a wallet's files: the shared core behind the terminal
/// command, the housekeeping tick and the desktop wallet's screen.
pub async fn run_files(
    files: &WalletFiles,
    cfg: &TelegramConfig,
    key: &Secret,
    force_checkpoint: bool,
    say: &(dyn Fn(String) + Send + Sync),
) -> Result<String> {
    let chat_id = target_chat(cfg).ok_or_else(|| Error::custom("nowhere to post: pair the bot first, or give a group id"))?;
    let mut index = BackupIndex::load(&files.wallet_dir);
    let entries = files.entries()?;
    let plan = if force_checkpoint { Plan::Checkpoint } else { plan(&index, &entries) };
    let stamp = chrono::Utc::now().format("c%Y%m%dT%H%M%S").to_string();
    let outcome = match plan {
        Plan::Nothing => return Ok("nothing has changed since the last backup".to_string()),
        Plan::Checkpoint => {
            let name = checkpoint_name(&files.name, &stamp);
            let packed = archive::pack(&entries, key)?;
            archive::unpack(&packed, key)?;
            say(format!("checkpoint {name}: {} files, {}", entries.len(), archive::human_size(packed.len())));
            post(&cfg.token, chat_id, &name, &packed, say).await?;
            index.checkpoint = stamp;
            index.checkpoint_at = now_secs();
            index.checkpoint_bytes = packed.len() as u64;
            index.delta_seq = 0;
            index.delta_bytes = 0;
            index.files = entries.iter().map(|e| (e.path.clone(), sha256_hex(&e.data))).collect();
            format!("checkpoint posted: {} files, {}", entries.len(), archive::human_size(packed.len()))
        }
        Plan::Delta { changed, removed } => {
            let seq = index.delta_seq + 1;
            let name = delta_name(&files.name, &index.checkpoint, seq);
            let mut delta_entries = changed.clone();
            let note = DeltaNote { checkpoint: index.checkpoint.clone(), seq, removed: removed.clone() };
            delta_entries.push(ArchiveEntry {
                path: DELTA_NOTE.to_string(),
                data: serde_json::to_vec(&note).map_err(|e| Error::custom(e.to_string()))?,
            });
            let packed = archive::pack(&delta_entries, key)?;
            say(format!(
                "delta {seq} of {}: {} file(s) changed, {} removed, {}",
                index.checkpoint,
                changed.len(),
                removed.len(),
                archive::human_size(packed.len())
            ));
            post(&cfg.token, chat_id, &name, &packed, say).await?;
            index.delta_seq = seq;
            index.delta_bytes += packed.len() as u64;
            for e in &changed {
                index.files.insert(e.path.clone(), sha256_hex(&e.data));
            }
            for p in &removed {
                index.files.remove(p);
            }
            format!("delta {seq} posted: {} file(s), {}", changed.len(), archive::human_size(packed.len()))
        }
    };
    index.last_post_at = now_secs();
    index.gate = files.gate();
    index.save(&files.wallet_dir)?;
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
    if target_chat(&cfg).is_none() {
        return;
    }
    let Ok(files) = WalletFiles::of_wallet(&wallet, &name).await else { return };
    let index = BackupIndex::load(&files.wallet_dir);
    // Nothing goes anywhere until the owner has asked once: the first
    // 'backup telegram' posts the first checkpoint, and from then on the
    // wallet keeps it current.
    if index.paused || index.checkpoint.is_empty() {
        return;
    }
    let gate = files.gate();
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
            let key = key_for(&wallet, &secret).await?;
            let quiet = |_line: String| {};
            run_files(&files, &cfg, &key, false, &quiet).await
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

/// The backup key of an open wallet: its 24 words, read from the vault under
/// the wallet password, through `key_from_words`.
pub async fn key_for(wallet: &Arc<Wallet>, secret: &Secret) -> Result<Secret> {
    let store = wallet.store().as_note_key_store()?;
    let words = store.recovery_words(secret).await?;
    Ok(key_from_words(&words))
}

/// A backup run sealed under the wallet's words, which the wallet password unlocks.
pub async fn run_with_words(cli: &Arc<KaspaCli>, secret: &Secret, force_checkpoint: bool) -> Result<String> {
    let key = key_for(&cli.wallet(), secret).await?;
    let quiet = |_line: String| {};
    run(cli, &key, force_checkpoint, &quiet).await
}

/// What a screen or a status line shows about the backups.
#[derive(Debug, Clone, Serialize)]
pub struct BackupStatus {
    /// Where the backups go, in words: "the bot's chat with you", "the group -5181777138", "nowhere yet".
    pub destination: String,
    /// A bot is set up for this wallet.
    pub bot: bool,
    /// The bot has been paired with its owner's Telegram account.
    pub paired: bool,
    /// The pairing code to send the bot, while one is live and nobody has paired.
    pub pairing_code: Option<String>,
    /// "on", "off", or "not started".
    pub automatic: String,
    pub checkpoint_at: u64,
    pub checkpoint_bytes: u64,
    pub deltas: u32,
    pub last_post_at: u64,
}

pub fn status(files: &WalletFiles, cfg: Option<&TelegramConfig>) -> BackupStatus {
    let index = BackupIndex::load(&files.wallet_dir);
    let destination = match cfg.map(|c| (c.backup_chat_id, c.chat_id)) {
        Some((Some(id), _)) => format!("the group {id}"),
        Some((None, Some(_))) => "the bot's chat with you".to_string(),
        _ => "nowhere yet".to_string(),
    };
    BackupStatus {
        destination,
        bot: cfg.is_some(),
        paired: cfg.is_some_and(|c| c.user_id.is_some()),
        pairing_code: cfg.and_then(|c| if c.user_id.is_none() && c.pairing_code_live() { c.pairing_code.clone() } else { None }),
        automatic: if index.paused {
            "off"
        } else if index.checkpoint.is_empty() {
            "not started"
        } else {
            "on"
        }
        .to_string(),
        checkpoint_at: index.checkpoint_at,
        checkpoint_bytes: index.checkpoint_bytes,
        deltas: index.delta_seq,
        last_post_at: index.last_post_at,
    }
}

pub fn set_paused(files: &WalletFiles, paused: bool) -> Result<()> {
    let mut index = BackupIndex::load(&files.wallet_dir);
    index.paused = paused;
    index.save(&files.wallet_dir)
}

/// The sealed archive of a wallet for a file backup, checked to read back.
pub fn pack_checked(files: &WalletFiles, key: &Secret) -> Result<(Vec<ArchiveEntry>, Vec<u8>)> {
    let entries = files.entries()?;
    let packed = archive::pack(&entries, key)?;
    let restored = archive::unpack(&packed, key)?;
    if restored.len() != entries.len() || entries.iter().zip(restored.iter()).any(|(a, b)| a.path != b.path || a.data != b.data) {
        return Err(Error::custom("the backup did not read back correctly — do not rely on it"));
    }
    Ok((entries, packed))
}

/// What a restore put in place.
pub struct Restored {
    pub written: usize,
    /// The name the backup carried.
    pub original: String,
    /// The name the files have now.
    pub name: String,
}

/// Puts decrypted backup entries in place under `folder` — under `new_name`
/// if given — and marks the wallet for key rotation on its first open. The
/// shared tail of every restore; the terminal adds its questions around it.
pub fn install_restored(entries: Vec<ArchiveEntry>, folder: &Path, new_name: Option<String>) -> Result<Restored> {
    let original = archive::wallet_name_in(&entries)?;
    let name = new_name.unwrap_or_else(|| original.clone());
    if name.to_lowercase() == "wallet" {
        return Err(Error::custom("a wallet cannot be named 'wallet'"));
    }
    let entries = if name == original { entries } else { archive::rename_entries(entries, &original, &name)? };
    let written = archive::extract(&entries, folder)?;
    // A backup is a copy of the keys, and any other copy of it can spend the
    // same notes. The first open of the restored wallet rotates every note to
    // fresh keys (POOL-SPEC.md P5.6), which needs the wallet open and a node:
    // this marker asks for it (threat pass, 2026-09-20).
    let marker = folder.join(kaspa_wallet_core::storage::local::wallet_dir_name(&name)).join("notes").join(archive::ROTATE_ON_OPEN);
    if let Some(dir) = marker.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&marker, b"restored from a backup; rotate every note on the first open\n").ok();
    Ok(Restored { written, original, name })
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
    if target_chat(&cfg).is_none() {
        return;
    }
    let Ok(files) = WalletFiles::of_wallet(wallet, name).await else { return };
    let index = BackupIndex::load(&files.wallet_dir);
    if index.paused || index.checkpoint.is_empty() || index.gate == files.gate() {
        return;
    }
    say(crate::ui::dim("Backing up the latest changes to Telegram before closing…"));
    let outcome = async {
        let key = key_for(wallet, secret).await?;
        let quiet = |_line: String| {};
        run_files(&files, &cfg, &key, false, &quiet).await
    }
    .await;
    match outcome {
        Ok(line) => say(crate::ui::dim(format!("Telegram backup: {line}."))),
        Err(err) => say(crate::ui::warn(format!("Telegram backup did not go out: {err}"))),
    }
}

/// Merges a checkpoint and the deltas after it into one set of files.
pub fn merge(collected: &BTreeMap<String, Vec<u8>>, key: &Secret) -> Result<(Vec<ArchiveEntry>, String, u32)> {
    let mut checkpoint: Option<(String, Vec<ArchiveEntry>)> = None;
    let mut deltas: BTreeMap<u32, Vec<ArchiveEntry>> = BTreeMap::new();
    let mut latest_stamp = String::new();
    // The newest checkpoint among those forwarded is the one restored.
    for name in collected.keys() {
        if let Some((stamp, None)) = parse_backup_name(name)
            && (latest_stamp.is_empty() || checkpoint_order(&stamp) > checkpoint_order(&latest_stamp))
        {
            latest_stamp = stamp;
        }
    }
    if latest_stamp.is_empty() {
        return Err(Error::custom(
            "no checkpoint among the forwarded messages — forward the newest 'full' backup and the deltas after it",
        ));
    }
    for (name, bytes) in collected {
        let Some((stamp, seq)) = parse_backup_name(name) else { continue };
        if stamp != latest_stamp {
            continue;
        }
        let entries = archive::unpack(bytes, key)?;
        match seq {
            None => checkpoint = Some((stamp, entries)),
            Some(seq) => {
                deltas.insert(seq, entries);
            }
        }
    }
    let (stamp, mut files) = checkpoint.ok_or_else(|| Error::custom("the checkpoint could not be read"))?;
    let mut by_path: BTreeMap<String, Vec<u8>> = files.drain(..).map(|e| (e.path, e.data)).collect();
    let mut applied = 0u32;
    for (seq, entries) in deltas {
        if seq != applied + 1 {
            return Err(Error::custom(format!(
                "delta {} of checkpoint {stamp} is missing — forward it too (deltas {} to {} were found)",
                applied + 1,
                seq,
                seq
            )));
        }
        for entry in entries {
            if entry.path == DELTA_NOTE {
                if let Ok(note) = serde_json::from_slice::<DeltaNote>(&entry.data) {
                    for removed in note.removed {
                        by_path.remove(&removed);
                    }
                }
                continue;
            }
            by_path.insert(entry.path, entry.data);
        }
        applied = seq;
    }
    Ok((by_path.into_iter().map(|(path, data)| ArchiveEntry { path, data }).collect(), stamp, applied))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names_parse() {
        assert_eq!(parse_backup_name("marigold-test10-c20260924T100000.full.mgb"), Some(("c20260924T100000".to_string(), None)));
        assert_eq!(parse_backup_name("marigold-my-wallet-c20260924T100000.d007.mgb"), Some(("c20260924T100000".to_string(), Some(7))));
        assert_eq!(
            parse_backup_name("marigold-test10-2026-09-23T20-10-01.mgb"),
            Some(("marigold-test10-2026-09-23T20-10-01".to_string(), None))
        );
        assert_eq!(parse_backup_name("notes.txt"), None);
        assert!(checkpoint_order("c20260924T100000") > checkpoint_order("marigold-test10-2026-09-23T20-10-01"));
        assert!(checkpoint_order("c20260925T000000") > checkpoint_order("c20260924T100000"));
    }

    #[test]
    fn checkpoint_then_deltas_merge() {
        let key = key_from_words(
            "abandon ability able about above absent absorb abstract absurd abuse access accident account accuse achieve acid acoustic acquire across act action actor actress actual",
        );
        let full = vec![
            ArchiveEntry { path: "w/keys".into(), data: b"k1".to_vec() },
            ArchiveEntry { path: "w/notes/a.note".into(), data: b"a".to_vec() },
            ArchiveEntry { path: "w/notes/b.note".into(), data: b"b".to_vec() },
        ];
        let d1 = vec![
            ArchiveEntry { path: "w/notes/c.note".into(), data: b"c".to_vec() },
            ArchiveEntry {
                path: DELTA_NOTE.into(),
                data: serde_json::to_vec(&DeltaNote { checkpoint: "c1".into(), seq: 1, removed: vec!["w/notes/a.note".into()] })
                    .unwrap(),
            },
        ];
        let d2 = vec![
            ArchiveEntry { path: "w/notes/b.note".into(), data: b"b2".to_vec() },
            ArchiveEntry {
                path: DELTA_NOTE.into(),
                data: serde_json::to_vec(&DeltaNote { checkpoint: "c1".into(), seq: 2, removed: vec![] }).unwrap(),
            },
        ];
        let mut collected = BTreeMap::new();
        collected.insert(checkpoint_name("w", "c20260924T100000"), archive::pack(&full, &key).unwrap());
        collected.insert(delta_name("w", "c20260924T100000", 1), archive::pack(&d1, &key).unwrap());
        collected.insert(delta_name("w", "c20260924T100000", 2), archive::pack(&d2, &key).unwrap());
        let (entries, stamp, applied) = merge(&collected, &key).unwrap();
        assert_eq!(stamp, "c20260924T100000");
        assert_eq!(applied, 2);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["w/keys", "w/notes/b.note", "w/notes/c.note"]);
        assert_eq!(entries[1].data, b"b2");

        collected.remove(&delta_name("w", "c20260924T100000", 1));
        assert!(merge(&collected, &key).is_err());
    }
}
