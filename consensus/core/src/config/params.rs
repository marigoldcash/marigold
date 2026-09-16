pub use super::{
    bps::{Bps, TenBps},
    constants::consensus::*,
    genesis::{DEVNET_GENESIS, GENESIS, GenesisBlock, SIMNET_GENESIS, TESTNET_GENESIS},
};
use crate::{
    BlockLevel, KType,
    constants::{BLOCK_VERSION, NOTE_POOL_BLOCK_VERSION, STORAGE_MASS_PARAMETER, TOCCATA_BLOCK_VERSION},
    mass::{BlockLaneLimits, BlockMassLimits, MassCofactors},
    network::{NetworkId, NetworkType},
};
use kaspa_addresses::Prefix;
use kaspa_math::Uint256;
use serde::{Deserialize, Serialize};
use std::{
    cmp::min,
    ops::{Deref, DerefMut},
};

const MEMPOOL_BLOCK_MASS_ACTIVATION_DELAY_SECONDS: u64 = 24 * 60 * 60;
const PRIOR_MAX_SIGNATURE_SCRIPT_LEN: usize = 10_000;
// Increased for stark proofs. This value is effectively covered by the post-Toccata
// transient block mass limit: 1_000_000 transient mass / 4 grams-per-byte = 250_000
// bytes for the entire block, so a larger signature script cannot be accepted anyway.
// TODO(post-toccata): check whether this early signature-script length guard can be
// removed entirely, or whether it remains useful as cheap early protection.
const NEW_MAX_SIGNATURE_SCRIPT_LEN: usize = 250_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkActivation(u64);

impl ForkActivation {
    const NEVER: u64 = u64::MAX;
    const ALWAYS: u64 = 0;

    pub const fn new(daa_score: u64) -> Self {
        Self(daa_score)
    }

    pub const fn never() -> Self {
        Self(Self::NEVER)
    }

    pub const fn always() -> Self {
        Self(Self::ALWAYS)
    }

    /// Returns the actual DAA score triggering the activation. Should be used only
    /// for cases where the explicit value is required for computations (e.g., coinbase subsidy).
    /// Otherwise, **activation checks should always go through `self.is_active(..)`**
    pub fn daa_score(self) -> u64 {
        self.0
    }

    pub fn is_active(self, current_daa_score: u64) -> bool {
        current_daa_score >= self.0
    }

    pub fn delayed_by(self, daa_score_delta: u64) -> Self {
        match self.0 {
            Self::ALWAYS | Self::NEVER => self,
            daa_score => Self(daa_score.saturating_add(daa_score_delta)),
        }
    }

    pub fn early_by(self, daa_score_delta: u64) -> Self {
        match self.0 {
            Self::ALWAYS | Self::NEVER => self,
            daa_score => Self(daa_score.saturating_sub(daa_score_delta)),
        }
    }

    /// Checks if the fork was "recently" activated, i.e., in the time frame of the provided range.
    /// This function returns false for forks that were always active, since they were never activated.
    pub fn is_within_range_from_activation(self, current_daa_score: u64, range: u64) -> bool {
        self != Self::always() && self.is_active(current_daa_score) && current_daa_score < self.0 + range
    }

    /// Checks if the fork is expected to be activated "soon", i.e., in the time frame of the provided range.
    /// Returns the distance from activation if so, or `None` otherwise.
    pub fn is_within_range_before_activation(self, current_daa_score: u64, range: u64) -> Option<u64> {
        if !self.is_active(current_daa_score) && current_daa_score + range > self.0 { Some(self.0 - current_daa_score) } else { None }
    }
}

/// A consensus parameter which depends on forking activation
#[derive(Clone, Copy, Debug)]
pub struct ForkedParam<T: Copy> {
    pre: T,
    post: T,
    activation: ForkActivation,
}

impl<T: Copy> ForkedParam<T> {
    const fn new(pre: T, post: T, activation: ForkActivation) -> Self {
        Self { pre, post, activation }
    }

    pub const fn new_const(val: T) -> Self {
        Self { pre: val, post: val, activation: ForkActivation::never() }
    }

    pub fn activation(&self) -> ForkActivation {
        self.activation
    }

    pub fn get(&self, daa_score: u64) -> T {
        if self.activation.is_active(daa_score) { self.post } else { self.pre }
    }

    pub fn with_delayed_activation(&self, delay_daa_score: u64) -> Self {
        Self::new(self.pre, self.post, self.activation.delayed_by(delay_daa_score))
    }

    /// Returns the value before activation (=pre unless activation = always)
    pub fn before(&self) -> T {
        match self.activation.0 {
            ForkActivation::ALWAYS => self.post,
            _ => self.pre,
        }
    }

    /// Returns the permanent long-term value after activation (=post unless the activation is never scheduled)
    pub fn after(&self) -> T {
        match self.activation.0 {
            ForkActivation::NEVER => self.pre,
            _ => self.post,
        }
    }

    /// Returns the configured post-fork value regardless of whether activation is scheduled.
    pub fn raw_post(&self) -> T {
        self.post
    }

    /// Maps the ForkedParam<T> to a new ForkedParam<U> by applying a map function on both pre and post
    pub fn map<U: Copy, F: Fn(T) -> U>(&self, f: F) -> ForkedParam<U> {
        ForkedParam::new(f(self.pre), f(self.post), self.activation)
    }
}

impl<T: Copy> From<T> for ForkedParam<T> {
    fn from(value: T) -> Self {
        Self::new_const(value)
    }
}

/// The block version as a function of DAA score, across a genuine three-tier version
/// history (pre-Toccata → Toccata/KIP-21 → note-pool, FORK-PLAN P6.5). Not expressed as
/// a second `ForkedParam<u16>` layered on the first: `ForkedParam` is a strictly binary
/// pre/post construct tied to one activation, and a third tier needs an explicit
/// priority chain (most-recently-activated fork wins), not another independent pair.
#[derive(Clone, Copy, Debug)]
pub struct BlockVersionParam {
    pre_toccata: u16,
    toccata: u16,
    pool: u16,
    toccata_activation: ForkActivation,
    pool_activation: ForkActivation,
}

