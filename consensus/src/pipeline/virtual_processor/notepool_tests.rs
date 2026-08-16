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
use kaspa_addresses::{Address, Prefix, Version};
use kaspa_consensus_core::{
    api::ConsensusApi,
    coinbase::MinerData,
    config::params::MAINNET_PARAMS,
    constants::TX_VERSION_TOCCATA,
    hashing::{sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash}, sighash_type::SIG_HASH_ALL},
    mass::MassCalculator,
    notepool::{
        DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, RedeemOp, SignedGroup, TransferOp, hashing as pool_hashing,
    },
    subnets::SUBNETWORK_ID_NOTE_POOL,
    tx::{
        ComputeCommit, SignableTransaction, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput,
        UtxoEntry,
    },
};
use kaspa_hashes::Hash;
use kaspa_txscript::standard::pay_to_address_script;

/// Deterministic single-key sign, mirroring `kaspa_consensus_core::sign::sign` exactly
/// except for using `sign_schnorr_no_aux_rand` instead of the (BIP340 aux-randomized)
/// `Keypair::sign_schnorr`. `parallel_double_rotate_resolves_deterministically` compares
/// outcomes across two independently-run `TestConsensus` instances that each mine their
/// own funding chain and sign their own mint from scratch — that comparison is only
/// meaningful if signing the identical logical transaction twice yields byte-identical
/// bytes both times, which the real (randomized) `sign()` cannot guarantee.
fn sign_deterministic(mut signable_tx: SignableTransaction, schnorr_key: &secp256k1::Keypair) -> SignableTransaction {
    let input_mass = if ComputeCommit::version_expects_compute_budget_field(signable_tx.tx.version) {
        kaspa_consensus_core::mass::ComputeBudget(10).into()
    } else {
        kaspa_consensus_core::mass::SigopCount(1).into()
    };
    for i in 0..signable_tx.tx.inputs.len() {
        signable_tx.tx.inputs[i].compute_commit = input_mass;
    }
    let reused_values = SigHashReusedValuesUnsync::new();
    for i in 0..signable_tx.tx.inputs.len() {
        let sig_hash = calc_schnorr_signature_hash(&signable_tx.as_verifiable(), i, SIG_HASH_ALL, &reused_values);
        let msg = secp256k1::Message::from_digest_slice(sig_hash.as_bytes().as_slice()).unwrap();
        let sig: [u8; 64] = *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, schnorr_key).as_ref();
        signable_tx.tx.inputs[i].signature_script = std::iter::once(65u8).chain(sig).chain([SIG_HASH_ALL.to_u8()]).collect();
    }
    signable_tx
}

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

    /// A standard P2PK script paying this wallet — used as `MinerData` (to receive a
    /// coinbase reward) and as a transparent change-output script.
    fn script(&self) -> kaspa_consensus_core::tx::ScriptPublicKey {
        pay_to_address_script(&Address::new(Prefix::Mainnet, Version::PubKey, &self.pk))
    }

    /// A signed rotate of `serials` (all under this wallet's pk) to `produced`.
    fn rotate(&self, serials: Vec<Hash>, produced: Vec<NewNote>, anchor: u64) -> PoolOp {
        let outputs_hash = pool_hashing::transparent_outputs_hash(&[]);
        let msg_hash = pool_hashing::signing_hash(1, &serials, &produced, outputs_hash, anchor);
        let msg = secp256k1::Message::from_digest(msg_hash.into());
        // Deterministic (no-aux-rand): see `sign_deterministic`'s doc comment — some of
        // this file's tests rebuild "the same" rotate across independent runs/instances
        // and compare outcomes for equality, which needs byte-identical signatures.
        let signature = *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, &self.keypair).as_ref();
        PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials, signature }],
            produced,
            freshness: FreshnessAnchor { anchor_daa_score: anchor },
        })
    }

    /// A signed redeem of `serials` (all under this wallet's pk) into `outputs`.
    fn redeem(&self, serials: Vec<Hash>, outputs: &[TransactionOutput], anchor: u64) -> PoolOp {
        let outputs_hash = pool_hashing::transparent_outputs_hash(outputs);
        let msg_hash = pool_hashing::signing_hash(2, &serials, &[], outputs_hash, anchor);
        let msg = secp256k1::Message::from_digest(msg_hash.into());
        let signature = *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, &self.keypair).as_ref();
        PoolOp::Redeem(RedeemOp { consumed: vec![SignedGroup { serials, signature }], freshness: FreshnessAnchor { anchor_daa_score: anchor } })
    }
}

fn pool_tx(op: &PoolOp) -> Transaction {
    Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, op.encode_payload())
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
            // These tests fund real mint transactions from real mined coinbase rewards
            // (FORK-PLAN P6.6 requires mint's transparent inputs to actually cover the
            // notes it creates); zeroing maturity avoids mining ~1000 throwaway blocks
            // per test just to wait it out, matching this codebase's own established
            // pattern (e.g. `testing/integration`'s `toccata_activation_test`).
            p.coinbase_maturity = 0;
        })
        .build()
}

