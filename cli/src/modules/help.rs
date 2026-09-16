use crate::imports::*;

#[derive(Default, Handler)]
#[help("Displays this help message")]
pub struct Help;

impl Help {
    async fn main(self: Arc<Self>, dyn_ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let term = dyn_ctx.term();
        term.writeln("\nCommands:".crlf());

        let ctx = dyn_ctx.clone().downcast_arc::<KaspaCli>()?;
        let handlers = ctx.handlers().collect();
        // Everyday commands only; the expert surface lives behind 'advanced'.
        let advanced = ctx.advanced();
        let everyday = crate::modules::advanced::everyday(&ctx).await;
        let handlers = handlers
            .into_iter()
            .filter_map(|h| h.verb(dyn_ctx).map(|verb| (verb, get_handler_help(h, dyn_ctx))))
            .filter(|(verb, _)| advanced || everyday.contains(verb))
            .collect::<Vec<_>>();

        term.help(&handlers, None)?;

        term.writeln("New to Marigold? 'guide' walks you through the basics.".crlf());
        if advanced {
            term.writeln("Advanced mode is on: this is every command. 'advanced off' shows the everyday ones only.".crlf());
        }

        Ok(())
    }
}
