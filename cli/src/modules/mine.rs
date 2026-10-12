use crate::imports::*;

#[derive(Default, Handler)]
#[help("Mine with spare CPU, once the network is synced on this machine")]
pub struct Mine;

impl Mine {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        match argv.first().map(|s| s.as_str()) {
            Some("start") => {
                // 'mine start [<percent>] [--pool <host:port>]', in either order.
                let mut percent = None;
                let mut pool = None;
                let mut rest = argv.iter().skip(1);
                while let Some(arg) = rest.next() {
                    match arg.as_str() {
                        "--pool" => pool = rest.next().cloned(),
                        other if other.starts_with("--pool=") => pool = Some(other["--pool=".len()..].to_string()),
                        other => percent = Some(other.to_string()),
                    }
                }
                let started = match pool {
                    Some(pool) => ctx.start_pool_mining(percent, pool).await,
                    None => ctx.start_mining(percent).await,
                };
                if started.is_ok() {
                    ctx.remember_mined().await;
                }
                started
            }
            Some("stop") => ctx.stop_mining().await,
            Some("status") => {
                ctx.mining_status().await;
                Ok(())
            }
            // Bare 'mine' asks what this does, so answer that rather than
            // silently running one of its subcommands.
            None => {
                ctx.term().help(
                    &[
                        ("start [<percent>]", "Mine with spare CPU (asks how much of the machine; default 50%)"),
                        (
                            "start --pool <host:port> [<percent>]",
                            "Mine for a pool instead: shares go there, it pays this wallet's address by its rules",
                        ),
                        ("stop", "Stop mining"),
                        ("status", "Speed, how much of the machine, and blocks found"),
                    ],
                    None,
                )?;
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("To mine without a wallet open — as a service that stays running — start the miner").dim());
                tprintln!(
                    ctx,
                    "{}",
                    style("program instead:  marigold-cli mine-to <address> 50   (the number is the share of the machine)").dim()
                );
                tprintln!(ctx, "");
                Ok(())
            }
            Some(other) => {
                tprintln!(ctx, "usage: 'mine start [percent] [--pool host:port]' | 'mine stop' | 'mine status'  (got '{other}')");
                Ok(())
            }
        }
    }
}