/// Mines two blocks funding `wallet` with a real, spendable coinbase reward: block A
/// (on `parent`) pays its reward to `wallet`'s script; block B (on A) is what actually
/// carries that reward in ITS OWN coinbase transaction — Kaspa/Marigold's mergeset
/// reward mechanism pays a merged block's subsidy in its child's coinbase, never its
/// own (`consensus/src/processes/coinbase.rs`'s `expected_coinbase_transaction` loops
/// `ghostdag_data.mergeset_blues`, not the current block). Returns the funding
/// `(outpoint, entry)` and B's hash (the new tip for whatever the caller builds next).
async fn fund(consensus: &TestConsensus, wallet: &Wallet, parent: Hash, hash_a: Hash, hash_b: Hash) -> (TransactionOutpoint, UtxoEntry, Hash) {
    let block_a = consensus.build_utxo_valid_block_with_parents(hash_a, vec![parent], MinerData::new(wallet.script(), vec![]), vec![]);
    let daa_score_a = block_a.header.daa_score;
    consensus.validate_and_insert_block(block_a.to_immutable()).virtual_state_task.await.unwrap();

    let block_b =
        consensus.build_utxo_valid_block_with_parents(hash_b, vec![hash_a], MinerData::new(Default::default(), vec![]), vec![]);
    consensus.validate_and_insert_block(block_b.to_immutable()).virtual_state_task.await.unwrap();

    let coinbase = &consensus.get_block(hash_b).unwrap().transactions[0];
    let outpoint = TransactionOutpoint::new(coinbase.id(), 0);
    let entry = UtxoEntry::new(coinbase.outputs[0].value, coinbase.outputs[0].script_public_key.clone(), daa_score_a, true, None);
    (outpoint, entry, hash_b)
}

/// Computes and commits this transaction's storage-mass field over `entries` (one per
/// `tx.inputs`, in order — empty for a pool-only tx with no transparent inputs). The block
/// builder requires this field to already be correct (`check_mass_commitment` in
/// `tx_validation_in_utxo_context.rs` rejects a mismatch) — in production that's the
/// mempool's job (`validate_mempool_transaction_in_utxo_context`), which nothing in this
/// hand-built test path goes through, so it's done explicitly here instead.
fn commit_storage_mass(tx: Transaction, entries: Vec<UtxoEntry>) -> Transaction {
    let populated = kaspa_consensus_core::tx::PopulatedTransaction::new(&tx, entries);
    let storage_mass = MassCalculator::new_with_consensus_params(&MAINNET_PARAMS).calc_contextual_masses(&populated).unwrap().storage_mass;
    tx.set_storage_mass(storage_mass);
    tx
}

/// Builds and signs a real mint transaction spending `funding`, with `outputs` as its
/// transparent outputs (whatever the caller wants — e.g. no outputs at all, to test
/// rejection of a mint that doesn't cover the notes it creates).
fn mint_funded_with_outputs(
    wallet: &Wallet,
    funding: (TransactionOutpoint, UtxoEntry),
    notes: Vec<NewNote>,
    outputs: Vec<TransactionOutput>,
) -> Transaction {
    let (outpoint, entry) = funding;
    let tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(outpoint, vec![], 0, 1)],
        outputs,
        0,
        SUBNETWORK_ID_NOTE_POOL,
        0,
        PoolOp::Mint(MintOp { new_notes: notes }).encode_payload(),
    );
    let signed = sign_deterministic(kaspa_consensus_core::tx::SignableTransaction::with_entries(tx, vec![entry.clone()]), &wallet.keypair);
    commit_storage_mass(signed.tx, vec![entry])
}

/// Builds and signs a real mint transaction spending `funding`, creating exactly
/// `notes` with any leftover value returned to `wallet` as ordinary transparent
/// change (zero fee — these tests aren't exercising fee amounts).
fn mint_funded(wallet: &Wallet, funding: (TransactionOutpoint, UtxoEntry), notes: Vec<NewNote>) -> Transaction {
    let notes_value: u64 = notes.iter().map(|n| n.d.petals()).sum();
    let change = funding.1.amount - notes_value; // panics on underflow: the test picked notes too large for the funding
    let outputs = if change > 0 { vec![TransactionOutput::new(change, wallet.script())] } else { vec![] };
    mint_funded_with_outputs(wallet, funding, notes, outputs)
}

