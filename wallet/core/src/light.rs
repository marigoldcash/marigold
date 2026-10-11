//! The light wallet's own copy of the pool (PLAN P8.7, docs/marigold/SYNC-LIGHT.md).
//!
//! A wallet that keeps no node of its own still has to know which notes are
//! live and under which key, and it must not find that out by naming its
//! notes to a public computer. So it takes the whole pool instead, which is
//! small, and checks it against the chain in two steps that need no trust in
//! the computer that handed it over:
//!
//! 1. The trustees' latest finality anchor names a block and its DAA score and
//!    carries the trustees' signatures. The wallet holds the trustees' keys
//!    (pinned in the network's parameters, the same keys every node checks
//!    anchors with) and verifies the signatures itself.
//! 2. That block's header commits to the pool state as of the block, after
//!    its mergeset, in `pool_commitment`. The wallet fetches the header,
//!    hashes it to make sure it is the block the anchor names, fetches the
//!    pool state as of that block, rebuilds the commitment from the entries
//!    and compares. A computer that lies about the state fails the check.
//!
//! From then on the wallet follows the chain: every few seconds it asks what
//! chain blocks were added and removed since the block it is at, and asks for
//! their pool diffs in one call, applying the removed ones in reverse and the
//! added ones in order while keeping the commitment tree up to date. Every new
//! anchor is checked the same way as the first: the commitment the wallet
//! computed for the anchored block must equal the signed block's. Any
//! disagreement, or a gap too long to catch up through diffs, starts the
//! wallet over from the latest anchor, which is minutes of work.
//!
//! What a note lookup costs with this in place: a hash-map read, and nobody
//! learns which notes the wallet asked about.

use crate::error::Error;
use crate::result::Result;
use crate::rpc::DynRpcApi;
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::finality_anchor::{FinalityAnchor, TrusteeKeys, verify_anchor};
use kaspa_consensus_core::header::{CompressedParents, Header};
use kaspa_consensus_core::network::NetworkId;
use kaspa_consensus_core::notepool::hashing::leaf_hash_entry;
use kaspa_consensus_core::notepool::{DenominationTag, NewNote, NoteLock, PoolEntry};
use kaspa_hashes::ZERO_HASH;
use kaspa_hashes::{Hash, NotePoolSmt};
use kaspa_rpc_core::{RpcHeader, RpcNoteEntry, RpcNoteLock, RpcPoolDiff};
use kaspa_smt::SmtHasher;
use kaspa_smt::store::{BTreeSmtStore, LeafUpdate, SortedLeafUpdates};
use kaspa_smt::tree::compute_root_update;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use workflow_core::time::unixtime_as_millis_u64;

/// How many chain blocks the wallet will catch up through diffs before it
/// prefers to start over from the latest anchor: at ten blocks a second this
/// is half an hour away from the chain.
const CATCH_UP_LIMIT: usize = 18_000;
/// The node answers at most this many diffs per call.
const DIFFS_PER_CALL: usize = 2_000;
/// How many recent (chain block, commitment) pairs are kept so a fresh anchor
/// can be checked against what the wallet computed when that block arrived:
/// anchors sit about 600 blocks back, the margin is for a slow check.
const ROOTS_KEPT: usize = 6_000;
/// How often the anchor is checked, in milliseconds. Anchors arrive about
/// every thirty seconds at launch cadence.
const ANCHOR_CHECK_EVERY_MS: u64 = 20_000;

/// The state and where it stands.
struct LightState {
    entries: HashMap<Hash, PoolEntry>,
    tree: LightTree,
    /// The chain block the state is as of, after its mergeset.
    at: Hash,
    at_daa: u64,
    /// The last anchor the state was checked against.
    anchored: Hash,
    anchored_daa: u64,
    checked_ms: u64,
    /// (chain block, commitment after it), newest last.
    recent_roots: VecDeque<(Hash, Hash)>,
}

/// The commitment tree over the entries, kept up to date one diff at a time
/// the way the node keeps its own: a batch of leaf updates, the changed
/// nodes written back, the new root remembered.
struct LightTree {
    store: BTreeSmtStore,
    root: Hash,
}

impl LightTree {
    fn new() -> Self {
        Self { store: BTreeSmtStore::new(), root: NotePoolSmt::empty_root() }
    }

