use crate::imports::*;
use crate::modules::note::qr_string;
use kaspa_wallet_core::storage::otp::{self, Otp as Authenticator, MAX_GRACE_SECS};

#[derive(Default, Handler)]
#[help("Ask for a code from your phone before this wallet spends anything")]
pub struct Otp;

impl Otp {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, mut argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        if !ctx.wallet().is_open() {
            tprintln!(ctx, "Open a wallet first — an authenticator belongs to one wallet, not to the program.");
            return Ok(());
        }

        if argv.is_empty() {
            return self.display_help(ctx, argv).await;
        }

        let action = argv.remove(0);
        match action.as_str() {
            "on" | "enable" | "add" => self.enable(&ctx).await,
            "off" | "disable" | "remove" => self.disable(&ctx).await,
            "status" => self.status(&ctx).await,
            "grace" => self.grace(&ctx, argv).await,
            "test" => self.test(&ctx).await,
            other => {
                tprintln!(ctx, "unknown command '{other}'");
                self.display_help(ctx, vec![]).await
            }
        }
    }

    async fn display_help(self: Arc<Self>, ctx: Arc<KaspaCli>, _argv: Vec<String>) -> Result<()> {
        ctx.term().help(
            &[
                ("on", "Set up an authenticator app for this wallet"),
                ("off", "Stop asking for a code (needs a code, or your 24 words)"),
                ("status", "Whether a code is being asked for, and what it does and does not cover"),
                ("grace [<minutes>]", "Let one code stand for a few minutes instead of asking every time"),
                ("test", "Check that your phone and this wallet agree, changing nothing"),
            ],
            None,
        )?;
        Ok(())
    }

    /// Enrol. The order matters: generate, show, *prove*, then store. A
    /// secret written before the user has shown they can produce a code from
    /// it is a wallet that demands something nobody can supply.
    async fn enable(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        if ctx.otp().is_some() {
            tprintln!(ctx, "");
            tprintln!(ctx, "This wallet already asks for a code. 'otp off' first if you want to enrol a different phone.");
            tprintln!(ctx, "");
            return Ok(());
        }

        tprintln!(ctx, "");
        tpara!(
            ctx,
            "This makes the wallet ask for a six-digit code from your phone before it spends, \
            exports, or hands over anything. It is protection for a wallet that is already \
            open — the machine you walked away from, the session someone else sits down at. "
        );
        tpara!(
            ctx,
            "It is not protection for the wallet file. Anyone with the file and your password \
            can read the same secret this sets up and generate their own codes. Your password \
            and your 24 words are still what stands between a thief and your money. "
        );

        let name = ctx.wallet().descriptor().map(|d| d.filename).unwrap_or_else(|| "wallet".to_string());
        let otp = Authenticator::generate();
        let uri = otp.provisioning_uri(&name);

        tprintln!(ctx, "");
        match qr_string(&uri) {
            Some(qr) => {
                tprintln!(ctx, "Scan this with Google Authenticator, Aegis, 1Password, or any app that does 6-digit codes:");
                tprintln!(ctx, "");
                ctx.term().writeln(qr);
            }
            None => tprintln!(ctx, "Add this to your authenticator app:"),
        }
        tprintln!(ctx, "");
        tprintln!(ctx, "Or type it in by hand:  {}", style(otp.manual_entry_key()).cyan());
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "Write that down somewhere safe, or enrol a second phone from it now. It is shown \
            once. If you lose it and the phone, the way back in is 'otp off' with your 24 \
            words — which is why those words matter more than any of this. "
        );
        tprintln!(ctx, "");

        // Prove it before storing it.
        let entered = ctx.term().ask(false, "Enter the code your app is showing now: ").await?;
        let now = otp::unix_now_secs();
        let Some(step) = otp.step_of(entered.trim(), now) else {
            tprintln!(ctx, "");
            tprintln!(ctx, "{}", style("That code does not match — nothing has been changed.").red());
            tpara!(
                ctx,
                "Either the secret went in wrong, or your phone's clock is off by more than half \
                a minute. Check the phone's time is set automatically, then run 'otp on' again. "
            );
            tprintln!(ctx, "");
            return Ok(());
        };

        let (wallet_secret, _) = ctx.ask_wallet_secret_without_otp(None).await?;
        ctx.wallet().store().set_otp(&wallet_secret, Some(otp)).await?;
        // The code they just proved counts as this session's code. Asking for
        // a second one immediately teaches nothing and annoys.
        ctx.note_otp_accepted(step, now);

        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style("Done — this wallet now asks for a code before it spends.").green());
        tprintln!(ctx, "");
        self.automation_caveat(ctx).await;
        tprintln!(ctx, "");
        Ok(())
    }

    /// Removing it needs the same proof as using it. Anything less and the
    /// protection is one typed word deep.
    async fn disable(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        if ctx.otp().is_none() {
            tprintln!(ctx, "");
            tprintln!(ctx, "This wallet is not asking for codes. Nothing to turn off.");
            tprintln!(ctx, "");
            return Ok(());
        }

        tprintln!(ctx, "");
        tprintln!(ctx, "Turning this off needs a current code — or, if the phone is gone, your 24 vault words.");
        tprintln!(ctx, "");

        // The password is asked for first and without a code, because the
        // recovery path below has to unlock the vault to check the words
        // against it, and demanding a code to get there would make the
        // recovery path need the thing it is recovering from.
        let (wallet_secret, _) = ctx.ask_wallet_secret_without_otp(None).await?;

        // The words are the way back for somebody whose phone is at the
        // bottom of a lake. They are the recovery path for everything else in
        // this wallet, and it would be strange if they were not the recovery
        // path for this.
        let proved = match ctx.require_otp("turning the authenticator off").await {
            Ok(()) => true,
            Err(_) => {
                tprintln!(ctx, "");
                tprintln!(ctx, "No code. Prove the wallet is yours with your 24 vault words instead.");
                tprintln!(ctx, "");
                self.prove_with_words(ctx, &wallet_secret).await?
            }
        };

        if !proved {
            tprintln!(ctx, "");
            tprintln!(ctx, "{}", style("Not turned off — nothing has been changed.").red());
            tprintln!(ctx, "");
            return Ok(());
        }

        ctx.wallet().store().set_otp(&wallet_secret, None).await?;
        ctx.reset_otp_session();

        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style("The authenticator has been removed. Codes are no longer asked for.").yellow());
        tprintln!(ctx, "Delete the entry from your phone too — it will keep showing codes nothing wants.");
        tprintln!(ctx, "");
        Ok(())
    }

    /// Check the 24 vault words against the vault. Returns whether they were
    /// right, and says nothing about *which* word was wrong.
    async fn prove_with_words(&self, ctx: &Arc<KaspaCli>, wallet_secret: &Secret) -> Result<bool> {
        let words = ctx.term().ask(true, "Your 24 vault recovery words: ").await?;
        let words = words.trim().to_string();
        if words.is_empty() {
            return Ok(false);
        }
        Ok(ctx.wallet().store().as_note_key_store()?.vault_words_match(&words, wallet_secret).await.unwrap_or(false))
    }

    async fn status(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        tprintln!(ctx, "");
        let Some(otp) = ctx.otp() else {
            tprintln!(ctx, "No authenticator. This wallet spends on your password alone.");
            tprintln!(ctx, "'otp on' sets one up — it takes about a minute.");
            tprintln!(ctx, "");
            return Ok(());
        };

        tprintln!(ctx, "{}", style("A code is asked for before this wallet spends.").green());
        if otp.grace_secs == 0 {
            tprintln!(ctx, "  every time              one code per operation");
        } else {
            tprintln!(ctx, "  grace                   one code stands for {} minutes", otp.grace_secs / 60);
        }
        tprintln!(ctx, "  set up                  {}", format_age(otp.enrolled_at));
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "What this protects: a wallet already open, on a machine somebody else reaches. \
            What it does not protect: the wallet file. The code's secret is inside the file, \
            under your password, so whoever has both can make their own codes. "
        );
        tprintln!(ctx, "");
        self.automation_caveat(ctx).await;
        tprintln!(ctx, "");
        Ok(())
    }

    /// Auto-mint and auto-sweep hold the password in memory on purpose and
    /// spend on a timer. Nothing asks them for a code, and somebody who has
    /// enrolled an authenticator has to be told that rather than find out.
    async fn automation_caveat(&self, ctx: &Arc<KaspaCli>) {
        if ctx.wallet().store().otp().ok().flatten().is_none() {
            return;
        }
        if ctx.auto_mint_armed() {
            tprintln!(ctx, "{}", style("Note: auto-mint or auto-sweep is armed on this wallet.").yellow());
            tprintln!(ctx, "{}", style("Those run on a timer and are not asked for a code — that is what arming them means.").dim());
            tprintln!(ctx, "{}", style("'auto off' if you would rather every spend went through the code.").dim());
        }
    }

    async fn grace(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        let Some(mut otp) = ctx.otp() else {
            tprintln!(ctx, "");
            tprintln!(ctx, "No authenticator is set up. 'otp on' first.");
            tprintln!(ctx, "");
            return Ok(());
        };

        let Some(arg) = argv.first() else {
            tprintln!(ctx, "");
            if otp.grace_secs == 0 {
                tprintln!(ctx, "A code is asked for every time. 'otp grace <minutes>' to let one stand for a while.");
            } else {
                tprintln!(ctx, "One code stands for {} minutes. 'otp grace 0' to ask every time.", otp.grace_secs / 60);
            }
            tprintln!(ctx, "");
            return Ok(());
        };

        let minutes: u64 = arg.parse().map_err(|_| Error::custom(format!("'{arg}' is not a number of minutes")))?;
        let seconds = minutes.saturating_mul(60);
        if seconds > MAX_GRACE_SECS {
            return Err(Error::custom(format!(
                "{} minutes is the most this will hold a code for — beyond that an unattended terminal is unprotected long enough that the code is decoration",
                MAX_GRACE_SECS / 60
            )));
        }

        otp.grace_secs = seconds;
        let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
        ctx.wallet().store().set_otp(&wallet_secret, Some(otp)).await?;

        tprintln!(ctx, "");
        if seconds == 0 {
            tprintln!(ctx, "{}", style("A code is now asked for every time.").green());
        } else {
            tprintln!(ctx, "{}", style(format!("One code now stands for {minutes} minutes.")).green());
            tprintln!(ctx, "{}", style("Anyone at this keyboard within that window spends without one.").dim());
        }
        tprintln!(ctx, "");
        Ok(())
    }

    /// Try a code and report, without spending anything or changing anything.
    /// The thing people want after setting this up at eleven at night.
    async fn test(&self, ctx: &Arc<KaspaCli>) -> Result<()> {
        let Some(otp) = ctx.otp() else {
            tprintln!(ctx, "");
            tprintln!(ctx, "No authenticator is set up, so there is nothing to test. 'otp on' first.");
            tprintln!(ctx, "");
            return Ok(());
        };

        tprintln!(ctx, "");
        let entered = ctx.term().ask(false, "Code from your authenticator: ").await?;
        let now = otp::unix_now_secs();
        if otp.step_of(entered.trim(), now).is_some() {
            tprintln!(ctx, "");
            tprintln!(ctx, "{}", style("That matches — your phone and this wallet agree.").green());
            tprintln!(ctx, "{}", style("Nothing was spent and nothing was changed; the code is still good for its own moment.").dim());
        } else {
            tprintln!(ctx, "");
            tprintln!(ctx, "{}", style("That does not match.").red());
            tpara!(
                ctx,
                "The usual cause is the phone's clock. Set it to update automatically and try \
                again. If it still fails, the entry in your app is for a different wallet — the \
                label it shows is the wallet's name. "
            );
        }
        tprintln!(ctx, "");
        Ok(())
    }
}

/// "three weeks ago", near enough. Exactness would be false precision on a
/// line whose only job is to tell somebody whether this is the phone they
/// still own.
fn format_age(enrolled_at: u64) -> String {
    let now = otp::unix_now_secs();
    if enrolled_at == 0 || enrolled_at > now {
        return "unknown".to_string();
    }
    let secs = now - enrolled_at;
    match secs {
        s if s < 3600 => "less than an hour ago".to_string(),
        s if s < 86_400 => format!("{} hours ago", s / 3600),
        s if s < 86_400 * 14 => format!("{} days ago", s / 86_400),
        s if s < 86_400 * 60 => format!("{} weeks ago", s / (86_400 * 7)),
        s => format!("{} months ago", s / (86_400 * 30)),
    }
}
