use crate::imports::*;

#[derive(Default, Handler)]
#[help("A walkthrough: your first notes, paying, checking, keeping it safe")]
pub struct Guide;

impl Guide {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> cli::Result<()> {
        let cli = ctx.clone().downcast_arc::<KaspaCli>()?;
        // The guide reads the wallet's state, so it only ever describes this
        // person's situation: no ledger talk to a notes-only wallet, no
        // notes-only caveats to a wallet with a ledger, and a machine without
        // a wallet is told what to do first.
        let wallet_open = cli.wallet().is_open();
        let notes_only = wallet_open && !cli.has_ledger_account().await;

        let guide = include_str!("guide.txt");
        let lines = guide.split('\n');

        let mut paras = Vec::<String>::new();
        let mut para = String::new();
        let regex = Regex::new(r"\s+").unwrap();
        for line in lines {
            if line.trim().is_empty() {
                if !para.is_empty() {
                    let text = regex.replace_all(para.trim(), " ");
                    paras.push(text.to_string());
                    para.clear();
                }
            } else {
                para.push_str(line);
                para.push(' ');
            }
        }

        if !para.is_empty() {
            let text = regex.replace_all(para.trim(), " ");
            paras.push(text.to_string());
            para.clear();
        }

        // A paragraph may open with a tag naming who it is for.
        let tagged = Regex::new(r"^#(?:\[(\w+)\])?\s*").unwrap();

        for para in paras {
            let text = match tagged.captures(para.as_str()) {
                None => para.clone(),
                Some(captures) => {
                    let show = match captures.get(1).map(|m| m.as_str()) {
                        Some("nowallet") => !wallet_open,
                        Some("ledger") => !notes_only,
                        Some("notesonly") => notes_only,
                        // A bare '#' or '#[desktop]' is the desktop build's.
                        _ => application_runtime::is_nw(),
                    };
                    if !show {
                        continue;
                    }
                    tagged.replace(para.as_str(), "").to_string()
                }
            };
            tprintln!(ctx);
            tpara!(ctx, "{}", text);
        }
        tprintln!(ctx);

        Ok(())
    }
}
