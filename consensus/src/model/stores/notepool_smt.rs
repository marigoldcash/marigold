use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::{PoolDiff, leaf_hash};
use kaspa_database::prelude::{BatchDbWriter, CachePolicy, CachedDbAccess, CachedDbItem, DB, StoreError, StoreResult};
use kaspa_database::registry::DatabaseStorePrefixes;
use kaspa_hashes::{NotePoolSmt, ZERO_HASH};
use kaspa_smt::SmtHasher;
use kaspa_smt::store::{BranchKey, Node, SmtStore, SortedLeafUpdates};
use kaspa_smt::tree::compute_root_update;
use kaspa_utils::mem_size::MemSizeEstimator;
use rocksdb::WriteBatch;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The pool's SMT commitment (POOL-SPEC.md P5.1, FORK-PLAN P6.2): a single current-state
/// tree over `sn -> leaf_hash(d, pk)`, plus the RocksDB-backed branch-node storage
/// [`compute_root_update`] needs to update it incrementally. Deliberately NOT built on
/// `consensus/smt-store`'s `SmtProcessor`/`BranchVersionKey` apparatus — that crate solves
/// a harder problem (block-versioned, multi-lane SMT state for seq-commit); the pool needs
/// only current-state apply/unapply, the same discipline `DbUtxoSetStore`/`UtxoDiff` use.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct NotePoolBranchKey([u8; 33]);

impl From<BranchKey> for NotePoolBranchKey {
    fn from(k: BranchKey) -> Self {
        let mut bytes = [0u8; 33];
        bytes[0] = k.depth;
        bytes[1..].copy_from_slice(k.node_key.as_bytes().as_ref());
        Self(bytes)
    }
}

impl AsRef<[u8]> for NotePoolBranchKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Display for NotePoolBranchKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "depth={} node_key={}", self.0[0], Hash::from_slice(&self.0[1..]))
    }
}

/// Wraps [`Node`]'s hand-rolled `to_bytes`/`from_bytes` (length-discriminated, no serde
/// derive) so it can ride `CachedDbAccess`'s bincode-based (de)serialization.
#[derive(Clone)]
struct NodeBytes(Vec<u8>);

impl Serialize for NodeBytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for NodeBytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(Vec::<u8>::deserialize(deserializer)?))
    }
}

impl MemSizeEstimator for NodeBytes {
    fn estimate_mem_bytes(&self) -> usize {
        self.0.len()
    }
}

impl From<Node> for NodeBytes {
    fn from(n: Node) -> Self {
        Self(n.to_bytes())
    }
}

impl NodeBytes {
    fn into_node(self) -> Node {
        Node::from_bytes(&self.0).expect("only ever written via Node::to_bytes")
    }
}

#[derive(Clone)]
pub struct DbNotePoolSmtStore {
    db: Arc<DB>,
    access: CachedDbAccess<NotePoolBranchKey, NodeBytes>,
    root: CachedDbItem<Hash>,
}

impl DbNotePoolSmtStore {
    pub fn new(db: Arc<DB>, cache_policy: CachePolicy) -> Self {
        Self {
            access: CachedDbAccess::new(Arc::clone(&db), cache_policy, DatabaseStorePrefixes::NotePoolSmtBranches.into()),
            root: CachedDbItem::new(Arc::clone(&db), vec![DatabaseStorePrefixes::NotePoolSmtRoot.into()]),
            db,
        }
    }

    /// The current commitment root. An empty (never-applied) store has no persisted row,
    /// so this falls back to the canonical empty-tree root rather than erroring.
    pub fn current_root(&self) -> StoreResult<Hash> {
        match self.root.read() {
            Ok(root) => Ok(root),
            Err(StoreError::KeyNotFound(_)) => Ok(NotePoolSmt::empty_root()),
            Err(e) => Err(e),
        }
    }