impl BlockVersionParam {
    pub fn get(&self, daa_score: u64) -> u16 {
        if self.pool_activation.is_active(daa_score) {
            self.pool
        } else if self.toccata_activation.is_active(daa_score) {
            self.toccata
        } else {
            self.pre_toccata
        }
    }
}

impl<T: Copy + Ord> ForkedParam<T> {
    /// Returns the min of `pre` and `post` values. Useful for non-consensus initializations
    /// which require knowledge of the value bounds.
    ///
    /// Note that if activation is not scheduled (set to never) then pre is always returned,
    /// and if activation is set to always (since inception), post will be returned.
    pub fn lower_bound(&self) -> T {
        match self.activation.0 {
            ForkActivation::NEVER => self.pre,
            ForkActivation::ALWAYS => self.post,
            _ => self.pre.min(self.post),
        }
    }

    /// Returns the max of `pre` and `post` values. Useful for non-consensus initializations
    /// which require knowledge of the value bounds.
    ///
    /// Note that if activation is not scheduled (set to never) then pre is always returned,
    /// and if activation is set to always (since inception), post will be returned.
    pub fn upper_bound(&self) -> T {
        match self.activation.0 {
            ForkActivation::NEVER => self.pre,
            ForkActivation::ALWAYS => self.post,
            _ => self.pre.max(self.post),
        }
    }
}

/// Launch finality-anchor consensus params (POOL-SPEC.md P5.8, FORK-PLAN P6.11).
/// Grouped under one struct — same reasoning as [`BlockrateParams`] — so the four
/// network const blocks and [`OverrideParams`] each carry a single field.
///
/// All score units are DAA-score units, never wall-clock (the P5.8 v1.1 redefinition
/// that makes the equivocation rule exactly decidable).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalityAnchorParams {
    /// The 5 pinned trustee public keys (BIP340 x-only), shipped with the software —
    /// the same bootstrap trust root as the genesis block and DNS seeders. `None`
    /// disables the mechanism entirely (permanent fail-open): the real keys are
    /// produced by the P9.1 trustee ceremony and pinned before mainnet launch; until
    /// then every network runs with `None` and tests inject generated keys.
    pub trustees: Option<crate::finality_anchor::TrusteeKeys>,

    /// Minimum anchored-block depth below the accepting context, in DAA-score units
    /// (600 at launch, ~1 minute at nominal 10 BPS).
    pub depth: u64,

    /// The stage-0 (launch) cadence interval: 300 DAA-score-units, ~30s at 10 BPS.
    pub launch_interval: u64,

    /// The staged cadence-decay schedule, stages 1–3 of the P5.8 table:
    /// `(activation, interval)` per stage, later stages superseding earlier ones once
    /// active. **The activations ship as `ForkActivation::never()`**: stages 1–2 are
    /// triggered by the sustained-difficulty condition (median ≥ T for M months, plus
    /// the K-year floor) whose threshold T is explicitly *not final* until the
    /// P9.5-gated sensitivity model exists (POOL-SPEC.md P5.8, "Calibration is a hard
    /// pre-launch gate") — wiring live trigger evaluation against an unfrozen T would
    /// be premature. These `ForkActivation` scores are the exact hook that evaluation
    /// (or the FORK-PLAN P9.x governance mechanism that upgrades the sunset story)
    /// sets when the parameters freeze. Stage 4 (expiry) is [`Self::hard_expiry`],
    /// enforced unconditionally regardless of these stages.
    pub decay_stages: [(ForkActivation, u64); 3],

    /// The hard maximum DAA score (stage 4): at and beyond it, trustee keys are
    /// consensus-expired unconditionally — no anchor has any consensus effect, no
    /// matter what the decay stages say or whether their triggers ever fired.
    /// "Trust must end even if network growth disappoints."
    pub hard_expiry_daa_score: u64,
}

impl FinalityAnchorParams {
    /// The launch configuration with **no keys pinned** — the mechanism ships inert on
    /// every network until the P9.1 ceremony produces real trustee keys.
    pub const LAUNCH_UNKEYED: Self = Self {
        trustees: None,
        depth: crate::finality_anchor::FINALITY_ANCHOR_DEPTH,
        launch_interval: crate::finality_anchor::FINALITY_ANCHOR_LAUNCH_INTERVAL,
        decay_stages: [
            (ForkActivation::never(), crate::finality_anchor::ANCHOR_STAGE_INTERVALS[0]),
            (ForkActivation::never(), crate::finality_anchor::ANCHOR_STAGE_INTERVALS[1]),
            (ForkActivation::never(), crate::finality_anchor::ANCHOR_STAGE_INTERVALS[2]),
        ],
        hard_expiry_daa_score: crate::finality_anchor::FINALITY_ANCHOR_HARD_EXPIRY_DAA_SCORE,
    };

    /// The cadence interval of the decay stage active at `daa_score` — the latest
    /// activated stage wins; stage 0 (`launch_interval`) if none has activated.
    pub fn cadence_interval(&self, daa_score: u64) -> u64 {
        self.decay_stages
            .iter()
            .rev()
            .find_map(|(activation, interval)| activation.is_active(daa_score).then_some(*interval))
            .unwrap_or(self.launch_interval)
    }

    /// The fail-open staleness bound at `daa_score`: `depth + 3 × interval`, scaling
    /// with the active decay stage so the *relative* tolerance for a missed anchor
    /// stays constant as the cadence stretches (POOL-SPEC.md P5.8).
    pub fn staleness_bound(&self, daa_score: u64) -> u64 {
        self.depth + crate::finality_anchor::ANCHOR_STALENESS_INTERVALS * self.cadence_interval(daa_score)
    }

    /// Whether the trustee keys are consensus-expired at `daa_score` (stage 4).
    pub fn expired(&self, daa_score: u64) -> bool {
        daa_score >= self.hard_expiry_daa_score
    }
}

/// Blockrate-related consensus params.
/// Grouped together under a single struct because they are logically related and
/// in order to easily support **future BPS acceleration hardforks** (by simply adding
/// a forked instance of blockrate params to the main [`Params`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockrateParams {
    pub target_time_per_block: u64, // (milliseconds)
    pub ghostdag_k: KType,
    pub past_median_time_sample_rate: u64,
    pub difficulty_sample_rate: u64,
    pub max_block_parents: u8,
    pub mergeset_size_limit: u64,
    pub merge_depth: u64,
    pub finality_depth: u64,
    pub pruning_depth: u64,
    pub coinbase_maturity: u64,
}

