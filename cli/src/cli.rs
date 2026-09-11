use crate::error::Error;
use crate::helpers::*;
use crate::imports::*;
use crate::modules::miner::Miner;
#[cfg(not(feature = "embedded-node"))]
use crate::modules::node::Node;
use crate::notifier::{Notification, Notifier};
use crate::result::Result;
use kaspa_daemon::{DaemonEvent, DaemonKind, Daemons};
use kaspa_wallet_core::account::Account;
use kaspa_wallet_core::rpc::DynRpcApi;
#[cfg(feature = "embedded-node")]
use kaspa_wallet_core::rpc::Rpc;
use kaspa_wallet_core::storage::{IdT, PrvKeyDataInfo};
use kaspa_wrpc_client::{KaspaRpcClient, Resolver};
use workflow_core::channel::*;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use workflow_core::time::Instant;
use workflow_log::*;
pub use workflow_terminal::Event as TerminalEvent;
use workflow_terminal::*;
pub use workflow_terminal::{Options as TerminalOptions, TargetElement as TerminalTarget};

const NOTIFY: &str = "\x1B[2m⎟\x1B[0m";

pub struct Options {
    pub daemons: Option<Arc<Daemons>>,
    pub terminal: TerminalOptions,
}

impl Options {
    pub fn new(terminal_options: TerminalOptions, daemons: Option<Arc<Daemons>>) -> Self {
        Self { daemons, terminal: terminal_options }
    }
}

pub struct KaspaCli {
    term: Arc<Mutex<Option<Arc<Terminal>>>>,
    wallet: Arc<Wallet>,
    notifications_task_ctl: DuplexChannel,
    mute: Arc<AtomicBool>,
    flags: Flags,
    last_interaction: Arc<Mutex<Instant>>,
    daemons: Arc<Daemons>,
    handlers: Arc<HandlerCli>,
    shutdown: Arc<AtomicBool>,
    #[cfg(not(feature = "embedded-node"))]
    node: Mutex<Option<Arc<Node>>>,
    miner: Mutex<Option<Arc<Miner>>>,
    notifier: Notifier,
    sync_state: Mutex<Option<SyncState>>,
    /// Auto-mint state. The secret lives in memory only while a wallet is
    /// open and auto-mint is armed (a hot-wallet posture, entered knowingly);
    /// it is never written anywhere and is dropped on close/disarm.
    auto_secret: Mutex<Option<Secret>>,
    /// The in-process node, once started. Held here so `node stop` and wallet
    /// shutdown can reach it; `None` means we are talking to someone else's.
    #[cfg(feature = "embedded-node")]
    embedded_node: Mutex<Option<Arc<crate::embedded::EmbeddedNode>>>,
    /// Whether the wallet is actually *using* that node.
    ///
    /// Running and using it are no longer the same thing: a node that is still
    /// catching up is left running while the wallet talks to a public one. Any
    /// claim about who can see your notes has to key off this, not off whether
    /// a node exists.
    #[cfg(feature = "embedded-node")]
    embedded_node_adopted: Arc<AtomicBool>,
    /// The CPU miner, while it is running. Only ever set when the wallet is on
    /// its own node — see `start_mining`.
    #[cfg(feature = "embedded-node")]
    cpu_miner: Mutex<Option<Arc<crate::miner::Miner>>>,
    /// True while the UTXO set is being read in, which on a wallet that has
    /// been mined into is minutes of work with nothing to show for it.
    loading: Arc<AtomicBool>,
    auto_payment_secret: Mutex<Option<Secret>>,
    auto_threshold_petals: Arc<AtomicU64>,
    /// 0 disables auto-sweep; otherwise the UTXO count that triggers one.
    auto_sweep_utxos: Arc<AtomicU64>,
    auto_busy: Arc<AtomicBool>,
    auto_verbose: Arc<AtomicBool>,
    /// Set at `open`; the opening report + housekeeping run on the first
    /// balance event after it, which is the moment the wallet actually knows
    /// its coins. Doing it inline raced the wallet's own startup.
    open_housekeeping_pending: Arc<AtomicBool>,
    /// Notes + ledger, refreshed once a minute. The prompt shows what you
    /// hold, not the per-block churn of the ledger's plumbing.
    prompt_total_petals: Arc<AtomicU64>,
    prompt_total_valid: Arc<AtomicBool>,
    /// Widest balance segment rendered this session — the prompt pads to it
    /// so the command line never shifts under the user's fingers.
    prompt_balance_width: Arc<AtomicUsize>,
    /// Authenticator state for this session: when a code was last accepted,
    /// and which time steps have already been spent. Neither is written
    /// anywhere. A lock that survived a restart would have to live in a file
    /// that whoever is at the machine could simply delete, so it would buy
    /// nothing and imply something it could not deliver.
    otp_session: Mutex<OtpSession>,
}

/// See [`KaspaCli::otp_session`].
#[derive(Default)]
pub struct OtpSession {
    /// Unix seconds of the last accepted code, for the grace window.
    pub verified_at: Option<u64>,
    /// Time steps already used. A thirty-second code is good for thirty
    /// seconds to anyone who read it over a shoulder; spending it once closes
    /// that.
    pub used_steps: std::collections::HashSet<u64>,
}

impl From<&KaspaCli> for Arc<Terminal> {
    fn from(ctx: &KaspaCli) -> Arc<Terminal> {
        ctx.term()
    }
}

impl AsRef<KaspaCli> for KaspaCli {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl workflow_log::Sink for KaspaCli {
    fn write(&self, _target: Option<&str>, _level: Level, args: &std::fmt::Arguments<'_>) -> bool {
        if let Some(term) = self.try_term() {
            cfg_if! {
                if #[cfg(target_arch = "wasm32")] {
                    if _level == Level::Error {
                        term.writeln(style(args.to_string().crlf()).red().to_string());
                    }
                    false
                } else {
                    match _level {
                        Level::Error => {
                            term.writeln(style(args.to_string().crlf()).red().to_string());
                        },
                        _ => {
                            term.writeln(args.to_string());
                        }
                    }
                    true
                }
            }
        } else {
            false
        }
    }
}

impl KaspaCli {
    pub fn init() {
        cfg_if! {
            if #[cfg(not(target_arch = "wasm32"))] {
                init_panic_hook(||{
                    std::println!("halt");
                    1
                });
                // NOT kaspa_core::log::init_logger: that installs log4rs with a
                // console appender writing straight to stdout, terminating lines
                // with a bare LF. In a raw-mode terminal the cursor never returns
                // to column zero, so every line starts where the last one ended
                // and the node's output cascades diagonally down the screen
                // (founder report, 2026-09-07). Route it through the terminal
                // instead, which knows to emit CRLF and to redraw the prompt.
                crate::log_sink::install();
            } else {
                kaspa_core::log::set_log_level(LevelFilter::Info);
            }
        }

