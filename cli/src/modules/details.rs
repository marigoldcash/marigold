use crate::imports::*;

#[derive(Default, Handler)]
#[help("Displays the detailed information about the currently selected account.")]
pub struct Details;

impl Details {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let account = ctx.select_account().await?.as_derivation_capable()?;

        let derivation = account.derivation();

        // One ledger address per account (2026-09-05). Any further addresses
        // shown here are historical — derived before the single-address
        // policy, or by an imported wallet — and are still tracked so their
        // funds stay visible and spendable.
        tprintln!(ctx, "Ledger address:");
        tprintln!(ctx.term(), "{:>4}{}", "", style(account.clone().as_dyn_arc().receive_address()?.to_string()).blue());

        // Historical addresses: only those actually HOLDING coins — a wallet
        // that has sent transactions before the single-address policy has
        // change sitting on the change branch, and that must stay visible.
        // A fresh wallet has none, and says nothing.
        let dyn_account = account.clone().as_dyn_arc();
        let current = dyn_account.receive_address()?.to_string();
        let (mature, pending, stasis) = dyn_account.utxo_context().utxo_entries_snapshot();
        let mut historical: std::collections::BTreeMap<String, u64> = Default::default();
        for entry in mature.iter().chain(pending.iter()).chain(stasis.iter()) {
            if let Some(address) = entry.utxo.address.as_ref() {
                let address = address.to_string();
                if address != current {
                    *historical.entry(address).or_default() += entry.amount();
                }
            }
        }
        if !historical.is_empty() {
            tprintln!(ctx, "");
            tprintln!(ctx, "Older addresses still holding coins ({}) — spendable as normal, and consolidated by 'sweep':", historical.len());
            for (address, amount) in historical {
                tprintln!(
                    ctx.term(),
                    "{:>4}{}  {}",
                    "",
                    style(address).dim(),
                    style(kaspa_wallet_core::utils::sompi_to_kaspa_string(amount)).dim()
                );
            }
        }
        tprintln!(ctx, "");

        if let Some(xpub_keys) = account.xpub_keys()
            && account.feature().is_some()
        {
            if let Some(feature) = account.feature() {
                tprintln!(ctx.term(), "Feature: {}", style(feature).cyan());
            }
            tprintln!(ctx.term(), "Extended public keys:");
            xpub_keys.iter().for_each(|xpub| {
                tprintln!(ctx.term(), "{:>4}{}", "", style(ctx.wallet().network_format_xpub(xpub)).dim());
            });
        }

        Ok(())
    }
}
