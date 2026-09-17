use crate::imports::*;

#[derive(Default, Handler)]
#[help("Create a wallet (shorthand for 'wallet create [<name>]')")]
pub struct Create;

impl Create {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        ctx.exec_within(&format!("wallet {cmd}")).await
    }
}