        workflow_log::set_colors_enabled(true);
    }

    pub async fn try_new_arc(options: Options) -> Result<Arc<Self>> {
        let wallet = Arc::new(Wallet::try_new(Wallet::local_store()?, Some(Resolver::default()), None)?);

        let kaspa_cli = Arc::new(KaspaCli {
            term: Arc::new(Mutex::new(None)),
            wallet,
            notifications_task_ctl: DuplexChannel::oneshot(),
            mute: Arc::new(AtomicBool::new(true)),
            flags: Flags::default(),
            last_interaction: Arc::new(Mutex::new(Instant::now())),
            handlers: Arc::new(HandlerCli::default()),
            daemons: options.daemons.unwrap_or_default(),
            shutdown: Arc::new(AtomicBool::new(false)),
            #[cfg(not(feature = "embedded-node"))]
            node: Mutex::new(None),
            miner: Mutex::new(None),
            notifier: Notifier::try_new()?,
            sync_state: Mutex::new(None),
            auto_secret: Mutex::new(None),
            #[cfg(feature = "embedded-node")]
            embedded_node: Mutex::new(None),
            #[cfg(feature = "embedded-node")]
            embedded_node_adopted: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "embedded-node")]
            cpu_miner: Mutex::new(None),
            loading: Arc::new(AtomicBool::new(false)),
            auto_payment_secret: Mutex::new(None),
            auto_threshold_petals: Arc::new(AtomicU64::new(0)),
            auto_sweep_utxos: Arc::new(AtomicU64::new(0)),
            auto_busy: Arc::new(AtomicBool::new(false)),
            auto_verbose: Arc::new(AtomicBool::new(false)),
            open_housekeeping_pending: Arc::new(AtomicBool::new(false)),
            prompt_total_petals: Arc::new(AtomicU64::new(0)),
            prompt_total_valid: Arc::new(AtomicBool::new(false)),
            prompt_balance_width: Arc::new(AtomicUsize::new(0)),
            otp_session: Mutex::new(OtpSession::default()),
        });

        let term = Arc::new(Terminal::try_new_with_options(kaspa_cli.clone(), options.terminal)?);
        term.init().await?;

        cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                kaspa_cli.init_panic_hook();
            }
        }

        Ok(kaspa_cli)
    }

    pub fn term(&self) -> Arc<Terminal> {
        self.term.lock().unwrap().as_ref().cloned().expect("WalletCli::term is not initialized")
    }

    pub fn try_term(&self) -> Option<Arc<Terminal>> {
        self.term.lock().unwrap().as_ref().cloned()
    }

    pub fn notifier(&self) -> &Notifier {
        &self.notifier
    }

    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    pub fn wallet(&self) -> Arc<Wallet> {
        self.wallet.clone()
    }

    pub fn is_connected(&self) -> bool {
        self.wallet.is_connected()
    }

    pub fn rpc_api(&self) -> Arc<DynRpcApi> {
        self.wallet.rpc_api().clone()
    }

    /// Start a node inside this process and point the wallet at it.
    ///
    /// The wallet is bound to the node's `RpcCoreService` directly — no socket,
    /// no port, nothing on the network between them. It must be bound BEFORE
    /// the ctl is signalled open, or it handles the connect event with no api
    /// to call.
    #[cfg(feature = "embedded-node")]
    pub async fn start_embedded_node(self: &Arc<Self>) -> Result<()> {
        let Some(rpc) = self.spawn_embedded_node().await? else {
            return Ok(());
        };
        self.adopt_embedded_node(rpc).await?;
        tprintln!(self, "Your node is running. It will catch up with the network in the background.");
        Ok(())
    }

    /// Start the node, but leave the wallet pointed wherever it is.
    ///
    /// Returns the node's `Rpc` so the caller can decide when — or whether —
    /// to hand the wallet over to it. `Ok(None)` means one was already running.
    #[cfg(feature = "embedded-node")]
    pub async fn spawn_embedded_node(self: &Arc<Self>) -> Result<Option<Rpc>> {
        if self.embedded_node.lock().unwrap().is_some() {
            tprintln!(self, "Your node is already running.");
            return Ok(None);
        }
        let network_id = self.wallet.network_id()?;
        let appdir = crate::embedded::default_appdir(network_id)?;

        // Silent: the callers say different things about the same event, and a
        // fixed paragraph here meant every one of them had to talk over it.
        // Wipe any progress left by a previous node in this session, or the
        // first 'node status' reports the old run's step.
        crate::log_sink::clear_sync_progress();
        let (node, rpc) = crate::embedded::EmbeddedNode::start(network_id, &appdir)?;
        self.embedded_node.lock().unwrap().replace(node);
        Ok(Some(rpc))
    }

    /// Point the wallet at the node we started.
    #[cfg(feature = "embedded-node")]
    pub async fn adopt_embedded_node(self: &Arc<Self>, rpc: Rpc) -> Result<()> {
        // The utxo processor subscribes to its RpcCtl's multiplexer once, when
        // it starts. Binding a new Rpc swaps the api but leaves that task
        // listening to the OLD ctl, so signalling the new one reaches nobody
        // and the wallet sits at DISCONNECTED with a working node inside it.
        // Stop it, bind, start again — then the subscription is to the ctl we
        // are about to signal.
        self.wallet.utxo_processor().stop().await?;
        self.wallet.bind_rpc(Some(rpc)).await?;
        self.wallet.utxo_processor().start().await?;
        // Bound to a local first: `if let Some(x) = guard...` keeps the
        // MutexGuard alive for the whole block, and a std guard held across an
        // await makes the future !Send, which this one has to be.
        let node = self.embedded_node.lock().unwrap().as_ref().cloned();
        if let Some(node) = node {
            node.signal_connected().await?;
        }
        self.embedded_node_adopted.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// True only when the wallet's questions are going to the node we started.
    #[cfg(feature = "embedded-node")]
    pub fn embedded_node_in_use(&self) -> bool {
        self.embedded_node_running() && self.embedded_node_adopted.load(Ordering::SeqCst)
    }

    /// A node of ours is running, but the wallet is still on someone else's
    /// while it catches up.
    #[cfg(feature = "embedded-node")]
    pub fn embedded_node_pending(&self) -> bool {
        self.embedded_node_running() && !self.embedded_node_adopted.load(Ordering::SeqCst)
    }

    /// Ask a node — any node — whether it considers itself caught up.
    ///
    /// This is the node's own verdict, read straight off its RPC rather than
    /// off the wallet's utxo processor, because the whole point here is to ask
    /// a node the wallet is *not* currently using.
    #[cfg(feature = "embedded-node")]
    async fn node_is_synced(rpc: &Rpc) -> bool {
        matches!(rpc.rpc_api().get_server_info().await, Ok(info) if info.is_synced)
    }

    /// Start your own node without making anyone wait for it.
    ///
    /// A node that has not finished its first sync answers every question
    /// truthfully about a chain it has not finished reading, so a wallet bound
    /// to one shows balances that are merely incomplete — which reads exactly
    /// like money having gone missing. The old choice was therefore a real one:
    /// privacy now and a wallet you cannot trust for an hour, or a working
    /// wallet and a stranger who can see which notes are yours.
    ///
    /// It does not have to be a choice. The public node answers while your own
    /// catches up, and the wallet moves across the moment yours is ready. Every
    /// step says so out loud, because silently changing which node sees your
    /// queries would be precisely the wrong thing to be quiet about.
    #[cfg(feature = "embedded-node")]
    pub async fn start_node_with_handover(self: &Arc<Self>) -> Result<()> {
        self.start_node_with_handover_inner(true).await
    }

    /// `ensure_connection` is false when the caller has just connected.
    ///
    /// `is_connected()` flips on an event, not when `connect` returns, so a
    /// caller that has this instant connected still reads false here. Acting
    /// on that would fire a second connect, which would in turn offer to start
    /// a local node — the offer that got us here. It terminates, because by
    /// then a node is running, but only by accident.
    #[cfg(feature = "embedded-node")]
    async fn start_node_with_handover_inner(self: &Arc<Self>, ensure_connection: bool) -> Result<()> {
        // A node that will not start must not cost you a working wallet. It
        // failed for real reasons — the p2p port taken by another Marigold, no
        // room on disk — and the answer to every one of them is the same: say
        // what happened, then connect to something that answers. Letting the
        // error out of here left a wallet that had chosen 'local' sitting at
        // DISCONNECTED with no node at all.
        let rpc = match self.spawn_embedded_node().await {
            Ok(Some(rpc)) => rpc,
            Ok(None) => return Ok(()),
            Err(err) => {
                tprintln!(self, "{}", style(format!("Your node could not start: {err}")).yellow());
                // Same reconnect trap as the success path below: a caller that
                // has just connected reads is_connected() as false, and
                // reconnecting here asked the "run your own node?" question a
                // second time and dropped the socket that was already open.
                if !ensure_connection || self.wallet.is_connected() {
                    tprintln!(self, "The wallet is using a public node instead.");
                    tprintln!(self, "{}", style("Whoever runs it sees which notes your wallet asks about. 'node start'").dim());
                    tprintln!(self, "{}", style("tries yours again once the problem above is dealt with.").dim());
                } else {
                    tprintln!(self, "Using a public node instead, so the wallet works meanwhile.");
                    tprintln!(self, "{}", style("Whoever runs it sees which notes your wallet asks about. 'node start'").dim());
                    tprintln!(self, "{}", style("tries yours again once the problem above is dealt with.").dim());
                    self.exec_within("connect public").await?;
                }
                tprintln!(self, "");
                return Ok(());
            }
        };

        // A node that is already caught up — the second and every later run —
        // needs no public node at all.
        if Self::node_is_synced(&rpc).await {
            self.adopt_embedded_node(rpc).await?;
            tprintln!(self, "{}", style("Your node is caught up. Using it — nobody else sees your notes.").green());
            tprintln!(self, "");
            tprintln!(self, "You can mine with spare CPU — 'mine start'.");
            tprintln!(self, "");
            return Ok(());
        }

        if ensure_connection && !self.wallet.is_connected() {
            if let Err(err) = self.exec_within("connect public").await {
                // No public node either: bind to our own anyway. An incomplete
                // view beats none, and 'node status' explains what it is.
                tprintln!(self, "Could not reach a public node ({err}) — using your own while it catches up.");
                self.adopt_embedded_node(rpc.clone()).await?;
            }
        }

        self.announce_sync_started();
        self.start_node_handover_task(rpc);
        Ok(())
    }

    /// What we tell someone the moment their own node begins its first sync.
    ///
    /// Four lines, and every one of them earns its place: it started, it is
    /// slow, quitting throws it away, and here is how to look. The data
    /// directory, the phase names and the block counts are not in it — none of
    /// them change what anybody does next.
    #[cfg(feature = "embedded-node")]
    pub fn announce_sync_started(self: &Arc<Self>) {
        tprintln!(self, "");
        tprintln!(self, "{}", style("Local node sync started!").green());
        tprintln!(self, "A first sync takes anywhere from half an hour to a few hours. Leaving the");
        tprintln!(self, "wallet before it finishes discards it — after that, restarts are free.");
        tprintln!(self, "Type 'node status' for progress info.");
        tprintln!(self, "");
    }

    /// The one thing to type next.
    ///
    /// Someone who has just connected is at a fork with exactly one sensible
    /// exit, and which one depends on state they cannot see: whether a wallet
    /// is open, and whether they have one at all. Printing all three and
    /// letting them work it out is how a person decides this program is not
    /// for them.
    pub async fn print_next_step(self: &Arc<Self>) {
        tprintln!(self, "");
        if self.wallet.is_open() {
            tprintln!(self, "Type 'balance' or 'help' for list of commands.");
        } else if self.store().wallet_list().await.map(|w| !w.is_empty()).unwrap_or(false) {
            tprintln!(self, "Type 'open' to open your wallet or 'help' for list of commands.");
        } else {
            tprintln!(self, "Type 'wallet create' to create a wallet or 'help' for list of commands.");
        }
        tprintln!(self, "");
    }

    /// Ask, once per public connection, whether they would rather not be on a
    /// public node at all.
    ///
    /// Asked here because this is the moment it is true: they have just
    /// connected to a node run by someone else, and that node can see which
    /// notes their wallet asks after. A "no" is not recorded — the question
    /// costs one keystroke and the answer may be different on a laptop that is
    /// staying put than on one about to be closed.
    #[cfg(feature = "embedded-node")]
    pub async fn offer_local_node(self: &Arc<Self>) -> Result<()> {
        if self.embedded_node_running() {
            return Ok(());
        }
        tprintln!(self, "Do you want to run a local node so that your wallet notes stay private?");
        tprintln!(self, "{}", style("(marigold.cash/faq explains what this choice costs)").dim());
        let answer = self.term().ask(false, "[Y/n]: ").await?.trim().to_lowercase();
        if answer.starts_with('n') {
            tprintln!(self, "");
            tprintln!(self, "Public node connected.");
            return Ok(());
        }
        // We connected a moment ago; do not let it connect again.
        self.start_node_with_handover_inner(false).await
    }

    /// Start the node without touching the wallet's connection.
    ///
    /// For callers that are themselves inside the connect command: routing
    /// back through `exec_within("connect public")` from there re-enters the
    /// handler that is still on the stack and the whole CLI stops dead — no
    /// echo, no prompt, nothing.
    #[cfg(feature = "embedded-node")]
    pub async fn start_local_node_now(self: &Arc<Self>) -> Result<()> {
        self.start_node_with_handover_inner(false).await
    }

    /// Watch the node we started, and move the wallet over when it is ready.
    #[cfg(feature = "embedded-node")]
    fn start_node_handover_task(self: &Arc<Self>, rpc: Rpc) {
        let this = self.clone();
        workflow_core::task::spawn(async move {
            loop {
                workflow_core::task::sleep(Duration::from_secs(60)).await;
                if this.shutdown.load(Ordering::SeqCst) {
                    break;
                }
                // Stopped by hand, or never took: nothing left to hand over to.
                if !this.embedded_node_running() {
                    break;
                }
                if this.wallet.try_rpc_api().map(|api| Arc::ptr_eq(&api, rpc.rpc_api())).unwrap_or(false) {
                    // Already ours — someone ran 'node start' in the meantime.
                    break;
                }
                if !Self::node_is_synced(&rpc).await {
                    continue;
                }
                match this.adopt_embedded_node(rpc.clone()).await {
                    Ok(()) => {
                        tprintln!(this, "");
                        tprintln!(this, "{}", style("Your node has caught up. The wallet is now using it —").green());
                        tprintln!(this, "{}", style("nobody else sees your address or which notes you hold.").green());
                        tprintln!(this, "");
                        // The moment this becomes true is the moment to say
                        // it: mining needs your own synced node, and this is
                        // the only point at which the wallet knows it has one.
                        tprintln!(this, "You can now mine with spare CPU — 'mine start'.");
                        tprintln!(this, "");
                        this.term().refresh_prompt();
                        this.request_open_housekeeping();
                    }
                    Err(err) => {
                        tprintln!(this, "Your node is ready, but the wallet could not switch to it: {err}");
                        tprintln!(this, "'node start' moves it across by hand.");
                    }
                }
                break;
            }
        });
    }


    /// `mine start [percent]` — lend the machine's spare CPU to the network.
    ///
    /// Asks for a percentage rather than a thread count because that is the
    /// question people can actually answer about their own computer. The
    /// threads run under SCHED_IDLE, so the honest description of what they
    /// take is "whatever nothing else wanted".
    #[cfg(feature = "embedded-node")]
    pub async fn start_mining(self: &Arc<Self>, arg: Option<String>) -> Result<()> {
        if self.cpu_miner.lock().unwrap().is_some() {
            tprintln!(self, "Already mining — 'mine status' for how it is going.");
            return Ok(());
        }
        // Mining against somebody else's node would hand them the address your
        // rewards are paid to, which is the one thing this wallet works to keep
        // off other people's machines.
        if !self.embedded_node_in_use() {
            tprintln!(self, "");
            if self.embedded_node_pending() {
                tprintln!(self, "Your node is still catching up. Mining starts once it is ready —");
                tprintln!(self, "'node status' shows how far along it is.");
            } else {
                tprintln!(self, "Mining needs your own node. Type 'node start' to run one.");
                tprintln!(self, "{}", style("Asking a public node for work would tell its operator which address").dim());
                tprintln!(self, "{}", style("your coins are paid to, which is the one thing worth not sharing.").dim());
            }
            tprintln!(self, "");
            return Ok(());
        }
        let account = match self.wallet.account() {
            Ok(account) => account,
            Err(_) => {
                tprintln!(self, "Open a wallet first — mined coins have to be paid to an address.");
                return Ok(());
            }
        };
        let address = account.receive_address()?;

        let cores = crate::miner::cores();
        let percent = match arg {
            Some(text) => match text.trim().trim_end_matches('%').parse::<u32>() {
                Ok(value) if (1..=100).contains(&value) => value,
                _ => {
                    tprintln!(self, "'{text}' is not a percentage between 1 and 100.");
                    return Ok(());
                }
            },
            None => {
                tprintln!(self, "");
                tpara!(
                    self,
                    "Mining runs on whatever your machine is not otherwise using. It is set to the \
                    lowest priority the system has, so it steps aside the moment anything else wants \
                    the processor — you should not be able to feel it. "
                );
                tprintln!(self, "");
                let answer = self
                    .term()
                    .ask(false, &format!("How much of this machine may it use? [1-100%, default 50]: "))
                    .await?
                    .trim()
                    .trim_end_matches('%')
                    .to_string();
                if answer.is_empty() {
                    50
                } else {
                    match answer.parse::<u32>() {
                        Ok(value) if (1..=100).contains(&value) => value,
                        _ => {
                            tprintln!(self, "'{answer}' is not a percentage between 1 and 100 — nothing started.");
                            return Ok(());
                        }
                    }
                }
            }
        };

        let (miner, solutions) = crate::miner::Miner::start(percent);
        self.cpu_miner.lock().unwrap().replace(miner.clone());

        tprintln!(self, "");
        tprintln!(
            self,
            "{}",
            style(format!("Mining started — {percent}% of this machine ({} of {cores} cores).", miner.thread_count())).green()
        );
        tprintln!(self, "{}", style("It yields to anything else that needs the processor.").dim());
        tprintln!(self, "Rewards are paid to this wallet and become notes on their own.");
        tprintln!(self, "'mine status' to check, 'mine stop' to stop.");
        tprintln!(self, "");

        // One task owns both halves: fetching work and submitting what comes
        // back. Keeping them together means there is a single place where the
        // job generation is advanced, so a solution can never be matched
        // against a template that has already been replaced.
        let this = self.clone();
        let miner_task = miner.clone();
        workflow_core::task::spawn(async move {
            let extra = format!("marigold-wallet/{}", env!("CARGO_PKG_VERSION")).into_bytes();
            let mut last_refresh = std::time::Instant::now() - std::time::Duration::from_secs(60);
            loop {
                if !miner_task.is_running() || this.shutdown.load(Ordering::SeqCst) {
                    break;
                }
                if last_refresh.elapsed() >= std::time::Duration::from_millis(crate::miner::TEMPLATE_REFRESH_MS) {
                    last_refresh = std::time::Instant::now();
                    match this.wallet.rpc_api().get_block_template(address.clone(), extra.clone()).await {
                        Ok(response) => {
                            // Building on a chain the node has not finished
                            // reading produces blocks nobody will accept.
                            if response.is_synced {
                                match kaspa_consensus_core::block::Block::try_from(response.block) {
                                    Ok(block) => miner_task.set_job(block),
                                    Err(err) => log_warn!("mine: unusable block template ({err})"),
                                }
                            }
                        }
                        Err(err) => log_warn!("mine: could not get work ({err})"),
                    }
                }
                // Drain whatever the threads found since the last pass.
                while let Ok(solution) = solutions.try_recv() {
                    let Some(rpc_block) = miner_task.block_for(&solution) else { continue };
                    match this.wallet.rpc_api().submit_block(rpc_block, false).await {
                        Ok(_) => {
                            // Deliberately silent. Announcing each block put a
                            // three-line interruption on the terminal every
                            // time one landed, on top of whatever the person
                            // was typing. 'mine status' reports the total.
                            miner_task.record_accepted();
                        }
                        Err(err) => {
                            miner_task.record_rejected();
                            log_warn!("mine: block not accepted ({err})");
                        }
                    }
                }
                workflow_core::task::sleep(Duration::from_millis(100)).await;
            }
        });
        Ok(())
    }

    #[cfg(feature = "embedded-node")]
    pub async fn stop_mining(self: &Arc<Self>) -> Result<()> {
        let miner = self.cpu_miner.lock().unwrap().take();
        match miner {
            Some(miner) => {
                let found = miner.blocks_found();
                miner.stop();
                tprintln!(self, "Mining stopped.");
                if found > 0 {
                    tprintln!(self, "Found {} block(s) this run.", found.separated_string());
                }
                Ok(())
            }
            None => {
                tprintln!(self, "Not mining.");
                Ok(())
            }
        }
    }

    #[cfg(feature = "embedded-node")]
    pub async fn mining_status(self: &Arc<Self>) {
        let miner = self.cpu_miner.lock().unwrap().clone();
        tprintln!(self, "");
        match miner {
            Some(miner) => {
                tprintln!(
                    self,
                    "Mining: {} of {} cores ({}% of this machine)",
                    miner.thread_count(),
                    crate::miner::cores(),
                    miner.percent()
                );
                tprintln!(self, "Speed:  {}", crate::miner::format_hashrate(miner.hashrate()));
                let found = miner.blocks_found();
                if found == 0 {
                    tprintln!(self, "Blocks: none yet");
                    tprintln!(self, "{}", style("Finding one is luck. Leaving it running is the whole technique.").dim());
                } else {
                    tprintln!(
                        self,
                        "Blocks: {} found, {} accepted{}",
                        found.separated_string(),
                        miner.blocks_accepted().separated_string(),
                        match miner.blocks_rejected() {
                            0 => String::new(),
                            n => format!(", {} not accepted", n.separated_string()),
                        }
                    );
                }
            }
            None => {
                tprintln!(self, "Not mining.");
                if self.embedded_node_in_use() {
                    tprintln!(self, "'mine start' begins, using whatever CPU nothing else wants.");
                } else {
                    tprintln!(self, "Mining needs your own node — 'node start'.");
                }
            }
        }
        tprintln!(self, "");
    }

    #[cfg(feature = "embedded-node")]
    pub async fn stop_embedded_node(self: &Arc<Self>) -> Result<()> {
        let node = self.embedded_node.lock().unwrap().take();
        self.embedded_node_adopted.store(false, Ordering::SeqCst);
        crate::log_sink::clear_sync_progress();
        match node {
            Some(node) => {
                if !self.wallet.utxo_processor().is_synced() {
                    tprintln!(self, "");
                    tprintln!(self, "{}", style("Note: this node has not finished its first sync, and that progress").yellow());
                    tprintln!(self, "{}", style("is discarded — the next start begins again from scratch.").yellow());
                    tprintln!(self, "");
                }
                tprintln!(self, "Stopping your node...");
                node.stop().await?;
                tprintln!(self, "Stopped.");
            }
            None => tprintln!(self, "No node of yours is running."),
        }
        Ok(())
    }

    /// A node that is running but has not finished its first sync — the state
    /// in which stopping throws the work away.
    #[cfg(feature = "embedded-node")]
    pub fn embedded_node_syncing(&self) -> bool {
        self.embedded_node_running() && !self.wallet.utxo_processor().is_synced()
    }

    #[cfg(feature = "embedded-node")]
    pub fn embedded_node_running(&self) -> bool {
        self.embedded_node.lock().unwrap().is_some()
    }

    pub fn try_rpc_api(&self) -> Option<Arc<DynRpcApi>> {
        self.wallet.try_rpc_api().clone()
    }

    pub fn try_rpc_client(&self) -> Option<Arc<KaspaRpcClient>> {
        self.wallet.try_wrpc_client().clone()
    }

    pub fn store(&self) -> Arc<dyn Interface> {
        self.wallet.store().clone()
    }

    pub fn daemons(&self) -> &Arc<Daemons> {
        &self.daemons
    }

    /// Run another command from inside a running one.
    ///
    /// NOT `term().exec()`: that draws a prompt when it finishes, which is
    /// right for the top-level loop it was written for and wrong here — the
    /// outer command then finishes and the loop draws a second, and the
    /// housekeeping tick refreshes a third. That is where "DISCONNECTED $ $ $"
    /// came from after accepting the connect offer, and the doubled "$ $"
    /// after `close` (founder report, 2026-09-07).
    pub async fn exec_within(self: &Arc<Self>, cmd: &str) -> Result<()> {
        self.handlers.execute(self, cmd).await?;
        Ok(())
    }

    pub fn handlers(&self) -> Arc<HandlerCli> {
        self.handlers.clone()
    }

    pub fn flags(&self) -> &Flags {
        &self.flags
    }

    /// Arm auto-mint for this session with the secret already in hand (the
    /// one typed at `open`) — no extra prompt, and nothing persisted.
    pub fn arm_auto_mint(&self, secret: Secret, payment_secret: Option<Secret>, threshold_petals: u64) {
        *self.auto_secret.lock().unwrap() = Some(secret);
        *self.auto_payment_secret.lock().unwrap() = payment_secret;
        self.auto_threshold_petals.store(threshold_petals, Ordering::SeqCst);
    }

    /// Check a password against the account's own key data before arming
    /// anything with it. Housekeeping signs in the background, an hour after
    /// the prompt has scrolled away — a typo accepted here becomes an
    /// automation that quietly does nothing, which is the worst way to find out.
    pub async fn verify_wallet_secret(&self, secret: &Secret, payment_secret: Option<&Secret>) -> Result<()> {
        let account = self.wallet().account()?;
        let id = account.prv_key_data_id()?;
        let key_data = match self.wallet().get_prv_key_data(secret, id).await {
            Ok(Some(key_data)) => key_data,
            Ok(None) => return Err(Error::custom("this account has no key to sign with")),
            Err(_) => return Err(Error::custom("That password does not open this wallet")),
        };
        // The wallet password unwraps the key store; a bip39 passphrase, if the
        // key has one, unwraps the key itself. Automation needs both.
        key_data.get_xprv(payment_secret).map_err(|_| Error::custom("That passphrase does not unlock this account's key"))?;
        Ok(())
    }

    /// Arm auto-sweep (consolidation only — no minting). Independent of
    /// auto-mint: a wallet that deliberately holds ledger balance still wants
    /// its dust consolidated.
    pub fn arm_auto_sweep(&self, secret: Secret, payment_secret: Option<Secret>, utxo_threshold: u64) {
        let mut guard = self.auto_secret.lock().unwrap();
        if guard.is_none() {
            *guard = Some(secret);
            *self.auto_payment_secret.lock().unwrap() = payment_secret;
        }
        drop(guard);
        self.auto_sweep_utxos.store(utxo_threshold, Ordering::SeqCst);
    }

    /// Stop auto-sweep in THIS session (the preference is stored separately).
    /// An already-running consolidation finishes; nothing new starts.
    pub fn disarm_auto_sweep(&self) {
        self.auto_sweep_utxos.store(0, Ordering::SeqCst);
        if self.auto_threshold_petals.load(Ordering::SeqCst) == 0 {
            self.auto_secret.lock().unwrap().take();
            self.auto_payment_secret.lock().unwrap().take();
        }
    }

    pub fn auto_sweep_threshold(&self) -> u64 {
        self.auto_sweep_utxos.load(Ordering::SeqCst)
    }

    pub fn disarm_auto_mint(&self) {
        self.auto_threshold_petals.store(0, Ordering::SeqCst);
        if self.auto_sweep_threshold() == 0 {
            self.auto_secret.lock().unwrap().take();
            self.auto_payment_secret.lock().unwrap().take();
        }
    }

    /// Drop every armed automation and the secret with it (wallet close).
    pub fn disarm_automation(&self) {
        self.auto_threshold_petals.store(0, Ordering::SeqCst);
        self.auto_sweep_utxos.store(0, Ordering::SeqCst);
        self.auto_secret.lock().unwrap().take();
        self.auto_payment_secret.lock().unwrap().take();
        self.prompt_total_valid.store(false, Ordering::SeqCst);
    }

    pub fn auto_mint_armed(&self) -> bool {
        self.auto_secret.lock().unwrap().is_some()
    }

    pub fn auto_mint_threshold(&self) -> u64 {
        self.auto_threshold_petals.load(Ordering::SeqCst)
    }

    /// True while this wallet has transactions it submitted that the chain has
    /// not confirmed yet. Starting another automated spend during that window
    /// is how a wallet double-spends its own inputs: the coins are gone from
    /// its point of view only once the spending transaction confirms, and the
    /// node rejects the second attempt with "already spent in the mempool".
    fn has_unconfirmed_spends(&self) -> bool {
        self.wallet.account().ok().and_then(|account| account.balance()).map(|balance| balance.outgoing > 0).unwrap_or(false)
    }

    /// Ask for the opening sequence (report, housekeeping, report) to run as
    /// soon as the wallet's coins are known.
    pub fn request_open_housekeeping(&self) {
        self.open_housekeeping_pending.store(true, Ordering::SeqCst);
    }

    pub fn set_auto_verbose(&self, verbose: bool) {
        self.auto_verbose.store(verbose, Ordering::SeqCst);
    }

    pub fn auto_verbose(&self) -> bool {
        self.auto_verbose.load(Ordering::SeqCst)
    }

    /// Wait for the wallet to be usable: an account selected (selection
    /// happens asynchronously, off the activation event, so right after
    /// `open` there is briefly none) and its coins loaded. Both the opening
    /// report and the housekeeping used to bail out silently in that window —
    /// which is why a wallet with a ledger balance reported none and minted
    /// nothing (2026-09-05). Deliberately does not wait for the balance to
    /// settle: on a mining wallet it never does, since every fee paid returns
    /// as a block reward within seconds.
    async fn wait_for_account(&self) -> Option<Arc<dyn Account>> {
        for i in 0..60 {
            if let Ok(account) = self.wallet.account() {
                let has_coins = account.utxo_context().mature_utxo_size() > 0
                    || account.balance().map(|b| b.mature > 0 || b.pending > 0).unwrap_or(false);
                // Once an account exists, give its first scan a moment; an
                // empty wallet must not hang here, so stop waiting after ~2s.
                if has_coins || i > 12 {
                    return Some(account);
                }
            }
            workflow_core::task::sleep(Duration::from_millis(150)).await;
        }
        self.wallet.account().ok()
    }

    /// Total holdings in petals: notes plus ledger. Notes first, because that
    /// is where the money lives; the ledger is a loading dock.
    pub async fn total_holdings(&self) -> (u64, u64, usize) {
        // Read the coins themselves, not the cached Balance: the context is
        // populated before `balance()` stops returning None, and trusting the
        // cache here reported an empty ledger on a wallet that had one.
        let (ledger, pieces) = self
            .wallet
            .account()
            .ok()
            .map(|account| {
                let (mature, _, _) = account.utxo_context().utxo_entries_snapshot();
                (mature.iter().map(|entry| entry.amount()).sum::<u64>(), mature.len())
            })
            .unwrap_or((0, 0));
        let mut notes = 0u64;
        if let Ok(store) = self.wallet.store().as_note_key_store() {
            if let Ok(mut stream) = store.iter().await {
                while let Ok(Some(info)) = stream.try_next().await {
                    // Mirrored notes count: they are the user's money, merely
                    // carried elsewhere, and this wallet still holds their keys.
                    if matches!(
                        info.status,
                        kaspa_wallet_core::storage::NoteStatus::Active | kaspa_wallet_core::storage::NoteStatus::Mirrored
                    ) {
                        notes += kaspa_consensus_core::notepool::DENOMINATION_PETALS[info.d as usize];
                    }
                }
            }
        }
        (notes, ledger, pieces)
    }

    /// Show what the wallet holds: notes, then the ledger — and the ledger
    /// only when it holds something, because for anyone but an exchange it
    /// should be empty most of the time.
    /// Count the UTXO set in as it is read, so a long load looks like work.
    ///
    /// A wallet that has been mined into holds millions of coinbase outputs —
    /// this one had 2.9 million — and reading them takes a minute during which
    /// the only output was the word "Loading". A number that climbs is the
    /// difference between waiting and wondering whether it has hung.
    ///
    /// Counted rather than given as a percentage: the total is known inside
    /// the scanner and is not reported out of it, so a percentage here would
    /// have to be invented.
    fn start_loading_progress(self: &Arc<Self>) {
        if self.loading.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        workflow_core::task::spawn(async move {
            let term = this.term();
            term.writeln("Loading...");
            let mut last = 0usize;
            while this.loading.load(Ordering::SeqCst) && !this.shutdown.load(Ordering::SeqCst) {
                workflow_core::task::sleep(Duration::from_millis(700)).await;
                if !this.loading.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(account) = this.wallet.account() else { continue };
                let context = account.utxo_context();
                let count = context.mature_utxo_size() + context.pending_utxo_size();
                if count != last {
                    last = count;
                    // \r rather than a new line: one line that is rewritten,
                    // not a minute of scrollback.
                    term.write(format!("\r  {} coins read...", count.separated_string()));
                }
            }
            if last > 0 {
                term.writeln(format!("\r  {} coins read.   ", last.separated_string()));
            }
        });
    }

    /// Stop the counter. Called when the figures are about to be printed,
    /// which is the moment the load has finished mattering.
    fn finish_loading_progress(&self) {
        self.loading.store(false, Ordering::SeqCst);
    }

    pub async fn report_holdings(self: &Arc<Self>) -> u64 {
        self.wait_for_account().await;
        self.finish_loading_progress();
        let (notes, ledger, pieces) = self.total_holdings().await;
        // Keep the prompt's figure in step, so it is right from the moment a
        // wallet opens rather than after the first minute tick.
        self.prompt_total_petals.store(notes + ledger, Ordering::SeqCst);
        self.prompt_total_valid.store(true, Ordering::SeqCst);
        tprintln!(self, "");
        tprintln!(self, "notes:  {} MAGLD", kaspa_wallet_core::utils::sompi_to_kaspa_string(notes));
        // Said once, on opening, so that reaching for a phone at the moment
        // of a payment is expected rather than alarming.
        if self.otp().is_some() {
            tprintln!(self, "{}", style("(this wallet asks for a code from your phone before it spends)").dim());
        }
        if ledger > 0 {
            tprintln!(
                self,
                "ledger: {} MAGLD  ({} piece{})",
                kaspa_wallet_core::utils::sompi_to_kaspa_string(ledger),
                pieces.separated_string(),
                if pieces == 1 { "" } else { "s" }
            );
        }
        // Same check the `balance` command runs, at the moment a wallet opens
        // and after each announced housekeeping pass — which is exactly when a
        // failed submission would have left a note behind that is not on chain.
        // Reconcile quietly. A note the pool has not got is usually a mint
        // that has not landed yet, and shouting about it every time a balance
        // is printed taught people to ignore a line that one day will matter.
        // Nothing is said until a synced node has said so three times, at
        // which point it has stopped being counted and that is worth one line.
        if notes > 0 && self.wallet.is_connected() {
            if let Ok(account) = self.wallet.account() {
                if let Ok(Some(result)) = kaspa_wallet_core::account::notepool::reconcile_held_notes(account).await {
                    if !result.moved_to_unknown.is_empty() {
                        let value: u64 = result
                            .moved_to_unknown
                            .iter()
                            .map(|i| kaspa_consensus_core::notepool::DENOMINATION_PETALS[i.d as usize])
                            .sum();
                        tprintln!(
                            self,
                            "{}",
                            style(format!(
                                "{} note(s) worth {} MAGLD are not on chain and have stopped being counted.",
                                result.moved_to_unknown.len(),
                                kaspa_wallet_core::utils::sompi_to_kaspa_string(value)
                            ))
                            .yellow()
                        );
                        tprintln!(self, "{}", style("Most often a payment that never landed, in which case the money never").dim());
                        tprintln!(self, "{}", style("left your ledger balance. 'note unknown' lists them.").dim());
                    }
                }
            }
        }
        tprintln!(self, "");
        notes
    }

    /// The ledger housekeeping sequence, in order and never overlapping:
    /// consolidate the coins, turn them into notes, then tidy the notes.
    /// `announce` narrates it — used when a wallet opens, where the backlog
    /// can be large and silence would look like a hang. The once-a-minute
    /// runs stay quiet unless 'auto verbose' is on: the ledger is plumbing,
    /// and plumbing should not talk.
    /// How many 0.01 notes to keep on hand. They are the fee stamps a pure
    /// pool operation spends, so a wallet needs a working supply — but only a
    /// working supply. Above this, minting stops making more.
    pub async fn stamp_count(&self) -> usize {
        let Ok(store) = self.wallet.store().as_note_key_store() else { return 0 };
        let Ok(mut stream) = store.iter().await else { return 0 };
        let mut count = 0usize;
        while let Ok(Some(info)) = stream.try_next().await {
            if info.status == kaspa_wallet_core::storage::NoteStatus::Active && info.d as usize == 0 {
                count += 1;
            }
        }
        count
    }

    pub async fn run_housekeeping(self: &Arc<Self>, announce: bool) {
        // Nothing housekeeping has to say during shutdown is worth hearing.
        // The wallet is going away, the node connection goes first, and every
        // message it then produces describes that — printed after "bye!", over
        // a shell prompt that has already come back.
        if self.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let loud = announce || self.auto_verbose();
        // "Armed" means something is actually configured to run — holding the
        // secret is not the same thing, and conflating them made a wallet with
        // only auto-sweep on look like auto-mint was armed too.
        if !self.auto_mint_armed() || (self.auto_mint_threshold() == 0 && self.auto_sweep_threshold() == 0) {
            if loud {
                tprintln!(self, "(nothing armed — 'auto on' turns on minting, 'auto sweep' consolidation)");
            }
            return;
        }
        if !self.wallet.is_connected() {
            return;
        }
        if self.auto_busy.swap(true, Ordering::SeqCst) {
            if loud {
                tprintln!(self, "(housekeeping already running)");
            }
            return;
        }
        let abortable = Abortable::default();
        let Some(account) = self.wait_for_account().await else {
            self.auto_busy.store(false, Ordering::SeqCst);
            return;
        };
        let Some(secret) = self.auto_secret.lock().unwrap().clone() else {
            self.auto_busy.store(false, Ordering::SeqCst);
            return;
        };
        let payment_secret = self.auto_payment_secret.lock().unwrap().clone();

        // --- 1. turn the ledger into notes ---
        // Minting comes FIRST because a mint IS a consolidation: it takes many
        // mature coins as inputs and leaves notes plus a single change coin.
        // Sweeping first spent the very coins the mint needed and pushed them
        // into "pending", so the mint that followed saw almost nothing —
        // 1.32 MAGLD of a 143,000 MAGLD ledger (founder report, 2026-09-06).
        let threshold = self.auto_mint_threshold();
        if threshold > 0 && self.has_unconfirmed_spends() && loud {
            tprintln!(self, "Waiting: this wallet has transactions the chain has not confirmed yet.");
        }
        if threshold > 0 && !self.has_unconfirmed_spends() {
            // Read the coins, not the cached Balance — it is None during the
            // window right after activation, which silently skipped minting.
            let (mature_entries, _, _) = account.utxo_context().utxo_entries_snapshot();
            let mature: u64 = mature_entries.iter().map(|entry| entry.amount()).sum();
            if loud && mature < threshold {
                let (_, pending, stasis) = account.utxo_context().utxo_entries_snapshot();
                let waiting: u64 = pending.iter().chain(stasis.iter()).map(|e| e.amount()).sum();
                if waiting > 0 {
                    tprintln!(
                        self,
                        "Nothing to mint yet: {} MAGLD is confirming (threshold {}). It will be minted as it lands.",
                        kaspa_wallet_core::utils::sompi_to_kaspa_string(waiting),
                        kaspa_wallet_core::utils::sompi_to_kaspa_string(threshold)
                    );
                } else {
                    tprintln!(
                        self,
                        "Nothing to mint: ledger holds {} MAGLD, threshold is {}.",
                        kaspa_wallet_core::utils::sompi_to_kaspa_string(mature),
                        kaspa_wallet_core::utils::sompi_to_kaspa_string(threshold)
                    );
                }
            }
            if mature >= threshold {
                let minted = async {
                    // mint_max owns the sizing: the estimate cannot predict the
                    // real transaction's shape, so it retries with a larger
                    // change reserve rather than trusting a margin.
                    let result = kaspa_wallet_core::account::notepool::mint_max(
                        account.clone(),
                        secret.clone(),
                        payment_secret.clone(),
                        None,
                        &abortable,
                        None,
                    )
                    .await?;
                    Ok::<_, kaspa_wallet_core::error::Error>(result.map(|(amount, result)| (amount, result.notes.len())))
                }
                .await;
                match minted {
                    Ok(Some((amount, notes))) => {
                        if loud {
                            tprintln!(
                                self,
                                "Minted {} MAGLD into {notes} note(s).",
                                kaspa_wallet_core::utils::sompi_to_kaspa_string(amount)
                            );
                        }
                    }
                    Ok(None) => {
                        if loud {
                            tprintln!(self, "Nothing mintable right now (the fee would exceed what is there).");
                        }
                    }
                    Err(err) => {
                        // A decrypt failure will fail identically forever.
                        // Retrying it every minute in silence is how an
                        // automation ends up doing nothing for a week without
                        // anyone noticing, so this one always speaks up.
                        if matches!(
                            err,
                            kaspa_wallet_core::error::Error::Chacha20poly1305(_)
                                | kaspa_wallet_core::error::Error::WalletDecrypt(_)
                        ) {
                            self.disarm_auto_mint();
                            tprintln!(self, "Auto-mint turned itself off: the stored password no longer opens this wallet.");
                            tprintln!(self, "Run 'auto on' and enter it again.");
                        } else if loud {
                            tprintln!(self, "Minting stopped: {err}");
                        }
                    }
                }
            }
        }

        // --- 2. consolidate whatever minting could not take ---
        // Only what the mint left behind: change, dust below a whole petal,
        // and coins that arrived while it ran.
        let sweep_threshold = self.auto_sweep_threshold();
        let pieces = account.utxo_context().mature_utxo_size() as u64;
        if sweep_threshold > 0 && pieces > sweep_threshold && !self.has_unconfirmed_spends() {
            if loud {
                tprintln!(self, "Consolidating {} ledger pieces — this can take a while...", pieces.separated_string());
            }
            let notifier: Option<kaspa_wallet_core::account::GenerationNotifier> = None;
            match account.clone().sweep(secret.clone(), payment_secret.clone(), None, &abortable, notifier).await {
                Ok(summary) => {
                    if loud {
                        tprintln!(
                            self,
                            "Consolidated (fees {} MAGLD).",
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(summary.0.aggregate_fees())
                        );
                    }
                    // No waiting here for the swept coins to confirm: that
                    // wait was racing the chain and breaking early, and there
                    // is nothing to race for. The next housekeeping run mints
                    // them once they are actually mature.
                    if loud {
                        tprintln!(self, "The consolidated coins will be minted once they confirm.");
                    }
                }
                Err(err) => {
                    if loud {
                        tprintln!(self, "Consolidation stopped: {err}");
                    }
                }
            }
        }

        // --- 3. remove recovery keys no account uses ---
        // Left behind by an interrupted 'account create'; they hold nothing
        // and no address can have received to them. Cleaning them up is
        // housekeeping, not a decision to put to the user.
        if let Ok(store) = self.wallet.store().as_prv_key_data_store() {
            if let Ok(mut stream) = store.iter().await {
                let mut orphans = Vec::new();
                let guard = self.wallet.guard();
                let guard = guard.lock().await;
                while let Ok(Some(info)) = stream.try_next().await {
                    if let Ok(mut accounts) = self.wallet.accounts(Some(info.id), &guard).await {
                        if accounts.try_next().await.ok().flatten().is_none() {
                            orphans.push(info.id);
                        }
                    }
                }
                drop(guard);
                if !orphans.is_empty() {
                    let removed = orphans.len();
                    for id in orphans {
                        store.remove(&secret, &id).await.ok();
                    }
                    self.wallet.store().commit(&secret).await.ok();
                    if loud {
                        tprintln!(self, "Removed {removed} unused recovery key(s).");
                    }
                }
            }
        }

        // --- 4. tidy the notes themselves (ten of a size become one larger) ---
        // Deliberately NOT gated on has_unconfirmed_spends(): that measures the
        // ledger's outgoing balance, and a note merge is a pure pool operation
        // that spends notes and pays its fee from a spare one — it touches no
        // ledger coin at all. Since minting runs first and always leaves an
        // outgoing balance behind, the gate meant this step simply never ran,
        // which is why ten 0.1s, twenty-two 1s and twelve 10,000s all sat
        // unmerged (founder report, 2026-09-06). Notes already in flight are
        // marked Superseded at submit time, so plan_merges cannot pick them
        // twice — the double-spend the gate was guarding against is handled
        // where it actually applies.
        match kaspa_wallet_core::account::notepool::merge_held_notes(account, secret, 250).await {
            Ok((merged, failure)) => {
                if merged > 0 && loud {
                    tprintln!(self, "Consolidated {merged} group(s) of ten notes into larger ones.");
                }
                if let Some(reason) = failure {
                    if loud {
                        tprintln!(self, "Note consolidation stopped: {reason}");
                    }
                }
            }
            Err(err) => {
                if loud {
                    tprintln!(self, "Note consolidation stopped: {err}");
                }
            }
        }

        self.auto_busy.store(false, Ordering::SeqCst);
    }

    /// One loop owns all housekeeping, so nothing can race anything else:
    /// event-driven triggers competed for the same busy flag and silently
    /// cancelled each other. It refreshes the prompt figure, runs the opening
    /// sequence once (announced, as soon as the wallet knows its coins), and
    /// thereafter runs quietly once a minute.
    fn start_housekeeping_task(self: &Arc<Self>) {
        let this = self.clone();
        workflow_core::task::spawn(async move {
            let mut last_run = Instant::now();
            loop {
                workflow_core::task::sleep(Duration::from_secs(5)).await;
                if this.shutdown.load(Ordering::SeqCst) {
                    break;
                }
                if !this.wallet.is_open() || !this.wallet.is_connected() {
                    continue;
                }

                let (notes, ledger, _) = this.total_holdings().await;
                this.prompt_total_petals.store(notes + ledger, Ordering::SeqCst);
                this.prompt_total_valid.store(true, Ordering::SeqCst);

                // The periodic run is NOT conditional on the opening sequence
                // having happened. It used to be, via a `first_run_done` flag
                // set only inside the branch below — so a wallet opened while
                // disconnected and connected by hand afterwards never set it,
                // and housekeeping never ran once for the whole session. The
                // prompt total kept climbing, because that is updated above
                // this point, which made it look alive: a wallet sat for an
                // hour minting nothing and merging nothing while its ledger
                // ran to 42,605 coins (founder report, 2026-09-06).
                if this.open_housekeeping_pending.swap(false, Ordering::SeqCst) {
                    last_run = Instant::now();
                    let before = this.report_holdings().await;
                    this.run_housekeeping(true).await;
                    // Only say it twice if it changed. Printing the same
                    // figure again reads as a stutter, not as a report.
                    if this.total_holdings().await.0 != before {
                        this.report_holdings().await;
                    }
                    this.term().refresh_prompt();
                } else if last_run.elapsed().as_secs() >= 60 {
                    last_run = Instant::now();
                    this.run_housekeeping(false).await;
                    this.term().refresh_prompt();
                }
            }
        });
    }

    pub fn toggle_mute(&self) -> &'static str {
        helpers::toggle(&self.mute)
    }

    pub fn is_mutted(&self) -> bool {
        self.mute.load(Ordering::SeqCst)
    }

    pub fn register_metrics(self: &Arc<Self>) -> Result<()> {
        use crate::modules::metrics;
        register_handlers!(self, self.handlers(), [metrics]);
        Ok(())
    }

    pub fn register_handlers(self: &Arc<Self>) -> Result<()> {
        crate::modules::register_handlers(self)?;

        #[cfg(not(feature = "embedded-node"))]
        if let Some(node) = self.handlers().get("node") {
            let node = node.downcast_arc::<crate::modules::node::Node>().ok();
            *self.node.lock().unwrap() = node;
        }

        if let Some(miner) = self.handlers().get("miner") {
            let miner = miner.downcast_arc::<crate::modules::miner::Miner>().ok();
            *self.miner.lock().unwrap() = miner;
        }

        crate::matchers::register_link_matchers(self)?;

        Ok(())
    }

    pub async fn handle_daemon_event(self: &Arc<Self>, event: DaemonEvent) -> Result<()> {
        match event.kind() {
            DaemonKind::Kaspad => {
                // Only the child-process node produces daemon events; the
                // embedded one runs in this process and has none.
                #[cfg(not(feature = "embedded-node"))]
                {
                    let node = self.node.lock().unwrap().clone();
                    if let Some(node) = node {
                        node.handle_event(self, event.into()).await?;
                    } else {
                        panic!("Stdio handler: node module is not initialized");
                    }
                }
            }
            DaemonKind::CpuMiner => {
                let miner = self.miner.lock().unwrap().clone();
                if let Some(miner) = miner {
                    miner.handle_event(self, event.into()).await?;
                } else {
                    panic!("Stdio handler: miner module is not initialized");
                }
            }
        }

        Ok(())
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        self.start_notification_pipe_task();
        self.start_housekeeping_task();
        self.handlers.start(self).await?;
        // wallet starts rpc and notifier
        self.wallet.load_settings().await.unwrap_or_else(|_| log_error!("Unable to load settings, discarding..."));
        // Apply the custom wallet-storage folder before anything opens a
        // wallet (the setting itself always lives at the default location).
        if let Some(folder) = self.wallet.settings().get::<String>(WalletSettings::Folder) {
            self.wallet.store().set_storage_folder(&folder).unwrap_or_else(|err| log_error!("Unable to apply wallet folder setting: {err}"));
        }
        self.wallet.start().await?;
        Ok(())
    }

    pub async fn run(self: &Arc<Self>) -> Result<()> {
        self.term().run().await?;
        Ok(())
    }

    pub async fn stop(self: &Arc<Self>) -> Result<()> {
        self.wallet.stop().await?;

        self.handlers.stop(self).await?;

        // stop notification pipe task
        self.stop_notification_pipe_task().await?;
        Ok(())
    }

    async fn stop_notification_pipe_task(self: &Arc<Self>) -> Result<()> {
        self.notifications_task_ctl.signal(()).await?;
        Ok(())
    }

    fn start_notification_pipe_task(self: &Arc<Self>) {
        let this = self.clone();
        let multiplexer = MultiplexerChannel::from(self.wallet.multiplexer());

        workflow_core::task::spawn(async move {
            // See the Events::Balance arm: rate-limits prompt redraws driven
            // by per-block balance updates.
            let mut last_balance_refresh = Instant::now();
            loop {
                select! {

                    _ = this.notifications_task_ctl.request.receiver.recv().fuse() => {
                        break;
                    },

                    msg = multiplexer.receiver.recv().fuse() => {

                        if let Ok(msg) = msg {
                            match *msg {
                                Events::WalletList { .. } => {},
                                Events::WalletPing => {
                                    // log_info!("Kaspa NG - received wallet ping");
                                },
                                Events::Metrics { network_id : _, metrics : _ } => {
                                    // log_info!("Kaspa NG - received metrics event {metrics:?}")
                                }
                                Events::FeeRate { .. } => {},
                                Events::Error { message } => { terrorln!(this,"{message}"); },
                                Events::UtxoProcStart => {},
                                Events::UtxoProcStop => {},
                                Events::UtxoProcError { message } => {
                                    terrorln!(this,"{message}");
                                },
                                #[allow(unused_variables)]
                                Events::Connect{ url, network_id } => {
                                    // log_info!("Connected to {url}");
                                },
                                #[allow(unused_variables)]
                                Events::Disconnect{ url, network_id } => {
                                    tprintln!(this, "Disconnected from {}",url.unwrap_or("N/A".to_string()));
                                    this.term().refresh_prompt();
                                },
                                Events::UtxoIndexNotEnabled { .. } => {
                                    tprintln!(this, "Error: Marigold node UTXO index is not enabled...")
                                },
                                Events::SyncState { sync_state } => {

                                    if sync_state.is_synced() && this.wallet().is_open() {
                                        let guard = this.wallet().guard();
                                        let guard = guard.lock().await;
                                        // reactivate: true — reload(false) stops every account and
                                        // resets the UTXO processor, leaving reactivation to the
                                        // caller... which this caller never did (inherited upstream).
                                        // Anyone who opened their wallet BEFORE connecting got a
                                        // permanent N/A balance out of it.
                                        if let Err(error) = this.wallet().reload(true, &guard).await {
                                            terrorln!(this, "Unable to reload wallet: {error}");
                                        }
                                    }

                                    this.sync_state.lock().unwrap().replace(sync_state);
                                    this.term().refresh_prompt();
                                }
                                Events::ServerStatus {
                                    is_synced,
                                    server_version,
                                    url,
                                    ..
                                } => {

                                    tprintln!(this, "Connected to Marigold node version {server_version} at {}", url.unwrap_or("N/A".to_string()));

                                    let is_open = this.wallet.is_open();

                                    if !is_synced {
                                        if is_open {
                                            terrorln!(this, "Unable to update the wallet state - Marigold node is currently syncing with the network...");

                                        } else {
                                            terrorln!(this, "Marigold node is currently syncing with the network, please wait for the sync to complete...");
                                        }
                                    }

                                    this.term().refresh_prompt();

                                },
                                Events::WalletHint {
                                    hint
                                } => {

                                    if let Some(hint) = hint {
                                        tprintln!(this, "\nYour wallet hint is: {hint}\n");
                                    }
                                    // Counting a large vault takes real time —
                                    // ten seconds on tens of thousands of
                                    // notes, more as it grows. Printed here
                                    // rather than from the open command so it
                                    // lands after the hint: the hint arrives
                                    // as an event and would otherwise overtake
                                    // it.
                                    this.start_loading_progress();
                                },
                                Events::AccountSelection { .. } => { },
                                Events::WalletCreate { .. } => { },
                                Events::WalletError { .. } => { },
                                // Events::WalletReady { .. } => { },

                                Events::WalletOpen { .. } |
                                Events::WalletReload { .. } => { },
                                Events::WalletClose => {
                                    // A code accepted for one wallet is not a
                                    // code accepted for the next.
                                    this.reset_otp_session();
                                    this.term().refresh_prompt();
                                },
                                Events::PrvKeyDataCreate { .. } => { },
                                Events::AccountDeactivation { .. } => { },
                                Events::AccountActivation { .. } => {
                                    // list all accounts
                                    this.list().await.unwrap_or_else(|err|terrorln!(this, "{err}"));

                                    // load default account if only one account exists
                                    this.wallet().autoselect_default_account_if_single().await.ok();
                                    this.term().refresh_prompt();
                                },
                                Events::AccountCreate { .. } => { },
                                Events::AccountUpdate { .. } => { },
                                Events::DaaScoreChange { current_daa_score } => {
                                    if this.is_mutted() && this.flags.get(Track::Daa) {
                                        tprintln!(this, "{NOTIFY} DAA: {current_daa_score}");
                                    }
                                },
                                Events::Discovery { .. } => { }
                                Events::Reorg {
                                    record
                                } => {
                                    if !this.is_mutted() || (this.is_mutted() && this.flags.get(Track::Pending)) {
                                        let guard = this.wallet.guard();
                                        let guard = guard.lock().await;

                                        let include_utxos = this.flags.get(Track::Utxo);
                                        let tx = record.format_transaction_with_state(&this.wallet,Some("reorg"),include_utxos, &guard).await;
                                        tx.iter().for_each(|line|tprintln!(this,"{NOTIFY} {line}"));
                                    }
                                },
                                Events::Stasis {
                                    record
                                } => {
                                    // Pending and coinbase stasis fall under the same `Track` category
                                    if !this.is_mutted() || (this.is_mutted() && this.flags.get(Track::Pending)) {
                                        let guard = this.wallet.guard();
                                        let guard = guard.lock().await;

                                        let include_utxos = this.flags.get(Track::Utxo);
                                        let tx = record.format_transaction_with_state(&this.wallet,Some("stasis"),include_utxos, &guard).await;
                                        tx.iter().for_each(|line|tprintln!(this,"{NOTIFY} {line}"));
                                    }
                                },
                                // Events::External {
                                //     record
                                // } => {
                                //     if !this.is_mutted() || (this.is_mutted() && this.flags.get(Track::Tx)) {
                                //         let include_utxos = this.flags.get(Track::Utxo);
                                //         let tx = record.format_with_state(&this.wallet,Some("external"),include_utxos).await;
                                //         tx.iter().for_each(|line|tprintln!(this,"{NOTIFY} {line}"));
                                //     }
                                // },
                                Events::Pending {
                                    record
                                } => {
                                    if !this.is_mutted() || (this.is_mutted() && this.flags.get(Track::Pending)) {
                                        let guard = this.wallet.guard();
                                        let guard = guard.lock().await;

                                        let include_utxos = this.flags.get(Track::Utxo);
                                        let tx = record.format_transaction_with_state(&this.wallet,Some("pending"),include_utxos, &guard).await;
                                        tx.iter().for_each(|line|tprintln!(this,"{NOTIFY} {line}"));
                                    }
                                },
                                Events::Maturity {
                                    record
                                } => {
                                    if !this.is_mutted() || (this.is_mutted() && this.flags.get(Track::Tx)) {
                                        let guard = this.wallet.guard();
                                        let guard = guard.lock().await;

                                        let include_utxos = this.flags.get(Track::Utxo);
                                        let tx = record.format_transaction_with_state(&this.wallet,Some("confirmed"),include_utxos, &guard).await;
                                        tx.iter().for_each(|line|tprintln!(this,"{NOTIFY} {line}"));
                                    }
                                },
                                // Events::Outgoing {
                                //     record
                                // } => {
                                //     if !this.is_mutted() || (this.is_mutted() && this.flags.get(Track::Tx)) {
                                //         let include_utxos = this.flags.get(Track::Utxo);
                                //         let tx = record.format_with_state(&this.wallet,Some("confirmed"),include_utxos).await;
                                //         tx.iter().for_each(|line|tprintln!(this,"{NOTIFY} {line}"));
                                //     }
                                // },
                                // Events::Change {
                                //     record
                                // } => {
                                //     if !this.is_mutted() || (this.is_mutted() && this.flags.get(Track::Tx)) {
                                //         let include_utxos = this.flags.get(Track::Utxo);
                                //         let tx = record.format_with_state(&this.wallet,Some("change"),include_utxos).await;
                                //         tx.iter().for_each(|line|tprintln!(this,"{NOTIFY} {line}"));
                                //     }
                                // },
                                Events::Balance {
                                    balance,
                                    id,
                                } => {

                                    if !this.is_mutted() || (this.is_mutted() && this.flags.get(Track::Balance)) {
                                        let network_id = this.wallet.network_id().expect("missing network type");
                                        let network_type = NetworkType::from(network_id);
                                        let balance_strings = BalanceStrings::from((balance.as_ref(),&network_type, None));
                                        let id = id.short();

                                        let mature_utxo_count = balance.as_ref().map(|balance|balance.mature_utxo_count.separated_string()).unwrap_or("N/A".to_string());
                                        let pending_utxo_count = balance.as_ref().map(|balance|balance.pending_utxo_count).unwrap_or(0);

                                        let pending_utxo_info = if pending_utxo_count > 0 {
                                            format!("({} pending)", pending_utxo_count)
                                        } else { "".to_string() };
                                        let utxo_info = style(format!("{mature_utxo_count} UTXOs {pending_utxo_info}")).dim();

                                        tprintln!(this, "{NOTIFY} {} {id}: {balance_strings}   {utxo_info}",style("balance".pad_to_width(8)).blue());
                                    }

                                    // Throttled: on a 10 BPS network a wallet holding the
                                    // mining payout address gets a Balance event nearly
                                    // every block; the unconditional redraw here repainted
                                    // the prompt ~10x/sec regardless of mute, making
                                    // typing next to impossible.
                                    if last_balance_refresh.elapsed() >= Duration::from_millis(1000) {
                                        last_balance_refresh = Instant::now();
                                        this.term().refresh_prompt();
                                    }


                                }
                            }
                        }
                    }
                }
            }

            this.notifications_task_ctl
                .response
                .sender
                .send(())
                .await
                .unwrap_or_else(|err| log_error!("WalletCli::notification_pipe_task() unable to signal task shutdown: `{err}`"));
        });
    }

    // ---

    /// Asks uses for a wallet secret, checks the supplied account's private key info
    /// and if it requires a payment secret, asks for it as well.
    pub(crate) async fn ask_wallet_secret(&self, account: Option<&Arc<dyn Account>>) -> Result<(Secret, Option<Secret>)> {
        let secrets = self.ask_wallet_secret_without_otp(account).await?;
        // Every moment that stops to ask for a password is a moment worth a
        // second factor — spends, exports, key changes. Gating here rather
        // than at each caller means a command added next year is covered by
        // default instead of by remembering.
        self.require_otp("this goes ahead").await?;
        Ok(secrets)
    }

    /// The password prompt alone. Only for the two places that must not
    /// demand a code: `otp off`, which exists for somebody whose phone is
    /// gone, and the enrolment that has not stored a secret yet.
    pub(crate) async fn ask_wallet_secret_without_otp(&self, account: Option<&Arc<dyn Account>>) -> Result<(Secret, Option<Secret>)> {
        // Re-ask on empty input instead of proceeding to a guaranteed decrypt
        // failure: an empty answer here has historically meant a stray Enter
        // reached the prompt, not an intentional empty password — and the
        // failure path trained users to retype their password at the normal
        // command prompt (cleartext, history). Ctrl+C still aborts.
        let mut attempts = 0;
        let wallet_secret = loop {
            let entered = self.term().ask(true, "Enter wallet password: ").await?.trim().as_bytes().to_vec();
            if !entered.is_empty() {
                break Secret::new(entered);
            }
            attempts += 1;
            if attempts >= 3 {
                return Err(Error::custom("no password entered"));
            }
            tprintln!(self, "Password was empty — try again (Ctrl+C to abort).");
        };

        let payment_secret = if let Some(account) = account {
            if self.wallet().is_account_key_encrypted(account).await?.is_some_and(|f| f) {
                Some(Secret::new(self.term().ask(true, "Enter payment password: ").await?.trim().as_bytes().to_vec()))
            } else {
                None
            }
        } else {
            None
        };

        Ok((wallet_secret, payment_secret))
    }

    /// The authenticator enrolled on the open wallet, if any.
    pub(crate) fn otp(&self) -> Option<kaspa_wallet_core::storage::Otp> {
        self.wallet().store().otp().ok().flatten()
    }

    /// Forget that a code was ever accepted. Called when a wallet closes, so
    /// that opening a different one does not inherit the last one's grace.
    pub(crate) fn reset_otp_session(&self) {
        *self.otp_session.lock().unwrap() = OtpSession::default();
    }

    /// Record a code as accepted. Split out so `otp enable` can arm the grace
    /// with the code the user just proved on their phone, rather than asking
    /// for a second one a breath later.
    pub(crate) fn note_otp_accepted(&self, step: u64, at: u64) {
        let mut session = self.otp_session.lock().unwrap();
        session.verified_at = Some(at);
        session.used_steps.insert(step);
    }

    /// Ask for a code, if one is enrolled. `Ok(())` when there is no
    /// authenticator, when a previous code is still inside the grace window,
    /// or when a fresh code is accepted. Otherwise the command is refused.
    ///
    /// `what` completes "...before <what>" and is there so the prompt says
    /// what is about to happen — a code demanded with no stated reason is a
    /// code people learn to type without looking.
    pub(crate) async fn require_otp(&self, what: &str) -> Result<()> {
        let Some(otp) = self.otp() else { return Ok(()) };
        let now = kaspa_wallet_core::storage::otp::unix_now_secs();

        if otp.grace_secs > 0 {
            let within = self.otp_session.lock().unwrap().verified_at.is_some_and(|at| now.saturating_sub(at) < otp.grace_secs);
            if within {
                return Ok(());
            }
        }

        tprintln!(self, "");
        tprintln!(self, "{}", style(format!("Authenticator code required before {what}.")).cyan());

        for remaining_tries in (0..3).rev() {
            let now = kaspa_wallet_core::storage::otp::unix_now_secs();
            let seconds = kaspa_wallet_core::storage::otp::Otp::seconds_remaining(now);
            // A code with three seconds left on it will have expired by the
            // time it is typed, and the failure looks like a broken phone
            // rather than a race. Say so before they start.
            if seconds <= 3 {
                tprintln!(self, "{}", style("(your code is about to change — wait for the next one)").dim());
            }
            let entered = self.term().ask(false, "Code from your authenticator: ").await?;
            let entered = entered.trim().to_string();
            if entered.is_empty() {
                return Err(Error::custom("cancelled — no code entered"));
            }

            match otp.step_of(&entered, now) {
                Some(step) if self.otp_session.lock().unwrap().used_steps.contains(&step) => {
                    tprintln!(self, "{}", style("That code has already been used. Wait for your phone to show the next one.").red());
                }
                Some(step) => {
                    self.note_otp_accepted(step, now);
                    return Ok(());
                }
                None if remaining_tries > 0 => {
                    tprintln!(
                        self,
                        "{}",
                        style(format!("That code is not right — {remaining_tries} more {}.", if remaining_tries == 1 { "try" } else { "tries" }))
                            .red()
                    );
                }
                None => {}
            }
        }

        // Deliberately not a hint about clock skew or wrong wallets. Whoever
        // is typing either has the phone or does not.
        Err(Error::custom("authenticator code not accepted"))
    }

    pub async fn account(&self) -> Result<Arc<dyn Account>> {
        if let Ok(account) = self.wallet.account() {
            Ok(account)
        } else {
            let account = self.select_account().await?;
            self.wallet.select(Some(&account)).await?;
            Ok(account)
        }
    }

    pub async fn find_accounts_by_name_or_id(&self, pat: &str) -> Result<Arc<dyn Account>> {
        let matches = self.wallet().find_accounts_by_name_or_id(pat).await?;
        if matches.is_empty() {
            Err(Error::AccountNotFound(pat.to_string()))
        } else if matches.len() > 1 {
            Err(Error::AmbiguousAccount(pat.to_string()))
        } else {
            Ok(matches[0].clone())
        }
    }

    pub async fn prompt_account(&self) -> Result<Arc<dyn Account>> {
        self.select_account_with_args(false).await
    }

    pub async fn select_account(&self) -> Result<Arc<dyn Account>> {
        self.select_account_with_args(true).await
    }

    async fn select_account_with_args(&self, autoselect: bool) -> Result<Arc<dyn Account>> {
        let guard = self.wallet.guard();
        let guard = guard.lock().await;

        let mut selection = None;

        let mut list_by_key = Vec::<(Arc<PrvKeyDataInfo>, Vec<(usize, Arc<dyn Account>)>)>::new();
        let mut flat_list = Vec::<Arc<dyn Account>>::new();

        let mut keys = self.wallet.keys().await?;
        while let Some(key) = keys.try_next().await? {
            let mut prv_key_accounts = Vec::new();
            let mut accounts = self.wallet.accounts(Some(key.id), &guard).await?;
            while let Some(account) = accounts.next().await {
                let account = account?;
                prv_key_accounts.push((flat_list.len(), account.clone()));
                flat_list.push(account.clone());
            }

            list_by_key.push((key.clone(), prv_key_accounts));
        }

        let mut watch_accounts = Vec::<(usize, Arc<dyn Account>)>::new();
        let mut unfiltered_accounts = self.wallet.accounts(None, &guard).await?;

        while let Some(account) = unfiltered_accounts.try_next().await? {
            if account.feature().is_some() {
                watch_accounts.push((flat_list.len(), account.clone()));
                flat_list.push(account.clone());
            }
        }

        if flat_list.is_empty() {
            return Err(Error::NoAccounts);
        } else if autoselect && flat_list.len() == 1 {
            return Ok(flat_list.pop().unwrap());
        }

        while selection.is_none() {
            tprintln!(self);

            list_by_key.iter().for_each(|(prv_key_data_info, accounts)| {
                tprintln!(self, "• {prv_key_data_info}");

                accounts.iter().for_each(|(seq, account)| {
                    // 1-based for humans (the internal index stays 0-based).
                    let seq = style((seq + 1).to_string()).cyan();
                    let ls_string = account.get_list_string().unwrap_or_else(|err| panic!("{err}"));
                    tprintln!(self, "    {seq}: {ls_string}");
                })
            });

            if !watch_accounts.is_empty() {
                tprintln!(self, "• watch-only");
            }

            watch_accounts.iter().for_each(|(seq, account)| {
                let seq = style((seq + 1).to_string()).cyan();
                let ls_string = account.get_list_string().unwrap_or_else(|err| panic!("{err}"));
                tprintln!(self, "    {seq}: {ls_string}");
            });

            tprintln!(self);

            let range = if flat_list.len() > 1 { format!("[{}..{}] ", 1, flat_list.len()) } else { "".to_string() };

            let text =
                self.term().ask(false, &format!("Please select account {}or <enter> to abort: ", range)).await?.trim().to_string();
            if text.is_empty() {
                return Err(Error::UserAbort);
            } else {
                match text.parse::<usize>() {
                    Ok(seq) if seq >= 1 && seq <= flat_list.len() => selection = flat_list.get(seq - 1).cloned(),
                    _ => {}
                };
            }
        }

        let account = selection.unwrap();
        let ident = style(account.name_with_id()).blue();
        tprintln!(self, "selecting account: {ident}");

        Ok(account)
    }

    pub async fn select_private_key(&self) -> Result<Arc<PrvKeyDataInfo>> {
        self.select_private_key_with_args(true).await
    }

    pub async fn select_private_key_with_args(&self, autoselect: bool) -> Result<Arc<PrvKeyDataInfo>> {
        let mut selection = None;

        // let mut list_by_key = Vec::<(Arc<PrvKeyDataInfo>, Vec<(usize, Arc<dyn Account>)>)>::new();
        let mut flat_list = Vec::<Arc<PrvKeyDataInfo>>::new();

        let mut keys = self.wallet.keys().await?;
        while let Some(key) = keys.try_next().await? {
            flat_list.push(key);
        }

        if flat_list.is_empty() {
            return Err(Error::NoKeys);
        } else if autoselect && flat_list.len() == 1 {
            return Ok(flat_list.pop().unwrap());
        }

        while selection.is_none() {
            tprintln!(self);

            flat_list.iter().enumerate().for_each(|(seq, prv_key_data_info)| {
                tprintln!(self, "    {seq}: {prv_key_data_info}");
            });

            tprintln!(self);

            let range = if flat_list.len() > 1 { format!("[{}..{}] ", 0, flat_list.len() - 1) } else { "".to_string() };

            let text =
                self.term().ask(false, &format!("Please select private key {}or <enter> to abort: ", range)).await?.trim().to_string();
            if text.is_empty() {
                return Err(Error::UserAbort);
            } else {
                match text.parse::<usize>() {
                    Ok(seq) if seq < flat_list.len() => selection = flat_list.get(seq).cloned(),
                    _ => {}
                };
            }
        }

        let prv_key_data_info = selection.unwrap();
        tprintln!(self, "\nselecting private key: {prv_key_data_info}\n");

        Ok(prv_key_data_info)
    }

    pub async fn list(&self) -> Result<()> {
        let guard = self.wallet.guard();
        let guard = guard.lock().await;

        let mut keys = self.wallet.keys().await?;

        tprintln!(self);
        // Show ACCOUNTS — what a person actually has. The keys underneath are
        // plumbing: a wallet can carry keys with no accounts (left over from
        // an interrupted create), and listing those made a wallet look full of
        // mystery entries. 'details' has the expert view; 'wallet tidy'
        // removes unused keys.
        let mut printed_accounts = 0usize;
        while let Some(key) = keys.try_next().await? {
            let mut accounts = self.wallet.accounts(Some(key.id), &guard).await?;
            while let Some(account) = accounts.try_next().await? {
                let receive_address = account.receive_address()?;
                // An unknown ledger balance is not "N/A" — it means the coins
                // have not been read yet. Say what is actually happening
                // rather than printing a value that looks like zero.
                if account.balance().is_none() {
                    let status = if self.wallet.is_connected() {
                        "connecting..."
                    } else {
                        "not connected — 'connect <node>' to read the ledger"
                    };
                    tprintln!(self, "• {}: {}", style(account.name_with_id()).blue(), style(status).dim());
                } else {
                    tprintln!(self, "• {}", account.get_list_string()?);
                }
                tprintln!(self, "  {}", style(receive_address.to_string()).blue());
                printed_accounts += 1;
            }
        }
        if printed_accounts == 0 {
            tprintln!(self, "No accounts yet — create one with 'account create'");
        }

        let mut unfiltered_accounts = self.wallet.accounts(None, &guard).await?;
        let mut feature_header_printed = false;
        while let Some(account) = unfiltered_accounts.try_next().await? {
            if let Some(feature) = account.feature() {
                if !feature_header_printed {
                    tprintln!(self, "{}", style("• watch-only").dim());
                    feature_header_printed = true;
                }
                tprintln!(self, "  • {}", account.get_list_string().unwrap());
                tprintln!(self, "      • {}", style(feature).cyan());
            }
        }
        tprintln!(self);

        // Discoverability nudge: ledger balance but no notes yet — the note
        // pool is the product; nobody should have to guess its entry point.
        if let Ok(account) = self.wallet.account() {
            let mature = account.balance().map(|b| b.mature).unwrap_or(0);
            if mature > 0 {
                let has_notes = match self.wallet.store().as_note_key_store() {
                    Ok(store) => match store.iter().await {
                        Ok(mut stream) => stream.try_next().await.ok().flatten().is_some(),
                        Err(_) => true,
                    },
                    Err(_) => true,
                };
                if !has_notes {
                    tprintln!(self, "Tip: turn ledger balance into bearer notes with 'note mint <amount>' (or 'note mint all')");
                    tprintln!(self);
                }
            }
        }

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        if !self.shutdown.load(Ordering::SeqCst) {
            self.shutdown.store(true, Ordering::SeqCst);

            tprintln!(self, "{}", style("shutting down...").magenta());

            let miner = self.daemons().try_cpu_miner();
            let kaspad = self.daemons().try_kaspad();

            if let Some(miner) = miner.as_ref() {
                miner.mute(false).await?;
                miner.stop().await?;
            }

            if let Some(kaspad) = kaspad.as_ref() {
                kaspad.mute(false).await?;
                kaspad.stop().await?;
            }

            if let Some(miner) = miner.as_ref() {
                miner.join().await?;
            }

            if let Some(kaspad) = kaspad.as_ref() {
                kaspad.join().await?;
            }

            self.term().exit().await;
        }

        Ok(())
    }

    fn sync_state(&self) -> Option<String> {
        if let Some(state) = self.sync_state.lock().unwrap().as_ref() {
            match state {
                SyncState::Proof { level } => {
                    if *level == 0 {
                        Some([style("SYNC").red().to_string(), style("...").black().to_string()].join(" "))
                    } else {
                        Some([style("SYNC PROOF").red().to_string(), style(level.separated_string()).dim().to_string()].join(" "))
                    }
                }
                SyncState::Headers { headers, progress } => Some(
                    [
                        style("SYNC IBD HDRS").red().to_string(),
                        style(format!("{} ({}%)", headers.separated_string(), progress)).dim().to_string(),
                    ]
                    .join(" "),
                ),
                SyncState::Blocks { blocks, progress } => Some(
                    [
                        style("SYNC IBD BLOCKS").red().to_string(),
                        style(format!("{} ({}%)", blocks.separated_string(), progress)).dim().to_string(),
                    ]
                    .join(" "),
                ),
                SyncState::TrustSync { processed, total } => {
                    let progress = processed * 100 / total;
                    Some(
                        [
                            style("SYNC TRUST").red().to_string(),
                            style(format!("{} ({}%)", processed.separated_string(), progress)).dim().to_string(),
                        ]
                        .join(" "),
                    )
                }
                SyncState::UtxoSync { total, .. } => {
                    Some([style("SYNC UTXO").red().to_string(), style(total.separated_string()).dim().to_string()].join(" "))
                }
                SyncState::SmtSync { processed, total } => Some(
                    [
                        style("SYNC SMT").red().to_string(),
                        style(format!("{} of {}", processed.separated_string(), total.separated_string())).dim().to_string(),
                    ]
                    .join(" "),
                ),
                SyncState::UtxoResync => Some([style("SYNC").red().to_string(), style("UTXO").black().to_string()].join(" ")),
                SyncState::NotSynced => Some([style("SYNC").red().to_string(), style("...").black().to_string()].join(" ")),
                SyncState::Synced => None,
            }
        } else {
            Some(style("SYNC").red().to_string())
        }
    }
}

