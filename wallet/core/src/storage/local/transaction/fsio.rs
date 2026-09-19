//!
//! Local file system transaction storage (native+NodeJS fs IO).
//!

use crate::encryption::*;
use crate::imports::*;
use crate::storage::TransactionRecord;
use crate::storage::interface::{StorageStream, TransactionRangeResult};
use crate::storage::{Binding, TransactionKind, TransactionRecordStore};
use kaspa_utils::hex::ToHex;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};
use workflow_store::fs;

pub struct Inner {
    known_folders: HashSet<String>,
}

pub struct TransactionStore {
    inner: Arc<Mutex<Inner>>,
    folder: PathBuf,
    name: String,
}

/// Records are filed under the first byte of their id: `.../ab/abcdef…`.
///
/// One flat directory was fine until a wallet that is a mining payout target
/// put 6,087,990 files in one of them. At that size ext4's directory hash
/// tree hits its two-level ceiling — `Directory index full, reach max htree
/// level: 2` — and every further create fails with `ENOSPC`, which surfaces
/// as "No space left on device" on a disk with 200 GB free. The errno is the
/// only one ext4 has for the condition; it is not about free space, and no
/// amount of deleting elsewhere helps.
///
/// 256 buckets over a uniformly distributed hash keeps each directory to a
/// few tens of thousands of entries even at the scale that broke it, which
/// every filesystem handles without comment.
fn shard_for(id: &TransactionId) -> String {
    // Transaction ids are hashes, so the leading byte is as good a spread as
    // any and needs no hashing of our own.
    id.to_hex().chars().take(2).collect()
}

/// Where a record is written. Always sharded.
fn record_path(folder: &Path, id: &TransactionId) -> PathBuf {
    folder.join(shard_for(id)).join(id.to_hex())
}

/// Where a record is read from: the shard, or the flat path beside it.
///
/// Wallets that predate sharding have their records sitting directly in the
/// network folder. They are left there rather than migrated — moving six
/// million files to fix a problem that only bites at six million files is
/// the wrong trade, and a wallet small enough for the migration to be quick
/// is a wallet that was never in trouble. Both layouts are simply readable.
fn existing_record_path(folder: &Path, id: &TransactionId) -> PathBuf {
    let sharded = record_path(folder, id);
    if sharded.exists() { sharded } else { folder.join(id.to_hex()) }
}

impl TransactionStore {
    pub fn new<P: AsRef<Path>>(folder: P, name: &str) -> TransactionStore {
        TransactionStore {
            inner: Arc::new(Mutex::new(Inner { known_folders: HashSet::default() })),
            folder: fs::resolve_path(folder.as_ref().to_str().unwrap()).expect("transaction store folder is invalid"),
            name: name.to_string(),
        }
    }

    #[inline(always)]
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    fn make_subfolder(&self, binding: &Binding, network_id: &NetworkId) -> String {
        let name = self.name.as_str();
        let binding_hex = binding.to_hex();
        let network_id = network_id.to_string();
        format!("{}/{binding_hex}/{network_id}", crate::storage::local::transactions_dir_name(name))
    }

    fn make_folder(&self, binding: &Binding, network_id: &NetworkId) -> PathBuf {
        self.folder.join(self.make_subfolder(binding, network_id))
    }

    async fn ensure_folder(&self, binding: &Binding, network_id: &NetworkId) -> Result<PathBuf> {
        let subfolder = self.make_subfolder(binding, network_id);
        let folder = self.folder.join(&subfolder);
        if !self.inner().known_folders.contains(&subfolder) {
            fs::create_dir_all(&folder).await?;
            self.inner().known_folders.insert(subfolder);
        }
        Ok(folder)
    }