impl BlockrateParams {
    pub const fn new<const BPS: u64>() -> Self {
        Self {
            target_time_per_block: Bps::<BPS>::target_time_per_block(),
            ghostdag_k: Bps::<BPS>::ghostdag_k(),
            past_median_time_sample_rate: Bps::<BPS>::past_median_time_sample_rate(),
            difficulty_sample_rate: Bps::<BPS>::difficulty_adjustment_sample_rate(),
            max_block_parents: Bps::<BPS>::max_block_parents(),
            mergeset_size_limit: Bps::<BPS>::mergeset_size_limit(),
            merge_depth: Bps::<BPS>::merge_depth_bound(),
            finality_depth: Bps::<BPS>::finality_depth(),
            pruning_depth: Bps::<BPS>::pruning_depth(),
            coinbase_maturity: Bps::<BPS>::coinbase_maturity(),
        }
    }

    pub const fn increase_max_block_parents(mut self, max_block_parents: u8) -> Self {
        if self.max_block_parents < max_block_parents {
            self.max_block_parents = max_block_parents;
        }
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverrideParams {
    /// Timestamp deviation tolerance (in seconds)
    pub timestamp_deviation_tolerance: Option<u64>,

    /// Size of the sampled block window that is used to calculate the past median time of each block
    pub past_median_time_window_size: Option<usize>,

    /// Size of the sampled block window that is used to calculate the required difficulty of each block
    pub difficulty_window_size: Option<usize>,

    /// The minimum size a difficulty window (full or sampled) must have to trigger a DAA calculation
    pub min_difficulty_window_size: Option<usize>,

    pub coinbase_payload_script_public_key_max_len: Option<u8>,
    pub max_coinbase_payload_len: Option<usize>,

    pub max_tx_inputs: Option<usize>,
    pub max_tx_outputs: Option<usize>,
    pub prior_max_signature_script_len: Option<usize>,
    pub new_max_signature_script_len: Option<usize>,
    pub max_script_public_key_len: Option<usize>,
    pub mass_per_tx_byte: Option<u64>,
    pub mass_per_script_pub_key_byte: Option<u64>,
    pub mass_per_sig_op: Option<u64>,
    pub prior_block_mass_limits: Option<BlockMassLimits>,
    pub new_transient_mass_limit: Option<u64>,
    pub block_lane_limits: Option<BlockLaneLimits>,

    /// The parameter for scaling inverse KAS value to mass units (KIP-0009)
    pub storage_mass_parameter: Option<u64>,

    /// DAA score after which the pre-deflationary period switches to the deflationary period
    pub deflationary_phase_daa_score: Option<u64>,

    pub pre_deflationary_phase_base_subsidy: Option<u64>,
    pub skip_proof_of_work: Option<bool>,
    pub max_block_level: Option<BlockLevel>,
    pub pruning_proof_m: Option<u64>,

    /// Blockrate-related params
    pub blockrate: Option<BlockrateParams>,

    /// Target time per block prior to the crescendo hardfork (in milliseconds)
    pub pre_crescendo_target_time_per_block: Option<u64>,

    /// Crescendo activation DAA score
    pub crescendo_activation: Option<ForkActivation>,

    pub toccata_activation: Option<ForkActivation>,

    /// Note-pool activation DAA score (POOL-SPEC.md P5.1, FORK-PLAN P6.5)
    pub pool_activation: Option<ForkActivation>,

    /// Time-locked notes activation DAA score (POOL-SPEC.md P5.9, FORK-PLAN P8.0g)
    pub note_locks_activation: Option<ForkActivation>,

    /// Launch finality-anchor params (POOL-SPEC.md P5.8, FORK-PLAN P6.11)
    pub finality_anchor: Option<FinalityAnchorParams>,
}

impl From<Params> for OverrideParams {
    fn from(p: Params) -> Self {
        Self {
            timestamp_deviation_tolerance: Some(p.timestamp_deviation_tolerance),
            pre_crescendo_target_time_per_block: Some(p.pre_crescendo_target_time_per_block),
            difficulty_window_size: Some(p.difficulty_window_size),
            past_median_time_window_size: Some(p.past_median_time_window_size),
            min_difficulty_window_size: Some(p.min_difficulty_window_size),
            coinbase_payload_script_public_key_max_len: Some(p.coinbase_payload_script_public_key_max_len),
            max_coinbase_payload_len: Some(p.max_coinbase_payload_len),
            max_tx_inputs: Some(p.max_tx_inputs),
            max_tx_outputs: Some(p.max_tx_outputs),
            prior_max_signature_script_len: Some(p.prior_max_signature_script_len),
            new_max_signature_script_len: Some(p.new_max_signature_script_len),
            max_script_public_key_len: Some(p.max_script_public_key_len),
            mass_per_tx_byte: Some(p.mass_per_tx_byte),
            mass_per_script_pub_key_byte: Some(p.mass_per_script_pub_key_byte),
            mass_per_sig_op: Some(p.mass_per_sig_op),
            prior_block_mass_limits: Some(p.prior_block_mass_limits),
            new_transient_mass_limit: Some(p.new_transient_mass_limit),
            block_lane_limits: Some(p.block_lane_limits),
            storage_mass_parameter: Some(p.storage_mass_parameter),
            deflationary_phase_daa_score: Some(p.deflationary_phase_daa_score),
            pre_deflationary_phase_base_subsidy: Some(p.pre_deflationary_phase_base_subsidy),
            skip_proof_of_work: Some(p.skip_proof_of_work),
            max_block_level: Some(p.max_block_level),
            pruning_proof_m: Some(p.pruning_proof_m),
            blockrate: Some(p.blockrate),
            crescendo_activation: Some(p.crescendo_activation),
            toccata_activation: Some(p.toccata_activation),
            pool_activation: Some(p.pool_activation),
            note_locks_activation: Some(p.note_locks_activation),
            finality_anchor: Some(p.finality_anchor),
        }
    }
}

/// Consensus parameters. Contains settings and configurations which are consensus-sensitive.
/// Changing one of these on a network node would exclude and prevent it from reaching consensus
/// with the other unmodified nodes.
#[derive(Clone, Debug)]
pub struct Params {
    pub dns_seeders: &'static [&'static str],
    pub net: NetworkId,
    pub genesis: GenesisBlock,

    /// Timestamp deviation tolerance (in seconds)
    pub timestamp_deviation_tolerance: u64,

    /// Defines the highest allowed proof of work difficulty value for a block as a [`Uint256`]
    pub max_difficulty_target: Uint256,

    /// Highest allowed proof of work difficulty as a floating number
    pub max_difficulty_target_f64: f64,

    /// Size of the sampled block window that is used to calculate the past median time of each block
    pub past_median_time_window_size: usize,

    /// Size of the sampled block window that is used to calculate the required difficulty of each block
    pub difficulty_window_size: usize,

    /// The minimum size a difficulty window must have to trigger a DAA calculation
    pub min_difficulty_window_size: usize,

    pub coinbase_payload_script_public_key_max_len: u8,
    pub max_coinbase_payload_len: usize,

    pub max_tx_inputs: usize,
    pub max_tx_outputs: usize,
    pub prior_max_signature_script_len: usize,
    pub new_max_signature_script_len: usize,
    pub max_script_public_key_len: usize,

    pub mass_per_tx_byte: u64,
    pub mass_per_script_pub_key_byte: u64,
    pub mass_per_sig_op: u64,
    pub prior_block_mass_limits: BlockMassLimits,
    pub new_transient_mass_limit: u64,
    pub block_lane_limits: BlockLaneLimits,

    /// The parameter for scaling inverse KAS value to mass units (KIP-0009)
    pub storage_mass_parameter: u64,

    /// DAA score after which the pre-deflationary period switches to the deflationary period
    pub deflationary_phase_daa_score: u64,

    pub pre_deflationary_phase_base_subsidy: u64,
    pub skip_proof_of_work: bool,
    pub max_block_level: BlockLevel,
    pub pruning_proof_m: u64,

    /// Blockrate-related params
    pub blockrate: BlockrateParams,

    /// Target time per block prior to the crescendo hardfork (in milliseconds).
    /// Required permanently in order to calculate the subsidy month from the current DAA score
    pub pre_crescendo_target_time_per_block: u64,

    /// Crescendo activation DAA score
    pub crescendo_activation: ForkActivation,

    pub toccata_activation: ForkActivation,

    /// Note-pool activation DAA score (POOL-SPEC.md P5.1, FORK-PLAN P6.5) — from this
    /// score onward, block headers carry a meaningful `pool_commitment` and blocks are
    /// mined at `NOTE_POOL_BLOCK_VERSION`. Modeled as its own `ForkActivation`, not a
    /// reuse of `toccata_activation`, per the same "one field, one meaning" reasoning
    /// `pool_commitment` itself follows — pool ops already require Toccata to be active
    /// (the user-lane subnetwork check gates on `TX_VERSION_TOCCATA`), so in practice
    /// this activates no earlier than Toccata on every network, but it is a genuinely
    /// separate switch.
    pub pool_activation: ForkActivation,

    /// Time-locked notes (POOL-SPEC.md P5.9, FORK-PLAN P8.0g): from this DAA score
    /// onward a `TransferLocked` op is valid and a note may carry a lock. Its own
    /// switch, after `pool_activation`, so every node reading locks has upgraded.
    pub note_locks_activation: ForkActivation,

    /// Launch finality-anchor params (POOL-SPEC.md P5.8, FORK-PLAN P6.11): the pinned
    /// trustee keys, anchoring depth, staged cadence schedule, and the unconditional
    /// hard trustee-expiry score. Ships unkeyed (mechanism inert) on every network
    /// until the P9.1 trustee ceremony.
    pub finality_anchor: FinalityAnchorParams,
}

impl Params {
    /// Returns the past median time sample rate
    #[inline]
    #[must_use]
    pub fn past_median_time_sample_rate(&self) -> u64 {
        self.blockrate.past_median_time_sample_rate
    }

