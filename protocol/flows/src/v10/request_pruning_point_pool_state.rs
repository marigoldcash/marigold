use crate::{flow_context::FlowContext, flow_trait::Flow, ibd::IBD_BATCH_SIZE};
use itertools::Itertools;
use kaspa_consensus_core::errors::consensus::ConsensusError;
use kaspa_core::debug;
use kaspa_hashes::Hash;
use kaspa_p2p_lib::{
    IncomingRoute, Router,
    common::ProtocolError,
    dequeue, make_message,
    pb::{
        DonePruningPointPoolStateChunksMessage, PruningPointPoolStateChunkMessage, UnexpectedPruningPointMessage,
        kaspad_message::Payload,
    },
};
use std::sync::Arc;

/// Serves the note-pool state positioned at the pruning point to IBD peers (PLAN
/// P6.8) — a structural clone of v7's `RequestPruningPointUtxoSetFlow`, chunked in
/// ascending serial order and terminated by an explicit Done sentinel. The client
/// verifies the rebuilt pool SMT root against the pruning point header's own
/// `pool_commitment`, so no per-chunk proof material is served.
pub struct RequestPruningPointPoolStateFlow {
    ctx: FlowContext,
    router: Arc<Router>,
    incoming_route: IncomingRoute,
}

#[async_trait::async_trait]
impl Flow for RequestPruningPointPoolStateFlow {
    fn router(&self) -> Option<Arc<Router>> {
        Some(self.router.clone())
    }

    async fn start(&mut self) -> Result<(), ProtocolError> {
        self.start_impl().await
    }
}

impl RequestPruningPointPoolStateFlow {
    pub fn new(ctx: FlowContext, router: Arc<Router>, incoming_route: IncomingRoute) -> Self {
        Self { ctx, router, incoming_route }
    }

    async fn start_impl(&mut self) -> Result<(), ProtocolError> {
        loop {
            let expected_pp = dequeue!(self.incoming_route, Payload::RequestPruningPointPoolState)?.try_into()?;
            self.handle_request(expected_pp).await?
        }
    }

    async fn handle_request(&mut self, expected_pp: Hash) -> Result<(), ProtocolError> {
        const CHUNK_SIZE: usize = 1000;
        let mut from_sn = None;
        let mut chunks_sent = 0;

        let consensus = self.ctx.consensus();
        let mut session = consensus.session().await;

        loop {
            // We avoid keeping the consensus session across the limitless dequeue call below
            let pool_entries =
                match session.async_get_pruning_point_pool_entries(expected_pp, from_sn, CHUNK_SIZE, chunks_sent != 0).await {
                    Err(ConsensusError::UnexpectedPruningPoint) => return self.send_unexpected_pruning_point_message().await,
                    res => res,
                }?;
            debug!("Retrieved {} pool notes for pruning point {}", pool_entries.len(), expected_pp);

            // Send the chunk
            self.router
                .enqueue(make_message!(
                    Payload::PruningPointPoolStateChunk,
                    PruningPointPoolStateChunkMessage { entries: pool_entries.iter().map(|entry| entry.into()).collect_vec() }
                ))
                .await?;

            chunks_sent += 1;
            if chunks_sent % IBD_BATCH_SIZE == 0 {
                drop(session); // Avoid holding the session through dequeue calls
                dequeue!(self.incoming_route, Payload::RequestNextPruningPointPoolStateChunk)?;
                session = consensus.session().await;
            }

            // This indicates that there are no more entries to query
            if pool_entries.len() < CHUNK_SIZE {
                return self.send_done_message(expected_pp).await;
            }

            // Mark the beginning of the next chunk
            from_sn = Some(pool_entries.last().expect("not empty by prev condition").0);
        }
    }

    async fn send_unexpected_pruning_point_message(&mut self) -> Result<(), ProtocolError> {
        self.router.enqueue(make_message!(Payload::UnexpectedPruningPoint, UnexpectedPruningPointMessage {})).await?;
        Ok(())
    }

    async fn send_done_message(&mut self, expected_pp: Hash) -> Result<(), ProtocolError> {
        debug!("Finished sending pool notes for pruning point {}", expected_pp);
        self.router
            .enqueue(make_message!(Payload::DonePruningPointPoolStateChunks, DonePruningPointPoolStateChunksMessage {}))
            .await?;
        Ok(())
    }
}
