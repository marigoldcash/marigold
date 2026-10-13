//! The wallet service: everything a front end may ask of an open wallet, in
//! plain words and plain values. The Telegram bot, the desktop app and (from
//! PH-M3 on) the phone apps call it; the terminal wallet uses parts of it.
//! (docs/marigold/MOBILE-PLAN.md, PH-M1c; it lived in `kaspa-cli`'s
//! `serve.rs` until then.)

use crate::providers::MinerHostApi;
use crate::telegram::TelegramConfig;
use futures::TryStreamExt;
use kaspa_consensus_core::network::NetworkId;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::account::notepool::{
    self, BEARER_NOTE_PREFIX, BearerNote, HANDOVER_PREFIX, Handover, HandoverSelection, LOCKED_HANDOVER_PREFIX, LockedHandover,
    SHARE_KEY_PREFIX,
};
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::rpc::DynRpcApi;
use kaspa_wallet_core::storage::NoteStatus;
use kaspa_wallet_core::storage::local::journal::{Journal, JournalEntry};
use kaspa_wallet_core::utils::sompi_to_kaspa_string;
use kaspa_wallet_core::wallet::Wallet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What was paid, for the bot to say.
pub struct Paid {
    pub code: String,
    pub value_petals: u64,
    pub fee_petals: u64,
    pub notes: usize,
    /// The serials handed over, for whoever wants to know when they are taken.
    pub serials: Vec<kaspa_consensus_core::Hash>,
}

/// Where the service tells whoever is watching what the bot did: the log
/// under `serve`, a dim line on the terminal inside the wallet.
pub type Say = Arc<dyn Fn(String) + Send + Sync>;

/// A miner running inside the terminal wallet, read for the bot; the
/// terminal owns starting and stopping it.
pub type LocalMiner = Arc<dyn Fn() -> Option<kaspa_rpc_core::RpcMinerStatus> + Send + Sync>;

/// The open wallet and everything the bot may do with it.
pub struct WalletService {
    wallet: Arc<Wallet>,
    secret: Secret,
    /// Where requests come from, for the history: "telegram" for the bot,
    /// "desktop" for the app.
    origin: &'static str,
    network_id: NetworkId,
    journal: Journal,
    miner: Option<Arc<dyn MinerHostApi>>,
    rpc: Arc<DynRpcApi>,
    /// (UTC day, petals paid out that day) — the daily limit's memory.
    spent: Mutex<(u64, u64)>,
    started: std::time::Instant,
    say: Say,
    local_miner: Option<LocalMiner>,
    /// The storage folder and the wallet's file name: where its Telegram
    /// settings and backup index live.
    folder: String,
    name: String,
}

fn utc_day() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

impl WalletService {
    pub fn new(
        wallet: Arc<Wallet>,
        secret: Secret,
        network_id: NetworkId,
        folder: &str,
        name: &str,
        miner: Option<Arc<dyn MinerHostApi>>,
        say: Say,
        local_miner: Option<LocalMiner>,
        origin: &'static str,
    ) -> Arc<Self> {
        let rpc = wallet.rpc_api();
        Arc::new(Self {
            wallet,
            secret,
            origin,
            network_id,
            journal: Journal::new(folder, name),
            miner,
            rpc,
            spent: Mutex::new((utc_day(), 0)),
            started: std::time::Instant::now(),
            say,
            local_miner,
            folder: folder.to_string(),
            name: name.to_string(),
        })
    }

    /// The miner this service runs, if it runs one.
    pub fn miner_host(&self) -> Option<&Arc<dyn MinerHostApi>> {
        self.miner.as_ref()
    }

    pub fn wallet_name(&self) -> &str {
        &self.name
    }

    /// The service's own remark channel, for a watcher that outlives a call.
    pub fn say_handle(&self) -> Say {
        self.say.clone()
    }

    /// The wallet's Telegram settings file.
    pub fn telegram_path(&self) -> std::path::PathBuf {
        TelegramConfig::path(&self.folder, &self.name)
    }

    pub fn telegram_config(&self) -> Option<TelegramConfig> {
        TelegramConfig::load(&self.telegram_path())
    }

    async fn backup_files(&self) -> std::result::Result<crate::bundle::WalletFiles, String> {
        crate::bundle::WalletFiles::of_wallet(&self.wallet, &self.name).await.map_err(|e| e.to_string())
    }

