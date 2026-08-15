use kaspa_consensus_core::{BlockHasher, Hash, notepool::{NewNote, PoolDiff}};
use kaspa_database::prelude::{CachePolicy, CachedDbAccess, DB, DirectDbWriter, StoreResult};
use kaspa_database::registry::DatabaseStorePrefixes;
use std::sync::Arc;

/// The pool state map: `sn -> (d, pk)` (POOL-SPEC.md P5.1, FORK-PLAN P6.2). Mirrors
/// `consensus/src/model/stores/utxo_set.rs`'s `DbUtxoSetStore` — same store shape,
/// keyed by note serial instead of transaction outpoint.
pub trait NotePoolStoreReader {
    fn get(&self, sn: Hash) -> StoreResult<NewNote>;
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
}

impl DbNotePoolStore {
    pub fn new(db: Arc<DB>, cache_policy: CachePolicy) -> Self {
        Self { access: CachedDbAccess::new(Arc::clone(&db), cache_policy, DatabaseStorePrefixes::NotePoolState.into()), db }
    }

    pub fn clear(&mut self) -> StoreResult<()> {
        self.access.delete_all(DirectDbWriter::new(&self.db))
    }

    pub fn iterator(&self) -> impl Iterator<Item = Result<(Hash, NewNote), Box<dyn std::error::Error>>> + '_ {
        self.access.iterator().map(|res| res.map(|(key, note)| (Hash::from_slice(key.as_ref()), note)).map_err(|e| e.into()))
    }
}

impl NotePoolStoreReader for DbNotePoolStore {
    fn get(&self, sn: Hash) -> StoreResult<NewNote> {
        self.access.read(sn)
    }

    fn has(&self, sn: Hash) -> StoreResult<bool> {
        self.access.has(sn)
    }
}

impl NotePoolStore for DbNotePoolStore {
    fn write_diff(&mut self, diff: &PoolDiff) -> StoreResult<()> {
        let mut writer = DirectDbWriter::new(&self.db);
        self.access.delete_many(&mut writer, &mut diff.remove.keys().copied())?;
        self.access.write_many(&mut writer, &mut diff.add.iter().map(|(sn, note)| (*sn, *note)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_consensus_core::notepool::DenominationTag;
    use kaspa_database::create_temp_db;
    use kaspa_database::prelude::ConnBuilder;
    use std::collections::HashMap;

    fn note(byte: u8) -> NewNote {
        NewNote { d: DenominationTag::D1, pk: [byte; 32] }
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
