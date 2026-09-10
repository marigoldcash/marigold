use crate::imports::*;

#[derive(Default, Handler)]
#[help("Mine with spare CPU while your own node is running")]
pub struct Mine;

impl Mine {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        match argv.first().map(|s| s.as_str()) {
            Some("start") => ctx.start_mining(argv.get(1).cloned()).await,
            Some("stop") => ctx.stop_mining().await,
            Some("status") | None => {
                ctx.mining_status().await;
                Ok(())
            }
            Some(other) => {
                tprintln!(ctx, "usage: 'mine start [percent]' | 'mine stop' | 'mine status'  (got '{other}')");
                Ok(())
            }
        }
    }
}