    /// What the backup screen shows.
    pub async fn backup_status(&self) -> std::result::Result<crate::tgbackup::BackupStatus, String> {
        let files = self.backup_files().await?;
        Ok(crate::tgbackup::status(&files, self.telegram_config().as_ref()))
    }

    /// A full copy now — and the automatic backups from then on.
    pub async fn backup_now(&self) -> std::result::Result<String, String> {
        let cfg = self.telegram_config().ok_or("no Telegram bot is set up for this wallet yet")?;
        if crate::tgbackup::target_chat(&cfg).is_none() {
            return Err("the bot is not paired yet: open it in Telegram and send it the pairing code first".to_string());
        }
        crate::bundle::ensure_recipient(&self.wallet, &self.name, &self.secret).await.map_err(|e| e.to_string())?;
        let files = self.backup_files().await?;
        crate::tgbackup::mark_started(&files).map_err(|e| e.to_string())?;
        let words = crate::bundle::words_for(&self.wallet, &self.secret).await.map_err(|e| e.to_string())?;
        let folder = crate::bundle::Folder::around(&files).map_err(|e| e.to_string())?;
        let say = self.say.clone();
        let progress = move |line: String| say(format!("Backup: {line}"));
        crate::tgbackup::run_folder(&folder, &cfg, Some((&self.name, &words)), true, true, &progress).await.map_err(|e| e.to_string())
    }

    /// 'backup folder <path>' from the desktop: the folder is set and the
    /// first full copy written; an empty path turns it off.
    pub async fn backup_folder(&self, path: &str) -> std::result::Result<String, String> {
        let files = self.backup_files().await?;
        let path = path.trim();
        if path.is_empty() {
            crate::tgbackup::set_folder(&files, None).map_err(|e| e.to_string())?;
            return Ok("Folder backups are off; the files already written stay.".to_string());
        }
        let dir = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let dir = dir.canonicalize().unwrap_or(dir);
        crate::tgbackup::set_folder(&files, Some(&dir)).map_err(|e| e.to_string())?;
        crate::bundle::ensure_recipient(&self.wallet, &self.name, &self.secret).await.map_err(|e| e.to_string())?;
        let files = self.backup_files().await?;
        let words = crate::bundle::words_for(&self.wallet, &self.secret).await.map_err(|e| e.to_string())?;
        let folder = crate::bundle::Folder::around(&files).map_err(|e| e.to_string())?;
        let say = self.say.clone();
        let progress = move |line: String| say(format!("Backup: {line}"));
        let dest = crate::tgbackup::Destination::Folder(dir);
        crate::tgbackup::run_to(&folder, &dest, Some((&self.name, &words)), true, true, &progress).await.map_err(|e| e.to_string())
    }

    /// A wallet of this folder that has no backup key yet, covered with its
    /// words typed once (checked against the wallet; nothing kept but a public key).
    pub async fn backup_cover(&self, name: &str, words: &str) -> std::result::Result<(), String> {
        let files = self.backup_files().await?;
        let folder = crate::bundle::Folder::around(&files).map_err(|e| e.to_string())?;
        crate::bundle::cover(&folder, name, words).map_err(|e| e.to_string())
    }

    /// "Not now" to the one-time question: remembered, never asked again.
    pub async fn backup_decline(&self) -> std::result::Result<(), String> {
        let files = self.backup_files().await?;
        crate::tgbackup::mark_asked(&files).map_err(|e| e.to_string())
    }

    pub async fn backup_automatic(&self, on: bool) -> std::result::Result<(), String> {
        let files = self.backup_files().await?;
        crate::tgbackup::set_paused(&files, !on).map_err(|e| e.to_string())
    }

    /// The sealed bundle for a backup file — every wallet on this computer,
    /// each under its own words — and the name to give it.
    pub async fn backup_file(&self) -> std::result::Result<(String, Vec<u8>), String> {
        crate::bundle::ensure_recipient(&self.wallet, &self.name, &self.secret).await.map_err(|e| e.to_string())?;
        let files = self.backup_files().await?;
        let words = crate::bundle::words_for(&self.wallet, &self.secret).await.map_err(|e| e.to_string())?;
        let (_, packed, names) = crate::bundle::bundle_checked(&files, &words).map_err(|e| e.to_string())?;
        // Named the way the Telegram backups are: readable, and sorted by date.
        let stamp = chrono::Local::now().format("%Y-%m-%d %H.%M");
        Ok((format!("Marigold backup - {} - {stamp}.mgb", crate::bundle::names_label(&names)), packed))
    }

