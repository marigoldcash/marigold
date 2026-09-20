use crate::imports::*;
#[cfg(feature = "embedded-node")]
use kaspa_wallet_core::rpc::Rpc;

#[derive(Default, Handler)]
#[help("Connect to a Marigold network")]
pub struct Connect;

impl Connect {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        // A connection dropped while this command runs was dropped on
        // purpose; the "Disconnected" / "Connected again" pair is for the
        // ones that were not.
        let _switching = ctx.switching();

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

        // 'connect' is the network, all of it (founder, 2026-09-16): the
        // copy of it this wallet keeps on this machine, its progress, and
        // — only when asked — a public computer to use meanwhile. There is
        // no 'node' to think about.
        #[cfg(feature = "embedded-node")]
        match argv.first().map(|s| s.as_str()) {
            Some("status") => {
                crate::modules::node::Node::default().status(&ctx).await;
                return Ok(());
            }
            Some("details") => {
                crate::modules::node::Node::default().details(&ctx).await;
                return Ok(());
            }
            Some("logs") => {
                let on = !matches!(argv.get(1).map(|s| s.as_str()), Some("off"));
                crate::embedded::set_logs_wanted(on);
                tprintln!(ctx, "Sync logs are {}.", if on { "on — 'connect logs off' to silence them" } else { "off" });
                return Ok(());
            }
            // Already syncing (or in sync): 'connect' alone is a status report.
            None if ctx.embedded_node_running() => {
                crate::modules::node::Node::default().status(&ctx).await;
                return Ok(());
            }
            _ => {}
        }

        let want_local = matches!(argv.first().map(|s| s.as_str()), Some("local") | Some("mine") | Some("own"));
        if want_local {
            argv.remove(0);
            if !cfg!(feature = "embedded-node") {
                tprintln!(ctx, "This build cannot sync the network itself. 'connect <host>:port' reaches a computer that does.");
                return Ok(());
            }
        }

