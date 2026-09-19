use kaspa_consensus_core::{
    BlockHasher, Hash,
    notepool::{NewNote, NoteLock, PoolDiff, PoolEntry, PoolStateView},
};
use kaspa_database::prelude::{BatchDbWriter, CachePolicy, CachedDbAccess, DB, DirectDbWriter, StoreResult, StoreResultExt};
use kaspa_database::registry::DatabaseStorePrefixes;
use rocksdb::WriteBatch;
use std::sync::Arc;

/// The pool state map: `sn -> (d, pk)` (POOL-SPEC.md P5.1, PLAN P6.2). Mirrors
/// `consensus/src/model/stores/utxo_set.rs`'s `DbUtxoSetStore` — same store shape,
/// keyed by note serial instead of transaction outpoint.
pub trait NotePoolStoreReader {
    fn get(&self, sn: Hash) -> StoreResult<PoolEntry>;
    fn has(&self, sn: Hash) -> StoreResult<bool>;
}

pub trait NotePoolStore: NotePoolStoreReader {
    /// Applies a [`PoolDiff`] — adding and removing entries correspondingly.
    /// `self` is `mut` to require write access even though the compiler does not
    /// strictly require it: concurrent readers can interfere with cache consistency
    /// (see `UtxoSetStore::write_diff`'s identical comment).
    fn write_diff(&mut self, diff: &PoolDiff) -> StoreResult<()>;
}

#[derive(Clone)]
pub struct DbNotePoolStore {
    db: Arc<DB>,
    access: CachedDbAccess<Hash, NewNote, BlockHasher>,
    /// The lock of every locked note, keyed like `access` (POOL-SPEC.md P5.9). Beside
    /// the notes rather than inside their rows, so the v1.1 rows read as they are.
    locks: CachedDbAccess<Hash, NoteLock, BlockHasher>,
}

impl DbNotePoolStore {
    pub fn new(db: Arc<DB>, cache_policy: CachePolicy) -> Self {
        Self::with_prefix(db, cache_policy, DatabaseStorePrefixes::NotePoolState.into(), DatabaseStorePrefixes::NotePoolLocks.into())
    }

    /// A pool state store under an explicit prefix — used for the pruning-point-positioned
    /// copy (`DatabaseStorePrefixes::PruningNotePool`, PLAN P6.8), mirroring how
    /// `DbUtxoSetStore::new` takes its prefix so the virtual and pruning UTXO sets share
    /// one implementation.
    pub fn with_prefix(db: Arc<DB>, cache_policy: CachePolicy, prefix: Vec<u8>, locks_prefix: Vec<u8>) -> Self {
        Self {
            access: CachedDbAccess::new(Arc::clone(&db), cache_policy, prefix),
            locks: CachedDbAccess::new(Arc::clone(&db), cache_policy, locks_prefix),
            db,
        }
    }

    pub fn clear(&mut self) -> StoreResult<()> {
        self.access.delete_all(DirectDbWriter::new(&self.db))?;
        self.locks.delete_all(DirectDbWriter::new(&self.db))
    }

    fn entry(&self, sn: Hash, note: NewNote) -> PoolEntry {
        PoolEntry { note, lock: self.locks.read(sn).optional().unwrap() }
    }