    fn root(&self) -> Hash {
        self.root
    }

    /// `leaf_hash` ZERO removes the leaf.
    fn update(&mut self, updates: impl IntoIterator<Item = (Hash, Hash)>) {
        let updates: Vec<(Hash, Hash)> = updates.into_iter().collect();
        if updates.is_empty() {
            return;
        }
        for (key, leaf_hash) in updates.iter() {
            self.store.insert_leaf(*key, *leaf_hash);
        }
        let sorted =
            SortedLeafUpdates::from_unsorted(updates.iter().map(|(key, leaf_hash)| LeafUpdate { key: *key, leaf_hash: *leaf_hash }));
        let (root, changes) =
            compute_root_update::<NotePoolSmt, _>(&self.store, self.root, sorted).expect("an in-memory store cannot fail");
        for (branch, node) in changes.iter() {
            self.store.insert_node(*branch, *node);
        }
        self.root = root;
    }
}

/// What a follow step did.
#[derive(Clone, Copy, Debug, Default)]
pub struct Followed {
    pub added: usize,
    pub removed: usize,
    /// The state was fetched afresh from the latest anchor.
    pub restarted: bool,
    /// A new anchor was checked and agreed.
    pub anchor_checked: bool,
}

/// Where the light wallet stands, for status lines.
#[derive(Clone, Debug)]
pub struct LightStatus {
    pub at: Hash,
    pub at_daa: u64,
    pub anchored: Hash,
    pub anchored_daa: u64,
    pub notes: usize,
    pub checked_ms: u64,
}

pub struct LightPool {
    network_id: NetworkId,
    trustees: TrusteeKeys,
    state: Mutex<LightState>,
}

impl LightPool {
    /// Whether a light wallet can run on this network: it needs trustees to
    /// check anchors against. Mainnet has none until the P9.1 ceremony.
    pub fn supported(network_id: NetworkId) -> bool {
        Params::from(network_id).finality_anchor.trustees.is_some()
    }

    /// Takes the pool state as of the latest anchor and checks it. Minutes of
    /// nothing on a small pool; the fetch is the only cost.
    pub async fn start(rpc: &Arc<DynRpcApi>, network_id: NetworkId) -> Result<Arc<Self>> {
        let trustees = Params::from(network_id).finality_anchor.trustees.ok_or_else(|| {
            Error::Custom("this network has no trustees pinned, so a light wallet cannot check an anchor".to_string())
        })?;
        let (anchor, header) = checked_anchor(rpc, &trustees).await?;
        let state = fetch_state(rpc, &anchor, &header).await?;
        Ok(Arc::new(Self { network_id, trustees, state: Mutex::new(state) }))
    }

    pub fn network_id(&self) -> NetworkId {
        self.network_id
    }

    /// One step of following the chain: catch up through diffs, and check the
    /// anchor when one is due. Call it every couple of seconds.
    pub async fn follow(&self, rpc: &Arc<DynRpcApi>) -> Result<Followed> {
        let mut done = Followed::default();
        let at = self.state.lock().unwrap().at;
        let chain = match rpc.get_virtual_chain_from_block(at, false, None).await {
            Ok(chain) => chain,
            // The block the wallet is at is gone from the node's window (a
            // long absence, or a different node): start over.
            Err(_) => {
                self.restart(rpc).await?;
                done.restarted = true;
                return Ok(done);
            }
        };
        let removed = chain.removed_chain_block_hashes;
        let added = chain.added_chain_block_hashes;
        if added.len() > CATCH_UP_LIMIT || (!removed.is_empty() && added.is_empty()) {
            self.restart(rpc).await?;
            done.restarted = true;
            return Ok(done);
        }
        if !removed.is_empty() {
            // Given from the block the wallet is at downwards: that is the
            // order to take them back out in.
            let diffs = fetch_diffs(rpc, &removed).await?;
            let mut state = self.state.lock().unwrap();
            for diff in diffs.iter() {
                unapply(&mut state, diff);
            }
            // The commitments recorded for the removed blocks are void.
            state.recent_roots.retain(|(block, _)| !removed.contains(block));
            done.removed = removed.len();
        }
        if !added.is_empty() {
            let diffs = fetch_diffs(rpc, &added).await?;
            let mut state = self.state.lock().unwrap();
            for diff in diffs.iter() {
                apply(&mut state, diff);
                let root = state.tree.root();
                state.recent_roots.push_back((diff.block, root));
                while state.recent_roots.len() > ROOTS_KEPT {
                    state.recent_roots.pop_front();
                }
            }
            if let Some(last) = added.last() {
                state.at = *last;
            }
            done.added = added.len();
        }
        if let Some(last) = added.last() {
            // The DAA score of where the wallet is now, for status lines; one
            // header fetch per step, not per block.
            if let Ok(block) = rpc.get_block(*last, false).await {
                self.state.lock().unwrap().at_daa = block.header.daa_score;
            }
        }
        let due = unixtime_as_millis_u64().saturating_sub(self.state.lock().unwrap().checked_ms) >= ANCHOR_CHECK_EVERY_MS;
        if due {
            match self.check_anchor(rpc).await? {
                AnchorCheck::Agreed => done.anchor_checked = true,
                AnchorCheck::NotYetReached => {}
                AnchorCheck::Disagreed => {
                    self.restart(rpc).await?;
                    done.restarted = true;
                }
            }
        }
        Ok(done)
    }