#[async_trait]
impl Cli for KaspaCli {
    fn init(self: Arc<Self>, term: &Arc<Terminal>) -> TerminalResult<()> {
        *self.term.lock().unwrap() = Some(term.clone());

        self.notifier().try_init()?;

        term.register_event_handler(Arc::new(Box::new(move |event| match event {
            TerminalEvent::Copy | TerminalEvent::Paste => {
                self.notifier().notify(Notification::Clipboard);
            }
        })))?;

        Ok(())
    }

    async fn digest(self: Arc<Self>, term: Arc<Terminal>, cmd: String) -> TerminalResult<()> {
        *self.last_interaction.lock().unwrap() = Instant::now();
        // '<command> [...] help' (or '?') explains the command instead of
        // running it — discoverability rule, wallet-UX refinements 2026-09-05.
        let tokens: Vec<&str> = cmd.split_whitespace().collect();
        if tokens.len() >= 2 && matches!(tokens.last().map(|s| s.to_lowercase()).as_deref(), Some("help") | Some("?")) {
            let verb = tokens[0].to_lowercase();
            let ctx: Arc<dyn Context> = self.clone();
            if let Some(handler) = self.handlers.get(&verb) {
                term.writeln(format!("\n{} — {}", verb, get_handler_help(handler, &ctx)));
                // Commands with sub-command tables print them too.
                if matches!(verb.as_str(), "wallet" | "note" | "account" | "history" | "node" | "miner") {
                    if let Err(err) = self.handlers.execute(&self, &format!("{verb} help")).await {
                        term.writeln(style(err.to_string()).red().to_string());
                    }
                } else {
                    term.writeln("");
                }
                return Ok(());
            }
        }
        if let Err(err) = self.handlers.execute(&self, &cmd).await {
            // Backing out of a prompt is a choice, not a fault. Ctrl+C at a
            // picker printed "Cli error cancelled" in red, which reads as
            // something having gone wrong when the user simply changed their
            // mind. Say nothing and hand the prompt back.
            let text = err.to_string();
            if !matches!(text.as_str(), "Cli error cancelled" | "cancelled" | "Aborted") {
                term.writeln(style(text).red().to_string());
            }
        }
        Ok(())
    }

