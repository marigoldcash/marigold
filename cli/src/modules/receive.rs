use crate::imports::*;

#[derive(Default, Handler)]
#[help("Receive money: takes the code someone paid you with")]
pub struct Receive;

impl Receive {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        crate::modules::note::Note::default().import(&ctx, argv).await
    }
}
