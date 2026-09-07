use crate::imports::*;

#[derive(Default, Handler)]
#[help("Stop local node and close wallet")]
pub struct Stop;

impl Stop {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        ctx.exec_within("wallet close").await?;
        ctx.exec_within("disconnect").await?;
        ctx.exec_within("node stop").await?;

        Ok(())
    }
}
