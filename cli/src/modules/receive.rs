use crate::imports::*;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::account::notepool::{self, BearerNote, HANDOVER_PREFIX, Handover, LOCKED_HANDOVER_PREFIX, LockedHandover};
use kaspa_wallet_core::utils::sompi_to_kaspa_string;

#[derive(Default, Handler)]
#[help("Receive money: takes the code someone paid you with")]
pub struct Receive;

impl Receive {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let ticker = ctx.ticker();
        let Some(code) = argv.first() else {
            tprintln!(ctx, "usage: 'receive <code>' — the code the payer gave you; 'receive key [name]' makes a key to be paid to");
            return Ok(());
        };
        ctx.node_ready_for_notes().await?;
        let wallet = ctx.wallet();
        let (wallet_secret, _payment_secret) = ctx.ask_wallet_secret(None).await?;
        let note = crate::modules::note::Note;
        note.ensure_vault_interactive(&ctx, &wallet_secret).await?;

        // 'receive key [name]': a standing key to hand out, like a phone number
        // (PLAN P8.0g). One for everyone, or one per person — a key given
        // to one person names them in 'history' and cannot be compared with
        // anyone else's. 'receive keys' lists them.
        if code == "key" || code == "keys" {
            let store = wallet.store().as_note_key_store()?;
            if code == "key" {
                let label = argv[1..].join(" ");
                let info = store.add_share_key(&wallet_secret, &label).await?;
                let text = notepool::share_key_to_text(&info.pk);
                tprintln!(ctx, "");
                if let Some(qr) = crate::modules::note::qr_string(&text) {
                    tprintln!(ctx, "{}", qr);
                }
                tprintln!(ctx, "{text}");
                tprintln!(ctx, "");
                tpara!(
                    ctx,
                    "Give this to whoever should pay you{}. They type 'pay <amount>' and this key; the money is theirs to send and yours to take for as long as the lock they set lasts, and comes back to them if you never take it.",
                    if label.is_empty() { String::new() } else { format!(" ({label})") }
                );
                tprintln!(ctx, "");
            } else {
                let keys = store.share_keys().await?;
                tprintln!(ctx, "");
                if keys.is_empty() {
                    tprintln!(ctx, "No keys yet — 'receive key' makes one.");
                }
                for key in keys {
                    tprintln!(
                        ctx,
                        "{:<16} {}",
                        if key.label.is_empty() { "(no name)".to_string() } else { key.label.clone() },
                        notepool::share_key_to_text(&key.pk)
                    );
                }
                tprintln!(ctx, "");
            }
            return Ok(());
        }

        if code.starts_with(LOCKED_HANDOVER_PREFIX) {
            // A payment to one of our keys, under a lock: derive the key, take
            // the notes while it is still ours to do so.
            let handover = LockedHandover::from_text(code)?;
            let result = notepool::receive_locked(&wallet, wallet_secret.clone(), handover).await?;
            let stamp = result.notes.iter().any(|(_, d)| *d == kaspa_consensus_core::notepool::DenominationTag::D0_01);
            let value = if stamp { result.value_petals - DENOMINATION_PETALS[0] } else { result.value_petals };
            ctx.record("received", value, 0, "locked code", result.rotation.transaction_id.to_string());
            ctx.refresh_prompt_total().await;
            tprintln!(ctx, "");
            tprintln!(
                ctx,
                "Received {} {ticker} in {} note(s).",
                sompi_to_kaspa_string(value),
                result.notes.len() - usize::from(stamp)
            );
            tprintln!(
                ctx,
                "{}",
                crate::ui::dim(format!("Taken in time and made yours alone (tx {}).", result.rotation.transaction_id))
            );
        } else if code.starts_with(HANDOVER_PREFIX) {
            // A payment: the notes come under one key the payer made for this
            // handover, with the stamp for making them ours.
            let handover = Handover::from_text(code)?;
            let result = notepool::receive_handover(&wallet, wallet_secret.clone(), handover).await?;
            let stamp = result.notes.iter().any(|(_, d)| *d == kaspa_consensus_core::notepool::DenominationTag::D0_01);
            let value = if stamp { result.value_petals - DENOMINATION_PETALS[0] } else { result.value_petals };
            ctx.record("received", value, 0, "code", result.rotation.transaction_id.to_string());
            ctx.refresh_prompt_total().await;
            tprintln!(ctx, "");
            tprintln!(
                ctx,
                "Received {} {ticker} in {} note(s).",
                sompi_to_kaspa_string(value),
                result.notes.len() - usize::from(stamp)
            );
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
            ctx.record(
                "received",
                DENOMINATION_PETALS[bearer.d as usize],
                result.rotation.fee_petals,
                "note",
                result.rotation.transaction_id.to_string(),
            );
            ctx.refresh_prompt_total().await;
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
