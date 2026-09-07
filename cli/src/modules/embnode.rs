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
            Some("status") | None => {
                self.status(&ctx).await;
                Ok(())
            }
            Some(other) => {
                tprintln!(ctx, "usage: 'node start' | 'node stop' | 'node status'  (got '{other}')");
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
        if wallet.utxo_processor().is_synced() {
            tprintln!(ctx, "Status: {}", style("caught up with the network").green());
        } else {
            tprintln!(ctx, "Status: {}", style("still catching up — balances are incomplete until it finishes").yellow());
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
            Err(err) => tprintln!(ctx, "{}", style(format!("(could not read the node's chain state: {err})")).dim()),
        }
        tprintln!(ctx, "");
    }
}
