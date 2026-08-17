//!
//! Note vault: file-per-note storage for the note key database (FORK-PLAN P7.6,
//! DECISIONS.md's "Note vault, backup, and restore-rotation policy" entry,
//! POOL-SPEC.md P5.6's "Note vault (primary backup) and paper export"). Replaces
//! P7.1's single encrypted `sn -> NoteKeyEntry` map: one file per note, plaintext
//! filename (value, serial), contents encrypted under a per-wallet vault key `K`
//! (never the Argon2-stretched wallet secret directly — see
//! `encryption::encrypt_xchacha20poly1305_raw_key`'s doc comment for why). Balance
//! and coin selection never decrypt anything — they read the plaintext manifest,
//! hydrated once into memory; a spend decrypts exactly the note files it selects.
//!
//! Directory layout, mirroring `transaction/fsio.rs`'s `<name>.transactions/`
//! convention exactly:
//! ```text
//! <folder>/<name>.notes/
//!     vault.key        -- K, wallet-password-wrapped (Argon2 path -- see below)
//!     manifest.tsv      -- plaintext (sn, value, pk, provenance, status, last_rotated_at)
//!     active/<value>_<sn>.note
//!     handed-over/<value>_<sn>.note
//!     superseded/<value>_<sn>.note
//! ```
//!
//! `manifest.tsv` deliberately does double duty as both DECISIONS.md's
//! user-facing "optional plaintext manifest" (serial, value, last-rotated-at) and
//! the vault's own mandatory in-memory-index source (which additionally needs
//! `pk`/`provenance`/`status` to avoid decrypting every note file just to answer
//! `iter()`/`load_info()`) — a deliberate simplification over keeping two
//! plaintext files in sync; recorded as such in NOTES.md's P7.6 entry. None of
//! its columns are sensitive: the pool is plaintext, so `sn`/`pk`/`d` are already
//! public on-chain, and `provenance`/`status` are wallet-internal hygiene, not
//! secrets (only `sk`, inside the per-note encrypted files, is).

use crate::encryption::{decrypt_xchacha20poly1305_raw_key, encrypt_xchacha20poly1305, encrypt_xchacha20poly1305_raw_key};
use crate::imports::*;
use crate::storage::interface::StorageStream;
use crate::storage::notekeys::{NoteKeyEntry, NoteKeyInfo, NoteProvenance, NoteStatus, NotesChangedApplyResult};
use futures::stream;
use kaspa_bip32::{Language, Mnemonic};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::DenominationTag;
use rand::RngCore;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use workflow_store::fs;

const VAULT_KEY_FILE: &str = "vault.key";
const MANIFEST_FILE: &str = "manifest.tsv";

fn status_subdir(status: NoteStatus) -> &'static str {
    match status {
        NoteStatus::Active => "active",
        NoteStatus::HandedOver => "handed-over",
        NoteStatus::Superseded => "superseded",
    }
}

fn status_to_str(status: NoteStatus) -> &'static str {
    status_subdir(status)
}

fn status_from_str(s: &str) -> Option<NoteStatus> {
    match s {
        "active" => Some(NoteStatus::Active),
        "handed-over" => Some(NoteStatus::HandedOver),
        "superseded" => Some(NoteStatus::Superseded),
        _ => None,
    }
}

fn provenance_to_str(p: NoteProvenance) -> &'static str {
    match p {
        NoteProvenance::Cold => "cold",
        NoteProvenance::Hot => "hot",
    }
}

fn provenance_from_str(s: &str) -> Option<NoteProvenance> {
    match s {
        "cold" => Some(NoteProvenance::Cold),
        "hot" => Some(NoteProvenance::Hot),
        _ => None,
    }
}

fn denomination_petals(d: DenominationTag) -> u64 {
    kaspa_consensus_core::notepool::DENOMINATION_PETALS[d as usize]
}

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A note file's decrypted contents — the sensitive half. Mirrors `NoteKeyEntry`
/// plus the manifest's `last_rotated_at` (stored redundantly here too, so a
/// manifest rebuild from raw files, e.g. after corruption, recovers it exactly).
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
struct VaultNoteFile {
    sn: Hash,
    sk: [u8; 32],
    d: DenominationTag,
    provenance: NoteProvenance,
    last_rotated_at: u64,
}