    async fn complete(self: Arc<Self>, _term: Arc<Terminal>, cmd: String) -> TerminalResult<Option<Vec<String>>> {
        // Tab completion: returns FULL-LINE candidates (the terminal's
        // contract). First word completes against registered verbs; known
        // multi-command verbs complete their subcommands; wallet-name slots
        // complete against the wallet files on disk.
        let ctx: Arc<dyn Context> = self.clone();
        let trailing_space = cmd.ends_with(' ');
        let tokens: Vec<String> = cmd.split_whitespace().map(String::from).collect();

        let subcommands = |verb: &str, sub: &str| -> Option<Vec<&'static str>> {
            match (verb, sub) {
                ("note", "vault") => Some(vec!["create", "backup", "verify", "restore", "export", "import"]),
                ("note", _) => Some(vec![
                    "mint", "rotate", "move", "mirror", "redeem", "request", "pay", "import", "export", "pos", "balance", "list", "history", "verify", "vault",
                    "help",
                ]),
                ("wallet", _) => Some(vec![
                    "list", "create", "import", "open", "close", "where", "autoconnect", "forget", "show", "rename", "tidy", "destroy", "hint", "help",
                ]),
                ("account", _) => Some(vec!["create", "import", "name", "recover", "watch", "help"]),
                ("history", _) => Some(vec!["list", "details"]),
                ("settings", _) => Some(vec!["set"]),
                ("track", _) => Some(vec!["balance", "pending", "tx", "utxo", "daa"]),
                ("network", _) => Some(vec!["mainnet", "testnet-10"]),
                ("node", _) | ("miner", _) => Some(vec!["start", "stop", "restart", "status"]),
                ("utxos", _) => Some(vec!["all"]),
                ("auto", _) => Some(vec!["on", "off", "sweep", "verbose"]),
                _ => None,
            }
        };

