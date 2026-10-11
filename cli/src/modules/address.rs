use crate::imports::*;

#[derive(Default, Handler)]
#[help("Show this account's ledger address (the transparent side of your wallet)")]
pub struct Address;

impl Address {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        if argv.is_empty() {
            // The one place for the ledger address ('details' and 'list'
            // showed the same line in another colour; founder, 2026-10-11).
            let account = ctx.account().await?;
            let address = account.receive_address()?.to_string();
            tprintln!(ctx, "\n{address}\n");
            // Older addresses still holding coins: a wallet that spent before
            // the one-address policy (2026-09-05) has change on the change
            // branch, and that must stay visible. A fresh wallet has none.
            let (mature, pending, stasis) = account.utxo_context().utxo_entries_snapshot();
            let mut historical: std::collections::BTreeMap<String, u64> = Default::default();
            for entry in mature.iter().chain(pending.iter()).chain(stasis.iter()) {
                if let Some(older) = entry.utxo.address.as_ref() {
                    let older = older.to_string();
                    if older != address {
                        *historical.entry(older).or_default() += entry.amount();
                    }
                }
            }
            if !historical.is_empty() {
                tprintln!(
                    ctx,
                    "Older addresses still holding coins ({}), spendable as normal and gathered by 'sweep':",
                    historical.len()
                );
                for (older, amount) in historical {
                    tprintln!(
                        ctx.term(),
                        "{:>4}{}  {}",
                        "",
                        style(older).dim(),
                        style(kaspa_wallet_core::utils::sompi_to_kaspa_string(amount)).dim()
                    );
                }
                tprintln!(ctx, "");
            }
            if ctx.advanced()
                && let Ok(derivation) = account.clone().as_derivation_capable()
                && let Some(xpub_keys) = derivation.xpub_keys()
            {
                if let Some(feature) = derivation.feature() {
                    tprintln!(ctx.term(), "Feature: {}", style(feature).cyan());
                }
                tprintln!(ctx.term(), "Extended public keys:");
                xpub_keys.iter().for_each(|xpub| {
                    tprintln!(ctx.term(), "{:>4}{}", "", style(ctx.wallet().network_format_xpub(xpub)).dim());
                });
                tprintln!(ctx, "");
            }
        } else {
            let op = argv.first().unwrap();
            match op.as_str() {
                // Marigold uses one ledger address per account (2026-09-05).
                // Explain rather than silently doing nothing — and point at
                // the thing that actually gives a fresh key per payment.
                "new" => {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "This account has one ledger address, and it doesn't change:");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "{}", style(ctx.account().await?.receive_address()?.to_string()).blue());
                    tprintln!(ctx, "");
                    tprintln!(ctx, "The ledger is the transparent side — it is not where privacy lives, so rotating");
                    tprintln!(ctx, "addresses there buys little and makes recovery a guessing game. For a fresh key");
                    tprintln!(ctx, "per payment, use notes: 'note request' issues one per invoice.");
                    tprintln!(ctx, "");
                }
                v => {
                    tprintln!(ctx, "unknown command: '{v}'\r\n");
                    return self.display_help(ctx, argv).await;
                }
            }
        }

        Ok(())
    }

    async fn display_help(self: Arc<Self>, ctx: Arc<KaspaCli>, _argv: Vec<String>) -> Result<()> {
        ctx.term().help(&[("address", "Show this account's ledger address (one per account; it never changes)")], None)?;

        Ok(())
    }
}
