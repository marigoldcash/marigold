use crate::imports::*;

#[derive(Default, Handler)]
#[help("Run a node inside this wallet — nobody else sees your notes or your address")]
pub struct Mynode;

impl Mynode {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        match argv.first().map(|s| s.as_str()) {
            Some("start") => ctx.start_embedded_node().await,
            Some("stop") => ctx.stop_embedded_node().await,
            Some("status") | None => {
                if ctx.embedded_node_running() {
                    tprintln!(ctx, "Your node is running.");
                } else {
                    tprintln!(ctx, "Your node is not running.");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "  'mynode start'   run one here. Nobody sees your notes or your address.");
                    tprintln!(ctx, "                   Takes a while to catch up the first time, and several");
                    tprintln!(ctx, "                   gigabytes of disk.");
                    tprintln!(ctx, "  'connect public' use a node the Marigold project runs instead — instant,");
                    tprintln!(ctx, "                   but they see what your wallet asks about.");
                }
                Ok(())
            }
            Some(other) => {
                tprintln!(ctx, "usage: 'mynode start' | 'mynode stop' | 'mynode status'  (got '{other}')");
                Ok(())
            }
        }
    }
}
