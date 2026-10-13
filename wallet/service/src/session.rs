//! Opening a wallet the way every front end does: find the folder and the
//! network from the wallet's settings, connect to a node (a public computer,
//! one on this machine, or one inside the process when the host program can
//! start it), follow the notes in a light copy checked against the trustees'
//! anchors when the node is not our own, open the wallet, read its ledger,
//! and hand back the service. (docs/marigold/MOBILE-PLAN.md, PH-M1c.)

use crate::bundle;
use crate::error::Error;
use crate::providers::{MinerHostApi, NodeInside, Providers};
use crate::result::Result;
use crate::service::{Say, WalletService};
use futures::TryStreamExt;
use kaspa_consensus_core::network::NetworkId;
use kaspa_rpc_core::api::miner::MinerControl;
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::rpc::DynRpcApi;
use kaspa_wallet_core::wallet::Wallet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

enum Node {
    Own(Arc<dyn NodeInside>),
    Wrpc(Arc<kaspa_wrpc_client::KaspaRpcClient>),
    #[cfg(feature = "grpc")]
    Grpc(Arc<kaspa_grpc_client::GrpcClient>),
}

/// What a session needs to open: the same things `marigold-cli serve` takes
/// from its arguments, and the GUI from its screens.
pub struct SessionOptions {
    pub wallet: String,
    pub password: Secret,
    /// `grpc://…`, `ws://…`/`wss://…`, or `None` for a node inside the process.
    pub node: Option<String>,
    pub mine: Option<u32>,
    pub network: Option<NetworkId>,
    /// Named in the history beside every entry this session makes.
    pub origin: &'static str,
}

/// An open wallet on a connected node, with the service the bot and the GUI
/// talk to. Dropping it does not stop a node started inside the process;
/// call `close` for that.
pub struct Session {
    pub service: Arc<WalletService>,
    pub wallet: Arc<Wallet>,
    pub rpc: Arc<DynRpcApi>,
    node: Node,
    pub miner_host: Option<Arc<dyn MinerHostApi>>,
    pub folder: String,
    pub network_id: NetworkId,
}

impl Session {
    pub fn own_node(&self) -> bool {
        matches!(self.node, Node::Own(_))
    }

    pub async fn close(self) {
        self.service.close_wallet().await;
        match &self.node {
            Node::Own(node) => {
                let _ = node.stop().await;
            }
            Node::Wrpc(client) => {
                let _ = client.disconnect().await;
            }
            #[cfg(feature = "grpc")]
            Node::Grpc(_) => {}
        }
    }

    /// Lets go of the node: a node inside the process is stopped, a client's
    /// connection is closed. The wallet is not touched.
    pub async fn stop_node(&self) -> Result<()> {
        match &self.node {
            Node::Own(node) => node.stop().await?,
            Node::Wrpc(client) => {
                client.disconnect().await.ok();
            }
            #[cfg(feature = "grpc")]
            Node::Grpc(client) => {
                client.disconnect().await.ok();
            }
        }
        Ok(())
    }
}

