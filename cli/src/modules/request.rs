use crate::imports::*;

#[derive(Default, Handler)]
#[help("Ask to be paid: prints a code for the payer, then waits for the money")]
pub struct Request;

impl Request {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        crate::modules::note::Note::default().request(&ctx, argv).await
    }
}
