use crate::imports::*;

/// The expert's way to turn the ledger into notes by hand. The wallet does
/// this on its own once the ledger is a workable size; above the
/// housekeeping ceiling it pauses and leaves the choice to the owner, and
/// this is the command that choice is made with (founder, 2026-09-16).
#[derive(Default, Handler)]
#[help("Turn ledger balance into notes by hand: 'mint <amount>' or 'mint all'")]
pub struct Mint;

impl Mint {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        crate::modules::note::Note.mint(&ctx, argv).await
    }
}
