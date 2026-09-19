use indexmap::IndexSet;
use itertools::Itertools;
use kaspa_consensus::consensus::Consensus;
use kaspa_consensus::model::stores::virtual_state::VirtualStateStoreReader;
use kaspa_consensus::params::Params;
use kaspa_consensus_core::api::ConsensusApi;
use kaspa_consensus_core::block::{Block, TemplateBuildMode, TemplateTransactionSelector};
use kaspa_consensus_core::coinbase::MinerData;
use kaspa_consensus_core::constants::{TX_VERSION, TX_VERSION_TOCCATA};
use kaspa_consensus_core::mass::MassCalculator;
use kaspa_consensus_core::notepool::{
    DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, RedeemOp, SignedGroup, TransferOp, hashing as pool_hashing,
};
use kaspa_consensus_core::sign::sign;
use kaspa_consensus_core::subnets::{SUBNETWORK_ID_NATIVE, SUBNETWORK_ID_NOTE_POOL, SubnetworkId};
use kaspa_consensus_core::tx::{
    MutableTransaction, ScriptPublicKey, ScriptVec, Transaction, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
};
use kaspa_consensus_core::utxo::utxo_view::UtxoView;
use kaspa_core::trace;
use kaspa_hashes::Hash;
use kaspa_utils::sim::{Environment, Process, Resumption, Suspension};
use rand::{Rng, rngs::StdRng};
use rand_distr::{Distribution, Exp};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use std::cmp::max;
use std::iter::once;
use std::sync::Arc;

pub struct LaneContext {
    pub miner_id: u64,
    pub sim_time: u64,
    pub block_index: u64,
    pub tx_index: u64,
    pub outpoint: TransactionOutpoint,
}

pub trait LaneProducer: Send {
    fn next_lane(&mut self, ctx: LaneContext) -> SubnetworkId;
}

pub struct NativeLaneProducer;

impl LaneProducer for NativeLaneProducer {
    fn next_lane(&mut self, _ctx: LaneContext) -> SubnetworkId {
        SUBNETWORK_ID_NATIVE
    }
}

pub struct MinerOptions {
    pub rng: StdRng,
    pub target_txs_per_block: u64,
    pub target_blocks: Option<u64>,
    pub long_payload: bool,
    pub lane_producer: Box<dyn LaneProducer>,
    /// FORK-PLAN P6.10: per-block probability this miner attempts one note-pool
    /// operation (mint/rotate/split-lite/merge/redeem, self-targeted) in addition to
    /// its ordinary native transfers. `0.0` (the default via [`NativeLaneProducer`]'s
    /// own callers) leaves existing simpa behavior completely unchanged.
    pub pool_op_probability: f64,
}

struct OnetimeTxSelector {
    txs: Option<Vec<Transaction>>,
    /// Whether `build_block_template` has rejected any of this batch. Tracked (rather
    /// than the previous unconditional `is_successful() -> true`) so a rejection
    /// surfaces as `build_new_block`'s own `.expect(...)` panicking with the actual
    /// `RuleError` — simulation txs are expected to always be valid, so any rejection
    /// is still a hard bug, just now a debuggable one instead of an opaque
    /// `unimplemented!()` (FORK-PLAN P6.10 found two real bugs this way: the retry
    /// loop's second `select_transactions()` call unconditionally panicking on `None`,
    /// and pool-op transactions needing `toccata_activation`/`pool_activation` forced
    /// on in `main_impl`, same as `crescendo_activation` already was).
    rejected: bool,
}

impl OnetimeTxSelector {
    fn new(txs: Vec<Transaction>) -> Self {
        Self { txs: Some(txs), rejected: false }
    }
}

impl TemplateTransactionSelector for OnetimeTxSelector {
    fn select_transactions(&mut self) -> Vec<Transaction> {
        self.txs.take().unwrap_or_default()
    }

    fn reject_selection(&mut self, _tx_id: kaspa_consensus_core::tx::TransactionId) {
        self.rejected = true;
    }

