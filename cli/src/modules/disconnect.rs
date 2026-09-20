use crate::imports::*;

#[derive(Default, Handler)]
#[help("Leave the network: stop syncing here, or let go of the public computer")]
pub struct Disconnect;

impl Disconnect {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let _switching = ctx.switching();
        // Two things this can mean, each confirmed on its own (founder,
        // 2026-09-16): stopping the sync running on this machine, and letting
        // go of a public computer. Both can be true at once while the sync is
        // catching up behind a public connection.
        #[cfg(feature = "embedded-node")]
        let own = ctx.embedded_node_running();
        #[cfg(not(feature = "embedded-node"))]
        let own = false;
        #[cfg(feature = "embedded-node")]
        let on_public = ctx.wallet().is_connected() && !ctx.embedded_node_in_use();
        #[cfg(not(feature = "embedded-node"))]
        let on_public = ctx.wallet().is_connected();

        if !own && !on_public {
            tprintln!(ctx, "Not connected to the network.");
            return Ok(());
        }
        if on_public {
            let answer = ctx.term().ask(false, "Disconnect from the public computer? [y/N]: ").await?.trim().to_lowercase();
            if answer.starts_with('y') {
                if let Some(wrpc_client) = ctx.wallet().try_wrpc_client().as_ref() {
                    wrpc_client.disconnect().await?;
                }
                ctx.note_remote_miner(None);
                tprintln!(ctx, "Disconnected from the public computer.");
            } else {
                tprintln!(ctx, "Still connected.");
            }
        }
        #[cfg(feature = "embedded-node")]
        if own {
            tprintln!(ctx, "");
            if ctx.embedded_node_syncing() {
                tprintln!(ctx, "{}", style("The sync has not finished its first run. Stopping it discards that progress;").yellow());
                tprintln!(ctx, "{}", style("the next start begins again from scratch.").yellow());
            }
            let answer = ctx.term().ask(false, "Stop syncing with the network? [y/N]: ").await?.trim().to_lowercase();
            if answer.starts_with('y') {
                ctx.stop_embedded_node().await?;
            } else {
                tprintln!(ctx, "Still syncing. 'connect status' shows progress.");
            }
        }
        Ok(())
    }
}