    /// Applies `updates` (`sn -> leaf_hash`, `ZERO_HASH` meaning "remove") to the tree,
    /// staging both the resulting branch-node changes and the new root into `batch`
    /// (caller commits), and returns the new root. Pure add/remove-note callers should
    /// go through [`Self::apply_note_diff`]/[`Self::apply_diff_batch`]; this lower-level
    /// entry point exists so apply and unapply (the reversed diff) share one code path.
    ///
    /// NOTE: `CachedDbAccess`/`CachedDbItem` update their in-memory caches immediately,
    /// before the batch commits — the standard pattern for batch writes in this codebase
    /// (`DbUtxoSetStore::write_diff_batch` behaves identically), safe under the caller's
    /// write lock.
    fn apply_leaf_updates_batch(&mut self, batch: &mut WriteBatch, updates: BTreeMap<Hash, Hash>) -> StoreResult<Hash> {
        let current_root = self.current_root()?;
        let leaf_updates = SortedLeafUpdates::from_sorted_map(&updates, |_, leaf_hash| *leaf_hash);
        let (new_root, changes) = compute_root_update::<NotePoolSmt, Self>(self, current_root, leaf_updates)?;

        for (key, node) in changes {
            match node {
                Some(n) => self.access.write(BatchDbWriter::new(batch), key.into(), NodeBytes::from(n))?,
                None => self.access.delete(BatchDbWriter::new(batch), key.into())?,
            }
        }
        self.root.write(BatchDbWriter::new(batch), &new_root)?;
        Ok(new_root)
    }

    fn apply_leaf_updates(&mut self, updates: BTreeMap<Hash, Hash>) -> StoreResult<Hash> {
        let mut batch = WriteBatch::default();
        let new_root = self.apply_leaf_updates_batch(&mut batch, updates)?;
        self.db.write(batch)?;
        Ok(new_root)
    }

    /// Applies a pool state diff to the commitment: `add` entries set their leaf to
    /// `leaf_hash(d, pk)`, `remove` entries clear their leaf. Pass `diff.to_reversed()`
    /// (or an equivalent add/remove swap) to unapply — see `PoolDiff::to_reversed`.
    pub fn apply_note_diff(&mut self, add: impl IntoIterator<Item = (Hash, Hash)>, remove: impl IntoIterator<Item = Hash>) -> StoreResult<Hash> {
        let mut updates = BTreeMap::new();
        for sn in remove {
            updates.insert(sn, ZERO_HASH);
        }
        for (sn, leaf_hash) in add {
            updates.insert(sn, leaf_hash);
        }
        self.apply_leaf_updates(updates)
    }

    /// Applies a [`PoolDiff`] directly: `add`'s `NewNote`s are hashed via
    /// [`kaspa_consensus_core::notepool::leaf_hash`] and set, `remove`'s serials are cleared.
    pub fn apply_diff(&mut self, diff: &PoolDiff) -> StoreResult<Hash> {
        self.apply_note_diff(diff.add.iter().map(|(sn, note)| (*sn, leaf_hash(note.d, &note.pk))), diff.remove.keys().copied())
    }

    /// Batch variant of [`Self::apply_diff`] — stages into the caller's `WriteBatch` so
    /// the pool commitment commits atomically with the rest of the virtual state.
    /// To unapply, pass the reversed diff (`PoolDiff::to_reversed`/`as_reversed`).
    pub fn apply_diff_batch(&mut self, batch: &mut WriteBatch, diff: &PoolDiff) -> StoreResult<Hash> {
        let mut updates = BTreeMap::new();
        for sn in diff.remove.keys() {
            updates.insert(*sn, ZERO_HASH);
        }
        for (sn, note) in diff.add.iter() {
            updates.insert(*sn, leaf_hash(note.d, &note.pk));
        }
        self.apply_leaf_updates_batch(batch, updates)
    }

    /// The exact inverse of [`Self::apply_diff`] — applies `diff`'s reversal (add/remove
    /// swapped), restoring the root that preceded the original `apply_diff(diff)` call.
    pub fn unapply_diff(&mut self, diff: &PoolDiff) -> StoreResult<Hash> {
        self.apply_diff(&diff.clone().to_reversed())
    }
}

impl SmtStore for DbNotePoolSmtStore {
    type Error = StoreError;