    /// The notes among `serials` that are live, in the shape a node answers
    /// `get_notes_by_serial` with, so callers need not know the difference.
    pub fn notes(&self, serials: &[Hash]) -> Vec<RpcNoteEntry> {
        let state = self.state.lock().unwrap();
        serials.iter().filter_map(|sn| state.entries.get(sn).map(|e| rpc_entry(*sn, e))).collect()
    }

    /// The live notes under any of `pks`: how a payment request is seen to be
    /// paid without telling anyone the request key.
    pub fn notes_on_keys(&self, pks: &[[u8; 32]]) -> Vec<RpcNoteEntry> {
        let state = self.state.lock().unwrap();
        state.entries.iter().filter(|(_, e)| pks.contains(&e.note.pk)).map(|(sn, e)| rpc_entry(*sn, e)).collect()
    }

    pub fn status(&self) -> LightStatus {
        let state = self.state.lock().unwrap();
        LightStatus {
            at: state.at,
            at_daa: state.at_daa,
            anchored: state.anchored,
            anchored_daa: state.anchored_daa,
            notes: state.entries.len(),
            checked_ms: state.checked_ms,
        }
    }

    async fn restart(&self, rpc: &Arc<DynRpcApi>) -> Result<()> {
        let (anchor, header) = checked_anchor(rpc, &self.trustees).await?;
        let fresh = fetch_state(rpc, &anchor, &header).await?;
        *self.state.lock().unwrap() = fresh;
        Ok(())
    }

    async fn check_anchor(&self, rpc: &Arc<DynRpcApi>) -> Result<AnchorCheck> {
        let (anchor, header) = checked_anchor(rpc, &self.trustees).await?;
        let mut state = self.state.lock().unwrap();
        if anchor.anchored_daa_score < state.anchored_daa {
            // An older anchor than the one already checked: a node behind
            // the one before, nothing to learn from it.
            return Ok(AnchorCheck::NotYetReached);
        }
        if anchor.anchored_block == state.anchored {
            state.checked_ms = unixtime_as_millis_u64();
            return Ok(AnchorCheck::Agreed);
        }
        let Some((_, root)) = state.recent_roots.iter().find(|(block, _)| *block == anchor.anchored_block) else {
            // The wallet has not reached the anchored block yet.
            return Ok(AnchorCheck::NotYetReached);
        };
        if *root != header.pool_commitment {
            return Ok(AnchorCheck::Disagreed);
        }
        state.anchored = anchor.anchored_block;
        state.anchored_daa = anchor.anchored_daa_score;
        state.checked_ms = unixtime_as_millis_u64();
        Ok(AnchorCheck::Agreed)
    }
}

enum AnchorCheck {
    Agreed,
    NotYetReached,
    Disagreed,
}