    /// Every record id under this binding, newest first.
    ///
    /// `readdir` is not recursive, so the shard directories are walked
    /// explicitly. Anything sitting directly in the network folder is a
    /// record from before sharding and is included alongside them, which is
    /// what keeps an older wallet's history visible.
    async fn enumerate(&self, binding: &Binding, network_id: &NetworkId) -> Result<VecDeque<TransactionId>> {
        let folder = self.make_folder(binding, network_id);
        let top = match fs::readdir(folder.clone(), false).await {
            Ok(entries) => entries,
            Err(e) => {
                return if e.code() == Some("ENOENT") {
                    Err(Error::NoRecordsFound)
                } else {
                    log_info!("TransactionStore::enumerate(): error reading folder: {:?}", e);
                    Err(e.into())
                };
            }
        };

        // (id, when it was written) — sorting needs the time, and the time
        // needs a stat, so they are gathered together rather than stat'd
        // again during the sort.
        let mut found: Vec<(TransactionId, u64)> = Vec::new();

        let mut collect = |entries: Vec<fs::DirEntry>| {
            for file in entries {
                match TransactionId::from_hex(file.file_name()) {
                    Ok(id) => {
                        let when = file.metadata().and_then(|meta| meta.created().or_else(|| meta.modified())).unwrap_or_default();
                        found.push((id, when));
                    }
                    // A two-character name is a shard directory, not a
                    // foreign file, and saying otherwise on every listing
                    // would be 256 lines of noise.
                    Err(_) if file.file_name().len() == 2 => {}
                    Err(_) => {
                        log_error!("TransactionStore::enumerate(): filename {:?} is not a hash (foreign file?)", file);
                    }
                }
            }
        };

        let shards: Vec<String> = top.iter().map(|e| e.file_name().to_string()).filter(|name| name.len() == 2).collect();
        collect(top);
        for shard in shards {
            if let Ok(entries) = fs::readdir(folder.join(&shard), true).await {
                collect(entries);
            }
        }

        // Newest first.
        found.sort_by_key(|(_, when)| std::cmp::Reverse(*when));
        Ok(found.into_iter().map(|(id, _)| id).collect())
    }
}

#[async_trait]
impl TransactionRecordStore for TransactionStore {
    async fn transaction_id_iter(&self, binding: &Binding, network_id: &NetworkId) -> Result<StorageStream<Arc<TransactionId>>> {
        Ok(Box::pin(TransactionIdStream::try_new(self, binding, network_id).await?))
    }

    async fn transaction_data_iter(&self, binding: &Binding, network_id: &NetworkId) -> Result<StorageStream<Arc<TransactionRecord>>> {
        Ok(Box::pin(TransactionRecordStream::try_new(self, binding, network_id).await?))
    }

    async fn load_single(&self, binding: &Binding, network_id: &NetworkId, id: &TransactionId) -> Result<Arc<TransactionRecord>> {
        let folder = self.make_folder(binding, network_id);
        let path = existing_record_path(&folder, id);
        Ok(Arc::new(read(&path, None).await?))
    }

    async fn load_multiple(
        &self,
        binding: &Binding,
        network_id: &NetworkId,
        ids: &[TransactionId],
    ) -> Result<Vec<Arc<TransactionRecord>>> {
        let folder = self.ensure_folder(binding, network_id).await?;
        let mut transactions = vec![];

        for id in ids {
            let path = existing_record_path(&folder, id);
            match read(&path, None).await {
                Ok(tx) => {
                    transactions.push(Arc::new(tx));
                }
                Err(err) => {
                    log_error!("Error loading transaction {id}: {:?}", err);
                }
            }
        }

        Ok(transactions)
    }

