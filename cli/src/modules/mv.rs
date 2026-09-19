use crate::imports::*;

/// 'move' — a keyword in Rust, so the module is `mv` and the verb is given
/// by hand rather than derived from the type name.
#[derive(Default)]
pub struct Mv;

#[async_trait]
impl Handler for Mv {
    fn verb(&self, _ctx: &Arc<dyn Context>) -> Option<&'static str> {
        Some("move")
    }

    fn help(&self, _ctx: &Arc<dyn Context>) -> &'static str {
        "Move notes into another wallet on this machine"
    }

    async fn handle(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> cli::Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        crate::modules::note::Note.move_notes(&ctx, argv).await.map_err(|e| e.into())
    }
}
