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
use kaspa_wallet_core::storage::NoteStatus;
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
fn qr_string(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(code.render::<qrcode::render::unicode::Dense1x2>().build())
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
            "redeem" => self.redeem(&ctx, argv).await,
            "request" => self.request(&ctx, argv).await,
            "pay" => self.pay(&ctx, argv).await,
            "import" => self.import(&ctx, argv).await,
            "export" => self.export(&ctx, argv).await,
            "pos" => self.pos(&ctx, argv).await,
            "balance" => self.balance(&ctx).await,
            "list" => self.list(&ctx).await,
            "vault" => self.vault(&ctx, argv).await,
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
        let account = ctx.wallet().account()?;
        let bearer = BearerNote::from_text(&argv[0])?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        let result = account.bearer_import(wallet_secret, bearer).await?;
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

    async fn mint(&self, ctx: &Arc<KaspaCli>, mut argv: Vec<String>) -> Result<()> {
        let account = ctx.wallet().account()?;

        // 'note mint' with no amount offers to mint everything; 'note mint all'
        // does it without asking. "Everything" is fee-aware: the mint
        // transaction's own fee comes out of the same balance.
        let all = match argv.first().map(|s| s.to_lowercase()).as_deref() {
            None => {
                let abortable = Abortable::default();
                let max = notepool::max_mintable_petals(account.clone(), None, &abortable).await?;
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
                let max = notepool::max_mintable_petals(account.clone(), None, &abortable).await?;
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
        let abortable = Abortable::default();

        let result = account.mint(wallet_secret, payment_secret, amount_petals, None, &abortable).await?;

        tprintln!(ctx, "minted {} MAGLD into {} note(s):", sompi_to_kaspa_string(amount_petals), result.notes.len());
        for entry in &result.notes {
            tprintln!(ctx, "  {} - {}", entry.sn, sompi_to_kaspa_string(DENOMINATION_PETALS[entry.d as usize]));
        }
        tprintln!(ctx, "tx: {}\r\n", result.transaction_ids.last().expect("mint always submits at least one transaction"));

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
            sompi_to_kaspa_string(result.fee_sompi),
            sompi_to_kaspa_string(result.redeemed_value_petals.saturating_sub(result.fee_sompi)),
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

    async fn list(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        let note_key_store = ctx.wallet().store().as_note_key_store()?;
        let mut stream = note_key_store.iter().await?;
        let mut notes = Vec::new();
        while let Some(info) = stream.try_next().await? {
            notes.push(info);
        }
        if notes.is_empty() {
            tprintln!(ctx, "no notes held\r\n");
            return Ok(());
        }
        // Active first (largest denomination first — matches `note balance`'s
        // active-only view), then handed-over, then superseded history.
        let status_rank = |status: &NoteStatus| match status {
            NoteStatus::Active => 0u8,
            NoteStatus::HandedOver => 1,
            NoteStatus::Superseded => 2,
        };
        notes.sort_by(|a, b| status_rank(&a.status).cmp(&status_rank(&b.status)).then(b.d.cmp(&a.d)).then(a.sn.cmp(&b.sn)));
        let mut current: Option<u8> = None;
        for info in &notes {
            let rank = status_rank(&info.status);
            if current != Some(rank) {
                current = Some(rank);
                let header = match info.status {
                    NoteStatus::Active => "active:",
                    NoteStatus::HandedOver => "handed over (awaiting the receiver's rotation):",
                    NoteStatus::Superseded => "superseded (spent history):",
                };
                tprintln!(ctx, "{}", style(header).dim());
            }
            tprintln!(
                ctx,
                "  {} - {} MAGLD - {:?}",
                info.sn,
                sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]),
                info.provenance,
            );
        }
        tprintln!(ctx, "");

        Ok(())
    }

    /// `note vault <create|backup|verify|restore|export|import>` (FORK-PLAN P7.6).
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
                ("mint <amount>", "Mint notes worth <amount> MAGLD from the transparent balance"),
                ("redeem <serial> [<serial> ...]", "Redeem specific notes by serial"),
                ("redeem amount <amount>", "Redeem enough owned notes to cover at least <amount> MAGLD"),
                ("request [<amount>]", "Create a payment request (QR + text), then watch for the payment"),
                ("pay <request-text> [<amount>]", "Pay a payment request from held notes"),
                ("import <bearer-text>", "Import a bearer note and immediately rotate it to fresh keys"),
                ("export <serial>", "Bearer-export a note (auto-isolates first if its key is shared)"),
                ("pos <amount>", "One POS checkout: fresh landing-pad pk, wait for payment, auto-sweep"),
                ("balance", "Show note balance by denomination"),
                ("list", "List every held note (serial, denomination, provenance, status)"),
                ("vault <cmd>", "Note vault: create/backup/verify/restore/export/import (see 'note vault')"),
            ],
            None,
        )?;

        Ok(())
    }
}
