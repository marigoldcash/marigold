//! The miner as a service (FORK-PLAN P8.3c).
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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const USAGE: &str = "usage: marigold-cli mine-to <address> [<percent>] [--network <id>]

  <address>        where the rewards go — an address from 'address' in your wallet
  <percent>        share of this machine to use, 1-100 (default 50); '--cpu 50' means the same
  --network <id>   mainnet, testnet-10, ... (default: the one the address belongs to)

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
}

fn parse(args: &[String]) -> std::result::Result<Options, String> {
    let mut address = None;
    let mut percent = 50u32;
    let mut network = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--cpu" => {
                let value = inline.or_else(|| iter.next().cloned()).ok_or("--cpu needs a percentage")?;
                percent = value.trim_end_matches('%').parse::<u32>().ok().filter(|p| (1..=100).contains(p)).ok_or_else(|| format!("'{value}' is not a percentage between 1 and 100"))?;
            }
            "--network" => {
                let value = inline.or_else(|| iter.next().cloned()).ok_or("--network needs a network id")?;
                network = Some(value.parse::<NetworkId>().map_err(|err| format!("'{value}' is not a network: {err}"))?);
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'\n\n{USAGE}")),
            // A bare number is the share of the machine: 'mine-to <address> 50'.
            other if other.trim_end_matches('%').chars().all(|c| c.is_ascii_digit()) && !other.is_empty() => {
                percent = other.trim_end_matches('%').parse::<u32>().ok().filter(|p| (1..=100).contains(p)).ok_or_else(|| format!("'{other}' is not a percentage between 1 and 100"))?;
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
    Ok(Options { address, percent, network_id })
}

pub async fn mine_to(args: Vec<String>) -> Result<()> {
    let options = match parse(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let Options { address, percent, network_id } = options;

    // Plain lines on stdout, one per record, for journald or a log file.
    kaspa_core::log::init_logger(None, "info");

    let shutdown = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(Stop(shutdown.clone()));
    Arc::new(Signals::new(&stop)).init();

    let appdir = crate::embedded::default_appdir(network_id)?;
    let host = MinerHost::new(address.clone(), shutdown.clone());
    let control: Arc<dyn kaspa_rpc_core::api::miner::MinerControl> = host.clone();
    let (node, rpc) = crate::embedded::EmbeddedNode::start_with(network_id, &appdir, Some(control))?;
    host.bind(rpc.rpc_api().clone());

    let port = network_id.default_borsh_rpc_port();
    log::info!(
        "Marigold miner {}: paying to {address}, {percent}% of this machine ({} of {} cores), {network_id}",
        env!("CARGO_PKG_VERSION"),
        crate::miner::threads_for_percent(percent),
        crate::miner::cores()
    );
    log::info!("A wallet on this machine reaches this miner at 127.0.0.1:{port} — 'connect' finds it by itself.");

    // Mine only on a synced copy: templates built on a chain the node has not
    // finished reading make blocks nobody accepts.
    // Started here once, when the sync first catches up. From then on a
    // wallet owns start and stop: a miner stopped with 'mine stop' stays
    // idle, and is not quietly restarted at the next tick.
    let mut waited = 0u64;
    let mut started_once = false;
    let mut last_report = std::time::Instant::now();
    while !shutdown.load(Ordering::SeqCst) {
        let synced = matches!(rpc.rpc_api().get_server_info().await, Ok(info) if info.is_synced);
        // A wallet may have started it already, before the sync caught up.
        if host.miner().is_some() {
            started_once = true;
        }
        if synced && !started_once {
            match host.start(percent) {
                Ok(status) => {
                    started_once = true;
                    log::info!("In sync with the network. Mining started: {} threads at {}% of the machine.", status.threads, status.percent);
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
    node.stop().await?;
    log::info!("Node stopped.");
    Ok(())
}
