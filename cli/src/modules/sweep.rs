use crate::imports::*;
use std::sync::atomic::AtomicU64;

#[derive(Default, Handler)]
#[help("Consolidate this account's coins into fewer, larger ones (fixes \"storage mass\" errors; for recovering funds from old derivation paths see 'account recover')")]
pub struct Sweep;

impl Sweep {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        let account = ctx.wallet().account()?;
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;

        let utxo_count = account.utxo_context().mature_utxo_size();
        tprintln!(
            ctx,
            "Consolidating {} coin(s) into fewer, larger ones — this builds and submits a chain of transactions; \
             a large wallet takes a while (progress below)...",
            utxo_count.separated_string()
        );
        // TODO fee_rate
        let fee_rate = None;
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
                    if n == 1 || n % 25 == 0 {
                        tprintln!(ctx_, "  submitted {n} transaction(s)... (latest {})", ptx.id());
                    }
                })),
            )
            .await?;

        tprintln!(ctx, "  submitted {} transaction(s) in total", submitted.load(Ordering::Relaxed));
        tprintln!(ctx, "Sweep: {summary}");

        Ok(())
    }
}
