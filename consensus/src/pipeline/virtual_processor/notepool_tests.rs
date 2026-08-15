//! Consensus-level note-pool tests (FORK-PLAN P6.4's verify criteria): parallel-block
//! double-rotate resolving deterministically via the composed-view mergeset walk, reorg
//! apply/unapply restoring pool state exactly, and freshness-anchor rejection in context.
//!
//! Stateless shape rules and the freshness window's exact inclusive boundaries (including
//! genuine staleness at `pov - anchor = WINDOW + 1`, which would need 36k mined blocks to
//! reach here) are unit-tested in `consensus-core`'s `notepool::validate` module; these
//! tests cover the pipeline wiring end-to-end.

use crate::config::ConfigBuilder;
use crate::consensus::test_consensus::TestConsensus;
use kaspa_consensus_core::{
    api::ConsensusApi,
    config::params::MAINNET_PARAMS,
    constants::TX_VERSION_TOCCATA,
    notepool::{
        DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, SignedGroup, TransferOp, hashing as pool_hashing,
    },
    subnets::SUBNETWORK_ID_NOTE_POOL,
    tx::{Transaction, TransactionId},
};
use kaspa_hashes::Hash;

struct Wallet {
    keypair: secp256k1::Keypair,
    pk: [u8; 32],
}

impl Wallet {
    fn new(seed: u8) -> Self {
        let keypair = secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[seed; 32]).unwrap();
        let pk = keypair.public_key().x_only_public_key().0.serialize();
        Self { keypair, pk }
    }

    fn note(&self, d: DenominationTag) -> NewNote {
        NewNote { d, pk: self.pk }
    }

    /// A signed rotate of `serials` (all under this wallet's pk) to `produced`.
    fn rotate(&self, serials: Vec<Hash>, produced: Vec<NewNote>, anchor: u64) -> PoolOp {
        let outputs_hash = pool_hashing::transparent_outputs_hash(&[]);
        let msg_hash = pool_hashing::signing_hash(1, &serials, &produced, outputs_hash, anchor);
        let msg = secp256k1::Message::from_digest(msg_hash.into());
        let signature = *self.keypair.sign_schnorr(msg).as_ref();
        PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials, signature }],
            produced,
            freshness: FreshnessAnchor { anchor_daa_score: anchor },
        })
    }
}

fn pool_tx(op: &PoolOp) -> Transaction {
    Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, op.encode_payload())
}

fn mint_tx(notes: Vec<NewNote>) -> Transaction {
    pool_tx(&PoolOp::Mint(MintOp { new_notes: notes }))
}

/// The serial the pool derives for `tx`'s `index`-th produced note.
fn produced_serial(tx: &Transaction, index: u32) -> Hash {
    pool_hashing::serial_hash(&tx.id(), index)
}

fn config() -> crate::config::Config {
    ConfigBuilder::new(MAINNET_PARAMS)
        .skip_proof_of_work()
        .edit_consensus_params(|p| {
            p.max_block_parents = 4;
            p.mergeset_size_limit = 10;
        })
        .build()
}

/// End-to-end happy path: a zero-input mint enters the pool via a block, a rotate moves
/// the note to a new key, and the virtual pool map + SMT root track both.
#[tokio::test]
async fn mint_and_rotate_update_pool_state_and_root() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let empty_root = consensus.pool_root();

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    // Block 1: mint one 1-MAGLD note to alice.
    let mint = mint_tx(vec![alice.note(DenominationTag::D1)]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(10.into(), vec![genesis], vec![mint]).await.unwrap();

    assert_eq!(consensus.pool_note(sn), Some(alice.note(DenominationTag::D1)), "minted note must be live at virtual");
    let root_after_mint = consensus.pool_root();
    assert_ne!(root_after_mint, empty_root, "pool commitment must move when a note enters the pool");

    // Block 2: alice rotates the note to bob.
    let rotate = pool_tx(&alice.rotate(vec![sn], vec![bob.note(DenominationTag::D1)], 0));
    let rotated_sn = produced_serial(&rotate, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![10.into()], vec![rotate]).await.unwrap();

    assert_eq!(consensus.pool_note(sn), None, "consumed serial must be retired (invariant I2)");
    assert_eq!(consensus.pool_note(rotated_sn), Some(bob.note(DenominationTag::D1)), "produced note must be live");
    let root_after_rotate = consensus.pool_root();
    assert_ne!(root_after_rotate, root_after_mint);
    assert_ne!(root_after_rotate, empty_root);

    consensus.shutdown(join_handles);
}

