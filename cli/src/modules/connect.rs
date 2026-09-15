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
            let typed_a_target = argv.first().is_some();
            let arg_or_server_address = argv
                .first()
                .cloned()
                .or_else(|| ctx.wallet().settings().get::<String>(WalletSettings::Server).filter(|s| !s.trim().is_empty()));
            let (mut is_public, url) = match arg_or_server_address.as_deref() {
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
            let dial = |target: String| ConnectOptions {
                block_async_connect: true,
                strategy: ConnectStrategy::Fallback,
                url: Some(target),
                ..Default::default()
            };
            let mut outcome = wrpc_client.connect(Some(dial(url))).await;

            // Your own node, not running. The old answer — "check the
            // address" — was about an address nobody typed: the wallet
            // remembers the node it last used, and a node inside this
            // program is not running until this program starts it. So start
            // it, say it is no use until it has caught up, and ask whether to
            // use a public node meanwhile. Ask, because whoever runs a public
            // node sees which notes this wallet asks about, and that is not a
            // choice to make on someone's behalf (founder, 2026-09-15).
            #[cfg(feature = "embedded-node")]
            let mut own_node_started = false;
            #[cfg(feature = "embedded-node")]
            if outcome.is_err() && is_local_target(&url_label) && !ctx.embedded_node_running() {
                tprintln!(ctx, "");
                tprintln!(ctx, "Your own node is not running. Starting it.");
                match ctx.spawn_embedded_node().await {
                    Err(err) => {
                        tprintln!(ctx, "{}", style(format!("Your node could not start: {err}")).yellow());
                        tprintln!(ctx, "'connect public' uses a public node instead.");
                        tprintln!(ctx, "");
                        return Ok(());
                    }
                    Ok(None) => {}
                    Ok(Some(rpc)) => {
                        if KaspaCli::node_is_synced(&rpc).await {
                            ctx.adopt_embedded_node(rpc).await?;
                            tprintln!(ctx, "{}", style("Your node is caught up. Using it — nobody else sees your notes.").green());
                            tprintln!(ctx, "");
                            ctx.print_next_step().await;
                            if ctx.wallet().is_open() {
                                ctx.request_open_housekeeping();
                            }
                            return Ok(());
                        }
                        ctx.announce_sync_started();
                        tprintln!(ctx, "Until it has caught up, your node cannot tell you what is on the ledger.");
                        tprintln!(ctx, "{}", style("A public node can, but whoever runs it sees which notes your wallet asks about.").dim());
                        let answer =
                            ctx.term().ask(false, "Use a public node while your node is loading? [y/N]: ").await?.trim().to_lowercase();
                        tprintln!(ctx, "");
                        let public = kaspa_wrpc_client::resolver::public_nodes(network_id)
                            .into_iter()
                            .next()
                            .and_then(|node| wrpc_client.parse_url_with_network_type(node, network_id.into()).ok());
                        match (answer.starts_with('y'), public) {
                            (true, Some(target)) => {
                                tprintln!(ctx, "Connecting to a public node until your own is ready.");
                                outcome = wrpc_client.connect(Some(dial(target))).await;
                                is_public = true;
                                own_node_started = true;
                                ctx.start_node_handover_task(rpc);
                            }
                            (true, None) => {
                                tprintln!(ctx, "There is no public node to use. Using your own while it catches up.");
                                ctx.adopt_embedded_node(rpc).await?;
                                ctx.print_next_step().await;
                                return Ok(());
                            }
                            (false, _) => {
                                ctx.adopt_embedded_node(rpc).await?;
                                tprintln!(ctx, "Using your own node. What it shows of the ledger is incomplete until it has");
                                tprintln!(ctx, "caught up — 'node status' shows progress.");
                                tprintln!(ctx, "");
                                ctx.print_next_step().await;
                                return Ok(());
                            }
                        }
                    }
                }
            }

            // A remembered node that is not answering should not leave the
            // wallet with nothing. Typing an address is a statement of intent
            // and is reported as-is; a saved default is just a preference, so
            // fall back to a public node and bring your own up behind it.
            let mut fell_back = false;
            if outcome.is_err() && !is_public && !typed_a_target {
                let public = kaspa_wrpc_client::resolver::public_nodes(network_id);
                if let Some(node) = public.into_iter().next() {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "{}", style(format!("{url_label} is not answering.")).yellow());
                    tprintln!(ctx, "Using a public node for now, and starting your own behind it.");
                    tprintln!(ctx, "");
                    if let Ok(target) = wrpc_client.parse_url_with_network_type(node, network_id.into()) {
                        outcome = wrpc_client.connect(Some(dial(target))).await;
                        if outcome.is_ok() {
                            is_public = true;
                            fell_back = true;
                        }
                    }
                }
            }

            if let Err(err) = outcome {
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
            if own_node_started {
                tprintln!(ctx, "Public node connected. Your own takes over once it has caught up.");
            } else if want_local || fell_back {
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

/// A node on this machine: the one this program can start.
#[cfg(feature = "embedded-node")]
fn is_local_target(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    ["localhost", "127.0.0.1", "[::1]", "0.0.0.0"].iter().any(|host| url.contains(host))
}
