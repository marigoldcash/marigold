use crate::imports::*;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::storage::NoteStatus;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;

#[derive(Default, Handler)]
#[help("Show everything you hold: ledger balance and notes, in one view")]
pub struct Balance;

impl Balance {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let account = ctx.wallet().account()?;

        tprintln!(ctx, "");
        let note_key_store = ctx.wallet().store().as_note_key_store()?;
        let mut stream = note_key_store.iter().await?;
        let mut counts = [0u64; DENOMINATION_PETALS.len()];
        let mut total = 0u64;
        let mut mirrored = 0u64;
        let mut mirrored_count = 0usize;
        while let Some(info) = stream.try_next().await? {
            match info.status {
                NoteStatus::Active => {
                    counts[info.d as usize] += 1;
                    total += DENOMINATION_PETALS[info.d as usize];
                }
                // Still yours — this wallet holds the key — but carried on the
                // phone and untouchable here. Listing it separately rather than
                // omitting it: money that silently disappears from the balance
                // when you fund a phone reads as money lost.
                NoteStatus::Mirrored => {
                    mirrored += DENOMINATION_PETALS[info.d as usize];
                    mirrored_count += 1;
                }
                _ => {}
            }
        }
        tprintln!(ctx, "notes:  {} MAGLD", sompi_to_kaspa_string(total));
        for (index, count) in counts.iter().enumerate().rev() {
            if *count > 0 {
                tprintln!(ctx, "  {} x {} MAGLD", count, sompi_to_kaspa_string(DENOMINATION_PETALS[index]));
            }
        }

        if mirrored > 0 {
            tprintln!(ctx, "");
            tprintln!(
                ctx,
                "on your phone:  {} MAGLD  ({} note{})",
                sompi_to_kaspa_string(mirrored),
                mirrored_count,
                if mirrored_count == 1 { "" } else { "s" }
            );
        }

        // Every balance is checked against the pool, and says so only when
        // there is something to say. A figure the wallet merely believes is
        // worth no more than the belief — but a line confirming the obvious on
        // every call trains people to stop reading it, so silence means good.
        if total > 0 || mirrored > 0 {
            if ctx.wallet().is_connected() {
                match kaspa_wallet_core::account::notepool::verify_held_notes(account.clone()).await {
                    Ok((_, phantom)) if !phantom.is_empty() => {
                        let lost: u64 = phantom.iter().map(|i| DENOMINATION_PETALS[i.d as usize]).sum();
                        tprintln!(ctx, "");
                        tprintln!(
                            ctx,
                            "{}",
                            style(format!(
                                "{} MAGLD of the above is NOT on chain ({} note(s)) — counted here but not spendable.",
                                sompi_to_kaspa_string(lost),
                                phantom.len()
                            ))
                            .red()
                        );
                        tprintln!(ctx, "{}", style("'note verify' shows which. Check the node is fully synced before writing them off.").dim());
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tprintln!(ctx, "");
                        tprintln!(ctx, "{}", style(format!("(could not check these against the chain: {err})")).dim());
                    }
                }
            } else {
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("Not verified on chain — this is what your vault says it holds, unchecked.").dim());
                tprintln!(ctx, "{}", style("Connect a node and run 'balance' again to confirm the notes really exist.").dim());
            }
        }

        // The ledger comes last and only when it holds something — for anyone
        // but an exchange it should be empty most of the time.
        let network_id = ctx.wallet().network_id()?;
        let network_type = NetworkType::from(network_id);
        if let Some(balance) = account.balance() {
            if balance.mature > 0 || balance.pending > 0 {
                let strings = BalanceStrings::from((Some(&balance), &network_type, None));
                tprintln!(ctx, "");
                tprintln!(
                    ctx,
                    "ledger: {strings}   ({} piece{})",
                    balance.mature_utxo_count,
                    if balance.mature_utxo_count == 1 { "" } else { "s" }
                );
            }
        }

        if total == 0 {
            if let Some(balance) = account.balance() {
                if balance.mature > 0 {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Tip: turn ledger balance into bearer notes with 'note mint <amount>' (or 'note mint all')");
                }
            }
        }
        tprintln!(ctx, "");

        Ok(())
    }
}