impl Zeroize for VaultNoteFile {
    fn zeroize(&mut self) {
        self.sk.zeroize();
    }
}
impl Drop for VaultNoteFile {
    fn drop(&mut self) {
        self.sk.zeroize();
    }
}

fn note_file_name(sn: &Hash, d: DenominationTag) -> String {
    format!("{}_{}.note", denomination_petals(d), sn.to_hex())
}

/// One in-memory row combining a manifest line's fields — used only while
/// (de)serializing `manifest.tsv`; the public shape is [`NoteKeyInfo`] plus
/// `last_rotated_at`, which `NoteKeyInfo` itself doesn't carry (P7.1 predates it).
#[derive(Clone)]
struct ManifestRow {
    info: NoteKeyInfo,
    last_rotated_at: u64,
}

fn manifest_line(row: &ManifestRow) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        row.info.sn.to_hex(),
        denomination_petals(row.info.d),
        row.info.pk.as_slice().to_hex(),
        provenance_to_str(row.info.provenance),
        status_to_str(row.info.status),
        row.last_rotated_at,
    )
}

fn parse_manifest_line(line: &str) -> Option<ManifestRow> {
    let mut fields = line.split('\t');
    let sn = Hash::from_hex(fields.next()?).ok()?;
    let value: u64 = fields.next()?.parse().ok()?;
    let d = kaspa_consensus_core::notepool::DENOMINATION_PETALS.iter().position(|v| *v == value)?;
    let d = DenominationTag::try_from(d as u8).ok()?;
    let pk_bytes = Vec::<u8>::from_hex(fields.next()?).ok()?;
    let pk: [u8; 32] = pk_bytes.try_into().ok()?;
    let provenance = provenance_from_str(fields.next()?)?;
    let status = status_from_str(fields.next()?)?;
    let last_rotated_at: u64 = fields.next()?.parse().ok()?;
    Some(ManifestRow { info: NoteKeyInfo { sn, pk, d, provenance, status }, last_rotated_at })
}

pub struct NoteVault {
    folder: PathBuf,
    /// In-memory index, hydrated from `manifest.tsv` on first use. Guarded by an
    /// async-aware lock (not `std::sync::RwLock`) so it's safe to hold across the
    /// `.await`s real file I/O now requires — see NOTES.md's P7.6 entry for the
    /// concurrency hazard this specifically avoids relative to P7.1's blob store.
    index: AsyncRwLock<HashMap<Hash, ManifestRow>>,
    loaded: AsyncMutex<bool>,
    /// The vault key `K`, cached after the first successful unwrap from
    /// `vault.key` under the wallet secret — avoids paying Argon2's deliberate
    /// slowness on every call. 32 raw bytes only; still far smaller exposure than
    /// P7.1's "every sk decrypted into memory on every touch."
    key: AsyncRwLock<Option<[u8; 32]>>,
}

impl NoteVault {
    /// `folder`/`name` follow the exact convention `fsio::TransactionStore::new`
    /// already established: `<folder>/<name>.notes/`.
    pub fn new<P: AsRef<Path>>(folder: P, name: &str) -> Self {
        let base = fs::resolve_path(folder.as_ref().to_str().unwrap()).expect("note vault folder is invalid");
        Self {
            folder: base.join(format!("{name}.notes")),
            index: AsyncRwLock::new(HashMap::new()),
            loaded: AsyncMutex::new(false),
            key: AsyncRwLock::new(None),
        }
    }

