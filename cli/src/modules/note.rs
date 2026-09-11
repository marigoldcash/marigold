use crate::imports::*;
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::account::notepool;
use kaspa_wallet_core::account::notepool::{
    BearerNote, PaymentRequest, RedeemSelection, await_payment_request, create_payment_request, deep_verify, export_active_entries,
    light_verify, light_verify_vault, paper_export_decode_page, paper_export_encode, paper_export_missing_pages,
    paper_export_peek_header, plan_restore_rotation,
};
use kaspa_wallet_core::storage::local::notevault::NoteVault;
use kaspa_wallet_core::storage::{NoteKeyEntry, NoteKeyInfo, NoteProvenance, NoteStatus};
use std::path::Path;
use std::time::Duration;
use workflow_core::abortable::Abortable;

/// Recursively copy a directory tree (native fs — the CLI is native-only, unlike
/// `wallet-core` which must also build for wasm32). Used for `note vault
/// backup`/`restore`'s "copy the files" half of "24 words + the files".
fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Render a payload as a terminal QR code (dense unicode half-blocks). Falls back to
/// nothing (text-only) if the payload somehow exceeds QR capacity — the text form
/// printed alongside is always sufficient.
///
/// The CRLF conversion is not cosmetic. The terminal runs in raw mode, where a
/// bare `\n` moves down a line without returning to column one, so a 29-line QR
/// printed in one call comes out as a diagonal smear — unreadable by eye and
/// unscannable by anything. `writeln` appends `\n\r` to the string it is given
/// but does not touch the newlines inside it, so a multi-line payload has to
/// arrive already converted. Done here rather than at the four call sites
/// because the fifth one would forget.
fn qr_string(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(code.render::<qrcode::render::unicode::Dense1x2>().build().crlf())
}

#[derive(Default, Handler)]
#[help("Mint, redeem, send, receive, and list notes")]
pub struct Note;

impl Note {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, mut argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        if argv.is_empty() {
            return self.display_help(ctx, argv).await;
        }