    pub fn iterator(&self) -> impl Iterator<Item = Result<(Hash, PoolEntry), Box<dyn std::error::Error>>> + '_ {
        self.access.iterator().map(|res| {
            res.map(|(key, note)| {
                let sn = Hash::from_slice(key.as_ref());
                (sn, self.entry(sn, note))
            })
        })
    }

    /// Chunked, resumable iteration in ascending serial order — the pool analog of
    /// `DbUtxoSetStore::seek_iterator`, used to serve pruning-point pool state to IBD
    /// peers (PLAN P6.8).
    pub fn seek_iterator(
        &self,
        from_sn: Option<Hash>,
        limit: usize,
        skip_first: bool,
    ) -> impl Iterator<Item = Result<(Hash, PoolEntry), Box<dyn std::error::Error>>> + '_ {
        self.access.seek_iterator(None, from_sn, limit, skip_first).map(|res| {
            res.map(|(key, note)| {
                let sn = Hash::from_slice(key.as_ref());
                (sn, self.entry(sn, note))
            })
        })
    }

    /// Appends `entries` directly (no diff semantics) — used while staging a downloaded
    /// pruning-point pool state chunk by chunk (mirrors `DbUtxoSetStore::write_many`).
    pub fn write_many(&mut self, entries: &[(Hash, PoolEntry)]) -> StoreResult<()> {
        let mut writer = DirectDbWriter::new(&self.db);
        self.access.write_many(&mut writer, &mut entries.iter().map(|(sn, e)| (*sn, e.note)))?;
        self.locks.write_many(&mut writer, &mut entries.iter().filter_map(|(sn, e)| e.lock.map(|l| (*sn, l))))?;
        Ok(())
    }

    fn apply<W: kaspa_database::prelude::DbWriter>(&self, writer: &mut W, diff: &PoolDiff) -> StoreResult<()> {
        self.access.delete_many(&mut *writer, &mut diff.remove.keys().copied())?;
        self.locks.delete_many(&mut *writer, &mut diff.remove.keys().copied())?;
        self.access.write_many(&mut *writer, &mut diff.add.iter().map(|(sn, e)| (*sn, e.note)))?;
        self.locks.write_many(&mut *writer, &mut diff.add.iter().filter_map(|(sn, e)| e.lock.map(|l| (*sn, l))))?;
        Ok(())
    }

    /// Batch variant of [`NotePoolStore::write_diff`] — stages into the caller's
    /// `WriteBatch` so the virtual pool state commits atomically with the rest of the
    /// virtual state (mirrors `DbUtxoSetStore::write_diff_batch`).
    pub fn write_diff_batch(&mut self, batch: &mut WriteBatch, diff: &PoolDiff) -> StoreResult<()> {
        let mut writer = BatchDbWriter::new(batch);
        self.apply(&mut writer, diff)
    }
}

/// The virtual pool state store is the base view the composed mergeset views stack on
/// (POOL-SPEC.md P5.3, PLAN P6.4) — the pool analog of `DbUtxoSetStore: UtxoView`.
impl PoolStateView for DbNotePoolStore {
    fn get_note(&self, sn: &Hash) -> Option<PoolEntry> {
        self.access.read(*sn).optional().unwrap().map(|note| self.entry(*sn, note))
    }
}

impl NotePoolStoreReader for DbNotePoolStore {
    fn get(&self, sn: Hash) -> StoreResult<PoolEntry> {
        let note = self.access.read(sn)?;
        Ok(self.entry(sn, note))
    }

    fn has(&self, sn: Hash) -> StoreResult<bool> {
        self.access.has(sn)
    }
}

impl NotePoolStore for DbNotePoolStore {
    fn write_diff(&mut self, diff: &PoolDiff) -> StoreResult<()> {
        let mut writer = DirectDbWriter::new(&self.db);
        self.apply(&mut writer, diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_consensus_core::notepool::DenominationTag;
    use kaspa_database::create_temp_db;
    use kaspa_database::prelude::ConnBuilder;
    use std::collections::HashMap;

    fn note(byte: u8) -> PoolEntry {
        PoolEntry::unlocked(NewNote { d: DenominationTag::D1, pk: [byte; 32] })
    }

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    #[test]
    fn write_diff_applies_add_and_remove() {
        let (_lifetime, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut store = DbNotePoolStore::new(db, CachePolicy::Count(16));

        store.write_diff(&PoolDiff::new(HashMap::from([(hash(1), note(1))]), HashMap::new())).unwrap();
        assert_eq!(store.get(hash(1)).unwrap(), note(1));
        assert!(store.has(hash(1)).unwrap());

        store.write_diff(&PoolDiff::new(HashMap::new(), HashMap::from([(hash(1), note(1))]))).unwrap();
        assert!(!store.has(hash(1)).unwrap());
    }

    #[test]
    fn iterator_yields_all_entries() {
        let (_lifetime, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut store = DbNotePoolStore::new(db, CachePolicy::Count(16));

        let add = HashMap::from([(hash(1), note(1)), (hash(2), note(2)), (hash(3), note(3))]);
        store.write_diff(&PoolDiff::new(add.clone(), HashMap::new())).unwrap();

        let collected: HashMap<_, _> = store.iterator().map(|r| r.unwrap()).collect();
        assert_eq!(collected, add);
    }
}