    fn is_successful(&self) -> bool {
        !self.rejected
    }
}

pub struct Miner {
    // ID
    pub(super) id: u64,

    // Consensus
    pub(super) consensus: Arc<Consensus>,
    pub(super) params: Params,

    // Miner data
    miner_data: MinerData,
    secret_key: secp256k1::SecretKey,
    /// This miner's own x-only public key, used as `NewNote.pk` for every note-pool
    /// op it self-targets (FORK-PLAN P6.10) — the same key material as `miner_data`'s
    /// P2PK script, just in the pool's raw 32-byte form.
    note_pk: [u8; 32],

    // UTXO data related to this miner
    possible_unspent_outpoints: IndexSet<TransactionOutpoint>,
    /// Serials of notes this miner believes it currently owns (FORK-PLAN P6.10),
    /// tracked the same way `possible_unspent_outpoints` tracks UTXOs: populated by
    /// scanning each processed block's own note-pool transactions, pruned on
    /// consumption. Since this miner only ever targets its own `note_pk`, only its
    /// own pool-op transactions ever add to this set.
    possible_notes: IndexSet<Hash>,

    // Rand
    dist: Exp<f64>, // The time interval between Poisson(lambda) events distributes ~Exp(lambda)
    rng: StdRng,

    // Counters
    num_blocks: u64,
    sim_time: u64,

    // Config
    target_txs_per_block: u64,
    target_blocks: Option<u64>,
    max_cached_outpoints: usize,
    long_payload: bool,
    lane_producer: Box<dyn LaneProducer>,
    pool_op_probability: f64,

    // Mass calculator
    mass_calculator: MassCalculator,
}

impl Miner {
    pub fn new(
        id: u64,
        bps: f64,
        hashrate: f64,
        sk: secp256k1::SecretKey,
        pk: secp256k1::PublicKey,
        consensus: Arc<Consensus>,
        params: &Params,
        options: MinerOptions,
    ) -> Self {
        let (schnorr_public_key, _) = pk.x_only_public_key();
        let script_pub_key_script = once(0x20).chain(schnorr_public_key.serialize()).chain(once(0xac)).collect_vec(); // TODO: Use script builder when available to create p2pk properly
        let script_pub_key_script_vec = ScriptVec::from_slice(&script_pub_key_script);
        Self {
            id,
            consensus,
            params: params.clone(),
            miner_data: MinerData::new(ScriptPublicKey::new(0, ScriptVec::from_slice(&script_pub_key_script_vec)), Vec::new()),
            secret_key: sk,
            note_pk: schnorr_public_key.serialize(),
            possible_unspent_outpoints: IndexSet::new(),
            possible_notes: IndexSet::new(),
            dist: Exp::new(bps * hashrate).unwrap(),
            rng: options.rng,
            num_blocks: 0,
            sim_time: 0,
            target_txs_per_block: options.target_txs_per_block,
            target_blocks: options.target_blocks,
            max_cached_outpoints: 10_000,
            mass_calculator: MassCalculator::new(
                params.mass_per_tx_byte,
                params.mass_per_script_pub_key_byte,
                params.storage_mass_parameter,
            ),
            long_payload: options.long_payload,
            lane_producer: options.lane_producer,
            pool_op_probability: options.pool_op_probability,
        }
    }

    fn build_new_block(&mut self, timestamp: u64) -> Block {
        let txs = self.build_txs();
        let nonce = self.id;
        let session = self.consensus.acquire_session();
        let mut block_template = self
            .consensus
            .build_block_template(self.miner_data.clone(), Box::new(OnetimeTxSelector::new(txs)), TemplateBuildMode::Standard)
            .expect("simulation txs are selected in sync with virtual state and are expected to be valid");
        drop(session);
        block_template.block.header.timestamp = timestamp; // Use simulation time rather than real time
        block_template.block.header.nonce = nonce;
        block_template.block.header.finalize();
        block_template.block.to_immutable()
    }

