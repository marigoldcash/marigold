use crate::imports::*;

/// Is this a note's serial: 64 hex characters.
fn is_serial(arg: &str) -> bool {
    arg.len() == 64 && arg.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Default, Handler)]
#[help("Pay: a request code, or hand a note over by serial")]
pub struct Pay;

impl Pay {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let note = crate::modules::note::Note::default();
        // One verb, told apart by what follows it (founder, 2026-09-15): a
        // request code pays that request with nothing to hand over; a serial
        // hands that note over as a code the receiver types into 'receive'.
        match argv.first().map(|s| s.as_str()) {
            Some(arg) if arg.starts_with("marigoldreq:") => note.pay(&ctx, argv).await,
            Some(arg) if is_serial(arg) => note.export(&ctx, argv).await,
            Some(arg) if arg.parse::<f64>().is_ok() => {
                tprintln!(ctx, "'pay <amount>' is on its way. Until then 'pay <serial>' hands one note over ('note list' shows them).");
                Ok(())
            }
            _ => {
                tprintln!(ctx, "usage: 'pay <request-code>' pays a request; 'pay <serial>' hands that note over");
                Ok(())
            }
        }
    }
}
