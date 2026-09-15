use crate::imports::*;

#[derive(Default, Handler)]
#[help("Show the note again — version, network, and what this is")]
pub struct About;

impl About {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        // The same note the wallet opens with, redrawn. It already carries
        // the version and the network, which is what anybody asking "what am
        // I running" wants — and after a few hundred lines of output the one
        // at startup has long since scrolled away.
        //
        // The network comes from the live wallet where there is one, because
        // `network` can change it mid-session and a note still naming the
        // network from startup would be quietly wrong.
        let network = ctx
            .wallet()
            .network_id()
            .ok()
            .map(|id| id.to_string())
            .or_else(|| ctx.wallet().settings().get::<String>(WalletSettings::Network));

        tprintln!(ctx, "");
        let has_wallet = ctx.store().wallet_list().await.map(|wallets| !wallets.is_empty()).ok();
        crate::splash::show(&ctx, env!("CARGO_PKG_VERSION"), network.as_deref(), has_wallet);
        tprintln!(ctx, "");

        Ok(())
    }
}