    /// Returns the difficulty sample rate
    #[inline]
    #[must_use]
    pub fn difficulty_sample_rate(&self) -> u64 {
        self.blockrate.difficulty_sample_rate
    }

    /// Returns the target time per block (milliseconds)
    #[inline]
    #[must_use]
    pub fn target_time_per_block(&self) -> u64 {
        self.blockrate.target_time_per_block
    }

    /// Returns the expected number of blocks per second
    #[inline]
    #[must_use]
    pub fn bps(&self) -> u64 {
        1000 / self.blockrate.target_time_per_block
    }

    /// Returns the expected number of blocks per second throughout history (currently represented as [`ForkedParam`]).
    /// Required permanently in order to calculate the subsidy month from the current DAA score.
    #[inline]
    #[must_use]
    pub fn bps_history(&self) -> ForkedParam<u64> {
        ForkedParam::new(
            1000 / self.pre_crescendo_target_time_per_block,
            1000 / self.blockrate.target_time_per_block,
            self.crescendo_activation,
        )
    }

    /// Returns the forked per-dimension block mass limits.
    #[inline]
    #[must_use]
    pub fn block_mass_limits(&self) -> ForkedParam<BlockMassLimits> {
        let mut new_block_mass_limits = self.prior_block_mass_limits;
        new_block_mass_limits.transient = self.new_transient_mass_limit;
        ForkedParam::new(self.prior_block_mass_limits, new_block_mass_limits, self.toccata_activation)
    }

    /// Returns the forked cofactors for normalizing block mass dimensions.
    #[inline]
    #[must_use]
    pub fn block_mass_cofactors(&self) -> ForkedParam<MassCofactors> {
        self.block_mass_limits().map(|limits| limits.cofactors())
    }