    async fn load_range(
        &self,
        binding: &Binding,
        network_id: &NetworkId,
        filter: Option<Vec<TransactionKind>>,
        range: std::ops::Range<usize>,
    ) -> Result<TransactionRangeResult> {
        let folder = self.ensure_folder(binding, network_id).await?;
        let ids = self.enumerate(binding, network_id).await?;
        let mut transactions = vec![];

        let total = if let Some(filter) = filter {
            let mut located = 0;

            for id in &ids {
                let path = existing_record_path(&folder, id);

                match read(&path, None).await {
                    Ok(tx) => {
                        if filter.contains(&tx.kind()) {
                            if located >= range.start && located < range.end {
                                transactions.push(Arc::new(tx));
                            }

                            located += 1;
                        }
                    }
                    Err(err) => {
                        log_error!("Error loading transaction {id}: {:?}", err);
                    }
                }
            }

            located
        } else {
            let iter = ids.iter().skip(range.start).take(range.len());

            for id in iter {
                let path = existing_record_path(&folder, id);
                match read(&path, None).await {
                    Ok(tx) => {
                        transactions.push(Arc::new(tx));
                    }
                    Err(err) => {
                        log_error!("Error loading transaction {id}: {:?}", err);
                    }
                }
            }

            ids.len()
        };

        Ok(TransactionRangeResult { transactions, total: total as u64 })
    }

    async fn store(&self, transaction_records: &[&TransactionRecord]) -> Result<()> {
        for tx in transaction_records {
            let folder = self.ensure_folder(tx.binding(), tx.network_id()).await?;
            let filename = record_path(&folder, tx.id());
            // The shard directory, not the network directory: `ensure_folder`
            // caches the latter and would skip this.
            if let Some(shard) = filename.parent() {
                fs::create_dir_all(shard).await?;
            }
            write(&filename, tx, None, EncryptionKind::XChaCha20Poly1305).await?;
        }

        Ok(())
    }

    async fn remove(&self, binding: &Binding, network_id: &NetworkId, ids: &[&TransactionId]) -> Result<()> {
        let folder = self.ensure_folder(binding, network_id).await?;
        for id in ids {
            let filename = existing_record_path(&folder, id);
            fs::remove(&filename).await?;
        }

        Ok(())
    }

    async fn store_transaction_note(
        &self,
        binding: &Binding,
        network_id: &NetworkId,
        id: TransactionId,
        note: Option<String>,
    ) -> Result<()> {
        let folder = self.make_folder(binding, network_id);
        let path = existing_record_path(&folder, &id);
        let mut transaction = read(&path, None).await?;
        transaction.note = note;
        write(&path, &transaction, None, EncryptionKind::XChaCha20Poly1305).await?;
        Ok(())
    }
    async fn store_transaction_metadata(
        &self,
        binding: &Binding,
        network_id: &NetworkId,
        id: TransactionId,
        metadata: Option<String>,
    ) -> Result<()> {
        let folder = self.make_folder(binding, network_id);
        let path = existing_record_path(&folder, &id);
        let mut transaction = read(&path, None).await?;
        transaction.metadata = metadata;
        write(&path, &transaction, None, EncryptionKind::XChaCha20Poly1305).await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct TransactionIdStream {
    transactions: VecDeque<TransactionId>,
}

impl TransactionIdStream {
    pub(crate) async fn try_new(store: &TransactionStore, binding: &Binding, network_id: &NetworkId) -> Result<Self> {
        let transactions = store.enumerate(binding, network_id).await?;
        Ok(Self { transactions })
    }
}

impl Stream for TransactionIdStream {
    type Item = Result<Arc<TransactionId>>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.transactions.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(self.transactions.pop_front().map(Arc::new).unwrap())))
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.transactions.len(), Some(self.transactions.len()))
    }
}

#[derive(Clone)]
pub struct TransactionRecordStream {
    transactions: VecDeque<TransactionId>,
    folder: PathBuf,
}

impl TransactionRecordStream {
    pub(crate) async fn try_new(store: &TransactionStore, binding: &Binding, network_id: &NetworkId) -> Result<Self> {
        let folder = store.make_folder(binding, network_id);
        let transactions = store.enumerate(binding, network_id).await?;
        Ok(Self { transactions, folder })
    }
}