        let complete_token = |prefix: &str, candidates: Vec<String>, head: &[String]| -> Vec<String> {
            let head = if head.is_empty() { String::new() } else { format!("{} ", head.join(" ")) };
            candidates
                .into_iter()
                .filter(|c| c.starts_with(prefix) )
                .map(|c| format!("{head}{c}"))
                .collect()
        };

        let verbs: Vec<String> = {
            let handlers = self.handlers.collect();
            let mut verbs: Vec<String> = handlers.into_iter().filter_map(|h| h.verb(&ctx).map(String::from)).collect();
            verbs.sort();
            verbs
        };

        let candidates: Vec<String> = match (tokens.len(), trailing_space) {
            (0, _) => verbs,
            (1, false) => complete_token(&tokens[0], verbs, &[]),
            _ => {
                let verb = tokens[0].to_lowercase();
                let (head, prefix): (&[String], &str) =
                    if trailing_space { (&tokens[..], "") } else { (&tokens[..tokens.len() - 1], tokens.last().unwrap()) };
                // wallet-name slots: 'open <name>', 'wallet open|destroy <name>'
                let wallet_name_slot = (verb == "open" && head.len() == 1)
                    || (verb == "wallet" && head.len() == 2 && matches!(head[1].as_str(), "open" | "destroy"));
                if wallet_name_slot {
                    let names = self
                        .store()
                        .wallet_list()
                        .await
                        .map(|list| list.into_iter().map(|w| w.filename).collect::<Vec<_>>())
                        .unwrap_or_default();
                    complete_token(prefix, names, head)
                } else {
                    let sub = tokens.get(1).map(|s| s.as_str()).unwrap_or("");
                    match subcommands(&verb, sub) {
                        Some(subs) if head.len() <= 2 || (verb == "note" && sub == "vault" && head.len() <= 3) => {
                            complete_token(prefix, subs.into_iter().map(String::from).collect(), head)
                        }
                        _ => vec![],
                    }
                }
            }
        };