    /// Returns the block mass limits used for mempool policy.
    ///
    /// Mempool policy lags the consensus transient mass relaxation, so transactions
    /// near activation are normalized by the stricter pre-activation limits.
    #[inline]
    #[must_use]
    pub fn mempool_block_mass_limits(&self) -> ForkedParam<BlockMassLimits> {
        let block_mass_limits = self.block_mass_limits();
        let prior_limits = block_mass_limits.before();
        let new_limits = block_mass_limits.after();
        assert_eq!(
            new_limits.compute, prior_limits.compute,
            "delaying mempool mass activation assumes the compute mass limit does not change"
        );
        assert_eq!(
            new_limits.storage, prior_limits.storage,
            "delaying mempool mass activation assumes the storage mass limit does not change"
        );
        assert!(
            new_limits.transient >= prior_limits.transient,
            "delaying mempool mass activation is only safe when the post-activation transient limit is not stricter"
        );

        block_mass_limits.with_delayed_activation(MEMPOOL_BLOCK_MASS_ACTIVATION_DELAY_SECONDS.saturating_mul(self.bps()))
    }

    /// Returns the mempool policy cofactors for normalizing block mass dimensions.
    #[inline]
    #[must_use]
    pub fn mempool_block_mass_cofactors(&self) -> ForkedParam<MassCofactors> {
        let cofactors = self.mempool_block_mass_limits().map(|limits| limits.cofactors());
        assert_eq!(
            cofactors.before().reference,
            cofactors.after().reference,
            "mempool mass normalization assumes the reference mass is stable across activation"
        );
        cofactors
    }

    /// Returns the forked maximum signature script length.
    #[inline]
    #[must_use]
    pub fn max_signature_script_len(&self) -> ForkedParam<usize> {
        ForkedParam::new(self.prior_max_signature_script_len, self.new_max_signature_script_len, self.toccata_activation)
    }

    pub fn ghostdag_k(&self) -> KType {
        self.blockrate.ghostdag_k
    }

    pub fn max_block_parents(&self) -> u8 {
        self.blockrate.max_block_parents
    }

    pub fn mergeset_size_limit(&self) -> u64 {
        self.blockrate.mergeset_size_limit
    }

    pub fn merge_depth(&self) -> u64 {
        self.blockrate.merge_depth
    }

    pub fn finality_depth(&self) -> u64 {
        self.blockrate.finality_depth
    }

    pub fn pruning_depth(&self) -> u64 {
        self.blockrate.pruning_depth
    }

    pub fn coinbase_maturity(&self) -> u64 {
        self.blockrate.coinbase_maturity
    }

    pub fn finality_duration_in_milliseconds(&self) -> u64 {
        self.blockrate.target_time_per_block * self.blockrate.finality_depth
    }

    pub fn difficulty_window_duration_in_block_units(&self) -> u64 {
        self.blockrate.difficulty_sample_rate * self.difficulty_window_size as u64
    }

    pub fn expected_difficulty_window_duration_in_milliseconds(&self) -> u64 {
        self.blockrate.target_time_per_block * self.blockrate.difficulty_sample_rate * self.difficulty_window_size as u64
    }

    /// Returns the depth at which the anticone of a chain block is final (i.e., is a permanently closed set).
    /// Based on the analysis at <https://github.com/kaspanet/docs/blob/main/Reference/prunality/Prunality.pdf>
    /// and on the decomposition of merge depth (rule R-I therein) from finality depth (φ)
    pub fn anticone_finalization_depth(&self) -> u64 {
        let anticone_finalization_depth = self.blockrate.finality_depth
            + self.blockrate.merge_depth
            + 4 * self.blockrate.mergeset_size_limit * self.blockrate.ghostdag_k as u64
            + 2 * self.blockrate.ghostdag_k as u64
            + 2;

        // In mainnet it's guaranteed that `self.pruning_depth` is greater
        // than `anticone_finalization_depth`, but for some tests we use
        // a smaller (unsafe) pruning depth, so we return the minimum of
        // the two to avoid a situation where a block can be pruned and
        // not finalized.
        min(self.blockrate.pruning_depth, anticone_finalization_depth)
    }

    pub fn block_version(&self) -> BlockVersionParam {
        BlockVersionParam {
            pre_toccata: BLOCK_VERSION,
            toccata: TOCCATA_BLOCK_VERSION,
            pool: NOTE_POOL_BLOCK_VERSION,
            toccata_activation: self.toccata_activation,
            pool_activation: self.pool_activation,
        }
    }

    pub fn network_name(&self) -> String {
        self.net.to_prefixed()
    }

    pub fn prefix(&self) -> Prefix {
        self.net.into()
    }

    pub fn default_p2p_port(&self) -> u16 {
        self.net.default_p2p_port()
    }

    pub fn default_rpc_port(&self) -> u16 {
        self.net.default_rpc_port()
    }