    /// Construct a vault handle pointed EXACTLY at `path` — no `<name>.notes`
    /// suffix appended, unlike [`Self::new`]. A wallet's own primary vault always
    /// uses `new()`'s folder/filename convention; `at()` is for standalone
    /// vault-shaped directories that exist independent of any wallet — a `note
    /// vault backup` copy, read back later by `note vault verify <dir>` or `note
    /// vault restore <dir>` without ever opening (or even having) a matching
    /// wallet at that moment (DECISIONS.md: "checking your backups against the
    /// manifest without actually restoring").
    pub fn at<P: AsRef<Path>>(path: P) -> Self {
        let folder = fs::resolve_path(path.as_ref().to_str().unwrap()).expect("note vault path is invalid");
        Self { folder, index: AsyncRwLock::new(HashMap::new()), loaded: AsyncMutex::new(false), key: AsyncRwLock::new(None) }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn subdir(&self, status: NoteStatus) -> PathBuf {
        self.folder.join(status_subdir(status))
    }

    async fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.folder).await?;
        for status in [NoteStatus::Active, NoteStatus::HandedOver, NoteStatus::Superseded] {
            fs::create_dir_all(self.subdir(status)).await?;
        }
        Ok(())
    }

    /// Create a fresh vault: generate `K`, wrap it under `wallet_secret`, return
    /// the 24 English BIP39 words encoding `K` directly (FORK-PLAN P7.6's
    /// ceremony — `K` *is* the entropy, the words are purely an encoding, not a
    /// derivation; see DECISIONS.md). Errors if a vault already exists here.
    pub async fn create(&self, wallet_secret: &Secret) -> Result<String> {
        self.ensure_dirs().await?;
        let key_path = self.folder.join(VAULT_KEY_FILE);
        if fs::exists(&key_path).await? {
            return Err(Error::Custom("a note vault already exists at this location".to_string()));
        }
        let mut k = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut k);
        let mnemonic = Mnemonic::from_entropy(k.to_vec(), Language::English)?;
        let words = mnemonic.phrase_string();
        let wrapped = encrypt_xchacha20poly1305(&k, wallet_secret)?;
        fs::write(&key_path, &wrapped).await?;
        self.key.write().await.replace(k);
        if !fs::exists(&self.folder.join(MANIFEST_FILE)).await? {
            fs::write(&self.folder.join(MANIFEST_FILE), b"").await?;
        }
        Ok(words)
    }

    pub async fn exists(&self) -> Result<bool> {
        Ok(fs::exists(&self.folder.join(VAULT_KEY_FILE)).await?)
    }

    /// Wipe every file this vault owns and reset in-memory state. Called by
    /// `LocalStoreInner::try_create`/`try_import` before constructing a fresh
    /// wallet at a folder/filename that may have hosted an *older* same-named
    /// wallet: leaving a stale `vault.key` behind would wrap `K` under a secret
    /// that no longer matches the new wallet's — the very first `store()` would
    /// fail to decrypt it. A brand-new location where nothing exists yet is a
    /// safe no-op. Never called from `try_load` (opening a wallet must keep its
    /// existing vault intact).
    pub async fn reset(&self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.folder.exists() {
                std::fs::remove_dir_all(&self.folder)
                    .map_err(|e| Error::Custom(format!("failed to reset note vault at {:?}: {e}", self.folder)))?;
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            log_warn!("note vault: reset() is a no-op on wasm32 — a stale vault.key from a prior same-named wallet may remain");
        }
        *self.index.write().await = HashMap::new();
        *self.loaded.lock().await = true;
        *self.key.write().await = None;
        Ok(())
    }

    /// Unwrap `K` from `vault.key` under `wallet_secret`, caching it for the
    /// session. Idempotent (returns the cached copy on later calls).
    async fn unlock(&self, wallet_secret: &Secret) -> Result<[u8; 32]> {
        if let Some(k) = *self.key.read().await {
            return Ok(k);
        }
        let key_path = self.folder.join(VAULT_KEY_FILE);
        let wrapped = fs::read(&key_path)
            .await
            .map_err(|_| Error::Custom("no note vault found (or vault.key is missing) — run vault creation first".to_string()))?;
        let k_secret = crate::encryption::decrypt_xchacha20poly1305(&wrapped, wallet_secret)?;
        let k: [u8; 32] =
            k_secret.as_ref().try_into().map_err(|_| Error::Custom("vault.key did not decrypt to a 32-byte key".to_string()))?;
        self.key.write().await.replace(k);
        Ok(k)
    }

    /// Recover `K` directly from its 24-word encoding (restore path — no wallet
    /// secret needed, per DECISIONS.md: recovery needs the words *or* the
    /// password-wrapped file, either unlocks the same `K`). Also re-wraps `K`
    /// under `wallet_secret` into a fresh `vault.key`, so daily use afterward
    /// doesn't need the words again.
    ///
    /// Forces the in-memory index to reload from `manifest.tsv` on the next call
    /// that needs it, regardless of whatever was cached before this call — the
    /// whole point of calling this is that the caller just copied a `manifest.tsv`
    /// (and note files) onto disk out from under this handle (a restore's "the
    /// files" half); anything cached from before that copy, including an empty
    /// index from an earlier `is_empty()`/`iter()` on a then-nonexistent vault,
    /// would otherwise silently shadow the restored data forever.
    pub async fn restore_key_from_words(&self, words: &str, wallet_secret: &Secret) -> Result<()> {
        let mnemonic = Mnemonic::new(words, Language::English)?;
        let entropy = mnemonic.entropy();
        let k: [u8; 32] =
            entropy.as_slice().try_into().map_err(|_| Error::Custom("recovery words must encode a 32-byte key (24 words)".to_string()))?;
        self.ensure_dirs().await?;
        let wrapped = encrypt_xchacha20poly1305(&k, wallet_secret)?;
        fs::write(&self.folder.join(VAULT_KEY_FILE), &wrapped).await?;
        self.key.write().await.replace(k);
        *self.loaded.lock().await = false;
        Ok(())
    }

    async fn ensure_loaded(&self) -> Result<()> {
        if *self.loaded.lock().await {
            return Ok(());
        }
        let manifest_path = self.folder.join(MANIFEST_FILE);
        let mut rows = HashMap::new();
        if let Ok(bytes) = fs::read(&manifest_path).await {
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                if let Some(row) = parse_manifest_line(line) {
                    rows.insert(row.info.sn, row);
                }
            }
        }
        *self.index.write().await = rows;
        *self.loaded.lock().await = true;
        Ok(())
    }

    async fn persist_manifest(&self) -> Result<()> {
        let index = self.index.read().await;
        let mut lines: Vec<(Hash, String)> = index.iter().map(|(sn, row)| (*sn, manifest_line(row))).collect();
        lines.sort_by_key(|(sn, _)| sn.as_bytes());
        let text = lines.into_iter().map(|(_, line)| line).collect::<Vec<_>>().join("\n");
        fs::write(&self.folder.join(MANIFEST_FILE), text.as_bytes()).await?;
        Ok(())
    }

    async fn read_note_file(&self, sn: &Hash, status: NoteStatus, k: &[u8; 32]) -> Result<VaultNoteFile> {
        let row = self.index.read().await.get(sn).cloned().ok_or_else(|| Error::Custom(format!("serial {sn} is not in the vault")))?;
        let path = self.subdir(status).join(note_file_name(sn, row.info.d));
        let bytes = fs::read(&path).await?;
        let plaintext = decrypt_xchacha20poly1305_raw_key(&bytes, k)?;
        Ok(VaultNoteFile::try_from_slice(plaintext.as_ref())?)
    }

    async fn write_note_file(&self, file: &VaultNoteFile, status: NoteStatus, k: &[u8; 32]) -> Result<()> {
        let path = self.subdir(status).join(note_file_name(&file.sn, file.d));
        let plaintext = borsh::to_vec(file)?;
        let ciphertext = encrypt_xchacha20poly1305_raw_key(&plaintext, k)?;
        fs::write(&path, &ciphertext).await?;
        Ok(())
    }

    async fn remove_note_file(&self, sn: &Hash, d: DenominationTag, status: NoteStatus) -> Result<()> {
        let path = self.subdir(status).join(note_file_name(sn, d));
        if fs::exists(&path).await? {
            fs::remove(&path).await?;
        }
        Ok(())
    }

    /// Move a note's file between status subdirectories (`fs::rename` — atomic on
    /// every native/NodeJS target this wallet ships to; see NOTES.md's P7.6 entry
    /// for the one platform caveat, plain-browser wasm32, and why it doesn't apply
    /// here).
    async fn move_note_file(&self, sn: &Hash, d: DenominationTag, from: NoteStatus, to: NoteStatus) -> Result<()> {
        if from == to {
            return Ok(());
        }
        let name = note_file_name(sn, d);
        let from_path = self.subdir(from).join(&name);
        let to_path = self.subdir(to).join(&name);
        fs::rename(&from_path, &to_path).await?;
        Ok(())
    }

    /// Full per-file rebuild of the manifest from the raw vault contents,
    /// decrypting every note (needs `K`) — the recovery path when `manifest.tsv`
    /// is lost or found corrupted, and also exactly what "deep verify" (below)
    /// does as a side effect of checking every file's integrity.
    pub async fn rebuild_manifest(&self, wallet_secret: &Secret) -> Result<usize> {
        let k = self.unlock(wallet_secret).await?;
        let mut rows = HashMap::new();
        for status in [NoteStatus::Active, NoteStatus::HandedOver, NoteStatus::Superseded] {
            let dir = self.subdir(status);
            let entries = match fs::readdir(dir.clone(), false).await {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries {
                let name = entry.file_name().to_string();
                let path = dir.join(&name);
                let Ok(bytes) = fs::read(&path).await else { continue };
                let Ok(plaintext) = decrypt_xchacha20poly1305_raw_key(&bytes, &k) else {
                    log_error!("note vault: {path:?} failed to decrypt during rebuild — skipped, left in place");
                    continue;
                };
                let Ok(file) = VaultNoteFile::try_from_slice(plaintext.as_ref()) else { continue };
                let pk = NoteKeyEntry::new(file.sn, file.sk, file.d, file.provenance).derive_pk()?;
                let info = NoteKeyInfo { sn: file.sn, pk, d: file.d, provenance: file.provenance, status };
                rows.insert(file.sn, ManifestRow { info, last_rotated_at: file.last_rotated_at });
            }
        }
        let count = rows.len();
        *self.index.write().await = rows;
        *self.loaded.lock().await = true;
        self.persist_manifest().await?;
        Ok(count)
    }
}