/// P6.4 verify criterion 1: two parallel blocks rotating the same serial to different
/// destinations resolve deterministically — exactly one rotate is accepted by the merging
/// chain block (blue order decides), and the outcome is identical regardless of block
/// arrival order.
#[tokio::test]
async fn parallel_double_rotate_resolves_deterministically() {
    // Run the identical DAG twice with opposite insertion orders for the conflicting
    // blocks; both runs must converge to the same accepted rotate and pool state.
    //
    // NOTE: the transactions are built ONCE, outside the loop — BIP340 signing uses
    // randomized aux nonces, so re-signing the same message yields a different signature
    // and therefore a different tx id and different derived serials. The comparison is
    // only meaningful over the identical transactions.
    let alice = Wallet::new(1);
    let carol = Wallet::new(3);
    let dave = Wallet::new(4);
    let mint = mint_tx(vec![alice.note(DenominationTag::D1)]);
    let sn = produced_serial(&mint, 0);
    // Two conflicting rotates of the same serial (distinct destinations => distinct txs).
    let rotate_c = pool_tx(&alice.rotate(vec![sn], vec![carol.note(DenominationTag::D1)], 0));
    let rotate_d = pool_tx(&alice.rotate(vec![sn], vec![dave.note(DenominationTag::D1)], 0));
    let sn_c = produced_serial(&rotate_c, 0);
    let sn_d = produced_serial(&rotate_d, 0);

    let mut outcomes = Vec::new();
    for swap_insertion_order in [false, true] {
        let consensus = TestConsensus::new(&config());
        let join_handles = consensus.init();
        let genesis = consensus.params().genesis.hash;

        // Base block: mint a note to alice.
        consensus.add_utxo_valid_block_with_parents(10.into(), vec![genesis], vec![mint.clone()]).await.unwrap();

        // Parallel blocks A (hash 20, rotate->carol) and B (hash 21, rotate->dave), both on the mint block.
        // Insertion order varies; block identity (hash) does not.
        let (first, second) = if swap_insertion_order {
            ((21.into(), rotate_d.clone()), (20.into(), rotate_c.clone()))
        } else {
            ((20.into(), rotate_c.clone()), (21.into(), rotate_d.clone()))
        };
        let (first_hash, first_tx): (Hash, Transaction) = first;
        let (second_hash, second_tx): (Hash, Transaction) = second;
        consensus.add_utxo_valid_block_with_parents(first_hash, vec![10.into()], vec![first_tx]).await.unwrap();
        consensus.add_utxo_valid_block_with_parents(second_hash, vec![10.into()], vec![second_tx]).await.unwrap();

        // Chain block C merges both.
        consensus.add_utxo_valid_block_with_parents(30.into(), vec![20.into(), 21.into()], vec![]).await.unwrap();

        // Exactly one rotate accepted in C's committed acceptance data.
        let acceptance = consensus.get_block_acceptance_data(30.into()).unwrap();
        let accepted_ids: Vec<TransactionId> =
            acceptance.iter().flat_map(|mbad| mbad.accepted_transactions.iter().map(|e| e.transaction_id)).collect();
        let c_accepted = accepted_ids.contains(&rotate_c.id());
        let d_accepted = accepted_ids.contains(&rotate_d.id());
        assert_ne!(c_accepted, d_accepted, "exactly one of the two conflicting rotates must be accepted");

        // The pool state agrees with the acceptance decision: consumed serial gone,
        // winner's destination live, loser's absent.
        assert_eq!(consensus.pool_note(sn), None);
        let winner_sn = if c_accepted { sn_c } else { sn_d };
        let loser_sn = if c_accepted { sn_d } else { sn_c };
        assert!(consensus.pool_note(winner_sn).is_some());
        assert_eq!(consensus.pool_note(loser_sn), None);

        outcomes.push((c_accepted, consensus.pool_root()));
        consensus.shutdown(join_handles);
    }

    assert_eq!(outcomes[0], outcomes[1], "conflict resolution must not depend on block arrival order");
}