    pub fn override_params(self, overrides: OverrideParams) -> Self {
        Self {
            dns_seeders: self.dns_seeders,
            net: self.net,
            genesis: self.genesis.clone(),

            timestamp_deviation_tolerance: overrides.timestamp_deviation_tolerance.unwrap_or(self.timestamp_deviation_tolerance),

            max_difficulty_target: self.max_difficulty_target,
            max_difficulty_target_f64: self.max_difficulty_target_f64,

            difficulty_window_size: overrides.difficulty_window_size.unwrap_or(self.difficulty_window_size),
            past_median_time_window_size: overrides.past_median_time_window_size.unwrap_or(self.past_median_time_window_size),
            min_difficulty_window_size: overrides.min_difficulty_window_size.unwrap_or(self.min_difficulty_window_size),

            coinbase_payload_script_public_key_max_len: overrides
                .coinbase_payload_script_public_key_max_len
                .unwrap_or(self.coinbase_payload_script_public_key_max_len),

            max_coinbase_payload_len: overrides.max_coinbase_payload_len.unwrap_or(self.max_coinbase_payload_len),

            max_tx_inputs: overrides.max_tx_inputs.unwrap_or(self.max_tx_inputs),
            max_tx_outputs: overrides.max_tx_outputs.unwrap_or(self.max_tx_outputs),
            prior_max_signature_script_len: overrides.prior_max_signature_script_len.unwrap_or(self.prior_max_signature_script_len),
            new_max_signature_script_len: overrides.new_max_signature_script_len.unwrap_or(self.new_max_signature_script_len),
            max_script_public_key_len: overrides.max_script_public_key_len.unwrap_or(self.max_script_public_key_len),
            mass_per_tx_byte: overrides.mass_per_tx_byte.unwrap_or(self.mass_per_tx_byte),
            mass_per_script_pub_key_byte: overrides.mass_per_script_pub_key_byte.unwrap_or(self.mass_per_script_pub_key_byte),
            mass_per_sig_op: overrides.mass_per_sig_op.unwrap_or(self.mass_per_sig_op),
            prior_block_mass_limits: overrides.prior_block_mass_limits.unwrap_or(self.prior_block_mass_limits),
            new_transient_mass_limit: overrides.new_transient_mass_limit.unwrap_or(self.new_transient_mass_limit),
            block_lane_limits: overrides.block_lane_limits.unwrap_or(self.block_lane_limits),

            storage_mass_parameter: overrides.storage_mass_parameter.unwrap_or(self.storage_mass_parameter),

            deflationary_phase_daa_score: overrides.deflationary_phase_daa_score.unwrap_or(self.deflationary_phase_daa_score),

            pre_deflationary_phase_base_subsidy: overrides
                .pre_deflationary_phase_base_subsidy
                .unwrap_or(self.pre_deflationary_phase_base_subsidy),

            skip_proof_of_work: overrides.skip_proof_of_work.unwrap_or(self.skip_proof_of_work),

            max_block_level: overrides.max_block_level.unwrap_or(self.max_block_level),

            pruning_proof_m: overrides.pruning_proof_m.unwrap_or(self.pruning_proof_m),

            blockrate: overrides.blockrate.clone().unwrap_or(self.blockrate.clone()),

            pre_crescendo_target_time_per_block: overrides
                .pre_crescendo_target_time_per_block
                .unwrap_or(self.pre_crescendo_target_time_per_block),

            crescendo_activation: overrides.crescendo_activation.unwrap_or(self.crescendo_activation),
            toccata_activation: overrides.toccata_activation.unwrap_or(self.toccata_activation),
            pool_activation: overrides.pool_activation.unwrap_or(self.pool_activation),
            note_locks_activation: overrides.note_locks_activation.unwrap_or(self.note_locks_activation),
            finality_anchor: overrides.finality_anchor.unwrap_or(self.finality_anchor),
        }
    }
}

impl Deref for Params {
    type Target = BlockrateParams;

    fn deref(&self) -> &Self::Target {
        &self.blockrate
    }
}

impl DerefMut for Params {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.blockrate
    }
}

impl From<NetworkType> for Params {
    fn from(value: NetworkType) -> Self {
        match value {
            NetworkType::Mainnet => MAINNET_PARAMS,
            NetworkType::Testnet => TESTNET_PARAMS,
            NetworkType::Devnet => DEVNET_PARAMS,
            NetworkType::Simnet => SIMNET_PARAMS,
        }
    }
}

impl From<NetworkId> for Params {
    fn from(value: NetworkId) -> Self {
        match value.network_type {
            NetworkType::Mainnet => MAINNET_PARAMS,
            NetworkType::Testnet => match value.suffix {
                Some(10) => TESTNET_PARAMS,
                Some(x) => panic!("Testnet suffix {} is not supported", x),
                None => panic!("Testnet suffix not provided"),
            },
            NetworkType::Devnet => DEVNET_PARAMS,
            NetworkType::Simnet => SIMNET_PARAMS,
        }
    }
}

/// Testnet-10's time-locked-notes activation (POOL-SPEC.md P5.9). Set past the score
/// at which every node the founder runs has the reading code, with a margin for the
/// testers' embedded nodes; an older node stops at the first locked transfer.
pub const TESTNET_NOTE_LOCKS_ACTIVATION_DAA_SCORE: u64 = 11_400_000;

pub const MAINNET_PARAMS: Params = Params {
    // Kaspa's DNS seeders removed (P2.4) — this is a different network, their seeders
    // would only ever return Kaspa peers, which P2.3's handshake now rejects anyway.
    // Marigold's own seeders are stood up at P9.2, near mainnet launch.
    dns_seeders: &[],
    net: NetworkId::new(NetworkType::Mainnet),
    genesis: GENESIS,
    timestamp_deviation_tolerance: TIMESTAMP_DEVIATION_TOLERANCE,
    max_difficulty_target: MAX_DIFFICULTY_TARGET,
    max_difficulty_target_f64: MAX_DIFFICULTY_TARGET_AS_F64,
    past_median_time_window_size: MEDIAN_TIME_SAMPLED_WINDOW_SIZE as usize,
    difficulty_window_size: DIFFICULTY_SAMPLED_WINDOW_SIZE as usize,
    min_difficulty_window_size: MIN_DIFFICULTY_WINDOW_SIZE,
    coinbase_payload_script_public_key_max_len: 150,
    max_coinbase_payload_len: 204,

    // Limit the cost of calculating compute/transient/storage masses
    max_tx_inputs: 1000,
    max_tx_outputs: 1000,
    // Transient mass enforces a limit of 125Kb, however script engine max scripts size is 10Kb so there's no point in surpassing that.
    prior_max_signature_script_len: PRIOR_MAX_SIGNATURE_SCRIPT_LEN,
    new_max_signature_script_len: NEW_MAX_SIGNATURE_SCRIPT_LEN,
    // Compute mass enforces a limit of ~45.5Kb, however script engine max scripts size is 10Kb so there's no point in surpassing that.
    // Note that storage mass will kick in and gradually penalize also for lower lengths (generalized KIP-0009, plurality will be high).
    max_script_public_key_len: 10_000,

    mass_per_tx_byte: 1,
    mass_per_script_pub_key_byte: 10,
    mass_per_sig_op: 1000,
    prior_block_mass_limits: BlockMassLimits::with_shared_limit(500_000),
    new_transient_mass_limit: 1_000_000,
    block_lane_limits: BlockLaneLimits { lanes_per_block: DEFAULT_LANES_PER_BLOCK_LIMIT, gas_per_lane: DEFAULT_GAS_PER_LANE_LIMIT },

    storage_mass_parameter: STORAGE_MASS_PARAMETER,

    // Deflationary from genesis (P1.4 decision: "no pre-deflationary phase") — this is a
    // from-scratch chain with no launch-outage history to protect, unlike real Kaspa's
    // value here (which encoded a real 3-day post-launch network outage). The real
    // subsidy table (P3.2) defines emission from block 0; pre_deflationary_phase_base_subsidy
    // is unused when deflationary_phase_daa_score is 0, kept as a harmless placeholder.
    deflationary_phase_daa_score: 0,
    pre_deflationary_phase_base_subsidy: TenBps::pre_deflationary_phase_base_subsidy(),
    skip_proof_of_work: false,
    max_block_level: 225,
    pruning_proof_m: 1000,

    blockrate: BlockrateParams::new::<10>(),

    // Matches `blockrate` — no real pre-crescendo history exists on a from-scratch
    // chain, so "before" and "after" are the same rate (mirrors simnet/devnet).
    pre_crescendo_target_time_per_block: TenBps::target_time_per_block(),

    // A new chain starts with all upgrades active from block 0 — no history to protect (P2.6).
    crescendo_activation: ForkActivation::always(),
    toccata_activation: ForkActivation::always(),
    pool_activation: ForkActivation::always(),
    note_locks_activation: ForkActivation::never(),
    finality_anchor: FinalityAnchorParams::LAUNCH_UNKEYED,
};