        Ok(Some(candidates))
    }

    fn prompt(&self) -> Option<String> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Some("halt $ ".to_string());
        }

        let mut prompt = vec![];

        #[cfg(not(feature = "embedded-node"))]
        let node_running = if let Some(node) = self.node.lock().unwrap().as_ref() { node.is_running() } else { false };
        #[cfg(feature = "embedded-node")]
        let node_running = self.embedded_node_running();

        let _miner_running = if let Some(miner) = self.miner.lock().unwrap().as_ref() { miner.is_running() } else { false };

        // match (node_running, miner_running) {
        //     (true, true) => prompt.push(style("NM").green().to_string()),
        //     (true, false) => prompt.push(style("N").green().to_string()),
        //     (false, true) => prompt.push(style("M").green().to_string()),
        //     _ => {}
        // }

        if (self.wallet.is_open() && !self.wallet.is_connected()) || (node_running && !self.wallet.is_connected()) {
            // "N/C" meant nothing to anyone who had not written it. The word
            // costs a few columns and vanishes the moment you connect.
            prompt.push(style("DISCONNECTED").red().to_string());
        } else if self.wallet.is_connected()
            && !self.wallet.is_synced()
            && let Some(state) = self.sync_state()
        {
            prompt.push(state);
        }

        if let Some(descriptor) = self.wallet.descriptor() {
            let title = descriptor.title.unwrap_or(descriptor.filename);
            if title.to_lowercase().as_str() != "marigold" {
                prompt.push(title);
            }

            if let Ok(account) = self.wallet.account() {
                prompt.push(style(account.name_with_id()).blue().to_string());

                // Stable-width balance: fixed 8 decimals and a monotonic
                // session pad, so the prompt (and the text being typed at it)
                // never jumps as per-block balance updates change digit
                // counts or the pending segment appears/disappears.
                // What you hold, in one number: notes plus ledger, two
                // decimals, refreshed once a minute. The ledger's piece count
                // and its per-block churn are plumbing — 'balance' has the
                // precise figures when they are wanted.
                if self.prompt_total_valid.load(Ordering::SeqCst) {
                    let petals = self.prompt_total_petals.load(Ordering::SeqCst);
                    let whole = petals / 100_000_000;
                    let hundredths = (petals % 100_000_000) / 1_000_000;
                    let suffix = self
                        .wallet
                        .network_id()
                        .map(|id| kaspa_wallet_core::utils::kaspa_suffix(&NetworkType::from(id)))
                        .unwrap_or("");
                    let segment = format!("{}.{:02} {suffix}", whole.separated_string(), hundredths);
                    let width = self.prompt_balance_width.fetch_max(segment.len(), Ordering::SeqCst).max(segment.len());
                    prompt.push(segment.pad_to_width(width));
                } else {
                    prompt.push("...".to_string());
                }
            }
        }

        prompt.is_not_empty().then(|| prompt.join(" • ") + " $ ")
    }
}