impl NoteVault {
    // ~~~ NoteKeyStore-shaped methods (FORK-PLAN P7.7 wires these into
    // `LocalStoreInner`'s `NoteKeyStore` impl, which continues to serve the
    // `store_payment_request`/`payment_requests`/`load_payment_request_key`/
    // `remove_payment_request` quartet from the existing `Payload`/`Cache` blob —
    // per DECISIONS.md, `PaymentRequestKey` storage is explicitly NOT migrated
    // here, so `NoteVault` itself only ever needs to speak the sn-keyed half of
    // the trait and is kept a plain struct rather than a `NoteKeyStore` impl. ~~~

    pub async fn is_empty(&self) -> Result<bool> {
        self.ensure_loaded().await?;
        Ok(self.index.read().await.is_empty())
    }

    pub async fn iter(&self) -> Result<StorageStream<Arc<NoteKeyInfo>>> {
        self.ensure_loaded().await?;
        let infos: Vec<Arc<NoteKeyInfo>> = self.index.read().await.values().map(|row| Arc::new(row.info.clone())).collect();
        Ok(Box::pin(stream::iter(infos.into_iter().map(Ok))))
    }

    pub async fn load_info(&self, sn: &Hash) -> Result<Option<Arc<NoteKeyInfo>>> {
        self.ensure_loaded().await?;
        Ok(self.index.read().await.get(sn).map(|row| Arc::new(row.info.clone())))
    }