pub const TESTNET_PARAMS: Params = Params {
    // Three static seed hostnames (P8, "Live infrastructure" in STATE.md) — DNS-only
    // (not Cloudflare-proxied; proxying breaks P2P) A records at fixed IPs, provisioned
    // via deploy/ansible/. Sufficient at this scale; the NS-delegated `dnsseeder` crawler
    // remains P9.2, deferred until mainnet needs more than 3 fixed seeds.
    dns_seeders: &["tn-seed1.marigold.cash", "tn-seed2.marigold.cash", "tn-seed3.marigold.cash"],
    net: NetworkId::with_suffix(NetworkType::Testnet, 10),
    genesis: TESTNET_GENESIS,
    timestamp_deviation_tolerance: TIMESTAMP_DEVIATION_TOLERANCE,
    max_difficulty_target: MAX_DIFFICULTY_TARGET,
    max_difficulty_target_f64: MAX_DIFFICULTY_TARGET_AS_F64,
    past_median_time_window_size: MEDIAN_TIME_SAMPLED_WINDOW_SIZE as usize,
    difficulty_window_size: DIFFICULTY_SAMPLED_WINDOW_SIZE as usize,
    min_difficulty_window_size: MIN_DIFFICULTY_WINDOW_SIZE,
    coinbase_payload_script_public_key_max_len: 150,
    max_coinbase_payload_len: 204,

    // Limit the cost of calculating compute/transient/storage masses
    max_tx_inputs: 1000,
    max_tx_outputs: 1000,
    // Transient mass enforces a limit of 125Kb, however script engine max scripts size is 10Kb so there's no point in surpassing that.
    prior_max_signature_script_len: PRIOR_MAX_SIGNATURE_SCRIPT_LEN,
    new_max_signature_script_len: NEW_MAX_SIGNATURE_SCRIPT_LEN,
    // Compute mass enforces a limit of ~45.5Kb, however script engine max scripts size is 10Kb so there's no point in surpassing that.
    // Note that storage mass will kick in and gradually penalize also for lower lengths (generalized KIP-0009, plurality will be high).
    max_script_public_key_len: 10_000,

    mass_per_tx_byte: 1,
    mass_per_script_pub_key_byte: 10,
    mass_per_sig_op: 1000,
    prior_block_mass_limits: BlockMassLimits::with_shared_limit(500_000),
    new_transient_mass_limit: 1_000_000,
    block_lane_limits: BlockLaneLimits { lanes_per_block: DEFAULT_LANES_PER_BLOCK_LIMIT, gas_per_lane: DEFAULT_GAS_PER_LANE_LIMIT },

    storage_mass_parameter: STORAGE_MASS_PARAMETER,
    // Deflationary from genesis — see MAINNET_PARAMS' comment above (same rationale).
    deflationary_phase_daa_score: 0,
    pre_deflationary_phase_base_subsidy: TenBps::pre_deflationary_phase_base_subsidy(),
    skip_proof_of_work: false,
    max_block_level: 250,
    pruning_proof_m: 1000,

    blockrate: BlockrateParams::new::<10>(),

    pre_crescendo_target_time_per_block: 1000,

    // A new chain starts with all upgrades active from block 0 — no history to protect (P2.6).
    crescendo_activation: ForkActivation::always(),
    toccata_activation: ForkActivation::always(),
    pool_activation: ForkActivation::always(),
    note_locks_activation: ForkActivation::new(TESTNET_NOTE_LOCKS_ACTIVATION_DAA_SCORE),
    finality_anchor: FinalityAnchorParams::LAUNCH_UNKEYED,
};

