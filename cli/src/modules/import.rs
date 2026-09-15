use crate::imports::*;
use kaspa_wallet_core::account::notepool::{self, BearerNote, HANDOVER_PREFIX};

#[derive(Default, Handler)]
#[help("Import note keys written by 'export' in another wallet of yours — stored as they are, no fee")]
pub struct Import;

impl Import {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        if argv.is_empty() {
            tprintln!(ctx, "usage: 'import <code> [<code> ...]' — the codes 'export' wrote in the other wallet");
            return Ok(());
        }
        if argv.iter().any(|code| code.starts_with(HANDOVER_PREFIX)) {
            tprintln!(ctx, "That is a payment, not an export: 'receive <code>' takes it.");
            return Ok(());
        }
        let bearers = argv
            .iter()
            .map(|code| BearerNote::from_text(code))
            .collect::<std::result::Result<Vec<_>, kaspa_wallet_core::error::Error>>()?;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(None).await?;
        crate::modules::note::Note::default().ensure_vault_interactive(&ctx, &wallet_secret).await?;
        let (count, verified) = notepool::import_keys(&ctx.wallet(), wallet_secret, bearers).await?;
        tprintln!(ctx, "");
        tprintln!(ctx, "Imported {count} note key(s){}.", if verified { ", checked against the chain" } else { " — not checked, no network; 'note verify' does that once connected" });
        tprintln!(
            ctx,
            "{}",
            crate::ui::dim("These keys came from another wallet, which can still spend them until you rotate: 'note rotate all' (0.01 a group) makes them yours alone.")
        );
        tprintln!(ctx, "");
        Ok(())
    }
}
