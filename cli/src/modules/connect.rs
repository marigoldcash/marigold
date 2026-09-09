use crate::imports::*;

#[derive(Default, Handler)]
#[help("Connect to a Marigold network")]
pub struct Connect;

impl Connect {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        // 'local' is a word this program puts in front of people — the connect
        // prompt offers it, 'node status' suggests it — so they type it at
        // 'connect'. It used to be passed to the resolver as a hostname and
        // came back "failed to lookup address information: Name or service not
        // known", which is a true statement about DNS and no help at all.
        //
        // It means the same thing as answering yes below, so it runs the same
        // path: connect to a public node, start your own, hand over when it is
        // ready. Only the question is skipped.
        let mut argv = argv;
        let want_local = matches!(argv.first().map(|s| s.as_str()), Some("local") | Some("mine") | Some("own"));
        if want_local {
            argv.remove(0);
            if !cfg!(feature = "embedded-node") {
                tprintln!(ctx, "This build has no node in it. 'connect <host>:port' reaches one elsewhere.");
                return Ok(());
            }
        }

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
                    // Name the network here. Until launch every build is a
                    // testnet build, and someone who has read about Marigold
                    // elsewhere should not have to wonder whether the coins
                    // they are about to hold are the real ones.
                    let which = if network_id.is_mainnet() { "" } else { " test" };
                    tprintln!(ctx, "Connecting to a public Marigold{which} node.");
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

            let url_label = url.clone();
            let options = ConnectOptions {
                block_async_connect: true,
                strategy: ConnectStrategy::Fallback,
                url: Some(url),
                ..Default::default()
            };
            if let Err(err) = wrpc_client.connect(Some(options)).await {
                tprintln!(ctx, "");
                if is_public {
                    tprintln!(ctx, "{}", style("Could not reach the public node.").yellow());
                    tprintln!(ctx, "It may be down, or this machine may be offline. Nothing is wrong with your");
                    tprintln!(ctx, "wallet — your notes are on this disk and are not going anywhere.");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "  'connect'       try again");
                    if cfg!(feature = "embedded-node") && !want_local {
                        tprintln!(ctx, "  'connect local' run your own node instead, which needs nobody else");
                    }
                } else {
                    tprintln!(ctx, "{}", style(format!("Could not reach {}.", url_label)).yellow());
                    tprintln!(ctx, "Check the address, or 'connect' to use a public node instead.");
                }
                tprintln!(ctx, "");
                // The technical reason, for whoever wants it, last and dimmed.
                tprintln!(ctx, "{}", style(format!("({err})")).dim());
                tprintln!(ctx, "");
                #[cfg(feature = "embedded-node")]
                if want_local {
                    // They asked for their own node. Not reaching somebody
                    // else's is no reason not to start it.
                    tprintln!(ctx, "Starting your own node anyway.");
                    ctx.start_local_node_now().await?;
                }
                return Ok(());
            }

            // The socket is up when connect() returns, but the wallet only
            // considers itself connected once the event reaches its utxo
            // processor. Everything below reads better for the wait: the
            // node's own greeting arrives before our question rather than
            // interrupting it, "Public node connected" is true when we say it,
            // and 'node status' typed straight afterwards does not answer
            // "not connected to any node".
            for _ in 0..40 {
                if ctx.wallet().is_connected() {
                    break;
                }
                workflow_core::task::sleep(std::time::Duration::from_millis(250)).await;
            }

            // Offered after the connection lands, not before: until it does,
            // "run your own instead" is a question about a thing that might
            // not have worked.
            #[cfg(feature = "embedded-node")]
            if want_local {
                ctx.start_local_node_now().await?;
            } else if is_public {
                ctx.offer_local_node().await?;
            }
            #[cfg(not(feature = "embedded-node"))]
            if is_public {
                tprintln!(ctx, "Public node connected.");
            }
            ctx.print_next_step().await;

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
