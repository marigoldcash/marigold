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
use kaspa_wallet_keys::guarded::Guarded;
use kaspa_wrpc_client::{KaspaRpcClient, Resolver};
use std::sync::atomic::{AtomicU64, AtomicUsize};
use workflow_core::channel::*;
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

/// One sompi per gram: the own lane's fee rate — a rounding error, and paid to ourselves.
pub const OWN_LANE_FEE_RATE: f64 = 1.0;
/// The longest a block of our own may be expected to take before the wallet
/// pays the network fee instead of waiting for it.
pub const OWN_LANE_MAX_WAIT: Duration = Duration::from_secs(3600);

/// Whether transactions this wallet submits will be mined by its own miner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OwnLane {
    /// Yes, with a block of our own expected about this often.
    Use { every: Duration },
    /// We mine, but a block of our own is too rare to wait for.
    TooSlow { every: Duration },
    /// On our own copy of the network, but not mining.
    NoMiner,
    /// Not on our own copy of the network at all.
    NotOwnCopy,
}

/// A spend priced for a block of our own, and what to do if that block never
/// comes: see [`KaspaCli::retry_stale_own_lane_spends`].
pub struct OwnLaneSpend {
    pending: kaspa_wallet_core::tx::PendingTransaction,
    submitted: Instant,
    every: Duration,
    /// For a mint: the notes this transaction creates, whose serials are
    /// derived from its id and so move with a replacement.
    mint_notes: Vec<kaspa_wallet_core::storage::NoteKeyEntry>,
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
    /// Held for the session, as two random-looking halves; see `Guarded`.
    auto_secret: Mutex<Option<Guarded>>,
    /// The in-process node, once started. Held here so `node stop` and wallet
    /// shutdown can reach it; `None` means we are talking to someone else's.
    #[cfg(feature = "embedded-node")]
    embedded_node: Mutex<Option<Arc<crate::embedded::EmbeddedNode>>>,
    /// 'advanced on': the technical side shown — every command in help,
    /// addresses, the reasons behind errors. Off by default; persisted.
    advanced: AtomicBool,
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
    /// A miner program running in the background on this machine, on the
    /// node this wallet is connected to (PLAN P8.3c). Its miner is ours:
    /// 'mine' steers it and the own lane counts on it. `remote_mining` is
    /// what it last said it was doing.
    remote_miner: Arc<AtomicBool>,
    remote_mining: Arc<AtomicBool>,
    /// The Telegram bot answered from inside this program while a paired
    /// wallet is open (PLAN P8.0h), so nothing has to be closed to use
    /// the phone. Aborted on close and on exit.
    #[cfg(feature = "embedded-node")]
    telegram_bot: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// True while the UTXO set is being read in, which on a wallet that has
    /// been mined into is minutes of work with nothing to show for it.
    loading: Arc<AtomicBool>,
    auto_payment_secret: Mutex<Option<Guarded>>,
    /// Spends priced for our own block, watched until they land; see
    /// `retry_stale_own_lane_spends`.
    own_lane_spends: Mutex<Vec<OwnLaneSpend>>,
    /// While an own-lane operation runs: the expected time to a block of ours,
    /// so every transaction it submits is remembered with it.
    own_lane_capture: Mutex<Option<Duration>>,
    /// The version the node we are on reported at connect.
    connected_server_version: Mutex<Option<String>>,
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
    /// Whether the wallet's view of the ledger can be stated as fact.
    ///
    /// It cannot, after a reload that failed: the reload clears the UTXO
    /// context and repopulates it from the node, so a failure leaves an empty
    /// context that reads as a balance of zero with zero coins in it. The
    /// wallet then told somebody holding 812,524 TMAGLD across 4.4 million
    /// coins that their ledger was empty (founder report, 2026-09-13).
    ///
    /// Zero and "not known" are different facts and only one of them is
    /// alarming. Nothing may print a ledger figure while this is false.
    ledger_known: Arc<AtomicBool>,
    /// A 'connect' or 'disconnect' is running: the disconnection it causes
    /// is its own doing and is not announced as a lost link.
    switching: Arc<AtomicBool>,
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

/// Transactions an automatic consolidation pass submits before stopping.
///
/// Housekeeping is one task doing minting and consolidation in sequence, so an
/// unbounded sweep on a large wallet stops the wallet minting for as long as it
/// takes. This bounds a pass to something that finishes inside the minute
/// between passes.
const SWEEP_TRANSACTIONS_PER_PASS_CONST: usize = 200;

/// Coins a consolidation transaction takes in, near enough.
///
/// The generator packs inputs until four fifths of the standard mass limit,
/// which lands around here. Only used to describe how long a backlog will
/// take, so approximate is what it needs to be.
const COINS_PER_SWEEP_TRANSACTION: u64 = 80;

/// Above this many ledger coins, housekeeping stops and asks.
///
/// Derived rather than picked: it is about fifteen minutes of automatic
/// consolidation. Below that the backlog is background work; above it, income
/// has been outrunning consolidation for long enough that somebody should
/// look, and hours of unasked-for work on somebody's money should be their
/// decision.
const AUTOMATIC_HOUSEKEEPING_CEILING: u64 = 250_000;

/// Ledger coins an automatic pass gets through in a minute.
fn coins_per_minute() -> u64 {
    SWEEP_TRANSACTIONS_PER_PASS_CONST as u64 * COINS_PER_SWEEP_TRANSACTION
}

/// "about 4 hours", "about 20 minutes" — a figure somebody can plan around,
/// not one they should hold us to.
pub(crate) fn humanised_minutes(minutes: u64) -> String {
    match minutes {
        0 => "a moment".to_string(),
        1 => "a minute".to_string(),
        m if m < 90 => format!("{m} minutes"),
        m => {
            let hours = (m + 30) / 60;
            format!("{hours} hour{}", if hours == 1 { "" } else { "s" })
        }
    }
}

/// "20 minutes", "3 hours", "4 days": how long a wait is, in the unit a
/// person would pick.
#[cfg(feature = "embedded-node")]
pub(crate) fn humanised_wait(seconds: f64) -> String {
    if seconds < 90.0 {
        return format!("{} seconds", (seconds.round() as u64).max(1));
    }
    let minutes = (seconds / 60.0).round() as u64;
    if minutes < 90 {
        return humanised_minutes(minutes);
    }
    let hours = (minutes + 30) / 60;
    if hours < 48 {
        return format!("{hours} hours");
    }
    format!("{} days", (hours + 12) / 24)
}

/// "about a block a minute", "a block about every 12 minutes": the cadence
/// of a miner, in words that survive the one-minute case ("every a minute"
/// did not, 2026-09-18).
pub(crate) fn block_cadence(every: Duration) -> String {
    match every.as_secs() / 60 {
        0 | 1 => "about a block a minute".to_string(),
        m => format!("a block about every {}", humanised_minutes(m)),
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
            advanced: AtomicBool::new(false),
            #[cfg(feature = "embedded-node")]
            embedded_node_adopted: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "embedded-node")]
            cpu_miner: Mutex::new(None),
            remote_miner: Arc::new(AtomicBool::new(false)),
            remote_mining: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "embedded-node")]
            telegram_bot: Mutex::new(None),
            loading: Arc::new(AtomicBool::new(false)),
            auto_payment_secret: Mutex::new(None),
            own_lane_spends: Mutex::new(Vec::new()),
            own_lane_capture: Mutex::new(None),
            connected_server_version: Mutex::new(None),
            auto_threshold_petals: Arc::new(AtomicU64::new(0)),
            auto_sweep_utxos: Arc::new(AtomicU64::new(0)),
            auto_busy: Arc::new(AtomicBool::new(false)),
            auto_verbose: Arc::new(AtomicBool::new(false)),
            open_housekeeping_pending: Arc::new(AtomicBool::new(false)),
            prompt_total_petals: Arc::new(AtomicU64::new(0)),
            prompt_total_valid: Arc::new(AtomicBool::new(false)),
            prompt_balance_width: Arc::new(AtomicUsize::new(0)),
            otp_session: Mutex::new(OtpSession::default()),
            ledger_known: Arc::new(AtomicBool::new(true)),
            switching: Arc::new(AtomicBool::new(false)),
        });

        let term = Arc::new(Terminal::try_new_with_options(kaspa_cli.clone(), options.terminal)?);
        term.init().await?;

        cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                kaspa_cli.init_panic_hook();
            }
        }

        // Every transaction the wallet submits passes here; the ones sent on
        // the own lane are kept and watched (see `retry_stale_own_lane_spends`).
        let weak = Arc::downgrade(&kaspa_cli);
        kaspa_cli.wallet().utxo_processor().set_submit_observer(Some(Arc::new(move |pending| {
            if let Some(cli) = weak.upgrade() {
                cli.note_submission(pending);
            }
        })));
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

    /// Whether the ledger figure can be stated as fact — see `ledger_known`.
    /// Does this wallet have a ledger account at all? A wallet that keeps
    /// notes only (PLAN P8.0b) has none, and `account()` failing is then
    /// the normal state rather than a selection that has not happened yet.
    ///
    /// Asked of the store, not of `wallet.accounts()`: that needs the wallet
    /// guard, and the `wallet` command holds it for its whole run — including
    /// the holdings report after `wallet open`, which would then wait on
    /// itself forever (found by the P8.0b verify, 2026-09-15).
    pub async fn has_ledger_account(&self) -> bool {
        match self.wallet.store().as_account_store() {
            Ok(store) => !store.is_empty().await.unwrap_or(true),
            Err(_) => false,
        }
    }

    /// The selected account, for commands that need the ledger. On a wallet
    /// that keeps notes only the refusal says what to do about it, instead of
    /// the bare "no account selected" that means something else.
    pub async fn ledger_account(&self) -> Result<Arc<dyn Account>> {
        match self.wallet.account() {
            Ok(account) => Ok(account),
            Err(kaspa_wallet_core::error::Error::AccountSelection) if !self.has_ledger_account().await => {
                Err(Error::custom("This wallet keeps notes only — there is no ledger account. 'account create bip32' adds one."))
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Whether the technical side is shown: every command in 'help',
    /// addresses, and the reasons behind errors. Off by default, because the
    /// front page of a wallet should read like a wallet.
    pub fn advanced(&self) -> bool {
        self.advanced.load(Ordering::SeqCst)
    }

    pub async fn set_advanced(&self, on: bool) {
        self.advanced.store(on, Ordering::SeqCst);
        self.wallet.settings().set(WalletSettings::Advanced, on).await.ok();
    }

    /// An error, as it should be said. With 'advanced on', verbatim. Without,
    /// the wallet's own sentences pass through and anything technical — a
    /// transport chain, an OS error code, a type name — becomes one plain
    /// line that still says what to do next. Amber rather than red: a
    /// mistake is not an alarm. Works from the text because the dispatcher
    /// hands back the terminal's error, which has already flattened ours.
    pub fn describe_error(&self, text: &str) -> String {
        use crate::ui::{self, Ink};
        if self.advanced() {
            return ui::paint(Ink::Amber, text);
        }
        let hint = ui::paint(Ink::Moss, "  ('advanced on' shows the reason)");
        let lower = text.to_ascii_lowercase();
        if ["wrpc", "websocket", "connection refused", "rpc", "not connected", "timed out"].iter().any(|m| lower.contains(m)) {
            format!("{}{hint}", ui::paint(Ink::Amber, "Could not reach the node. 'connect' tries again."))
        } else if is_technical(text) {
            format!("{}{hint}", ui::paint(Ink::Amber, "That did not work."))
        } else {
            ui::paint(Ink::Amber, text)
        }
    }

    /// Has mining ever been started on the open wallet? Decides whether
    /// 'mine' is an everyday command here.
    pub async fn has_mined(&self) -> bool {
        let Some(descriptor) = self.wallet.store().descriptor() else { return false };
        self.wallet.store().client_metadata(&descriptor.filename).await.ok().flatten().map(|m| m.mined).unwrap_or(false)
    }

    pub async fn remember_mined(&self) {
        let Some(descriptor) = self.wallet.store().descriptor() else { return };
        if let Ok(meta) = self.wallet.store().client_metadata(&descriptor.filename).await {
            let mut meta = meta.unwrap_or_default();
            if !meta.mined {
                meta.mined = true;
                self.wallet.store().set_client_metadata(&descriptor.filename, Some(meta)).await.ok();
            }
        }
    }

    /// Answer the wallet's paired Telegram bot from here, with the password
    /// just typed at 'open', for as long as the wallet stays open. Nothing
    /// persisted, nothing listening; the same loop 'serve' runs.
    #[cfg(feature = "embedded-node")]
    pub async fn start_telegram_bot(self: &Arc<Self>, secret: Secret) {
        self.stop_telegram_bot();
        let Some(descriptor) = self.wallet.store().descriptor() else { return };
        let folder: String = self
            .wallet
            .settings()
            .get(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
        let path = crate::telegram::TelegramConfig::path(&folder, &descriptor.filename);
        let Some(cfg) = crate::telegram::TelegramConfig::load(&path) else { return };
        let Ok(network_id) = self.wallet.network_id() else { return };
        let this = self.clone();
        let say: crate::serve::Say = Arc::new(move |line: String| tprintln!(this, "{}", style(line).dim()));
        // A miner started in this terminal, for the bot's /status and /mine.
        let this = self.clone();
        let local_miner: crate::serve::LocalMiner = Arc::new(move || {
            let miner = this.cpu_miner.lock().unwrap().clone()?;
            Some(kaspa_rpc_core::RpcMinerStatus {
                available: true,
                mining: miner.is_running(),
                percent: miner.percent(),
                threads: miner.thread_count() as u32,
                cores: crate::miner::cores() as u32,
                hashrate: miner.hashrate(),
                blocks_found: miner.blocks_found(),
                blocks_accepted: miner.blocks_accepted(),
                blocks_rejected: miner.blocks_rejected(),
                address: String::new(),
                uptime_seconds: miner.uptime().as_secs(),
            })
        });
        let service = crate::serve::WalletService::new(
            self.wallet.clone(),
            secret,
            network_id,
            &folder,
            &descriptor.filename,
            None,
            say,
            Some(local_miner),
        );
        let handle = tokio::spawn(crate::telegram::run_bot(service, path, cfg));
        self.telegram_bot.lock().unwrap().replace(handle);
        tprintln!(self, "{}", style("Answering your Telegram bot while this wallet is open.").dim());
    }

    #[cfg(feature = "embedded-node")]
    pub fn stop_telegram_bot(&self) {
        if let Some(handle) = self.telegram_bot.lock().unwrap().take() {
            handle.abort();
        }
    }

    #[cfg(not(feature = "embedded-node"))]
    pub async fn start_telegram_bot(self: &Arc<Self>, _secret: Secret) {}

    #[cfg(not(feature = "embedded-node"))]
    pub fn stop_telegram_bot(&self) {}

    /// This wallet's payments journal, when a wallet is open.
    pub fn journal(&self) -> Option<kaspa_wallet_core::storage::local::journal::Journal> {
        let descriptor = self.wallet.store().descriptor()?;
        let folder: String = self
            .wallet
            .settings()
            .get(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
        Some(kaspa_wallet_core::storage::local::journal::Journal::new(&folder, &descriptor.filename))
    }

    /// Note money moving, for 'history'. Never fails the caller: a payment
    /// that went through is not undone by a line that could not be written.
    pub fn record(&self, kind: &str, petals: u64, stamp_petals: u64, detail: impl Into<String>, tx: impl Into<String>) {
        if let Some(journal) = self.journal() {
            let _ =
                journal.append(&kaspa_wallet_core::storage::local::journal::JournalEntry::now(kind, petals, stamp_petals, detail, tx));
        }
    }

    /// Whether this wallet's own miner will mine what it submits, and how
    /// long that takes (PLAN P8.3b). The lane is taken only on our own
    /// copy of the network, only while mining here, and only when a block of
    /// our own is expected within the hour: a transaction below the relay
    /// floor is kept out of relay by our node, so nobody else will ever mine
    /// it, and a CPU against a network of ASICs might wait for days.
    /// Whether a marigoldd on this machine's loopback takes our tidying at the
    /// own-lane fee: from build 2.0.195 a node whose RPC listens on loopback
    /// only does so by default. Older ones reject the fee, so they are treated
    /// as any other computer.
    pub fn local_node_carries_own_lane(&self) -> bool {
        if !self.connected_to_local_node() {
            return false;
        }
        let version = self.connected_server_version.lock().unwrap().clone().unwrap_or_default();
        let mut parts = version.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
        let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        (major, minor, patch) >= (2, 0, 195)
    }

    pub async fn own_lane(&self) -> OwnLane {
        #[cfg(feature = "embedded-node")]
        {
            if !self.embedded_node_in_use() && !self.local_node_carries_own_lane() {
                return self.remote_own_lane().await;
            }
            let hashrate = match self.cpu_miner.lock().unwrap().as_ref() {
                Some(miner) => miner.hashrate(),
                None => return OwnLane::NoMiner,
            };
            if hashrate <= 0.0 {
                return OwnLane::NoMiner;
            }
            // The network's difficulty is the expected number of hashes a
            // block takes; at our rate that is a time.
            let Ok(info) = self.wallet.rpc_api().get_block_dag_info().await else { return OwnLane::NoMiner };
            let every = Duration::from_secs_f64((info.difficulty / hashrate).clamp(1.0, 1.0e9));
            if every <= OWN_LANE_MAX_WAIT { OwnLane::Use { every } } else { OwnLane::TooSlow { every } }
        }
        #[cfg(not(feature = "embedded-node"))]
        self.remote_own_lane().await
    }

    /// The own lane through the background miner on this machine: the node
    /// we are on has a miner in it, and that miner is ours.
    async fn remote_own_lane(&self) -> OwnLane {
        if !self.remote_miner_present() {
            return OwnLane::NotOwnCopy;
        }
        let Some(status) = self.detect_remote_miner().await else { return OwnLane::NotOwnCopy };
        if !status.mining || status.hashrate <= 0.0 {
            return OwnLane::NoMiner;
        }
        let Ok(info) = self.wallet.rpc_api().get_block_dag_info().await else { return OwnLane::NoMiner };
        let every = Duration::from_secs_f64((info.difficulty / status.hashrate).clamp(1.0, 1.0e9));
        if every <= OWN_LANE_MAX_WAIT { OwnLane::Use { every } } else { OwnLane::TooSlow { every } }
    }

    pub fn remote_miner_present(&self) -> bool {
        self.remote_miner.load(Ordering::SeqCst)
    }

    /// Why to mine, said once where it matters: before the wallet is armed to
    /// tidy on its own, and after a sweep that paid the network rate. Only
    /// when this wallet is on a node of its own that is not mining; anywhere
    /// else the advice does not apply. (Founder, 2026-09-19.)
    pub async fn mining_invitation(&self) -> Option<String> {
        if !matches!(self.own_lane().await, OwnLane::NoMiner) {
            return None;
        }
        Some(
            "Tidying is priced by size, not value: a hundred coins cost about a tenth of a coin at the network rate. \
             Mining on this node, even a little, makes it nearly free — only your own blocks carry your own tidying, \
             at a hundredth of the network rate. 'mine start 10' uses a tenth of this machine."
                .to_string(),
        )
    }

    /// Connected to a node on this machine's own loopback: a marigoldd the
    /// user runs beside the wallet. Its operator is the user, so mining
    /// through it gives nothing away.
    pub fn connected_to_local_node(&self) -> bool {
        self.wallet().is_connected()
            && self.wallet().try_wrpc_client().and_then(|c| c.url()).is_some_and(|url| crate::modules::connect::is_local_target(&url))
    }

    /// A background miner is there but not mining.
    pub fn remote_miner_idle(&self) -> bool {
        self.remote_miner.load(Ordering::SeqCst) && !self.remote_mining.load(Ordering::SeqCst)
    }

    /// Remember what the node we are on said about its miner; `None` forgets.
    pub fn note_remote_miner(&self, status: Option<&kaspa_rpc_core::RpcMinerStatus>) {
        match status {
            Some(status) if status.available => {
                self.remote_miner.store(true, Ordering::SeqCst);
                self.remote_mining.store(status.mining, Ordering::SeqCst);
            }
            _ => {
                self.remote_miner.store(false, Ordering::SeqCst);
                self.remote_mining.store(false, Ordering::SeqCst);
            }
        }
    }

    /// Ask the node the wallet is on whether it has a miner in it, and
    /// remember the answer. `Some` only when it does.
    pub async fn detect_remote_miner(&self) -> Option<kaspa_rpc_core::RpcMinerStatus> {
        let status = match self.try_rpc_api() {
            Some(rpc) if self.wallet.is_connected() => rpc.get_miner_status().await.ok(),
            _ => None,
        };
        self.note_remote_miner(status.as_ref());
        status.filter(|status| status.available)
    }

    /// How much of the machine to mine with, from the argument or by asking.
    /// `None` means nothing should start; the reason has been printed.
    async fn mining_share(&self, arg: Option<String>) -> Result<Option<u32>> {
        Ok(match arg {
            Some(text) => match text.trim().trim_end_matches('%').parse::<u32>() {
                Ok(value) if (1..=100).contains(&value) => Some(value),
                _ => {
                    tprintln!(self, "'{text}' is not a percentage between 1 and 100.");
                    None
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
                    .ask(false, "How much of this machine may it use? [1-100%, default 50]: ")
                    .await?
                    .trim()
                    .trim_end_matches('%')
                    .to_string();
                if answer.is_empty() {
                    Some(50)
                } else {
                    match answer.parse::<u32>() {
                        Ok(value) if (1..=100).contains(&value) => Some(value),
                        _ => {
                            tprintln!(self, "'{answer}' is not a percentage between 1 and 100 — nothing started.");
                            None
                        }
                    }
                }
            }
        })
    }

    fn print_remote_miner(&self, status: &kaspa_rpc_core::RpcMinerStatus) {
        tprintln!(self, "");
        if !status.mining {
            tprintln!(self, "The background miner on this machine is idle — 'mine start' sets it going.");
            tprintln!(self, "Rewards go to: {}", status.address);
            tprintln!(self, "");
            return;
        }
        tprintln!(
            self,
            "Mining: {} of {} cores ({}% of this machine) — {}",
            status.threads,
            status.cores,
            status.percent,
            style("the miner program running in the background").dim()
        );
        tprintln!(self, "Speed:  {}", crate::miner::format_hashrate(status.hashrate));
        if status.blocks_found == 0 {
            tprintln!(self, "Blocks: none yet");
            tprintln!(self, "{}", style("Finding one is luck. Leaving it running is the whole technique.").dim());
        } else {
            tprintln!(
                self,
                "Blocks: {} found, {} accepted{}",
                status.blocks_found.separated_string(),
                status.blocks_accepted.separated_string(),
                match status.blocks_rejected {
                    0 => String::new(),
                    n => format!(", {} rejected", n.separated_string()),
                }
            );
        }
        tprintln!(self, "Rewards go to: {}", status.address);
        tprintln!(self, "{}", style("That is the address the miner was started with, not necessarily this wallet's.").dim());
        tprintln!(self, "");
    }

    /// 'mine start' when the miner is the background program on this machine.
    pub async fn start_remote_mining(self: &Arc<Self>, arg: Option<String>) -> Result<()> {
        let Some(percent) = self.mining_share(arg).await? else { return Ok(()) };
        match self.wallet.rpc_api().control_miner(true, Some(percent)).await {
            Ok(status) => {
                self.note_remote_miner(Some(&status));
                tprintln!(self, "");
                tprintln!(
                    self,
                    "{}",
                    style(format!(
                        "Mining started in the background program — {percent}% of this machine ({} of {} cores).",
                        status.threads, status.cores
                    ))
                    .green()
                );
                tprintln!(self, "Rewards go to: {}", status.address);
                tprintln!(self, "'mine status' to check, 'mine stop' to stop.");
                tprintln!(self, "");
            }
            Err(err) => {
                tprintln!(
                    self,
                    "{}",
                    style(format!("The background miner did not start: {}", self.describe_error(&err.to_string()))).yellow()
                );
            }
        }
        Ok(())
    }

    pub async fn stop_remote_mining(self: &Arc<Self>) -> Result<()> {
        match self.wallet.rpc_api().control_miner(false, None).await {
            Ok(status) => {
                self.note_remote_miner(Some(&status));
                tprintln!(self, "Mining stopped in the background program. It stays running, idle, until 'mine start'.");
            }
            Err(err) => {
                tprintln!(
                    self,
                    "{}",
                    style(format!("Could not stop the background miner: {}", self.describe_error(&err.to_string()))).yellow()
                );
            }
        }
        Ok(())
    }

    pub async fn remote_mining_status(self: &Arc<Self>) {
        match self.detect_remote_miner().await {
            Some(status) => self.print_remote_miner(&status),
            None => {
                tprintln!(self, "");
                tprintln!(self, "The background miner is not answering any more.");
                tprintln!(self, "");
            }
        }
    }

    /// Remember a submitted transaction while an own-lane operation runs.
    pub fn note_submission(&self, pending: &kaspa_wallet_core::tx::PendingTransaction) {
        let Some(every) = *self.own_lane_capture.lock().unwrap() else { return };
        self.own_lane_spends.lock().unwrap().push(OwnLaneSpend {
            pending: pending.clone(),
            submitted: Instant::now(),
            every,
            mint_notes: Vec::new(),
        });
    }

    /// Start or stop remembering submissions: `Some(every)` while an operation
    /// priced for our own block runs, `None` after.
    pub fn own_lane_capture(&self, every: Option<Duration>) {
        *self.own_lane_capture.lock().unwrap() = every;
    }

    /// A mint's notes take their serials from the transaction that creates
    /// them; keep them with that transaction so a replacement can re-derive.
    pub fn attach_mint_notes(
        &self,
        tx_id: kaspa_consensus_core::tx::TransactionId,
        notes: &[kaspa_wallet_core::storage::NoteKeyEntry],
    ) {
        if let Some(spend) = self.own_lane_spends.lock().unwrap().iter_mut().find(|s| s.pending.id() == tx_id) {
            spend.mint_notes = notes.to_vec();
        }
    }

    /// The safety net under the own lane. A transaction priced for our own
    /// block is held out of relay, so only our block can carry it: if that
    /// block does not come — the estimate was wrong, the miner was stopped,
    /// the network grew — it would sit in our mempool for a day and lapse.
    /// After three expected block intervals (ten minutes at least) it is
    /// re-sent at the network rate as a replacement spending the same coins,
    /// which the mempool accepts because the fee is higher; the old one goes
    /// with it. A mint's notes get their serials from the transaction id, so
    /// they are re-derived from the replacement's.
    pub async fn retry_stale_own_lane_spends(self: &Arc<Self>) {
        let due: Vec<OwnLaneSpend> = {
            let mut spends = self.own_lane_spends.lock().unwrap();
            if spends.is_empty() {
                return;
            }
            let now = Instant::now();
            let (due, keep): (Vec<_>, Vec<_>) =
                spends.drain(..).partition(|s| now.duration_since(s.submitted) > (s.every * 3).max(Duration::from_secs(600)));
            *spends = keep;
            due
        };
        if due.is_empty() {
            return;
        }
        let Some(rpc) = self.try_rpc_api() else {
            self.own_lane_spends.lock().unwrap().extend(due);
            return;
        };
        let ticker = self.ticker();
        for spend in due {
            // Landed, or lapsed on its own: nothing to do.
            if rpc.get_mempool_entry(spend.pending.id(), false, false).await.is_err() {
                continue;
            }
            let waited = humanised_minutes(spend.submitted.elapsed().as_secs() / 60);
            let fee = spend.pending.mass() * kaspa_wallet_core::account::notepool::POOL_FEE_RATE as u64;
            let replacement = match spend.pending.with_fee(fee) {
                Ok(Some(replacement)) => replacement,
                Ok(None) => {
                    tprintln!(self, "{}", style(format!("A tidying transaction has waited {waited} for a block of ours and cannot be re-priced; it lapses within a day and the coins come back to the ledger.")).yellow());
                    continue;
                }
                Err(err) => {
                    tprintln!(self, "{}", style(format!("A tidying transaction could not be re-priced: {err}")).yellow());
                    continue;
                }
            };
            if let Err(err) = replacement.try_sign() {
                tprintln!(self, "{}", style(format!("A tidying transaction could not be re-signed: {err}")).yellow());
                continue;
            }
            if let Err(err) = spend.pending.withdraw().await {
                tprintln!(self, "{}", style(format!("A tidying transaction could not be withdrawn: {err}")).yellow());
                continue;
            }
            match replacement.try_submit_replacement(&rpc).await {
                Ok(id) => {
                    if !spend.mint_notes.is_empty() {
                        let secret = self.auto_secret.lock().unwrap().as_mut().map(|g| g.reveal());
                        match (secret, self.wallet.store().as_note_key_store()) {
                            (Some(secret), Ok(store)) => {
                                for (index, entry) in spend.mint_notes.iter().enumerate() {
                                    let sn = kaspa_consensus_core::notepool::hashing::serial_hash(&id, index as u32);
                                    let fresh = kaspa_wallet_core::storage::NoteKeyEntry::new(sn, entry.sk, entry.d, entry.provenance);
                                    if let Err(err) = store.store(&secret, fresh).await {
                                        tprintln!(self, "{}", style(format!("could not record a re-derived note: {err}")).yellow());
                                    }
                                    let _ = store.remove(&secret, &entry.sn).await;
                                }
                            }
                            _ => tprintln!(
                                self,
                                "{}",
                                style("The replacement mint's notes could not be re-derived: no session secret.").yellow()
                            ),
                        }
                    }
                    self.record("re-sent", 0, fee, format!("waited {waited} for a block of ours"), id.to_string());
                    tprintln!(
                        self,
                        "{}",
                        style(format!(
                            "A tidying transaction waited {waited} for a block of ours; re-sent at the network rate (fee {} {ticker}).",
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(fee)
                        ))
                        .yellow()
                    );
                }
                Err(err) => {
                    tprintln!(self, "{}", style(format!("A tidying transaction waited {waited} for a block of ours and the network would not take a replacement ({err}); it lapses within a day and the coins come back.")).yellow());
                }
            }
        }
    }

    /// The fee rate to pay: as good as nothing when our own miner will mine
    /// it, the network's otherwise.
    pub async fn own_lane_fee_rate(&self) -> Option<f64> {
        match self.own_lane().await {
            OwnLane::Use { .. } => Some(OWN_LANE_FEE_RATE),
            _ => None,
        }
    }

    /// Whether the own lane could apply but does not, because nothing is
    /// mining here: on our own copy of the network, miner off. The moment to
    /// say that mining would make the tidying free.
    pub fn own_lane_wants_a_miner(&self) -> bool {
        #[cfg(feature = "embedded-node")]
        {
            return (self.embedded_node_in_use() && self.cpu_miner.lock().unwrap().is_none()) || self.remote_miner_idle();
        }
        #[cfg(not(feature = "embedded-node"))]
        self.remote_miner_idle()
    }

    pub fn ledger_is_known(&self) -> bool {
        self.ledger_known.load(Ordering::SeqCst)
    }

    /// Ask the node outright what this account's ledger address holds.
    ///
    /// One number over the wire, which is the whole point. The wallet's own
    /// figure comes from its UTXO context, and that context is filled by
    /// fetching every coin on the address in a single unpaginated response —
    /// four million of them does not arrive inside the RPC timeout, so the
    /// context stays empty and the wallet has nothing to report. The node can
    /// answer the simpler question immediately whatever the coin count is.
    ///
    /// This is for saying what is there. It is not a substitute for the
    /// context: spending coins needs the coins, not their total.
    pub async fn ledger_total_from_node(&self) -> Option<u64> {
        if !self.wallet.is_connected() {
            return None;
        }
        let address = self.wallet.account().ok()?.receive_address().ok()?;
        self.wallet.rpc_api().get_balance_by_address(address).await.ok()
    }

    /// What this network calls its money — `MAGLD` on mainnet, `TMAGLD` on
    /// testnet. Worth asking for rather than writing out: the prompt has
    /// always used the real ticker, so every hardcoded "MAGLD" beside it was
    /// naming the same money twice, differently, on every testnet wallet
    /// there is.
    pub fn ticker(&self) -> &'static str {
        self.wallet.network_id().map(|id| kaspa_wallet_core::utils::kaspa_suffix(&NetworkType::from(id))).unwrap_or("{ticker}")
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
        tprintln!(self, "The network sync is running. It will catch up in the background.");
        Ok(())
    }

    /// Start the node, but leave the wallet pointed wherever it is.
    ///
    /// Returns the node's `Rpc` so the caller can decide when — or whether —
    /// to hand the wallet over to it. `Ok(None)` means one was already running.
    /// Report on the disk before a long job, and say whether it should run.
    ///
    /// Returns false only when there is genuinely not enough. A filesystem we
    /// cannot measure reads as fine — refusing to work because the
    /// measurement failed would be the worse of the two errors.
    ///
    /// `what` finishes "…to run <what>", and `remedy` is the line that says
    /// what to do about it, because a refusal with no way forward is just a
    /// wall.
    pub fn disk_allows(self: &Arc<Self>, path: &std::path::Path, need: crate::space::Need, what: &str, remedy: &str) -> bool {
        use crate::space::{Verdict, human};
        match crate::space::check(path, need) {
            Verdict::Fine => true,
            Verdict::Tight { available } => {
                tprintln!(self, "");
                tprintln!(self, "{}", style(format!("{} free — enough to start {what}, not much more.", human(available))).yellow());
                tprintln!(self, "{}", style(remedy).dim());
                tprintln!(self, "");
                true
            }
            Verdict::Short { available, required } => {
                tprintln!(self, "");
                tprintln!(
                    self,
                    "{}",
                    style(format!("Not enough room to run {what}: {} free, about {} needed.", human(available), human(required)))
                        .red()
                );
                tprintln!(self, "{}", style(remedy).dim());
                tprintln!(self, "");
                false
            }
        }
    }

    #[cfg(feature = "embedded-node")]
    pub async fn spawn_embedded_node(self: &Arc<Self>) -> Result<Option<Rpc>> {
        if self.embedded_node.lock().unwrap().is_some() {
            tprintln!(self, "The sync is already running here.");
            return Ok(None);
        }
        let network_id = self.wallet.network_id()?;
        let appdir = crate::embedded::appdir_in(self.wallet.settings().get::<String>(WalletSettings::Folder).as_deref(), network_id)?;

        // Asked before the node is started rather than discovered eight hours
        // into a sync. A node that runs out of disk part way through leaves a
        // half-written database, and the person finds out when it will not
        // open again.
        if !self.disk_allows(
            &appdir,
            crate::space::LOCAL_NODE,
            "your own node",
            "A node keeps its own copy of the chain. 'history clear' frees whatever old\ntransaction history is using, or point the wallet at a bigger disk with\n'settings set folder <path>' before starting one.",
        ) {
            return Ok(None);
        }

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
    pub(crate) async fn node_is_synced(rpc: &Rpc) -> bool {
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
                tprintln!(self, "{}", style(format!("Sync could not start: {err}")).yellow());
                // Same reconnect trap as the success path below: a caller that
                // has just connected reads is_connected() as false, and
                // reconnecting here asked the "run your own node?" question a
                // second time and dropped the socket that was already open.
                if !ensure_connection || self.wallet.is_connected() {
                    tprintln!(self, "The wallet is using a public computer instead.");
                    tprintln!(self, "{}", style("Whoever runs it sees which notes your wallet asks about. 'connect' tries").dim());
                    tprintln!(self, "{}", style("the sync here again once the problem above is dealt with.").dim());
                } else {
                    tprintln!(self, "Using a public computer instead, so the wallet works meanwhile.");
                    tprintln!(self, "{}", style("Whoever runs it sees which notes your wallet asks about. 'connect' tries").dim());
                    tprintln!(self, "{}", style("the sync here again once the problem above is dealt with.").dim());
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
            tprintln!(self, "{}", style("In sync with the network. Nobody else sees your notes.").green());
            tprintln!(self, "");
            tprintln!(self, "You can mine with spare CPU — 'mine start'.");
            tprintln!(self, "");
            return Ok(());
        }

        if ensure_connection && !self.wallet.is_connected() {
            if let Err(err) = self.exec_within("connect public").await {
                // No public node either: bind to our own anyway. An incomplete
                // view beats none, and 'node status' explains what it is.
                tprintln!(self, "Could not reach a public computer ({err}) — using your own copy while it catches up.");
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
        tprintln!(self, "{}", style("Network sync started!").green());
        tprintln!(self, "");
        tprintln!(self, "A first sync takes anywhere from half an hour to a few hours. Leaving the");
        tprintln!(self, "wallet before it finishes discards it — after that, restarts are free.");
        tprintln!(self, "");
        tprintln!(self, "Type 'connect status' for progress info.");
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
    pub(crate) fn start_node_handover_task(self: &Arc<Self>, rpc: Rpc) {
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
                        tprintln!(
                            this,
                            "{}",
                            style("Your sync has caught up. The wallet is now on its own copy of the network —").green()
                        );
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
                        tprintln!(this, "Your sync is ready, but the wallet could not switch to it: {err}");
                        tprintln!(this, "'connect' moves it across by hand.");
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
        if self.remote_miner_present() {
            return self.start_remote_mining(arg).await;
        }
        if self.cpu_miner.lock().unwrap().is_some() {
            tprintln!(self, "Already mining — 'mine status' for how it is going.");
            return Ok(());
        }
        // Mining against somebody else's node would hand them the address your
        // rewards are paid to, which is the one thing this wallet works to keep
        // off other people's machines. A node on this machine's own loopback is
        // not somebody else's: the founder ran a plain marigoldd beside the
        // wallet and was refused (2026-09-19).
        if !self.embedded_node_in_use() && !self.connected_to_local_node() {
            tprintln!(self, "");
            if self.embedded_node_pending() {
                tprintln!(self, "The sync is still catching up. Mining starts once it is ready —");
                tprintln!(self, "'connect status' shows how far along it is.");
            } else if self.wallet().is_connected() && !self.remote_miner_present() && !self.connected_to_local_node() {
                // Connected, but not to a node of ours and not to the background
                // miner: another wallet on this machine holds the network, and
                // 'connect' just finds it again. Saying "type connect" here sent
                // the founder round in a circle (2026-09-19).
                tprintln!(self, "Another Marigold program on this machine holds the network, and it mines to its own wallet.");
                tprintln!(self, "To mine to this one, either stop the node there and 'connect' here, or run the miner");
                tprintln!(self, "as a service for both:  marigold-cli mine-to <address from 'address'> 10   then 'connect'.");
            } else {
                tprintln!(self, "Mining needs the network synced on this machine. Type 'connect' to start that.");
                tprintln!(self, "{}", style("Asking a public computer for work would tell its operator which address").dim());
                tprintln!(self, "{}", style("your coins are paid to, which is the one thing worth not sharing.").dim());
            }
            tprintln!(self, "");
            return Ok(());
        }
        // Mining writes no files itself, but every block it finds pays into
        // this wallet and grows the node's database. Starting a miner on a
        // disk that cannot hold the growth is how a node ends up corrupt at
        // three in the morning.
        let network_id = self.wallet.network_id()?;
        if let Ok(appdir) =
            crate::embedded::appdir_in(self.wallet.settings().get::<String>(WalletSettings::Folder).as_deref(), network_id)
        {
            if !self.disk_allows(
                &appdir,
                crate::space::MINING,
                "mining",
                "Mining pays into this wallet block after block, and the node's copy of the\nchain grows with it. 'history clear' frees old transaction history.",
            ) {
                return Ok(());
            }
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
        let Some(percent) = self.mining_share(arg).await? else { return Ok(()) };

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
        if !self.wallet.utxo_processor().is_synced() {
            tprintln!(
                self,
                "{}",
                style("The sync is still catching up, so there is no work yet; mining begins on its own when there is.").yellow()
            );
        }
        tprintln!(self, "");

        crate::miner::spawn_session(self.wallet.rpc_api(), address, miner, solutions, self.shutdown.clone());
        Ok(())
    }

    #[cfg(feature = "embedded-node")]
    pub async fn stop_mining(self: &Arc<Self>) -> Result<()> {
        if self.remote_miner_present() && self.cpu_miner.lock().unwrap().is_none() {
            return self.stop_remote_mining().await;
        }
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

    /// What this speed buys against the whole network: how long, on average,
    /// between blocks of ours. The question every new miner asks after ten
    /// minutes of nothing (tester, 2026-09-20).
    #[cfg(feature = "embedded-node")]
    async fn expected_block_cadence(&self, own_hashrate: f64) -> Option<String> {
        if own_hashrate <= 0.0 || !self.wallet.is_connected() {
            return None;
        }
        let network_id = self.wallet.network_id().ok()?;
        let network = self.wallet.rpc_api().estimate_network_hashes_per_second(1000, None).await.ok()?;
        if network == 0 {
            return None;
        }
        let bps = kaspa_consensus_core::config::params::Params::from(network_id).bps().max(1) as f64;
        let seconds = network as f64 / own_hashrate / bps;
        Some(format!(
            "At this speed a block comes about every {} on average; the network as a whole is doing {}.",
            humanised_wait(seconds),
            crate::miner::format_hashrate(network as f64)
        ))
    }

    #[cfg(feature = "embedded-node")]
    pub async fn mining_status(self: &Arc<Self>) {
        if self.remote_miner_present() && self.cpu_miner.lock().unwrap().is_none() {
            return self.remote_mining_status().await;
        }
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
                if !self.wallet.utxo_processor().is_synced() {
                    tprintln!(
                        self,
                        "{}",
                        style("No work yet: the sync is still catching up. Mining begins on its own when it has.").yellow()
                    );
                } else if let Some(line) = self.expected_block_cadence(miner.hashrate()).await {
                    tprintln!(self, "{}", style(line).dim());
                }
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
                if self.embedded_node_in_use() || self.connected_to_local_node() {
                    tprintln!(self, "'mine start' begins, using whatever CPU nothing else wants.");
                } else {
                    tprintln!(self, "Mining needs the network synced on this machine — 'connect'.");
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
                    tprintln!(self, "{}", style("Note: the sync has not finished its first run, and that progress").yellow());
                    tprintln!(self, "{}", style("is discarded — the next start begins again from scratch.").yellow());
                    tprintln!(self, "");
                }
                tprintln!(self, "Stopping the network sync...");
                node.stop().await?;
                tprintln!(self, "Stopped.");
            }
            None => tprintln!(self, "No sync of your own is running."),
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
        *self.auto_secret.lock().unwrap() = Some(Guarded::from_secret(secret));
        *self.auto_payment_secret.lock().unwrap() = payment_secret.map(Guarded::from_secret);
        self.auto_threshold_petals.store(threshold_petals, Ordering::SeqCst);
    }

    /// Check a password against the account's own key data before arming
    /// anything with it. Housekeeping signs in the background, an hour after
    /// the prompt has scrolled away — a typo accepted here becomes an
    /// automation that quietly does nothing, which is the worst way to find out.
    pub async fn verify_wallet_secret(&self, secret: &Secret, payment_secret: Option<&Secret>) -> Result<()> {
        let account = match self.wallet().account() {
            Ok(account) => account,
            Err(_) if !self.has_ledger_account().await => {
                // Notes only: the vault key is the only key there is, and it is
                // wrapped under the wallet password.
                let store = self.wallet().store().as_note_key_store()?;
                return store.verify_secret(secret).await.map_err(|_| Error::custom("That password does not open this wallet"));
            }
            Err(err) => return Err(err.into()),
        };
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
            *guard = Some(Guarded::from_secret(secret));
            *self.auto_payment_secret.lock().unwrap() = payment_secret.map(Guarded::from_secret);
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
        if !self.has_ledger_account().await {
            return None;
        }
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
        if let Ok(store) = self.wallet.store().as_note_key_store()
            && let Ok(mut stream) = store.iter().await
        {
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
        let (mut notes, ledger, pieces) = self.total_holdings().await;
        let ticker = self.ticker();

        // Checked before it is printed, not after. A note that stops being
        // counted stops being counted in this figure too — announcing the
        // total first and then saying part of it does not exist left a wrong
        // number on screen and the corrected one in the prompt beneath it.
        //
        // Nothing is said unless a synced node has failed to find the same
        // note three separate times. A note the pool has not got is usually a
        // mint that has not landed, and saying so on every open taught people
        // to ignore a line that one day will matter.
        // The opposite correction first: notes this wallet wrote off as spent
        // that the pool still holds under its keys, because the transaction
        // that was to consume them never landed. Back into the balance, and
        // into the history, so the money is neither lost nor a mystery.
        if self.wallet.is_connected()
            && let Ok(revived) = kaspa_wallet_core::account::notepool::revive_unspent_notes(&self.wallet).await
            && !revived.is_empty()
        {
            let value: u64 = revived.iter().map(|i| kaspa_consensus_core::notepool::DENOMINATION_PETALS[i.d as usize]).sum();
            notes += value;
            for info in &revived {
                self.record(
                    "returned",
                    kaspa_consensus_core::notepool::DENOMINATION_PETALS[info.d as usize],
                    0,
                    "its transaction never landed",
                    "",
                );
            }
            tprintln!(
                self,
                "{}",
                crate::ui::dim(format!(
                    "{} note(s) worth {} {ticker} came back: the transaction that was to spend them never landed, and the pool still holds them under your keys.",
                    revived.len(),
                    kaspa_wallet_core::utils::sompi_to_kaspa_string(value)
                ))
            );
        }
        let mut vanished: Option<(usize, u64)> = None;
        if notes > 0 && self.wallet.is_connected() {
            {
                if let Ok(Some(result)) = kaspa_wallet_core::account::notepool::reconcile_held_notes(&self.wallet).await
                    && !result.moved_to_unknown.is_empty()
                {
                    let value: u64 = result
                        .moved_to_unknown
                        .iter()
                        .map(|i| kaspa_consensus_core::notepool::DENOMINATION_PETALS[i.d as usize])
                        .sum();
                    notes = notes.saturating_sub(value);
                    vanished = Some((result.moved_to_unknown.len(), value));
                }
            }
        }

        // Keep the prompt's figure in step, so it is right from the moment a
        // wallet opens rather than after the first minute tick.
        self.prompt_total_petals.store(notes, Ordering::SeqCst);
        self.prompt_total_valid.store(true, Ordering::SeqCst);
        tprintln!(self, "");
        tprintln!(self, "notes:  {} {ticker}", kaspa_wallet_core::utils::sompi_to_kaspa_string(notes));
        // Said once, on opening, so that reaching for a phone at the moment
        // of a payment is expected rather than alarming.
        if self.otp().is_some() {
            tprintln!(self, "{}", style("(this wallet asks for a code from your phone before it spends)").dim());
        }
        if !self.ledger_is_known() {
            match self.ledger_total_from_node().await {
                Some(total) => tprintln!(
                    self,
                    "{}",
                    crate::ui::dim(format!(
                        "ledger: {} {ticker}  (as the node sees it — still reading the coins)",
                        crate::ui::ledger_amount(total)
                    ))
                ),
                None => tprintln!(self, "{}", crate::ui::dim("ledger: not read yet — the node has not answered")),
            }
        } else if ledger > 0 {
            tprintln!(
                self,
                "ledger: {} {ticker}  ({} piece{})",
                crate::ui::ledger_amount(ledger),
                pieces.separated_string(),
                if pieces == 1 { "" } else { "s" }
            );
        }
        if let Some((count, value)) = vanished {
            tprintln!(self, "");
            tprintln!(
                self,
                "{}",
                crate::ui::warn(format!(
                    "{count} note(s) worth {} {ticker} are not on chain and are not counted above.",
                    kaspa_wallet_core::utils::sompi_to_kaspa_string(value)
                ))
            );
            tprintln!(self, "{}", crate::ui::dim("Most often a payment that never landed, in which case the money never"));
            tprintln!(self, "{}", crate::ui::dim("left your ledger balance. 'note unknown' lists them."));
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
        let ticker = self.ticker();
        // Never decide anything from a ledger figure the wallet has not
        // actually read. An unfinished reload reads as zero, and "nothing to
        // mint, the ledger holds 0" was printed over a ledger holding
        // 812,524 TMAGLD — the decision was as wrong as the sentence.
        if !self.ledger_is_known() {
            // Try to make it known rather than waiting for another sync edge
            // that may never come: the edge fires once, and if its reload
            // timed out the wallet would otherwise sit not knowing forever.
            // A big wallet's UTXO set is a single large RPC response and
            // whether it arrives inside the timeout is luck on the day, so
            // the thing to do is keep asking.
            let guard = self.wallet.guard();
            let guard = guard.lock().await;
            let recovered = self.wallet.reload(true, &guard).await.is_ok();
            drop(guard);
            self.ledger_known.store(recovered, Ordering::SeqCst);
            if !recovered {
                if loud {
                    tprintln!(
                        self,
                        "{}",
                        crate::ui::dim("(still reading the ledger from the node — nothing is being minted meanwhile)")
                    );
                }
                self.own_lane_capture(None);
                self.auto_busy.store(false, Ordering::SeqCst);
                return;
            }
        }
        // Offered notes (PLAN P8.0g): once a lock has lapsed, what the
        // receiver never took comes back under its refund key; what they took
        // in time is marked paid. Needs the secret the automation holds.
        let secret_for_offers = self.auto_secret.lock().unwrap().as_mut().map(|g| g.reveal());
        if let Some(secret) = secret_for_offers {
            match kaspa_wallet_core::account::notepool::reclaim_lapsed(&self.wallet, secret).await {
                Ok(report) => {
                    for (_, d) in &report.taken_back {
                        self.record(
                            "returned",
                            kaspa_consensus_core::notepool::DENOMINATION_PETALS[*d as usize],
                            0,
                            "offer lapsed, taken back",
                            "",
                        );
                    }
                    for (_, d) in &report.taken_by_receiver {
                        self.record(
                            "paid",
                            kaspa_consensus_core::notepool::DENOMINATION_PETALS[*d as usize],
                            0,
                            "locked code, taken in time",
                            "",
                        );
                    }
                    if !report.taken_back.is_empty() {
                        let back: u64 = report
                            .taken_back
                            .iter()
                            .map(|(_, d)| kaspa_consensus_core::notepool::DENOMINATION_PETALS[*d as usize])
                            .sum();
                        tprintln!(
                            self,
                            "{}",
                            crate::ui::dim(format!(
                                "An offer lapsed untaken: {} {ticker} came back to you.",
                                kaspa_wallet_core::utils::sompi_to_kaspa_string(back)
                            ))
                        );
                    }
                    if !report.taken_by_receiver.is_empty() {
                        let paid: u64 = report
                            .taken_by_receiver
                            .iter()
                            .map(|(_, d)| kaspa_consensus_core::notepool::DENOMINATION_PETALS[*d as usize])
                            .sum();
                        tprintln!(
                            self,
                            "{}",
                            crate::ui::dim(format!(
                                "A locked payment of {} {ticker} was taken in time.",
                                kaspa_wallet_core::utils::sompi_to_kaspa_string(paid)
                            ))
                        );
                    }
                }
                Err(err) => {
                    if loud {
                        tprintln!(self, "{}", crate::ui::dim(format!("(offers not checked: {err})")));
                    }
                }
            }
        }
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
        // None on a wallet that keeps notes only: steps 0–3 are the ledger's
        // and are skipped; the notes are still tidied.
        let account = self.wait_for_account().await;
        let Some(secret) = self.auto_secret.lock().unwrap().as_mut().map(|g| g.reveal()) else {
            self.own_lane_capture(None);
            self.auto_busy.store(false, Ordering::SeqCst);
            return;
        };
        let payment_secret = self.auto_payment_secret.lock().unwrap().as_mut().map(|g| g.reveal());

        if let Some(account) = account.as_ref() {
            // --- 0. is this more than an automatic process should start on its own? ---
            //
            // Automatic consolidation proceeds at a rate this wallet sets: a pass a
            // minute, and SWEEP_TRANSACTIONS_PER_PASS transactions in a pass. Past
            // a certain backlog that is hours of work on somebody's money, begun
            // without being asked, and hours during which every pass is also
            // minting and adding change of its own.
            //
            // A backlog that size means something has already gone wrong — income
            // outrunning consolidation for a long time. The wallet says so and
            // stops, rather than grinding through it quietly and leaving the person
            // to wonder why their wallet has been busy since Tuesday.
            let pieces = account.utxo_context().mature_utxo_size() as u64;
            if pieces > AUTOMATIC_HOUSEKEEPING_CEILING {
                if loud {
                    let minutes = pieces / coins_per_minute().max(1);
                    let ledger: u64 = account.utxo_context().utxo_entries_snapshot().0.iter().map(|entry| entry.amount()).sum();
                    tprintln!(self, "");
                    tprintln!(
                        self,
                        "{}",
                        crate::ui::warn(format!(
                            "{} ledger coin(s) worth {} {ticker} — too many to tidy up unattended.",
                            pieces.separated_string(),
                            crate::ui::ledger_amount(ledger)
                        ))
                    );
                    tprintln!(
                        self,
                        "{}",
                        crate::ui::dim(format!(
                            "Consolidating them takes roughly {} at the rate this runs in the background,",
                            humanised_minutes(minutes)
                        ))
                    );
                    tprintln!(self, "{}", crate::ui::dim("so it is left to you rather than started without asking."));
                    tprintln!(self, "");
                    tprintln!(
                        self,
                        "{}",
                        crate::ui::dim("  'sweep <amount>'   consolidate that much and stop — as many times as you like")
                    );
                    tprintln!(self, "{}", crate::ui::dim("  'sweep'            consolidate all of it in one run"));
                    tprintln!(self, "");
                    tprintln!(self, "{}", crate::ui::dim("Minting is paused until the ledger is back to a workable size."));
                    if self.own_lane_wants_a_miner() {
                        tprintln!(
                            self,
                            "{}",
                            crate::ui::dim(
                                "Mining here makes all of it free — 'mine start' — your own blocks mine it and the fee comes back to you."
                            )
                        );
                    }
                    tprintln!(self, "");
                }
                self.own_lane_capture(None);
                self.auto_busy.store(false, Ordering::SeqCst);
                return;
            }

            // Whether our own miner will mine what follows (PLAN P8.3b):
            // decided once for the pass, used by the mint and the sweep alike.
            let lane = self.own_lane().await;
            self.own_lane_capture(match lane {
                OwnLane::Use { every } => Some(every),
                _ => None,
            });
            let lane_fee_rate = match lane {
                OwnLane::Use { .. } => Some(OWN_LANE_FEE_RATE),
                _ => None,
            };

            // --- 1. turn the ledger into notes ---
            // Minting comes FIRST because a mint IS a consolidation: it takes many
            // mature coins as inputs and leaves notes plus a single change coin.
            // Sweeping first spent the very coins the mint needed and pushed them
            // into "pending", so the mint that followed saw almost nothing —
            // 1.32 MAGLD of a 143,000 MAGLD ledger (founder report, 2026-09-06).
            // The ordering above is what protects the mint, not a guard. This used
            // to also refuse to mint while anything at all was unconfirmed, which
            // was the same blunt instrument that starved the sweep below: any one
            // outgoing transaction anywhere stopped the whole pipeline. Now that
            // consolidation runs every pass, its transactions would have been in
            // flight most of the time and the starvation would simply have moved
            // from the sweep to the mint.
            //
            // What the mint actually needs is enough MATURE balance, which is
            // checked directly below — and `mature` already excludes every coin a
            // pending transaction has claimed.
            let threshold = self.auto_mint_threshold();
            if threshold > 0 {
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
                            "Nothing to mint yet: {} {ticker} is confirming (threshold {}). It will be minted as it lands.",
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(waiting),
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(threshold)
                        );
                    } else if mature > 0 {
                        // Only when there is something there but not enough. An
                        // empty ledger under a threshold is not news — it is the
                        // normal state of every wallet that keeps its money as
                        // notes, which is all of them, reported once per open.
                        tprintln!(
                            self,
                            "Nothing to mint: ledger holds {} {ticker}, threshold is {}.",
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(mature),
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(threshold)
                        );
                    }
                }
                if loud && mature > 0 {
                    match lane {
                        OwnLane::Use { every } => tprintln!(
                            self,
                            "{}",
                            crate::ui::dim(format!(
                                "Your own miner will mine the tidying ({}), so the fee goes to your miner's wallet.",
                                block_cadence(every)
                            ))
                        ),
                        OwnLane::TooSlow { every } => tprintln!(
                            self,
                            "{}",
                            crate::ui::dim(format!(
                                "Your miner finds {} — too rare to wait for, so the network fee is paid.",
                                block_cadence(every)
                            ))
                        ),
                        OwnLane::NoMiner => {
                            tprintln!(
                                self,
                                "{}",
                                crate::ui::dim(
                                    "Turning the ledger into notes costs the network fee. Mining here makes it free — 'mine start' —"
                                )
                            );
                            tprintln!(
                                self,
                                "{}",
                                crate::ui::dim("because your own blocks mine your own tidying and the fee comes back to you.")
                            );
                        }
                        OwnLane::NotOwnCopy => {}
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
                            lane_fee_rate,
                            &abortable,
                            None,
                        )
                        .await?;
                        if let Some((_, result)) = &result
                            && let Some(id) = result.transaction_ids.last()
                        {
                            self.attach_mint_notes(*id, &result.notes);
                        }
                        Ok::<_, kaspa_wallet_core::error::Error>(
                            result.map(|(amount, result)| (amount, result.notes.len(), result.fees, result.transaction_ids.len())),
                        )
                    }
                    .await;
                    match minted {
                        Ok(Some((amount, notes, fees, transactions))) => {
                            self.record("minted", amount, fees, format!("{notes} notes, on its own"), "");
                            if loud {
                                tprintln!(
                                    self,
                                    "Minted {} {ticker} into {notes} note(s) (fees {} {ticker}).",
                                    kaspa_wallet_core::utils::sompi_to_kaspa_string(amount),
                                    kaspa_wallet_core::utils::sompi_to_kaspa_string(fees)
                                );
                                // An own-lane mint is not done when it is submitted:
                                // it is a queue of batches only this machine's miner
                                // will include, at a handful per block. A mining
                                // wallet's 212,281 pieces made two thousand of them,
                                // hours of mining — during which the ledger reads as
                                // spent and the notes as held, and a node restart
                                // would lose the queue (founder, 2026-09-18). Say how
                                // long, and to stay.
                                if let OwnLane::Use { every } = lane
                                    && transactions > 1
                                {
                                    // Roughly five batches fit a block.
                                    let minutes = (transactions as u64 * every.as_secs()).div_ceil(5 * 60).max(1);
                                    tprintln!(
                                        self,
                                        "{}",
                                        crate::ui::dim(format!(
                                            "Submitted as {transactions} transactions for your miner to land — about {}. Keep this wallet open until then; the change and the notes settle as they confirm.",
                                            humanised_minutes(minutes)
                                        ))
                                    );
                                }
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
            // Deliberately NOT gated on unconfirmed spends, unlike the mint above.
            //
            // It used to be, and that guard is what let this wallet reach
            // 4,439,373 coins. The mint runs first and registers its transactions
            // as outgoing, so by the time this line was reached there were always
            // unconfirmed spends — the ones the mint had just made. On a wallet
            // with income arriving continuously the mint always has something to
            // do, so the sweep was skipped on every single pass and the coin count
            // only ever grew. Step one guaranteed step two would not run.
            //
            // The guard was never needed for safety either: registering an
            // outgoing transaction removes its inputs from `mature` (see
            // `UtxoContext::register_outgoing_transaction`), so a sweep starting
            // here can only see coins the mint did not take.
            let sweep_threshold = self.auto_sweep_threshold();
            // Re-read: the mint above has just spent some of them.
            let pieces = account.utxo_context().mature_utxo_size() as u64;
            if sweep_threshold > 0 && pieces > sweep_threshold {
                if loud {
                    tprintln!(self, "Consolidating {} ledger pieces — this can take a while...", pieces.separated_string());
                }
                let notifier: Option<kaspa_wallet_core::account::GenerationNotifier> = None;
                // Bounded, because housekeeping is one task: minting and
                // consolidation run in sequence, so an unbounded sweep on a large
                // wallet stops the wallet minting for as long as it takes. A pass
                // this size clears roughly a million coins an hour while leaving
                // the minute's minting to happen.
                const SWEEP_TRANSACTIONS_PER_PASS: usize = SWEEP_TRANSACTIONS_PER_PASS_CONST;
                match account
                    .clone()
                    .sweep(
                        secret.clone(),
                        payment_secret.clone(),
                        lane_fee_rate,
                        &abortable,
                        notifier,
                        None,
                        Some(SWEEP_TRANSACTIONS_PER_PASS),
                    )
                    .await
                {
                    Ok(summary) => {
                        if loud {
                            tprintln!(
                                self,
                                "Consolidated (fees {} {ticker}).",
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
            if let Ok(store) = self.wallet.store().as_prv_key_data_store()
                && let Ok(mut stream) = store.iter().await
            {
                let mut orphans = Vec::new();
                let guard = self.wallet.guard();
                let guard = guard.lock().await;
                while let Ok(Some(info)) = stream.try_next().await {
                    if let Ok(mut accounts) = self.wallet.accounts(Some(info.id), &guard).await
                        && accounts.try_next().await.ok().flatten().is_none()
                    {
                        orphans.push(info.id);
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
        // Deliberately NOT gated on whether anything is unconfirmed: that measures the
        // ledger's outgoing balance, and a note merge is a pure pool operation
        // that spends notes and pays its fee from a spare one — it touches no
        // ledger coin at all. Since minting runs first and always leaves an
        // outgoing balance behind, the gate meant this step simply never ran,
        // which is why ten 0.1s, twenty-two 1s and twelve 10,000s all sat
        // unmerged (founder report, 2026-09-06). Notes already in flight are
        // marked Superseded at submit time, so plan_merges cannot pick them
        // twice — the double-spend the gate was guarding against is handled
        // where it actually applies.
        match kaspa_wallet_core::account::notepool::merge_held_notes(&self.wallet, secret, 250).await {
            Ok((merged, failure)) => {
                if merged > 0 && loud {
                    tprintln!(self, "Consolidated {merged} group(s) of ten notes into larger ones.");
                }
                if let Some(reason) = failure
                    && loud
                {
                    tprintln!(self, "Note consolidation stopped: {reason}");
                }
            }
            Err(err) => {
                if loud {
                    tprintln!(self, "Note consolidation stopped: {err}");
                }
            }
        }

        self.own_lane_capture(None);
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

                let (notes, _ledger, _) = this.total_holdings().await;
                this.prompt_total_petals.store(notes, Ordering::SeqCst);
                this.prompt_total_valid.store(true, Ordering::SeqCst);
                this.retry_stale_own_lane_spends().await;

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

    /// Notes move through the node the wallet is on, and a node still
    /// catching up holds only part of the pool: a payment sent through one is
    /// refused, or — worse — taken in and lost when the sync replaces what
    /// it was validated against. A tester paid twice through their own
    /// syncing node and got one of each (2026-09-20). So paying, receiving,
    /// minting and transferring wait until the node is caught up.
    pub fn node_ready_for_notes(&self) -> Result<()> {
        if self.wallet.is_connected() && !self.wallet.utxo_processor().is_synced() {
            return Err(Error::custom(
                "The copy of the network this wallet is on is still catching up (the SYNC in the prompt). A copy that is \
                 behind holds only part of the pool, so a payment sent through it is refused — or taken in and never \
                 reaches anyone. Paying, receiving and minting wait until it has caught up: 'connect status' shows \
                 progress, 'connect public' uses a public computer meanwhile.",
            ));
        }
        Ok(())
    }

    /// Mark a deliberate change of connection for as long as the guard lives.
    pub fn switching(&self) -> SwitchingGuard {
        self.switching.store(true, Ordering::SeqCst);
        SwitchingGuard(self.switching.clone())
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
            self.wallet
                .store()
                .set_storage_folder(&folder)
                .unwrap_or_else(|err| log_error!("Unable to apply wallet folder setting: {err}"));
        }
        self.wallet.start().await?;
        Ok(())
    }

    pub async fn run(self: &Arc<Self>) -> Result<()> {
        self.term().run().await?;
        Ok(())
    }

    pub async fn stop(self: &Arc<Self>) -> Result<()> {
        // The terminal has already gone by the time this runs, so these lines
        // go to stdout directly. Said in plain words and only when a step is
        // actually slow: a wallet that leaves in under a second says nothing.
        // Each step is timed and named, so a long wait explains itself and
        // tells us which part it was.
        let slow = |what: &str, started: std::time::Instant| {
            let took = started.elapsed();
            if took.as_secs() >= 2 {
                std::println!("  ({what}: {} s)", took.as_secs());
            }
        };
        let overall = std::time::Instant::now();
        let announced = Arc::new(AtomicBool::new(false));
        {
            // Say something if the first second passes without being done.
            let announced = announced.clone();
            workflow_core::task::spawn(async move {
                workflow_core::task::sleep(Duration::from_millis(1200)).await;
                if !announced.swap(true, Ordering::SeqCst) {
                    std::println!("Tidying up before leaving — a moment, please.");
                }
            });
        }

        let step = std::time::Instant::now();
        self.wallet.stop().await?;
        slow("the wallet's own bookkeeping", step);

        let step = std::time::Instant::now();
        self.handlers.stop(self).await?;
        slow("the commands' housekeeping", step);

        // stop notification pipe task
        let step = std::time::Instant::now();
        self.stop_notification_pipe_task().await?;
        slow("the notification relay", step);

        announced.store(true, Ordering::SeqCst);
        if overall.elapsed().as_secs() >= 2 {
            std::println!("Done.");
        }
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
                                    // Announced when the server status arrives, below — for a
                                    // first connection and for one the socket made again on
                                    // its own after the node went away.
                                },
                                #[allow(unused_variables)]
                                Events::Disconnect{ url, network_id } => {
                                    this.ledger_known.store(false, Ordering::SeqCst);
                                    if !this.switching.load(Ordering::SeqCst) {
                                        tprintln!(this, "Disconnected from {} — trying again until it is back.", url.unwrap_or_else(|| "the node".to_string()));
                                        this.term().refresh_prompt();
                                    }
                                },
                                Events::UtxoIndexNotEnabled { .. } => {
                                    tprintln!(this, "Error: Marigold node UTXO index is not enabled...")
                                },
                                Events::SyncState { sync_state } => {
                                    // Only on the edge into synced. This used to fire on every
                                    // synced sync-state event, and a node that keeps reporting
                                    // itself synced — which is what a healthy one does — meant a
                                    // full wallet reload over and over, each one re-reading the
                                    // UTXO set over RPC. On a large wallet they overlap and time
                                    // out, which is where "RPC request timeout" came from.
                                    let was_synced =
                                        this.sync_state.lock().unwrap().as_ref().map(|state| state.is_synced()).unwrap_or(false);
                                    let became_synced = sync_state.is_synced() && !was_synced;

                                    this.sync_state.lock().unwrap().replace(sync_state);

                                    if became_synced && this.wallet().is_open() {
                                        let guard = this.wallet().guard();
                                        let guard = guard.lock().await;
                                        // reactivate: true — reload(false) stops every account and
                                        // resets the UTXO processor, leaving reactivation to the
                                        // caller... which this caller never did (inherited upstream).
                                        // Anyone who opened their wallet BEFORE connecting got a
                                        // permanent N/A balance out of it.
                                        //
                                        // Tried twice: the first attempt lands exactly when the
                                        // node has just finished syncing and is at its busiest, and
                                        // a timeout there is a slow node rather than a broken one.
                                        let mut outcome = this.wallet().reload(true, &guard).await;
                                        if outcome.is_err() {
                                            workflow_core::task::sleep(Duration::from_secs(3)).await;
                                            outcome = this.wallet().reload(true, &guard).await;
                                        }
                                        // A failed reload leaves the UTXO context empty, and an
                                        // empty context is indistinguishable from an empty ledger
                                        // unless something remembers which it is.
                                        this.ledger_known.store(outcome.is_ok(), Ordering::SeqCst);
                                        if outcome.is_err() {
                                            // Not the raw error. "RPC Server (remote error) ->
                                            // RPC request timeout" tells a person nothing except
                                            // that something broke, and it did not break — the
                                            // wallet simply has not read the ledger yet.
                                            tprintln!(
                                                this,
                                                "{}",
                                                crate::ui::dim(
                                                    "(the node did not answer in time — the ledger has not been read yet; 'balance' again in a moment)"
                                                )
                                            );
                                        }
                                    }

                                    this.term().refresh_prompt();
                                }
                                Events::ServerStatus {
                                    is_synced,
                                    server_version,
                                    url,
                                    ..
                                } => {

                                    // No URL means the node is inside this process — there is no
                                    // address to print, and "at N/A" reads like a fault.
                                    this.connected_server_version.lock().unwrap().replace(server_version.to_string());
                                    match &url {
                                        Some(url) => tprintln!(this, "Connected to {url}, Marigold version {server_version}"),
                                        None => tprintln!(this, "Using your own copy of the network, Marigold version {server_version}"),
                                    }

                                    let is_open = this.wallet.is_open();

                                    if !is_synced {
                                        // Not a fault, so not red: the node is catching up and the
                                        // ledger side waits for it. The old line, in red, read as an
                                        // error to the founder (2026-09-19).
                                        let whose = if url.as_deref().is_some_and(crate::modules::connect::is_local_target) {
                                            "Your node on this machine"
                                        } else if url.is_none() {
                                            "Your own copy of the network"
                                        } else {
                                            "That computer"
                                        };
                                        tprintln!(
                                            this,
                                            "{}",
                                            style(format!(
                                                "{whose} is still catching up with the network. Paying, receiving and minting wait until it has caught up — 'connect status' shows progress, 'connect public' uses a public computer meanwhile."
                                            ))
                                            .yellow()
                                        );
                                        let _ = is_open;
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
                                    this.stop_telegram_bot();
                                    this.term().refresh_prompt();
                                },
                                Events::PrvKeyDataCreate { .. } => { },
                                Events::AccountDeactivation { .. } => { },
                                Events::AccountActivation { .. } => {
                                    // The ledger line, once there is a node to
                                    // read it from. Offline it only repeated
                                    // "not connected" under the line that had
                                    // just said so.
                                    if this.wallet.is_connected() {
                                        this.list().await.unwrap_or_else(|err|terrorln!(this, "{err}"));
                                    }

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
                                    // Deliberately does NOT mark the ledger as read.
                                    //
                                    // It used to, and that was wrong in the way that mattered: a
                                    // balance event fires whenever the figure changes, including
                                    // when it is recomputed from a context a failed reload left
                                    // empty. One arriving coin was enough to re-label an unread
                                    // ledger as read, and the wallet went back to presenting 0.00
                                    // as fact — the exact thing the flag exists to prevent.
                                    //
                                    // Only a reload that actually completed proves the coins were
                                    // read, so only that sets it.

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
    /// The password for tidying — mint, sweep — which moves money between
    /// this wallet's own pockets and nowhere else. If the session already
    /// holds it (auto-mint or auto-sweep armed, which sign with it in the
    /// background anyway), it is not asked for again, and no second factor
    /// either: a spend is gated, a sweep is not (founder, 2026-09-20).
    pub(crate) async fn ask_wallet_secret_for_tidying(&self, account: Option<&Arc<dyn Account>>) -> Result<(Secret, Option<Secret>)> {
        let held = self.auto_secret.lock().unwrap().as_mut().map(|g| g.reveal());
        if let Some(secret) = held {
            let payment = self.auto_payment_secret.lock().unwrap().as_mut().map(|g| g.reveal());
            return Ok((secret, payment));
        }
        self.ask_wallet_secret(account).await
    }

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
                        style(format!(
                            "That code is not right — {remaining_tries} more {}.",
                            if remaining_tries == 1 { "try" } else { "tries" }
                        ))
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
            // Nothing to select from on a wallet that keeps notes only —
            // say so, rather than "no accounts", which reads as a fault.
            if !self.has_ledger_account().await {
                return self.ledger_account().await;
            }
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

    /// Recompute the figure the prompt shows and redraw it. Called after a
    /// note changes hands: the prompt kept saying 11.05 after a receive of
    /// 10.00 until the next 'balance' (founder's tester, 2026-09-19).
    pub async fn refresh_prompt_total(self: &Arc<Self>) {
        let (notes, _ledger, _) = self.total_holdings().await;
        self.prompt_total_petals.store(notes, Ordering::SeqCst);
        self.prompt_total_valid.store(true, Ordering::SeqCst);
        self.term().refresh_prompt();
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
                // The ledger, by that name. The account's title and id said
                // nothing to anyone — one wallet has one ledger — and the
                // address is 'address''s to print, when it is wanted, not
                // something to put on screen at every open (founder,
                // 2026-09-15). An unknown balance is not "N/A": the coins
                // have not been read yet, and that is what gets said.
                if account.balance().is_none() {
                    let status = if self.wallet.is_connected() { "reading..." } else { "not connected — 'connect' to read it" };
                    tprintln!(self, "• ledger: {}", style(status).dim());
                } else {
                    let pieces = account.utxo_context().mature_utxo_size();
                    let pending = account.utxo_context().pending_utxo_size();
                    let info = match (pieces, pending) {
                        (0, 0) => String::new(),
                        (_, 0) => format!("{} piece(s)", pieces.separated_string()),
                        (0, _) => format!("{} piece(s) pending", pending.separated_string()),
                        _ => format!("{} piece(s), {} pending", pieces.separated_string(), pending.separated_string()),
                    };
                    tprintln!(self, "• ledger: {}   {}", account.balance_as_strings(None)?, style(info).dim());
                }
                if self.advanced() {
                    tprintln!(self, "  {}", style(account.receive_address()?.to_string()).blue());
                }
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
            self.stop_telegram_bot();

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
                        Some([crate::ui::paint(crate::ui::Ink::Gold, "SYNC"), style("...").black().to_string()].join(" "))
                    } else {
                        Some(
                            [crate::ui::paint(crate::ui::Ink::Gold, "SYNC PROOF"), style(level.separated_string()).dim().to_string()]
                                .join(" "),
                        )
                    }
                }
                SyncState::Headers { headers, progress } => Some(
                    [
                        crate::ui::paint(crate::ui::Ink::Gold, "SYNC IBD HDRS"),
                        style(format!("{} ({}%)", headers.separated_string(), progress)).dim().to_string(),
                    ]
                    .join(" "),
                ),
                SyncState::Blocks { blocks, progress } => Some(
                    [
                        crate::ui::paint(crate::ui::Ink::Gold, "SYNC IBD BLOCKS"),
                        style(format!("{} ({}%)", blocks.separated_string(), progress)).dim().to_string(),
                    ]
                    .join(" "),
                ),
                SyncState::TrustSync { processed, total } => {
                    let progress = processed * 100 / total;
                    Some(
                        [
                            crate::ui::paint(crate::ui::Ink::Gold, "SYNC TRUST"),
                            style(format!("{} ({}%)", processed.separated_string(), progress)).dim().to_string(),
                        ]
                        .join(" "),
                    )
                }
                SyncState::UtxoSync { total, .. } => Some(
                    [crate::ui::paint(crate::ui::Ink::Gold, "SYNC UTXO"), style(total.separated_string()).dim().to_string()].join(" "),
                ),
                SyncState::SmtSync { processed, total } => Some(
                    [
                        crate::ui::paint(crate::ui::Ink::Gold, "SYNC SMT"),
                        style(format!("{} of {}", processed.separated_string(), total.separated_string())).dim().to_string(),
                    ]
                    .join(" "),
                ),
                SyncState::UtxoResync => {
                    Some([crate::ui::paint(crate::ui::Ink::Gold, "SYNC"), style("UTXO").black().to_string()].join(" "))
                }
                SyncState::NotSynced => {
                    Some([crate::ui::paint(crate::ui::Ink::Gold, "SYNC"), style("...").black().to_string()].join(" "))
                }
                SyncState::Synced => None,
            }
        } else {
            Some(crate::ui::paint(crate::ui::Ink::Gold, "SYNC"))
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
                        term.writeln(self.describe_error(&err.to_string()));
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
            if !matches!(text.as_str(), "cancelled" | "Aborted") {
                term.writeln(self.describe_error(&err.to_string()));
            }
        }
        // Ctrl+D at a prompt cancelled it; now that the command has unwound,
        // it means what it means at the main line.
        if term.take_eof() {
            self.handlers.execute(&self, "exit").await.ok();
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
                    "mint", "rotate", "move", "mirror", "redeem", "request", "pay", "import", "export", "pos", "balance", "list",
                    "history", "verify", "vault", "help",
                ]),
                ("wallet", _) => Some(vec![
                    "list",
                    "create",
                    "import",
                    "open",
                    "close",
                    "where",
                    "autoconnect",
                    "forget",
                    "show",
                    "rename",
                    "tidy",
                    "destroy",
                    "hint",
                    "help",
                ]),
                ("account", _) => Some(vec!["create", "import", "name", "recover", "watch", "help"]),
                ("history", _) => Some(vec!["list", "details"]),
                ("settings", _) => Some(vec!["set"]),
                ("track", _) => Some(vec!["balance", "pending", "tx", "utxo", "daa"]),
                ("network", _) => Some(vec!["mainnet", "testnet-10"]),
                ("connect", _) => Some(vec!["status", "details", "logs", "public"]),
                ("node", _) | ("miner", _) => Some(vec!["start", "stop", "restart", "status"]),
                ("utxos", _) => Some(vec!["all"]),
                ("auto", _) => Some(vec!["on", "off", "sweep", "verbose"]),
                _ => None,
            }
        };

        let complete_token = |prefix: &str, candidates: Vec<String>, head: &[String]| -> Vec<String> {
            let head = if head.is_empty() { String::new() } else { format!("{} ", head.join(" ")) };
            candidates.into_iter().filter(|c| c.starts_with(prefix)).map(|c| format!("{head}{c}")).collect()
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
        use crate::ui::{self, Ink};
        if self.shutdown.load(Ordering::SeqCst) {
            return Some(format!("halt {}", ui::paint(Ink::Gold, "› ")));
        }

        let mut prompt = vec![];

        #[cfg(not(feature = "embedded-node"))]
        let node_running = if let Some(node) = self.node.lock().unwrap().as_ref() { node.is_running() } else { false };
        #[cfg(feature = "embedded-node")]
        let node_running = self.embedded_node_running();

        // Everything in the prompt is in the wallet's own inks. It used to
        // borrow the terminal's red for DISCONNECTED and its blue for the
        // account, which on the founder's screen came out as an alarm and an
        // ochre nobody had chosen. Being offline is a state, not a fault;
        // capitals say it plainly enough.
        if (self.wallet.is_open() && !self.wallet.is_connected()) || (node_running && !self.wallet.is_connected()) {
            prompt.push(ui::paint(Ink::Gold, "DISCONNECTED"));
        } else if self.wallet.is_connected()
            && !self.wallet.is_synced()
            && let Some(state) = self.sync_state()
        {
            prompt.push(state);
        }

        if let Some(descriptor) = self.wallet.descriptor() {
            // The file name — what 'open' listed and what was typed. A title
            // is a description, and the account id meant nothing to anyone
            // who had not written it.
            prompt.push(ui::paint(Ink::Gold, descriptor.filename));

            // Notes, and said so. The ledger is a loading dock whose figure
            // depends on a node being connected and read; folding it in here
            // produced a number that was wrong whenever that was not the
            // case, which is exactly when people look at the prompt. Stable
            // width: a monotonic session pad, so the line being typed never
            // jumps as the digits change.
            if self.prompt_total_valid.load(Ordering::SeqCst) {
                let petals = self.prompt_total_petals.load(Ordering::SeqCst);
                let whole = petals / 100_000_000;
                let hundredths = (petals % 100_000_000) / 1_000_000;
                let suffix =
                    self.wallet.network_id().map(|id| kaspa_wallet_core::utils::kaspa_suffix(&NetworkType::from(id))).unwrap_or("");
                let segment = format!("{}.{:02} {suffix} in notes", whole.separated_string(), hundredths);
                let width = self.prompt_balance_width.fetch_max(segment.len(), Ordering::SeqCst).max(segment.len());
                prompt.push(ui::paint(Ink::Petal, segment.pad_to_width(width)));
            } else {
                prompt.push(ui::paint(Ink::Moss, "..."));
            }
        }

        // A colon would read like every question the wallet asks; '›' is
        // "your turn" and nothing else.
        prompt.is_not_empty().then(|| prompt.join(&ui::paint(Ink::Moss, " • ")) + " " + &ui::paint(Ink::Gold, "› "))
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

    // An embedder that supplied its own banner gets exactly that; otherwise
    // the wallet introduces itself properly. 'connect' leads because nothing
    // works until the wallet can reach a node, and 'help' answers a question
    // a new user has not formed yet.
    // A Marigold note is printed on deep green, and in a browser the page is
    // ours to set — so it is set, before anything is drawn on it. In a real
    // terminal the ground belongs to whoever configured the terminal and is
    // left alone; that is why `Ink::Cream` is the default foreground rather
    // than an actual cream.
    cli.term()
        .set_theme(workflow_terminal::Theme {
            background: Some("#0f1813".into()),
            foreground: Some("#efe7d3".into()),
            cursor: Some("#f3cf82".into()),
            selection: Some("#2a3d33".into()),
        })
        .ok();

    // Measure only after the terminal has been fitted to its element: in a
    // browser it claims eighty by twenty-four until the first layout, which
    // would hand every window the compact mark meant for small ones.
    cli.term().fit().ok();
    // The wallet speaks in its own ink, not the terminal's white.
    cli.term().set_voice(crate::ui::voice());

    match banner {
        Some(banner) => cli.term().writeln(banner),
        None => {
            // Settings are loaded again by `start()` below, but that happens
            // after this point and the splash wants to name the network it is
            // about to use. Loading twice is cheap; guessing is not.
            cli.wallet().load_settings().await.ok();
            cli.advanced.store(cli.wallet().settings().get::<bool>(WalletSettings::Advanced).unwrap_or(false), Ordering::SeqCst);
            let network = cli.wallet().settings().get::<String>(WalletSettings::Network);
            // Whether there is a wallet to open decides the one line under
            // the note: 'open' if there is, 'wallet create' if not.
            let has_wallet = cli.store().wallet_list().await.map(|wallets| !wallets.is_empty()).ok();
            cli.term().writeln("");
            crate::splash::show(&cli, env!("CARGO_PKG_VERSION"), network.as_deref(), has_wallet);
            cli.term().writeln("");
        }
    }

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

/// Text that was written for a log, not for a person: a transport chain, an
/// OS error code, a Rust type or path, or simply too much of it.
fn is_technical(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.len() > 160
        || [" -> ", "os error", "error(", "::", "0x", "wrpc", "rpc", "websocket", "serde", "panicked", "unwrap"]
            .iter()
            .any(|marker| lower.contains(marker))
}

/// Held by 'connect' and 'disconnect' while they run; see [`KaspaCli::switching`].
pub struct SwitchingGuard(Arc<AtomicBool>);

impl Drop for SwitchingGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
