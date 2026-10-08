use crate::imports::*;

#[derive(Default, Handler)]
#[help("Update the wallet to the newest release: download, check the trustees' signatures, install, restart")]
pub struct Update;

impl Update {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        crate::selfupdate::command(&ctx).await
    }
}
