use crate::imports::*;

#[derive(Default, Handler)]
#[help("Import a list of note keys from another wallet (see 'export')")]
pub struct Import;

impl Import {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        crate::modules::note::Note::default().import(&ctx, argv).await
    }
}
