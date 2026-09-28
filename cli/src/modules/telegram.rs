use crate::imports::*;

/// 'telegram': the wallet's own Telegram bot as a first-class command
/// (founder, 2026-09-27: "make the Telegram command a first level citizen —
/// it's important"). It replaces 'mobile telegram …' and the note mirror,
/// which the bot superseded.
#[derive(Default, Handler)]
#[help("Your phone as a remote control, and the wallet's backup, through your own Telegram bot")]
pub struct Telegram;

impl Telegram {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        #[cfg(not(feature = "embedded-node"))]
        {
            let _ = argv;
            tprintln!(ctx, "This build cannot run the Telegram bot. Use a release build.");
            Ok(())
        }
        #[cfg(feature = "embedded-node")]
        self.run(&ctx, argv).await
    }
}

#[cfg(feature = "embedded-node")]
impl Telegram {
    async fn run(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        use crate::telegram::TelegramConfig;
        let sub = argv.first().map(|s| s.as_str());
        // A restore is what you do before you have a wallet here.
        if sub == Some("restore") {
            return ctx.exec_within(&format!("wallet restore telegram {}", argv[1..].join(" "))).await;
        }
        let Some(descriptor) = ctx.wallet().store().descriptor() else {
            tprintln!(ctx, "Open the wallet first — the bot is paired to one wallet. ('telegram restore' brings one back from its backups.)");
            return Ok(());
        };
        let name = descriptor.filename.clone();
        let path = crate::tgbackup::telegram_config_path(ctx)?;
        let existing = TelegramConfig::load(&path);
        match sub {
            None | Some("status") => self.status(ctx, &name, existing.as_ref()).await,
            Some("link") => self.link(ctx, argv.get(1).map(|s| s.as_str()), existing, &path).await,
            Some("unlink") => {
                ctx.stop_telegram_bot();
                if path.exists() {
                    std::fs::remove_file(&path).map_err(|e| Error::custom(e.to_string()))?;
                    tprintln!(ctx, "Unlinked. The bot no longer reaches this wallet; the backups already in its chat stay there.");
                } else {
                    tprintln!(ctx, "No bot is linked.");
                }
                Ok(())
            }
            Some("backup") => ctx.exec_within("wallet backup telegram now").await,
            Some("autobackup") => match argv.get(1).map(|s| s.as_str()) {
                Some("on") => ctx.exec_within("wallet backup telegram on").await,
                Some("off") => ctx.exec_within("wallet backup telegram off").await,
                _ => ctx.exec_within("wallet backup telegram status").await,
            },
            Some("limit") => {
                let Some(mut cfg) = existing else {
                    tprintln!(ctx, "No bot yet — 'telegram link <token>' first.");
                    return Ok(());
                };
                let petals = crate::utils::try_parse_required_nonzero_kaspa_as_sompi_u64(argv.get(1))?;
                cfg.daily_limit_petals = petals;
                cfg.save(&path).map_err(|e| Error::custom(e.to_string()))?;
                tprintln!(ctx, "Daily limit is now {} {}.", sompi_to_kaspa_string(petals), ctx.ticker());
                self.restart_bot(ctx).await;
                Ok(())
            }
            Some("pin") => {
                let Some(mut cfg) = existing else {
                    tprintln!(ctx, "No bot yet — 'telegram link <token>' first.");
                    return Ok(());
                };
                let Some(pin) = Self::ask_pin(ctx).await? else { return Ok(()) };
                cfg.set_pin(&pin);
                cfg.locked = false;
                cfg.save(&path).map_err(|e| Error::custom(e.to_string()))?;
                tprintln!(ctx, "PIN changed.");
                self.restart_bot(ctx).await;
                Ok(())
            }
            Some("unlock") => {
                let Some(mut cfg) = existing else {
                    tprintln!(ctx, "No bot yet — 'telegram link <token>' first.");
                    return Ok(());
                };
                cfg.locked = false;
                cfg.save(&path).map_err(|e| Error::custom(e.to_string()))?;
                tprintln!(ctx, "Unlocked. The bot answers the PIN again.");
                self.restart_bot(ctx).await;
                Ok(())
            }
            Some("code") => {
                let Some(mut cfg) = existing else {
                    tprintln!(ctx, "No bot yet — 'telegram link <token>' first.");
                    return Ok(());
                };
                if cfg.user_id.is_some() {
                    tprintln!(ctx, "Already paired with Telegram user {}. 'telegram unlink' and 'telegram link' pair it afresh.", cfg.user_id.unwrap_or(0));
                    return Ok(());
                }
                cfg.new_pairing_code();
                cfg.save(&path).map_err(|e| Error::custom(e.to_string()))?;
                tprintln!(ctx, "A fresh code, good for fifteen minutes. Open your bot in Telegram and send it:  /start {}", cfg.pairing_code.as_deref().unwrap_or(""));
                self.restart_bot(ctx).await;
                Ok(())
            }
            Some(other) => {
                tprintln!(ctx, "'telegram {other}'? The commands:");
                self.usage(ctx);
                Ok(())
            }
        }
    }

    fn usage(&self, ctx: &Arc<KaspaCli>) {
        tprintln!(ctx, "  telegram                    where things stand");
        tprintln!(ctx, "  telegram link <token>       pair your own bot (a token from @BotFather)");
        tprintln!(ctx, "  telegram unlink             forget the bot");
        tprintln!(ctx, "  telegram backup             back the wallet up to the bot's chat now, and keep it current from then on");
        tprintln!(ctx, "  telegram autobackup on|off  pause or resume the automatic backup");
        tprintln!(ctx, "  telegram restore [<name>]   bring a wallet back from the backups you forward to the bot");
        tprintln!(ctx, "  telegram limit <amount>     the daily spending limit from the phone");
        tprintln!(ctx, "  telegram pin                change the PIN the bot asks for");
        tprintln!(ctx, "  telegram unlock             clear the lockout after three wrong PINs");
        tprintln!(ctx, "  telegram code               a fresh pairing code");
    }

