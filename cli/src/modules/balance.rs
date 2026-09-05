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
        let network_id = ctx.wallet().network_id()?;
        let network_type = NetworkType::from(network_id);
        match account.balance() {
            Some(balance) => {
                let strings = BalanceStrings::from((Some(&balance), &network_type, None));
                tprintln!(ctx, "ledger balance: {strings}   ({} UTXOs, {} pending)", balance.mature_utxo_count, balance.pending_utxo_count);
            }
            None => tprintln!(ctx, "ledger balance: n/a (connect and open a wallet first)"),
        }

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
        tprintln!(ctx, "note balance:   {} MAGLD", sompi_to_kaspa_string(total));
        for (index, count) in counts.iter().enumerate().rev() {
            if *count > 0 {
                tprintln!(ctx, "  {} x {} MAGLD", count, sompi_to_kaspa_string(DENOMINATION_PETALS[index]));
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
