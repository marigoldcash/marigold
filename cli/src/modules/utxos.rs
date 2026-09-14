use crate::imports::*;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;

#[derive(Default, Handler)]
#[help("List the individual UTXOs held by the selected account ('utxos all' to show every one)")]
pub struct Utxos;

impl Utxos {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let ticker = ctx.ticker();
        let account = ctx.wallet().account()?;
        let show_all = argv.first().map(|s| s.to_lowercase()).as_deref() == Some("all");
        // A mining wallet on this network can hold hundreds of thousands of
        // coinbase UTXOs — cap the default view.
        const DEFAULT_LIMIT: usize = 50;

        let (mut mature, pending, stasis) = account.utxo_context().utxo_entries_snapshot();
        mature.sort_by(|a, b| b.amount().cmp(&a.amount()));

        tprintln!(ctx, "");
        if mature.is_empty() && pending.is_empty() && stasis.is_empty() {
            tprintln!(ctx, "no UTXOs held (is the wallet connected? see 'connect')\r\n");
            return Ok(());
        }

        let total: u64 = mature.iter().map(|e| e.amount()).sum();
        tprintln!(ctx, "mature: {} UTXOs, {} {ticker} total", mature.len(), sompi_to_kaspa_string(total));
        let limit = if show_all { mature.len() } else { DEFAULT_LIMIT };
        for entry in mature.iter().take(limit) {
            let coinbase = if entry.is_coinbase() { "  (coinbase)" } else { "" };
            tprintln!(
                ctx,
                "  {} {ticker} - daa {}{}",
                sompi_to_kaspa_string(entry.amount()).pad_to_width_with_alignment(16, pad::Alignment::Right),
                entry.block_daa_score(),
                coinbase
            );
        }
        if mature.len() > limit {
            tprintln!(ctx, "  ... and {} more ('utxos all' to list every one)", mature.len() - limit);
        }
        if !pending.is_empty() {
            let pending_total: u64 = pending.iter().map(|e| e.amount()).sum();
            tprintln!(ctx, "pending: {} UTXOs, {} {ticker}", pending.len(), sompi_to_kaspa_string(pending_total));
        }
        if !stasis.is_empty() {
            tprintln!(ctx, "stasis (fresh coinbase): {} UTXOs", stasis.len());
        }
        tprintln!(ctx, "");

        Ok(())
    }
}