impl cli::Context for KaspaCli {
    fn term(&self) -> Arc<Terminal> {
        self.term.lock().unwrap().as_ref().unwrap().clone()
    }
}

impl KaspaCli {}

#[allow(dead_code)]
async fn select_item<T>(
    term: &Arc<Terminal>,
    prompt: &str,
    argv: &mut Vec<String>,
    iter: impl Stream<Item = Result<Arc<T>>>,
) -> Result<Arc<T>>
where
    T: std::fmt::Display + IdT + Clone + Send + Sync + 'static,
{
    let mut selection = None;
    let list = iter.try_collect::<Vec<_>>().await?;

    if !argv.is_empty() {
        let text = argv.remove(0);
        let matched = list
            .into_iter()
            // - TODO match by name
            .filter(|item| item.id().to_hex().starts_with(&text))
            .collect::<Vec<_>>();

        if matched.len() == 1 {
            return Ok(matched.first().cloned().unwrap());
        } else {
            return Err(Error::MultipleMatches(text));
        }
    }

    while selection.is_none() {
        list.iter().enumerate().for_each(|(seq, item)| {
            term.writeln(format!("{}: {} ({})", seq, item, item.id().to_hex()));
        });

        let text = term.ask(false, &format!("{prompt} ({}..{}) or <enter> to abort: ", 0, list.len() - 1)).await?.trim().to_string();
        if text.is_empty() {
            term.writeln("aborting...");
            return Err(Error::UserAbort);
        } else {
            match text.parse::<usize>() {
                Ok(seq) if seq < list.len() => selection = list.get(seq).cloned(),
                _ => {}
            };
        }
    }

    Ok(selection.unwrap())
}