/// The same conflict, one level deeper (P6.4 verify criterion 2): a reorg to a heavier
/// branch carrying a conflicting rotate makes the losing branch's op invalid in the new
/// context, and the virtual pool state + root converge to exactly what a node that only
/// ever saw the winning branch computes. The sink handoff across branches exercises the
/// walk-down (diff unapply) path of `calculate_utxo_state_relatively`.
#[tokio::test]
async fn reorg_past_pool_op_restores_prior_pool_state() {
    let alice = Wallet::new(1);
    let carol = Wallet::new(3);
    let dave = Wallet::new(4);

    let mint = mint_tx(vec![alice.note(DenominationTag::D1)]);
    let sn = produced_serial(&mint, 0);
    let rotate_x = pool_tx(&alice.rotate(vec![sn], vec![carol.note(DenominationTag::D1)], 0));
    let rotate_y = pool_tx(&alice.rotate(vec![sn], vec![dave.note(DenominationTag::D1)], 0));
    let sn_x = produced_serial(&rotate_x, 0);
    let sn_y = produced_serial(&rotate_y, 0);

    // Full node: sees the X branch first (rotate->carol becomes the accepted op), then a
    // heavier Y branch (rotate->dave) built on the same mint block.
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    consensus.add_utxo_valid_block_with_parents(10.into(), vec![genesis], vec![mint.clone()]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(20.into(), vec![10.into()], vec![rotate_x.clone()]).await.unwrap();

    // X branch is the accepted context: carol's note is live.
    assert!(consensus.pool_note(sn_x).is_some());
    assert_eq!(consensus.pool_note(sn_y), None);
    let root_x = consensus.pool_root();

    // Heavier Y branch: Y1 carries the conflicting rotate, Y2/Y3 outweigh the X branch.
    consensus.add_utxo_valid_block_with_parents(31.into(), vec![10.into()], vec![rotate_y.clone()]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(32.into(), vec![31.into()], vec![]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(33.into(), vec![32.into()], vec![]).await.unwrap();

    // The sink is now on the Y branch; X's rotate is invalid in the new context (its
    // serial was first-accepted-consumed by Y1) and must be fully unapplied.
    assert_eq!(consensus.pool_note(sn_x), None, "losing branch's produced note must be unapplied after the reorg");
    assert!(consensus.pool_note(sn_y).is_some(), "winning branch's produced note must be live after the reorg");
    assert_eq!(consensus.pool_note(sn), None);
    let root_after_reorg = consensus.pool_root();
    assert_ne!(root_after_reorg, root_x);

    // Reference node: never saw the X branch at all. Its pool root is the ground truth
    // the reorged node must converge to exactly.
    let reference = TestConsensus::new(&config());
    let reference_handles = reference.init();
    let reference_genesis = reference.params().genesis.hash;
    reference.add_utxo_valid_block_with_parents(10.into(), vec![reference_genesis], vec![mint]).await.unwrap();
    reference.add_utxo_valid_block_with_parents(31.into(), vec![10.into()], vec![rotate_y]).await.unwrap();
    reference.add_utxo_valid_block_with_parents(32.into(), vec![31.into()], vec![]).await.unwrap();
    reference.add_utxo_valid_block_with_parents(33.into(), vec![32.into()], vec![]).await.unwrap();

    assert_eq!(
        root_after_reorg,
        reference.pool_root(),
        "reorged node's pool root must equal a never-forked node's root — no residue from the unapplied branch"
    );

    consensus.shutdown(join_handles);
    reference.shutdown(reference_handles);
}

/// P6.4 verify criterion 3 (the consensus-level half): an op whose freshness anchor is
/// outside the valid window is rejected in the UTXO/pool context stage. A future anchor
/// exercises the same `check_freshness` gate as a stale one without needing 36k mined
/// blocks; the stale side's exact inclusive boundaries are pinned by consensus-core unit
/// tests (`freshness_window_boundaries_are_inclusive`).
#[tokio::test]
async fn out_of_window_anchor_op_rejected_in_context() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    let mint = mint_tx(vec![alice.note(DenominationTag::D1)]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(10.into(), vec![genesis], vec![mint]).await.unwrap();

    // Anchor far in the future of any POV this test can reach.
    let bad_rotate = pool_tx(&alice.rotate(vec![sn], vec![bob.note(DenominationTag::D1)], u64::MAX));
    let miner_data = kaspa_consensus_core::coinbase::MinerData::new(kaspa_consensus_core::tx::ScriptPublicKey::from_vec(0, vec![]), vec![]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        consensus.build_utxo_valid_block_with_parents(11.into(), vec![10.into()], miner_data, vec![bad_rotate])
    }));
    assert!(result.is_err(), "building a block containing an out-of-window-anchor op must fail template validation");

    // The pool state is untouched and a correctly-anchored rotate still works.
    assert!(consensus.pool_note(sn).is_some());
    let good_rotate = pool_tx(&alice.rotate(vec![sn], vec![bob.note(DenominationTag::D1)], 0));
    let good_sn = produced_serial(&good_rotate, 0);
    consensus.add_utxo_valid_block_with_parents(12.into(), vec![10.into()], vec![good_rotate]).await.unwrap();
    assert!(consensus.pool_note(good_sn).is_some());

    consensus.shutdown(join_handles);
}

/// The pre-P6.6 soundness case the active produced-serial check closes: the SAME
/// zero-input mint transaction included in two parallel blocks. Both blocks are
/// individually valid; the merging context must accept the mint exactly once (the second
/// instance fails `SerialAlreadyExists` against the composed view) instead of
/// double-adding the serial.
#[tokio::test]
async fn duplicate_mint_across_parallel_blocks_accepted_once() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let mint = mint_tx(vec![alice.note(DenominationTag::D1)]);
    let sn = produced_serial(&mint, 0);

    // The same tx in two parallel blocks (legal in a blockDAG).
    consensus.add_utxo_valid_block_with_parents(20.into(), vec![genesis], vec![mint.clone()]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(21.into(), vec![genesis], vec![mint.clone()]).await.unwrap();
    // Merge both.
    consensus.add_utxo_valid_block_with_parents(30.into(), vec![20.into(), 21.into()], vec![]).await.unwrap();

    // Accepted exactly once across the merging block's acceptance data.
    let acceptance = consensus.get_block_acceptance_data(30.into()).unwrap();
    let accepted_count =
        acceptance.iter().flat_map(|mbad| mbad.accepted_transactions.iter()).filter(|e| e.transaction_id == mint.id()).count();
    assert_eq!(accepted_count, 1, "the duplicated mint must be accepted exactly once");
    assert_eq!(consensus.pool_note(sn), Some(alice.note(DenominationTag::D1)));

    consensus.shutdown(join_handles);
}

/// P6.5 has two independent pool-commitment computation paths that must always agree:
/// the fast incremental tracking store (`DbNotePoolSmtStore`, used for virtual's own
/// `pool_root()`) and the from-scratch rebuild (`recompute_pool_commitment`, used both
/// at template-build time to fill `header.pool_commitment` and again at verification
/// time). Every successful block insertion already implicitly cross-checks these — a
/// disagreement would make the node reject its own mined block — but this test asserts
/// it directly and permanently.
///
/// The comparison is offset by one block, not same-block: a block's own
/// `header.pool_commitment` reflects its *ancestors'* state (the standard GHOSTDAG
/// commitment shape — `calculate_utxo_state`'s mergeset walk always replays the
/// selected parent's own transactions but never the current block's own body; a block's
/// own transactions only surface in ITS descendants' commitments). So block N+1's header
/// commitment must equal `pool_root()` as it stood right after block N — not after N+1.
#[tokio::test]
async fn incremental_and_full_rebuild_commitments_agree() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    let mint = mint_tx(vec![alice.note(DenominationTag::D1), alice.note(DenominationTag::D10)]);
    let sn0 = produced_serial(&mint, 0);
    // Block 10's ancestor (genesis) has an empty pool — matches the canonical empty root.
    consensus.add_utxo_valid_block_with_parents(10.into(), vec![genesis], vec![mint]).await.unwrap();
    let root_after_10 = consensus.pool_root();

    let rotate = pool_tx(&alice.rotate(vec![sn0], vec![bob.note(DenominationTag::D1)], 0));
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![10.into()], vec![rotate]).await.unwrap();

    // Block 11's own commitment reflects its ancestor (block 10)'s state, i.e. exactly
    // what pool_root() was right after block 10 — not block 11's own rotate.
    assert_eq!(consensus.header_pool_commitment(11.into()), root_after_10);
    // ...while virtual's own live root (which replays block 11's own tx too) has moved on.
    assert_ne!(consensus.pool_root(), root_after_10);

    consensus.shutdown(join_handles);
}
