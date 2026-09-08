use crate::imports::*;

#[derive(Default, Handler)]
#[help("Connect to a Marigold network")]
pub struct Connect;

impl Connect {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        if let Some(wrpc_client) = ctx.wallet().try_wrpc_client().as_ref() {
            let network_id = ctx.wallet().network_id()?;

            // A cleared setting is stored as an empty string; treat it as absent,
            // or `connect` would try to dial "" instead of a public node.
            let arg_or_server_address = argv
                .first()
                .cloned()
                .or_else(|| ctx.wallet().settings().get::<String>(WalletSettings::Server).filter(|s| !s.trim().is_empty()));
            let (is_public, url) = match arg_or_server_address.as_deref() {
                // No public nodes exist for Marigold yet, so say that rather
                // than fail against an empty list — and certainly rather than
                // reach for Kaspa's public node network, which this fork used
                // to inherit wholesale. Marigold's testnet-10 answers to the
                // same network-id string as Kaspa's, so being handed one of
                // their nodes would attach the wallet to a different chain.
                // A node we run, offered directly. Not a resolver: Marigold
                // has none deployed and at this scale needs none — the list is
                // shuffled, which load-balances well enough across a handful.
                Some("public") | None
                    if !kaspa_wrpc_client::resolver::public_nodes(network_id).is_empty() =>
                {
                    let node = kaspa_wrpc_client::resolver::public_nodes(network_id).remove(0);
                    tprintln!(ctx, "Connecting to a public Marigold node");
                    (true, wrpc_client.parse_url_with_network_type(node, network_id.into()).map_err(|e| e.to_string())?)
                }
                Some("public") | None if !Resolver::default().is_configured() => {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Marigold has no public nodes yet — there is nowhere to connect you automatically.");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Connect to a node by address:");
                    tprintln!(ctx, "  connect 127.0.0.1:27210      a node running on this machine");
                    tprintln!(ctx, "  connect <host>:27210         someone else's node");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "{}", style("'server <address>' remembers one, so 'connect' alone works next time.").dim());
                    tprintln!(ctx, "");
                    return Ok(());
                }
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

                    // Kaspa's wording said this infrastructure is run by
                    // contributors and load-balanced, and asked you not to
                    // connect directly. None of that is true here: this is one
                    // node the project runs, and connecting directly is exactly
                    // what happens. Saying so matters more for Marigold than it
                    // would for a transparent chain — the node sees which notes
                    // a wallet asks after, which is the one linkage the design
                    // otherwise never records.
                    tprintln!(ctx);
                    tpara!(
                        ctx,
                        "This is a node the Marigold project runs, offered so you can use a wallet without \
                        setting one up. Whoever runs a node sees the address you connect from and which notes \
                        your wallet asks about — the chain itself never records that, so a node you do not \
                        control is the one place it exists. For anything you care about, run your own — \
                        'node start', or see marigold.cash/faq. \
                        ",
                    );
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

            // Connecting by hand, after opening a wallet offline, is a normal
            // way to start a session — and it should get the same loud opening
            // housekeeping that opening while connected does.
            if ctx.wallet().is_open() {
                ctx.request_open_housekeeping();
            }

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