    fn get_node(&self, key: &BranchKey) -> Result<Option<Node>, Self::Error> {
        match self.access.read((*key).into()) {
            Ok(node_bytes) => Ok(Some(node_bytes.into_node())),
            Err(StoreError::KeyNotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_database::create_temp_db;
    use kaspa_database::prelude::ConnBuilder;
    use kaspa_hashes::{HasherBase, NotePoolLeafHash};

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn leaf(byte: u8) -> Hash {
        let mut hasher = NotePoolLeafHash::new();
        hasher.update([byte]);
        hasher.finalize()
    }

    #[test]
    fn empty_store_root_is_canonical_empty_root() {
        let (_lifetime, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let store = DbNotePoolSmtStore::new(db, CachePolicy::Count(16));
        assert_eq!(store.current_root().unwrap(), NotePoolSmt::empty_root());
    }

    #[test]
    fn apply_then_unapply_restores_prior_root() {
        let (_lifetime, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut store = DbNotePoolSmtStore::new(db, CachePolicy::Count(16));

        let root0 = store.current_root().unwrap();
        let add = vec![(hash(1), leaf(1)), (hash(2), leaf(2)), (hash(3), leaf(3))];

        let root1 = store.apply_note_diff(add.clone(), std::iter::empty()).unwrap();
        assert_ne!(root1, root0);

        // Unapply: remove exactly what was added.
        let root2 = store.apply_note_diff(std::iter::empty(), add.iter().map(|(sn, _)| *sn)).unwrap();
        assert_eq!(root2, root0);
    }

    #[test]
    fn commitment_is_deterministic_across_insertion_order() {
        let (_lifetime, db1) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut store1 = DbNotePoolSmtStore::new(db1, CachePolicy::Count(16));
        let root_a = store1.apply_note_diff(vec![(hash(1), leaf(1)), (hash(2), leaf(2)), (hash(3), leaf(3))], std::iter::empty()).unwrap();

        let (_lifetime, db2) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut store2 = DbNotePoolSmtStore::new(db2, CachePolicy::Count(16));
        let root_b = store2.apply_note_diff(vec![(hash(3), leaf(3)), (hash(1), leaf(1)), (hash(2), leaf(2))], std::iter::empty()).unwrap();

        assert_eq!(root_a, root_b);
    }

    #[test]
    fn pool_diff_apply_then_unapply_restores_prior_root() {
        use kaspa_consensus_core::notepool::{DenominationTag, NewNote};
        use std::collections::HashMap;

        let (_lifetime, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut store = DbNotePoolSmtStore::new(db, CachePolicy::Count(16));

        let note = |byte: u8| NewNote { d: DenominationTag::D1, pk: [byte; 32] };
        let root0 = store.current_root().unwrap();

        let diff = PoolDiff::new(HashMap::from([(hash(1), note(1)), (hash(2), note(2))]), HashMap::new());
        let root1 = store.apply_diff(&diff).unwrap();
        assert_ne!(root1, root0);

        let root2 = store.unapply_diff(&diff).unwrap();
        assert_eq!(root2, root0);
    }

    #[test]
    fn root_persists_across_store_instances() {
        let (_lifetime, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut store = DbNotePoolSmtStore::new(Arc::clone(&db), CachePolicy::Count(16));
        let root = store.apply_note_diff(vec![(hash(1), leaf(1))], std::iter::empty()).unwrap();

        let reopened = DbNotePoolSmtStore::new(db, CachePolicy::Count(16));
        assert_eq!(reopened.current_root().unwrap(), root);
    }
}

/// Regression guard distilled from a P6.4 pipeline-test investigation: the committed
/// root must be a pure function of the final leaf set, independent of which apply/unapply
/// history produced it (no stale branch nodes surviving between applications).
#[cfg(test)]
mod history_independence_tests {
    use super::*;
    use kaspa_consensus_core::notepool::{DenominationTag, NewNote, PoolDiff};
    use kaspa_database::create_temp_db;
    use kaspa_database::prelude::ConnBuilder;
    use std::collections::HashMap;

    #[test]
    fn same_final_leaf_set_same_root_regardless_of_history() {
        let note = |b: u8| NewNote { d: DenominationTag::D1, pk: [b; 32] };
        let h = |b: u8| Hash::from_bytes([b; 32]);
        let diff = |add: &[u8], remove: &[u8]| {
            PoolDiff::new(
                add.iter().map(|&b| (h(b), note(b))).collect::<HashMap<_, _>>(),
                remove.iter().map(|&b| (h(b), note(b))).collect::<HashMap<_, _>>(),
            )
        };

        // History A: +1; (-1, +2); (-2, +3)
        let (_l1, db1) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut s1 = DbNotePoolSmtStore::new(db1, CachePolicy::Count(16));
        s1.apply_diff(&diff(&[1], &[])).unwrap();
        s1.apply_diff(&diff(&[2], &[1])).unwrap();
        let root_a = s1.apply_diff(&diff(&[3], &[2])).unwrap();

        // History B: +1; (-1, +3)
        let (_l2, db2) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut s2 = DbNotePoolSmtStore::new(db2, CachePolicy::Count(16));
        s2.apply_diff(&diff(&[1], &[])).unwrap();
        let root_b = s2.apply_diff(&diff(&[3], &[1])).unwrap();

        assert_eq!(root_a, root_b, "same final leaf set must give same root regardless of history");
    }
}