/// A real redeem transaction: `op` (built via `Wallet::redeem`, over the same `outputs`)
/// paired with the transparent `outputs` it actually carries.
fn redeem_tx(op: &PoolOp, outputs: Vec<TransactionOutput>) -> Transaction {
    let tx = Transaction::new(TX_VERSION_TOCCATA, vec![], outputs, 0, SUBNETWORK_ID_NOTE_POOL, 0, op.encode_payload());
    commit_storage_mass(tx, vec![])
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

    // Blocks A/B: mine and mature a real coinbase reward for alice (P6.6: mint must
    // spend real transparent inputs summing to at least the notes it creates).
    let funding = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let tip = funding.2;

    // Block 3: mint one 0.01-MAGLD note to alice, funded from her coinbase reward (a
    // single block's coinbase can't cover a full 1-MAGLD note under Marigold's real
    // subsidy schedule — see `fund`'s doc comment).
    let mint = mint_funded(&alice, (funding.0, funding.1), vec![alice.note(DenominationTag::D0_01)]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();

    assert_eq!(consensus.pool_note(sn), Some(alice.note(DenominationTag::D0_01)), "minted note must be live at virtual");
    let root_after_mint = consensus.pool_root();
    assert_ne!(root_after_mint, empty_root, "pool commitment must move when a note enters the pool");

    // Block 4: alice rotates the note to bob.
    let rotate = pool_tx(&alice.rotate(vec![sn], vec![bob.note(DenominationTag::D0_01)], 0));
    let rotated_sn = produced_serial(&rotate, 0);
    consensus.add_utxo_valid_block_with_parents(12.into(), vec![11.into()], vec![rotate]).await.unwrap();

    assert_eq!(consensus.pool_note(sn), None, "consumed serial must be retired (invariant I2)");
    assert_eq!(consensus.pool_note(rotated_sn), Some(bob.note(DenominationTag::D0_01)), "produced note must be live");
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
    // NOTE: the mint/rotate transactions are (re)built fresh inside each loop iteration,
    // against each iteration's own freshly-mined coinbase funding (P6.6: mint must spend
    // real transparent inputs, and each fresh `TestConsensus` instance has its own
    // independent UTXO set). This stays comparable across iterations only because signing
    // in this file is deterministic (no BIP340 aux-rand — see `sign_deterministic`) and
    // block-building is otherwise a pure function of the DAG structure (same hash-labeled
    // parents in, same coinbase/tx bytes out): rebuilding "the same" logical transaction
    // twice yields byte-identical transactions both times, so the two loop iterations are
    // still comparing the identical DAG, just with insertion order swapped.
    let alice = Wallet::new(1);
    let carol = Wallet::new(3);
    let dave = Wallet::new(4);

    let mut outcomes = Vec::new();
    for swap_insertion_order in [false, true] {
        let consensus = TestConsensus::new(&config());
        let join_handles = consensus.init();
        let genesis = consensus.params().genesis.hash;

        // Blocks A/B: mine and mature a real coinbase reward for alice, then mint from it.
        let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
        let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_01)]);
        let sn = produced_serial(&mint, 0);
        // Two conflicting rotates of the same serial (distinct destinations => distinct txs).
        let rotate_c = pool_tx(&alice.rotate(vec![sn], vec![carol.note(DenominationTag::D0_01)], 0));
        let rotate_d = pool_tx(&alice.rotate(vec![sn], vec![dave.note(DenominationTag::D0_01)], 0));
        let sn_c = produced_serial(&rotate_c, 0);
        let sn_d = produced_serial(&rotate_d, 0);

        // Base block: mint a note to alice.
        consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();

        // Parallel blocks A (hash 20, rotate->carol) and B (hash 21, rotate->dave), both on the mint block.
        // Insertion order varies; block identity (hash) does not.
        let (first, second) = if swap_insertion_order {
            ((21.into(), rotate_d.clone()), (20.into(), rotate_c.clone()))
        } else {
            ((20.into(), rotate_c.clone()), (21.into(), rotate_d.clone()))
        };
        let (first_hash, first_tx): (Hash, Transaction) = first;
        let (second_hash, second_tx): (Hash, Transaction) = second;
        consensus.add_utxo_valid_block_with_parents(first_hash, vec![11.into()], vec![first_tx]).await.unwrap();
        consensus.add_utxo_valid_block_with_parents(second_hash, vec![11.into()], vec![second_tx]).await.unwrap();

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

    // Full node: sees the X branch first (rotate->carol becomes the accepted op), then a
    // heavier Y branch (rotate->dave) built on the same mint block.
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    // Blocks A/B: mine and mature a real coinbase reward for alice, then mint from it.
    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_01)]);
    let sn = produced_serial(&mint, 0);
    let rotate_x = pool_tx(&alice.rotate(vec![sn], vec![carol.note(DenominationTag::D0_01)], 0));
    let rotate_y = pool_tx(&alice.rotate(vec![sn], vec![dave.note(DenominationTag::D0_01)], 0));
    let sn_x = produced_serial(&rotate_x, 0);
    let sn_y = produced_serial(&rotate_y, 0);

    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint.clone()]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(20.into(), vec![11.into()], vec![rotate_x.clone()]).await.unwrap();

    // X branch is the accepted context: carol's note is live.
    assert!(consensus.pool_note(sn_x).is_some());
    assert_eq!(consensus.pool_note(sn_y), None);
    let root_x = consensus.pool_root();

    // Heavier Y branch: Y1 carries the conflicting rotate, Y2/Y3 outweigh the X branch.
    consensus.add_utxo_valid_block_with_parents(31.into(), vec![11.into()], vec![rotate_y.clone()]).await.unwrap();
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
    // the reorged node must converge to exactly. It must independently mine the identical
    // funding blocks (same hashes) so its own UTXO set also contains the coinbase outpoint
    // `mint` spends; `mint` itself is reused verbatim (not rebuilt) so there's no question
    // of whether re-deriving it would be byte-identical.
    let reference = TestConsensus::new(&config());
    let reference_handles = reference.init();
    let reference_genesis = reference.params().genesis.hash;
    let (_, _, reference_tip) = fund(&reference, &alice, reference_genesis, 5.into(), 10.into()).await;
    reference.add_utxo_valid_block_with_parents(11.into(), vec![reference_tip], vec![mint]).await.unwrap();
    reference.add_utxo_valid_block_with_parents(31.into(), vec![11.into()], vec![rotate_y]).await.unwrap();
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

    // Blocks A/B: mine and mature a real coinbase reward for alice, then mint from it.
    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_01)]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();

    // Anchor far in the future of any POV this test can reach.
    let bad_rotate = pool_tx(&alice.rotate(vec![sn], vec![bob.note(DenominationTag::D0_01)], u64::MAX));
    let miner_data = kaspa_consensus_core::coinbase::MinerData::new(kaspa_consensus_core::tx::ScriptPublicKey::from_vec(0, vec![]), vec![]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        consensus.build_utxo_valid_block_with_parents(12.into(), vec![11.into()], miner_data, vec![bad_rotate])
    }));
    assert!(result.is_err(), "building a block containing an out-of-window-anchor op must fail template validation");

    // The pool state is untouched and a correctly-anchored rotate still works.
    assert!(consensus.pool_note(sn).is_some());
    let good_rotate = pool_tx(&alice.rotate(vec![sn], vec![bob.note(DenominationTag::D0_01)], 0));
    let good_sn = produced_serial(&good_rotate, 0);
    consensus.add_utxo_valid_block_with_parents(13.into(), vec![11.into()], vec![good_rotate]).await.unwrap();
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
    // Blocks A/B: mine and mature a real coinbase reward for alice, then mint from it.
    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_01)]);
    let sn = produced_serial(&mint, 0);

    // The same tx in two parallel blocks (legal in a blockDAG).
    consensus.add_utxo_valid_block_with_parents(20.into(), vec![tip], vec![mint.clone()]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(21.into(), vec![tip], vec![mint.clone()]).await.unwrap();
    // Merge both.
    consensus.add_utxo_valid_block_with_parents(30.into(), vec![20.into(), 21.into()], vec![]).await.unwrap();

    // Accepted exactly once across the merging block's acceptance data.
    let acceptance = consensus.get_block_acceptance_data(30.into()).unwrap();
    let accepted_count =
        acceptance.iter().flat_map(|mbad| mbad.accepted_transactions.iter()).filter(|e| e.transaction_id == mint.id()).count();
    assert_eq!(accepted_count, 1, "the duplicated mint must be accepted exactly once");
    assert_eq!(consensus.pool_note(sn), Some(alice.note(DenominationTag::D0_01)));

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

    // Blocks A/B: mine and mature a real coinbase reward for alice, then mint from it.
    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_01), alice.note(DenominationTag::D0_1)]);
    let sn0 = produced_serial(&mint, 0);
    // Block 11's ancestor chain (5, 10) has an empty pool — matches the canonical empty root.
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();
    let root_after_11 = consensus.pool_root();

    let rotate = pool_tx(&alice.rotate(vec![sn0], vec![bob.note(DenominationTag::D0_01)], 0));
    consensus.add_utxo_valid_block_with_parents(12.into(), vec![11.into()], vec![rotate]).await.unwrap();

    // Block 12's own commitment reflects its ancestor (block 11)'s state, i.e. exactly
    // what pool_root() was right after block 11 — not block 12's own rotate.
    assert_eq!(consensus.header_pool_commitment(12.into()), root_after_11);
    // ...while virtual's own live root (which replays block 12's own tx too) has moved on.
    assert_ne!(consensus.pool_root(), root_after_11);

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.6 verify criterion: a mint whose transparent inputs don't sum to at least
/// its new notes' total value is rejected — the value-binding conservation check added by
/// P6.6, not the stateless shape rules (those are unit-tested in consensus-core).
#[tokio::test]
async fn mint_with_insufficient_transparent_inputs_rejected() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    assert!(entry.amount < DenominationTag::D1.petals(), "test assumes a single block's coinbase can't cover a 1-MAGLD note");

    // Mint a note worth more than the funding, with no transparent output to expose the
    // shortfall (an honest change output would itself require the same excess value).
    let bad_mint = mint_funded_with_outputs(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D1)], vec![]);
    let miner_data = MinerData::new(kaspa_consensus_core::tx::ScriptPublicKey::from_vec(0, vec![]), vec![]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        consensus.build_utxo_valid_block_with_parents(11.into(), vec![tip], miner_data, vec![bad_mint])
    }));
    assert!(result.is_err(), "a mint whose transparent inputs don't cover its new notes must be rejected");

    // A correctly-funded mint at the same point still works.
    let (outpoint2, entry2, tip2) = fund(&consensus, &alice, tip, 20.into(), 21.into()).await;
    let good_mint = mint_funded(&alice, (outpoint2, entry2), vec![alice.note(DenominationTag::D0_01)]);
    let sn = produced_serial(&good_mint, 0);
    consensus.add_utxo_valid_block_with_parents(22.into(), vec![tip2], vec![good_mint]).await.unwrap();
    assert_eq!(consensus.pool_note(sn), Some(alice.note(DenominationTag::D0_01)));

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.6 verify criterion: a redeem claiming more transparent value than its
/// consumed notes are worth is rejected.
#[tokio::test]
async fn redeem_with_excessive_transparent_outputs_rejected() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    // A 0.1-MAGLD note, not 0.01: a redeem's transparent output has no offsetting
    // transparent input for the storage-mass (KIP-0009) formula to net against (unlike
    // mint's real funding input), so an output much smaller than `STORAGE_MASS_PARAMETER`
    // would trip the anti-dust storage-mass limit on its own, unrelated to what this test
    // is actually checking.
    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_1)]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();
    assert!(consensus.pool_note(sn).is_some());

    // Redeem claims one more petal than the consumed note is actually worth.
    let over_claim = TransactionOutput::new(DenominationTag::D0_1.petals() + 1, bob.script());
    let bad_redeem_op = alice.redeem(vec![sn], std::slice::from_ref(&over_claim), 0);
    let bad_redeem = redeem_tx(&bad_redeem_op, vec![over_claim]);
    let miner_data = MinerData::new(kaspa_consensus_core::tx::ScriptPublicKey::from_vec(0, vec![]), vec![]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        consensus.build_utxo_valid_block_with_parents(12.into(), vec![11.into()], miner_data, vec![bad_redeem])
    }));
    assert!(result.is_err(), "a redeem claiming more transparent value than its consumed notes must be rejected");

    // The pool state is untouched and a correctly-valued redeem still works.
    assert!(consensus.pool_note(sn).is_some());
    let good_output = TransactionOutput::new(DenominationTag::D0_1.petals(), bob.script());
    let good_redeem_op = alice.redeem(vec![sn], std::slice::from_ref(&good_output), 0);
    let good_redeem = redeem_tx(&good_redeem_op, vec![good_output]);
    consensus.add_utxo_valid_block_with_parents(13.into(), vec![11.into()], vec![good_redeem]).await.unwrap();
    assert_eq!(consensus.pool_note(sn), None, "redeemed note must be retired");

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.10: split's own happy path mined end-to-end (previously only unit-tested
/// at the `validate_stateful` level, `transfer_split_under_conservation_passes`) — one
/// consumed note fans out into several smaller produced notes, same shape and ratio as
/// that unit test (1 MAGLD -> 9 x 0.1, scaled down one denomination tier so it fits a
/// single funding block's coinbase, matching this file's other single-`fund()` tests).
#[tokio::test]
async fn split_happy_path_mines_and_updates_pool_state() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_1)]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();
    assert!(consensus.pool_note(sn).is_some());
    let root_before_split = consensus.pool_root();

    // 0.1 MAGLD -> 9 x 0.01 MAGLD (0.01 to fee): a real split, mined through a block.
    let produced: Vec<NewNote> = (0..9).map(|_| bob.note(DenominationTag::D0_01)).collect();
    let split = pool_tx(&alice.rotate(vec![sn], produced.clone(), 0));
    let produced_sns: Vec<Hash> = (0..9).map(|i| produced_serial(&split, i)).collect();
    consensus.add_utxo_valid_block_with_parents(12.into(), vec![11.into()], vec![split]).await.unwrap();

    assert_eq!(consensus.pool_note(sn), None, "the split consumed note must be retired");
    for sn in &produced_sns {
        assert_eq!(consensus.pool_note(*sn), Some(bob.note(DenominationTag::D0_01)), "every produced note must be live");
    }
    let root_after_split = consensus.pool_root();
    assert_ne!(root_after_split, root_before_split);

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.10: merge's own happy path mined end-to-end (previously only unit-tested
/// at the `validate_stateful` level, `merchant_sweep_one_signature_many_serials` — which
/// re-keys the same note *count*, not a value merge). Two consumed notes under one shared
/// key, one signature, fold into a single smaller produced note.
#[tokio::test]
async fn merge_happy_path_mines_and_updates_pool_state() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    // One mint tx produces two 0.01-MAGLD notes to alice, both under her one key.
    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_01), alice.note(DenominationTag::D0_01)]);
    let sn0 = produced_serial(&mint, 0);
    let sn1 = produced_serial(&mint, 1);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();
    assert!(consensus.pool_note(sn0).is_some());
    assert!(consensus.pool_note(sn1).is_some());
    let root_before_merge = consensus.pool_root();

    // Merge: both consumed by one signature, one smaller note produced (0.01 to fee).
    let merge = pool_tx(&alice.rotate(vec![sn0, sn1], vec![bob.note(DenominationTag::D0_01)], 0));
    let merged_sn = produced_serial(&merge, 0);
    consensus.add_utxo_valid_block_with_parents(12.into(), vec![11.into()], vec![merge]).await.unwrap();

    assert_eq!(consensus.pool_note(sn0), None, "both merged serials must be retired");
    assert_eq!(consensus.pool_note(sn1), None);
    assert_eq!(consensus.pool_note(merged_sn), Some(bob.note(DenominationTag::D0_01)), "the merged note must be live");
    let root_after_merge = consensus.pool_root();
    assert_ne!(root_after_merge, root_before_merge);

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.10: `PoolOpContextError::BadPublicKey` had no test anywhere in the
/// codebase before this. A note minted with a `pk` that isn't a valid secp256k1 x-only
/// public key (P5.1's mint validation never checks curve membership, only denomination
/// validity — see `validate_mint`) can't be consumed: any attempt to rotate/redeem it
/// fails parsing the *stored* note's pk before signature verification is even reached,
/// regardless of who signs or what they sign.
#[tokio::test]
async fn bad_public_key_on_stored_note_rejects_consumption() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    // x = 0 is not a valid secp256k1 x-only public key (not on the curve).
    let bogus_note = NewNote { d: DenominationTag::D0_01, pk: [0u8; 32] };
    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![bogus_note]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();
    assert_eq!(consensus.pool_note(sn), Some(bogus_note), "an unparseable pk is not rejected at mint time");

    // Any attempted consumption — signer identity is irrelevant, parsing the stored pk
    // fails first.
    let bad_rotate = pool_tx(&alice.rotate(vec![sn], vec![bob.note(DenominationTag::D0_01)], 0));
    let miner_data = MinerData::new(kaspa_consensus_core::tx::ScriptPublicKey::from_vec(0, vec![]), vec![]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        consensus.build_utxo_valid_block_with_parents(12.into(), vec![11.into()], miner_data, vec![bad_rotate])
    }));
    assert!(result.is_err(), "a note with an unparseable stored pk must reject any attempt to consume it");
    assert!(consensus.pool_note(sn).is_some(), "the unconsumable note is untouched, not silently dropped");

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.10: `TxRuleError::MalformedNotePoolPayload` (an on-`SUBNETWORK_ID_NOTE_POOL`
/// transaction whose payload doesn't borsh-decode as any `PoolOp` variant) was previously
/// tested only at `PoolOp::decode_payload` itself (consensus-core's own unit tests); this
/// drives the identical invalid payload through a real block build, pinning the
/// `tx_validation_in_isolation` gate that actually protects the chain.
#[tokio::test]
async fn malformed_pool_payload_rejected_in_block() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    // Discriminant 3 doesn't exist (only Mint=0, Transfer=1, Redeem=2) — same invalid
    // payload consensus-core's own `malformed_pool_op_discriminant_rejected` uses.
    let malformed =
        Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, vec![3u8]);
    let miner_data = MinerData::new(kaspa_consensus_core::tx::ScriptPublicKey::from_vec(0, vec![]), vec![]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        consensus.build_utxo_valid_block_with_parents(11.into(), vec![genesis], miner_data, vec![malformed])
    }));
    assert!(result.is_err(), "a note-pool transaction with an undecodable payload must be rejected");

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.10: a conflict across two DIFFERENT op *types* on the same serial, not
/// just two rotates (`parallel_double_rotate_resolves_deterministically`) — a rotate and a
/// redeem race to consume the same note in parallel blocks. Same first-accepted-wins
/// mechanism, exercised on a shape the existing conflict test never covers (Transfer vs.
/// Redeem both implementing `ImmutablePoolDiff` the same way).
#[tokio::test]
async fn parallel_rotate_vs_redeem_conflict_resolves_deterministically() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_1)]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();

    let rotate = pool_tx(&alice.rotate(vec![sn], vec![bob.note(DenominationTag::D0_1)], 0));
    let rotated_sn = produced_serial(&rotate, 0);
    let redeem_output = TransactionOutput::new(DenominationTag::D0_1.petals(), bob.script());
    let redeem_op = alice.redeem(vec![sn], std::slice::from_ref(&redeem_output), 0);
    let redeem = redeem_tx(&redeem_op, vec![redeem_output]);

    consensus.add_utxo_valid_block_with_parents(20.into(), vec![11.into()], vec![rotate.clone()]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(21.into(), vec![11.into()], vec![redeem.clone()]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(30.into(), vec![20.into(), 21.into()], vec![]).await.unwrap();

    let acceptance = consensus.get_block_acceptance_data(30.into()).unwrap();
    let accepted_ids: Vec<TransactionId> =
        acceptance.iter().flat_map(|mbad| mbad.accepted_transactions.iter().map(|e| e.transaction_id)).collect();
    let rotate_accepted = accepted_ids.contains(&rotate.id());
    let redeem_accepted = accepted_ids.contains(&redeem.id());
    assert_ne!(rotate_accepted, redeem_accepted, "exactly one of the conflicting rotate/redeem must be accepted");

    assert_eq!(consensus.pool_note(sn), None);
    if rotate_accepted {
        assert!(consensus.pool_note(rotated_sn).is_some());
    } else {
        assert_eq!(consensus.pool_note(rotated_sn), None, "the losing rotate's produced note must not be live");
    }

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.10's "deep reorg" verify criterion: the same walk-down/walk-up correctness
/// `reorg_past_pool_op_restores_prior_pool_state` already proves at a 3-block scale must
/// hold over a materially longer replacement chain too — not just as a matter of degree,
/// since a bounded-depth optimization bug in the mergeset/diff walk could pass a shallow
/// reorg and still fail here. The Y branch is stretched to `DEEP_REORG_BLOCKS` blocks
/// (comfortably inside `finality_depth`, so this is purely a depth-of-walk stress, not a
/// finality-boundary test).
#[tokio::test]
async fn deep_reorg_past_pool_op_restores_prior_pool_state() {
    const DEEP_REORG_BLOCKS: u64 = 60;

    let alice = Wallet::new(1);
    let carol = Wallet::new(3);
    let dave = Wallet::new(4);

    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_01)]);
    let sn = produced_serial(&mint, 0);
    let rotate_x = pool_tx(&alice.rotate(vec![sn], vec![carol.note(DenominationTag::D0_01)], 0));
    let rotate_y = pool_tx(&alice.rotate(vec![sn], vec![dave.note(DenominationTag::D0_01)], 0));
    let sn_x = produced_serial(&rotate_x, 0);
    let sn_y = produced_serial(&rotate_y, 0);

    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint.clone()]).await.unwrap();
    consensus.add_utxo_valid_block_with_parents(20.into(), vec![11.into()], vec![rotate_x.clone()]).await.unwrap();
    assert!(consensus.pool_note(sn_x).is_some());
    let root_x = consensus.pool_root();

    // A Y branch DEEP_REORG_BLOCKS long, carrying the conflicting rotate at its base —
    // deep enough that the reorg's walk-up has to replay dozens of blocks, not a handful.
    let y_hashes: Vec<Hash> = (0..DEEP_REORG_BLOCKS).map(|i| (1000 + i).into()).collect();
    consensus.add_utxo_valid_block_with_parents(y_hashes[0], vec![11.into()], vec![rotate_y.clone()]).await.unwrap();
    for w in y_hashes.windows(2) {
        consensus.add_utxo_valid_block_with_parents(w[1], vec![w[0]], vec![]).await.unwrap();
    }

    assert_eq!(consensus.pool_note(sn_x), None, "losing branch's produced note must be unapplied after a deep reorg");
    assert!(consensus.pool_note(sn_y).is_some(), "winning branch's produced note must be live after a deep reorg");
    let root_after_reorg = consensus.pool_root();
    assert_ne!(root_after_reorg, root_x);

    // Reference node: mines only the Y branch from scratch, never sees X at all.
    let reference = TestConsensus::new(&config());
    let reference_handles = reference.init();
    let reference_genesis = reference.params().genesis.hash;
    let (_, _, reference_tip) = fund(&reference, &alice, reference_genesis, 5.into(), 10.into()).await;
    reference.add_utxo_valid_block_with_parents(11.into(), vec![reference_tip], vec![mint]).await.unwrap();
    reference.add_utxo_valid_block_with_parents(y_hashes[0], vec![11.into()], vec![rotate_y]).await.unwrap();
    for w in y_hashes.windows(2) {
        reference.add_utxo_valid_block_with_parents(w[1], vec![w[0]], vec![]).await.unwrap();
    }

    assert_eq!(
        root_after_reorg,
        reference.pool_root(),
        "a deeply reorged node's pool root must equal a never-forked node's root — no residue"
    );

    consensus.shutdown(join_handles);
    reference.shutdown(reference_handles);
}

