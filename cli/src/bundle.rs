//! Every wallet in the folder in one backup, each sealed to its own words.
//!
//! The part of the backups that needs no Telegram: which wallets a folder
//! holds and which have a backup key, their files as archive entries, the
//! sealed bundle (`backup::seal_to` per wallet), the names the files carry,
//! and the merge of a checkpoint with the deltas after it at a restore. The
//! file backup command and the desktop app's restore use this directly;
//! `tgbackup` posts what it makes.

use crate::backup::{self as archive, ArchiveEntry, BundleItem};
use crate::cli::KaspaCli;
use crate::imports::*;
use kaspa_bip32::secp256k1;
use kaspa_wallet_core::wallet::Wallet;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The entry inside a delta that says what it is.
pub const DELTA_NOTE: &str = "__marigold_delta__.json";

/// A wallet's whereabouts on disk, for one backup run.
pub struct WalletFiles {
    pub name: String,
    pub wallet_dir: PathBuf,
    pub wallet_file: PathBuf,
    pub vault_folder: PathBuf,
    /// The public half of its backup key, once it has one.
    pub recipient: Option<secp256k1::PublicKey>,
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
        Self::in_dir(wallet_dir, name)
    }

    /// A wallet's files by its directory, open or not.
    pub fn in_dir(wallet_dir: PathBuf, name: &str) -> Result<Self> {
        let wallet_file = wallet_dir.join(kaspa_wallet_core::storage::local::keys_file_name(name));
        if !wallet_file.exists() {
            return Err(Error::custom(format!("{} is missing — nothing to back up", wallet_file.display())));
        }
        let vault_folder = wallet_dir.join("notes");
        let recipient = archive::read_recipient(&wallet_dir);
        Ok(Self { name: name.to_string(), wallet_dir, wallet_file, vault_folder, recipient })
    }

    /// The folder the wallet sits in.
    pub fn folder(&self) -> PathBuf {
        self.wallet_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| self.wallet_dir.clone())
    }

    /// Every file of the wallet as archive entries, paths as a backup names them.
    pub fn entries(&self) -> Result<Vec<ArchiveEntry>> {
        let dir = kaspa_wallet_core::storage::local::wallet_dir_name(&self.name);
        let mut entries = vec![ArchiveEntry {
            path: format!("{dir}/{}", kaspa_wallet_core::storage::local::keys_file_name(&self.name)),
            data: std::fs::read(&self.wallet_file).map_err(|e| Error::custom(format!("cannot read the wallet file: {e}")))?,
        }];
        // The public half travels with the wallet, so a restored copy is
        // covered by the next backup without being opened first.
        if let Ok(data) = std::fs::read(archive::recipient_pub_path(&self.wallet_dir)) {
            entries.push(ArchiveEntry { path: format!("{dir}/{}", archive::RECIPIENT_FILE), data });
        }
        if self.vault_folder.exists() {
            archive::collect_tree(&self.vault_folder, &format!("{dir}/notes"), &mut entries)?;
        }
        Ok(entries)
    }

    /// Every file of the wallet as the folder lists it — path, where it is,
    /// size and modification time — without reading any. A delta reads only
    /// what this says has changed; a wallet of sixty thousand notes was read
    /// whole for every post (tester on Windows, 2026-10-08: 22 minutes).
    pub fn listing(&self) -> Result<Vec<Listed>> {
        let dir = kaspa_wallet_core::storage::local::wallet_dir_name(&self.name);
        let mut out = vec![Listed::of(
            format!("{dir}/{}", kaspa_wallet_core::storage::local::keys_file_name(&self.name)),
            self.wallet_file.clone(),
        )];
        let public = archive::recipient_pub_path(&self.wallet_dir);
        if public.exists() {
            out.push(Listed::of(format!("{dir}/{}", archive::RECIPIENT_FILE), public));
        }
        if self.vault_folder.exists() {
            list_tree(&self.vault_folder, &format!("{dir}/notes"), &mut out)?;
        }
        Ok(out)
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

    /// The wallet's owner has said not to ask for this wallet's words again.
    pub fn skipped(&self) -> bool {
        self.wallet_dir.join(SKIP_FILE).exists()
    }
}