    pub async fn load_key(&self, wallet_secret: &Secret, sn: &Hash) -> Result<Option<NoteKeyEntry>> {
        self.ensure_loaded().await?;
        let status = match self.index.read().await.get(sn) {
            Some(row) => row.info.status,
            None => return Ok(None),
        };
        let k = self.unlock(wallet_secret).await?;
        let file = self.read_note_file(sn, status, &k).await?;
        Ok(Some(NoteKeyEntry::new(file.sn, file.sk, file.d, file.provenance)))
    }

    pub async fn store(&self, wallet_secret: &Secret, entry: NoteKeyEntry) -> Result<()> {
        self.ensure_loaded().await?;
        let k = self.unlock(wallet_secret).await?;
        let pk = entry.derive_pk()?;
        let last_rotated_at = now_unix();
        let file = VaultNoteFile { sn: entry.sn, sk: entry.sk, d: entry.d, provenance: entry.provenance, last_rotated_at };
        self.write_note_file(&file, NoteStatus::Active, &k).await?;
        let info = NoteKeyInfo { sn: entry.sn, pk, d: entry.d, provenance: entry.provenance, status: NoteStatus::Active };
        self.index.write().await.insert(entry.sn, ManifestRow { info, last_rotated_at });
        self.persist_manifest().await?;
        Ok(())
    }

