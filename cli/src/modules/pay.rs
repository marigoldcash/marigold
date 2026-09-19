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
    /// The lock's length from 'for <n> days|hours', default three days: about the
    /// pruning period, which is when to give up, not a limit the chain imposes.
    fn lock_length_seconds(rest: &[String]) -> Result<u64> {
        let words: Vec<&str> = rest.iter().map(|s| s.as_str()).filter(|w| *w != "for").collect();
        match words.as_slice() {
            [] => Ok(3 * 86_400),
            [n, unit] => {
                let n: u64 = n.parse().map_err(|_| Error::custom(format!("'{n}' is not a number")))?;
                let seconds = match unit.trim_end_matches('s') {
                    "hour" | "h" => 3_600,
                    "day" | "d" => 86_400,
                    "week" | "w" => 7 * 86_400,
                    other => return Err(Error::custom(format!("'{other}' — say hours, days or weeks"))),
                };
                let total = n * seconds;
                if !(3_600..=30 * 86_400).contains(&total) {
                    return Err(Error::custom("a lock is between an hour and thirty days".to_string()));
                }
                Ok(total)
            }
            _ => Err(Error::custom("usage: pay <amount> <key> [for <n> days|hours|weeks]".to_string())),
        }
    }

    async fn pay_locked(ctx: &Arc<KaspaCli>, amount: &str, key: &str, rest: &[String]) -> Result<()> {
        let ticker = ctx.ticker();
        let petals = try_parse_required_nonzero_kaspa_as_sompi_u64(Some(amount))?;
        let share_pk = notepool::share_key_from_text(key)?;
        let seconds = Self::lock_length_seconds(rest)?;
        let network_id = ctx.wallet().network_id()?;
        let bps = kaspa_consensus_core::config::params::Params::from(network_id).bps();
        let now = ctx.wallet().rpc_api().get_server_info().await?.virtual_daa_score;
        let until_daa = now + seconds * bps;
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(None).await?;
        let result = notepool::hand_over_locked(&ctx.wallet(), wallet_secret, petals, share_pk, until_daa).await?;
        let text = result.handover.to_text();
        ctx.record(
            "offered",
            result.value_petals,
            result.stamp_petals + result.transfer.fee_petals,
            format!("locked {}", crate::cli::humanised_minutes(seconds / 60)),
            result.transfer.transaction_id.to_string(),
        );
        tprintln!(ctx, "");
        if let Some(qr) = crate::modules::note::qr_string(&text) {
            tprintln!(ctx, "{}", qr);
        }
        tprintln!(ctx, "{text}");
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "{} {ticker} offered for {}, plus a 0.01 stamp (fee {} {ticker}). Give them this code; they type 'receive' and the code. Only their key can take it, and if they have not by then it comes back to you on its own.",
            sompi_to_kaspa_string(result.value_petals),
            crate::cli::humanised_minutes(seconds / 60),
            sompi_to_kaspa_string(result.transfer.fee_petals)
        );
        tprintln!(ctx, "");
        Ok(())
    }

    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        // One verb, told apart by what follows it (founder, 2026-09-15): a
        // request code pays that request with nothing to hand over; an
        // amount or a serial becomes one code the receiver types into
        // 'receive', with the receiver's stamp inside — the sender pays.
        // 'pay <amount> <key> [for <n> days|hours]': to someone's share key,
        // under a lock (PLAN P8.0g). Theirs to take until the lock lapses,
        // ours again after; the code carries no key the payer could use.
        if let (Some(amount), Some(key)) = (argv.first(), argv.get(1))
            && key.starts_with(notepool::SHARE_KEY_PREFIX)
        {
            return Self::pay_locked(&ctx, amount, key, &argv[2..]).await;
        }
        let selection = match argv.first().map(|s| s.as_str()) {
            Some(arg) if arg.starts_with("marigoldreq:") => return crate::modules::note::Note.pay(&ctx, argv).await,
            Some(arg) if is_serial(arg) => {
                HandoverSelection::Serial(arg.parse::<Hash>().map_err(|_| Error::custom("that is not a note serial"))?)
            }
            Some(arg) if arg.parse::<f64>().is_ok() => {
                HandoverSelection::Amount(try_parse_required_nonzero_kaspa_as_sompi_u64(argv.first())?)
            }
            _ => {
                tprintln!(
                    ctx,
                    "usage: 'pay <amount>' or 'pay <serial>' makes a code to hand over; 'pay <request-code>' pays a request"
                );
                return Ok(());
            }
        };
        let ticker = ctx.ticker();
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(None).await?;
        let result = notepool::hand_over(&ctx.wallet(), wallet_secret, selection).await?;
        let text = result.handover.to_text();
        ctx.record(
            "paid",
            result.value_petals,
            result.stamp_petals + result.transfer.fee_petals,
            "code handed over",
            result.transfer.transaction_id.to_string(),
        );
        ctx.refresh_prompt_total().await;

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
        tprintln!(
            ctx,
            "{}",
            crate::ui::warn(
                "Anyone who sees this code can take the money. Give it to the receiver now; they type 'receive' and the code."
            )
        );

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
            tprintln!(
                ctx,
                "{}",
                crate::ui::dim(
                    "Not confirmed yet — if the receiver is told it is not in the pool, they should try again in a moment."
                )
            );
        }
        let _ = DENOMINATION_PETALS;
        tprintln!(ctx, "");
        Ok(())
    }
}
