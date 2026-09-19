use crate::imports::*;

#[derive(Default, Handler)]
#[help("Put notes on your phone, take them back, or kill a lost phone's copies")]
pub struct Mobile;

impl Mobile {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        if argv.first().map(|s| s.as_str()) == Some("telegram") {
            return Self::telegram(&ctx, &argv[1..]).await;
        }
        crate::modules::note::Note::default().mirror(&ctx, argv).await
    }
}

impl Mobile {
    /// 'mobile telegram <token>' pairs your own Telegram bot with this
    /// wallet (FORK-PLAN P8.0h); 'mobile telegram' shows the state; 'mobile
    /// telegram limit <amount>' sets the daily limit; 'mobile telegram off'
    /// forgets it all. The bot answers only while 'marigold-cli serve' runs.
    #[cfg(feature = "embedded-node")]
    async fn telegram(ctx: &Arc<KaspaCli>, argv: &[String]) -> Result<()> {
        use crate::telegram::{DEFAULT_DAILY_LIMIT_PETALS, TelegramConfig};
        let Some(descriptor) = ctx.wallet().store().descriptor() else {
            tprintln!(ctx, "Open the wallet first — the bot is paired to one wallet.");
            return Ok(());
        };
        let folder: String = ctx
            .wallet()
            .settings()
            .get(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
        let path = TelegramConfig::path(&folder, &descriptor.filename);
        let existing = TelegramConfig::load(&path);
        match argv.first().map(|s| s.as_str()) {
            None => {
                tprintln!(ctx, "");
                match existing {
                    None => {
                        tpara!(
                            ctx,
                            "No Telegram bot yet. Make one with BotFather in Telegram (/newbot), then here: 'mobile telegram <token>'."
                        );
                    }
                    Some(cfg) => {
                        match (cfg.user_id, cfg.pairing_code) {
                            (Some(id), _) => tprintln!(ctx, "Paired with Telegram user {id}."),
                            (None, Some(code)) => tprintln!(ctx, "Not paired yet. Send your bot:  /start {code}"),
                            (None, None) => tprintln!(ctx, "Not paired."),
                        }
                        tprintln!(ctx, "Daily limit: {} {}", sompi_to_kaspa_string(cfg.daily_limit_petals), ctx.ticker());
                        tprintln!(ctx, "The bot answers while 'marigold-cli serve {}' runs.", descriptor.filename);
                    }
                }
                tprintln!(ctx, "");
            }
            Some("off") => {
                if path.exists() {
                    std::fs::remove_file(&path).map_err(|e| Error::custom(e.to_string()))?;
                    tprintln!(ctx, "Forgotten. The bot no longer reaches this wallet.");
                } else {
                    tprintln!(ctx, "Nothing to forget.");
                }
            }
            Some("limit") => {
                let Some(mut cfg) = existing else {
                    tprintln!(ctx, "No bot yet — 'mobile telegram <token>' first.");
                    return Ok(());
                };
                let petals = crate::utils::try_parse_required_nonzero_kaspa_as_sompi_u64(argv.get(1))?;
                cfg.daily_limit_petals = petals;
                cfg.save(&path).map_err(|e| Error::custom(e.to_string()))?;
                tprintln!(ctx, "Daily limit is now {} {} (from the next service start).", sompi_to_kaspa_string(petals), ctx.ticker());
            }
            Some(token) => {
                if !token.contains(':') || token.len() < 20 {
                    tprintln!(ctx, "That does not look like a bot token (BotFather gives one like 123456789:AA...).");
                    return Ok(());
                }
                tprintln!(ctx, "");
                tpara!(ctx, "A PIN the bot asks for before it pays. Four digits or more; it is not the wallet password.");
                let pin = ctx.term().ask(true, "PIN: ").await?.trim().to_string();
                if pin.len() < 4 {
                    tprintln!(ctx, "Four characters at least — nothing set up.");
                    return Ok(());
                }
                let again = ctx.term().ask(true, "PIN again: ").await?.trim().to_string();
                if again != pin {
                    tprintln!(ctx, "They differ — nothing set up.");
                    return Ok(());
                }
                let cfg = TelegramConfig::new(
                    token.to_string(),
                    &pin,
                    existing.as_ref().map(|c| c.daily_limit_petals).unwrap_or(DEFAULT_DAILY_LIMIT_PETALS),
                );
                cfg.save(&path).map_err(|e| Error::custom(e.to_string()))?;
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("Saved.").green());
                tprintln!(
                    ctx,
                    "Start the service:   marigold-cli serve {} --password-file <file with the wallet password>",
                    descriptor.filename
                );
                tprintln!(ctx, "Then send your bot:  /start {}", cfg.pairing_code.as_deref().unwrap_or(""));
                tprintln!(
                    ctx,
                    "Daily limit {} {}; 'mobile telegram limit <amount>' changes it.",
                    sompi_to_kaspa_string(cfg.daily_limit_petals),
                    ctx.ticker()
                );
                tpara!(ctx, "{}", style("The Telegram account that pairs can then move money here. Turn on two-step verification in Telegram: accounts recover by SMS otherwise.").yellow());
                tprintln!(ctx, "");
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "embedded-node"))]
    async fn telegram(ctx: &Arc<KaspaCli>, _argv: &[String]) -> Result<()> {
        tprintln!(ctx, "This build cannot run the wallet as a service. Use a release build.");
        Ok(())
    }
}