/// The latest anchor, verified against the trustees' keys, with the header of
/// the block it names, verified to be that block.
async fn checked_anchor(rpc: &Arc<DynRpcApi>, trustees: &TrusteeKeys) -> Result<(FinalityAnchor, Header)> {
    let got = rpc
        .get_finality_anchor()
        .await
        .map_err(|e| Error::Custom(format!("the public computer cannot serve a light wallet (no finality anchor call): {e}")))?
        .anchor
        .ok_or_else(|| Error::Custom("the public computer has no finality anchor yet".to_string()))?;
    if got.signatures.len() % 64 != 0 {
        return Err(Error::Custom("the anchor's signatures are malformed".to_string()));
    }
    let signatures: Vec<[u8; 64]> = got.signatures.chunks_exact(64).map(|c| c.try_into().expect("64 bytes")).collect();
    let anchor = FinalityAnchor {
        anchored_block: got.anchored_block,
        anchored_daa_score: got.anchored_daa_score,
        signer_bitmap: got.signer_bitmap,
        signatures,
    };
    verify_anchor(&anchor, trustees)
        .map_err(|e| Error::Custom(format!("the anchor does not verify against the trustees' keys: {e}")))?;
    let block = rpc
        .get_block(anchor.anchored_block, false)
        .await
        .map_err(|e| Error::Custom(format!("the anchored block could not be fetched: {e}")))?;
    let header = header_from_rpc(&block.header);
    if header.hash != anchor.anchored_block {
        return Err(Error::Custom("the block the public computer sent is not the block the anchor names".to_string()));
    }
    if header.daa_score != anchor.anchored_daa_score {
        return Err(Error::Custom("the anchored block's DAA score does not match the anchor".to_string()));
    }
    Ok((anchor, header))
}

/// The pool state as of the anchored block, checked against its commitment.
async fn fetch_state(rpc: &Arc<DynRpcApi>, anchor: &FinalityAnchor, header: &Header) -> Result<LightState> {
    let got = rpc
        .get_pool_state_at_block(anchor.anchored_block)
        .await
        .map_err(|e| Error::Custom(format!("the public computer cannot serve a light wallet (no pool state call): {e}")))?;
    let mut entries = HashMap::with_capacity(got.notes.len());
    let mut tree = LightTree::new();
    let mut leaves = Vec::with_capacity(got.notes.len());
    for note in got.notes.iter() {
        let entry = pool_entry(note)?;
        leaves.push((note.sn, leaf_hash_entry(&entry)));
        entries.insert(note.sn, entry);
    }
    tree.update(leaves);
    if tree.root() != header.pool_commitment {
        return Err(Error::Custom(
            "the pool state the public computer sent does not match what the anchored block commits to; not using it".to_string(),
        ));
    }
    let mut recent_roots = VecDeque::new();
    recent_roots.push_back((anchor.anchored_block, header.pool_commitment));
    Ok(LightState {
        entries,
        tree,
        at: anchor.anchored_block,
        at_daa: anchor.anchored_daa_score,
        anchored: anchor.anchored_block,
        anchored_daa: anchor.anchored_daa_score,
        checked_ms: unixtime_as_millis_u64(),
        recent_roots,
    })
}

async fn fetch_diffs(rpc: &Arc<DynRpcApi>, blocks: &[Hash]) -> Result<Vec<RpcPoolDiff>> {
    let mut diffs = Vec::with_capacity(blocks.len());
    for chunk in blocks.chunks(DIFFS_PER_CALL) {
        let got =
            rpc.get_pool_diff(chunk.to_vec()).await.map_err(|e| Error::Custom(format!("the pool diffs could not be fetched: {e}")))?;
        if got.diffs.len() != chunk.len() || got.diffs.iter().zip(chunk).any(|(d, b)| d.block != *b) {
            return Err(Error::Custom("the public computer answered with diffs for other blocks than asked".to_string()));
        }
        diffs.extend(got.diffs);
    }
    Ok(diffs)
}

/// Takes `out` out of the state and puts `into` in, in one tree update.
fn shift(state: &mut LightState, out: &[RpcNoteEntry], into: &[RpcNoteEntry]) {
    let mut leaves = Vec::with_capacity(out.len() + into.len());
    for note in out.iter() {
        state.entries.remove(&note.sn);
        leaves.push((note.sn, ZERO_HASH));
    }
    for note in into.iter() {
        if let Ok(entry) = pool_entry(note) {
            leaves.push((note.sn, leaf_hash_entry(&entry)));
            state.entries.insert(note.sn, entry);
        }
    }
    state.tree.update(leaves);
}

