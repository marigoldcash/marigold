use crate::imports::*;

/// Verbs shown by the plain `help`. Everything else lives behind `advanced` —
/// the CLI's front page should read like a wallet, not like a node console.
pub const EVERYDAY: &[&str] = &[
    "balance", "note", "exchange", "node", "address", "list", "open", "close", "wallet", "connect", "network", "guide", "help", "advanced",
    "exit", "quit",
];

#[derive(Default, Handler)]
#[help("Show the advanced/expert commands (the everyday ones are in 'help')")]
pub struct Advanced;

impl Advanced {
    async fn main(self: Arc<Self>, dyn_ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let term = dyn_ctx.term();
        let ctx = dyn_ctx.clone().downcast_arc::<KaspaCli>()?;

        term.writeln("\nAdvanced commands — you rarely need these.".crlf());

        let handlers = ctx.handlers().collect();
        let handlers = handlers
            .into_iter()
            .filter_map(|h| h.verb(dyn_ctx).map(|verb| (verb, get_handler_help(h, dyn_ctx))))
            .filter(|(verb, _)| !EVERYDAY.contains(verb))
            .collect::<Vec<_>>();

        term.help(&handlers, None)?;

        Ok(())
    }
}
