use crate::imports::*;

#[derive(Default, Handler)]
#[help("Show this account's ledger address (the transparent side of your wallet)")]
pub struct Address;

impl Address {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        if argv.is_empty() {
            let address = ctx.account().await?.receive_address()?.to_string();
            tprintln!(ctx, "\n{address}\n");
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