        let action = argv.remove(0);
        match action.as_str() {
            "mint" => self.mint(&ctx, argv).await,
            "rotate" => self.rotate(&ctx, argv).await,
            "move" => self.move_notes(&ctx, argv).await,
            "redeem" => self.redeem(&ctx, argv).await,
            "request" => self.request(&ctx, argv).await,
            "pay" => self.pay(&ctx, argv).await,
            "import" => self.import(&ctx, argv).await,
            "export" => self.export(&ctx, argv).await,
            "pos" => self.pos(&ctx, argv).await,
            "balance" => self.balance(&ctx).await,
            "mirror" => self.mirror(&ctx, argv).await,
            "verify" => self.verify(&ctx, argv).await,
            "list" => self.list(&ctx).await,
            "history" => self.history(&ctx).await,
            "unknown" => self.unknown(&ctx).await,
            "vault" => self.vault(&ctx, argv).await,
            "help" => self.display_help(ctx, argv).await,
            v => {
                tprintln!(ctx, "unknown command: '{v}'\r\n");
                self.display_help(ctx, argv).await
            }
        }
    }

    /// `note request [amount]` — create a payment request (fresh pk, persisted
    /// before display), show its QR + text, then watch for the payment.
    async fn request(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        let account = ctx.wallet().account()?;
        // Amount is optional per POOL-SPEC.md P5.6's two QR forms: pinned (40-byte)
        // or left for the payer to fill in (32-byte, the printed/static form).
        let amount_petals = if argv.is_empty() { None } else { Some(try_parse_required_nonzero_kaspa_as_sompi_u64(argv.first())?) };
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        let request = create_payment_request(&ctx.wallet(), &wallet_secret, amount_petals).await?;
        let text = request.to_text();
        if let Some(qr) = qr_string(&text) {
            tprintln!(ctx, "{}", qr);
        }
        tprintln!(ctx, "{text}");
        match amount_petals {
            Some(amount) => tprintln!(ctx, "requesting {} MAGLD", sompi_to_kaspa_string(amount)),
            None => tprintln!(ctx, "no pinned amount - the payer chooses"),
        }

        let timeout = Duration::from_secs(120);
        tprintln!(ctx, "watching for payment (up to {}s; the request stays claimable after a timeout)...", timeout.as_secs());
        match await_payment_request(&ctx.wallet(), &wallet_secret, request.pk, timeout).await {
            Ok(claimed) => {
                tprintln!(ctx, "payment received: {} MAGLD in {} note(s):", sompi_to_kaspa_string(claimed.total_petals), claimed.notes.len());
                for note in &claimed.notes {
                    tprintln!(ctx, "  {} - {}", note.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[note.d as usize]));
                }
                tprintln!(ctx, "");
            }
            Err(err) => {
                tprintln!(ctx, "{err}");
                tprintln!(ctx, "(re-run 'note request' later or watch 'note list' - the request key remains stored)\r\n");
            }
        }
        Ok(())
    }

    /// `note pay <request-text> [amount]` — pay a payment request from held notes.
    async fn pay(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note pay <request-text> [amount]'\r\n");
            return Ok(());
        }
        let account = ctx.wallet().account()?;
        let request = PaymentRequest::from_text(&argv[0])?;
        let amount_override =
            if argv.len() > 1 { Some(try_parse_required_nonzero_kaspa_as_sompi_u64(argv.get(1))?) } else { None };
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        let result = account.pay_payment_request(wallet_secret, request, amount_override).await?;
        tprintln!(
            ctx,
            "paid {} note(s) (fee {} MAGLD); tx: {}",
            result.external_serials.len(),
            sompi_to_kaspa_string(result.fee_petals),
            result.transaction_id
        );
        tprintln!(ctx, "");
        Ok(())
    }

    /// `note import <bearer-text>` — bearer-note import: verify on-chain, store
    /// (Hot), immediately rotate to fresh Cold keys, report.
    async fn import(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note import <bearer-text>'\r\n");
            return Ok(());
        }
        let account: Arc<dyn kaspa_wallet_core::account::Account> = ctx.wallet().account()?;
        let bearer = BearerNote::from_text(&argv[0])?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        self.ensure_vault_interactive(ctx, &wallet_secret).await?;

        let result = account.clone().bearer_import(wallet_secret.clone(), bearer).await?;
        tprintln!(ctx, "imported note {} and immediately rotated it to fresh cold key(s):", result.imported_sn);
        for note in &result.rotation.own_notes {
            tprintln!(ctx, "  {} - {}", note.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[note.d as usize]));
        }
        tprintln!(
            ctx,
            "rotation tx: {} (fee {} MAGLD); the note is yours once this confirms",
            result.rotation.transaction_id,
            sompi_to_kaspa_string(result.rotation.fee_petals)
        );

        // Housekeeping on receipt: ten notes of one size become one of the
        // next, so a vault never accumulates a drawer full of small change.
        match notepool::merge_held_notes(account.clone(), wallet_secret, 4).await {
            Ok((0, None)) => {}
            Ok((merged, failure)) => {
                if merged > 0 {
                    tprintln!(ctx, "consolidated {merged} group(s) of ten notes into larger ones");
                }
                if let Some(reason) = failure {
                    tprintln!(ctx, "(note consolidation stopped: {reason})");
                }
            }
            Err(err) => tprintln!(ctx, "(note consolidation skipped: {err})"),
        }
        tprintln!(ctx, "");
        Ok(())
    }

    /// `note export <serial>` — bearer-export a note: auto-isolate if its key is
    /// shared, wait for the isolation to land on-chain, then show the handover
    /// QR + text and mark the note handed over.
    async fn export(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note export <serial>'\r\n");
            return Ok(());
        }
        let account = ctx.wallet().account()?;
        let sn = argv[0].parse::<Hash>().map_err(|_| Error::Custom(format!("'{}' is not a valid note serial (32-byte hex)", argv[0])))?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        let result = account.bearer_export(wallet_secret, sn).await?;
        if let Some(isolation) = &result.isolation {
            tprintln!(
                ctx,
                "key was shared - isolated onto a fresh solo key first (tx {}, fee {} MAGLD)",
                isolation.transaction_id,
                sompi_to_kaspa_string(isolation.fee_petals)
            );
            tprintln!(ctx, "waiting for the isolation to confirm before the receiver can verify it...");
            let rpc = ctx.wallet().rpc_api();
            let mut confirmed = false;
            for _ in 0..120 {
                if rpc.get_notes_by_serial(vec![result.bearer.sn]).await?.iter().any(|entry| entry.sn == result.bearer.sn) {
                    confirmed = true;
                    break;
                }
                workflow_core::task::sleep(Duration::from_millis(500)).await;
            }
            if !confirmed {
                tprintln!(ctx, "isolation not yet confirmed - share the payload below only once it is (check 'note list')\r\n");
            }
        }

        let text = result.bearer.to_text();
        if let Some(qr) = qr_string(&text) {
            tprintln!(ctx, "{}", qr);
        }
        tprintln!(ctx, "{text}");
        tprintln!(
            ctx,
            "note {} ({} MAGLD) handed over - it is the receiver's once they rotate it; both of you can spend it until then",
            result.bearer.sn,
            sompi_to_kaspa_string(DENOMINATION_PETALS[result.bearer.d as usize])
        );
        tprintln!(ctx, "");
        Ok(())
    }

    /// `note pos <amount>` — one POS checkout: fresh landing-pad `pk`, wait for
    /// exact payment, sweep the instant it confirms.
    async fn pos(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note pos <amount>'\r\n");
            return Ok(());
        }
        let account = ctx.wallet().account()?;
        let amount_petals = try_parse_required_nonzero_kaspa_as_sompi_u64(argv.first())?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        tprintln!(ctx, "checkout: {} MAGLD", sompi_to_kaspa_string(amount_petals));
        let timeout = Duration::from_secs(120);
        let ctx_for_qr = ctx.clone();
        let on_request = Box::new(move |request: &notepool::PaymentRequest| {
            let text = request.to_text();
            if let Some(qr) = qr_string(&text) {
                tprintln!(ctx_for_qr, "{}", qr);
            }
            tprintln!(ctx_for_qr, "{text}");
        });
        let result = account.pos_checkout(wallet_secret, amount_petals, timeout, Some(on_request)).await?;

        tprintln!(
            ctx,
            "payment received: {} MAGLD in {} note(s); swept to {} fresh key(s) (fee {} MAGLD), tx {}",
            sompi_to_kaspa_string(result.claimed.total_petals),
            result.claimed.notes.len(),
            result.sweep.own_notes.len(),
            sompi_to_kaspa_string(result.sweep.fee_petals),
            result.sweep.transaction_id
        );
        tprintln!(ctx, "one-note-one-key restored\r\n");
        Ok(())
    }

    fn progress_printer(ctx: &Arc<KaspaCli>) -> notepool::NoteProgress {
        let ctx = ctx.clone();
        std::sync::Arc::new(move |message: String| {
            tprintln!(ctx, "  {message}");
        })
    }

    async fn mint(&self, ctx: &Arc<KaspaCli>, mut argv: Vec<String>) -> Result<()> {
        let account = ctx.wallet().account()?;

        // 'note mint' with no amount offers to mint everything; 'note mint all'
        // does it without asking. "Everything" is fee-aware: the mint
        // transaction's own fee comes out of the same balance.
        let all = match argv.first().map(|s| s.to_lowercase()).as_deref() {
            None => {
                let abortable = Abortable::default();
                tprintln!(ctx, "Estimating the largest mintable amount — this dry-runs a sweep of your entire ledger balance and can take a while on a large wallet...");
                let progress = Self::progress_printer(ctx);
                let max = notepool::max_mintable_petals(account.clone(), None, &abortable, Some(progress)).await?;
                if max == 0 {
                    tprintln!(ctx, "usage: 'note mint <amount>' or 'note mint all'  (no mintable balance right now)\r\n");
                    return Ok(());
                }
                let answer = ctx
                    .term()
                    .ask(false, &format!("Mint all available (~{} MAGLD)? [y/N]: ", sompi_to_kaspa_string(max)))
                    .await?
                    .trim()
                    .to_lowercase();
                if answer != "y" && answer != "yes" {
                    tprintln!(ctx, "usage: 'note mint <amount>' or 'note mint all'\r\n");
                    return Ok(());
                }
                Some(max)
            }
            Some("all") => {
                argv.remove(0);
                let abortable = Abortable::default();
                tprintln!(ctx, "Estimating the largest mintable amount — this dry-runs a sweep of your entire ledger balance and can take a while on a large wallet...");
                let progress = Self::progress_printer(ctx);
                let max = notepool::max_mintable_petals(account.clone(), None, &abortable, Some(progress)).await?;
                if max == 0 {
                    tprintln!(ctx, "no mintable balance right now\r\n");
                    return Ok(());
                }
                Some(max)
            }
            _ => None,
        };

        let amount_petals = match all {
            Some(petals) => petals,
            None => {
                let petals = try_parse_required_nonzero_kaspa_as_sompi_u64(argv.first())?;
                argv.remove(0);
                petals
            }
        };
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        self.ensure_vault_interactive(ctx, &wallet_secret).await?;
        let abortable = Abortable::default();

        tprintln!(
            ctx,
            "Minting {} MAGLD — building, signing, and submitting the funding transactions (a large wallet sweeps in many batches; progress below)...",
            sompi_to_kaspa_string(amount_petals)
        );
        let progress = Self::progress_printer(ctx);
        // "All" goes through mint_max, which retries with a larger change
        // reserve when the transaction comes out too heavy — the estimate is
        // shaped differently from the real mint and cannot be made exact. An
        // explicit amount is taken literally: the user asked for a number.
        let (amount_petals, result) = if all.is_some() {
            match notepool::mint_max(account.clone(), wallet_secret, payment_secret, None, &abortable, Some(progress)).await? {
                Some((amount, result)) => (amount, result),
                None => {
                    tprintln!(ctx, "Nothing could be minted — the fee would exceed what is on the ledger.\r\n");
                    return Ok(());
                }
            }
        } else {
            let result = notepool::mint_with_progress(
                account.clone(),
                wallet_secret,
                payment_secret,
                amount_petals,
                None,
                &abortable,
                Some(progress),
            )
            .await?;
            (amount_petals, result)
        };

        tprintln!(ctx, "minted {} MAGLD into {} note(s):", sompi_to_kaspa_string(amount_petals), result.notes.len());
        for entry in &result.notes {
            tprintln!(ctx, "  {} - {}", entry.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[entry.d as usize]));
        }
        tprintln!(ctx, "tx: {}\r\n", result.transaction_ids.last().expect("mint always submits at least one transaction"));

        Ok(())
    }


    /// Interactive vault ceremony for wallets created before vaults moved into
    /// the creation wizard — replaces the lazy auto-create that logged the 24
    /// words as a passing warning (which is how the founder's vault words
    /// ended up in scrollback, 2026-09-05).
    async fn ensure_vault_interactive(&self, ctx: &Arc<KaspaCli>, wallet_secret: &Secret) -> Result<()> {
        let store = ctx.wallet().store().as_note_key_store()?;
        if store.vault_exists().await? {
            return Ok(());
        }
        tprintln!(ctx, "");
        tprintln!(ctx, "This wallet has no note vault yet — creating one now.");
        let words = loop {
            let input = ctx
                .term()
                .ask(false, "Enter your own 24-word vault recovery phrase, or press <enter> to generate one: ")
                .await?
                .trim()
                .to_string();
            if input.is_empty() {
                break None;
            }
            let count = input.split_whitespace().count();
            if count != 24 {
                tprintln!(ctx, "Expected 24 words, got {count} — try again (or press <enter> to generate)");
                continue;
            }
            match kaspa_bip32::Mnemonic::new(input.clone(), kaspa_bip32::Language::default()) {
                Ok(_) => break Some(input),
                Err(err) => {
                    tprintln!(ctx, "Not a valid 24-word phrase ({err}) — try again (or press <enter> to generate)");
                }
            }
        };
        match words {
            Some(words) => {
                store.vault_restore_from_words(&words, wallet_secret).await?;
                tprintln!(ctx, "Note vault created from your recovery phrase.\r\n");
            }
            None => {
                let words = store.vault_create(wallet_secret).await?;
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("Your note vault recovery phrase — write these 24 words down NOW:").red());
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style(&words).cyan());
                tprintln!(ctx, "");
                tprintln!(ctx, "Recovery requires BOTH these words AND the vault files ('note vault backup <dir>').");
                ctx.term().ask(false, "Press <enter> once you have written them down: ").await?;
            }
        }
        Ok(())
    }

    /// Shared batched-rotation driver (same per-batch liveness re-check as the
    /// vault-restore flow — an earlier batch's fee stamp may already have
    /// rotated a later batch's note; that's the mechanism working, not an error).
    async fn rotate_serials(
        &self,
        ctx: &Arc<KaspaCli>,
        account: &Arc<dyn kaspa_wallet_core::account::Account>,
        wallet_secret: &Secret,
        serials: Vec<Hash>,
    ) -> Result<()> {
        let store = ctx.wallet().store().as_note_key_store()?;
        let batches = plan_restore_rotation(serials);
        for (i, batch) in batches.iter().enumerate() {
            let mut still_active = Vec::with_capacity(batch.len());
            for sn in batch {
                if let Some(info) = store.load_info(sn).await?
                    && info.status == NoteStatus::Active
                {
                    still_active.push(*sn);
                }
            }
            if still_active.is_empty() {
                tprintln!(ctx, "  batch {}/{}: already rotated by an earlier batch's fee stamp - skipped", i + 1, batches.len());
                continue;
            }
            match account.clone().rotate_notes(wallet_secret.clone(), still_active).await {
                Ok(result) => tprintln!(
                    ctx,
                    "  batch {}/{}: {} note(s), tx {} (fee {} MAGLD)",
                    i + 1,
                    batches.len(),
                    result.own_notes.len(),
                    result.transaction_id,
                    sompi_to_kaspa_string(result.fee_petals)
                ),
                Err(err) => tprintln!(ctx, "  batch {}/{}: failed - {err} (other batches still attempted)", i + 1, batches.len()),
            }
        }
        Ok(())
    }

    /// `note rotate all` / `note rotate <serial> [...]` — on-chain rotation to
    /// fresh cold keys: the remedy when a vault's recovery words or files may
    /// have leaked, making every old or stolen copy of the keys worthless.
    async fn rotate(&self, ctx: &Arc<KaspaCli>, mut argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note rotate all' or 'note rotate <serial> [<serial> ...]'\r\n");
            return Ok(());
        }
        let account = ctx.wallet().account()?;
        let store = ctx.wallet().store().as_note_key_store()?;
        let serials: Vec<Hash> = if argv[0].to_lowercase() == "all" {
            let mut stream = store.iter().await?;
            let mut serials = Vec::new();
            while let Some(info) = stream.try_next().await? {
                if info.status == NoteStatus::Active {
                    serials.push(info.sn);
                }
            }
            serials
        } else {
            let mut serials = Vec::with_capacity(argv.len());
            for raw in argv.drain(..) {
                let sn = raw.parse::<Hash>().map_err(|_| Error::Custom(format!("'{raw}' is not a valid note serial (32-byte hex)")))?;
                serials.push(sn);
            }
            serials
        };
        if serials.is_empty() {
            tprintln!(ctx, "no active notes to rotate\r\n");
            return Ok(());
        }
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        tprintln!(ctx, "rotating {} note(s) to fresh cold keys...", serials.len());
        self.rotate_serials(ctx, &account, &wallet_secret, serials).await?;
        tprintln!(ctx, "rotation complete - every old copy (backups, exports, stolen files) of these keys is now worthless\r\n");
        Ok(())
    }

    /// `note move` — move ALL active notes into another wallet's vault on this
    /// machine: pure key handover between vaults, no chain transaction, no
    /// UTXO involvement (the litepaper's "move between wallets" promise). The
    /// destination stores them Hot (a key that crossed a wallet boundary), and
    /// an optional up-front rotation covers the compromised-vault case.
    async fn move_notes(&self, ctx: &Arc<KaspaCli>, _argv: Vec<String>) -> Result<()> {
        let account = ctx.wallet().account()?;
        let store = ctx.wallet().store().as_note_key_store()?;
        let Some(descriptor) = ctx.store().descriptor() else {
            tprintln!(ctx, "Unable to resolve the open wallet's file\r\n");
            return Ok(());
        };

        // Destination picker (other wallets only).
        let wallets = ctx.store().wallet_list().await?;
        let others: Vec<_> = wallets.into_iter().filter(|w| w.filename != descriptor.filename).collect();
        if others.is_empty() {
            tprintln!(ctx, "No other wallet exists to move notes into — create one first with 'wallet create <name>', then re-run 'note move'.\r\n");
            return Ok(());
        }
        tprintln!(ctx, "");
        for (i, w) in others.iter().enumerate() {
            let n = i + 1;
            match &w.title {
                Some(title) => tprintln!(ctx, "{n}: {title} ({})", w.filename),
                None => tprintln!(ctx, "{n}: {}", w.filename),
            }
        }
        tprintln!(ctx, "");
        let selection =
            ctx.term().ask(false, &format!("Move all active notes to which wallet [1..{}]? ", others.len())).await?.trim().to_string();
        let dest = match selection.parse::<usize>() {
            Ok(i) if i >= 1 && i <= others.len() => others[i - 1].filename.clone(),
            _ => {
                tprintln!(ctx, "No such wallet: '{selection}'\r\n");
                return Ok(());
            }
        };

        // Secrets: source (decrypt keys out) and destination (write them in).
        let wallet_secret = Secret::new(
            ctx.term().ask(true, &format!("Enter the CURRENT wallet's ('{}') password: ", descriptor.filename)).await?.trim().as_bytes().to_vec(),
        );
        let dest_secret =
            Secret::new(ctx.term().ask(true, &format!("Enter the password for '{dest}': ")).await?.trim().as_bytes().to_vec());

        // Validate the destination password before touching anything.
        use kaspa_wallet_core::storage::local::{Storage, WalletStorage, wallet_file_name};
        let folder: String = ctx
            .wallet()
            .settings()
            .get(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
        let dest_storage = Storage::try_new_with_folder(&folder, &wallet_file_name(&dest))?;
        let dest_wallet = WalletStorage::try_load(&dest_storage).await?;
        if dest_wallet.payload(&dest_secret).is_err() {
            tprintln!(ctx, "Unable to decrypt '{dest}' with that password — nothing was moved.\r\n");
            return Ok(());
        }

        // Optional (recommended) on-chain rotation first: moving copies the
        // SAME keys — only rotation makes old/stolen copies worthless.
        let answer = ctx
            .term()
            .ask(false, "Rotate the notes on-chain before moving (recommended - makes any old or stolen copies of their keys worthless)? [Y/n]: ")
            .await?
            .trim()
            .to_lowercase();
        if answer.is_empty() || answer == "y" || answer == "yes" {
            let mut stream = store.iter().await?;
            let mut serials = Vec::new();
            while let Some(info) = stream.try_next().await? {
                if info.status == NoteStatus::Active {
                    serials.push(info.sn);
                }
            }
            tprintln!(ctx, "rotating {} note(s) first...", serials.len());
            self.rotate_serials(ctx, &account, &wallet_secret, serials).await?;
        }

        // Destination vault (ceremony if it doesn't exist yet).
        let dest_vault = NoteVault::new(&folder, &dest);
        if !dest_vault.exists().await? {
            tprintln!(ctx, "'{dest}' has no note vault yet — creating one (its own 24-word recovery phrase):");
            let words = dest_vault.create(&dest_secret).await?;
            tprintln!(ctx, "");
            tprintln!(ctx, "{}", style(&words).cyan());
            tprintln!(ctx, "");
            ctx.term().ask(false, "Write these 24 words down for the destination wallet, then press <enter>: ").await?;
        }

        // The move itself: dest write, verify, source delete — per note, so an
        // interruption leaves at most one duplicate, cleaned by re-running.
        let mut stream = store.iter().await?;
        let mut serials = Vec::new();
        while let Some(info) = stream.try_next().await? {
            if info.status == NoteStatus::Active {
                serials.push(info.sn);
            }
        }
        if serials.is_empty() {
            tprintln!(ctx, "no active notes to move\r\n");
            return Ok(());
        }
        let mut moved = 0usize;
        let mut moved_petals = 0u64;
        for sn in &serials {
            let Some(entry) = store.load_key(&wallet_secret, sn).await? else { continue };
            dest_vault.store(&dest_secret, NoteKeyEntry::new(entry.sn, entry.sk, entry.d, NoteProvenance::Hot)).await?;
            if dest_vault.load_info(sn).await?.is_none() {
                tprintln!(ctx, "  {} - destination write could not be verified, keeping it here", sn);
                continue;
            }
            store.remove(&wallet_secret, sn).await?;
            moved += 1;
            moved_petals += DENOMINATION_PETALS[entry.d as usize];
            if moved % 25 == 0 {
                tprintln!(ctx, "  moved {moved}/{} notes...", serials.len());
            }
        }
        tprintln!(ctx, "");
        tprintln!(ctx, "moved {} note(s) ({} MAGLD) into '{dest}' — no chain transaction involved.", moved, sompi_to_kaspa_string(moved_petals));
        tprintln!(ctx, "They no longer exist in this wallet. Open '{dest}' to use them (it holds them as imported Hot keys).\r\n");
        Ok(())
    }

    async fn redeem(&self, ctx: &Arc<KaspaCli>, mut argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note redeem <serial> [<serial> ...]' or 'note redeem amount <amount>'\r\n");
            return Ok(());
        }

        let account = ctx.wallet().account()?;

        let selection = if argv[0] == "amount" {
            if argv.len() != 2 {
                tprintln!(ctx, "usage: 'note redeem amount <amount>'\r\n");
                return Ok(());
            }
            let amount_petals = try_parse_required_nonzero_kaspa_as_sompi_u64(argv.get(1))?;
            RedeemSelection::Amount(amount_petals)
        } else {
            let mut serials = Vec::with_capacity(argv.len());
            for raw in argv.drain(..) {
                let sn = raw.parse::<Hash>().map_err(|_| Error::Custom(format!("'{raw}' is not a valid note serial (32-byte hex)")))?;
                serials.push(sn);
            }
            RedeemSelection::Serials(serials)
        };

        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        let result = account.redeem(wallet_secret, selection).await?;

        tprintln!(
            ctx,
            "redeemed {} note(s) worth {} MAGLD (fee {} MAGLD); transparent balance +{} MAGLD",
            result.serials.len(),
            sompi_to_kaspa_string(result.redeemed_value_petals),
            sompi_to_kaspa_string(result.fee_petals),
            sompi_to_kaspa_string(result.redeemed_value_petals.saturating_sub(result.fee_petals)),
        );
        tprintln!(ctx, "tx: {}\r\n", result.transaction_id);

        Ok(())
    }

    async fn balance(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        let note_key_store = ctx.wallet().store().as_note_key_store()?;
        let mut stream = note_key_store.iter().await?;
        let mut counts = [0u64; DENOMINATION_PETALS.len()];
        let mut total = 0u64;
        while let Some(info) = stream.try_next().await? {
            if info.status == NoteStatus::Active {
                counts[info.d as usize] += 1;
                total += DENOMINATION_PETALS[info.d as usize];
            }
        }

        tprintln!(ctx, "note balance: {} MAGLD", sompi_to_kaspa_string(total));
        for (index, count) in counts.iter().enumerate() {
            if *count > 0 {
                tprintln!(ctx, "  {} x {} MAGLD", count, sompi_to_kaspa_string(DENOMINATION_PETALS[index]));
            }
        }
        tprintln!(ctx, "");

        Ok(())
    }

    /// `note unknown` — notes a synced node has repeatedly said it has not got.
    ///
    /// They stopped being counted after three separate checks against a node
    /// that reported itself caught up. This wallet cannot say which of two
    /// things happened, and does not pretend to: either the transaction that
    /// would have created the note never landed — in which case the money
    /// never left the ledger and nothing was lost — or something holding the
    /// same key spent it.
    async fn unknown(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        let store = ctx.wallet().store().as_note_key_store()?;
        let mut unknown: Vec<Arc<NoteKeyInfo>> = Vec::new();
        let mut stream = store.iter().await?;
        while let Some(info) = stream.try_next().await? {
            if info.status == NoteStatus::Unknown {
                unknown.push(info);
            }
        }

        tprintln!(ctx, "");
        if unknown.is_empty() {
            tprintln!(ctx, "Nothing unaccounted for — every note you hold is on chain.");
            tprintln!(ctx, "");
            return Ok(());
        }

        let total: u64 = unknown.iter().map(|i| DENOMINATION_PETALS[i.d as usize]).sum();
        tprintln!(ctx, "{} note(s), {} MAGLD, not on chain:", unknown.len(), sompi_to_kaspa_string(total));
        tprintln!(ctx, "");
        for info in unknown.iter().take(30) {
            tprintln!(ctx, "  {} MAGLD   {}", sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]), info.sn);
        }
        if unknown.len() > 30 {
            tprintln!(ctx, "  ... and {} more", unknown.len() - 30);
        }
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "A synced node has said three separate times that it does not have these. \
            That usually means the payment which would have created them never landed, in \
            which case the money never left your ledger balance and nothing is missing — \
            check 'balance' against what you expect. "
        );
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "The alternative is that something else holding the same keys spent them. \
            Telling the two apart needs a node that keeps history rather than only the \
            current pool, which is not something this wallet can ask for yet. "
        );
        tprintln!(ctx, "");
        Ok(())
    }

    /// `note verify` — check the vault against the pool.
    ///
    /// `balance` reports what this wallet believes it holds. Belief and fact
    /// diverge when a transaction is submitted, its notes recorded locally, and
    /// the transaction then fails to land: the note stays in the vault and
    /// exists nowhere else. This is the command that tells the difference.
    async fn verify(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if !ctx.wallet().is_connected() {
            tprintln!(ctx, "Connect to a node first — this checks your notes against the network.");
            return Ok(());
        }
        let account = ctx.wallet().account()?;
        tprintln!(ctx, "");
        tprintln!(ctx, "Checking your notes against the pool...");
        let (present, phantom) = notepool::verify_held_notes(account).await?;
        let value = |notes: &[Arc<NoteKeyInfo>]| -> u64 { notes.iter().map(|i| DENOMINATION_PETALS[i.d as usize]).sum() };

        tprintln!(ctx, "");
        tprintln!(ctx, "confirmed on chain:  {} MAGLD in {} note(s)", sompi_to_kaspa_string(value(&present)), present.len());
        if phantom.is_empty() {
            tprintln!(ctx, "");
            tprintln!(ctx, "Every note you hold exists in the pool. Your balance is real.");
            tprintln!(ctx, "");
            return Ok(());
        }

        let lost = value(&phantom);
        tprintln!(ctx, "not in the pool:     {} MAGLD in {} note(s)", sompi_to_kaspa_string(lost), phantom.len());
        tprintln!(ctx, "");
        tprintln!(ctx, "These notes are in your vault but not on chain. That happens when a");
        tprintln!(ctx, "transaction was submitted, its notes recorded here, and the transaction");
        tprintln!(ctx, "then failed to land. They are not spendable and never will be.");
        tprintln!(ctx, "");
        let mut phantom = phantom;
        phantom.sort_by(|a, b| b.d.cmp(&a.d));
        for info in phantom.iter().take(20) {
            tprintln!(ctx, "  {} - {} MAGLD", info.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]));
        }
        if phantom.len() > 20 {
            tprintln!(ctx, "  ... and {} more", phantom.len() - 20);
        }
        tprintln!(ctx, "");

        // Clearing is opt-in. A note missing because the node is mid-sync, or
        // answering from a pruned view, is not a lost note — and writing off
        // real money on a bad answer is worse than leaving a wrong number up.
        if argv.first().map(|s| s.as_str()) != Some("clear") {
            tprintln!(ctx, "'note verify clear' writes them off, once you are sure the node is fully synced.");
            tprintln!(ctx, "");
            return Ok(());
        }
        let answer = ctx.term().ask(false, &format!("Write off {} MAGLD as unrecoverable? [y/N]: ", sompi_to_kaspa_string(lost)))
            .await?
            .trim()
            .to_lowercase();
        if !answer.starts_with('y') {
            tprintln!(ctx, "Left alone.");
            return Ok(());
        }
        let store = ctx.wallet().store().as_note_key_store()?;
        for info in &phantom {
            store.mark_status(&info.sn, NoteStatus::Superseded).await?;
        }
        tprintln!(ctx, "Wrote off {} note(s). They are in 'note history' now.", phantom.len());
        Ok(())
    }

    /// `note mirror` — the notes that are also on your phone.
    ///
    /// A mirrored note stays in this vault, key and all: that is what makes it
    /// a mirror rather than a move, and what lets `note mirror revoke` kill the
    /// copies on a lost device. What changes is that nothing here will spend
    /// it — every spend, fee-source and merge selector filters on `Active`, so
    /// marking a note `Mirrored` takes it out of all of them at once.
    async fn mirror(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        let store = ctx.wallet().store().as_note_key_store()?;
        let mut mirrored: Vec<Arc<NoteKeyInfo>> = Vec::new();
        let mut spendable: Vec<Arc<NoteKeyInfo>> = Vec::new();
        let mut stream = store.iter().await?;
        while let Some(info) = stream.try_next().await? {
            match info.status {
                NoteStatus::Mirrored => mirrored.push(info),
                NoteStatus::Active => spendable.push(info),
                _ => {}
            }
        }
        let total = |notes: &[Arc<NoteKeyInfo>]| -> u64 { notes.iter().map(|i| DENOMINATION_PETALS[i.d as usize]).sum() };

        let arg = argv.first().map(|s| s.as_str());
        match arg {
            None | Some("list") => {
                if mirrored.is_empty() {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Nothing is on your phone.");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "  'note mirror <amount>'   put that much on the phone");
                    tprintln!(ctx, "  'note mirror export'     produce the encrypted block for your phone");
                    tprintln!(ctx, "  'note mirror return'     take it all back");
                    tprintln!(ctx, "  'note mirror revoke'     kill the copies on a lost phone");
                    tprintln!(ctx, "");
                    tprintln!(
                        ctx,
                        "{}",
                        style("Notes on your phone stay here too — this wallet keeps the key, which is what lets you revoke.").dim()
                    );
                    tprintln!(ctx, "");
                    return Ok(());
                }
                mirrored.sort_by(|a, b| b.d.cmp(&a.d));
                tprintln!(ctx, "");
                tprintln!(ctx, "On your phone: {} MAGLD", sompi_to_kaspa_string(total(&mirrored)));
                for info in &mirrored {
                    tprintln!(ctx, "  {} - {} MAGLD", info.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]));
                }
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("This wallet will not spend or merge these. 'note mirror revoke' if the phone is lost.").dim());
                tprintln!(ctx, "");
            }
            Some("return") => {
                if mirrored.is_empty() {
                    tprintln!(ctx, "Nothing is on your phone.");
                    return Ok(());
                }
                let amount = total(&mirrored);
                let count = mirrored.len();
                for info in &mirrored {
                    store.mark_status(&info.sn, NoteStatus::Active).await?;
                }
                tprintln!(ctx, "Took back {count} note(s), {} MAGLD.", sompi_to_kaspa_string(amount));
                tprintln!(ctx, "");
                tprintln!(
                    ctx,
                    "{}",
                    style("Do this only when the phone no longer holds them — a copy still on the phone can still be spent there.").dim()
                );
                tprintln!(ctx, "Use 'note mirror revoke' instead if you are not sure.");
            }
            Some("export") => {
                if mirrored.is_empty() {
                    tprintln!(ctx, "Nothing is on your phone. 'note mirror <amount>' first.");
                    return Ok(());
                }
                tprintln!(ctx, "");
                tpara!(
                    ctx,
                    "This produces the encrypted block your phone reads. Choose a passphrase for it — \
                    a DIFFERENT one from your wallet password. It never leaves this machine, and \
                    whoever stores the block cannot read it without the passphrase. \
                    ",
                );
                tprintln!(ctx, "");
                tpara!(
                    ctx,
                    "Forgetting it costs nothing: these notes are still here. That is the point of \
                    mirroring rather than moving — the copy is disposable. \
                    ",
                );
                tprintln!(ctx, "");
                let pass = ctx.term().ask(true, "Passphrase for the phone copy: ").await?.trim().to_string();
                if pass.is_empty() {
                    tprintln!(ctx, "No passphrase — nothing exported.");
                    return Ok(());
                }
                let again = ctx.term().ask(true, "Again: ").await?.trim().to_string();
                if pass != again {
                    tprintln!(ctx, "Those did not match — nothing exported.");
                    return Ok(());
                }

                let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                let mut entries = Vec::with_capacity(mirrored.len());
                for info in &mirrored {
                    if let Some(entry) = store.load_key(&wallet_secret, &info.sn).await? {
                        entries.push(entry);
                    }
                }
                let pages = notepool::mirror_export_pages(&entries, &Secret::from(pass.as_bytes().to_vec()))?;
                tprintln!(ctx, "");
                tprintln!(ctx, "{} MAGLD in {} note(s), as {} block(s):", sompi_to_kaspa_string(total(&mirrored)), entries.len(), pages.len());
                for (i, page) in pages.iter().enumerate() {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "{}", style(format!("--- block {} of {} ---", i + 1, pages.len())).dim());
                    ctx.term().writeln(page.clone());
                }
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("Every block is needed — one missing means the notes in it are unreadable.").dim());
                tprintln!(ctx, "");
            }
            Some("revoke") => {
                if mirrored.is_empty() {
                    tprintln!(ctx, "Nothing is on your phone.");
                    return Ok(());
                }
                let amount = total(&mirrored);
                tprintln!(ctx, "");
                tprintln!(ctx, "This rotates {} MAGLD onto fresh keys.", sompi_to_kaspa_string(amount));
                tprintln!(ctx, "Every copy on the phone dies the moment it lands — including any a thief has.");
                tprintln!(ctx, "The money comes back here.");
                tprintln!(ctx, "");
                let answer = ctx.term().ask(false, "Revoke? [y/N]: ").await?.trim().to_lowercase();
                if !answer.starts_with('y') {
                    tprintln!(ctx, "Left alone.");
                    return Ok(());
                }
                let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                let account = ctx.wallet().account()?;
                let serials: Vec<Hash> = mirrored.iter().map(|i| i.sn).collect();
                match notepool::rotate_notes(account, wallet_secret, serials).await {
                    Ok(result) => {
                        tprintln!(ctx, "");
                        tprintln!(ctx, "Revoked. {} MAGLD is back on fresh keys here.", sompi_to_kaspa_string(amount));
                        tprintln!(ctx, "{} note(s), transaction {}", result.own_notes.len(), result.transaction_id);
                    }
                    Err(err) => {
                        tprintln!(ctx, "Could not revoke: {err}");
                        tprintln!(ctx, "Nothing changed — the notes are still marked as being on the phone.");
                    }
                }
            }
            Some(amount) => {
                let target = try_parse_required_nonzero_kaspa_as_sompi_u64(Some(&amount.to_string()))?;
                // Largest first, never going over: mirroring more than asked
                // would put more at risk than the user chose to carry.
                spendable.sort_by(|a, b| b.d.cmp(&a.d));
                let mut chosen = Vec::new();
                let mut sum = 0u64;
                for info in &spendable {
                    let value = DENOMINATION_PETALS[info.d as usize];
                    if sum + value <= target {
                        sum += value;
                        chosen.push(info.clone());
                    }
                }
                if chosen.is_empty() {
                    tprintln!(ctx, "");
                    if spendable.is_empty() {
                        tprintln!(ctx, "You hold no notes to put on the phone.");
                    } else {
                        let smallest = spendable.iter().map(|i| DENOMINATION_PETALS[i.d as usize]).min().unwrap_or(0);
                        tprintln!(ctx, "No note here is small enough to make up {} MAGLD.", sompi_to_kaspa_string(target));
                        tprintln!(ctx, "Your smallest is {} MAGLD — mint or split one first.", sompi_to_kaspa_string(smallest));
                    }
                    tprintln!(ctx, "");
                    return Ok(());
                }
                for info in &chosen {
                    store.mark_status(&info.sn, NoteStatus::Mirrored).await?;
                }
                tprintln!(ctx, "");
                tprintln!(ctx, "On your phone: {} MAGLD in {} note(s).", sompi_to_kaspa_string(sum), chosen.len());
                if sum < target {
                    tprintln!(
                        ctx,
                        "{}",
                        style(format!(
                            "(you asked for {} — notes come in fixed sizes, so this is the closest without going over)",
                            sompi_to_kaspa_string(target)
                        ))
                        .dim()
                    );
                }
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("This wallet keeps the keys and will not spend these. If the phone is lost, 'note mirror revoke'.").dim());
                tprintln!(ctx, "");
            }
        }
        Ok(())
    }

    /// `note list` — what you hold. Superseded notes are history, not
    /// holdings, and on an active wallet they pile up quickly; they live in
    /// `note history` instead.
    async fn list(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        let note_key_store = ctx.wallet().store().as_note_key_store()?;
        let mut stream = note_key_store.iter().await?;
        let mut notes = Vec::new();
        let mut retired = 0usize;
        while let Some(info) = stream.try_next().await? {
            match info.status {
                NoteStatus::Superseded => retired += 1,
                _ => notes.push(info),
            }
        }
        if notes.is_empty() {
            tprintln!(ctx, "no notes held\r\n");
            return Ok(());
        }
        notes.sort_by(|a, b| b.d.cmp(&a.d).then(a.sn.cmp(&b.sn)));
        let mut handed_over_header = false;
        for info in notes.iter().filter(|i| i.status == NoteStatus::Active) {
            tprintln!(
                ctx,
                "  {} - {} MAGLD - {:?}",
                info.sn,
                sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]),
                info.provenance
            );
        }
        let mut phone_header = false;
        for info in notes.iter().filter(|i| i.status == NoteStatus::Mirrored) {
            if !phone_header {
                tprintln!(ctx, "{}", style("on your phone:").dim());
                phone_header = true;
            }
            tprintln!(ctx, "  {} - {} MAGLD", info.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]));
        }
        for info in notes.iter().filter(|i| i.status == NoteStatus::HandedOver) {
            if !handed_over_header {
                tprintln!(ctx, "{}", style("handed over (awaiting the receiver's rotation):").dim());
                handed_over_header = true;
            }
            tprintln!(ctx, "  {} - {} MAGLD", info.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]));
        }
        if retired > 0 {
            tprintln!(ctx, "{}", style(format!("({retired} spent note(s) in 'note history')")).dim());
        }
        tprintln!(ctx, "");

        Ok(())
    }

    /// `note history` — notes this wallet once held and has since spent.
    /// The vault keeps them as tombstones with the time they were last
    /// rotated, which is the closest thing to a note-spend record: the pool
    /// records that a serial was retired, never who retired it.
    async fn history(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        let note_key_store = ctx.wallet().store().as_note_key_store()?;
        let mut stream = note_key_store.iter().await?;
        let mut retired = Vec::new();
        while let Some(info) = stream.try_next().await? {
            if info.status == NoteStatus::Superseded {
                retired.push(info);
            }
        }
        if retired.is_empty() {
            tprintln!(ctx, "no spent notes yet\r\n");
            return Ok(());
        }
        retired.sort_by(|a, b| b.d.cmp(&a.d).then(a.sn.cmp(&b.sn)));
        tprintln!(ctx, "spent notes ({}):", retired.len());
        for info in &retired {
            tprintln!(ctx, "  {} - {} MAGLD", info.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]));
        }
        tprintln!(ctx, "");
        Ok(())
    }

    async fn vault(&self, ctx: &Arc<KaspaCli>, mut argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            return self.vault_help(ctx).await;
        }
        let sub = argv.remove(0);
        match sub.as_str() {
            "create" => self.vault_create(ctx).await,
            "backup" => self.vault_backup(ctx, argv).await,
            "verify" => self.vault_verify(ctx, argv).await,
            "restore" => self.vault_restore(ctx, argv).await,
            "export" => self.vault_export(ctx, argv).await,
            "import" => self.vault_import(ctx, argv).await,
            v => {
                tprintln!(ctx, "unknown vault command: '{v}'\r\n");
                self.vault_help(ctx).await
            }
        }
    }

    async fn vault_help(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        ctx.term().help(
            &[
                ("vault create", "Run the vault's 24-word creation ceremony now (auto-runs on first note otherwise)"),
                ("vault backup <dir>", "Copy the vault's files to <dir> (pair with the 24 words for a full recovery)"),
                ("vault verify", "Light-verify this wallet's active notes against the live pool (no secret needed)"),
                ("vault verify deep", "Deep-verify: decrypt and re-derive every active note's key"),
                ("vault verify backup <dir>", "Light-verify a standalone backup directory without opening/restoring it"),
                ("vault restore <dir> <24 words>", "Copy files from <dir>, recover K from the words, deep-verify, offer rotation"),
                ("vault export <dir>", "Paper QR export: encrypted pages written to <dir>, password printed once"),
                ("vault import <page-file> ...", "Import notes from a paper export's decoded pages (prompts for the password)"),
            ],
            None,
        )?;
        Ok(())
    }

    async fn vault_create(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        let account = ctx.wallet().account()?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        let store = ctx.wallet().store().as_note_key_store()?;
        if store.vault_exists().await? {
            tprintln!(ctx, "a note vault already exists for this wallet\r\n");
            return Ok(());
        }
        let words = store.vault_create(&wallet_secret).await?;
        tprintln!(ctx, "WRITE THESE 24 WORDS DOWN NOW - they are shown only this once:");
        tprintln!(ctx, "{words}");
        tprintln!(ctx, "recovery needs BOTH these words AND a copy of the vault files ('note vault backup <dir>')\r\n");
        Ok(())
    }

    async fn vault_backup(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note vault backup <dir>'\r\n");
            return Ok(());
        }
        let store = ctx.wallet().store().as_note_key_store()?;
        let folder = store.vault_folder().await?;
        let target = std::path::PathBuf::from(&argv[0]);
        copy_dir_recursive(&folder, &target).map_err(|e| Error::Custom(format!("backup copy failed: {e}")))?;
        tprintln!(ctx, "copied vault files to {}\r\n", target.display());
        Ok(())
    }

    async fn vault_verify(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.first().map(String::as_str) == Some("backup") {
            if argv.len() < 2 {
                tprintln!(ctx, "usage: 'note vault verify backup <dir>'\r\n");
                return Ok(());
            }
            let vault = NoteVault::at(&argv[1]);
            let rpc = ctx.wallet().rpc_api();
            let report = light_verify_vault(&vault, &rpc).await?;
            tprintln!(
                ctx,
                "backup at {}: {} live, {} stale (already spent/rotated since this backup was made)\r\n",
                argv[1],
                report.live.len(),
                report.stale.len()
            );
            return Ok(());
        }
        if argv.first().map(String::as_str) == Some("deep") {
            let account = ctx.wallet().account()?;
            let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
            let report = deep_verify(account, wallet_secret).await?;
            tprintln!(
                ctx,
                "deep verify: {} live, {} stale, {} corrupted",
                report.live.len(),
                report.stale.len(),
                report.corrupted.len()
            );
            if !report.corrupted.is_empty() {
                tprintln!(ctx, "corrupted serial(s): {:?}", report.corrupted);
            }
            tprintln!(ctx, "");
            return Ok(());
        }
        let account = ctx.wallet().account()?;
        let report = light_verify(account).await?;
        tprintln!(ctx, "light verify: {} live, {} stale\r\n", report.live.len(), report.stale.len());
        Ok(())
    }

    async fn vault_restore(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.len() < 2 {
            tprintln!(ctx, "usage: 'note vault restore <dir> <24 recovery words>'\r\n");
            return Ok(());
        }
        let dir = argv[0].clone();
        let words = argv[1..].join(" ");
        if words.split_whitespace().count() != 24 {
            tprintln!(
                ctx,
                "expected exactly 24 recovery words, got {} - check the words and try again\r\n",
                words.split_whitespace().count()
            );
            return Ok(());
        }
        let account = ctx.wallet().account()?;
        let store = ctx.wallet().store().as_note_key_store()?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        // A vault already existing here usually means this wallet has its own K
        // (and possibly its own notes under it) - copying a backup's `vault.key`
        // over it would silently strand anything already stored under the old K.
        // But it can also mean this is exactly the SAME restore run partway
        // through: `note vault restore` copies the files and recovers K before
        // attempting any rotation, so a rotation-batch failure (a real, expected
        // possibility - see the batch-continuation note below) leaves a vault in
        // place that looks identical to a genuine pre-existing one. Distinguish
        // the two by checking whether these words unlock the vault that's already
        // there: if so, this is a safe idempotent re-run, not a clobber.
        if store.vault_exists().await? {
            if !store.vault_words_match(&words, &wallet_secret).await? {
                tprintln!(
                    ctx,
                    "this wallet already has a different note vault - restoring here would overwrite its vault.key and \
                     strand any notes already stored under it. Restore into a fresh wallet instead.\r\n"
                );
                return Ok(());
            }
            tprintln!(ctx, "a vault from this same restore already exists here (recognized by these words) - resuming...");
        }

        let folder = store.vault_folder().await?;
        copy_dir_recursive(Path::new(&dir), &folder).map_err(|e| Error::Custom(format!("restore copy failed: {e}")))?;
        store.vault_restore_from_words(&words, &wallet_secret).await?;

        tprintln!(ctx, "vault files copied in and key recovered from words - deep-verifying...");
        let report = deep_verify(account.clone(), wallet_secret.clone()).await?;
        tprintln!(
            ctx,
            "recovered {} live note(s); {} stale; {} corrupted",
            report.live.len(),
            report.stale.len(),
            report.corrupted.len()
        );

        // deep_verify is a read-only diagnostic - it doesn't touch local status
        // itself. A serial it found stale (already spent elsewhere before this
        // backup was made, or since) must be reconciled to Superseded here, or
        // rotate_notes's fee-source selection will keep proposing it as a spare
        // and repeatedly failing every batch that draws it, since locally it
        // still looks Active.
        for sn in &report.stale {
            store.mark_status(sn, NoteStatus::Superseded).await?;
        }

        if report.live.is_empty() {
            tprintln!(ctx, "");
            return Ok(());
        }

        let batches = plan_restore_rotation(report.live.clone());
        tprintln!(
            ctx,
            "restore-time rotation (default): {} note(s) in {} batch(es) - rotating invalidates every OLD backup copy of \
             these notes, including any stolen one",
            report.live.len(),
            batches.len()
        );
        // A batch's own serials may already be gone by the time its turn comes up:
        // an earlier batch's fee-stamp (P5.2's mechanism) freely draws on any other
        // currently-Active note as its fee source, which rotates that note's value
        // too (as the fee source's change) — a real, correct side effect ("rotation
        // doubles as backup revocation", DECISIONS.md), not an error. Re-check
        // liveness immediately before each batch and skip anything already handled.
        for (i, batch) in batches.iter().enumerate() {
            let mut still_active = Vec::with_capacity(batch.len());
            for sn in batch {
                if let Some(info) = store.load_info(sn).await?
                    && info.status == NoteStatus::Active
                {
                    still_active.push(*sn);
                }
            }
            if still_active.is_empty() {
                tprintln!(ctx, "  batch {}/{}: already rotated as a side effect of an earlier batch's fee stamp - skipped", i + 1, batches.len());
                continue;
            }
            // One batch can genuinely conflict without the others being at fault —
            // e.g. a note that's also still held (and mid-spend) in whatever wallet
            // this backup was copied from, a real instance of POOL-SPEC.md's
            // same-key-in-two-wallets hazard. Report it and keep going: the other
            // batches' notes aren't affected and still deserve to be rotated.
            match account.clone().rotate_notes(wallet_secret.clone(), still_active).await {
                Ok(result) => tprintln!(
                    ctx,
                    "  batch {}/{}: {} note(s), tx {} (fee {} MAGLD)",
                    i + 1,
                    batches.len(),
                    result.own_notes.len(),
                    result.transaction_id,
                    sompi_to_kaspa_string(result.fee_petals)
                ),
                Err(err) => tprintln!(ctx, "  batch {}/{}: failed - {err} (other batches still attempted)", i + 1, batches.len()),
            }
        }
        tprintln!(ctx, "rotation complete - make a fresh backup now ('note vault backup <dir>'); every old copy is now invalid\r\n");
        Ok(())
    }

    async fn vault_export(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note vault export <dir>'\r\n");
            return Ok(());
        }
        let dir = std::path::PathBuf::from(&argv[0]);
        std::fs::create_dir_all(&dir).map_err(|e| Error::Custom(format!("could not create {}: {e}", dir.display())))?;

        let account = ctx.wallet().account()?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        let entries = export_active_entries(account, wallet_secret).await?;
        if entries.is_empty() {
            tprintln!(ctx, "no active notes to export\r\n");
            return Ok(());
        }

        let mnemonic = kaspa_bip32::Mnemonic::random(kaspa_bip32::WordCount::Words12, kaspa_bip32::Language::English)
            .map_err(|e| Error::Custom(format!("failed to generate paper export password: {e}")))?;
        let password = Secret::from(mnemonic.phrase_string().as_str());
        let pages = paper_export_encode(&entries, &password)?;

        for (i, page) in pages.iter().enumerate() {
            let path = dir.join(format!("page-{i}.txt"));
            let hex_text = page.to_hex();
            std::fs::write(&path, &hex_text).map_err(|e| Error::Custom(format!("could not write {}: {e}", path.display())))?;
            if let Some(qr) = qr_string(&hex_text) {
                tprintln!(ctx, "page {}/{}:", i + 1, pages.len());
                tprintln!(ctx, "{}", qr);
            }
        }
        tprintln!(ctx, "wrote {} page(s) to {}", pages.len(), dir.display());
        tprintln!(ctx, "paper export password (write this on the printed pages): {}", mnemonic.phrase_string());
        tprintln!(ctx, "");
        Ok(())
    }

    async fn vault_import(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note vault import <page-file> [<page-file> ...]'\r\n");
            return Ok(());
        }
        let mut pages: Vec<Vec<u8>> = Vec::with_capacity(argv.len());
        for path in &argv {
            let hex_text = std::fs::read_to_string(path).map_err(|e| Error::Custom(format!("could not read {path}: {e}")))?;
            let bytes = Vec::<u8>::from_hex(hex_text.trim()).map_err(|e| Error::Custom(format!("{path}: invalid hex: {e}")))?;
            pages.push(bytes);
        }

        let mut headers = Vec::with_capacity(pages.len());
        for page in &pages {
            headers.push(paper_export_peek_header(page)?);
        }
        let missing = paper_export_missing_pages(&headers);
        if !missing.is_empty() {
            tprintln!(ctx, "missing page(s): {:?} - provide every page before importing\r\n", missing);
            return Ok(());
        }

        let password = Secret::new(ctx.term().ask(true, "Enter paper backup password: ").await?.trim().as_bytes().to_vec());
        let account = ctx.wallet().account()?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        let mut imported = 0usize;
        for page in &pages {
            let (_, entries) = paper_export_decode_page(page, &password)
                .map_err(|_| Error::Custom("could not decrypt this paper backup - check the password and try again".to_string()))?;
            for bearer in entries {
                account.clone().bearer_import(wallet_secret.clone(), bearer).await?;
                imported += 1;
            }
        }
        tprintln!(ctx, "imported and rotated {imported} note(s) from the paper backup\r\n");
        Ok(())
    }

    async fn display_help(self: Arc<Self>, ctx: Arc<KaspaCli>, _argv: Vec<String>) -> Result<()> {
        ctx.term().help(
            &[
                ("mint <amount> | all", "Mint notes from the transparent balance ('all' mints everything, fee-aware)"),
                ("rotate all | <serial> ...", "Rotate notes to fresh keys on-chain (revokes old backups/stolen copies)"),
                ("move", "Move ALL active notes into another wallet's vault - no chain transaction"),
                ("redeem <serial> [<serial> ...]", "Redeem specific notes by serial"),
                ("redeem amount <amount>", "Redeem enough owned notes to cover at least <amount> MAGLD"),
                ("request [<amount>]", "Create a payment request (QR + text), then watch for the payment"),
                ("pay <request-text> [<amount>]", "Pay a payment request from held notes"),
                ("import <bearer-text>", "Import a bearer note and immediately rotate it to fresh keys"),
                ("export <serial>", "Bearer-export a note (auto-isolates first if its key is shared)"),
                ("pos <amount>", "One POS checkout: fresh landing-pad pk, wait for payment, auto-sweep"),
                ("balance", "Show note balance by denomination"),
                ("list", "List the notes you hold"),
                ("verify [clear]", "Check your notes against the pool — proves the balance is real"),
                ("mirror [<amount>|export|return|revoke]", "Put notes on your phone, take them back, or kill a lost phone's copies"),
                ("history", "List notes this wallet has spent"),
                ("unknown", "Notes a synced node says it has not got, and what that means"),
                ("vault <cmd>", "Note vault: create/backup/verify/restore/export/import (see 'note vault')"),
            ],
            None,
        )?;

        Ok(())
    }
}
