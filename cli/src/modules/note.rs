use crate::imports::*;
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::account::notepool::RedeemSelection;
use kaspa_wallet_core::storage::NoteStatus;
use workflow_core::abortable::Abortable;

#[derive(Default, Handler)]
#[help("Mint, redeem, and list notes (FORK-PLAN P7.2)")]
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
            "balance" => self.balance(&ctx).await,
            "list" => self.list(&ctx).await,
            v => {
                tprintln!(ctx, "unknown command: '{v}'\r\n");
                self.display_help(ctx, argv).await
            }
        }
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
                ("balance", "Show note balance by denomination"),
                ("list", "List every held note (serial, denomination, provenance, status)"),
            ],
            None,
        )?;

        Ok(())
    }
}
