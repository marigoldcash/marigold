use crate::imports::*;

#[derive(Default, Handler)]
#[help("Consolidate this account's coins into fewer, larger ones (fixes \"storage mass\" errors; for recovering funds from old derivation paths see 'account recover')")]
pub struct Sweep;

impl Sweep {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        let account = ctx.wallet().account()?;
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        // TODO fee_rate
        let fee_rate = None;
        let abortable = Abortable::default();
        // let ctx_ = ctx.clone();
        let (summary, _ids) = account
            .sweep(
                wallet_secret,
                payment_secret,
                fee_rate,
                &abortable,
                Some(Arc::new(move |_ptx| {
                    // tprintln!(ctx_, "Sending transaction: {}", ptx.id());
                })),
            )
            .await?;

        tprintln!(ctx, "Sweep: {summary}");

        Ok(())
    }
}
