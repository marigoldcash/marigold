use crate::imports::*;
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::account::notepool;
use kaspa_wallet_core::account::notepool::{
    BearerNote, PaymentRequest, RedeemSelection, await_payment_request, create_payment_request,
};
use kaspa_wallet_core::storage::NoteStatus;
use std::time::Duration;
use workflow_core::abortable::Abortable;

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
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'note mint <amount>'\r\n");
            return Ok(());
        }

        let account = ctx.wallet().account()?;
        let amount_petals = try_parse_required_nonzero_kaspa_as_sompi_u64(argv.first())?;
        argv.remove(0);
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        let abortable = Abortable::default();

        let result = account.mint(wallet_secret, payment_secret, amount_petals, None, &abortable).await?;

        tprintln!(ctx, "Minted {} MAGLD into {} note(s):", sompi_to_kaspa_string(amount_petals), result.notes.len());
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
            "Redeemed {} note(s) worth {} MAGLD (fee {} sompi); transparent balance +{} MAGLD",
            result.serials.len(),
            sompi_to_kaspa_string(result.redeemed_value_petals),
            result.fee_sompi,
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

        tprintln!(ctx, "Note balance: {} MAGLD", sompi_to_kaspa_string(total));
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
        let mut printed = 0;
        while let Some(info) = stream.try_next().await? {
            tprintln!(
                ctx,
                "{} - {} MAGLD - {:?} - {:?}",
                info.sn,
                sompi_to_kaspa_string(DENOMINATION_PETALS[info.d as usize]),
                info.provenance,
                info.status,
            );
            printed += 1;
        }
        if printed == 0 {
            tprintln!(ctx, "no notes held\r\n");
        } else {
            tprintln!(ctx, "");
        }

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
            ],
            None,
        )?;

        Ok(())
    }
}