    /// Sets the wallet's bot up the way 'telegram link <token>' does, and
    /// returns the pairing code to send it. A running bot has to be started
    /// (or restarted) by the caller, which owns that task.
    pub fn telegram_setup(&self, token: &str, pin: &str) -> std::result::Result<String, String> {
        let token = token.trim();
        if !token.contains(':') || token.len() < 20 {
            return Err("that does not look like a bot token (BotFather gives one like 123456789:AA...)".to_string());
        }
        if pin.trim().len() < 4 {
            return Err("the PIN needs four characters at least".to_string());
        }
        let existing = self.telegram_config();
        let cfg = TelegramConfig::new(
            token.to_string(),
            pin.trim(),
            existing.map(|c| c.daily_limit_petals).unwrap_or(crate::telegram::DEFAULT_DAILY_LIMIT_PETALS),
        );
        cfg.save(&self.telegram_path()).map_err(|e| format!("cannot save the bot settings: {e}"))?;
        Ok(cfg.pairing_code.unwrap_or_default())
    }

    /// The automatic backup's tick, for a front end that has no housekeeping loop of its own.
    pub async fn backup_tick(&self, state: &Arc<Mutex<crate::tgbackup::AutoState>>) {
        crate::tgbackup::auto_tick_for(
            self.wallet.clone(),
            self.telegram_path(),
            self.name.clone(),
            self.secret.clone(),
            state.clone(),
            self.say.clone(),
            self.say.clone(),
        )
        .await;
    }

    /// Before the wallet closes: post any change not yet posted.
    /// Closes the wallet the way the terminal's 'close' does, so its lock
    /// is let go: the desktop's close used to leave the store open, and the
    /// terminal then said the wallet was open in another program until the
    /// desktop was quit (founder, 2026-10-11).
    pub async fn close_wallet(&self) {
        self.wallet.set_light(None);
        if self.wallet.is_open() {
            if let Err(err) = self.wallet.store().commit(&self.secret).await {
                log::warn!("saving the wallet before closing it: {err}");
            }
            if let Err(err) = self.wallet.close().await {
                log::warn!("closing the wallet: {err}");
            }
        }
        if let Err(err) = self.wallet.stop().await {
            log::warn!("stopping the wallet: {err}");
        }
    }

    pub async fn backup_flush(&self) {
        let say = self.say.clone();
        let relay = move |line: String| say(line);
        crate::tgbackup::flush_for(&self.wallet, &self.telegram_path(), &self.name, &self.secret, &relay, &relay).await;
    }

    /// Which miner, if any: one this service runs, one inside the terminal
    /// wallet, or the background miner on this machine, whose node we are on.
    async fn miner_status(&self) -> Option<(kaspa_rpc_core::RpcMinerStatus, &'static str)> {
        if let Some(host) = &self.miner {
            return Some((host.status(), "here"));
        }
        if let Some(local) = &self.local_miner
            && let Some(status) = local()
        {
            return Some((status, "in the terminal"));
        }
        match self.rpc.get_miner_status().await {
            Ok(status) if status.available => Some((status, "in the background program")),
            _ => None,
        }
    }

    fn miner_line(status: &kaspa_rpc_core::RpcMinerStatus, where_: &str) -> String {
        if status.mining && !status.pool.is_empty() {
            format!(
                "Miner ({where_}): {} on {} threads ({}%) for the pool at {}, {} shares accepted",
                crate::format::format_hashrate(status.hashrate),
                status.threads,
                status.percent,
                status.pool,
                status.shares_accepted
            )
        } else if status.mining {
            format!(
                "Miner ({where_}): {} on {} threads ({}%), {} blocks found, {} accepted",
                crate::format::format_hashrate(status.hashrate),
                status.threads,
                status.percent,
                status.blocks_found,
                status.blocks_accepted
            )
        } else {
            format!("Miner ({where_}): idle")
        }
    }

    pub fn say(&self, line: String) {
        (self.say)(line);
    }

