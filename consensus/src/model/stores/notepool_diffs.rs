use std::sync::Arc;

use kaspa_consensus_core::{BlockHasher, notepool::PoolDiff};
use kaspa_database::prelude::CachePolicy;
use kaspa_database::prelude::DB;
use kaspa_database::prelude::StoreError;
use kaspa_database::prelude::{BatchDbWriter, CachedDbAccess, DirectDbWriter};
use kaspa_database::registry::DatabaseStorePrefixes;
use kaspa_hashes::Hash;
use rocksdb::WriteBatch;

/// Store for holding the pool-state difference (delta) of a chain block relative to its
/// selected parent — the note pool's exact analog of `utxo_diffs.rs`'s `DbUtxoDiffsStore`
/// (FORK-PLAN P6.4). Kept in lockstep with that store: a block with `StatusUTXOValid` has
/// both diffs, written in the same batch (`commit_utxo_state`), so reorg walks can
/// apply/unapply the two in the same pass. The `remove` side carries each consumed note's
/// `(d, pk)`, which is what makes reversal possible at all — after removal the value
/// exists nowhere else.
pub trait NotePoolDiffsStoreReader {
    fn get(&self, hash: Hash) -> Result<Arc<PoolDiff>, StoreError>;
}

pub trait NotePoolDiffsStore: NotePoolDiffsStoreReader {
    fn insert(&self, hash: Hash, pool_diff: Arc<PoolDiff>) -> Result<(), StoreError>;
    fn delete(&self, hash: Hash) -> Result<(), StoreError>;
}

#[derive(Clone)]
pub struct DbNotePoolDiffsStore {
    db: Arc<DB>,
    access: CachedDbAccess<Hash, Arc<PoolDiff>, BlockHasher>,
}

impl DbNotePoolDiffsStore {
    pub fn new(db: Arc<DB>, cache_policy: CachePolicy) -> Self {
        Self { db: Arc::clone(&db), access: CachedDbAccess::new(db, cache_policy, DatabaseStorePrefixes::NotePoolDiffsV2.into()) }
    }

    pub fn clone_with_new_cache(&self, cache_policy: CachePolicy) -> Self {
        Self::new(Arc::clone(&self.db), cache_policy)
    }

    pub fn insert_batch(&self, batch: &mut WriteBatch, hash: Hash, pool_diff: Arc<PoolDiff>) -> Result<(), StoreError> {
        if self.access.has(hash)? {
            return Err(StoreError::HashAlreadyExists(hash));
        }
        self.access.write(BatchDbWriter::new(batch), hash, pool_diff)?;
        Ok(())
    }

    pub fn delete_batch(&self, batch: &mut WriteBatch, hash: Hash) -> Result<(), StoreError> {
        self.access.delete(BatchDbWriter::new(batch), hash)
    }
}

impl NotePoolDiffsStoreReader for DbNotePoolDiffsStore {
    fn get(&self, hash: Hash) -> Result<Arc<PoolDiff>, StoreError> {
        self.access.read(hash)
    }
}

impl NotePoolDiffsStore for DbNotePoolDiffsStore {
    fn insert(&self, hash: Hash, pool_diff: Arc<PoolDiff>) -> Result<(), StoreError> {
        if self.access.has(hash)? {
            return Err(StoreError::HashAlreadyExists(hash));
        }
        self.access.write(DirectDbWriter::new(&self.db), hash, pool_diff)?;
        Ok(())
    }

    fn delete(&self, hash: Hash) -> Result<(), StoreError> {
        self.access.delete(DirectDbWriter::new(&self.db), hash)
    }
}