    pub async fn remove(&self, wallet_secret: &Secret, sn: &Hash) -> Result<()> {
        self.ensure_loaded().await?;
        let _ = wallet_secret;
        let removed = self.index.write().await.remove(sn);
        if let Some(row) = removed {
            self.remove_note_file(sn, row.info.d, row.info.status).await?;
            self.persist_manifest().await?;
        }
        Ok(())
    }

    pub async fn import_bearer_key(&self, wallet_secret: &Secret, sn: Hash, sk: [u8; 32], d: DenominationTag) -> Result<()> {
        // A key crossing a wallet boundary is Hot by definition (POOL-SPEC.md
        // P5.6) — this entry point never accepts a caller-supplied provenance.
        let entry = NoteKeyEntry::new(sn, sk, d, NoteProvenance::Hot);
        self.store(wallet_secret, entry).await
    }

    pub async fn mark_status(&self, sn: &Hash, status: NoteStatus) -> Result<()> {
        self.ensure_loaded().await?;
        let old = {
            let mut index = self.index.write().await;
            match index.get_mut(sn) {
                Some(row) => {
                    let old_status = row.info.status;
                    if old_status == status {
                        return Ok(());
                    }
                    row.info.status = status;
                    Some((old_status, row.info.d))
                }
                None => None,
            }
        };
        if let Some((old_status, d)) = old {
            self.move_note_file(sn, d, old_status, status).await?;
            self.persist_manifest().await?;
        }
        Ok(())
    }

    pub async fn apply_notes_changed(
        &self,
        wallet_secret: Option<&Secret>,
        notification: &kaspa_rpc_core::message::NotesChangedNotification,
    ) -> Result<NotesChangedApplyResult> {
        self.ensure_loaded().await?;
        let mut result = NotesChangedApplyResult::default();

        let superseded: Vec<Hash> = {
            let index = self.index.read().await;
            notification.removed.iter().map(|entry| entry.sn).filter(|sn| index.contains_key(sn)).collect()
        };
        for sn in superseded {
            self.mark_status(&sn, NoteStatus::Superseded).await?;
            result.superseded.push(sn);
        }

        let candidates: Vec<(Hash, Hash, u8, NoteProvenance)> = {
            let index = self.index.read().await;
            notification
                .added
                .iter()
                .filter_map(|entry| {
                    index.values().find(|row| row.info.pk == entry.pk).map(|row| (entry.sn, row.info.sn, entry.denomination, row.info.provenance))
                })
                .collect()
        };

        for (new_sn, source_sn, denomination, provenance) in candidates {
            let Some(secret) = wallet_secret else {
                result.deferred.push(new_sn);
                continue;
            };
            let d = kaspa_consensus_core::notepool::DenominationTag::try_from(denomination)
                .map_err(|_| Error::Custom(format!("NotesChanged: unknown denomination tag {denomination}")))?;
            if let Some(source) = self.load_key(secret, &source_sn).await? {
                let entry = NoteKeyEntry::new(new_sn, source.sk, d, provenance);
                self.store(secret, entry).await?;
                result.added.push(new_sn);
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_rpc_core::message::RpcNoteEntry;

    fn make_vault(dir: &tempfile::TempDir) -> NoteVault {
        NoteVault::new(dir.path(), "test")
    }

    #[tokio::test]
    async fn create_then_store_load_remove_round_trip() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let secret = Secret::from("vault-test-secret");
        let vault = make_vault(&dir);

        assert!(!vault.exists().await?);
        let words = vault.create(&secret).await?;
        assert_eq!(words.split(' ').count(), 24);
        assert!(vault.exists().await?);

        let sn = Hash::from([0x11u8; 32]);
        let sk = [0x22u8; 32];
        let entry = NoteKeyEntry::new(sn, sk, DenominationTag::D1, NoteProvenance::Cold);

        assert!(vault.is_empty().await?);
        vault.store(&secret, entry.clone()).await?;
        assert!(!vault.is_empty().await?);

        let loaded = vault.load_key(&secret, &sn).await?.expect("entry round-trips");
        assert_eq!(loaded, entry);

        let info = vault.load_info(&sn).await?.expect("info round-trips");
        assert_eq!(info.provenance, NoteProvenance::Cold);
        assert_eq!(info.status, NoteStatus::Active);
        assert_eq!(info.pk, entry.derive_pk()?);

        // The plaintext file layout: value+serial in the filename, under active/.
        let file_path = dir.path().join("test.notes/active").join(note_file_name(&sn, DenominationTag::D1));
        assert!(fs::exists(&file_path).await?);

        vault.remove(&secret, &sn).await?;
        assert!(vault.load_key(&secret, &sn).await?.is_none());
        assert!(!fs::exists(&file_path).await?);

        Ok(())
    }

    #[tokio::test]
    async fn mark_status_moves_the_file_between_subdirectories() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let secret = Secret::from("vault-test-secret");
        let vault = make_vault(&dir);
        vault.create(&secret).await?;

        let sn = Hash::from([0x33u8; 32]);
        let entry = NoteKeyEntry::new(sn, [0x44u8; 32], DenominationTag::D0_1, NoteProvenance::Hot);
        vault.store(&secret, entry).await?;

        let active_path = dir.path().join("test.notes/active").join(note_file_name(&sn, DenominationTag::D0_1));
        let superseded_path = dir.path().join("test.notes/superseded").join(note_file_name(&sn, DenominationTag::D0_1));
        assert!(fs::exists(&active_path).await?);

        vault.mark_status(&sn, NoteStatus::Superseded).await?;
        assert!(!fs::exists(&active_path).await?);
        assert!(fs::exists(&superseded_path).await?);
        assert_eq!(vault.load_info(&sn).await?.unwrap().status, NoteStatus::Superseded);

        Ok(())
    }

    #[tokio::test]
    async fn import_bearer_key_is_always_hot() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let secret = Secret::from("vault-test-secret");
        let vault = make_vault(&dir);
        vault.create(&secret).await?;

        let sn = Hash::from([0x55u8; 32]);
        vault.import_bearer_key(&secret, sn, [0x66u8; 32], DenominationTag::D10).await?;
        assert_eq!(vault.load_info(&sn).await?.unwrap().provenance, NoteProvenance::Hot);

        Ok(())
    }