/// Opens the wallet and connects it, the way `serve` does; `say` receives
/// the service's one-line remarks.
pub async fn open_session(options: &SessionOptions, shutdown: Arc<AtomicBool>, say: Say, providers: &Providers) -> Result<Session> {
    // The wallet's settings say which network and which folder; read them the
    // way the terminal wallet does, before anything opens.
    let probe = Wallet::try_with_rpc(None, Wallet::local_store()?, None)?;
    probe.load_settings().await.ok();
    let folder: String = probe
        .settings()
        .get::<String>(WalletSettings::Folder)
        .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
    let network_id = match options.network {
        Some(id) => id,
        None => probe
            .settings()
            .get::<String>(WalletSettings::Network)
            .and_then(|s| s.parse::<NetworkId>().ok())
            .unwrap_or(NetworkId::with_suffix(kaspa_consensus_core::network::NetworkType::Testnet, 10)),
    };
    drop(probe);

    log::info!("Marigold wallet service {}: wallet '{}' on {network_id}", env!("CARGO_PKG_VERSION"), options.wallet);
    let miner_host: Option<Arc<dyn MinerHostApi>> = match (options.mine, &providers.miner) {
        (Some(_), Some(factory)) => Some(factory(shutdown.clone())),
        (Some(_), None) => {
            log::warn!("--mine ignored: this program has no miner");
            None
        }
        _ => None,
    };
    let (node, rpc, rpc_ctl): (Node, Arc<DynRpcApi>, kaspa_wallet_core::rpc::Rpc) = match options.node.clone() {
        #[cfg(feature = "grpc")]
        Some(url) if url.starts_with("grpc://") => {
            let client = Arc::new(
                kaspa_grpc_client::GrpcClient::connect_with_args(
                    kaspa_rpc_core::notify::mode::NotificationMode::MultiListeners,
                    url.clone(),
                    None,
                    true,
                    None,
                    false,
                    None,
                    Default::default(),
                )
                .await
                .map_err(|err| Error::custom(format!("cannot reach {url}: {err}")))?,
            );
            log::info!("Using the node at {url}");
            let rpc: Arc<DynRpcApi> = client.clone();
            // A gRPC client has no connection control of its own for the
            // wallet to listen to: make one and open it once the wallet runs.
            let ctl = kaspa_wallet_core::rpc::RpcCtl::new();
            (Node::Grpc(client.clone()), rpc.clone(), kaspa_wallet_core::rpc::Rpc::new(rpc, ctl))
        }
        #[cfg(not(feature = "grpc"))]
        Some(url) if url.starts_with("grpc://") => {
            return Err(Error::custom(format!("{url}: this program has no gRPC client; use a ws:// address")));
        }
        Some(url) => {
            use kaspa_wallet_core::rpc::{ConnectOptions, ConnectStrategy, WrpcEncoding};
            say(if url.contains("127.0.0.1") || url.contains("localhost") {
                "Connecting to the Marigold program on this computer…".to_string()
            } else {
                "Connecting to the public computer…".to_string()
            });
            let client = Arc::new(
                kaspa_wrpc_client::KaspaRpcClient::new(WrpcEncoding::Borsh, Some(&url), None, Some(network_id), None)
                    .map_err(|err| Error::custom(format!("{url}: {err}")))?,
            );
            // Fallback with a deadline, not retry without end: a node that is
            // not there must say so in seconds, not hang the screen
            // (desktop tester, 2026-10-09, GitHub #13).
            let options = ConnectOptions {
                block_async_connect: true,
                strategy: ConnectStrategy::Fallback,
                url: Some(url.clone()),
                connect_timeout: Some(std::time::Duration::from_secs(15)),
                ..Default::default()
            };
            log::info!("Using the node at {url}");
            client.connect(Some(options)).await.map_err(|err| Error::custom(format!("cannot reach {url}: {err}")))?;
            let rpc: Arc<DynRpcApi> = client.clone();
            (Node::Wrpc(client.clone()), rpc.clone(), kaspa_wallet_core::rpc::Rpc::new(rpc, client.ctl().clone()))
        }
        None => {
            let Some(starter) = &providers.node else {
                return Err(Error::custom("this program cannot sync a node of its own: name a node to use"));
            };
            let control = miner_host.clone().map(|h| h as Arc<dyn MinerControl>);
            let (node, rpc) = starter.start(network_id, control).await?;
            log::info!("Syncing the network here");
            (Node::Own(node), rpc.rpc_api().clone(), rpc)
        }
    };

    #[cfg(feature = "grpc")]
    let rpc_ctl_open = rpc_ctl.rpc_ctl().clone();
    let wallet = Arc::new(Wallet::try_with_rpc(Some(rpc_ctl), Wallet::local_store()?, Some(network_id))?);
    wallet.load_settings().await.ok();
    wallet.store().set_storage_folder(&folder)?;
    wallet.clone().start().await?;
    match &node {
        Node::Own(node) => node.signal_connected().await?,
        #[cfg(feature = "grpc")]
        Node::Grpc(_) => {
            rpc_ctl_open.signal_open().await.map_err(|e| Error::custom(format!("rpc: {e}")))?;
        }
        Node::Wrpc(_) => {}
    }
    // On any computer that is not our own node, the notes are followed in a
    // light copy of the pool checked against the trustees' anchors, so the
    // computer is never asked which notes are ours (SYNC-LIGHT.md).
    if !matches!(node, Node::Own(_)) && kaspa_wallet_core::light::LightPool::supported(network_id) {
        say("Connected. Taking your own copy of the notes and checking it against the network…".to_string());
        match kaspa_wallet_core::light::LightPool::start(&rpc, network_id).await {
            Ok(light) => {
                let st = light.status();
                log::info!("light copy: {} notes as of the trustees' anchor at DAA {}", st.notes, st.anchored_daa);
                say("Your own copy of the notes is here, checked against the network. This computer is never asked which notes are yours.".to_string());
                wallet.set_light(Some(light.clone()));
                let (wallet_, rpc_, shutdown_) = (wallet.clone(), rpc.clone(), shutdown.clone());
                workflow_core::task::spawn(async move {
                    loop {
                        workflow_core::task::sleep(std::time::Duration::from_secs(2)).await;
                        if shutdown_.load(std::sync::atomic::Ordering::SeqCst) {
                            break;
                        }
                        match wallet_.light() {
                            Some(current) if Arc::ptr_eq(&current, &light) => {}
                            _ => break,
                        }
                        if !wallet_.is_connected() {
                            continue;
                        }
                        if let Err(err) = light.follow(&rpc_).await {
                            log::warn!("light wallet: {err}");
                        }
                    }
                });
            }
            Err(err) => say(format!(
                "This computer cannot serve a light wallet ({err}); it is asked about notes the old way, which tells it which notes are yours."
            )),
        }
    }
    match &node {
        _ => {}
    }
    say("Opening the wallet…".to_string());
    let descriptors = wallet.clone().wallet_open(options.password.clone(), Some(options.wallet.clone()), true, false).await?;
    if let Some(descriptors) = descriptors {
        let ids: Vec<_> = descriptors.iter().map(|d| d.account_id).collect();
        if !ids.is_empty() {
            // A wallet that has mined has thousands of ledger entries, and
            // through a public computer they come over the internet: this is
            // where a minute goes (founder, 2026-10-11).
            say("Reading your ledger. A wallet that has mined has many entries, so this can take a minute…".to_string());
            wallet.clone().accounts_activate(Some(ids)).await?;
        }
    }
    // Select the first account, as the terminal's 'open' does: `wallet.account()`
    // answers only for a selected one, and the miner's payout address, the
    // ledger line of the balance and the bot's mining all ask it. Without this
    // the desktop wallet showed "No miner runs with this wallet" and no ledger.
    {
        let guard = wallet.guard();
        let guard = guard.lock().await;
        if let Ok(mut accounts) = wallet.accounts(None, &guard).await
            && let Ok(Some(account)) = accounts.try_next().await
        {
            wallet.select(Some(&account)).await?;
        }
    }
    log::info!("Wallet '{}' open{}", options.wallet, if wallet.account().is_ok() { " with a ledger account" } else { ", notes only" });

    // The miner pays to this wallet's own address.
    if let (Some(host), Some(percent)) = (&miner_host, options.mine) {
        match wallet.account().and_then(|a| a.receive_address()) {
            Ok(address) => {
                host.bind_address(address.clone());
                host.bind(rpc.clone());
                log::info!("Mining {percent}% to {address} once the sync has caught up");
            }
            Err(_) => log::warn!("--mine ignored: this wallet keeps notes only, and mining pays to a ledger address"),
        }
    }

    // The wallet's backup key, written on the first open since the backups
    // started covering every wallet (2026-10-06).
    match bundle::ensure_recipient(&wallet, &options.wallet, &options.password).await {
        Ok(bundle::Covered::NewWords(_)) => {
            log::info!("wallet '{}' had no 24 words; made now — 'words' shows them", options.wallet)
        }
        Ok(_) => {}
        Err(err) => log::warn!("could not write the backup key of '{}': {err}", options.wallet),
    }

    let service = WalletService::new(
        wallet.clone(),
        options.password.clone(),
        network_id,
        &folder,
        &options.wallet,
        miner_host.clone().filter(|h| h.address_bound()),
        say,
        None,
        options.origin,
    );
    Ok(Session { service, wallet, rpc, node, miner_host, folder, network_id })
}