/// The marker 'never' leaves in a wallet that is not to be covered.
pub const SKIP_FILE: &str = "backup.skip";

/// A file as the folder lists it, before it is read.
#[derive(Clone)]
pub struct Listed {
    /// Relative, `/`-separated, as a backup names it.
    pub path: String,
    pub file: PathBuf,
    /// `size:mtime` — what says "unchanged" without a read.
    pub stamp: String,
}

impl Listed {
    fn of(path: String, file: PathBuf) -> Self {
        let stamp = stamp_of(&file);
        Self { path, file, stamp }
    }
}

fn stamp_of(file: &Path) -> String {
    std::fs::metadata(file)
        .ok()
        .map(|m| {
            let modified =
                m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
            format!("{}:{modified}", m.len())
        })
        .unwrap_or_default()
}

/// Like `backup::collect_tree`, without the reading.
fn list_tree(root: &Path, prefix: &str, out: &mut Vec<Listed>) -> Result<()> {
    let mut dirs = vec![(root.to_path_buf(), prefix.to_string())];
    while let Some((dir, rel)) = dirs.pop() {
        let listing = std::fs::read_dir(&dir).map_err(|e| Error::custom(format!("cannot read {}: {e}", dir.display())))?;
        for entry in listing {
            let entry = entry.map_err(|e| Error::custom(format!("cannot read {}: {e}", dir.display())))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let meta = entry.path().metadata().map_err(|e| Error::custom(format!("cannot read {}: {e}", entry.path().display())))?;
            if meta.is_dir() {
                dirs.push((entry.path(), child_rel));
            } else {
                out.push(Listed::of(child_rel, entry.path()));
            }
        }
    }
    Ok(())
}

/// Reads listed files into entries.
pub fn read_listed(listed: &[Listed]) -> Result<Vec<ArchiveEntry>> {
    listed
        .iter()
        .map(|l| {
            let data = std::fs::read(&l.file).map_err(|e| Error::custom(format!("cannot read {}: {e}", l.file.display())))?;
            Ok(ArchiveEntry { path: l.path.clone(), data })
        })
        .collect()
}

/// Every wallet in a folder, for one backup run.
pub struct Folder {
    pub path: PathBuf,
    /// Sorted by name; every directory with a keys file inside.
    pub wallets: Vec<WalletFiles>,
    /// The wallet that is open, when one is.
    pub open: Option<String>,
}

impl Folder {
    /// The folder around the open wallet.
    pub fn around(files: &WalletFiles) -> Result<Self> {
        let mut folder = Self::scan(&files.folder())?;
        folder.open = Some(files.name.clone());
        Ok(folder)
    }

    /// Every wallet in the folder: `<name>.wallet/<name>.keys`.
    pub fn scan(path: &Path) -> Result<Self> {
        let listing = std::fs::read_dir(path).map_err(|e| Error::custom(format!("cannot read {}: {e}", path.display())))?;
        let mut wallets = Vec::new();
        for entry in listing.flatten() {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let Some(name) = dir_name.strip_suffix(".wallet") else { continue };
            if !entry.path().is_dir() {
                continue;
            }
            if let Ok(files) = WalletFiles::in_dir(entry.path(), name) {
                wallets.push(files);
            }
        }
        wallets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self { path: path.to_path_buf(), wallets, open: None })
    }

    pub fn open_wallet(&self) -> Option<&WalletFiles> {
        let open = self.open.as_deref()?;
        self.wallets.iter().find(|w| w.name == open)
    }

    pub fn wallet(&self, name: &str) -> Option<&WalletFiles> {
        self.wallets.iter().find(|w| w.name == name)
    }

    /// The wallets a backup covers: those with a public half.
    pub fn covered(&self) -> Vec<&WalletFiles> {
        self.wallets.iter().filter(|w| w.recipient.is_some()).collect()
    }

    /// The wallets waiting for their words (or one open with their password),
    /// those marked 'never' left out.
    pub fn uncovered(&self) -> Vec<String> {
        self.wallets.iter().filter(|w| w.recipient.is_none() && !w.skipped()).map(|w| w.name.clone()).collect()
    }

    /// Every covered wallet's gate in one string.
    pub fn gate(&self) -> String {
        self.covered().iter().map(|w| format!("{}={}", w.name, w.gate())).collect::<Vec<_>>().join(";")
    }

    /// Every covered wallet's files as listed, unread.
    pub fn listing(&self) -> Result<Vec<Listed>> {
        let mut all = Vec::new();
        for wallet in self.covered() {
            all.extend(wallet.listing()?);
        }
        Ok(all)
    }

    /// Every covered wallet's files.
    pub fn entries(&self) -> Result<Vec<ArchiveEntry>> {
        let mut all = Vec::new();
        for wallet in self.covered() {
            all.extend(wallet.entries()?);
        }
        Ok(all)
    }
}