/// FORK-PLAN P6.10's value-conservation verify criterion extended to split and merge (
/// `value_conservation_across_mint_transfer_redeem` already covers plain mint/transfer/
/// redeem): `Σ pool notes + transparent supply` stays constant — modulo each op's own
/// fee, which strictly decreases it — across a mint, a split, and a merge.
#[tokio::test]
async fn value_conservation_across_split_and_merge() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    let pool_value = |sns: &[Hash]| -> u64 { sns.iter().filter_map(|sn| consensus.pool_note(*sn)).map(|n| n.d.petals()).sum() };

    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_1)]);
    let sn = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();
    assert_eq!(pool_value(&[sn]), DenominationTag::D0_1.petals());

    // Split: 0.1 -> 9 x 0.01 (0.01 fee) — pool value must strictly decrease by the fee.
    let split_produced: Vec<NewNote> = (0..9).map(|_| bob.note(DenominationTag::D0_01)).collect();
    let split = pool_tx(&alice.rotate(vec![sn], split_produced, 0));
    let split_sns: Vec<Hash> = (0..9).map(|i| produced_serial(&split, i)).collect();
    consensus.add_utxo_valid_block_with_parents(12.into(), vec![11.into()], vec![split]).await.unwrap();
    assert_eq!(
        pool_value(&split_sns),
        DenominationTag::D0_1.petals() - DenominationTag::D0_01.petals(),
        "split's produced value must equal consumed minus its fee, exactly"
    );

    // Merge: fold 9 x 0.01 back down to 1 x 0.01 (0.08 fee) — value can only shrink
    // further. Signed by bob, not alice: the split's produced notes belong to bob.
    let merge = pool_tx(&bob.rotate(split_sns.clone(), vec![bob.note(DenominationTag::D0_01)], 0));
    let merged_sn = produced_serial(&merge, 0);
    consensus.add_utxo_valid_block_with_parents(13.into(), vec![12.into()], vec![merge]).await.unwrap();
    for sn in &split_sns {
        assert_eq!(consensus.pool_note(*sn), None);
    }
    assert_eq!(pool_value(&[merged_sn]), DenominationTag::D0_01.petals(), "merge's produced value must equal its one output note");

    consensus.shutdown(join_handles);
}