    fn build_txs(&mut self) -> Vec<Transaction> {
        let virtual_read = self.consensus.virtual_stores.read();
        let virtual_state = virtual_read.state.get().unwrap();
        let virtual_utxo_view = &virtual_read.utxo_set;
        let pool_state = &virtual_read.pool_state;
        let multiple_outputs = self.possible_unspent_outpoints.len() < 5_000;
        let schnorr_key = secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &self.secret_key.secret_bytes()).unwrap();
        let mut mutable_txs = Vec::with_capacity(self.target_txs_per_block as usize);

        // FORK-PLAN P6.10: at most one note-pool op per block, ahead of the ordinary
        // native transfers below — reserves any UTXO it consumes (for a mint) before
        // the native loop gets a chance to spend the same outpoint. A free function
        // (not a `&mut self` method) so the compiler can see `rng`/`possible_notes`/
        // `possible_unspent_outpoints` are disjoint from `pool_state`/`virtual_utxo_view`,
        // which alias `self.consensus` through `virtual_read`.
        if let Some(pool_op_tx) = Self::maybe_build_pool_op(
            &mut self.rng,
            &mut self.possible_notes,
            &mut self.possible_unspent_outpoints,
            self.note_pk,
            self.miner_data.script_public_key.clone(),
            &schnorr_key,
            self.pool_op_probability,
            pool_state,
            virtual_utxo_view,
            virtual_state.daa_score,
            self.params.coinbase_maturity(),
        ) {
            mutable_txs.push(pool_op_tx);
        }

        for &outpoint in &self.possible_unspent_outpoints {
            if mutable_txs.len() == self.target_txs_per_block as usize {
                break;
            }
            let Some(entry) = self.get_spendable_entry(virtual_utxo_view, outpoint, virtual_state.daa_score) else {
                continue;
            };
            let tx_index = mutable_txs.len() as u64;
            let lane = self.lane_producer.next_lane(LaneContext {
                miner_id: self.id,
                sim_time: self.sim_time,
                block_index: self.num_blocks,
                tx_index,
                outpoint,
            });
            let mut unsigned_tx = self.create_unsigned_tx(outpoint, entry.amount, multiple_outputs, lane);
            if self.long_payload {
                unsigned_tx.payload = vec![0; 90_000];
            }
            mutable_txs.push(MutableTransaction::with_entries(unsigned_tx, vec![entry]));
        }

        let txs = mutable_txs
            .into_par_iter()
            .map(|mutable_tx| {
                let signed_tx = sign(mutable_tx, schnorr_key);
                let mass = self.mass_calculator.calc_contextual_masses(&signed_tx.as_verifiable()).unwrap().storage_mass;
                signed_tx.tx.set_storage_mass(mass);
                let mut signed_tx = signed_tx.tx;
                signed_tx.finalize();
                signed_tx
            })
            .collect::<Vec<_>>();

