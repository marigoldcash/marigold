//! A full node inside the wallet.
//!
//! Compiled only with the `embedded-node` feature, and started only when the
//! user asks. The wallet is handed the node's `RpcCoreService` directly as its
//! `RpcApi` — there is no socket, no port, no TLS and no wRPC serialisation
//! between them, because they are the same process. `RpcCtl::signal_open` is
//! what tells the wallet it is connected; nothing else about the wallet knows
//! or cares that the other end is in-process.
//!
//! Why anyone would want it: whoever runs a node sees the address you connect
//! from and which notes your wallet asks after. The chain never records that,
//! so a node you do not control is the one place that linkage exists. Running
//! your own is the only way to close it.

use crate::imports::*;
use kaspa_rpc_core::api::ctl::RpcCtl;
use kaspa_wallet_core::rpc::Rpc;
use kaspa_core::core::Core;
use kaspad_lib::args::Args;
use kaspad_lib::daemon::{Runtime, create_core_with_runtime};
use kaspa_core::signals::Shutdown;
use std::path::Path;
use std::sync::Mutex;
use std::thread::JoinHandle;

/// Where an embedded node keeps its chain data: beside the wallet's own files,
/// so a person who deletes their Marigold directory removes both and is not
/// left with seven gigabytes they cannot account for.
pub fn default_appdir(network_id: NetworkId) -> Result<std::path::PathBuf> {
    let base = workflow_store::fs::resolve_path(kaspa_wallet_core::storage::local::default_storage_folder())
        .map_err(|err| Error::custom(format!("cannot resolve the wallet folder: {err}")))?;
    Ok(base.join(format!("node-{network_id}")))
}

/// A running in-process node. Dropping this does NOT stop it — call
/// [`EmbeddedNode::stop`], which signals the core and joins its workers.
pub struct EmbeddedNode {
    /// Held for the node's lifetime. Two nodes on one data directory is a
    /// corrupted database: rocksdb has its own LOCK and would refuse the
    /// second, but from inside a background thread, as a panic that takes the
    /// wallet with it. Refusing here turns that into a sentence.
    _lock: std::fs::File,
    core: Arc<Core>,
    workers: Mutex<Option<Vec<JoinHandle<()>>>>,
    ctl: RpcCtl,
}

