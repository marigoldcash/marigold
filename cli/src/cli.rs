use crate::error::Error;
use crate::helpers::*;
use crate::imports::*;
use crate::modules::miner::Miner;
use crate::modules::node::Node;
use crate::notifier::{Notification, Notifier};
use crate::result::Result;
use kaspa_daemon::{DaemonEvent, DaemonKind, Daemons};
use kaspa_wallet_core::account::Account;
use kaspa_wallet_core::rpc::DynRpcApi;
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
    node: Mutex<Option<Arc<Node>>>,
    miner: Mutex<Option<Arc<Miner>>>,
    notifier: Notifier,
    sync_state: Mutex<Option<SyncState>>,
    /// Auto-mint state. The secret lives in memory only while a wallet is
    /// open and auto-mint is armed (a hot-wallet posture, entered knowingly);
    /// it is never written anywhere and is dropped on close/disarm.
    auto_secret: Mutex<Option<Secret>>,
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
                kaspa_core::log::init_logger(None, "info");
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
            node: Mutex::new(None),
            miner: Mutex::new(None),
            notifier: Notifier::try_new()?,
            sync_state: Mutex::new(None),
            auto_secret: Mutex::new(None),
            auto_payment_secret: Mutex::new(None),
            auto_threshold_petals: Arc::new(AtomicU64::new(0)),
            auto_sweep_utxos: Arc::new(AtomicU64::new(0)),
            auto_busy: Arc::new(AtomicBool::new(false)),
            auto_verbose: Arc::new(AtomicBool::new(false)),
            open_housekeeping_pending: Arc::new(AtomicBool::new(false)),
            prompt_total_petals: Arc::new(AtomicU64::new(0)),
            prompt_total_valid: Arc::new(AtomicBool::new(false)),
            prompt_balance_width: Arc::new(AtomicUsize::new(0)),
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
                    if info.status == kaspa_wallet_core::storage::NoteStatus::Active {
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
    pub async fn report_holdings(self: &Arc<Self>) {
        self.wait_for_account().await;
        let (notes, ledger, pieces) = self.total_holdings().await;
        // Keep the prompt's figure in step, so it is right from the moment a
        // wallet opens rather than after the first minute tick.
        self.prompt_total_petals.store(notes + ledger, Ordering::SeqCst);
        self.prompt_total_valid.store(true, Ordering::SeqCst);
        tprintln!(self, "");
        tprintln!(self, "notes:  {} MAGLD", kaspa_wallet_core::utils::sompi_to_kaspa_string(notes));
        if ledger > 0 {
            tprintln!(
                self,
                "ledger: {} MAGLD  ({} piece{})",
                kaspa_wallet_core::utils::sompi_to_kaspa_string(ledger),
                pieces.separated_string(),
                if pieces == 1 { "" } else { "s" }
            );
        }
        tprintln!(self, "");
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
                    let mintable =
                        kaspa_wallet_core::account::notepool::max_mintable_petals(account.clone(), None, &abortable, None).await?;
                    // No ceiling on the amount. Amounts decompose greedily
                    // from the largest denomination down, so a big mint makes
                    // FEW notes (143,000 MAGLD is about thirty) while a small
                    // one makes dust — 1.32 MAGLD is six notes, two of them
                    // 0.01. The old 100 MAGLD-per-run cap therefore produced
                    // exactly the fragmentation it was meant to avoid, and
                    // took a day to drain a mining wallet besides. What is
                    // actually expensive is the number of input coins, and
                    // that is what consolidation below is for.
                    // Round down to whole MAGLD once the stamp reserve is
                    // full. Minting an exact remainder like 1.32 mints two
                    // 0.01 notes with it, and 0.01s are the fee stamps every
                    // pool operation spends — plan_merges deliberately refuses
                    // to merge them away, so they only ever accumulate. 125 of
                    // them on a wallet that needs a handful is not tidy, it is
                    // a leak (founder report, 2026-09-06). The remainder stays
                    // on the ledger and joins the next whole MAGLD.
                    let whole = kaspa_consensus_core::notepool::DENOMINATION_PETALS[2];
                    let stamps = self.stamp_count().await;
                    let amount = if stamps >= kaspa_wallet_core::account::notepool::STAMP_RESERVE { mintable / whole * whole } else { mintable };
                    if amount == 0 {
                        return Ok(None);
                    }
                    let result = kaspa_wallet_core::account::notepool::mint(
                        account.clone(),
                        secret.clone(),
                        payment_secret.clone(),
                        amount,
                        None,
                        &abortable,
                    )
                    .await?;
                    Ok::<_, kaspa_wallet_core::error::Error>(Some((amount, result.notes.len())))
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
        match kaspa_wallet_core::account::notepool::merge_held_notes(account, secret, 12).await {
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
            let mut first_run_done = false;
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

                if this.open_housekeeping_pending.swap(false, Ordering::SeqCst) {
                    first_run_done = true;
                    last_run = Instant::now();
                    this.report_holdings().await;
                    this.run_housekeeping(true).await;
                    this.report_holdings().await;
                    this.term().refresh_prompt();
                } else if first_run_done && last_run.elapsed().as_secs() >= 60 {
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
                let node = self.node.lock().unwrap().clone();
                if let Some(node) = node {
                    node.handle_event(self, event.into()).await?;
                } else {
                    panic!("Stdio handler: node module is not initialized");
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
                                    tprintln!(this, "Loading...");

                                },
                                Events::AccountSelection { .. } => { },
                                Events::WalletCreate { .. } => { },
                                Events::WalletError { .. } => { },
                                // Events::WalletReady { .. } => { },

                                Events::WalletOpen { .. } |
                                Events::WalletReload { .. } => { },
                                Events::WalletClose => {
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
            term.writeln(style(err.to_string()).red().to_string());
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
                    "mint", "rotate", "move", "redeem", "request", "pay", "import", "export", "pos", "balance", "list", "history", "vault",
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

        let node_running = if let Some(node) = self.node.lock().unwrap().as_ref() { node.is_running() } else { false };

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

    let banner =
        banner.unwrap_or_else(|| format!("Marigold Cli Wallet v{} (type 'help' for list of commands)", env!("CARGO_PKG_VERSION")));
    cli.term().writeln(banner);

    // redirect the global log output to terminal
    #[cfg(not(target_arch = "wasm32"))]
    workflow_log::pipe(Some(cli.clone()));

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
