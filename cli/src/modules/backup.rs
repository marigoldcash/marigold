use crate::imports::*;

#[derive(Default, Handler)]
#[help("Write the whole wallet — keys, notes and all — to one encrypted file")]
pub struct Backup;

impl Backup {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        ctx.exec_within(&format!("wallet backup {}", argv.join(" "))).await
    }
}
