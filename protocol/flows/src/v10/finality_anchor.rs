//! Finality-anchor gossip (POOL-SPEC.md P5.8, FORK-PLAN P6.12): the second, mining-
//! independent distribution channel for anchors — "gossiped directly over P2P, so an
//! anchor's availability doesn't depend on it having been mined into a block yet".
//!
//! One flow per peer, handling both directions:
//! - On start it immediately requests the peer's best anchor (the `ReceiveAddressesFlow`
//!   on-connect pattern) — this is also what makes IBD anchor-aware: a fresh node asks
//!   *every* connected peer, not just its sync peer, before committing to any chain.
//! - Incoming `RequestFinalityAnchor` → reply with our best full anchor, if any.
//! - Incoming `FinalityAnchor` → offer to consensus (context-free verification against
//!   the pinned trustee keys + deny-list happens there); anything that improved local
//!   state (ratcheted or held pending) is re-relayed to all other peers, so a single
//!   honest path suffices to deliver the newest anchor network-wide.
//!
//! Rate/spam bounds: verification rejects everything not genuinely trustee-signed, and
//! only *improving* anchors propagate — a peer flooding stale or invalid anchors burns
//! its own connection's budget and nothing else. When the mechanism is unkeyed
//! (`finality_anchor.trustees = None`, every network until the P9.1 ceremony) incoming
//! anchors are ignored and requests are answered with silence, both harmless.

use crate::{flow_context::FlowContext, flow_trait::Flow};
use kaspa_core::debug;
use kaspa_p2p_lib::{
    IncomingRoute, Router,
    common::ProtocolError,
    make_message,
    pb::{RequestFinalityAnchorMessage, kaspad_message::Payload},
};
use std::sync::Arc;

pub struct FinalityAnchorFlow {
    ctx: FlowContext,
    router: Arc<Router>,
    incoming_route: IncomingRoute,
}

#[async_trait::async_trait]
impl Flow for FinalityAnchorFlow {
    fn router(&self) -> Option<Arc<Router>> {
        Some(self.router.clone())
    }

    async fn start(&mut self) -> Result<(), ProtocolError> {
        self.start_impl().await
    }
}

impl FinalityAnchorFlow {
    pub fn new(ctx: FlowContext, router: Arc<Router>, incoming_route: IncomingRoute) -> Self {
        Self { ctx, router, incoming_route }
    }

    async fn start_impl(&mut self) -> Result<(), ProtocolError> {
        // On connect: ask this peer for its best anchor. No timeout on the reply —
        // a peer holding no anchor legitimately stays silent, and the loop below
        // serves both message kinds for the connection's lifetime either way.
        self.router.enqueue(make_message!(Payload::RequestFinalityAnchor, RequestFinalityAnchorMessage {})).await?;

        while let Some(msg) = self.incoming_route.recv().await {
            match msg.payload {
                Some(Payload::RequestFinalityAnchor(_)) => {
                    let session = self.ctx.consensus().unguarded_session();
                    if let Some(anchor) = session.async_get_latest_full_finality_anchor().await {
                        self.router.enqueue(make_message!(Payload::FinalityAnchor, (&anchor).into())).await?;
                    }
                }
                Some(Payload::FinalityAnchor(anchor_msg)) => {
                    let anchor: kaspa_consensus_core::finality_anchor::FinalityAnchor = anchor_msg.try_into()?;
                    let session = self.ctx.consensus().unguarded_session();
                    let outcome = session.async_apply_external_finality_anchor(anchor.clone()).await;
                    drop(session);
                    match outcome {
                        kaspa_consensus_core::finality_anchor::ExternalAnchorOutcome::Ratcheted
                        | kaspa_consensus_core::finality_anchor::ExternalAnchorOutcome::Pending => {
                            debug!(
                                "[FINALITY ANCHOR] Relaying improved anchor (block {}, DAA score {}) received from {}",
                                anchor.anchored_block, anchor.anchored_daa_score, self.router
                            );
                            self.ctx
                                .hub()
                                .broadcast(make_message!(Payload::FinalityAnchor, (&anchor).into()), Some(self.router.key()))
                                .await;
                        }
                        kaspa_consensus_core::finality_anchor::ExternalAnchorOutcome::Ignored => {}
                    }
                }
                _ => {
                    return Err(ProtocolError::UnexpectedMessage(
                        stringify!(FinalityAnchorFlow),
                        msg.payload.as_ref().map(|v| v.into()),
                    ));
                }
            }
        }
        Err(ProtocolError::ConnectionClosed)
    }
}
