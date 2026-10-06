use crate::imports::*;

#[derive(Default, Handler)]
#[help("Show the wallet's 24 words, for paper (shorthand for 'wallet words')")]
pub struct Words;

impl Words {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        ctx.exec_within("wallet words").await
    }
}
