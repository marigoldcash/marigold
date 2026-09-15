use crate::imports::*;

#[derive(Default, Handler)]
#[help("Sends Marigold to a public exchange")]
pub struct Exchange;

impl Exchange {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        // address, amount, priority fee
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let ticker = ctx.ticker();

        // None on a wallet that keeps notes only (FORK-PLAN P8.0b): the
        // payment then comes from notes alone, redeemed straight to the
        // address, and there is no ledger to return change to.
        let account = match ctx.wallet().account() {
            Ok(account) => Some(account),
            Err(_) if !ctx.has_ledger_account().await => None,
            Err(err) => return Err(err.into()),
        };

        if argv.len() < 2 {
            tprintln!(ctx, "usage: exchange <address> <amount> <priority fee>");
            return Ok(());
        }

        let address = Address::try_from(argv.first().unwrap().as_str())?;
        let amount_sompi = try_parse_required_nonzero_kaspa_as_sompi_u64(argv.get(1))?;
        // TODO fee_rate
        let fee_rate = None;
        let priority_fee_sompi = try_parse_optional_kaspa_as_sompi_i64(argv.get(2))?.unwrap_or(0);
        let outputs = PaymentOutputs::from((address.clone(), amount_sompi));
        let abortable = Abortable::default();
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(account.as_ref()).await?;

        // Ledger short but notes cover it? Redeem straight to the recipient —
        // one transaction destroys the notes and pays the address, with change
        // returning here. This is the exchange-deposit path: the user thinks
        // "send 500", not "redeem, wait, then send".
        let mature = account.as_ref().and_then(|account| account.balance()).map(|b| b.mature).unwrap_or(0);
        if mature < amount_sompi {
            let note_total = {
                let store = ctx.wallet().store().as_note_key_store()?;
                let mut stream = store.iter().await?;
                let mut total = 0u64;
                while let Some(info) = stream.try_next().await? {
                    if info.status == kaspa_wallet_core::storage::NoteStatus::Active {
                        total += kaspa_consensus_core::notepool::DENOMINATION_PETALS[info.d as usize];
                    }
                }
                total
            };
            // Neither side alone covers it, but together they do: one
            // transaction spends transparent coins AND consumes notes.
            if let Some(account) = &account
                && note_total < amount_sompi
                && mature + note_total > amount_sompi
            {
                tprintln!(
                    ctx,
                    "Paying {} {ticker} from both sides at once: {} {ticker} on the ledger plus notes, in one transaction.",
                    sompi_to_kaspa_string(amount_sompi),
                    sompi_to_kaspa_string(mature)
                );
                let (transaction_id, fee, utxos, notes) = kaspa_wallet_core::account::notepool::send_combined(
                    account.clone(),
                    wallet_secret,
                    payment_secret,
                    address.clone(),
                    amount_sompi,
                )
                .await?;
                ctx.record("paid", amount_sompi, fee, format!("to {address}"), transaction_id.to_string());
                tprintln!(
                    ctx,
                    "\nSent {} {ticker} to {address} from {utxos} ledger coin(s) and {notes} note(s) (fee {} {ticker}); tx: {}\n",
                    sompi_to_kaspa_string(amount_sompi),
                    sompi_to_kaspa_string(fee),
                    transaction_id
                );
                return Ok(());
            }

            if note_total >= amount_sompi {
                if account.is_some() {
                    tprintln!(
                        ctx,
                        "Ledger balance is {} {ticker} — paying from notes instead (one transaction: notes are redeemed straight to {address}).",
                        sompi_to_kaspa_string(mature)
                    );
                } else {
                    tprintln!(ctx, "Paying from notes: they are redeemed straight to {address} in one transaction.");
                }
                // Select enough notes to cover the payment plus room for the fee.
                let selection = kaspa_wallet_core::account::notepool::RedeemSelection::Amount(
                    amount_sompi.saturating_add(kaspa_consensus_core::notepool::DENOMINATION_PETALS[0]),
                );
                // Change goes back to the ledger when there is one. Without one
                // the notes must cover the amount to within a 0.01 note —
                // redeem_with says so, in those words, when they do not.
                let change = account.as_ref().map(|account| account.change_address()).transpose()?;
                let result = kaspa_wallet_core::account::notepool::redeem_with(
                    &ctx.wallet(),
                    wallet_secret,
                    selection,
                    Some((address.clone(), amount_sompi)),
                    change,
                )
                .await?;
                ctx.record("paid", amount_sompi, result.fee_petals, format!("to {address}"), result.transaction_id.to_string());
                tprintln!(
                    ctx,
                    "\nSent {} {ticker} to {address} from {} note(s) (fee {} {ticker}); tx: {}\n",
                    sompi_to_kaspa_string(amount_sompi),
                    result.serials.len(),
                    sompi_to_kaspa_string(result.fee_petals),
                    result.transaction_id
                );
                return Ok(());
            }
        }

        let Some(account) = account else {
            tprintln!(
                ctx,
                "Your notes do not cover {} {ticker}, and this wallet has no ledger to make up the difference.",
                sompi_to_kaspa_string(amount_sompi)
            );
            return Ok(());
        };

        // let ctx_ = ctx.clone();
        let (summary, _ids) = account
            .send(
                outputs.into(),
                fee_rate,
                priority_fee_sompi.into(),
                None,
                wallet_secret,
                payment_secret,
                &abortable,
                Some(Arc::new(move |_ptx| {
                    // tprintln!(ctx_, "Sending transaction: {}", ptx.id());
                })),
            )
            .await?;

        tprintln!(ctx, "Send - {summary}");
        tprintln!(ctx, "\nSending {} {ticker} to {address}, tx ids:", sompi_to_kaspa_string(amount_sompi));
        // tprintln!(ctx, "{}\n", ids.into_iter().map(|a| a.to_string()).collect::<Vec<_>>().join("\n"));

        Ok(())
    }
}
