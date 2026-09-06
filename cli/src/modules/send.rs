use crate::imports::*;

#[derive(Default, Handler)]
#[help("Send a Marigold transaction to a public address")]
pub struct Send;

impl Send {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        // address, amount, priority fee
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        let account = ctx.wallet().account()?;

        if argv.len() < 2 {
            tprintln!(ctx, "usage: send <address> <amount> <priority fee>");
            return Ok(());
        }

        let address = Address::try_from(argv.first().unwrap().as_str())?;
        let amount_sompi = try_parse_required_nonzero_kaspa_as_sompi_u64(argv.get(1))?;
        // TODO fee_rate
        let fee_rate = None;
        let priority_fee_sompi = try_parse_optional_kaspa_as_sompi_i64(argv.get(2))?.unwrap_or(0);
        let outputs = PaymentOutputs::from((address.clone(), amount_sompi));
        let abortable = Abortable::default();
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        // Ledger short but notes cover it? Redeem straight to the recipient —
        // one transaction destroys the notes and pays the address, with change
        // returning here. This is the exchange-deposit path: the user thinks
        // "send 500", not "redeem, wait, then send".
        let mature = account.balance().map(|b| b.mature).unwrap_or(0);
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
            if note_total >= amount_sompi {
                tprintln!(
                    ctx,
                    "Ledger balance is {} MAGLD — paying from notes instead (one transaction: notes are redeemed straight to {address}).",
                    sompi_to_kaspa_string(mature)
                );
                // Select enough notes to cover the payment plus room for the fee.
                let selection = kaspa_wallet_core::account::notepool::RedeemSelection::Amount(
                    amount_sompi.saturating_add(kaspa_consensus_core::notepool::DENOMINATION_PETALS[0]),
                );
                let result = kaspa_wallet_core::account::notepool::redeem_to(
                    account.clone(),
                    wallet_secret,
                    selection,
                    Some((address.clone(), amount_sompi)),
                )
                .await?;
                tprintln!(
                    ctx,
                    "\nSent {} MAGLD to {address} from {} note(s) (fee {} MAGLD); tx: {}\n",
                    sompi_to_kaspa_string(amount_sompi),
                    result.serials.len(),
                    sompi_to_kaspa_string(result.fee_sompi),
                    result.transaction_id
                );
                return Ok(());
            }
        }

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
        tprintln!(ctx, "\nSending {} MAGLD to {address}, tx ids:", sompi_to_kaspa_string(amount_sompi));
        // tprintln!(ctx, "{}\n", ids.into_iter().map(|a| a.to_string()).collect::<Vec<_>>().join("\n"));

        Ok(())
    }
}
