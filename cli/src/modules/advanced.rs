use crate::imports::*;

/// Verbs shown by the plain `help`, for this wallet as it is. Everything
/// else lives behind `advanced` — the front page should read like a wallet,
/// not like a node console. 'address' only means something with a ledger;
/// 'mine' only once mining has been started on this wallet (founder,
/// 2026-09-15: the basics, without the techno-babble).
pub async fn everyday(ctx: &Arc<KaspaCli>) -> Vec<&'static str> {
    let mut verbs = vec![
        "balance", "pay", "receive", "request", "exchange", "move", "mobile", "backup", "history", "wallet", "open", "close", "connect",
        "disconnect", "node", "guide", "help", "advanced", "exit",
    ];
    if ctx.wallet().is_open() && ctx.has_ledger_account().await {
        verbs.push("address");
        if ctx.has_mined().await {
            verbs.push("mine");
        }
    }
    verbs
}

#[derive(Default, Handler)]
#[help("Show the expert commands; 'advanced on' shows the technical side everywhere, 'advanced off' hides it")]
pub struct Advanced;

impl Advanced {
    async fn main(self: Arc<Self>, dyn_ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let term = dyn_ctx.term();
        let ctx = dyn_ctx.clone().downcast_arc::<KaspaCli>()?;

        // One switch for the technical side, remembered across sessions
        // (founder, 2026-09-15): off, the wallet shows no addresses and no
        // cryptic errors and 'help' lists the everyday commands; on, it is
        // verbose — every command, addresses, the reason behind each error.
        match argv.first().map(|s| s.to_lowercase()).as_deref() {
            Some("on") => {
                ctx.set_advanced(true).await;
                tprintln!(ctx, "");
                tprintln!(ctx, "Advanced on: 'help' lists every command, addresses are shown, and errors say why.");
                tprintln!(ctx, "{}", style("'advanced off' puts it back.").dim());
                tprintln!(ctx, "");
                return Ok(());
            }
            Some("off") => {
                ctx.set_advanced(false).await;
                tprintln!(ctx, "");
                tprintln!(ctx, "Advanced off: the everyday wallet, without the technical side.");
                tprintln!(ctx, "");
                return Ok(());
            }
            Some(other) if other != "help" => {
                tprintln!(ctx, "usage: 'advanced', 'advanced on' or 'advanced off'");
                return Ok(());
            }
            _ => {}
        }

        let state = if ctx.advanced() { "on" } else { "off" };
        term.writeln(format!("\nAdvanced commands — you rarely need these. Advanced mode is {state} ('advanced on' / 'advanced off').").crlf());

        let everyday = everyday(&ctx).await;
        let handlers = ctx.handlers().collect();
        let handlers = handlers
            .into_iter()
            .filter_map(|h| h.verb(dyn_ctx).map(|verb| (verb, get_handler_help(h, dyn_ctx))))
            .filter(|(verb, _)| !everyday.contains(verb))
            .collect::<Vec<_>>();

        term.help(&handlers, None)?;

        Ok(())
    }
}
