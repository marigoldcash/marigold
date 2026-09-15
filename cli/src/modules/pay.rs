use crate::imports::*;
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::account::notepool::{self, HandoverSelection};
use kaspa_wallet_core::utils::sompi_to_kaspa_string;

/// Is this a note's serial: 64 hex characters.
fn is_serial(arg: &str) -> bool {
    arg.len() == 64 && arg.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Default, Handler)]
#[help("Pay: an amount or a note as a code to hand over, or a request code")]
pub struct Pay;

impl Pay {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        // One verb, told apart by what follows it (founder, 2026-09-15): a
        // request code pays that request with nothing to hand over; an
        // amount or a serial becomes one code the receiver types into
        // 'receive', with the receiver's stamp inside — the sender pays.
        let selection = match argv.first().map(|s| s.as_str()) {
            Some(arg) if arg.starts_with("marigoldreq:") => return crate::modules::note::Note::default().pay(&ctx, argv).await,
            Some(arg) if is_serial(arg) => HandoverSelection::Serial(arg.parse::<Hash>().map_err(|_| Error::custom("that is not a note serial"))?),
            Some(arg) if arg.parse::<f64>().is_ok() => HandoverSelection::Amount(try_parse_required_nonzero_kaspa_as_sompi_u64(argv.first())?),
            _ => {
                tprintln!(ctx, "usage: 'pay <amount>' or 'pay <serial>' makes a code to hand over; 'pay <request-code>' pays a request");
                return Ok(());
            }
        };
        let ticker = ctx.ticker();
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(None).await?;
        let result = notepool::hand_over(&ctx.wallet(), wallet_secret, selection).await?;
        let text = result.handover.to_text();
        ctx.record("paid", result.value_petals, result.stamp_petals + result.transfer.fee_petals, "code handed over", result.transfer.transaction_id.to_string());

        tprintln!(ctx, "");
        if let Some(qr) = crate::modules::note::qr_string(&text) {
            tprintln!(ctx, "{}", qr);
        }
        tprintln!(ctx, "{text}");
        tprintln!(ctx, "");
        let count = result.handover.notes.len() - 1;
        tprintln!(
            ctx,
            "{} {ticker} in {count} note{}, plus a 0.01 stamp so the receiver can make it theirs (fee {} {ticker}).",
            sompi_to_kaspa_string(result.value_petals),
            if count == 1 { "" } else { "s" },
            sompi_to_kaspa_string(result.transfer.fee_petals)
        );
        tprintln!(ctx, "{}", crate::ui::warn("Anyone who sees this code can take the money. Give it to the receiver now; they type 'receive' and the code."));

        // The receiver can only take it once the transfer has landed; say
        // which it is rather than let them find out.
        let first = result.handover.notes[0].0;
        let rpc = ctx.wallet().rpc_api();
        let mut confirmed = false;
        for _ in 0..120 {
            if rpc.get_notes_by_serial(vec![first]).await?.iter().any(|entry| entry.sn == first) {
                confirmed = true;
                break;
            }
            workflow_core::task::sleep(Duration::from_millis(500)).await;
        }
        if confirmed {
            tprintln!(ctx, "{}", crate::ui::dim("Confirmed — the receiver can take it now."));
        } else {
            tprintln!(ctx, "{}", crate::ui::dim("Not confirmed yet — if the receiver is told it is not in the pool, they should try again in a moment."));
        }
        let _ = DENOMINATION_PETALS;
        tprintln!(ctx, "");
        Ok(())
    }
}