/// Seals each wallet's entries to its own public half and bundles them.
/// `check_words`: the open wallet's words, to read its part back before
/// anything is done with the bundle — a backup that was never opened is a
/// guess. Returns the bundle and the names it covers.
pub fn seal_folder(
    folder: &Folder,
    entries: &[ArchiveEntry],
    check_words: Option<(&str, &str)>,
    delta: Option<(&str, u32, &[String])>,
) -> Result<(Vec<u8>, Vec<String>)> {
    let mut items = Vec::new();
    for wallet in folder.covered() {
        let Some(pk) = wallet.recipient.as_ref() else { continue };
        let prefix = format!("{}.wallet/", wallet.name);
        let mut own: Vec<ArchiveEntry> = entries.iter().filter(|e| e.path.starts_with(&prefix)).cloned().collect();
        if let Some((checkpoint, seq, removed)) = delta {
            let removed: Vec<String> = removed.iter().filter(|p| p.starts_with(&prefix)).cloned().collect();
            if own.is_empty() && removed.is_empty() {
                continue;
            }
            let note = DeltaNote { checkpoint: checkpoint.to_string(), seq, removed };
            own.push(ArchiveEntry {
                path: DELTA_NOTE.to_string(),
                data: serde_json::to_vec(&note).map_err(|e| Error::custom(e.to_string()))?,
            });
        } else if own.is_empty() {
            continue;
        }
        let blob = archive::seal_to(pk, &own)?;
        if let Some((name, words)) = check_words
            && name == wallet.name
        {
            let back = archive::unseal_with(words, &blob)?;
            if back.len() != own.len() || own.iter().zip(back.iter()).any(|(a, b)| a.path != b.path || a.data != b.data) {
                return Err(Error::custom("the backup did not read back correctly — do not rely on it"));
            }
        }
        items.push(BundleItem { name: wallet.name.clone(), blob });
    }
    let names = items.iter().map(|i| i.name.clone()).collect();
    Ok((archive::pack_bundle(&items)?, names))
}

/// "marigold and reserve", "marigold, reserve and savings", or "5 wallets".
pub fn wallets_phrase(names: &[String]) -> String {
    match names.len() {
        0 => "no wallet".to_string(),
        1 => format!("the wallet '{}'", names[0]),
        2 | 3 => {
            let quoted: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
            let (last, head) = quoted.split_last().unwrap();
            format!("the wallets {} and {last}", head.join(", "))
        }
        n => format!("{n} wallets"),
    }
}