pub const SIMNET_PARAMS: Params = Params {
    dns_seeders: &[],
    net: NetworkId::new(NetworkType::Simnet),
    genesis: SIMNET_GENESIS,
    timestamp_deviation_tolerance: TIMESTAMP_DEVIATION_TOLERANCE,
    max_difficulty_target: MAX_DIFFICULTY_TARGET,
    max_difficulty_target_f64: MAX_DIFFICULTY_TARGET_AS_F64,
    past_median_time_window_size: MEDIAN_TIME_SAMPLED_WINDOW_SIZE as usize,
    difficulty_window_size: DIFFICULTY_SAMPLED_WINDOW_SIZE as usize,
    min_difficulty_window_size: MIN_DIFFICULTY_WINDOW_SIZE,

    // Unlike MAINNET/TESTNET/DEVNET_PARAMS, simnet deliberately keeps a real pre-deflationary
    // flat-subsidy phase (checked at P3.2, not simply left over from before P2.6): simnet is an
    // internal PoW-skipped benchmark/test harness, never a real user-facing network, so P1.4/
    // P1.5's "no pre-deflationary phase, fair launch" commitment doesn't apply to it — and
    // testing/integration/src/daemon_integration_tests.rs's daemon_utxos_propagation_test relies
    // on exactly this: it mines `coinbase_maturity` blocks and asserts the resulting balance as
    // `initial_blocks * SIMNET_PARAMS.pre_deflationary_phase_base_subsidy`, which only holds if
    // simnet actually spends that many blocks in the flat pre-deflationary phase. Confirmed by
    // breaking that test locally when this was set to 0 to "match" the other three networks.
    deflationary_phase_daa_score: TenBps::deflationary_phase_daa_score(),
    pre_deflationary_phase_base_subsidy: TenBps::pre_deflationary_phase_base_subsidy(),
    coinbase_payload_script_public_key_max_len: 150,
    max_coinbase_payload_len: 204,

    max_tx_inputs: 1000,
    max_tx_outputs: 1000,
    prior_max_signature_script_len: NEW_MAX_SIGNATURE_SCRIPT_LEN,
    new_max_signature_script_len: NEW_MAX_SIGNATURE_SCRIPT_LEN,
    max_script_public_key_len: 10_000,

    mass_per_tx_byte: 1,
    mass_per_script_pub_key_byte: 10,
    mass_per_sig_op: 1000,
    // Transient mass is increased for stark proofs
    prior_block_mass_limits: BlockMassLimits::with_shared_limit(500_000),
    new_transient_mass_limit: 1_000_000,
    block_lane_limits: BlockLaneLimits { lanes_per_block: DEFAULT_LANES_PER_BLOCK_LIMIT, gas_per_lane: DEFAULT_GAS_PER_LANE_LIMIT },

    storage_mass_parameter: STORAGE_MASS_PARAMETER,

    skip_proof_of_work: true, // For simnet only, PoW can be simulated by default
    max_block_level: 250,
    pruning_proof_m: PRUNING_PROOF_M,

    // For simnet, we deviate from default 10BPS configuration and allow at least 64 parents in order to support mempool benchmarks out of the box
    blockrate: BlockrateParams::new::<10>().increase_max_block_parents(64),

    pre_crescendo_target_time_per_block: TenBps::target_time_per_block(),

    crescendo_activation: ForkActivation::always(),
    toccata_activation: ForkActivation::always(),
    pool_activation: ForkActivation::always(),
    note_locks_activation: ForkActivation::always(),
    finality_anchor: FinalityAnchorParams::LAUNCH_UNKEYED,
};

pub const DEVNET_PARAMS: Params = Params {
    dns_seeders: &[],
    net: NetworkId::new(NetworkType::Devnet),
    genesis: DEVNET_GENESIS,
    timestamp_deviation_tolerance: TIMESTAMP_DEVIATION_TOLERANCE,
    max_difficulty_target: MAX_DIFFICULTY_TARGET,
    max_difficulty_target_f64: MAX_DIFFICULTY_TARGET_AS_F64,
    past_median_time_window_size: MEDIAN_TIME_SAMPLED_WINDOW_SIZE as usize,
    difficulty_window_size: DIFFICULTY_SAMPLED_WINDOW_SIZE as usize,
    min_difficulty_window_size: MIN_DIFFICULTY_WINDOW_SIZE,
    coinbase_payload_script_public_key_max_len: 150,
    max_coinbase_payload_len: 204,

    max_tx_inputs: 1000,
    max_tx_outputs: 1000,
    prior_max_signature_script_len: NEW_MAX_SIGNATURE_SCRIPT_LEN,
    new_max_signature_script_len: NEW_MAX_SIGNATURE_SCRIPT_LEN,
    max_script_public_key_len: 10_000,

    mass_per_tx_byte: 1,
    mass_per_script_pub_key_byte: 10,
    mass_per_sig_op: 1000,

    // Transient mass is increased for stark proofs
    prior_block_mass_limits: BlockMassLimits::with_shared_limit(500_000),
    new_transient_mass_limit: 1_000_000,
    block_lane_limits: BlockLaneLimits { lanes_per_block: DEFAULT_LANES_PER_BLOCK_LIMIT, gas_per_lane: DEFAULT_GAS_PER_LANE_LIMIT },

    storage_mass_parameter: STORAGE_MASS_PARAMETER,

    deflationary_phase_daa_score: 0,
    pre_deflationary_phase_base_subsidy: TenBps::pre_deflationary_phase_base_subsidy(),
    skip_proof_of_work: false,
    max_block_level: 250,
    pruning_proof_m: 1000,

    blockrate: BlockrateParams::new::<10>(),

    pre_crescendo_target_time_per_block: TenBps::target_time_per_block(),

    crescendo_activation: ForkActivation::always(),
    toccata_activation: ForkActivation::never(),
    pool_activation: ForkActivation::never(),
    note_locks_activation: ForkActivation::never(),
    finality_anchor: FinalityAnchorParams::LAUNCH_UNKEYED,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_params_deserializes_toccata_activation() {
        let override_params: OverrideParams = serde_json::from_str(r#"{"toccata_activation":42}"#).unwrap();

        assert_eq!(override_params.toccata_activation, Some(ForkActivation::new(42)));
    }

    #[test]
    fn override_params_rejects_unknown_top_level_fields() {
        let err = serde_json::from_str::<OverrideParams>(r#"{"unexpected":42}"#).unwrap_err();

        assert!(err.to_string().contains("unknown field `unexpected`"), "{err}");
    }

    #[test]
    fn override_params_rejects_unknown_nested_blockrate_fields() {
        let err = serde_json::from_str::<OverrideParams>(
            r#"{
                "blockrate": {
                    "target_time_per_block": 100,
                    "ghostdag_k": 124,
                    "past_median_time_sample_rate": 10,
                    "difficulty_sample_rate": 2,
                    "max_block_parents": 16,
                    "mergeset_size_limit": 248,
                    "merge_depth": 36000,
                    "finality_depth": 432000,
                    "pruning_depth": 1080000,
                    "coinbase_maturity": 200,
                    "unexpected": 1
                }
            }"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown field `unexpected`"), "{err}");
    }

    #[test]
    fn override_params_rejects_unknown_nested_mass_limit_fields() {
        let err = serde_json::from_str::<OverrideParams>(
            r#"{
                "prior_block_mass_limits": {
                    "storage": 500000,
                    "compute": 500000,
                    "transient": 500000,
                    "unexpected": 1
                }
            }"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown field `unexpected`"), "{err}");
    }
}