    #[tokio::test]
    async fn apply_notes_changed_supersedes_and_rotates() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let secret = Secret::from("vault-test-secret");
        let vault = make_vault(&dir);
        vault.create(&secret).await?;

        let sn_old = Hash::from([0x77u8; 32]);
        let sn_new = Hash::from([0x88u8; 32]);
        let entry = NoteKeyEntry::new(sn_old, [0x99u8; 32], DenominationTag::D1, NoteProvenance::Cold);
        let pk = entry.derive_pk()?;
        vault.store(&secret, entry).await?;

        let notification = kaspa_rpc_core::message::NotesChangedNotification {
            added: Arc::new(vec![RpcNoteEntry { sn: sn_new, denomination: DenominationTag::D1 as u8, pk }]),
            removed: Arc::new(vec![RpcNoteEntry { sn: sn_old, denomination: DenominationTag::D1 as u8, pk }]),
        };

        let result = vault.apply_notes_changed(None, &notification).await?;
        assert_eq!(result.superseded, vec![sn_old]);
        assert_eq!(result.deferred, vec![sn_new]);
        assert_eq!(vault.load_info(&sn_old).await?.unwrap().status, NoteStatus::Superseded);

        let result = vault.apply_notes_changed(Some(&secret), &notification).await?;
        assert_eq!(result.added, vec![sn_new]);
        let new_key = vault.load_key(&secret, &sn_new).await?.expect("rotated row stored");
        assert_eq!(new_key.sk, [0x99u8; 32]);