/// The middle of a backup's file name: the wallet names while they are short.
pub fn names_label(names: &[String]) -> String {
    let joined = names.join(", ");
    if names.len() == 1 || joined.len() <= 40 { joined } else { format!("{} wallets", names.len()) }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct DeltaNote {
    pub(crate) checkpoint: String,
    pub(crate) seq: u32,
    pub(crate) removed: Vec<String>,
}

/// The names people see in the chat (founder, 2026-09-24: the old ones were
/// "huge and really technical" and would put anyone's grandmother off):
///
///   Marigold backup - test10 - 2026-09-24 01.17.46 - full.mgb
///   Marigold backup - test10 - 2026-09-24 01.17.46 - change 3.mgb
///
/// with " (part 1 of 3)" before the extension when an archive is split. The
/// checkpoint's stamp in the index stays `c20260924T011746`; the name carries
/// the same moment in readable form, and both convert back and forth.
pub fn checkpoint_name(wallet: &str, checkpoint: &str) -> String {
    format!("Marigold backup - {wallet} - {} - full.mgb", pretty_stamp(checkpoint))
}

pub fn delta_name(wallet: &str, checkpoint: &str, seq: u32) -> String {
    format!("Marigold backup - {wallet} - {} - change {seq}.mgb", pretty_stamp(checkpoint))
}

/// `c20260924T011746` → `2026-09-24 01.17.46`; a stamp of another shape is left as it is.
/// The checkpoint id is UTC, so that the newest full copy sorts newest on
/// any machine; what people read is their own machine's time, with the
/// offset written in so it stays unambiguous and parses back to the same
/// id (tester Charly, 2026-10-06: the names were UTC with no label).
/// `c20260924T011746` → `2026-09-24 03.17.46 +0200` on a machine two hours
/// east of UTC; a stamp of another shape is left as it is.
pub(crate) fn pretty_stamp(stamp: &str) -> String {
    match stamp_to_utc(stamp) {
        Some(utc) => utc.with_timezone(&chrono::Local).format("%Y-%m-%d %H.%M.%S %z").to_string(),
        None => stamp.to_string(),
    }
}

fn stamp_to_utc(stamp: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let d: String = stamp.trim_start_matches('c').chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() != 14 {
        return None;
    }
    chrono::NaiveDateTime::parse_from_str(&d, "%Y%m%d%H%M%S").ok().map(|naive| naive.and_utc())
}

/// `2026-09-24 03.17.46 +0200` → `c20260924T011746`; a name from before the
/// offset was written in (`2026-09-24 01.17.46`) was UTC and is read as such.
fn stamp_from_pretty(pretty: &str) -> Option<String> {
    let pretty = pretty.trim();
    let utc = match chrono::DateTime::parse_from_str(pretty, "%Y-%m-%d %H.%M.%S %z") {
        Ok(with_offset) => with_offset.with_timezone(&chrono::Utc),
        Err(_) => {
            let d: String = pretty.chars().filter(|c| c.is_ascii_digit()).collect();
            if d.len() != 14 {
                return None;
            }
            chrono::NaiveDateTime::parse_from_str(&d, "%Y%m%d%H%M%S").ok()?.and_utc()
        }
    };
    Some(utc.format("c%Y%m%dT%H%M%S").to_string())
}

/// The moment a checkpoint carries, for people, in the machine's own time
/// with the zone named: `2026-09-24 03:17 (UTC+02:00)`.
pub fn checkpoint_moment(stamp: &str) -> String {
    match stamp_to_utc(stamp) {
        Some(utc) => {
            let local = utc.with_timezone(&chrono::Local);
            let offset = local.format("%:z").to_string();
            format!("{} (UTC{})", local.format("%Y-%m-%d %H:%M"), offset)
        }
        None => stamp.to_string(),
    }
}

/// Which backup a name is: (checkpoint stamp, None for the checkpoint itself
/// or Some(seq) for a delta). The readable names above; the first scheme
/// (`marigold-<wallet>-c<stamp>.full.mgb` / `.d<seq>.mgb`, 2026-09-24
/// morning); and a name from before checkpoints — one passphrase-sealed
/// archive — which counts as a checkpoint of its own, named by its stem.
pub fn parse_backup_name(name: &str) -> Option<(String, Option<u32>)> {
    let stem = name.strip_suffix(".mgb")?;
    if let Some(rest) = stem.strip_prefix("Marigold backup - ") {
        // `<wallet> - <pretty stamp> - full` or `… - change N`; the wallet
        // name may itself hold " - ", so read from the right.
        let (rest, kind) = rest.rsplit_once(" - ")?;
        let (_, pretty) = rest.rsplit_once(" - ")?;
        let stamp = stamp_from_pretty(pretty)?;
        return match kind {
            "full" => Some((stamp, None)),
            k => k.strip_prefix("change ").and_then(|n| n.parse::<u32>().ok()).map(|seq| (stamp, Some(seq))),
        };
    }
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
pub(crate) fn checkpoint_order(stamp: &str) -> (bool, String) {
    (stamp.starts_with('c') && stamp[1..].chars().all(|c| c.is_ascii_digit() || c == 'T'), stamp.to_string())
}

/// The open wallet's 24 words, read from the vault under the wallet password.
pub async fn words_for(wallet: &Arc<Wallet>, secret: &Secret) -> Result<String> {
    let store = wallet.store().as_note_key_store()?;
    Ok(store.recovery_words(secret).await?)
}

/// What `ensure_recipient` found.
pub enum Covered {
    /// The wallet had its key already.
    Already,
    /// Written now from the wallet's existing words.
    Written,
    /// The wallet had no 24 words at all — made before notes had them — so
    /// they were made now, and the caller shows them: nobody has seen them.
    NewWords(String),
}

/// Writes the open wallet's public half if it has none yet. Called at every
/// open, so a wallet from before 2026-10-06 is covered the first time it is
/// opened; a wallet from before the 24 words existed gets its words first
/// (founder, 2026-10-07: "there is no 24 words to see, just create them").
pub async fn ensure_recipient(wallet: &Arc<Wallet>, name: &str, secret: &Secret) -> Result<Covered> {
    let files = WalletFiles::of_wallet(wallet, name).await?;
    if files.recipient.is_some() {
        return Ok(Covered::Already);
    }
    let store = wallet.store().as_note_key_store()?;
    let (words, new) = if store.vault_exists().await? {
        (store.recovery_words(secret).await?, false)
    } else {
        (store.vault_create(secret).await?, true)
    };
    archive::write_recipient(&files.wallet_dir, &words)?;
    let _ = std::fs::remove_file(files.wallet_dir.join(SKIP_FILE));
    Ok(if new { Covered::NewWords(words) } else { Covered::Written })
}

/// Covers a closed wallet with its words, typed once: checked against one of
/// its note files (the vault key is the words' entropy), then the public half
/// is written. A wallet without notes cannot be checked; it is covered when
/// opened once instead.
pub fn cover(folder: &Folder, name: &str, words: &str) -> Result<()> {
    let wallet =
        folder.wallet(name).ok_or_else(|| Error::custom(format!("there is no wallet '{name}' in {}", folder.path.display())))?;
    archive::cover_with_words(&wallet.wallet_dir, words)?;
    let _ = std::fs::remove_file(wallet.wallet_dir.join(SKIP_FILE));
    Ok(())
}

/// Covers a closed wallet with its password instead of its words: the words
/// are read from the wallet's own files the way an open wallet reads them,
/// shown, and the public half written. For the wallets made before the words
/// were shown at creation (founder, 2026-10-06: "this being a test net, I did
/// not write them down from the beginning"). A wallet from before the words
/// existed at all gets them made now. Returns the words and whether they are new.
pub async fn cover_with_password(folder: &Folder, name: &str, secret: &Secret) -> Result<(String, bool)> {
    use kaspa_wallet_core::storage::local::{Storage, WalletStorage, notevault::NoteVault, wallet_file_name};
    let wallet =
        folder.wallet(name).ok_or_else(|| Error::custom(format!("there is no wallet '{name}' in {}", folder.path.display())))?;
    // The password is checked against the wallet's own file first: making
    // the words under a wrong password would lock the wallet's notes away
    // from its real one.
    let storage = Storage::try_new_with_folder(&folder.path.to_string_lossy(), &wallet_file_name(name))?;
    let keys = WalletStorage::try_load(&storage).await?;
    keys.payload(secret).map_err(|_| Error::custom("that is not this wallet's password"))?;
    let vault = NoteVault::at(&wallet.vault_folder);
    let (words, new) = if vault.exists().await? {
        (vault.recovery_words(secret).await.map_err(|_| Error::custom("that is not this wallet's password"))?, false)
    } else {
        (vault.create(secret).await?, true)
    };
    archive::write_recipient(&wallet.wallet_dir, &words)?;
    let _ = std::fs::remove_file(wallet.wallet_dir.join(SKIP_FILE));
    Ok((words, new))
}

/// 'never' for a wallet: it is left out of the question from now on.
pub fn skip(folder: &Folder, name: &str) -> Result<()> {
    let wallet = folder.wallet(name).ok_or_else(|| Error::custom(format!("there is no wallet '{name}'")))?;
    std::fs::write(wallet.wallet_dir.join(SKIP_FILE), b"left out of the backups by its owner; 'telegram cover' undoes this\n")
        .map_err(|e| Error::custom(format!("cannot write the marker: {e}")))
}

/// The bundle for a backup file — every covered wallet, the open one's part
/// read back with its words — and the names it holds.
pub fn bundle_checked(files: &WalletFiles, words: &str) -> Result<(Vec<ArchiveEntry>, Vec<u8>, Vec<String>)> {
    let folder = Folder::around(files)?;
    if files.recipient.is_none() {
        return Err(Error::custom(format!("the wallet '{}' has no backup key yet — close and open it once", files.name)));
    }
    let entries = folder.entries()?;
    let (packed, names) = seal_folder(&folder, &entries, Some((&files.name, words)), None)?;
    Ok((entries, packed, names))
}

/// What opens a restore: the wallets' words by wallet name (a bundle), or a
/// single key for an archive from before bundles — 24 words through
/// `key_from_words`, or the passphrase of one older still.
pub enum Unlock<'a> {
    Words(&'a BTreeMap<String, String>),
    Legacy(&'a Secret),
}

/// A restore's files, per wallet.
pub struct Merged {
    pub wallets: Vec<(String, Vec<ArchiveEntry>)>,
    pub stamp: String,
    pub deltas: u32,
}

/// The newest checkpoint among the forwarded names, and the names belonging to it.
/// A checkpoint's stamp, its bytes, and the deltas after it by sequence number.
type NewestSet<'a> = (String, &'a [u8], BTreeMap<u32, &'a Vec<u8>>);

fn newest_set(collected: &BTreeMap<String, Vec<u8>>) -> Result<NewestSet<'_>> {
    let mut latest_stamp = String::new();
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
    let mut checkpoint: Option<&Vec<u8>> = None;
    let mut deltas = BTreeMap::new();
    for (name, bytes) in collected {
        let Some((stamp, seq)) = parse_backup_name(name) else { continue };
        if stamp != latest_stamp {
            continue;
        }
        match seq {
            None => checkpoint = Some(bytes),
            Some(seq) => {
                deltas.insert(seq, bytes);
            }
        }
    }
    let checkpoint = checkpoint.ok_or_else(|| Error::custom("the checkpoint could not be read"))?;
    Ok((latest_stamp, checkpoint, deltas))
}

/// One backup file as a collected set of one: `merge` and the questions
/// around it then serve the file restore and the Telegram restore alike.
pub fn single(bytes: Vec<u8>) -> BTreeMap<String, Vec<u8>> {
    let mut collected = BTreeMap::new();
    collected.insert("backup-file.mgb".to_string(), bytes);
    collected
}

/// The wallets the newest forwarded backup holds — empty for an archive from
/// before bundles, which holds one wallet behind one key.
pub fn bundle_wallets(collected: &BTreeMap<String, Vec<u8>>) -> Result<Vec<String>> {
    let (_, checkpoint, deltas) = newest_set(collected)?;
    if !archive::is_bundle(checkpoint) {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = archive::parse_bundle(checkpoint)?.into_iter().map(|i| i.name).collect();
    for bytes in deltas.values() {
        if archive::is_bundle(bytes) {
            for item in archive::parse_bundle(bytes)? {
                if !names.contains(&item.name) {
                    names.push(item.name);
                }
            }
        }
    }
    Ok(names)
}

/// Which of a bundle's wallets these words open.
pub fn wallets_for_words(collected: &BTreeMap<String, Vec<u8>>, words: &str) -> Result<Vec<String>> {
    let (_, checkpoint, deltas) = newest_set(collected)?;
    let mut names = Vec::new();
    for bytes in std::iter::once(checkpoint).chain(deltas.values().map(|b| b.as_slice())) {
        if !archive::is_bundle(bytes) {
            continue;
        }
        for item in archive::parse_bundle(bytes)? {
            if archive::words_fit(words, &item.blob) && !names.contains(&item.name) {
                names.push(item.name);
            }
        }
    }
    Ok(names)
}

/// Merges a checkpoint and the deltas after it into one set of files per wallet.
pub fn merge(collected: &BTreeMap<String, Vec<u8>>, unlock: Unlock<'_>) -> Result<Merged> {
    let (stamp, checkpoint, deltas) = newest_set(collected)?;
    let seqs: Vec<u32> = deltas.keys().copied().collect();
    for (i, seq) in seqs.iter().enumerate() {
        if *seq != i as u32 + 1 {
            return Err(Error::custom(format!(
                "delta {} of checkpoint {stamp} is missing — forward it too (deltas {} to {} were found)",
                i + 1,
                seqs.first().copied().unwrap_or(0),
                seqs.last().copied().unwrap_or(0)
            )));
        }
    }
    let applied = seqs.len() as u32;

    // Each wallet's entries, checkpoint first and then every delta, opened
    // by whatever opens them.
    let open_sets: Vec<(String, Vec<Vec<ArchiveEntry>>)> = if archive::is_bundle(checkpoint) {
        let Unlock::Words(words_by_wallet) = unlock else {
            return Err(Error::custom("this backup holds several wallets; each opens with its own 24 words"));
        };
        let mut sets: BTreeMap<String, Vec<Vec<ArchiveEntry>>> = BTreeMap::new();
        for bytes in std::iter::once(checkpoint).chain(deltas.values().map(|b| b.as_slice())) {
            for item in archive::parse_bundle(bytes)? {
                let Some(words) = words_by_wallet.get(&item.name) else { continue };
                sets.entry(item.name.clone()).or_default().push(archive::unseal_with(words, &item.blob)?);
            }
        }
        sets.into_iter().collect()
    } else {
        let open = |bytes: &[u8]| -> Result<Vec<ArchiveEntry>> {
            match &unlock {
                Unlock::Legacy(key) => archive::unpack(bytes, key),
                Unlock::Words(map) => {
                    for words in map.values() {
                        if let Ok(entries) = archive::unpack(bytes, &archive::key_from_words(words)) {
                            return Ok(entries);
                        }
                    }
                    Err(Error::custom("none of the words given open this backup"))
                }
            }
        };
        let mut all = vec![open(checkpoint)?];
        for bytes in deltas.values() {
            all.push(open(bytes)?);
        }
        let name = archive::wallet_name_in(&all[0]).unwrap_or_else(|_| "wallet".to_string());
        vec![(name, all)]
    };
    if open_sets.is_empty() {
        return Err(Error::custom("no wallet was opened — give the 24 words of at least one"));
    }

    let mut wallets = Vec::new();
    for (name, sets) in open_sets {
        let mut by_path: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for entries in sets {
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
        }
        wallets.push((name, by_path.into_iter().map(|(path, data)| ArchiveEntry { path, data }).collect()));
    }
    Ok(Merged { wallets, stamp, deltas: applied })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names_parse() {
        // The readable part is local time with the offset; whatever the zone, it parses back to the same id.
        let full = checkpoint_name("test10", "c20260924T011746");
        assert!(full.starts_with("Marigold backup - test10 - 2026-09-24 ") && full.ends_with(" - full.mgb"), "{full}");
        assert_eq!(parse_backup_name(&full), Some(("c20260924T011746".to_string(), None)));
        let delta = delta_name("my - wallet", "c20260924T011746", 3);
        assert_eq!(parse_backup_name(&delta), Some(("c20260924T011746".to_string(), Some(3))));
        // A name written two hours east of UTC, read anywhere.
        assert_eq!(
            parse_backup_name("Marigold backup - test10 - 2026-09-24 03.17.46 +0200 - full.mgb"),
            Some(("c20260924T011746".to_string(), None))
        );
        // A name from before the offset was written in was UTC.
        assert_eq!(
            parse_backup_name("Marigold backup - test10 - 2026-09-24 01.17.46 - full.mgb"),
            Some(("c20260924T011746".to_string(), None))
        );
        assert!(checkpoint_moment("c20260924T011746").contains("(UTC"));
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
        let key = archive::key_from_words(
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
        let merged = merge(&collected, Unlock::Legacy(&key)).unwrap();
        assert_eq!(merged.stamp, "c20260924T100000");
        assert_eq!(merged.deltas, 2);
        let entries = &merged.wallets[0].1;
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["w/keys", "w/notes/b.note", "w/notes/c.note"]);
        assert_eq!(entries[1].data, b"b2");
        assert!(bundle_wallets(&collected).unwrap().is_empty());

        collected.remove(&delta_name("w", "c20260924T100000", 1));
        assert!(merge(&collected, Unlock::Legacy(&key)).is_err());
    }

    const WORDS_A: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
    const WORDS_B: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote";

    fn e(path: &str, data: &[u8]) -> ArchiveEntry {
        ArchiveEntry { path: path.into(), data: data.to_vec() }
    }

    #[test]
    fn a_bundle_merges_per_wallet_and_each_needs_its_own_words() {
        let pk_a = archive::recipient_public(WORDS_A);
        let pk_b = archive::recipient_public(WORDS_B);
        let full = archive::pack_bundle(&[
            BundleItem {
                name: "a".into(),
                blob: archive::seal_to(&pk_a, &[e("a.wallet/a.keys", b"ka"), e("a.wallet/notes/x.note", b"x")]).unwrap(),
            },
            BundleItem { name: "b".into(), blob: archive::seal_to(&pk_b, &[e("b.wallet/b.keys", b"kb")]).unwrap() },
        ])
        .unwrap();
        let note = |removed: Vec<&str>| {
            serde_json::to_vec(&DeltaNote { checkpoint: "c".into(), seq: 1, removed: removed.into_iter().map(String::from).collect() })
                .unwrap()
        };
        // Only wallet a changed: its x.note went, y.note came.
        let d1 = archive::pack_bundle(&[BundleItem {
            name: "a".into(),
            blob: archive::seal_to(&pk_a, &[e("a.wallet/notes/y.note", b"y"), e(DELTA_NOTE, &note(vec!["a.wallet/notes/x.note"]))])
                .unwrap(),
        }])
        .unwrap();
        let mut collected = BTreeMap::new();
        collected.insert(checkpoint_name("a, b", "c20261006T100000"), full);
        collected.insert(delta_name("a, b", "c20261006T100000", 1), d1);

        assert_eq!(bundle_wallets(&collected).unwrap(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(wallets_for_words(&collected, WORDS_B).unwrap(), vec!["b".to_string()]);

        let mut words = BTreeMap::new();
        words.insert("a".to_string(), WORDS_A.to_string());
        let merged = merge(&collected, Unlock::Words(&words)).unwrap();
        assert_eq!(merged.deltas, 1);
        assert_eq!(merged.wallets.len(), 1);
        let paths: Vec<&str> = merged.wallets[0].1.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(paths, vec!["a.wallet/a.keys", "a.wallet/notes/y.note"]);

        words.insert("b".to_string(), WORDS_B.to_string());
        let merged = merge(&collected, Unlock::Words(&words)).unwrap();
        assert_eq!(merged.wallets.len(), 2);
        assert_eq!(merged.wallets[1].1[0].data, b"kb");

        // The wrong words for b: refused, not garbled.
        words.insert("b".to_string(), WORDS_A.to_string());
        assert!(merge(&collected, Unlock::Words(&words)).is_err());
        // A passphrase cannot open a bundle.
        assert!(merge(&collected, Unlock::Legacy(&Secret::from(b"x".to_vec()))).is_err());
    }
}