impl EmbeddedNode {
    /// Start a node for `network_id`, storing its data under `appdir`, and
    /// return it along with the `Rpc` the wallet should bind to.
    ///
    /// Blocking work — creating the core opens rocksdb, which on an existing
    /// datadir is seconds and on a fresh one is quick but then syncs for a
    /// while. Callers run this off the terminal's task so the prompt keeps
    /// responding.
    pub fn start(network_id: NetworkId, appdir: &Path) -> Result<(Arc<Self>, Rpc)> {
        // The p2p listener panics from inside a tokio worker if the port is
        // taken, which aborts the whole process — wallet included. Someone
        // already running marigoldd on this machine is the ordinary way to hit
        // that, so check first and refuse politely.
        let p2p_port = network_id.default_p2p_port();
        if std::net::TcpListener::bind(("0.0.0.0", p2p_port)).is_err() {
            return Err(Error::custom(format!(
                "port {p2p_port} is already in use — something else is running a Marigold node on this machine. \
                 Stop it, or 'connect 127.0.0.1:{}' to use it instead.",
                network_id.default_borsh_rpc_port()
            )));
        }

        let mut args = Args {
            appdir: Some(appdir.to_string_lossy().to_string()),
            utxoindex: true,
            // The node writes no log files of its own: without its logger
            // installed there is no file appender to write them, and its
            // records reach the wallet's logger instead.
            no_log_files: true,
            // No gRPC listener. The wallet holds the RpcCoreService directly,
            // so the only thing a socket would add is a port to conflict with
            // and an attack surface on a machine holding note keys.
            disable_grpc: true,
            // No UPnP. Asking the router to forward a port is a reasonable
            // thing for a server to do and a rude thing for a wallet to do
            // unasked, and it fails noisily where there is no UPnP router.
            disable_upnp: true,
            // The wallet needs the UTXO index to see ledger balance at all.
            // Everything else stays at kaspad's defaults, deliberately: this is
            // an ordinary node, not a special one, and the fewer knobs the
            // wallet invents the fewer ways it can differ from marigoldd.
            ..Default::default()
        };
        match network_id.network_type() {
            NetworkType::Testnet => {
                args.testnet = true;
                args.testnet_suffix = network_id.suffix().unwrap_or(10);
            }
            NetworkType::Devnet => args.devnet = true,
            NetworkType::Simnet => args.simnet = true,
            NetworkType::Mainnet => {}
        }

        // marigoldd's main() raises the soft file-descriptor limit before
        // computing its budget, and a node wants far more handles than a shell
        // hands out by default. Skipping it left the node with whatever the
        // terminal happened to allow — commonly 1024, against a daemon that
        // asks for far more.
        if let Err(err) = kaspa_utils::fd_budget::try_set_fd_limit(kaspad_lib::daemon::DESIRED_DAEMON_SOFT_FD_LIMIT) {
            log::warn!("could not raise the file descriptor limit for the node: {err}");
        }

        // Same budget arithmetic as marigoldd's own main(): whatever the
        // process is allowed, less what the node's own listeners will take.
        let fd_total_budget = kaspa_utils::fd_budget::limit()
            - args.rpc_max_clients as i32
            - args.inbound_limit as i32
            - args.outbound_target as i32;

        // Not `create_core`: that builds a Runtime via `Runtime::from_args`,
        // which installs a global logger and panics with SetLoggerError when
        // one already exists — and the wallet installs one at startup.
        // Claim the data directory before anything opens it.
        use fs4::fs_std::FileExt;
        std::fs::create_dir_all(appdir).map_err(|err| Error::custom(format!("cannot create {}: {err}", appdir.display())))?;
        let lock_path = appdir.join("node.lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|err| Error::custom(format!("cannot create the node lock file: {err}")))?;
        lock.try_lock_exclusive().map_err(|_| {
            Error::custom(
                "another Marigold wallet is already running a node on this data directory.                  Use that one, or stop it first — two nodes sharing a database would corrupt it.",
            )
        })?;

        let runtime = Runtime::from_args_without_logger(&args);
        // The node logs at INFO once a second. The wallet talks to its user
        // through the terminal, not the log crate, so clamping the global level
        // silences the node without costing the wallet anything a person sees.
        // Both levers, because they gate different paths: workflow_log for the
        // wallet's own macros, and the `log` facade for everything inside the
        // node. Setting only the first left "Accepted N blocks" scrolling past
        // once a second.
        kaspa_core::log::set_log_level(log::LevelFilter::Warn);
        log::set_max_level(log::LevelFilter::Warn);

        let (core, rpc_service) = create_core_with_runtime(&runtime, &args, fd_total_budget);
        let workers = core.start();

        let ctl = RpcCtl::new();
        let rpc = Rpc::new(rpc_service, ctl.clone());
        let node = Arc::new(Self { _lock: lock, core, workers: Mutex::new(Some(workers)), ctl });
        Ok((node, rpc))
    }

    /// Tell the wallet the node is up. Separate from [`Self::start`] because
    /// the wallet must be bound to the rpc BEFORE it is told to consider itself
    /// connected, or it processes the connect event with no api to call.
    pub async fn signal_connected(&self) -> Result<()> {
        self.ctl.signal_open().await.map_err(|err| Error::custom(format!("embedded node: {err}")))?;
        Ok(())
    }

    /// Stop the node and wait for its threads. Signals the wallet closed first,
    /// so nothing tries to make an RPC call into a core that is shutting down.
    pub async fn stop(&self) -> Result<()> {
        self.ctl.signal_close().await.ok();
        self.core.shutdown();
        if let Some(workers) = self.workers.lock().unwrap().take() {
            self.core.join(workers);
        }
        Ok(())
    }
}