        for outpoint in txs.iter().flat_map(|t| t.inputs.iter().map(|i| i.previous_outpoint)) {
            self.possible_unspent_outpoints.swap_remove(&outpoint);
        }
        // Mirror the UTXO removal above for any note this block's pool op consumed —
        // `process_block` (once the block is actually inserted) is what adds newly
        // *produced* serials, but a consumed one must disappear immediately so the next
        // block doesn't try to spend it again before that happens.
        for tx in txs.iter().filter(|t| t.subnetwork_id == SUBNETWORK_ID_NOTE_POOL) {
            if let Some(op) = PoolOp::decode_payload(&tx.payload) {
                for sn in op.consumed_serials() {
                    self.possible_notes.swap_remove(&sn);
                }
            }
        }
        txs
    }

    /// FORK-PLAN P6.10: decides whether to build one note-pool operation this block and,
    /// if so, builds it. A free function rather than a `&mut self` method — its params are
    /// disjoint field projections of `self` (`rng`/`possible_notes`/
    /// `possible_unspent_outpoints`) plus `pool_state`/`virtual_utxo_view`, which alias
    /// `self.consensus` through the read guard `build_txs` already holds; going through
    /// `&mut self` here would conflict with that live borrow.
    ///
    /// Prefers consuming an existing owned note over minting a new one, so the miner's
    /// own note backlog doesn't grow unbounded: two notes sharing a denomination tier
    /// merge into one (the other's value becomes the fee); otherwise a single note
    /// rotates to the next-smaller tier (always a strictly smaller value, so always a
    /// valid positive fee) or, at the smallest tier, redeems back to transparent value
    /// minus a fee. Only mints when there are no live notes to work with at all.
    #[allow(clippy::too_many_arguments)]
    fn maybe_build_pool_op(
        rng: &mut StdRng,
        possible_notes: &mut IndexSet<Hash>,
        possible_unspent_outpoints: &mut IndexSet<TransactionOutpoint>,
        note_pk: [u8; 32],
        change_script: ScriptPublicKey,
        schnorr_key: &secp256k1::Keypair,
        pool_op_probability: f64,
        pool_state: &impl kaspa_consensus::model::stores::notepool::NotePoolStoreReader,
        virtual_utxo_view: &impl UtxoView,
        daa_score: u64,
        coinbase_maturity: u64,
    ) -> Option<MutableTransaction<Transaction>> {
        if pool_op_probability <= 0.0 || rng.r#gen::<f64>() >= pool_op_probability {
            return None;
        }

        let mut live: Vec<(Hash, NewNote)> =
            possible_notes.iter().filter_map(|&sn| pool_state.get(sn).ok().map(|entry| (sn, entry.note))).collect();
        live.sort_by_key(|(_, note)| note.d as u8);

        if let Some(pair) = live.windows(2).find(|w| w[0].1.d == w[1].1.d) {
            let (sn_a, note) = pair[0];
            let (sn_b, _) = pair[1];
            possible_notes.swap_remove(&sn_a);
            possible_notes.swap_remove(&sn_b);
            let produced = vec![NewNote { d: note.d, pk: note_pk }];
            return Some(Self::build_pool_transfer(vec![sn_a, sn_b], produced, schnorr_key, daa_score));
        }
        if let Some(&(sn, note)) = live.iter().find(|(_, note)| note.d as u8 > 0) {
            let lower = DenominationTag::try_from(note.d as u8 - 1).expect("d - 1 is a valid tier since d > 0");
            possible_notes.swap_remove(&sn);
            let produced = vec![NewNote { d: lower, pk: note_pk }];
            return Some(Self::build_pool_transfer(vec![sn], produced, schnorr_key, daa_score));
        }
        if let Some(&(sn, note)) = live.iter().find(|(_, note)| note.d as u8 == 0) {
            possible_notes.swap_remove(&sn);
            let value = note.d.petals();
            let fee = (value / 10).max(1);
            let output = TransactionOutput::new(value - fee, change_script);
            return Some(Self::build_pool_redeem(sn, output, schnorr_key, daa_score));
        }

        // No live notes: mint one, spending a real transparent UTXO. A 2x safety
        // margin below the chosen denomination's petal value comfortably covers the
        // real (small) mass-based relay fee and leaves room for a change output.
        // Matches `get_spendable_entry`'s own maturity check exactly (this is a free
        // function so it can't call that `&self` method directly).
        let is_spendable = |entry: &UtxoEntry| {
            entry.amount >= 2 && !(entry.is_coinbase && (daa_score as i64 - entry.block_daa_score as i64) <= coinbase_maturity as i64)
        };
        let outpoint =
            possible_unspent_outpoints.iter().copied().find(|o| virtual_utxo_view.get(o).is_some_and(|e| is_spendable(&e)))?;
        let entry = virtual_utxo_view.get(&outpoint)?;
        let denomination = (0..8u8)
            .rev()
            .map(|i| DenominationTag::try_from(i).expect("0..8 are all valid tiers"))
            .find(|d| d.petals().saturating_mul(2) <= entry.amount)?;
        possible_unspent_outpoints.swap_remove(&outpoint);
        Some(Self::build_pool_mint(outpoint, entry, denomination, note_pk, change_script))
    }

    fn build_pool_transfer(
        serials: Vec<Hash>,
        produced: Vec<NewNote>,
        schnorr_key: &secp256k1::Keypair,
        anchor_daa_score: u64,
    ) -> MutableTransaction<Transaction> {
        let outputs_hash = pool_hashing::transparent_outputs_hash(&[]);
        let msg_hash = pool_hashing::signing_hash(1, &serials, &produced, outputs_hash, anchor_daa_score);
        let msg = secp256k1::Message::from_digest(msg_hash.into());
        let signature = *schnorr_key.sign_schnorr(msg).as_ref();
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials, signature }],
            produced,
            freshness: FreshnessAnchor { anchor_daa_score },
        });
        let tx =
            Transaction::new_non_finalized(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, op.encode_payload());
        MutableTransaction::with_entries(tx, vec![])
    }

    fn build_pool_redeem(
        sn: Hash,
        output: TransactionOutput,
        schnorr_key: &secp256k1::Keypair,
        anchor_daa_score: u64,
    ) -> MutableTransaction<Transaction> {
        let outputs = vec![output];
        let outputs_hash = pool_hashing::transparent_outputs_hash(&outputs);
        let msg_hash = pool_hashing::signing_hash(2, &[sn], &[], outputs_hash, anchor_daa_score);
        let msg = secp256k1::Message::from_digest(msg_hash.into());
        let signature = *schnorr_key.sign_schnorr(msg).as_ref();
        let op =
            RedeemOp { consumed: vec![SignedGroup { serials: vec![sn], signature }], freshness: FreshnessAnchor { anchor_daa_score } };
        let tx = Transaction::new_non_finalized(
            TX_VERSION_TOCCATA,
            vec![],
            outputs,
            0,
            SUBNETWORK_ID_NOTE_POOL,
            0,
            PoolOp::Redeem(op).encode_payload(),
        );
        MutableTransaction::with_entries(tx, vec![])
    }

    fn build_pool_mint(
        outpoint: TransactionOutpoint,
        entry: UtxoEntry,
        denomination: DenominationTag,
        note_pk: [u8; 32],
        change_script: ScriptPublicKey,
    ) -> MutableTransaction<Transaction> {
        let change = entry.amount - denomination.petals();
        let outputs = if change > 0 { vec![TransactionOutput::new(change, change_script)] } else { vec![] };
        let tx = Transaction::new_non_finalized(
            TX_VERSION_TOCCATA,
            vec![TransactionInput::new(outpoint, vec![], 0, 0)],
            outputs,
            0,
            SUBNETWORK_ID_NOTE_POOL,
            0,
            PoolOp::Mint(MintOp { new_notes: vec![NewNote { d: denomination, pk: note_pk }] }).encode_payload(),
        );
        MutableTransaction::with_entries(tx, vec![entry])
    }

    /// Scans a just-inserted block for note-pool activity affecting this miner's own
    /// `note_pk` (FORK-PLAN P6.10) — the pool analog of `process_block`'s existing UTXO
    /// output scan. Since this miner only ever targets its own key, only its own
    /// transactions (built by `maybe_build_pool_op`) ever touch `possible_notes`.
    fn scan_block_for_notes(&mut self, block: &Block) {
        for tx in block.transactions.iter().filter(|t| t.subnetwork_id == SUBNETWORK_ID_NOTE_POOL) {
            let Some(op) = PoolOp::decode_payload(&tx.payload) else { continue };
            for sn in op.consumed_serials() {
                self.possible_notes.swap_remove(&sn);
            }
            let tx_id = tx.id();
            if let PoolOp::Mint(mint) = &op {
                for (i, note) in mint.new_notes.iter().enumerate() {
                    if note.pk == self.note_pk {
                        self.possible_notes.insert(pool_hashing::serial_hash(&tx_id, i as u32));
                    }
                }
            } else if let PoolOp::Transfer(transfer) = &op {
                for (i, note) in transfer.produced.iter().enumerate() {
                    if note.pk == self.note_pk {
                        self.possible_notes.insert(pool_hashing::serial_hash(&tx_id, i as u32));
                    }
                }
            }
        }
    }

    fn get_spendable_entry(
        &self,
        utxo_view: &impl UtxoView,
        outpoint: TransactionOutpoint,
        virtual_daa_score: u64,
    ) -> Option<UtxoEntry> {
        let entry = utxo_view.get(&outpoint)?;
        if entry.amount < 2
            || (entry.is_coinbase
                && (virtual_daa_score as i64 - entry.block_daa_score as i64) <= self.params.coinbase_maturity() as i64)
        {
            return None;
        }
        Some(entry)
    }

    fn create_unsigned_tx(
        &self,
        outpoint: TransactionOutpoint,
        input_amount: u64,
        multiple_outputs: bool,
        subnetwork_id: SubnetworkId,
    ) -> Transaction {
        Transaction::new_non_finalized(
            if subnetwork_id.is_native() { TX_VERSION } else { TX_VERSION_TOCCATA },
            vec![TransactionInput::new(outpoint, vec![], 0, 0)],
            if multiple_outputs && input_amount > 4 {
                vec![
                    TransactionOutput::new(input_amount / 2, self.miner_data.script_public_key.clone()),
                    TransactionOutput::new(input_amount / 2 - 1, self.miner_data.script_public_key.clone()),
                ]
            } else {
                vec![TransactionOutput::new(input_amount - 1, self.miner_data.script_public_key.clone())]
            },
            0,
            subnetwork_id,
            0,
            vec![],
        )
    }

    pub fn mine(&mut self, env: &mut Environment<Block>) -> Suspension {
        let block = self.build_new_block(env.now());
        env.broadcast(self.id, block);
        self.sample_mining_interval()
    }

    fn sample_mining_interval(&mut self) -> Suspension {
        Suspension::Timeout(max((self.dist.sample(&mut self.rng) * 1000.0) as u64, 1))
    }

    fn process_block(&mut self, block: Block, env: &mut Environment<Block>) -> Suspension {
        for tx in block.transactions.iter() {
            for (i, output) in tx.outputs.iter().enumerate() {
                if output.script_public_key.eq(&self.miner_data.script_public_key) {
                    if self.possible_unspent_outpoints.len() == self.max_cached_outpoints {
                        self.possible_unspent_outpoints.swap_remove_index(self.rng.gen_range(0..self.max_cached_outpoints));
                    }
                    self.possible_unspent_outpoints.insert(TransactionOutpoint::new(tx.id(), i as u32));
                }
            }
        }
        self.scan_block_for_notes(&block);
        if self.report_progress(env) {
            Suspension::Halt
        } else {
            let session = self.consensus.acquire_session();
            let status = futures::executor::block_on(self.consensus.validate_and_insert_block(block).virtual_state_task).unwrap();
            assert!(status.is_utxo_valid_or_pending());
            drop(session);
            Suspension::Idle
        }
    }

    fn report_progress(&mut self, env: &mut Environment<Block>) -> bool {
        self.num_blocks += 1;
        if let Some(target_blocks) = self.target_blocks
            && self.num_blocks > target_blocks
        {
            return true; // Exit
        }
        if self.id != 0 {
            return false;
        }
        if self.num_blocks.is_multiple_of(50) || self.sim_time / 5000 != env.now() / 5000 {
            trace!("Simulation time: {}\tGenerated {} blocks", env.now() as f64 / 1000.0, self.num_blocks);
        }
        self.sim_time = env.now();
        false
    }
}

impl Process<Block> for Miner {
    fn resume(&mut self, resumption: Resumption<Block>, env: &mut Environment<Block>) -> Suspension {
        match resumption {
            Resumption::Initial => self.sample_mining_interval(),
            Resumption::Scheduled => self.mine(env),
            Resumption::Message(block) => self.process_block(block, env),
        }
    }
}
