use crate::imports::*;

#[derive(Default, Handler)]
#[help("Select the network to use ('network' alone lists what this build supports)")]
pub struct Network;

impl Network {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        if let Some(network_id) = argv.first() {
            let network_id: NetworkId = network_id.trim().parse::<NetworkId>()?;
            tprintln!(ctx, "Setting network id to: {network_id}");
            ctx.wallet().set_network_id(&network_id)?;
            ctx.wallet().settings().set(WalletSettings::Network, network_id).await?;

            // Keep the open wallet's own remembered network in step.
            if ctx.wallet().is_open()
                && let Some(descriptor) = ctx.wallet().store().descriptor()
                && let Ok(Some(mut meta)) = ctx.wallet().store().client_metadata(&descriptor.filename).await
                && meta.remember
            {
                meta.network = Some(network_id.to_string());
                ctx.wallet().store().set_client_metadata(&descriptor.filename, Some(meta)).await.ok();
            }
        } else {
            match ctx.wallet().network_id() {
                Ok(network_id) => tprintln!(ctx, "Current network: {network_id}"),
                Err(_) => tprintln!(ctx, "No network selected yet"),
            }
            tprintln!(ctx, "Available: {}", NetworkId::supported_list());
            tprintln!(ctx, "(select one with 'network <name>'; bare 'testnet' means the current public testnet)");
        }

        Ok(())
    }
}