fn apply(state: &mut LightState, diff: &RpcPoolDiff) {
    shift(state, &diff.removed, &diff.added);
}

fn unapply(state: &mut LightState, diff: &RpcPoolDiff) {
    shift(state, &diff.added, &diff.removed);
}

fn pool_entry(note: &RpcNoteEntry) -> Result<PoolEntry> {
    let d = DenominationTag::try_from(note.denomination)
        .map_err(|_| Error::Custom(format!("unknown denomination tag {}", note.denomination)))?;
    let lock = note.lock.as_ref().map(|l| NoteLock { refund_pk: l.refund_pk, until_daa: l.until_daa });
    Ok(PoolEntry { note: NewNote { d, pk: note.pk }, lock })
}

fn rpc_entry(sn: Hash, e: &PoolEntry) -> RpcNoteEntry {
    RpcNoteEntry {
        sn,
        denomination: e.note.d as u8,
        pk: e.note.pk,
        lock: e.lock.map(|l| RpcNoteLock { refund_pk: l.refund_pk, until_daa: l.until_daa }),
    }
}

/// The header as the chain hashes it, rebuilt from the RPC's fields so the
/// hash is the wallet's own computation, not the node's word.
fn header_from_rpc(h: &RpcHeader) -> Header {
    let mut parents = CompressedParents::default();
    for level in h.parents_by_level.iter() {
        parents.push(level.clone());
    }
    Header::new_finalized(
        h.version,
        parents,
        h.hash_merkle_root,
        h.accepted_id_merkle_root,
        h.utxo_commitment,
        h.pool_commitment,
        h.timestamp,
        h.bits,
        h.nonce,
        h.daa_score,
        h.blue_work,
        h.blue_score,
        h.pruning_point,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_consensus_core::notepool::DenominationTag;

    fn entry(sn: u64, pk: u8) -> RpcNoteEntry {
        RpcNoteEntry { sn: Hash::from_u64_word(sn), denomination: DenominationTag::D1 as u8, pk: [pk; 32], lock: None }
    }

    /// Applying a diff and taking it back out leaves the entries and the
    /// commitment exactly as they were: what a reorganisation relies on.
    #[test]
    fn apply_then_unapply_is_the_identity() {
        let mut state = LightState {
            entries: HashMap::new(),
            tree: LightTree::new(),
            at: Hash::from_u64_word(1),
            at_daa: 0,
            anchored: Hash::from_u64_word(1),
            anchored_daa: 0,
            checked_ms: 0,
            recent_roots: VecDeque::new(),
        };
        let before = state.tree.root();
        apply(&mut state, &RpcPoolDiff { block: Hash::from_u64_word(2), added: vec![entry(10, 1), entry(11, 2)], removed: vec![] });
        let with_two = state.tree.root();
        assert_ne!(with_two, before);
        let rotate = RpcPoolDiff { block: Hash::from_u64_word(3), added: vec![entry(12, 3)], removed: vec![entry(10, 1)] };
        apply(&mut state, &rotate);
        assert!(state.entries.contains_key(&Hash::from_u64_word(12)) && !state.entries.contains_key(&Hash::from_u64_word(10)));
        unapply(&mut state, &rotate);
        assert_eq!(state.tree.root(), with_two);
        assert!(state.entries.contains_key(&Hash::from_u64_word(10)) && !state.entries.contains_key(&Hash::from_u64_word(12)));
    }

    /// The commitment the wallet computes is the one the node computes: the
    /// same leaf hashing over the same entries, whatever the order of arrival.
    #[test]
    fn commitment_does_not_depend_on_order() {
        let mut a = LightTree::new();
        let mut b = LightTree::new();
        let notes = [entry(5, 1), entry(6, 2), entry(7, 3)];
        for n in notes.iter() {
            a.update([(n.sn, leaf_hash_entry(&pool_entry(n).unwrap()))]);
        }
        b.update(notes.iter().rev().map(|n| (n.sn, leaf_hash_entry(&pool_entry(n).unwrap()))));
        assert_eq!(a.root(), b.root());
    }
}
