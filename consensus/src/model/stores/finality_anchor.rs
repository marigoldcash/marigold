//! Finality-anchor node state (POOL-SPEC.md P5.8, FORK-PLAN P6.11): the latest-anchor
//! ratchet and the trustee deny-list.
//!
//! Both items are deliberately **monotone** — appended to, never rolled back — which is
//! what lets them live outside the diff-based POV state machinery the UTXO set and note
//! pool need:
//!
//! - **The ratchet** ("a node persists the highest-scoring valid anchor it has ever
//!   accepted and never adopts a chain conflicting with it — across restarts, resyncs,
//!   and reorgs", P5.8's rollback-resistance rule) is monotone by definition.
//! - **Deny-list entries** are keyed by the chain block whose accepted equivocation
//!   evidence produced them. POV scoping ("disqualification takes effect ... in
//!   validation contexts whose POV chain includes the block containing the accepted
//!   evidence") then falls out of a reachability check at read time: an entry simply
//!   doesn't apply in chains that don't contain its accepting block, so reorgs need no
//!   entry deletion — a reorged-out entry is inert by keying, and permanence ("no
//!   un-disqualify mechanism short of a hard fork") holds because nothing ever
//!   deletes entries.
//!
//! Neither item is per-block data, so pruning never touches them (surviving pruning
//! is the trivial case here, unlike the pool state's P5.4/P6.8 machinery).

use kaspa_consensus_core::Hash;
use kaspa_database::prelude::{BatchDbWriter, CachedDbItem, DB, StoreResult, StoreResultExt};
use kaspa_database::registry::DatabaseStorePrefixes;
use rocksdb::WriteBatch;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The persisted form of the node's latest accepted anchor. Only the certified
/// (block, score) pair — the signatures were verified before acceptance and are
/// never needed again (re-serving anchors to peers is P6.12's gossip cache, not this).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredAnchor {
    pub anchored_block: Hash,
    pub anchored_daa_score: u64,
}

/// One permanent trustee disqualification: `trustee_index`'s key stops counting toward
/// anchor quorums in every validation context whose POV chain includes
/// `accepting_block` (the chain block whose acceptance data carries the evidence).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DenyListEntry {
    pub trustee_index: u8,
    pub accepting_block: Hash,
}

/// DB store for both items. Held by consensus storage as a sibling of the virtual
/// stores; writes join `commit_virtual_state`'s batch so anchor state and virtual
/// state land atomically.
#[derive(Clone)]
pub struct DbFinalityAnchorStore {
    latest: CachedDbItem<StoredAnchor>,
    deny_list: CachedDbItem<Vec<DenyListEntry>>,
}

impl DbFinalityAnchorStore {
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            latest: CachedDbItem::new(db.clone(), DatabaseStorePrefixes::FinalityAnchorLatest.into()),
            deny_list: CachedDbItem::new(db, DatabaseStorePrefixes::FinalityAnchorDenyList.into()),
        }
    }

    /// The ratchet's current anchor, if any anchor has ever been accepted.
    pub fn latest(&self) -> StoreResult<Option<StoredAnchor>> {
        self.latest.read().optional()
    }

    /// Advances the ratchet. Callers must only pass anchors with a strictly higher
    /// `anchored_daa_score` than [`Self::latest`] — the ratchet never moves backward.
    pub fn set_latest_batch(&mut self, batch: &mut WriteBatch, anchor: StoredAnchor) -> StoreResult<()> {
        self.latest.write(BatchDbWriter::new(batch), &anchor)
    }

    /// All deny-list entries ever recorded. Which of them bind in a given context is a
    /// per-POV reachability question answered by the caller; the raw list is tiny by
    /// construction (at most a handful of evidence acceptances against 5 keys, ever).
    pub fn deny_list(&self) -> StoreResult<Vec<DenyListEntry>> {
        Ok(self.deny_list.read().optional()?.unwrap_or_default())
    }

    /// Appends deny-list entries (idempotent duplicates are the caller's concern —
    /// exact-duplicate entries are skipped here as a cheap invariant).
    pub fn append_deny_entries_batch(&mut self, batch: &mut WriteBatch, new_entries: &[DenyListEntry]) -> StoreResult<()> {
        let mut entries = self.deny_list()?;
        for entry in new_entries {
            if !entries.contains(entry) {
                entries.push(*entry);
            }
        }
        self.deny_list.write(BatchDbWriter::new(batch), &entries)
    }
}
