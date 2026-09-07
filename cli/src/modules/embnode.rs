use crate::imports::*;

#[derive(Default, Handler)]
#[help("Run a node inside this wallet, or see which node you are using")]
pub struct Node;

impl Node {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        match argv.first().map(|s| s.as_str()) {
            Some("start") => ctx.start_embedded_node().await,
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
            Some(other) => {
                tprintln!(ctx, "usage: 'node start' | 'node stop' | 'node status' | 'node logs [off]'  (got '{other}')");
                Ok(())
            }
        }
    }

    /// Which node the wallet is talking to, and whether it has caught up.
    ///
    /// "Caught up" matters more than it looks: a node still syncing answers
    /// every question truthfully about a chain it has not finished reading, so
    /// balances and note checks are simply incomplete rather than wrong. Saying
    /// so is the difference between a user waiting and a user thinking their
    /// money is missing.
    async fn status(&self, ctx: &Arc<KaspaCli>) {
        let mine = ctx.embedded_node_running();
        let connected = ctx.wallet().is_connected();

        tprintln!(ctx, "");
        match (mine, connected) {
            (true, _) => {
                tprintln!(ctx, "Using: {}", style("your own node, running inside this wallet").bold());
                tprintln!(ctx, "{}", style("Nobody else sees your address or which notes you hold.").dim());
            }
            (false, true) => {
                let wallet = ctx.wallet();
                let url = wallet.utxo_processor().rpc_url().unwrap_or_else(|| "a node".into());
                tprintln!(ctx, "Using: {}", style(url).bold());
                tprintln!(ctx, "{}", style("Whoever runs it can see the address you connect from and which").dim());
                tprintln!(ctx, "{}", style("notes your wallet asks about — 'node start' runs your own instead.").dim());
            }
            (false, false) => {
                tprintln!(ctx, "Not connected to any node.");
                tprintln!(ctx, "");
                tprintln!(ctx, "  'node start'      run one here. Nobody sees your notes or your address.");
                tprintln!(ctx, "                    Takes a while to catch up the first time, and several");
                tprintln!(ctx, "                    gigabytes of disk.");
                tprintln!(ctx, "  'connect public'  use a node the Marigold project runs — ready at once,");
                tprintln!(ctx, "                    but they see what your wallet asks about.");
                tprintln!(ctx, "");
                return;
            }
        }

        if !connected {
            tprintln!(ctx, "Status: {}", style("starting up — not answering yet").yellow());
            tprintln!(ctx, "");
            return;
        }

        // is_synced is the node's own verdict on whether it has reached the tip.
        let wallet = ctx.wallet();
        let synced = wallet.utxo_processor().is_synced();
        let dag = ctx.wallet().rpc_api().get_block_dag_info().await.ok();

        if synced {
            tprintln!(ctx, "Status: {}", style("caught up with the network").green());
        } else {
            use crate::log_sink::SyncProgress;
            match crate::log_sink::sync_progress() {
                // Levels count DOWN from 250, so progress is how far it has come.
                Some(SyncProgress::VerifyingProof { level }) => {
                    let done = 250u32.saturating_sub(level);
                    tprintln!(ctx, "Status: {}", style(format!("verifying the chain's history — level {level}, {done} of 250 done")).yellow());
                    tprintln!(ctx, "{}", style("Nothing lands on disk during this phase; it is checking proof of work.").dim());
                }
                // Bounded by finality depth, so there is a real ceiling.
                Some(SyncProgress::ChainSegment { headers }) => {
                    let params = kaspa_consensus_core::config::params::Params::from(
                        ctx.wallet().network_id().unwrap_or(NetworkId::with_suffix(NetworkType::Testnet, 10)),
                    );
                    let max = params.finality_depth() + 2 * params.ghostdag_k as u64 + 1;
                    let pct = (headers as f64 / max as f64 * 100.0) as u32;
                    tprintln!(
                        ctx,
                        "Status: {}",
                        style(format!(
                            "downloading the chain — {} headers, at most {} ({pct}% of the maximum)",
                            headers.separated_string(),
                            max.separated_string()
                        ))
                        .yellow()
                    );
                }
                Some(SyncProgress::Headers { headers, percent, block_time }) => {
                    tprintln!(
                        ctx,
                        "Status: {}",
                        style(format!("checking the chain's headers — {} done, {percent}%", headers.separated_string())).yellow()
                    );
                    if let Some(t) = block_time {
                        tprintln!(ctx, "{}", style(format!("reached blocks from {t}")).dim());
                    }
                }
                Some(SyncProgress::Blocks { blocks, percent }) => {
                    tprintln!(
                        ctx,
                        "Status: {}",
                        style(format!("downloading blocks — {} done, {percent}%", blocks.separated_string())).yellow()
                    );
                }
                None => {
                    tprintln!(ctx, "Status: {}", style("starting its first sync").yellow());
                }
            }
            tprintln!(
                ctx,
                "{}",
                style("A first sync takes anywhere from half an hour to a few hours. Leaving the").dim()
            );
            tprintln!(ctx, "{}", style("wallet before it finishes discards it — after that, restarts are free.").dim());
        }

        // Only once the first sync has committed: until then this reads the
        // ACTIVE consensus while all the work is in staging, so it says zero.
        match dag.filter(|info| info.block_count > 0) {
            Some(info) => {
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
            None => {}
        }
        tprintln!(ctx, "");
    }
}
