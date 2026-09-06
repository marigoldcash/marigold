use crate::imports::*;

#[derive(Default, Handler)]
#[help("Connect to a Marigold network")]
pub struct Connect;

impl Connect {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        if let Some(wrpc_client) = ctx.wallet().try_wrpc_client().as_ref() {
            let network_id = ctx.wallet().network_id()?;

            let arg_or_server_address = argv.first().cloned().or_else(|| ctx.wallet().settings().get(WalletSettings::Server));
            let (is_public, url) = match arg_or_server_address.as_deref() {
                Some("public") => {
                    tprintln!(ctx, "Connecting to a public node");
                    (true, Resolver::default().get_url(WrpcEncoding::Borsh, network_id).await.map_err(|e| e.to_string())?)
                }
                None => {
                    tprintln!(ctx, "No server set, connecting to a public node");
                    (true, Resolver::default().get_url(WrpcEncoding::Borsh, network_id).await.map_err(|e| e.to_string())?)
                }
                Some(url) => {
                    (false, wrpc_client.parse_url_with_network_type(url.to_string(), network_id.into()).map_err(|e| e.to_string())?)
                }
            };

            if is_public {
                static WARNING: AtomicBool = AtomicBool::new(false);
                if !WARNING.load(Ordering::Relaxed) {
                    WARNING.store(true, Ordering::Relaxed);

                    tprintln!(ctx);

                    tpara!(
                        ctx,
                        "Please note that public node infrastructure is operated by contributors and \
                        accessing it may expose your IP address to different node providers. \
                        ",
                    );
                    tprintln!(ctx);
                    tpara!(ctx, "Please do not connect to public nodes directly as they are load-balanced.");
                    tprintln!(ctx);
                }
            }

            let options = ConnectOptions {
                block_async_connect: true,
                strategy: ConnectStrategy::Fallback,
                url: Some(url),
                ..Default::default()
            };
            wrpc_client.connect(Some(options)).await.map_err(|e| e.to_string())?;

            // Persist what we actually connected to, so the next session (and
            // this wallet's own metadata) reflect reality — previously only
            // the `server` command wrote the setting and it drifted.
            // Record what we actually connected to — including the no-argument
            // case, which is how most people connect and which previously
            // recorded nothing, so the wallet could never offer to reconnect.
            let recorded = argv.first().cloned().or_else(|| ctx.wallet().settings().get::<String>(WalletSettings::Server));
            if let Some(explicit) = recorded.as_ref() {
                if explicit != "public" {
                    ctx.wallet().settings().set(WalletSettings::Server, explicit.clone()).await.ok();
                    if ctx.wallet().is_open() {
                        if let Some(descriptor) = ctx.wallet().store().descriptor() {
                            if let Ok(meta) = ctx.wallet().store().client_metadata(&descriptor.filename).await {
                                // Absent metadata means a pre-v1 wallet: remembering is the
                                // default. An explicit remember=false is an opt-out — honor it.
                                let remember = meta.as_ref().map(|m| m.remember).unwrap_or(true);
                                if remember {
                                    let mut meta = meta.unwrap_or_default();
                                    meta.remember = true;
                                    meta.server = Some(explicit.clone());
                                    ctx.wallet().store().set_client_metadata(&descriptor.filename, Some(meta)).await.ok();
                                }
                            }
                        }
                    }
                }
            }
        } else {
            terrorln!(ctx, "Unable to connect with non-wRPC client");
        }
        Ok(())
    }
}
