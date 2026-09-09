use crate::imports::*;

#[derive(Default, Handler)]
#[help("Run a node inside this wallet, or see which node you are using")]
pub struct Node;

impl Node {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        match argv.first().map(|s| s.as_str()) {
            // Handover, not a straight bind: if the wallet is already talking
            // to a node that can answer, there is no reason to swap it for one
            // that cannot yet. You still end up on your own node.
            Some("start") => ctx.start_node_with_handover().await,
            Some("stop") => ctx.stop_embedded_node().await,
            Some("logs") => {
                // The node's own logging is clamped to warnings so it does not
                // scroll a wallet's terminal once a second. That also made a
                // node that would not sync completely undiagnosable, so it can
                // be turned back on.
                let on = !matches!(argv.get(1).map(|s| s.as_str()), Some("off"));
                crate::embedded::set_logs_wanted(on);
                tprintln!(ctx, "Node logs are {}.", if on { "on — 'node logs off' to silence them" } else { "off" });
                Ok(())
            }
            Some("status") | None => {
                self.status(&ctx).await;
                Ok(())
            }
            Some("details") => {
                self.details(&ctx).await;
                Ok(())
            }
            Some(other) => {
                tprintln!(ctx, "usage: 'node start' | 'node stop' | 'node status' | 'node details' | 'node logs [off]'  (got '{other}')");
                Ok(())
            }
        }
    }

    /// How far our own node has got, as a step out of three and a percentage.
    ///
    /// Best effort, and read out of the node's own log records rather than its
    /// RPC: during the first phase it reports zero blocks and zero headers
    /// however you ask it, because the work is going into a staging consensus
    /// that is not committed until the very end. The log is the only place the
    /// progress exists at all.
    ///
    /// Three steps because there are three waits, not because the node has
    /// three phases — it checks the chain's proof, fetches the headers in two
    /// internal stages, then fetches the blocks. Someone watching a bar wants
    /// to know how many more times it fills.
    fn sync_step(ctx: &Arc<KaspaCli>) -> String {
        use crate::log_sink::SyncProgress;
        let (step, percent) = match crate::log_sink::sync_progress() {
            // Levels count DOWN from 250, so progress is how far it has come.
            Some(SyncProgress::VerifyingProof { level }) => (1, 250u32.saturating_sub(level) * 100 / 250),
            Some(SyncProgress::ChainSegment { headers }) => {
                let params = kaspa_consensus_core::config::params::Params::from(
                    ctx.wallet().network_id().unwrap_or(NetworkId::with_suffix(NetworkType::Testnet, 10)),
                );
                let max = params.finality_depth() + 2 * params.ghostdag_k as u64 + 1;
                (2, ((headers as f64 / max as f64) * 100.0) as u32)
            }
            Some(SyncProgress::Headers { percent, .. }) => (2, percent),
            Some(SyncProgress::Blocks { percent, .. }) => (3, percent),
            None => (1, 0),
        };
        format!("Step {step} of 3, {}% done", percent.min(100))
    }

    /// Which node the wallet is talking to, and — if it is not yet your own —
    /// how far off that is.
    ///
    /// Two facts, in this order: which node is answering, and what that means
    /// for who can see your notes. Everything else a node knows about itself
    /// is behind 'node details', because block counts and DAA scores change
    /// nobody's next move and reading them is a skill this wallet should not
    /// require.
    async fn status(&self, ctx: &Arc<KaspaCli>) {
        let mine = ctx.embedded_node_in_use();
        let pending = ctx.embedded_node_pending();
        let connected = ctx.wallet().is_connected();

        tprintln!(ctx, "");
        if mine {
            tprintln!(ctx, "Using: {}", style("your own local node, running inside this wallet, all in sync with the network.").bold());
            tprintln!(ctx, "Your wallet notes are not announced to a public node.");
            tprintln!(ctx, "");
            return;
        }
        if pending {
            tprintln!(ctx, "Using: {} Syncing local node, {}.", style("public node.").bold(), Self::sync_step(ctx));
            tprintln!(ctx, "Your wallet notes are still being announced through a public node until the");
            tprintln!(ctx, "sync is complete.");
            tprintln!(ctx, "");
            return;
        }
        if connected {
            tprintln!(ctx, "Using: {}", style("a public node.").bold());
            tprintln!(ctx, "Whoever runs it can see which notes your wallet asks about.");
            tprintln!(ctx, "Type 'node start' to run your own instead.");
            tprintln!(ctx, "");
            return;
        }
        tprintln!(ctx, "Not connected to any node.");
        tprintln!(ctx, "");
        tprintln!(ctx, "Type 'connect' to get started.");
        tprintln!(ctx, "");
    }

    /// The node's own figures. Kept out of 'node status' on purpose — see
    /// there — but a node that will not sync cannot be diagnosed without them.
    async fn details(&self, ctx: &Arc<KaspaCli>) {
        tprintln!(ctx, "");
        if !ctx.wallet().is_connected() {
            tprintln!(ctx, "Not connected to any node.");
            tprintln!(ctx, "");
            return;
        }
        let synced = ctx.wallet().utxo_processor().is_synced();
        tprintln!(ctx, "Node reports: {}", if synced { "caught up" } else { "not caught up" });
        if ctx.embedded_node_running() {
            tprintln!(ctx, "Your node:    {}", Self::sync_step(ctx));
            tprintln!(ctx, "Adopted:      {}", if ctx.embedded_node_in_use() { "yes" } else { "not yet" });
            // The raw figures behind the step-of-three. 'node status' does not
            // carry them because they answer nothing anyone asks; here they are
            // the whole point, since a node that will not sync is diagnosed by
            // watching which of these stops moving.
            use crate::log_sink::SyncProgress;
            match crate::log_sink::sync_progress() {
                Some(SyncProgress::VerifyingProof { level }) => {
                    tprintln!(ctx, "{}", style(format!("              proof level {level} of 250, counting down")).dim())
                }
                Some(SyncProgress::ChainSegment { headers }) => {
                    tprintln!(ctx, "{}", style(format!("              {} chain headers", headers.separated_string())).dim())
                }
                Some(SyncProgress::Headers { headers, block_time, .. }) => {
                    let at = block_time.map(|t| format!(", reached blocks from {t}")).unwrap_or_default();
                    tprintln!(ctx, "{}", style(format!("              {} headers{at}", headers.separated_string())).dim())
                }
                Some(SyncProgress::Blocks { blocks, .. }) => {
                    tprintln!(ctx, "{}", style(format!("              {} blocks", blocks.separated_string())).dim())
                }
                None => {}
            }
        }
        match ctx.wallet().rpc_api().get_block_dag_info().await {
            Ok(info) => {
                tprintln!(
                    ctx,
                    "{}",
                    style(format!(
                        "{} blocks, {} headers, virtual DAA score {}",
                        info.block_count.separated_string(),
                        info.header_count.separated_string(),
                        info.virtual_daa_score.separated_string()
                    ))
                    .dim()
                );
            }
            Err(err) => tprintln!(ctx, "{}", style(format!("(could not read the node's figures: {err})")).dim()),
        }
        tprintln!(ctx, "");
    }
}