// async fn select_variant<T>(term: &Arc<Terminal>, prompt: &str, argv: &mut Vec<String>) -> Result<T>
// where
//     T: ToString + DeserializeOwned + Clone + Serialize,
// {
//     if !argv.is_empty() {
//         let text = argv.remove(0);
//         if let Ok(v) = serde_json::from_str::<T>(text.as_str()) {
//             return Ok(v);
//         } else {
//             let accepted = T::list().iter().map(|v| serde_json::to_string(v).unwrap()).collect::<Vec<_>>().join(", ");
//             return Err(Error::UnrecognizedArgument(text, accepted));
//         }
//     }

//     let mut selection = None;
//     let list = T::list();
//     while selection.is_none() {
//         list.iter().enumerate().for_each(|(seq, item)| {
//             let name = serde_json::to_string(item).unwrap();
//             term.writeln(format!("{}: '{name}' - {}", seq, item.descr()));
//         });

//         let text = term.ask(false, &format!("{prompt} ({}..{}) or <enter> to abort: ", 0, list.len() - 1)).await?.trim().to_string();
//         if text.is_empty() {
//             term.writeln("aborting...");
//             return Err(Error::UserAbort);
//         } else if let Ok(v) = serde_json::from_str::<T>(text.as_str()) {
//             selection = Some(v);
//         } else {
//             match text.parse::<usize>() {
//                 Ok(seq) if seq > 0 && seq < list.len() => selection = list.get(seq).cloned(),
//                 _ => {}
//             };
//         }
//     }

//     Ok(selection.unwrap())
// }

pub async fn kaspa_cli(terminal_options: TerminalOptions, banner: Option<String>) -> Result<()> {
    KaspaCli::init();

    let options = Options::new(terminal_options, None);
    let cli = KaspaCli::try_new_arc(options).await?;

    // 'connect' first, because it is: nothing works until the wallet can reach
    // a node, and 'help' answers a question a new user has not formed yet.
    let banner = banner
        .unwrap_or_else(|| format!("Marigold Cli Wallet v{} (type 'connect' or 'help' for list of commands)", env!("CARGO_PKG_VERSION")));
    cli.term().writeln(banner);

    // redirect the global log output to terminal
    #[cfg(not(target_arch = "wasm32"))]
    workflow_log::pipe(Some(cli.clone()));
    // ...and the `log` crate's, which is what everything inside the node uses.
    #[cfg(not(target_arch = "wasm32"))]
    crate::log_sink::attach(&cli);

    cli.register_handlers()?;

    // cli starts notification->term trace pipe task
    cli.start().await?;

    // terminal blocks async execution, delivering commands to the terminals
    cli.run().await?;

    // cli stops notification->term trace pipe task
    cli.stop().await?;

    Ok(())
}

mod panic_handler {
    use regex::Regex;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = console, js_name="error")]
        pub fn console_error(msg: String);

        type Error;

        #[wasm_bindgen(constructor)]
        fn new() -> Error;

        #[wasm_bindgen(structural, method, getter)]
        fn stack(error: &Error) -> String;
    }

    pub fn process(info: &std::panic::PanicHookInfo) -> String {
        let mut msg = info.to_string();

        // Add the error stack to our message.
        //
        // This ensures that even if the `console` implementation doesn't
        // include stacks for `console.error`, the stack is still available
        // for the user. Additionally, Firefox's console tries to clean up
        // stack traces, and ruins Rust symbols in the process
        // (https://bugzilla.mozilla.org/show_bug.cgi?id=1519569) but since
        // it only touches the logged message's associated stack, and not
        // the message's contents, by including the stack in the message
        // contents we make sure it is available to the user.

        msg.push_str("\n\nStack:\n\n");
        let e = Error::new();
        let stack = e.stack();

        let regex = Regex::new(r"chrome-extension://[^/]+").unwrap();
        let stack = regex.replace_all(&stack, "");

        msg.push_str(&stack);

        // Safari's devtools, on the other hand, _do_ mess with logged
        // messages' contents, so we attempt to break their heuristics for
        // doing that by appending some whitespace.
        // https://github.com/rustwasm/console_error_panic_hook/issues/7

        msg.push_str("\n\n");

        msg
    }
}

impl KaspaCli {
    pub fn init_panic_hook(self: &Arc<Self>) {
        let this = self.clone();
        let handler = move |info: &std::panic::PanicHookInfo| {
            let msg = panic_handler::process(info);
            this.term().writeln(msg.crlf());
            panic_handler::console_error(msg);
        };

        std::panic::set_hook(Box::new(handler));

        // #[cfg(target_arch = "wasm32")]
        workflow_log::pipe(Some(self.clone()));
    }
}