    pub fn ticker(&self) -> &'static str {
        kaspa_wallet_core::utils::kaspa_suffix(&self.network_id.network_type())
    }

    /// self.tag("request").as_str(), "code (desktop)": what happened, and through what.
    fn tag(&self, what: &str) -> String {
        format!("{what} ({})", self.origin)
    }

    fn record(&self, kind: &str, petals: u64, stamp: u64, detail: &str, tx: &str) {
        let _ = self.journal.append(&JournalEntry::now(kind, petals, stamp, detail, tx));
    }

    pub fn spend_allowed(&self, petals: u64, limit: u64) -> std::result::Result<(), String> {
        let (day, spent) = *self.spent.lock().unwrap();
        let spent = if day == utc_day() { spent } else { 0 };
        if spent + petals > limit {
            return Err(format!(
                "That would pass today's limit of {} {} ({} paid so far today). The limit resets at midnight UTC; 'telegram limit' in the wallet changes it.",
                sompi_to_kaspa_string(limit),
                self.ticker(),
                sompi_to_kaspa_string(spent)
            ));
        }
        Ok(())
    }

    /// What the day's total was when the service last ran, from the bot's
    /// file, so a restart does not start the day over.
    pub fn seed_spent(&self, day: u64, petals: u64) {
        if day == utc_day() && petals > 0 {
            *self.spent.lock().unwrap() = (day, petals);
        }
    }

    /// The day and what has been spent in it, for the bot to keep on disk.
    pub fn spent_today(&self) -> (u64, u64) {
        let (day, spent) = *self.spent.lock().unwrap();
        if day == utc_day() { (day, spent) } else { (utc_day(), 0) }
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
        if let Ok(store) = self.wallet.store().as_note_key_store()
            && let Ok(mut stream) = store.iter().await
        {
            while let Ok(Some(info)) = stream.try_next().await {
                if info.status == NoteStatus::Active {
                    counts[info.d as usize] += 1;
                    total += DENOMINATION_PETALS[info.d as usize];
                }
            }
        }
        lines.push(format!("Notes: {} {}", sompi_to_kaspa_string(total), self.ticker()));
        for (i, n) in counts.iter().enumerate().rev() {
            if *n > 0 {
                lines.push(format!("  {} × {}", n, sompi_to_kaspa_string(DENOMINATION_PETALS[i])));
            }
        }
        if let Ok(account) = self.wallet.account()
            && let Some(balance) = account.balance()
        {
            let pending = balance.pending;
            // Two places, the terminal's way: stopped at the hundredth, never
            // rounded up — a balance must not show more than can be spent,
            // and the bot and the terminal must agree (tester Charly saw 0.80
            // against 0.79, 2026-10-06). Notes are whole denominations and
            // need no rounding.
            lines.push(format!(
                "Ledger: {} {}{}",
                crate::format::ledger_amount(balance.mature),
                self.ticker(),
                if pending > 0 { format!(" ({} pending)", crate::format::ledger_amount(pending)) } else { String::new() }
            ));
        }
        if !self.wallet.is_connected() {
            lines.push("Not connected to the network right now; these are the last known figures.".to_string());
        }
        lines.join("\n")
    }

    pub async fn pay(&self, petals: u64) -> std::result::Result<Paid, String> {
        let result = notepool::hand_over(&self.wallet, self.secret.clone(), HandoverSelection::Amount(petals))
            .await
            .map_err(|e| e.to_string())?;
        let code = result.handover.to_text();
        self.record(
            "paid",
            result.value_petals,
            result.stamp_petals + result.transfer.fee_petals,
            self.tag("code handed over").as_str(),
            &result.transfer.transaction_id.to_string(),
        );
        Ok(Paid {
            code,
            value_petals: result.value_petals,
            fee_petals: result.transfer.fee_petals,
            notes: result.handover.notes.len().saturating_sub(1),
            serials: result.handover.notes.iter().map(|(sn, _)| *sn).collect(),
        })
    }

    /// Whether none of these handed-over notes is still waiting to be taken:
    /// taken (superseded), came back from a lock (active again), or gone.
    pub async fn handover_settled(&self, serials: &[kaspa_consensus_core::Hash]) -> bool {
        let Ok(store) = self.wallet.store().as_note_key_store() else { return false };
        let Ok(mut stream) = store.iter().await else { return false };
        while let Ok(Some(info)) = stream.try_next().await {
            if serials.contains(&info.sn) && info.status == NoteStatus::HandedOver {
                return false;
            }
        }
        true
    }

    /// What a request code asks for, once it has been checked: the pinned
    /// amount. A request without one needs the terminal, where an amount
    /// can be typed beside it.
    pub fn request_amount(code: &str) -> std::result::Result<u64, String> {
        let request = notepool::PaymentRequest::from_text(code).map_err(|e| e.to_string())?;
        request.verify().map_err(|e| e.to_string())?;
        request.amount_petals.ok_or_else(|| "this request pins no amount; in the wallet: pay <code> <amount>".to_string())
    }

    /// Pay a request code: the receiver's key and amount, signed by them,
    /// checked again here. `Paid::code` is the receipt to hand back.
    pub async fn pay_request(&self, code: &str) -> std::result::Result<Paid, String> {
        self.pay_request_with(code, None).await
    }

    /// Pays a request; `amount` is what the payer chose when the request pins
    /// none (the desktop wallet asks for it), and is ignored when it does.
    pub async fn pay_request_with(&self, code: &str, amount: Option<u64>) -> std::result::Result<Paid, String> {
        let request = notepool::PaymentRequest::from_text(code).map_err(|e| e.to_string())?;
        request.verify().map_err(|e| e.to_string())?;
        let (amount, chosen) = match request.amount_petals {
            Some(pinned) => (pinned, None),
            None => {
                let chosen = amount.ok_or_else(|| "this request pins no amount; say how much to pay".to_string())?;
                (chosen, Some(chosen))
            }
        };
        let result =
            notepool::pay_payment_request(&self.wallet, self.secret.clone(), request, chosen).await.map_err(|e| e.to_string())?;
        self.record("paid", amount, result.fee_petals, self.tag("request").as_str(), &result.transaction_id.to_string());
        let receipt =
            notepool::PaymentReceipt { transaction_id: result.transaction_id, request_pk: request.pk, amount_petals: amount };
        Ok(Paid {
            code: receipt.to_text(),
            value_petals: amount,
            fee_petals: result.fee_petals,
            notes: result.external_serials.len(),
            serials: vec![],
        })
    }

    /// 'pay' to a share key under a lock (P8.0g): the receiver's to take until
    /// `seconds` from now, ours again after.
    pub async fn pay_locked(&self, petals: u64, key: &str, seconds: u64) -> std::result::Result<Paid, String> {
        let share_pk = notepool::share_key_from_text(key).map_err(|e| e.to_string())?;
        let bps = kaspa_consensus_core::config::params::Params::from(self.network_id).bps();
        let now = self.rpc.get_server_info().await.map_err(|e| e.to_string())?.virtual_daa_score;
        let result = notepool::hand_over_locked(&self.wallet, self.secret.clone(), petals, share_pk, now + seconds * bps)
            .await
            .map_err(|e| e.to_string())?;
        let code = result.handover.to_text();
        self.record(
            "offered",
            result.value_petals,
            result.stamp_petals + result.transfer.fee_petals,
            self.tag("locked code").as_str(),
            &result.transfer.transaction_id.to_string(),
        );
        Ok(Paid {
            code,
            value_petals: result.value_petals,
            fee_petals: result.transfer.fee_petals,
            notes: result.handover.notes.len().saturating_sub(1),
            serials: result.handover.notes.iter().map(|(sn, _)| *sn).collect(),
        })
    }

    /// A share key to hand out (P8.0g).
    /// The wallet's 24 recovery words, against the password given now: a wrong
    /// one cannot open the vault, so nothing is shown. For the desktop wallet;
    /// the bot never offers this.
    pub async fn recovery_words(&self, password: Secret) -> std::result::Result<String, String> {
        let store = self.wallet.store().as_note_key_store().map_err(|e| e.to_string())?;
        // `recovery_words` alone would answer from the key an open wallet already holds, whatever
        // password was typed; the file is read again to check it.
        let wrong = |_| "that is not this wallet's password".to_string();
        store.verify_secret(&password).await.map_err(wrong)?;
        store.recovery_words(&password).await.map_err(wrong)
    }

    pub async fn share_key(&self, label: &str) -> std::result::Result<String, String> {
        let store = self.wallet.store().as_note_key_store().map_err(|e| e.to_string())?;
        let info = store.add_share_key(&self.secret, label).await.map_err(|e| e.to_string())?;
        Ok(notepool::share_key_to_text(&info.pk))
    }

    pub fn is_share_key(text: &str) -> bool {
        text.starts_with(SHARE_KEY_PREFIX)
    }

    pub async fn receive(&self, code: &str) -> std::result::Result<String, String> {
        if code.starts_with(LOCKED_HANDOVER_PREFIX) {
            let handover = LockedHandover::from_text(code).map_err(|e| e.to_string())?;
            let result = notepool::receive_locked(&self.wallet, self.secret.clone(), handover).await.map_err(|e| e.to_string())?;
            let stamp = result.notes.iter().any(|(_, d)| *d == kaspa_consensus_core::notepool::DenominationTag::D0_01);
            let value = if stamp { result.value_petals - DENOMINATION_PETALS[0] } else { result.value_petals };
            self.record("received", value, 0, self.tag("locked code").as_str(), &result.rotation.transaction_id.to_string());
            Ok(format!(
                "Received {} {} in {} note(s), taken in time and made yours alone.",
                sompi_to_kaspa_string(value),
                self.ticker(),
                result.notes.len() - usize::from(stamp)
            ))
        } else if code.starts_with(HANDOVER_PREFIX) {
            let handover = Handover::from_text(code).map_err(|e| e.to_string())?;
            let result = notepool::receive_handover(&self.wallet, self.secret.clone(), handover).await.map_err(|e| e.to_string())?;
            let stamp = result.notes.iter().any(|(_, d)| *d == kaspa_consensus_core::notepool::DenominationTag::D0_01);
            let value = if stamp { result.value_petals - DENOMINATION_PETALS[0] } else { result.value_petals };
            self.record("received", value, 0, self.tag("code").as_str(), &result.rotation.transaction_id.to_string());
            Ok(format!(
                "Received {} {} in {} note(s). Made yours alone with the payer's stamp.",
                sompi_to_kaspa_string(value),
                self.ticker(),
                result.notes.len() - usize::from(stamp)
            ))
        } else if code.starts_with(BEARER_NOTE_PREFIX) {
            let bearer = BearerNote::from_text(code).map_err(|e| e.to_string())?;
            let (sn, value) = (bearer.sn, DENOMINATION_PETALS[bearer.d as usize]);
            match notepool::bearer_import(&self.wallet, self.secret.clone(), bearer).await {
                Ok(result) => {
                    self.record(
                        "received",
                        value,
                        result.rotation.fee_petals,
                        self.tag("note").as_str(),
                        &result.rotation.transaction_id.to_string(),
                    );
                    self.note_spent(result.rotation.fee_petals);
                    Ok(format!(
                        "Received {} {} (fee {}).",
                        sompi_to_kaspa_string(value),
                        self.ticker(),
                        sompi_to_kaspa_string(result.rotation.fee_petals)
                    ))
                }
                Err(err) => {
                    // The key is stored before the lock is turned, so a failed
                    // turn still leaves the note in the wallet: it is received,
                    // and the history must say so (tester, 2026-10-10: a faucet
                    // note counted in the balance and absent from the history).
                    let kept = match self.wallet.store().as_note_key_store() {
                        Ok(store) => {
                            matches!(store.load_info(&sn).await, Ok(Some(info)) if info.status == kaspa_wallet_core::storage::NoteStatus::Active)
                        }
                        Err(_) => false,
                    };
                    if !kept {
                        return Err(err.to_string());
                    }
                    self.record("received", value, 0, self.tag("note, lock not turned yet").as_str(), "");
                    Ok(format!(
                        "Received {} {}. Its lock could not be turned yet ({err}); the wallet turns it on its own once connected, or 'note rotate all' does it now.",
                        sompi_to_kaspa_string(value),
                        self.ticker()
                    ))
                }
            }
        } else {
            Err("that is not a Marigold code (marigoldpay: or marigoldnote:)".to_string())
        }
    }

    pub async fn request(&self, petals: Option<u64>) -> std::result::Result<String, String> {
        let request = notepool::create_payment_request(&self.wallet, &self.secret, petals, None).await.map_err(|e| e.to_string())?;
        Ok(request.to_text())
    }

    /// Watch a request until it is paid, up to `timeout`; the line to say if it was.
    pub async fn await_request(&self, code: &str, timeout: Duration) -> std::result::Result<Option<String>, String> {
        let request = notepool::PaymentRequest::from_text(code).map_err(|e| e.to_string())?;
        match notepool::await_payment_request(&self.wallet, &self.secret, request.pk, timeout).await {
            Ok(claimed) => {
                self.record("received", claimed.total_petals, 0, self.tag("request").as_str(), "");
                Ok(Some(format!(
                    "Paid: {} {} arrived in {} note(s).",
                    sompi_to_kaspa_string(claimed.total_petals),
                    self.ticker(),
                    claimed.notes.len()
                )))
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
                let when = chrono::DateTime::<chrono::Utc>::from_timestamp(e.at as i64, 0)
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                format!("{when}  {:<9} {:>12}  {}", e.kind, sompi_to_kaspa_string(e.petals), e.detail)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub async fn status_text(&self) -> String {
        let mut lines = Vec::new();
        match self.rpc.get_server_info().await {
            Ok(info) => lines.push(format!(
                "Network: {} ({} {})",
                if info.is_synced { "in sync" } else { "catching up" },
                info.network_id,
                info.server_version
            )),
            Err(_) => lines.push("Network: not reachable".to_string()),
        }
        lines.push(format!(
            "Wallet: {} for {}",
            if self.wallet.is_connected() { "connected" } else { "not connected" },
            crate::format::humanised_minutes(self.started.elapsed().as_secs() / 60)
        ));
        if let Some(light) = self.wallet.light() {
            let s = light.status();
            let ago = workflow_core::time::unixtime_as_millis_u64().saturating_sub(s.checked_ms) / 1000;
            lines.push(format!(
                "Light wallet: {} notes in your own copy of the pool, as of DAA {}; the trustees' anchor agreed {ago}s ago. This computer is never asked which notes are yours.",
                s.notes, s.at_daa
            ));
        }
        match self.miner_status().await {
            Some((status, where_)) => {
                lines.push(Self::miner_line(&status, where_));
                if status.mining && !status.address.is_empty() {
                    lines.push(format!("Rewards go to: {}", status.address));
                }
            }
            None => lines.push("Miner: none".to_string()),
        }
        lines.join("\n")
    }

    pub async fn mine(&self, what: Option<&str>) -> String {
        let Some(host) = &self.miner else {
            // Not ours to run: the background miner takes start and stop
            // over RPC; one inside the terminal is the terminal's.
            let remote = matches!(self.rpc.get_miner_status().await, Ok(s) if s.available);
            if remote {
                return match what {
                    Some("start") => match self.rpc.control_miner(true, None).await {
                        Ok(s) => format!("Mining in the background program: {} threads ({}%).", s.threads, s.percent),
                        Err(e) => format!("Could not start: {e}"),
                    },
                    Some("stop") => match self.rpc.control_miner(false, None).await {
                        Ok(_) => "The background miner is idle now.".to_string(),
                        Err(e) => format!("Could not stop: {e}"),
                    },
                    _ => match self.miner_status().await {
                        Some((s, w)) => Self::miner_line(&s, w),
                        None => "Miner: none".to_string(),
                    },
                };
            }
            return match self.miner_status().await {
                Some((s, w)) => format!("{}\nStart and stop it from the terminal.", Self::miner_line(&s, w)),
                None => "No miner runs with this wallet. In the terminal, 'mine start'; or 'marigold-cli mine-to' as a service."
                    .to_string(),
            };
        };
        match what {
            Some("start") => match host.start(host.mining().unwrap_or(50)) {
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
                    format!(
                        "Mining: {} on {} threads ({}%), {} blocks found, {} accepted.",
                        crate::format::format_hashrate(s.hashrate),
                        s.threads,
                        s.percent,
                        s.blocks_found,
                        s.blocks_accepted
                    )
                } else if self.origin == "telegram" {
                    // The bot's user types commands; the desktop has a button
                    // (spacecharly, GitHub #27 and #35).
                    "Not mining. /mine start".to_string()
                } else {
                    "Not mining.".to_string()
                }
            }
        }
    }
}