/// FORK-PLAN P6.6's own stated verify condition: `Σ pool notes + transparent supply ==
/// emitted supply` holds across a real mint -> transfer -> redeem sequence. Restricted to
/// the value this test itself injects (one funding block's coinbase reward) rather than
/// the whole chain's total emission — every other block mined along the way (including the
/// funding blocks' own predecessor rewards) pays its subsidy to an unrelated null script
/// this test never queries, so it can't leak into the balances checked below and the
/// invariant still holds exactly for this closed subsystem.
#[tokio::test]
async fn value_conservation_across_mint_transfer_redeem() {
    let consensus = TestConsensus::new(&config());
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);

    let balance = |script: &kaspa_consensus_core::tx::ScriptPublicKey| {
        consensus
            .get_virtual_utxos(None, usize::MAX, false)
            .into_iter()
            .filter(|(_, entry)| &entry.script_public_key == script)
            .map(|(_, entry)| entry.amount)
            .sum::<u64>()
    };

    let (outpoint, entry, tip) = fund(&consensus, &alice, genesis, 5.into(), 10.into()).await;
    let emitted = entry.amount;

    // A 0.1-MAGLD note, not 0.01: the redeem step's transparent output has no offsetting
    // transparent input for the storage-mass (KIP-0009) formula to net against, so an
    // output much smaller than `STORAGE_MASS_PARAMETER` would trip the anti-dust storage-
    // mass limit on its own, unrelated to what this test is actually checking.
    //
    // Mint: part of the funding becomes a pool note, the rest stays transparent change.
    let mint = mint_funded(&alice, (outpoint, entry), vec![alice.note(DenominationTag::D0_1)]);
    let sn0 = produced_serial(&mint, 0);
    consensus.add_utxo_valid_block_with_parents(11.into(), vec![tip], vec![mint]).await.unwrap();
    assert_eq!(
        balance(&alice.script()) + balance(&bob.script()) + DenominationTag::D0_1.petals(),
        emitted,
        "transparent change + the minted note must equal the funding it came from"
    );

    // Transfer: alice rotates her note to bob — same total value, now under a different key.
    let rotate = pool_tx(&alice.rotate(vec![sn0], vec![bob.note(DenominationTag::D0_1)], 0));
    let sn1 = produced_serial(&rotate, 0);
    consensus.add_utxo_valid_block_with_parents(12.into(), vec![11.into()], vec![rotate]).await.unwrap();
    assert_eq!(consensus.pool_note(sn0), None);
    assert!(consensus.pool_note(sn1).is_some());
    assert_eq!(
        balance(&alice.script()) + balance(&bob.script()) + DenominationTag::D0_1.petals(),
        emitted,
        "a pure pool transfer must not change total value"
    );

    // Redeem: bob converts his note back to a transparent output.
    let redeem_output = TransactionOutput::new(DenominationTag::D0_1.petals(), bob.script());
    let redeem_op = bob.redeem(vec![sn1], std::slice::from_ref(&redeem_output), 0);
    let redeem = redeem_tx(&redeem_op, vec![redeem_output]);
    consensus.add_utxo_valid_block_with_parents(13.into(), vec![12.into()], vec![redeem]).await.unwrap();
    assert_eq!(consensus.pool_note(sn1), None, "redeemed note must be retired");
    assert_eq!(
        balance(&alice.script()) + balance(&bob.script()),
        emitted,
        "the pool is empty again — all value must be back in the transparent supply"
    );

    consensus.shutdown(join_handles);
}
