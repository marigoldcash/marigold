//! The wallet as a service (FORK-PLAN P8.0h): `marigold-cli serve <wallet>`
//! keeps a wallet open with no terminal, syncing its own copy of the network
//! (or using a node already running), optionally mining, and — when
//! `mobile telegram <token>` has been run — answering the person's own
//! Telegram bot. Hot posture: the wallet's secret stays in memory, the way
//! the auto-mint already holds it. Foreground, stdout, SIGTERM.

use crate::miner::MinerHost;
use crate::result::Result;
use crate::telegram::TelegramConfig;
use kaspa_consensus_core::network::NetworkId;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_core::signals::{Shutdown, Signals};
use kaspa_wallet_core::account::notepool::{self, BearerNote, Handover, HandoverSelection, BEARER_NOTE_PREFIX, HANDOVER_PREFIX};
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::rpc::DynRpcApi;
use kaspa_wallet_core::storage::local::journal::{Journal, JournalEntry};
use kaspa_wallet_core::storage::NoteStatus;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;
use kaspa_wallet_core::wallet::Wallet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use futures::TryStreamExt;

pub const USAGE: &str = "usage: marigold-cli serve <wallet> [--password-file <path>] [--node <url>] [--mine <percent>] [--network <id>]

  <wallet>                the wallet's name, as 'wallet list' shows it
  --password-file <path>  the wallet's password, first line of the file
                          (or the MARIGOLD_WALLET_PASSWORD environment variable)
  --node <url>            use a node already running (grpc://127.0.0.1:26210 or
                          ws://127.0.0.1:27210) instead of syncing one here
  --mine <percent>        also mine, to this wallet's own address
  --network <id>          mainnet, testnet-10, ... (default: the wallet's setting)

Runs in the foreground and logs to stdout; stop it with Ctrl-C or SIGTERM.
With 'mobile telegram <token>' set up, the wallet answers your Telegram bot.";

struct Stop(Arc<AtomicBool>);

impl Shutdown for Stop {
    fn shutdown(self: &Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Options {
    wallet: String,
    password: Secret,
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
            "--mine" => mine = Some(value("a percentage")?.trim_end_matches('%').parse::<u32>().ok().filter(|p| (1..=100).contains(p)).ok_or("--mine takes 1-100")?),
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
        Some(path) => std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))?.lines().next().unwrap_or("").to_string(),
        None => std::env::var("MARIGOLD_WALLET_PASSWORD").unwrap_or_default(),
    };
    if password.is_empty() {
        return Err("the wallet's password is needed: --password-file <path>, or MARIGOLD_WALLET_PASSWORD in the environment".to_string());
    }
    Ok(Options { wallet, password: Secret::from(password), node, mine, network })
}

/// What was paid, for the bot to say.
pub struct Paid {
    pub code: String,
    pub value_petals: u64,
    pub fee_petals: u64,
    pub notes: usize,
}

enum Node {
    Own(Arc<crate::embedded::EmbeddedNode>),
    Wrpc(Arc<kaspa_wrpc_client::KaspaRpcClient>),
    Grpc(Arc<kaspa_grpc_client::GrpcClient>),
}

/// The open wallet and everything the bot may do with it.
pub struct WalletService {
    wallet: Arc<Wallet>,
    secret: Secret,
    network_id: NetworkId,
    journal: Journal,
    miner: Option<Arc<MinerHost>>,
    rpc: Arc<DynRpcApi>,
    /// (UTC day, petals paid out that day) — the daily limit's memory.
    spent: Mutex<(u64, u64)>,
    started: std::time::Instant,
}

fn utc_day() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

impl WalletService {
    pub fn ticker(&self) -> &'static str {
        kaspa_wallet_core::utils::kaspa_suffix(&self.network_id.network_type())
    }

    fn record(&self, kind: &str, petals: u64, stamp: u64, detail: &str, tx: &str) {
        let _ = self.journal.append(&JournalEntry::now(kind, petals, stamp, detail, tx));
    }

    pub fn spend_allowed(&self, petals: u64, limit: u64) -> std::result::Result<(), String> {
        let (day, spent) = *self.spent.lock().unwrap();
        let spent = if day == utc_day() { spent } else { 0 };
        if spent + petals > limit {
            return Err(format!(
                "That would pass today's limit of {} {} ({} paid so far today). The limit resets at midnight UTC; 'mobile telegram limit' in the wallet changes it.",
                sompi_to_kaspa_string(limit),
                self.ticker(),
                sompi_to_kaspa_string(spent)
            ));
        }
        Ok(())
    }

    pub fn note_spent(&self, petals: u64) {
        let mut guard = self.spent.lock().unwrap();
        let today = utc_day();
        if guard.0 != today {
            *guard = (today, 0);
        }
        guard.1 += petals;
    }

    pub async fn balance_text(&self) -> String {
        let mut lines = Vec::new();
        let mut total = 0u64;
        let mut counts = [0usize; DENOMINATION_PETALS.len()];
        if let Ok(store) = self.wallet.store().as_note_key_store() {
            if let Ok(mut stream) = store.iter().await {
                while let Ok(Some(info)) = stream.try_next().await {
                    if info.status == NoteStatus::Active {
                        counts[info.d as usize] += 1;
                        total += DENOMINATION_PETALS[info.d as usize];
                    }
                }
            }
        }
        lines.push(format!("Notes: {} {}", sompi_to_kaspa_string(total), self.ticker()));
        for (i, n) in counts.iter().enumerate().rev() {
            if *n > 0 {
                lines.push(format!("  {} × {}", n, sompi_to_kaspa_string(DENOMINATION_PETALS[i])));
            }
        }
        if let Ok(account) = self.wallet.account() {
            if let Some(balance) = account.balance() {
                let pending = balance.pending;
                lines.push(format!(
                    "Ledger: {} {}{}",
                    sompi_to_kaspa_string(balance.mature),
                    self.ticker(),
                    if pending > 0 { format!(" ({} pending)", sompi_to_kaspa_string(pending)) } else { String::new() }
                ));
            }
        }
        if !self.wallet.is_connected() {
            lines.push("Not connected to the network right now; these are the last known figures.".to_string());
        }
        lines.join("\n")
    }

    pub async fn pay(&self, petals: u64) -> std::result::Result<Paid, String> {
        let result = notepool::hand_over(&self.wallet, self.secret.clone(), HandoverSelection::Amount(petals)).await.map_err(|e| e.to_string())?;
        let code = result.handover.to_text();
        self.record("paid", result.value_petals, result.stamp_petals + result.transfer.fee_petals, "code handed over (telegram)", &result.transfer.transaction_id.to_string());
        Ok(Paid { code, value_petals: result.value_petals, fee_petals: result.transfer.fee_petals, notes: result.handover.notes.len().saturating_sub(1) })
    }

    pub async fn receive(&self, code: &str) -> std::result::Result<String, String> {
        if code.starts_with(HANDOVER_PREFIX) {
            let handover = Handover::from_text(code).map_err(|e| e.to_string())?;
            let result = notepool::receive_handover(&self.wallet, self.secret.clone(), handover).await.map_err(|e| e.to_string())?;
            let stamp = result.notes.iter().any(|(_, d)| *d == kaspa_consensus_core::notepool::DenominationTag::D0_01);
            let value = if stamp { result.value_petals - DENOMINATION_PETALS[0] } else { result.value_petals };
            self.record("received", value, 0, "code (telegram)", &result.rotation.transaction_id.to_string());
            Ok(format!("Received {} {} in {} note(s). Made yours alone with the payer's stamp.", sompi_to_kaspa_string(value), self.ticker(), result.notes.len() - usize::from(stamp)))
        } else if code.starts_with(BEARER_NOTE_PREFIX) {
            let bearer = BearerNote::from_text(code).map_err(|e| e.to_string())?;
            let result = notepool::bearer_import(&self.wallet, self.secret.clone(), bearer).await.map_err(|e| e.to_string())?;
            let value = DENOMINATION_PETALS[bearer.d as usize];
            self.record("received", value, result.rotation.fee_petals, "note (telegram)", &result.rotation.transaction_id.to_string());
            Ok(format!("Received {} {} (fee {}).", sompi_to_kaspa_string(value), self.ticker(), sompi_to_kaspa_string(result.rotation.fee_petals)))
        } else {
            Err("that is not a Marigold code (marigoldpay: or marigoldnote:)".to_string())
        }
    }

    pub async fn request(&self, petals: Option<u64>) -> std::result::Result<String, String> {
        let request = notepool::create_payment_request(&self.wallet, &self.secret, petals).await.map_err(|e| e.to_string())?;
        Ok(request.to_text())
    }

    /// Watch a request until it is paid, up to `timeout`; the line to say if it was.
    pub async fn await_request(&self, code: &str, timeout: Duration) -> std::result::Result<Option<String>, String> {
        let request = notepool::PaymentRequest::from_text(code).map_err(|e| e.to_string())?;
        match notepool::await_payment_request(&self.wallet, &self.secret, request.pk, timeout).await {
            Ok(claimed) => {
                self.record("received", claimed.total_petals, 0, "request (telegram)", "");
                Ok(Some(format!("Paid: {} {} arrived in {} note(s).", sompi_to_kaspa_string(claimed.total_petals), self.ticker(), claimed.notes.len())))
            }
            Err(_) => Ok(None),
        }
    }

    pub fn history_text(&self, n: usize) -> String {
        let entries = match self.journal.read() {
            Ok(e) => e,
            Err(_) => return "No history yet.".to_string(),
        };
        if entries.is_empty() {
            return "No history yet.".to_string();
        }
        entries
            .iter()
            .rev()
            .take(n)
            .map(|e| {
                let when = chrono::DateTime::<chrono::Utc>::from_timestamp(e.at as i64, 0).map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                format!("{when}  {:<9} {:>12}  {}", e.kind, sompi_to_kaspa_string(e.petals), e.detail)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub async fn status_text(&self) -> String {
        let mut lines = Vec::new();
        match self.rpc.get_server_info().await {
            Ok(info) => lines.push(format!("Network: {} ({} {})", if info.is_synced { "in sync" } else { "catching up" }, info.network_id, info.server_version)),
            Err(_) => lines.push("Network: not reachable".to_string()),
        }
        lines.push(format!("Wallet: {} for {}", if self.wallet.is_connected() { "connected" } else { "not connected" }, crate::cli::humanised_minutes(self.started.elapsed().as_secs() / 60)));
        match &self.miner {
            Some(host) => {
                let s = host.status();
                if s.mining {
                    lines.push(format!("Miner: {} on {} threads ({}%), {} blocks found, {} accepted", crate::miner::format_hashrate(s.hashrate), s.threads, s.percent, s.blocks_found, s.blocks_accepted));
                } else {
                    lines.push("Miner: idle (/mine start)".to_string());
                }
            }
            None => lines.push("Miner: none (start the service with --mine <percent>)".to_string()),
        }
        lines.join("\n")
    }

    pub async fn mine(&self, what: Option<&str>) -> String {
        let Some(host) = &self.miner else { return "This service was started without --mine; restart it with --mine <percent> to mine.".to_string() };
        match what {
            Some("start") => match host.start(host.miner().map(|m| m.percent()).unwrap_or(50)) {
                Ok(s) => format!("Mining: {} threads ({}% of the machine).", s.threads, s.percent),
                Err(e) => format!("Could not start: {e}"),
            },
            Some("stop") => {
                host.stop();
                "Mining stopped.".to_string()
            }
            _ => {
                let s = host.status();
                if s.mining {
                    format!("Mining: {} on {} threads ({}%), {} blocks found, {} accepted.", crate::miner::format_hashrate(s.hashrate), s.threads, s.percent, s.blocks_found, s.blocks_accepted)
                } else {
                    "Not mining. /mine start".to_string()
                }
            }
        }
    }
}

pub async fn serve(args: Vec<String>) -> Result<()> {
    let options = match parse(&args) {
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

    // The wallet's settings say which network and which folder; read them the
    // way the terminal wallet does, before anything opens.
    let probe = Wallet::try_with_rpc(None, Wallet::local_store()?, None)?;
    probe.load_settings().await.ok();
    let folder: String = probe.settings().get::<String>(WalletSettings::Folder).unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
    let network_id = match options.network {
        Some(id) => id,
        None => probe.settings().get::<String>(WalletSettings::Network).and_then(|s| s.parse::<NetworkId>().ok()).unwrap_or(NetworkId::with_suffix(kaspa_consensus_core::network::NetworkType::Testnet, 10)),
    };
    drop(probe);

    let telegram_path = TelegramConfig::path(&folder, &options.wallet);
    let telegram = TelegramConfig::load(&telegram_path);

    log::info!("Marigold wallet service {}: wallet '{}' on {network_id}", env!("CARGO_PKG_VERSION"), options.wallet);
    let miner_host = options.mine.map(|_| MinerHost::new_unbound(shutdown.clone()));
    let (node, rpc, rpc_ctl): (Node, Arc<DynRpcApi>, kaspa_wallet_core::rpc::Rpc) = match options.node.clone() {
        Some(url) if url.starts_with("grpc://") => {
            let client = Arc::new(
                kaspa_grpc_client::GrpcClient::connect_with_args(kaspa_rpc_core::notify::mode::NotificationMode::MultiListeners, url.clone(), None, true, None, false, None, Default::default())
                    .await
                    .map_err(|err| crate::error::Error::custom(format!("cannot reach {url}: {err}")))?,
            );
            log::info!("Using the node at {url}");
            let rpc: Arc<DynRpcApi> = client.clone();
            // A gRPC client has no connection control of its own for the
            // wallet to listen to: make one and open it once the wallet runs.
            let ctl = kaspa_wallet_core::rpc::RpcCtl::new();
            (Node::Grpc(client.clone()), rpc.clone(), kaspa_wallet_core::rpc::Rpc::new(rpc, ctl))
        }
        Some(url) => {
            use kaspa_wallet_core::rpc::{ConnectOptions, ConnectStrategy, WrpcEncoding};
            let client = Arc::new(
                kaspa_wrpc_client::KaspaRpcClient::new(WrpcEncoding::Borsh, Some(&url), None, Some(network_id), None)
                    .map_err(|err| crate::error::Error::custom(format!("{url}: {err}")))?,
            );
            let options = ConnectOptions { block_async_connect: true, strategy: ConnectStrategy::Retry, url: Some(url.clone()), ..Default::default() };
            log::info!("Using the node at {url}");
            client.connect(Some(options)).await.map_err(|err| crate::error::Error::custom(format!("cannot reach {url}: {err}")))?;
            let rpc: Arc<DynRpcApi> = client.clone();
            (Node::Wrpc(client.clone()), rpc.clone(), kaspa_wallet_core::rpc::Rpc::new(rpc, client.ctl().clone()))
        }
        None => {
            let appdir = crate::embedded::default_appdir(network_id)?;
            let control = miner_host.clone().map(|h| h as Arc<dyn kaspa_rpc_core::api::miner::MinerControl>);
            let (node, rpc) = crate::embedded::EmbeddedNode::start_with(network_id, &appdir, control)?;
            log::info!("Syncing the network here");
            (Node::Own(node), rpc.rpc_api().clone(), rpc)
        }
    };

    let rpc_ctl_open = rpc_ctl.rpc_ctl().clone();
    let wallet = Arc::new(Wallet::try_with_rpc(Some(rpc_ctl), Wallet::local_store()?, Some(network_id))?);
    wallet.load_settings().await.ok();
    wallet.store().set_storage_folder(&folder)?;
    wallet.clone().start().await?;
    match &node {
        Node::Own(node) => node.signal_connected().await?,
        Node::Grpc(_) => {
            rpc_ctl_open.signal_open().await.map_err(|e| crate::error::Error::custom(format!("rpc: {e}")))?;
        }
        Node::Wrpc(_) => {}
    }
    let descriptors = wallet.clone().wallet_open(options.password.clone(), Some(options.wallet.clone()), true, false).await?;
    if let Some(descriptors) = descriptors {
        let ids: Vec<_> = descriptors.iter().map(|d| d.account_id).collect();
        if !ids.is_empty() {
            wallet.clone().accounts_activate(Some(ids)).await?;
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

    let service = Arc::new(WalletService {
        wallet: wallet.clone(),
        secret: options.password.clone(),
        network_id,
        journal: Journal::new(&folder, &options.wallet),
        miner: miner_host.clone().filter(|h| h.address_bound()),
        rpc: rpc.clone(),
        spent: Mutex::new((utc_day(), 0)),
        started: std::time::Instant::now(),
    });

    match telegram {
        Some(cfg) => {
            tokio::spawn(crate::telegram::run_bot(service.clone(), telegram_path.clone(), cfg));
        }
        None => log::info!("No Telegram bot: run 'mobile telegram <token>' in the wallet to pair one"),
    }

    let mut started_once = false;
    let mut last_report = std::time::Instant::now();
    while !shutdown.load(Ordering::SeqCst) {
        let synced = matches!(rpc.get_server_info().await, Ok(info) if info.is_synced);
        if let (Some(host), Some(percent)) = (&service.miner, options.mine) {
            if synced && !started_once && host.miner().is_none() {
                match host.start(percent) {
                    Ok(s) => {
                        started_once = true;
                        log::info!("In sync. Mining started: {} threads at {}%.", s.threads, s.percent);
                    }
                    Err(e) => log::warn!("mining could not start: {e}"),
                }
            }
        }
        if last_report.elapsed() >= Duration::from_secs(300) {
            last_report = std::time::Instant::now();
            log::info!("{}", service.status_text().await.replace('\n', "; "));
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    log::info!("Stopping.");
    if let Some(host) = &service.miner {
        host.stop();
    }
    wallet.clone().stop().await.ok();
    match node {
        Node::Own(node) => {
            node.stop().await?;
            log::info!("Node stopped.");
        }
        Node::Wrpc(client) => {
            client.disconnect().await.ok();
        }
        Node::Grpc(client) => {
            client.disconnect().await.ok();
        }
    }
    Ok(())
}
