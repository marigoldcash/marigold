use crate::imports::*;
use crate::ui;
use std::sync::atomic::AtomicU64;

#[derive(Default, Handler)]
#[help(
    "Consolidate this account's coins into fewer, larger ones (fixes \"storage mass\" errors; for recovering funds from old derivation paths see 'account recover')"
)]
pub struct Sweep;

impl Sweep {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        // `sweep <amount>` bounds the work by value. Four million coins is
        // hours of consolidation in one unbroken run, and somebody who types a
        // number wants a piece of it, not all of it — this argument used to be
        // taken and dropped on the floor, so `sweep 1000` consolidated all
        // 4,439,680 coins without a word about the number it was given.
        let limit = match argv.first() {
            Some(text) => Some(try_parse_required_nonzero_kaspa_as_sompi_u64(Some(text))?),
            None => None,
        };

        let account = ctx.ledger_account().await?;
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        let utxo_count = account.utxo_context().mature_utxo_size();
        // Zero coins means one of two things, and only one of them is "nothing
        // to do". Right after connecting, or while the sync is still catching
        // up, the ledger simply has not been read yet — a sweep then reported
        // "0 coin(s)" and a fee for zero transactions (founder, 2026-09-16).
        if utxo_count == 0 {
            if !ctx.ledger_is_known() {
                tprintln!(ctx, "The ledger has not been read yet — still connecting, or the sync is still catching up.");
                tprintln!(ctx, "{}", ui::dim("Try again in a moment; 'connect status' shows where it is."));
            } else {
                tprintln!(ctx, "Nothing to consolidate — the ledger holds no coins.");
            }
            return Ok(());
        }
        let ticker = ctx.ticker();

        // A sweep of several million coins is hours of work, and finding out
        // half way through that the disk is full costs all of it. Batch
        // records are not kept, so in the ordinary case this needs almost
        // nothing — it is 'history detail' that turns every batch into a file.
        let detail = ctx.wallet().settings().get::<bool>(WalletSettings::HistoryDetail).unwrap_or(false);
        let estimate = crate::space::sweep_estimate(utxo_count as u64, detail);
        if estimate > 0 {
            let folder = kaspa_wallet_core::storage::local::default_storage_folder().to_string();
            if let Ok(path) = workflow_store::fs::resolve_path(&folder) {
                // Twice the estimate: room to write it, and room for the
                // filesystem to not be at its very last block when we finish.
                let need = crate::space::Need { required: estimate, comfortable: estimate.saturating_mul(2) };
                if !ctx.disk_allows(
                    &path,
                    need,
                    "this sweep",
                    "'history detail off' stops the sweep recording every internal batch,\nwhich is what the estimate above is almost entirely made of.",
                ) {
                    return Ok(());
                }
            }
        }

        match limit {
            Some(limit) => tprintln!(
                ctx,
                "Consolidating up to {} {ticker} of this wallet's {} coin(s), then stopping — run 'sweep' again for more.",
                ui::ledger_amount(limit),
                utxo_count.separated_string()
            ),
            None => tprintln!(
                ctx,
                "Consolidating all {} coin(s) into fewer, larger ones — this builds and submits a chain of transactions; \
                 a large wallet takes a while (progress below). 'sweep <amount>' does it a piece at a time.",
                utxo_count.separated_string()
            ),
        }
        // As good as free when this wallet's own miner will mine it: the fee
        // comes back to our own coinbase (PLAN P8.3b).
        let lane = ctx.own_lane().await;
        let fee_rate = match lane {
            crate::cli::OwnLane::Use { .. } => Some(crate::cli::OWN_LANE_FEE_RATE),
            _ => None,
        };
        match lane {
            crate::cli::OwnLane::Use { every } => tprintln!(
                ctx,
                "{}",
                ui::dim(format!(
                    "Your own miner will mine these (a block about every {}), so the fee comes back to you.",
                    crate::cli::humanised_minutes((every.as_secs() / 60).max(1))
                ))
            ),
            crate::cli::OwnLane::TooSlow { every } => tprintln!(
                ctx,
                "{}",
                ui::dim(format!(
                    "Your miner finds a block about every {} — too rare to wait for, so this pays the network fee.",
                    crate::cli::humanised_minutes((every.as_secs() / 60).max(1))
                ))
            ),
            crate::cli::OwnLane::NoMiner => tprintln!(
                ctx,
                "{}",
                ui::dim("This costs the network fee. Mining here would make it free — 'mine start' first, then sweep.")
            ),
            crate::cli::OwnLane::NotOwnCopy => {}
        }
        let abortable = Abortable::default();
        let ctx_ = ctx.clone();
        let submitted = Arc::new(AtomicU64::new(0));
        let submitted_ = submitted.clone();

        let (summary, _ids) = account
            .sweep(
                wallet_secret,
                payment_secret,
                fee_rate,
                &abortable,
                Some(Arc::new(move |ptx| {
                    // Every 25th transaction, so a long sweep shows life
                    // without drowning the terminal.
                    let n = submitted_.fetch_add(1, Ordering::Relaxed) + 1;
                    if n == 1 || n.is_multiple_of(25) {
                        tprintln!(ctx_, "  submitted {n} transaction(s)... (latest {})", ptx.id());
                    }
                })),
                limit,
                // A sweep somebody typed is bounded by what they asked for,
                // not by a count chosen for the background.
                None,
            )
            .await?;

        tprintln!(ctx, "  submitted {} transaction(s) in total", submitted.load(Ordering::Relaxed));
        tprintln!(ctx, "Sweep: {summary}");
        if matches!(lane, crate::cli::OwnLane::NoMiner) && summary.aggregate_fees() > 0 {
            // Paid at the network rate. Say what the own lane would have cost:
            // the fee is linear in mass, so the ratio of the rates is the ratio
            // of the fees.
            let own = summary.aggregate_fees() / (kaspa_wallet_core::account::notepool::POOL_FEE_RATE as u64).max(1);
            tprintln!(
                ctx,
                "{}",
                ui::dim(format!(
                    "That was the network rate. With this node mining, the same sweep would have cost about {} {ticker}.",
                    kaspa_wallet_core::utils::sompi_to_kaspa_string(own.max(1))
                ))
            );
        }
        // The consolidated coins are on the ledger line as pending until they
        // confirm; that is the whole of what happens in the background.
        tprintln!(ctx, "{}", ui::dim("The new coins show as pending on the ledger until they confirm — 'balance' shows it."));

        Ok(())
    }
}
