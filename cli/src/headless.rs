//! The miner as a service (PLAN P8.3c).
//!
//! `marigold-cli mine-to <address> [--cpu N] [--network <id>]` runs with no
//! terminal and no wallet: the network syncing on this machine, a CPU miner
//! paying to the address given, and an RPC on 127.0.0.1 — this machine only —
//! so a wallet here finds it, uses its copy of the network instead of syncing
//! a second one, and steers the miner with 'mine start', 'mine stop' and
//! 'mine status'. It stays in the foreground and logs to stdout; systemd or
//! Docker does the daemonising, which is what they are for. SIGTERM (and
//! Ctrl-C) stop it cleanly.

use crate::miner::MinerHost;
use crate::result::Result;
use kaspa_addresses::{Address, Prefix};
use kaspa_consensus_core::network::{NetworkId, NetworkType};
use kaspa_core::signals::{Shutdown, Signals};
use kaspa_rpc_core::api::rpc::RpcApi;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const USAGE: &str = "usage: marigold-cli mine-to <address> [<percent>] [--network <id>] [--node <url>]

  <address>        where the rewards go — an address from 'address' in your wallet
  <percent>        share of this machine to use, 1-100 (default 50); '--cpu 50' means the same
  --network <id>   mainnet, testnet-10, ... (default: the one the address belongs to)
  --listen <addr>  let wallets on other machines connect to this miner, e.g. --listen 192.168.1.20:27210
                   (default: this machine only, 127.0.0.1). Your own network only: anyone who can
                   reach that address can start or stop the miner and read the payout address.
  --node <url>     mine against a node already running instead of syncing one here:
                   ws://127.0.0.1:27210 (wRPC) or grpc://127.0.0.1:26210 (gRPC, what a
                   marigoldd has on by default). No wallet can steer the miner then.

Runs in the foreground and logs to stdout; stop it with Ctrl-C or SIGTERM.
A wallet on this machine finds it when it connects and steers it with 'mine'.";

struct Stop(Arc<AtomicBool>);

impl Shutdown for Stop {
    fn shutdown(self: &Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Options {
    address: Address,
    percent: u32,
    network_id: NetworkId,
    node: Option<String>,
    /// Where the miner's own node listens for wallets; `None` is loopback.
    listen: Option<String>,
}

fn parse(args: &[String]) -> std::result::Result<Options, String> {
    let mut address = None;
    let mut percent = 50u32;
    let mut network = None;
    let mut node = None;
    let mut listen: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--cpu" => {
                let value = inline.or_else(|| iter.next().cloned()).ok_or("--cpu needs a percentage")?;
                percent = value
                    .trim_end_matches('%')
                    .parse::<u32>()
                    .ok()
                    .filter(|p| (1..=100).contains(p))
                    .ok_or_else(|| format!("'{value}' is not a percentage between 1 and 100"))?;
            }
            "--network" => {
                let value = inline.or_else(|| iter.next().cloned()).ok_or("--network needs a network id")?;
                network = Some(value.parse::<NetworkId>().map_err(|err| format!("'{value}' is not a network: {err}"))?);
            }
            "--listen" => {
                let value = inline
                    .or_else(|| iter.next().cloned())
                    .ok_or("--listen needs an address, e.g. 0.0.0.0:27210 or 192.168.1.20:27210")?;
                listen = Some(value);
            }
            "--node" => {
                let value = inline.or_else(|| iter.next().cloned()).ok_or("--node needs a url, e.g. ws://127.0.0.1:27210")?;
                node = Some(if value.contains("://") { value } else { format!("ws://{value}") });
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'\n\n{USAGE}")),
            // A bare number is the share of the machine: 'mine-to <address> 50'.
            other if other.trim_end_matches('%').chars().all(|c| c.is_ascii_digit()) && !other.is_empty() => {
                percent = other
                    .trim_end_matches('%')
                    .parse::<u32>()
                    .ok()
                    .filter(|p| (1..=100).contains(p))
                    .ok_or_else(|| format!("'{other}' is not a percentage between 1 and 100"))?;
            }
            other => {
                if address.is_some() {
                    return Err(format!("one address only\n\n{USAGE}"));
                }
                address = Some(Address::try_from(other).map_err(|err| format!("'{other}' is not a Marigold address: {err}"))?);
            }
        }
    }
    let address = address.ok_or_else(|| format!("an address to pay the rewards to is needed\n\n{USAGE}"))?;
    let network_id = match network {
        Some(id) => {
            if Prefix::from(id.network_type()) != address.prefix {
                return Err(format!("{address} is not an address on {id}"));
            }
            id
        }
        None => match address.prefix {
            Prefix::Mainnet => NetworkId::new(NetworkType::Mainnet),
            Prefix::Testnet => NetworkId::with_suffix(NetworkType::Testnet, 10),
            Prefix::Simnet => NetworkId::new(NetworkType::Simnet),
            Prefix::Devnet => NetworkId::new(NetworkType::Devnet),
        },
    };
    Ok(Options { address, percent, network_id, node, listen })
}

