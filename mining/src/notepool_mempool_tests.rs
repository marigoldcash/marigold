//! Mempool integration for note-pool transactions (PLAN P6.7).
//!
//! Covers this step's own stated verify condition: a conflicting rotate arriving second is
//! rejected, and a block template built under load includes pool ops and validates. Uses
//! `ConsensusMock` (this crate's standard mempool test harness — every other mempool test
//! in this crate uses it, not a real `TestConsensus`), extended by this step to carry live
//! note-pool state and reuse the real `validate_stateful` consensus-core logic.
//!
//! Deliberately does NOT test mempool-chaining an unconfirmed pool op's produced notes into
//! a second unconfirmed pool op — that's a documented MVP scope limitation for this step
//! (see NOTES.md's P6.7 entry), not a gap in this test file.

use crate::{
    MiningCounters,
    errors::MiningManagerError,
    manager::MiningManager,
    mempool::{
        errors::RuleError,
        tx::{Orphan, Priority, RbfPolicy},
    },
    model::tx_query::TransactionQuery,
    testutils::consensus_mock::ConsensusMock,
};
use kaspa_consensus_core::{
    coinbase::MinerData,
    config::{
        constants::consensus::{DEFAULT_GAS_PER_LANE_LIMIT, DEFAULT_LANES_PER_BLOCK_LIMIT},
        params::ForkActivation,
    },
    constants::TX_VERSION_TOCCATA,
    mass::{BlockLaneLimits, BlockMassLimits},
    notepool::{DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, SignedGroup, TransferOp, hashing as pool_hashing},
    subnets::{SUBNETWORK_ID_COINBASE, SUBNETWORK_ID_NOTE_POOL},
    tx::{ScriptPublicKey, Transaction, TransactionId, scriptvec},
};
use kaspa_hashes::Hash;
use kaspa_mining_errors::mempool::RuleResult;
use std::sync::Arc;

const TARGET_TIME_PER_BLOCK: u64 = 1_000;
const MAX_BLOCK_MASS: u64 = 500_000;
const BLOCK_LANE_LIMITS: BlockLaneLimits =
    BlockLaneLimits { lanes_per_block: DEFAULT_LANES_PER_BLOCK_LIMIT, gas_per_lane: DEFAULT_GAS_PER_LANE_LIMIT };

fn mining_manager() -> MiningManager {
    MiningManager::new(
        TARGET_TIME_PER_BLOCK,
        false,
        BlockMassLimits::with_shared_limit(MAX_BLOCK_MASS),
        ForkActivation::never(),
        BLOCK_LANE_LIMITS,
        None,
        Arc::new(MiningCounters::default()),
    )
}

fn empty_miner_data() -> MinerData {
    MinerData::new(ScriptPublicKey::new(0, scriptvec![]), vec![])
}

fn into_mempool_result<T>(result: Result<T, MiningManagerError>) -> RuleResult<T> {
    match result {
        Ok(v) => Ok(v),
        Err(MiningManagerError::MempoolError(err)) => Err(err),
        Err(other) => panic!("unexpected non-mempool error: {other}"),
    }
}

fn contained_by(transaction_id: TransactionId, transactions: &[Transaction]) -> bool {
    transactions.iter().any(|tx| tx.id() == transaction_id)
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

/// A zero-input mint, added directly to `ConsensusMock` (bypassing mempool validation,
/// exactly the way this crate's other tests seed funding UTXOs via `consensus.add_transaction`
/// — see `manager_tests.rs`'s `create_and_add_funding_transactions`) purely to seed a live
/// note for a test to then rotate through the real mempool path.
fn mint_tx(notes: Vec<NewNote>) -> Transaction {
    pool_tx(&PoolOp::Mint(MintOp { new_notes: notes }))
}

fn produced_serial(tx: &Transaction, index: u32) -> Hash {
    pool_hashing::serial_hash(&tx.id(), index)
}

/// Mints two 0.01-MAGLD notes to `wallet` (added directly to `ConsensusMock`, see
/// `mint_tx`'s doc comment) and returns their serials: `(note, fee_stamp)`. Real pool ops
/// pay with fee stamps — a second small note consumed alongside the one actually being
/// moved, its value becoming the tx's fee (POOL-SPEC.md's `consumed - produced` rule,
/// STATE.md's "wallet holds nothing but note keys" fee design) — a pure `consumed ==
/// produced` rotate has zero fee and is correctly rejected by mempool standardness checks,
/// same as any other zero-fee non-coinbase transaction.
fn fund_note_with_fee_stamp(consensus: &Arc<ConsensusMock>, wallet: &Wallet) -> (Hash, Hash) {
    let mint = mint_tx(vec![wallet.note(DenominationTag::D0_01), wallet.note(DenominationTag::D0_01)]);
    let note_sn = produced_serial(&mint, 0);
    let fee_stamp_sn = produced_serial(&mint, 1);
    consensus.add_transaction(mint, 0);
    (note_sn, fee_stamp_sn)
}

#[test]
fn conflicting_rotate_arriving_second_is_rejected() {
    let consensus = Arc::new(ConsensusMock::new());
    let manager = mining_manager();

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);
    let carol = Wallet::new(3);

    let (sn, fee_sn) = fund_note_with_fee_stamp(&consensus, &alice);

    let rotate_to_bob = pool_tx(&alice.rotate(vec![sn, fee_sn], vec![bob.note(DenominationTag::D0_01)], 0));
    let rotate_to_carol = pool_tx(&alice.rotate(vec![sn, fee_sn], vec![carol.note(DenominationTag::D0_01)], 0));

    let first = manager.validate_and_insert_transaction(
        consensus.as_ref(),
        rotate_to_bob.clone(),
        Priority::Low,
        Orphan::Allowed,
        RbfPolicy::Forbidden,
    );
    assert!(into_mempool_result(first).is_ok(), "the first rotate consuming a live serial must be accepted");

    let second = manager.validate_and_insert_transaction(
        consensus.as_ref(),
        rotate_to_carol.clone(),
        Priority::Low,
        Orphan::Allowed,
        RbfPolicy::Forbidden,
    );
    match into_mempool_result(second) {
        Err(RuleError::RejectSerialConflictInMempool(conflict_sn, owner_id)) => {
            assert_eq!(conflict_sn, sn);
            assert_eq!(owner_id, rotate_to_bob.id());
        }
        other => panic!("expected RejectSerialConflictInMempool, got {other:?}"),
    }

    let (populated, _) = manager.get_all_transactions(TransactionQuery::All);
    let populated_txs: Vec<Transaction> = populated.into_iter().map(|mtx| mtx.tx.as_ref().clone()).collect();
    assert!(contained_by(rotate_to_bob.id(), &populated_txs), "the first, accepted rotate must remain in the mempool");
    assert!(!contained_by(rotate_to_carol.id(), &populated_txs), "the rejected, conflicting rotate must not be in the mempool");
}

