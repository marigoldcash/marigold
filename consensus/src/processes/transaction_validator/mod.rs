pub mod errors;
pub mod tx_validation_in_header_context;
pub mod tx_validation_in_isolation;
pub mod tx_validation_in_utxo_context;
use std::sync::Arc;

use kaspa_txscript::{
    SigCacheKey,
    caches::{Cache, TxScriptCacheCounters},
};

use kaspa_consensus_core::{
    KType,
    config::params::{FinalityAnchorParams, ForkActivation, ForkedParam},
    mass::MassCalculator,
};

#[derive(Clone)]
pub struct TransactionValidator {
    max_tx_inputs: usize,
    max_tx_outputs: usize,
    max_signature_script_len: ForkedParam<usize>,
    max_script_public_key_len: usize,
    coinbase_payload_script_public_key_max_len: u8,
    coinbase_maturity: u64,
    ghostdag_k: KType,
    sig_cache: Cache<SigCacheKey, bool>,
    toccata_activation: ForkActivation,
    mass_per_sig_op: u64,
    /// Finality-anchor params (P6.11) — anchor-subnetwork payloads are fully
    /// verifiable in isolation (parse/shape/signatures/equivocation rule are all
    /// context-free given the pinned trustee keys), so that verification lives here
    /// with the other isolation checks.
    finality_anchor_params: FinalityAnchorParams,

    pub(crate) mass_calculator: MassCalculator,
}

impl TransactionValidator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_tx_inputs: usize,
        max_tx_outputs: usize,
        max_signature_script_len: impl Into<ForkedParam<usize>>,
        max_script_public_key_len: usize,
        coinbase_payload_script_public_key_max_len: u8,
        coinbase_maturity: u64,
        ghostdag_k: KType,
        counters: Arc<TxScriptCacheCounters>,
        mass_calculator: MassCalculator,
        toccata_activation: ForkActivation,
        mass_per_sig_op: u64,
        finality_anchor_params: FinalityAnchorParams,
    ) -> Self {
        Self {
            max_tx_inputs,
            max_tx_outputs,
            max_signature_script_len: max_signature_script_len.into(),
            max_script_public_key_len,
            coinbase_payload_script_public_key_max_len,
            coinbase_maturity,
            ghostdag_k,
            sig_cache: Cache::with_counters(10_000, counters),
            mass_calculator,
            toccata_activation,
            mass_per_sig_op,
            finality_anchor_params,
        }
    }

    pub fn new_for_tests(
        max_tx_inputs: usize,
        max_tx_outputs: usize,
        max_signature_script_len: impl Into<ForkedParam<usize>>,
        max_script_public_key_len: usize,
        coinbase_payload_script_public_key_max_len: u8,
        coinbase_maturity: u64,
        ghostdag_k: KType,
        counters: Arc<TxScriptCacheCounters>,
    ) -> Self {
        Self {
            max_tx_inputs,
            max_tx_outputs,
            max_signature_script_len: max_signature_script_len.into(),
            max_script_public_key_len,
            coinbase_payload_script_public_key_max_len,
            coinbase_maturity,
            ghostdag_k,
            sig_cache: Cache::with_counters(10_000, counters),
            mass_calculator: MassCalculator::new(0, 0, 0),
            toccata_activation: ForkActivation::never(),
            mass_per_sig_op: 0,
            finality_anchor_params: FinalityAnchorParams::LAUNCH_UNKEYED,
        }
    }
}
