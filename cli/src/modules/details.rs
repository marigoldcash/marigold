use crate::imports::*;

/// Kept as a name people may still type: 'address' is the one command for the
/// ledger address, and 'details' shows the same (founder, 2026-10-11).
#[derive(Default, Handler)]
#[help("The same as 'address'")]
pub struct Details;

impl Details {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        ctx.exec_within("address").await
    }
}