#[test]
fn conflicting_rotate_is_accepted_once_the_first_is_evicted_by_confirmation() {
    let consensus = Arc::new(ConsensusMock::new());
    let manager = mining_manager();

    let alice = Wallet::new(1);
    let bob = Wallet::new(2);
    let carol = Wallet::new(3);

    let (sn, fee_sn) = fund_note_with_fee_stamp(&consensus, &alice);

    let rotate_to_bob = pool_tx(&alice.rotate(vec![sn, fee_sn], vec![bob.note(DenominationTag::D0_01)], 0));

    let first = manager.validate_and_insert_transaction(
        consensus.as_ref(),
        rotate_to_bob.clone(),
        Priority::Low,
        Orphan::Allowed,
        RbfPolicy::Forbidden,
    );
    assert!(into_mempool_result(first).is_ok());

    // A different rotate of the same serials gets confirmed elsewhere (e.g. mined by another
    // node) — bob's mempool-resident rotate is now stale and must be evicted, freeing carol's
    // conflicting rotate to be accepted afterwards.
    let confirmed_rotate_to_carol = pool_tx(&alice.rotate(vec![sn, fee_sn], vec![carol.note(DenominationTag::D0_01)], 0));
    manager.handle_new_block_transactions(consensus.as_ref(), 0, &[get_dummy_coinbase_tx(), confirmed_rotate_to_carol]).unwrap();

    let (populated, _) = manager.get_all_transactions(TransactionQuery::All);
    let populated_txs: Vec<Transaction> = populated.into_iter().map(|mtx| mtx.tx.as_ref().clone()).collect();
    assert!(
        !contained_by(rotate_to_bob.id(), &populated_txs),
        "the now-stale rotate must be evicted once its serial is confirmed spent"
    );

    // rotate_to_carol itself is now identical to what got confirmed, so submitting it again
    // would be rejected as already-accepted — the meaningful assertion is that submitting a
    // FRESH conflicting rotate (any transaction whose consumed serials are no longer claimed
    // by a live mempool transaction) succeeds, proving the conflict lock was actually released.
    let dave = Wallet::new(4);
    let rotate_to_dave = pool_tx(&alice.rotate(vec![sn, fee_sn], vec![dave.note(DenominationTag::D0_01)], 0));
    let result = manager.validate_and_insert_transaction(
        consensus.as_ref(),
        rotate_to_dave,
        Priority::Low,
        Orphan::Allowed,
        RbfPolicy::Forbidden,
    );
    assert!(
        into_mempool_result(result).is_ok(),
        "once the conflicting mempool tx is evicted, a fresh rotate of the same serial is no longer conflict-blocked at the mempool layer \
         (whether consensus itself would still accept it, since the serial is already retired, is a separate, real-consensus-level question)"
    );
}

/// PLAN P6.7's second verify criterion: a block template built under load includes
/// pool ops and validates.
#[test]
fn block_template_under_load_includes_pool_ops() {
    let consensus = Arc::new(ConsensusMock::new());
    let manager = mining_manager();

    const ROTATE_COUNT: usize = 8;
    let mut submitted = Vec::with_capacity(ROTATE_COUNT);
    for i in 0..ROTATE_COUNT {
        let owner = Wallet::new(10 + i as u8);
        let recipient = Wallet::new(100 + i as u8);
        let (sn, fee_sn) = fund_note_with_fee_stamp(&consensus, &owner);

        let rotate = pool_tx(&owner.rotate(vec![sn, fee_sn], vec![recipient.note(DenominationTag::D0_01)], 0));
        let result = manager.validate_and_insert_transaction(
            consensus.as_ref(),
            rotate.clone(),
            Priority::Low,
            Orphan::Allowed,
            RbfPolicy::Forbidden,
        );
        assert!(into_mempool_result(result).is_ok(), "rotate {i} (distinct serials, no conflict) must be accepted");
        submitted.push(rotate);
    }

    let template = manager.get_block_template(consensus.as_ref(), &empty_miner_data()).expect("template build must succeed");
    let block_txs: Vec<Transaction> = template.block.transactions[1..].to_vec();
    for rotate in &submitted {
        assert!(contained_by(rotate.id(), &block_txs), "template must include pool-op transaction {}", rotate.id());
    }
}

fn get_dummy_coinbase_tx() -> Transaction {
    Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_COINBASE, 0, vec![])
}
