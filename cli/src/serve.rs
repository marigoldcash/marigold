//! The wallet as a service (PLAN P8.0h): `marigold-cli serve <wallet>`
//! keeps a wallet open with no terminal, syncing its own copy of the network
//! (or using a node already running), optionally mining, and — when
//! `telegram link <token>` has been run — answering the person's own
//! Telegram bot. Hot posture: the wallet's secret stays in memory, the way
//! the auto-mint already holds it. Foreground, stdout, SIGTERM.

use crate::miner::MinerHost;
use crate::result::Result;
use crate::telegram::TelegramConfig;
use kaspa_consensus_core::network::NetworkId;
use kaspa_core::signals::{Shutdown, Signals};
use kaspa_wallet_core::account::notepool;
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_service::providers::{MinerHostApi, Providers};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// The service and the opening of a session moved to the wallet service crate
// (PH-M1c); the paths `kaspa_cli_lib::serve::{WalletService, Session, ...}` stay.
pub use kaspa_wallet_service::service::{LocalMiner, Paid, Say, WalletService};
pub use kaspa_wallet_service::session::{Session, SessionOptions};

pub const USAGE: &str =
    "usage: marigold-cli serve <wallet> [--password-file <path>] [--node <url>] [--mine <percent>] [--network <id>]

  <wallet>                the wallet's name, as 'wallet list' shows it
  --password-file <path>  the wallet's password, first line of the file
                          (or the MARIGOLD_WALLET_PASSWORD environment variable)
  --node <url>            use a node already running (grpc://127.0.0.1:26210 or
                          ws://127.0.0.1:27210) instead of syncing one here
  --mine <percent>        also mine, to this wallet's own address
  --network <id>          mainnet, testnet-10, ... (default: the wallet's setting)

Runs in the foreground and logs to stdout; stop it with Ctrl-C or SIGTERM.
With 'telegram link <token>' set up, the wallet answers your Telegram bot.";

struct Stop(Arc<AtomicBool>);

impl Shutdown for Stop {
    fn shutdown(self: &Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Options {
    wallet: String,
    /// Read once from the file or the environment, then held as two halves.
    password: kaspa_wallet_keys::guarded::Guarded,
    node: Option<String>,
    mine: Option<u32>,
    network: Option<NetworkId>,
}

fn parse(args: &[String]) -> std::result::Result<Options, String> {
    let mut wallet = None;
    let mut password_file = None;
    let mut node = None;
    let mut mine = None;
    let mut network = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = |what: &str| inline.clone().or_else(|| iter.next().cloned()).ok_or(format!("{flag} needs {what}"));
        match flag {
            "--password-file" => password_file = Some(value("a path")?),
            "--node" => {
                let v = value("a url")?;
                node = Some(if v.contains("://") { v } else { format!("ws://{v}") });
            }
            "--mine" => {
                mine = Some(
                    value("a percentage")?
                        .trim_end_matches('%')
                        .parse::<u32>()
                        .ok()
                        .filter(|p| (1..=100).contains(p))
                        .ok_or("--mine takes 1-100")?,
                )
            }
            "--network" => network = Some(value("a network id")?.parse::<NetworkId>().map_err(|e| format!("--network: {e}"))?),
            "--help" | "-h" => return Err(USAGE.to_string()),
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'\n\n{USAGE}")),
            other => {
                if wallet.is_some() {
                    return Err(format!("one wallet only\n\n{USAGE}"));
                }
                wallet = Some(other.to_string());
            }
        }
    }
    let wallet = wallet.ok_or_else(|| format!("which wallet?\n\n{USAGE}"))?;
    let password = match password_file {
        Some(path) => {
            std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))?.lines().next().unwrap_or("").to_string()
        }
        None => std::env::var("MARIGOLD_WALLET_PASSWORD").unwrap_or_default(),
    };
    if password.is_empty() {
        return Err(
            "the wallet's password is needed: --password-file <path>, or MARIGOLD_WALLET_PASSWORD in the environment".to_string()
        );
    }
    Ok(Options { wallet, password: kaspa_wallet_keys::guarded::Guarded::from_secret(Secret::from(password)), node, mine, network })
}

/// What this program lends the service: the CPU miner and the node inside the
/// process (the phone apps lend neither).
pub fn providers() -> Providers {
    Providers {
        miner: Some(Arc::new(|shutdown| MinerHost::new_unbound(shutdown) as Arc<dyn MinerHostApi>)),
        node: Some(Arc::new(crate::embedded::Starter)),
    }
}

/// Opens the wallet and connects it, the way `serve` does; `say` receives
/// the service's one-line remarks.
pub async fn open_session(options: &SessionOptions, shutdown: Arc<AtomicBool>, say: Say) -> Result<Session> {
    Ok(kaspa_wallet_service::session::open_session(options, shutdown, say, &providers()).await?)
}

pub async fn serve(args: Vec<String>) -> Result<()> {
    let mut options = match parse(&args) {
        Ok(o) => o,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    kaspa_core::log::init_logger(None, "info");
    let shutdown = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(Stop(shutdown.clone()));
    Arc::new(Signals::new(&stop)).init();

    let session = open_session(
        &SessionOptions {
            wallet: options.wallet.clone(),
            password: options.password.reveal(),
            node: options.node.clone(),
            mine: options.mine,
            network: options.network,
            folder: None,
            origin: "telegram",
        },
        shutdown.clone(),
        Arc::new(|line: String| log::info!("{line}")),
    )
    .await?;
    let service = session.service.clone();
    let wallet = session.wallet.clone();
    let rpc = session.rpc.clone();
    let folder = session.folder.clone();
    let telegram_path = TelegramConfig::path(&folder, &options.wallet);
    let telegram = TelegramConfig::load(&telegram_path);

    match telegram {
        Some(cfg) => {
            tokio::spawn(crate::telegram::run_bot(service.clone(), telegram_path.clone(), cfg));
        }
        None => log::info!("No Telegram bot: run 'telegram link <token>' in the wallet to pair one"),
    }

    let mut started_once = false;
    let mut last_report = std::time::Instant::now();
    while !shutdown.load(Ordering::SeqCst) {
        let synced = matches!(rpc.get_server_info().await, Ok(info) if info.is_synced);
        if let (Some(host), Some(percent)) = (service.miner_host(), options.mine)
            && synced
            && !started_once
            && host.mining().is_none()
        {
            match host.start(percent) {
                Ok(s) => {
                    started_once = true;
                    log::info!("In sync. Mining started: {} threads at {}%.", s.threads, s.percent);
                }
                Err(e) => log::warn!("mining could not start: {e}"),
            }
        }
        if last_report.elapsed() >= Duration::from_secs(300) {
            last_report = std::time::Instant::now();
            log::info!("{}", service.status_text().await.replace('\n', "; "));
            // Offers whose lock lapsed come back (P8.0g).
            match notepool::reclaim_lapsed(&wallet, options.password.reveal()).await {
                Ok(report) if !report.taken_back.is_empty() || !report.taken_by_receiver.is_empty() => {
                    log::info!(
                        "offers: {} note(s) taken back after their lock lapsed, {} taken by the receiver in time",
                        report.taken_back.len(),
                        report.taken_by_receiver.len()
                    );
                }
                Ok(_) => {}
                Err(e) => log::warn!("offers not checked: {e}"),
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    log::info!("Stopping.");
    if let Some(host) = service.miner_host() {
        host.stop();
    }
    wallet.clone().stop().await.ok();
    let own_node = session.own_node();
    session.stop_node().await?;
    if own_node {
        log::info!("Node stopped.");
    }
    Ok(())
}
