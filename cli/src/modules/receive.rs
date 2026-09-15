use crate::imports::*;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::account::notepool::{self, BearerNote, Handover, HANDOVER_PREFIX};
use kaspa_wallet_core::utils::sompi_to_kaspa_string;

#[derive(Default, Handler)]
#[help("Receive money: takes the code someone paid you with")]
pub struct Receive;

impl Receive {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let ticker = ctx.ticker();
        let Some(code) = argv.first() else {
            tprintln!(ctx, "usage: 'receive <code>' — the code the payer gave you");
            return Ok(());
        };
        let wallet = ctx.wallet();
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(None).await?;
        let note = crate::modules::note::Note::default();
        note.ensure_vault_interactive(&ctx, &wallet_secret).await?;

        if code.starts_with(HANDOVER_PREFIX) {
            // A payment: the notes come under one key the payer made for this
            // handover, with the stamp for making them ours.
            let handover = Handover::from_text(code)?;
            let result = notepool::receive_handover(&wallet, wallet_secret.clone(), handover).await?;
            let stamp = result.notes.iter().any(|(_, d)| *d == kaspa_consensus_core::notepool::DenominationTag::D0_01);
            let value = if stamp { result.value_petals - DENOMINATION_PETALS[0] } else { result.value_petals };
            ctx.record("received", value, 0, "code", result.rotation.transaction_id.to_string());
            tprintln!(ctx, "");
            tprintln!(ctx, "Received {} {ticker} in {} note(s).", sompi_to_kaspa_string(value), result.notes.len() - usize::from(stamp));
            tprintln!(
                ctx,
                "{}",
                crate::ui::dim(format!(
                    "Made yours alone with the payer's stamp (tx {}) — nobody else can spend it once this confirms.",
                    result.rotation.transaction_id
                ))
            );
        } else {
            // A single note's own key, handed over the older way: rotating it
            // costs one of our own stamps.
            let bearer = BearerNote::from_text(code)?;
            let result = notepool::bearer_import(&wallet, wallet_secret.clone(), bearer).await?;
            ctx.record("received", DENOMINATION_PETALS[bearer.d as usize], result.rotation.fee_petals, "note", result.rotation.transaction_id.to_string());
            tprintln!(ctx, "");
            tprintln!(ctx, "Received {} {ticker}.", sompi_to_kaspa_string(DENOMINATION_PETALS[bearer.d as usize]));
            tprintln!(
                ctx,
                "{}",
                crate::ui::dim(format!(
                    "Made yours alone (tx {}, {} {ticker} of your own paid for that).",
                    result.rotation.transaction_id,
                    sompi_to_kaspa_string(result.rotation.fee_petals)
                ))
            );
        }

        // Housekeeping on receipt: ten notes of one size become one of the
        // next, so a vault never accumulates a drawer full of small change.
        match notepool::merge_held_notes(&wallet, wallet_secret, 4).await {
            Ok((0, None)) => {}
            Ok((merged, failure)) => {
                if merged > 0 {
                    tprintln!(ctx, "{}", crate::ui::dim(format!("Tidied {merged} group(s) of ten notes into larger ones.")));
                }
                if let Some(reason) = failure {
                    tprintln!(ctx, "{}", crate::ui::dim(format!("(note tidying stopped: {reason})")));
                }
            }
            Err(err) => tprintln!(ctx, "{}", crate::ui::dim(format!("(note tidying skipped: {err})"))),
        }
        tprintln!(ctx, "");
        Ok(())
    }
}
