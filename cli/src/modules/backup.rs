use crate::imports::*;

#[derive(Default, Handler)]
#[help("Every wallet on this computer into one encrypted file; 'backup folder <path>' keeps them current in a folder")]
pub struct Backup;

impl Backup {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        ctx.exec_within(&format!("wallet backup {}", argv.join(" "))).await
    }
}
