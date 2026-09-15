use crate::imports::*;

#[derive(Default, Handler)]
#[help("Put notes on your phone, take them back, or kill a lost phone's copies")]
pub struct Mobile;

impl Mobile {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        crate::modules::note::Note::default().mirror(&ctx, argv).await
    }
}
