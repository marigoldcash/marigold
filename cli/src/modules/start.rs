use crate::imports::*;

#[derive(Default, Handler)]
#[help("Start local node and open wallet")]
pub struct Start;

impl Start {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        // - TODO - check states

        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        ctx.exec_within("wallet open").await?;
        ctx.exec_within("node start").await?;

        Ok(())
    }
}
