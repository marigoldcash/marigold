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
        let headers = dag.as_ref().map(|d| d.header_count).unwrap_or(0);

        if synced {
            tprintln!(ctx, "Status: {}", style("caught up with the network").green());
        } else if headers == 0 {
            // A new node spends its first minutes verifying the proof of work
            // behind the chain before it downloads any of it. Nothing moves in
            // that phase — no blocks, no headers, no growth on disk — so
            // reporting "still catching up" beside three zeroes made a healthy
            // node look wedged (founder report, 2026-09-07). Say what it is
            // doing instead.
            tprintln!(ctx, "Status: {}", style("verifying the chain's history before downloading it").yellow());
            tprintln!(ctx, "{}", style("This is the first phase and shows no progress by design — no blocks, no").dim());
            tprintln!(ctx, "{}", style("headers, no growth on disk. It takes a few minutes. 'node logs' shows it.").dim());
        } else {
            tprintln!(ctx, "Status: {}", style("downloading the chain — balances are incomplete until it finishes").yellow());
        }

        // Peers first: a node with none will sit at zero blocks for ever, and
        // that is a different problem from a node that is downloading slowly.
        // Without this the two look identical from outside.
        match ctx.wallet().rpc_api().get_connected_peer_info().await {
            Ok(info) => {
                let n = info.peer_info.len();
                if n == 0 {
                    tprintln!(ctx, "Peers:  {}", style("none — it cannot sync without them").red());
                    tprintln!(ctx, "{}", style("Check outbound connections to port 26211 are not blocked.").dim());
                    tprintln!(ctx, "{}", style("'node logs' shows what it is doing.").dim());
                } else {
                    tprintln!(ctx, "Peers:  {n}");
                }
            }
            Err(err) => tprintln!(ctx, "{}", style(format!("(could not read peers: {err})")).dim()),
        }

        match dag {
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
            None => tprintln!(ctx, "{}", style("(could not read the node's chain state)").dim()),
        }
        tprintln!(ctx, "");
    }
}
