use crate::imports::*;

#[derive(Default, Handler)]
#[help("Exit the application ('quit' and Ctrl+D work too)")]
pub struct Exit;

impl Exit {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> cli::Result<()> {
        let term = ctx.term();

        // A node's first sync is all-or-nothing: everything it downloads goes
        // into a staging database that is deleted on the next start, because a
        // partial one is not something a node may trust. Leaving mid-sync
        // therefore throws away every minute of it, and the founder lost an
        // hour that way before anything said so (2026-09-07).
        #[cfg(feature = "embedded-node")]
        if let Ok(cli) = ctx.clone().downcast_arc::<KaspaCli>() {
            if cli.embedded_node_syncing() {
                tprintln!(cli, "");
                tprintln!(cli, "{}", style("Your node has not finished its first sync.").yellow());
                tprintln!(cli, "{}", style("Leaving now discards all of it — the next start begins again from scratch.").yellow());
                tprintln!(cli, "{}", style("(Once it has finished the first time, stopping and starting is free.)").dim());
                tprintln!(cli, "");
                let answer = term.ask(false, "Leave anyway, losing the progress? [y/N]: ").await?.trim().to_lowercase();
                if !answer.starts_with('y') {
                    tprintln!(cli, "Staying. 'node status' shows how far along it is.");
                    return Ok(());
                }
            }
        }

        term.writeln("bye!");
        // Through the CLI's own shutdown, which raises the flag every
        // background loop checks — housekeeping, the minute tick, the node
        // handover watch — and stops any daemon it started. Leaving through
        // the terminal alone left those loops running, redrawing the prompt
        // over "bye!" while the wallet was still shutting down.
        #[cfg(not(target_arch = "wasm32"))]
        match ctx.clone().downcast_arc::<KaspaCli>() {
            Ok(cli) => {
                if let Err(err) = cli.shutdown().await {
                    term.writeln(format!("{err}"));
                    term.exit().await;
                }
            }
            Err(_) => term.exit().await,
        }
        #[cfg(target_arch = "wasm32")]
        workflow_dom::utils::window().location().reload().ok();

        Ok(())
    }
}
