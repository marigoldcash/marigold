use crate::imports::*;
use kaspa_wrpc_client::parse::parse_host;

#[derive(Default, Handler)]
#[help("Set the node this wallet connects to ('server public' to use a Marigold node instead)")]
pub struct Server;

impl Server {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        match argv.first().map(|s| s.as_str()) {
            // There was no way to un-set this. Once a node was remembered,
            // plain `connect` used it for ever — so someone who set it to a
            // local node and later shut that node down had no way back to a
            // public one short of editing settings by hand.
            Some("public") | Some("none") | Some("forget") | Some("clear") => {
                ctx.wallet().settings().set(WalletSettings::Server, "").await?;
                tprintln!(ctx, "Forgotten. 'connect' will use a node the Marigold project runs.");
                tprintln!(ctx, "{}", style("('server <host>' to go back to your own.)").dim());
            }
            Some(url) => {
                let Ok(_) = parse_host(url) else {
                    tprintln!(ctx, "Invalid host: {url}");
                    return Ok(());
                };
                ctx.wallet().settings().set(WalletSettings::Server, url).await?;
                tprintln!(ctx, "Setting RPC server to: {url}");
            }
            None => {
                let server = ctx.wallet().settings().get::<String>(WalletSettings::Server).unwrap_or_default();
                if server.is_empty() {
                    tprintln!(ctx, "No node set — 'connect' uses a node the Marigold project runs.");
                    tprintln!(ctx, "{}", style("('server <host>' to use your own, e.g. 'server 127.0.0.1:27210')").dim());
                } else {
                    tprintln!(ctx, "'connect' uses: {server}");
                    tprintln!(ctx, "{}", style("('server public' to forget it and use a Marigold node instead)").dim());
                }
            }
        }

        Ok(())
    }
}