    async fn status(&self, ctx: &Arc<KaspaCli>, name: &str, cfg: Option<&crate::telegram::TelegramConfig>) -> Result<()> {
        tprintln!(ctx, "");
        let Some(cfg) = cfg else {
            tpara!(ctx, "No Telegram bot yet. Make one with @BotFather in Telegram (/newbot, two minutes), then here: 'telegram link <token>'.");
            tprintln!(ctx, "");
            self.usage(ctx);
            tprintln!(ctx, "");
            return Ok(());
        };
        match (cfg.user_id, cfg.pairing_code_live()) {
            (Some(id), _) => {
                tprintln!(ctx, "Paired with Telegram user {id}.");
                if cfg.locked {
                    tprintln!(ctx, "{}", style("Locked after three wrong PINs — 'telegram unlock' clears it.").yellow());
                }
            }
            (None, true) => tprintln!(ctx, "Not paired yet. Open your bot in Telegram and send it:  /start {}", cfg.pairing_code.as_deref().unwrap_or("")),
            (None, false) => tprintln!(ctx, "Not paired yet, and the pairing code has lapsed — 'telegram code' makes a fresh one."),
        }
        tprintln!(ctx, "Daily limit: {} {} — 'telegram limit <amount>' changes it.", sompi_to_kaspa_string(cfg.daily_limit_petals), ctx.ticker());
        tprintln!(ctx, "Telegram answers as long as the {name} wallet is running.");
        if let Ok(files) = crate::tgbackup::WalletFiles::of(ctx).await {
            let st = crate::tgbackup::status(&files, Some(cfg));
            let when = |secs: u64| -> String {
                if secs == 0 {
                    return "never".to_string();
                }
                chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0)
                    .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default()
            };
            tprintln!(
                ctx,
                "Backup: {} — {}; last full copy {}{}, {} change set(s) since.",
                st.destination,
                match st.automatic.as_str() {
                    "on" => "automatic".to_string(),
                    "off" => "paused ('telegram autobackup on' resumes it)".to_string(),
                    _ => "not started ('telegram backup' starts it)".to_string(),
                },
                when(st.checkpoint_at),
                if st.checkpoint_at > 0 { format!(" ({})", crate::backup::human_size(st.checkpoint_bytes as usize)) } else { String::new() },
                st.deltas
            );
        }
        tprintln!(ctx, "");
        Ok(())
    }

    async fn ask_pin(ctx: &Arc<KaspaCli>) -> Result<Option<String>> {
        tpara!(ctx, "A PIN the bot asks for before it pays. Four digits or more; it is not the wallet password.");
        let pin = ctx.term().ask(true, "PIN: ").await?.trim().to_string();
        if pin.len() < 4 {
            tprintln!(ctx, "Four characters at least — nothing changed.");
            return Ok(None);
        }
        let again = ctx.term().ask(true, "PIN again: ").await?.trim().to_string();
        if again != pin {
            tprintln!(ctx, "They differ — nothing changed.");
            return Ok(None);
        }
        Ok(Some(pin))
    }

    async fn link(
        &self,
        ctx: &Arc<KaspaCli>,
        token: Option<&str>,
        existing: Option<crate::telegram::TelegramConfig>,
        path: &std::path::Path,
    ) -> Result<()> {
        use crate::telegram::{DEFAULT_DAILY_LIMIT_PETALS, TelegramConfig};
        let Some(token) = token else {
            tprintln!(ctx, "usage: 'telegram link <token>' — the token @BotFather gave you for your bot (/newbot).");
            return Ok(());
        };
        if !token.contains(':') || token.len() < 20 {
            tprintln!(ctx, "That does not look like a bot token (BotFather gives one like 123456789:AA...).");
            return Ok(());
        }
        tprintln!(ctx, "");
        let Some(pin) = Self::ask_pin(ctx).await? else { return Ok(()) };
        let cfg = TelegramConfig::new(
            token.to_string(),
            &pin,
            existing.as_ref().map(|c| c.daily_limit_petals).unwrap_or(DEFAULT_DAILY_LIMIT_PETALS),
        );
        cfg.save(path).map_err(|e| Error::custom(e.to_string()))?;
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style("Linked.").green());
        tprintln!(ctx, "Open your bot in Telegram and send it, within fifteen minutes:  /start {}", cfg.pairing_code.as_deref().unwrap_or(""));
        tprintln!(ctx, "Daily limit {} {}; 'telegram limit <amount>' changes it.", sompi_to_kaspa_string(cfg.daily_limit_petals), ctx.ticker());
        tpara!(ctx, "{}", style("The Telegram account that pairs can then move money here. Turn on two-step verification in Telegram: accounts recover by SMS otherwise.").yellow());
        tprintln!(ctx, "");
        // The bot answers from this wallet while it is open, so the pairing
        // can happen now; 'marigold-cli serve' does the same headless.
        self.restart_bot(ctx).await;
        Ok(())
    }

    /// Settings are read when the bot starts: start it again so they hold now.
    async fn restart_bot(&self, ctx: &Arc<KaspaCli>) {
        match ctx.tidying_secret() {
            Some(secret) => ctx.start_telegram_bot(secret).await,
            None => tprintln!(ctx, "{}", crate::ui::dim("(in effect the next time the wallet is opened)")),
        }
    }
}