impl Stream for TransactionRecordStream {
    type Item = Result<Arc<TransactionRecord>>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.transactions.is_empty() {
            Poll::Ready(None)
        } else {
            let id = self.transactions.pop_front().unwrap();
            let path = existing_record_path(&self.folder, &id);
            match read_sync(&path, None) {
                Ok(transaction_data) => Poll::Ready(Some(Ok(Arc::new(transaction_data)))),
                Err(err) => Poll::Ready(Some(Err(err))),
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.transactions.len(), Some(self.transactions.len()))
    }
}

async fn read(path: &Path, secret: Option<&Secret>) -> Result<TransactionRecord> {
    let bytes = fs::read(path).await?;
    let encryptable = Encryptable::<TransactionRecord>::try_from_slice(bytes.as_slice())?;
    Ok(encryptable.decrypt(secret)?.unwrap())
}

fn read_sync(path: &Path, secret: Option<&Secret>) -> Result<TransactionRecord> {
    let bytes = fs::read_sync(path)?;
    let encryptable = Encryptable::<TransactionRecord>::try_from_slice(bytes.as_slice())?;
    Ok(encryptable.decrypt(secret)?.unwrap())
}

async fn write(path: &Path, record: &TransactionRecord, secret: Option<&Secret>, encryption_kind: EncryptionKind) -> Result<()> {
    let data = if let Some(secret) = secret {
        Encryptable::from(record.clone()).into_encrypted(secret, encryption_kind)?
    } else {
        Encryptable::from(record.clone())
    };
    fs::write(path, &borsh::to_vec(&data)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Binding;
    use crate::storage::transaction::{TransactionData, UtxoRecord};
    use crate::utxo::UtxoContextId;
    use kaspa_consensus_core::network::NetworkType;

    fn a_record(id_byte: u8, binding: &Binding, network_id: &NetworkId) -> TransactionRecord {
        TransactionRecord {
            id: TransactionId::from_bytes([id_byte; 32]),
            unixtime_msec: Some(1_757_000_000_000),
            value: 100,
            binding: binding.clone(),
            block_daa_score: 1,
            network_id: *network_id,
            transaction_data: TransactionData::Incoming { aggregate_input_value: 100, utxo_entries: Vec::<UtxoRecord>::new() },
            note: None,
            metadata: None,
        }
    }

    fn a_store(dir: &tempfile::TempDir) -> (TransactionStore, Binding, NetworkId) {
        let store = TransactionStore::new(dir.path(), "test");
        let binding = Binding::Custom(UtxoContextId::default());
        let network_id = NetworkId::with_suffix(NetworkType::Testnet, 10);
        (store, binding, network_id)
    }

    fn network_folder(dir: &tempfile::TempDir, binding: &Binding, network_id: &NetworkId) -> PathBuf {
        dir.path().join(crate::storage::local::transactions_dir_name("test")).join(binding.to_hex()).join(network_id.to_string())
    }

    /// The whole point: a record must not land directly in the network
    /// folder, because six million of those is what wedged ext4's directory
    /// index in the first place.
    #[tokio::test]
    async fn a_record_is_written_into_a_shard_and_read_back() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (store, binding, network_id) = a_store(&dir);
        let record = a_record(0xab, &binding, &network_id);

        store.store(&[&record]).await?;

        let folder = network_folder(&dir, &binding, &network_id);
        let id_hex = record.id.to_hex();
        assert!(folder.join("ab").join(&id_hex).is_file(), "the record belongs under its first byte");
        assert!(!folder.join(&id_hex).is_file(), "and not loose in the network folder");

        let loaded = store.load_single(&binding, &network_id, &record.id).await?;
        assert_eq!(loaded.id, record.id);
        assert_eq!(loaded.value, 100);
        Ok(())
    }

    /// Wallets that predate sharding keep their records loose in the network
    /// folder. They are not migrated, so they have to stay readable — if this
    /// breaks, existing history silently disappears.
    #[tokio::test]
    async fn a_record_from_before_sharding_is_still_found() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (store, binding, network_id) = a_store(&dir);
        let old = a_record(0x11, &binding, &network_id);
        let new = a_record(0x22, &binding, &network_id);

        // Written the way the old code wrote it: straight into the folder.
        let folder = network_folder(&dir, &binding, &network_id);
        fs::create_dir_all(&folder).await?;
        write(&folder.join(old.id.to_hex()), &old, None, EncryptionKind::XChaCha20Poly1305).await?;

        store.store(&[&new]).await?;

        assert_eq!(store.load_single(&binding, &network_id, &old.id).await?.id, old.id, "the old one still loads");
        assert_eq!(store.load_single(&binding, &network_id, &new.id).await?.id, new.id);

        // And both show up in a listing, which is what `history` walks.
        let listed = store.enumerate(&binding, &network_id).await?;
        assert_eq!(listed.len(), 2, "a listing spans both layouts");
        assert!(listed.contains(&old.id) && listed.contains(&new.id));
        Ok(())
    }

    /// A shard directory is not a stray file, and must not be logged as one
    /// 256 times per listing.
    #[tokio::test]
    async fn many_records_spread_across_shards_and_all_come_back() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (store, binding, network_id) = a_store(&dir);

        let records: Vec<TransactionRecord> = (0..=255u8).map(|b| a_record(b, &binding, &network_id)).collect();
        let refs: Vec<&TransactionRecord> = records.iter().collect();
        store.store(&refs).await?;

        let folder = network_folder(&dir, &binding, &network_id);
        let shards = std::fs::read_dir(&folder)?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count();
        assert_eq!(shards, 256, "an id per leading byte fills every bucket");

        let mut loose = std::fs::read_dir(&folder)?.filter_map(|e| e.ok()).filter(|e| e.path().is_file()).peekable();
        assert!(loose.peek().is_none(), "nothing is left in the network folder itself");

        let listed = store.enumerate(&binding, &network_id).await?;
        assert_eq!(listed.len(), 256, "every record is enumerated exactly once");
        Ok(())
    }

    /// Removal has to find a record wherever it actually lives, or
    /// `history clear`-style work leaves the files behind.
    #[tokio::test]
    async fn removal_finds_a_record_in_either_layout() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (store, binding, network_id) = a_store(&dir);
        let old = a_record(0x33, &binding, &network_id);
        let new = a_record(0x44, &binding, &network_id);

        let folder = network_folder(&dir, &binding, &network_id);
        fs::create_dir_all(&folder).await?;
        write(&folder.join(old.id.to_hex()), &old, None, EncryptionKind::XChaCha20Poly1305).await?;
        store.store(&[&new]).await?;

        store.remove(&binding, &network_id, &[&old.id, &new.id]).await?;
        assert!(!folder.join(old.id.to_hex()).exists());
        assert!(!folder.join("44").join(new.id.to_hex()).exists());
        Ok(())
    }

    /// Editing a note must rewrite the record where it is, not silently
    /// duplicate it into the other layout.
    #[tokio::test]
    async fn annotating_an_old_record_does_not_duplicate_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (store, binding, network_id) = a_store(&dir);
        let old = a_record(0x55, &binding, &network_id);

        let folder = network_folder(&dir, &binding, &network_id);
        fs::create_dir_all(&folder).await?;
        write(&folder.join(old.id.to_hex()), &old, None, EncryptionKind::XChaCha20Poly1305).await?;

        store.store_transaction_note(&binding, &network_id, old.id, Some("paid the plumber".into())).await?;

        assert!(folder.join(old.id.to_hex()).is_file(), "it stays where it was");
        assert!(!folder.join("55").join(old.id.to_hex()).exists(), "and does not gain a second copy");
        assert_eq!(store.enumerate(&binding, &network_id).await?.len(), 1);
        assert_eq!(store.load_single(&binding, &network_id, &old.id).await?.note.as_deref(), Some("paid the plumber"));
        Ok(())
    }
}