        if let Some(wrpc_client) = ctx.wallet().try_wrpc_client().as_ref() {
            let network_id = ctx.wallet().network_id()?;

            // A cleared setting is stored as an empty string; treat it as absent.
            // A remembered "public", or a remembered address on this machine,
            // is not a target either: bare 'connect' means the network here.
            let typed_a_target = !argv.is_empty();
            let mut arg_or_server_address = argv.first().cloned().or_else(|| {
                ctx.wallet()
                    .settings()
                    .get::<String>(WalletSettings::Server)
                    .filter(|s| !s.trim().is_empty() && s != "public" && !is_local_target(s))
            });

            // Something on this machine may already be syncing the network:
            // the background miner (PLAN P8.3c), or a marigoldd. Use it
            // rather than start a second sync beside it.
            #[cfg(feature = "embedded-node")]
            let own_sync_running = ctx.embedded_node_running();
            #[cfg(not(feature = "embedded-node"))]
            let own_sync_running = false;
            if arg_or_server_address.is_none()
                && !own_sync_running
                && let Some(status) = probe_local_node(network_id).await
            {
                if status.available {
                    tpara!(ctx, "Found the Marigold miner running in the background on this machine — using its copy of the network.");
                } else {
                    tpara!(ctx, "A Marigold node is already running on this machine — using it.");
                }
                arg_or_server_address = Some(format!("127.0.0.1:{}", network_id.default_borsh_rpc_port()));
            }

            // Bare 'connect' with the network compiled in: start syncing here.
            #[cfg(feature = "embedded-node")]
            if arg_or_server_address.is_none() {
                let public = kaspa_wrpc_client::resolver::public_nodes(network_id)
                    .into_iter()
                    .next()
                    .and_then(|node| wrpc_client.parse_url_with_network_type(node, network_id.into()).ok());
                match start_network_sync(&ctx, public.is_some()).await? {
                    SyncStart::Settled => return Ok(()),
                    SyncStart::PublicOnly => {
                        let target = public.expect("offered only when there is one");
                        if let Err(err) = dial(wrpc_client, target).await {
                            tprintln!(ctx, "{}", style("Could not reach the public computer.").yellow());
                            if ctx.advanced() {
                                tprintln!(ctx, "{}", style(format!("({err})")).dim());
                            }
                            return Ok(());
                        }
                        for _ in 0..40 {
                            if ctx.wallet().is_connected() {
                                break;
                            }
                            workflow_core::task::sleep(std::time::Duration::from_millis(250)).await;
                        }
                        tprintln!(ctx, "Public computer connected.");
                        ctx.print_next_step().await;
                        if ctx.wallet().is_open() {
                            ctx.request_open_housekeeping();
                        }
                        return Ok(());
                    }
                    SyncStart::PublicMeanwhile(rpc) => {
                        let target = public.expect("offered only when there is one");
                        if let Err(err) = dial(wrpc_client, target).await {
                            tprintln!(ctx, "{}", style("Could not reach the public computer.").yellow());
                            if ctx.advanced() {
                                tprintln!(ctx, "{}", style(format!("({err})")).dim());
                            }
                            tprintln!(ctx, "Using your own copy while it catches up — 'connect status' shows progress.");
                            ctx.adopt_embedded_node(rpc).await?;
                            return Ok(());
                        }
                        ctx.start_node_handover_task(rpc);
                        for _ in 0..40 {
                            if ctx.wallet().is_connected() {
                                break;
                            }
                            workflow_core::task::sleep(std::time::Duration::from_millis(250)).await;
                        }
                        tprintln!(ctx, "Public computer connected. Your own sync takes over once it has caught up.");
                        ctx.print_next_step().await;
                        if ctx.wallet().is_open() {
                            ctx.request_open_housekeeping();
                        }
                        return Ok(());
                    }
                }
            }

            let (mut is_public, url) = match arg_or_server_address.as_deref() {
                // No public computers exist for Marigold yet, so say that rather
                // than fail against an empty list — and certainly rather than
                // reach for Kaspa's public node network, which this fork used
                // to inherit wholesale. Marigold's testnet-10 answers to the
                // same network-id string as Kaspa's, so being handed one of
                // their nodes would attach the wallet to a different chain.
                Some("public") | None if !kaspa_wrpc_client::resolver::public_nodes(network_id).is_empty() => {
                    let node = kaspa_wrpc_client::resolver::public_nodes(network_id).remove(0);
                    let which = if network_id.is_mainnet() { "" } else { " test" };
                    tprintln!(ctx, "Connecting to a public Marigold{which} computer.");
                    (true, wrpc_client.parse_url_with_network_type(node, network_id.into()).map_err(|e| e.to_string())?)
                }
                Some("public") | None if !Resolver::default().is_configured() => {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Marigold has no public computers yet — there is nowhere to connect you automatically.");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Connect to one by address:");
                    tprintln!(ctx, "  connect <host>:27210         someone else's copy of the network");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "{}", style("'server <address>' remembers one, so 'connect' alone works next time.").dim());
                    tprintln!(ctx, "");
                    return Ok(());
                }
                Some("public") => {
                    tprintln!(ctx, "Connecting to a public computer");
                    (true, Resolver::default().get_url(WrpcEncoding::Borsh, network_id).await.map_err(|e| e.to_string())?)
                }
                None => {
                    tprintln!(ctx, "Connecting to a public computer");
                    (true, Resolver::default().get_url(WrpcEncoding::Borsh, network_id).await.map_err(|e| e.to_string())?)
                }
                Some(url) => {
                    (false, wrpc_client.parse_url_with_network_type(url.to_string(), network_id.into()).map_err(|e| e.to_string())?)
                }
            };

            let url_label = url.clone();
            let mut outcome = dial(wrpc_client, url).await;

            // An address on this machine that is not answering: the network
            // is not being synced here yet, so start it — same flow as bare
            // 'connect'.
            #[cfg(feature = "embedded-node")]
            let mut own_node_started = false;
            #[cfg(feature = "embedded-node")]
            if outcome.is_err() && is_local_target(&url_label) && !ctx.embedded_node_running() {
                let public = kaspa_wrpc_client::resolver::public_nodes(network_id)
                    .into_iter()
                    .next()
                    .and_then(|node| wrpc_client.parse_url_with_network_type(node, network_id.into()).ok());
                match start_network_sync(&ctx, public.is_some()).await? {
                    SyncStart::Settled => return Ok(()),
                    SyncStart::PublicOnly => {
                        let target = public.expect("offered only when there is one");
                        outcome = dial(wrpc_client, target).await;
                        is_public = true;
                    }
                    SyncStart::PublicMeanwhile(rpc) => {
                        let target = public.expect("offered only when there is one");
                        outcome = dial(wrpc_client, target).await;
                        is_public = true;
                        own_node_started = true;
                        ctx.start_node_handover_task(rpc);
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
                    tprintln!(ctx, "Using a public computer for now, and starting your own sync behind it.");
                    tprintln!(ctx, "");
                    if let Ok(target) = wrpc_client.parse_url_with_network_type(node, network_id.into()) {
                        outcome = dial(wrpc_client, target).await;
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
                    tprintln!(ctx, "{}", style("Could not reach the public computer.").yellow());
                    tprintln!(ctx, "It may be down, or this machine may be offline. Nothing is wrong with your");
                    tprintln!(ctx, "wallet — your notes are on this disk and are not going anywhere.");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "  'connect'       try again");
                    if cfg!(feature = "embedded-node") && !want_local {
                        tprintln!(ctx, "  'connect local' sync the network here instead, which needs nobody else");
                    }
                } else {
                    tprintln!(ctx, "{}", style(format!("Could not reach {}.", url_label)).yellow());
                    tprintln!(ctx, "Check the address, or 'connect public' to use a public computer instead.");
                }
                tprintln!(ctx, "");
                // The technical reason, for whoever wants it, last and dimmed.
                if ctx.advanced() {
                    tprintln!(ctx, "{}", style(format!("({err})")).dim());
                } else {
                    tprintln!(ctx, "{}", style("'advanced on' shows the reason.").dim());
                }
                tprintln!(ctx, "");
                #[cfg(feature = "embedded-node")]
                if want_local {
                    // They asked for their own node. Not reaching somebody
                    // else's is no reason not to start it.
                    tprintln!(ctx, "Starting the sync here anyway.");
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
            // 'connect public' was a choice; it is not followed by the question
            // about syncing here — 'connect' alone does that.
            #[cfg(feature = "embedded-node")]
            if own_node_started {
                tprintln!(ctx, "Public computer connected. Your own sync takes over once it has caught up.");
            } else if want_local || fell_back {
                ctx.start_local_node_now().await?;
            } else if is_public {
                tprintln!(ctx, "Public computer connected.");
            }
            #[cfg(not(feature = "embedded-node"))]
            {
                // Set above for the build with a node; read here so this one
                // does not call the assignment dead.
                let _ = fell_back;
                if is_public {
                    tprintln!(ctx, "Public computer connected.");
                }
            }

            // A node on this machine may have the miner program in it. If so
            // it is ours to steer, and the own lane counts on it. Asked
            // through a throwaway client, not the wallet's: a node from before
            // this call existed drops the connection on a question it does
            // not know, and the wallet's own link is not the place for that.
            if is_local_target(&url_label) {
                let status = probe_local_node(network_id).await.filter(|status| status.available);
                ctx.note_remote_miner(status.as_ref());
                if let Some(status) = status {
                    if status.mining {
                        tprintln!(
                            ctx,
                            "The background miner is at work: {} — 'mine status' for more.",
                            crate::miner::format_hashrate(status.hashrate)
                        );
                    } else {
                        tprintln!(ctx, "The background miner is idle — 'mine start' sets it going.");
                    }
                }
            } else {
                ctx.note_remote_miner(None);
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
            if let Some(explicit) = recorded.as_ref()
                && explicit != "public"
            {
                ctx.wallet().settings().set(WalletSettings::Server, explicit.clone()).await.ok();
                if ctx.wallet().is_open()
                    && let Some(descriptor) = ctx.wallet().store().descriptor()
                    && let Ok(meta) = ctx.wallet().store().client_metadata(&descriptor.filename).await
                {
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
        } else {
            tprintln!(ctx, "This wallet cannot connect from here.");
        }
        Ok(())
    }
}

/// How long a first dial waits for an answer before the wallet gives up.
const DIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Dial `url` for the wallet's own connection: bounded on the first attempt,
/// and kept alive afterwards.
///
/// The wallet used to dial with the fallback strategy, which returns the
/// reason when a node cannot be reached — and also gives up for good the
/// first time a reconnect fails. A node restarted while the wallet was on it
/// left the wallet disconnected until somebody typed 'connect', with a miner
/// asking a dead socket for work every half second (founder, 2026-09-20).
/// Now the socket retries on its own for as long as the wallet is on this
/// node; a first dial that nobody answers within [`DIAL_TIMEOUT`] is stopped
/// here and reported, so a wrong address still fails fast.
pub(crate) async fn dial(client: &kaspa_wrpc_client::KaspaRpcClient, url: String) -> std::result::Result<(), String> {
    let options = ConnectOptions {
        block_async_connect: false,
        strategy: ConnectStrategy::Retry,
        url: Some(url.clone()),
        connect_timeout: Some(DIAL_TIMEOUT),
        retry_interval: Some(std::time::Duration::from_secs(3)),
    };
    client.connect(Some(options)).await.map_err(|err| err.to_string())?;
    let started = std::time::Instant::now();
    while started.elapsed() < DIAL_TIMEOUT {
        if client.is_connected() {
            return Ok(());
        }
        workflow_core::task::sleep(std::time::Duration::from_millis(100)).await;
    }
    // Nobody answered: stop the retrying, or the wallet would sit dialling a
    // typo forever behind whatever the person does next.
    client.disconnect().await.ok();
    Err(format!("no answer from {url} within {} seconds", DIAL_TIMEOUT.as_secs()))
}

/// Is a node answering on this machine's wRPC port, and does it have a miner
/// in it? `None` when nothing answers. A node too old to know the question is
/// treated as a node without a miner.
async fn probe_local_node(network_id: NetworkId) -> Option<kaspa_rpc_core::RpcMinerStatus> {
    let url = format!("ws://127.0.0.1:{}", network_id.default_borsh_rpc_port());
    let client = kaspa_wrpc_client::KaspaRpcClient::new(WrpcEncoding::Borsh, Some(&url), None, Some(network_id), None).ok()?;
    let options = ConnectOptions {
        block_async_connect: true,
        strategy: ConnectStrategy::Fallback,
        url: Some(url),
        connect_timeout: Some(std::time::Duration::from_secs(2)),
        ..Default::default()
    };
    if client.connect(Some(options)).await.is_err() {
        return None;
    }
    let status = client.get_miner_status().await.unwrap_or_default();
    client.disconnect().await.ok();
    Some(status)
}

/// An address on this machine: the network sync this program runs itself.
pub(crate) fn is_local_target(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    ["localhost", "127.0.0.1", "[::1]", "0.0.0.0"].iter().any(|host| url.contains(host))
}

#[cfg(feature = "embedded-node")]
enum SyncStart {
    /// Everything is settled here: in sync already, or staying on our own
    /// copy while it catches up, or nothing could start.
    Settled,
    /// The person wants a public computer meanwhile; the caller dials it and
    /// hands over to this sync once it has caught up.
    PublicMeanwhile(Rpc),
    /// No sync of our own could start (no room for one); the person chose a
    /// public computer outright.
    PublicOnly,
}

/// Start syncing the network on this machine and say what that means.
///
/// The words are the founder's (2026-09-16): no node, just the network and
/// the sync of it. A sync that has not caught up can neither show the
/// ledger nor move notes, so the one question asked is whether to use a
/// public computer meanwhile — asked, never assumed, because whoever runs
/// it sees which notes this wallet asks about.
#[cfg(feature = "embedded-node")]
async fn start_network_sync(ctx: &Arc<KaspaCli>, public_available: bool) -> Result<SyncStart> {
    tprintln!(ctx, "");
    tprintln!(ctx, "Starting sync with the network...");
    let rpc = match ctx.spawn_embedded_node().await {
        Err(err) => {
            tprintln!(ctx, "{}", style(format!("Sync could not start: {err}")).yellow());
            tprintln!(ctx, "'connect public' uses a public computer instead.");
            tprintln!(ctx, "");
            return Ok(SyncStart::Settled);
        }
        Ok(None) => {
            // Already syncing here: nothing to add. Refused (no room): the
            // wallet used to stop there, disconnected, on a Mac with a small
            // disk (founder, 2026-09-19). Offer the public computer, with the
            // same caveat the catching-up case gives.
            if ctx.embedded_node_running() || !public_available {
                return Ok(SyncStart::Settled);
            }
            tpara!(
                ctx,
                "{}",
                style("A public computer has all the data already, but whoever runs it sees which notes your wallet asks about.")
                    .dim()
            );
            tprintln!(ctx, "");
            let answer = ctx.term().ask(false, "Use a public computer instead? [y/N]: ").await?.trim().to_lowercase();
            tprintln!(ctx, "");
            if answer.starts_with('y') {
                return Ok(SyncStart::PublicOnly);
            }
            tprintln!(ctx, "Staying disconnected. 'connect public' reaches a public computer whenever you want one.");
            tprintln!(ctx, "");
            return Ok(SyncStart::Settled);
        }
        Ok(Some(rpc)) => rpc,
    };
    if KaspaCli::node_is_synced(&rpc).await {
        ctx.adopt_embedded_node(rpc).await?;
        tprintln!(ctx, "{}", style("In sync with the network. Nobody else sees your notes.").green());
        tprintln!(ctx, "");
        ctx.print_next_step().await;
        if ctx.wallet().is_open() {
            ctx.request_open_housekeeping();
        }
        return Ok(SyncStart::Settled);
    }
    ctx.announce_sync_started();
    // Wrapped to the terminal, not by hand: hand-broken lines spilled a word
    // onto the next line on an 80-column screen (founder, 2026-09-16).
    tpara!(ctx, "Until sync has caught up, we can't tell what is on the ledger, and some things don't work without it.");
    tpara!(
        ctx,
        "{}",
        style("We could connect to a public computer that has all the data already, but whoever runs it sees which notes your wallet asks about.").dim()
    );
    tprintln!(ctx, "");
    let answer = ctx.term().ask(false, "Use a public computer until sync has caught up? [y/N]: ").await?.trim().to_lowercase();
    tprintln!(ctx, "");
    if answer.starts_with('y') {
        if public_available {
            tprintln!(ctx, "Connecting to a public computer until your own sync has caught up.");
            return Ok(SyncStart::PublicMeanwhile(rpc));
        }
        tprintln!(ctx, "There is no public computer to use. Staying on your own copy while it catches up.");
    } else {
        tpara!(
            ctx,
            "Staying on your own. The ledger stays unread and some things wait until sync has caught up — 'connect status' shows progress."
        );
    }
    ctx.adopt_embedded_node(rpc).await?;
    tprintln!(ctx, "");
    ctx.print_next_step().await;
    Ok(SyncStart::Settled)
}