        Ok(())
    }

    #[tokio::test]
    async fn restore_from_words_recovers_the_same_key_and_notes() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let secret = Secret::from("vault-test-secret");
        let words = {
            let vault = make_vault(&dir);
            let words = vault.create(&secret).await?;
            let sn = Hash::from([0xaau8; 32]);
            let entry = NoteKeyEntry::new(sn, [0xbbu8; 32], DenominationTag::D1, NoteProvenance::Cold);
            vault.store(&secret, entry).await?;
            words
        };

        // Simulate "24 words + the files": a fresh in-memory NoteVault pointed at
        // the same on-disk folder, unlocked from the words rather than a live
        // wallet-secret-wrapped `vault.key`.
        let restored = make_vault(&dir);
        let new_secret = Secret::from("a-different-wallet-secret-after-restore");
        restored.restore_key_from_words(&words, &new_secret).await?;

        let sn = Hash::from([0xaau8; 32]);
        let loaded = restored.load_key(&new_secret, &sn).await?.expect("note recovered via the words");
        assert_eq!(loaded.sk, [0xbbu8; 32]);

        Ok(())
    }

    #[tokio::test]
    async fn restore_from_words_reloads_even_if_the_index_was_already_cached_empty() -> Result<()> {
        // Reproduces the exact sequence the CLI/live restore flow uses: check the
        // (as-yet-empty, nonexistent) destination vault first — caching an empty
        // index — THEN copy real vault files in from a backup, THEN restore the
        // key from words. If `restore_key_from_words` doesn't invalidate the
        // earlier cache, the copied-in notes silently vanish.
        let source_dir = tempfile::tempdir().unwrap();
        let secret = Secret::from("vault-test-secret");
        let words = {
            let vault = make_vault(&source_dir);
            let words = vault.create(&secret).await?;
            let sn = Hash::from([0xeeu8; 32]);
            let entry = NoteKeyEntry::new(sn, [0x22u8; 32], DenominationTag::D1, NoteProvenance::Cold);
            vault.store(&secret, entry).await?;
            words
        };

        let dest_dir = tempfile::tempdir().unwrap();
        let restored = make_vault(&dest_dir);
        // Prime the cache empty, exactly like the CLI's `store_b.is_empty()` /
        // `NoteKeyStore::is_empty` check before copying anything in.
        assert!(restored.is_empty().await?);

        // Now copy the backup's files in behind the vault handle's back.
        let source_folder = make_vault(&source_dir).folder;
        std::fs::create_dir_all(&restored.folder).unwrap();
        for entry in std::fs::read_dir(&source_folder).unwrap() {
            let entry = entry.unwrap();
            let target = restored.folder.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                std::fs::create_dir_all(&target).unwrap();
                for inner in std::fs::read_dir(entry.path()).unwrap() {
                    let inner = inner.unwrap();
                    std::fs::copy(inner.path(), target.join(inner.file_name())).unwrap();
                }
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }

        let new_secret = Secret::from("a-different-wallet-secret-after-restore");
        restored.restore_key_from_words(&words, &new_secret).await?;

        assert!(!restored.is_empty().await?, "the copied-in note must be visible after restore, not shadowed by the earlier empty cache");
        let sn = Hash::from([0xeeu8; 32]);
        let loaded = restored.load_key(&new_secret, &sn).await?.expect("note recovered via the words after a stale empty cache");
        assert_eq!(loaded.sk, [0x22u8; 32]);

        Ok(())
    }

    #[tokio::test]
    async fn rebuild_manifest_recovers_after_manifest_loss() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let secret = Secret::from("vault-test-secret");
        let vault = make_vault(&dir);
        vault.create(&secret).await?;

        let sn = Hash::from([0xccu8; 32]);
        let entry = NoteKeyEntry::new(sn, [0xddu8; 32], DenominationTag::D1, NoteProvenance::Cold);
        vault.store(&secret, entry).await?;

        // Corrupt the mandatory plaintext index the way losing/truncating
        // `manifest.tsv` would (e.g. a crash mid-write) — the raw `.note` files
        // are untouched and still hold everything needed to reconstruct it.
        fs::write(&dir.path().join("test.notes/manifest.tsv"), b"").await?;

        let fresh = make_vault(&dir);
        let count = fresh.rebuild_manifest(&secret).await?;
        assert_eq!(count, 1);
        let info = fresh.load_info(&sn).await?.expect("row recovered from raw files");
        assert_eq!(info.pk, entry_pk(&sn)?);

        Ok(())
    }

    fn entry_pk(sn: &Hash) -> Result<[u8; 32]> {
        NoteKeyEntry::new(*sn, [0xddu8; 32], DenominationTag::D1, NoteProvenance::Cold).derive_pk()
    }
}