/// The miner's node: one started here, or one already running elsewhere.
enum Node {
    Own(Arc<crate::embedded::EmbeddedNode>),
    Remote(Arc<kaspa_wrpc_client::KaspaRpcClient>),
    Grpc(Arc<kaspa_grpc_client::GrpcClient>),
}

pub async fn mine_to(args: Vec<String>) -> Result<()> {
    let options = match parse(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let Options { address, percent, network_id, node: node_url, listen } = options;

    // Plain lines on stdout, one per record, for journald or a log file.
    kaspa_core::log::init_logger(None, "info");

    let shutdown = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(Stop(shutdown.clone()));
    Arc::new(Signals::new(&stop)).init();

    let host = MinerHost::new(address.clone(), shutdown.clone());
    log::info!(
        "Marigold miner {}: paying to {address}, {percent}% of this machine ({} of {} cores), {network_id}",
        env!("CARGO_PKG_VERSION"),
        crate::miner::threads_for_percent(percent),
        crate::miner::cores()
    );
    let (node, rpc): (Node, Arc<kaspa_wallet_core::rpc::DynRpcApi>) = match node_url {
        // A node already running — a marigoldd on this machine, say. Nothing
        // to sync here, and no RPC of our own for a wallet to steer.
        Some(url) if url.starts_with("grpc://") => {
            let client = kaspa_grpc_client::GrpcClient::connect_with_args(
                kaspa_rpc_core::notify::mode::NotificationMode::Direct,
                url.clone(),
                None,
                true,
                None,
                false,
                None,
                Default::default(),
            )
            .await
            .map_err(|err| crate::error::Error::custom(format!("cannot reach {url}: {err}")))?;
            let client = Arc::new(client);
            log::info!("Mining against the node at {url}; it keeps reconnecting if that node goes away.");
            if let Ok(info) = client.get_server_info().await
                && info.network_id != network_id
            {
                return Err(crate::error::Error::custom(format!(
                    "{url} is on {}, but {address} is a {network_id} address",
                    info.network_id
                )));
            }
            let rpc: Arc<kaspa_wallet_core::rpc::DynRpcApi> = client.clone();
            (Node::Grpc(client), rpc)
        }
        Some(url) => {
            use kaspa_wallet_core::rpc::{ConnectOptions, ConnectStrategy, WrpcEncoding};
            let client = Arc::new(
                kaspa_wrpc_client::KaspaRpcClient::new(WrpcEncoding::Borsh, Some(&url), None, Some(network_id), None)
                    .map_err(|err| crate::error::Error::custom(format!("{url}: {err}")))?,
            );
            let options = ConnectOptions {
                block_async_connect: true,
                strategy: ConnectStrategy::Retry,
                url: Some(url.clone()),
                ..Default::default()
            };
            log::info!("Mining against the node at {url}; it keeps reconnecting if that node goes away.");
            client.connect(Some(options)).await.map_err(|err| crate::error::Error::custom(format!("cannot reach {url}: {err}")))?;
            if let Ok(info) = client.get_server_info().await
                && info.network_id != network_id
            {
                return Err(crate::error::Error::custom(format!(
                    "{url} is on {}, but {address} is a {network_id} address",
                    info.network_id
                )));
            }
            let rpc: Arc<kaspa_wallet_core::rpc::DynRpcApi> = client.clone();
            (Node::Remote(client), rpc)
        }
        None => {
            let appdir = crate::embedded::appdir(network_id).await?;
            let control: Arc<dyn kaspa_rpc_core::api::miner::MinerControl> = host.clone();
            let (node, rpc) = crate::embedded::EmbeddedNode::start_with(network_id, &appdir, Some(control), listen.as_deref())?;
            let port = network_id.default_borsh_rpc_port();
            match &listen {
                Some(address) => log::info!(
                    "Wallets reach this miner at {address} — 'connect {address}' on another machine, 'connect' on this one."
                ),
                None => log::info!("A wallet on this machine reaches this miner at 127.0.0.1:{port} — 'connect' finds it by itself."),
            }
            (Node::Own(node), rpc.rpc_api().clone())
        }
    };
    host.bind(rpc.clone());

    // Mine only on a synced copy: templates built on a chain the node has not
    // finished reading make blocks nobody accepts.
    // Started here once, when the sync first catches up. From then on a
    // wallet owns start and stop: a miner stopped with 'mine stop' stays
    // idle, and is not quietly restarted at the next tick.
    let mut waited = 0u64;
    let mut started_once = false;
    let mut last_report = std::time::Instant::now();
    while !shutdown.load(Ordering::SeqCst) {
        // The anchor drill's attacker node never calls itself synced (no peers,
        // so no incoming blocks); MARIGOLD_MINE_UNSYNCED lifts the gate there.
        let synced = matches!(rpc.get_server_info().await, Ok(info) if info.is_synced) || crate::miner::mine_unsynced();
        // A wallet may have started it already, before the sync caught up.
        if host.miner().is_some() {
            started_once = true;
        }
        if synced && !started_once {
            match host.start(percent) {
                Ok(status) => {
                    started_once = true;
                    log::info!(
                        "In sync with the network. Mining started: {} threads at {}% of the machine.",
                        status.threads,
                        status.percent
                    );
                }
                Err(err) => log::warn!("mining could not start: {err}"),
            }
        } else if !synced && !started_once {
            waited += 1;
            if waited % 12 == 1 {
                log::info!("Waiting for the sync to catch up before mining.");
            }
        }
        // The wallet may have stopped or resized the miner over RPC; report
        // whatever is actually running, once a minute.
        if last_report.elapsed() >= Duration::from_secs(60) {
            last_report = std::time::Instant::now();
            let status = host.status();
            if status.mining {
                log::info!(
                    "mining: {} on {} threads ({}% of the machine), {} blocks found, {} accepted, {} rejected",
                    crate::miner::format_hashrate(status.hashrate),
                    status.threads,
                    status.percent,
                    status.blocks_found,
                    status.blocks_accepted,
                    status.blocks_rejected
                );
            } else if started_once {
                log::info!("miner idle (stopped from a wallet); 'mine start' there sets it going again");
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    let status = host.stop();
    log::info!("Mining stopped: {} blocks found this run, {} accepted.", status.blocks_found, status.blocks_accepted);
    match node {
        Node::Own(node) => {
            node.stop().await?;
            log::info!("Node stopped.");
        }
        Node::Remote(client) => {
            client.disconnect().await.ok();
            log::info!("Disconnected from the node.");
        }
        Node::Grpc(client) => {
            client.disconnect().await.ok();
            log::info!("Disconnected from the node.");
        }
    }
    Ok(())
}
