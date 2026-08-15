use std::{ops::Deref, sync::Arc};

use crate::model::stores::{
    pruning::PruningStoreReader, utxo_multisets::UtxoMultisetsStoreReader, virtual_state::VirtualStateStoreReader,
};
use kaspa_consensus_core::{
    block::BlockTemplate, blockhash::ORIGIN, coinbase::MinerData, errors::block::RuleError, notepool::PoolViewComposition,
    tx::Transaction, utxo::utxo_view::UtxoViewComposition,
};
use kaspa_hashes::Hash;

use super::VirtualStateProcessor;

/// Wrapper for virtual processor with util methods for building a block with any parent context
pub struct TestBlockBuilder {
    processor: Arc<VirtualStateProcessor>,
}

impl Deref for TestBlockBuilder {
    type Target = VirtualStateProcessor;

    fn deref(&self) -> &Self::Target {
        &self.processor
    }
}

impl TestBlockBuilder {
    pub fn new(processor: Arc<VirtualStateProcessor>) -> Self {
        Self { processor }
    }

    /// Test-only helper method for building a block template with specific parents
    pub(crate) fn build_block_template_with_parents(
        &self,
        parents: Vec<Hash>,
        miner_data: MinerData,
        txs: Vec<Transaction>,
    ) -> Result<BlockTemplate, RuleError> {
        //
        // In the context of this method "pov virtual" is the virtual block which has `parents` as tips and not the actual virtual
        //
        let pruning_point = self.pruning_point_store.read().pruning_point().unwrap();
        let virtual_read = self.virtual_stores.read();
        let virtual_state = virtual_read.state.get().unwrap();
        let finality_point = ORIGIN; // No real finality point since we are not actually building virtual here
        let sink = virtual_state.ghostdag_data.selected_parent;
        let mut accumulated_diff = virtual_state.utxo_diff.clone().to_reversed();
        let mut accumulated_pool_diff = virtual_read.virtual_pool_diff().to_reversed();
        // Search for the sink block from the PoV of this virtual
        let (pov_sink, virtual_parent_candidates) = self.sink_search_algorithm(
            &virtual_read,
            &mut accumulated_diff,
            &mut accumulated_pool_diff,
            sink,
            parents,
            finality_point,
            pruning_point,
        );
        let (pov_virtual_parents, pov_virtual_ghostdag_data) =
            self.pick_virtual_parents(pov_sink, virtual_parent_candidates, pruning_point);
        let pov_sink_multiset = self.utxo_multisets_store.get(pov_sink).unwrap();
        let (pov_virtual_state, _pov_virtual_pool_diff) = self.calculate_virtual_state(
            &virtual_read,
            pov_virtual_parents,
            pov_virtual_ghostdag_data,
            pov_sink_multiset,
            &mut accumulated_diff,
            &mut accumulated_pool_diff,
        )?;
        // Computed against `accumulated_pool_diff` BEFORE it's moved into the composed view
        // below — this pov-hypothetical virtual is generally NOT the real committed virtual
        // (that's the entire point of this method), so it must NOT reuse
        // `virtual_stores.pool_smt`'s fast incremental root (which only ever tracks the real
        // one); the same full-rebuild `recompute_pool_commitment` verification will use is the
        // only mechanism guaranteed correct for an arbitrary pov (POOL-SPEC.md P5.1, FORK-PLAN P6.5).
        let pool_commitment = self.recompute_pool_commitment(
            &virtual_read.pool_state,
            &accumulated_pool_diff,
            &kaspa_consensus_core::notepool::PoolDiff::default(),
        );
        let pov_virtual_utxo_view = (&virtual_read.utxo_set).compose(accumulated_diff);
        let pov_virtual_pool_view = PoolViewComposition::compose(&virtual_read.pool_state, accumulated_pool_diff);
        self.validate_block_template_transactions(&txs, &pov_virtual_state, &pov_virtual_utxo_view, &pov_virtual_pool_view)?;
        drop(virtual_read);
        self.build_block_template_from_virtual_state(pov_virtual_state, miner_data, txs, vec![], pool_commitment)
    }
}
