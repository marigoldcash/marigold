//! The terminal's side of the automatic Telegram backups. The backups
//! themselves moved to the wallet service crate (PH-M1b4), where the phone
//! apps use the same code; what stays here takes the terminal's context
//! (`KaspaCli`): the one-time question at open, the wizard that covers wallets
//! without a backup key, and the thin functions that pass the open wallet to
//! the service. Everything that moved is re-exported, so paths do not change.

use crate::bundle::*;
use crate::cli::KaspaCli;
use crate::imports::*;
use crate::telegram::TelegramConfig;
use std::path::PathBuf;

pub use kaspa_wallet_service::tgbackup::*;

/// One backup run from the terminal: the open wallet's folder, its words to
/// check its own part by. Returns what was posted, for the line the caller prints.
pub async fn run(cli: &Arc<KaspaCli>, words: &str, force_checkpoint: bool, say: &(dyn Fn(String) + Send + Sync)) -> Result<String> {
    let files = wallet_files(cli).await?;
    let cfg_path = telegram_config_path(cli)?;
    let cfg = TelegramConfig::load(&cfg_path).ok_or_else(|| Error::custom("no Telegram bot is set up for this wallet"))?;
    let folder = Folder::around(&files)?;
    Ok(run_folder(&folder, &cfg, Some((&files.name, words)), force_checkpoint, true, say).await?)
}

pub fn telegram_config_path(cli: &Arc<KaspaCli>) -> Result<PathBuf> {
    let descriptor = cli.store().descriptor().ok_or_else(|| Error::custom("no wallet is open"))?;
    let folder: String = cli
        .wallet()
        .settings()
        .get(WalletSettings::Folder)
        .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
    Ok(TelegramConfig::path(&folder, &descriptor.filename))
}

/// Called from housekeeping every few seconds; cheap unless a post is due.
/// A post runs on its own task so housekeeping never waits on Telegram.
pub async fn auto_tick(cli: &Arc<KaspaCli>, state: &Arc<Mutex<AutoState>>) {
    if !cli.wallet().is_open() {
        return;
    }
    let Some(secret) = cli.tidying_secret() else { return };
    let Some(descriptor) = cli.store().descriptor() else { return };
    let Ok(cfg_path) = telegram_config_path(cli) else { return };
    let cli_ = cli.clone();
    let cli_warn = cli.clone();
    let say: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line: String| tprintln!(cli_, "{}", crate::ui::dim(line)));
    let warn: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line: String| tprintln!(cli_warn, "{}", crate::ui::warn(line)));
    auto_tick_for(cli.wallet(), cfg_path, descriptor.filename.clone(), secret, state.clone(), say, warn).await;
}

/// A backup run sealed per wallet, the open wallet's words — which the
/// wallet password unlocks — checking its own part.
pub async fn run_with_words(cli: &Arc<KaspaCli>, secret: &Secret, force_checkpoint: bool) -> Result<String> {
    let words = words_for(&cli.wallet(), secret).await?;
    let quiet = |_line: String| {};
    run(cli, &words, force_checkpoint, &quiet).await
}

/// At 'open' in the terminal: the one-time question, and the first checkpoint on a yes.
pub async fn offer_at_open(cli: &Arc<KaspaCli>) {
    let Some(secret) = cli.tidying_secret() else { return };
    let Some(descriptor) = cli.store().descriptor() else { return };
    let Ok(cfg_path) = telegram_config_path(cli) else { return };
    let cfg = TelegramConfig::load(&cfg_path);
    let Ok(files) = WalletFiles::of_wallet(&cli.wallet(), &descriptor.filename).await else { return };
    let settings = WalletBackupSettings::load(&files.wallet_dir);
    if !should_offer(&settings, cfg.as_ref()) {
        return;
    }
    tprintln!(cli, "");
    tprintln!(
        cli,
        "Your Telegram bot is paired. The wallet can keep an encrypted copy of every wallet on this computer in that chat by itself — a full copy every day, the changes within minutes of a payment, silently, the last two copies kept — each wallet sealed with its own 24 words."
    );
    let answer = match cli
        .term()
        .ask(false, "Back up your wallets automatically to your Telegram bot chat? (highly recommended) [Y/n]: ")
        .await
    {
        Ok(a) => a.trim().to_lowercase(),
        Err(_) => return,
    };
    let _ = mark_asked(&files);
    if answer.starts_with('n') {
        tprintln!(cli, "{}", crate::ui::dim("Run 'telegram backup' later if you change your mind."));
        tprintln!(cli, "");
        return;
    }
    tprintln!(cli, "Posting the first full copy…");
    let _ = mark_started(&files);
    match run_with_words(cli, &secret, true).await {
        Ok(line) => {
            tprintln!(
                cli,
                "{}",
                crate::ui::dim(format!("Telegram backup: {line}. From now on it stays current by itself while the wallet is open."))
            );
        }
        Err(err) => {
            tprintln!(cli, "{}", crate::ui::warn(format!("The backup did not go out: {err} — 'telegram backup' tries again.")))
        }
    }
    tprintln!(cli, "");
}

/// At 'open' in the terminal, after the offer: wallets in the folder that
/// have no backup key yet (made before 2026-10-06 and not opened since), and
/// what to do about them. Only when the backups have somewhere to go.
pub async fn cover_at_open(cli: &Arc<KaspaCli>) {
    let Some(descriptor) = cli.store().descriptor() else { return };
    let Ok(cfg_path) = telegram_config_path(cli) else { return };
    let Some(cfg) = TelegramConfig::load(&cfg_path) else { return };
    if target_chat(&cfg).is_none() {
        return;
    }
    let Ok(files) = WalletFiles::of_wallet(&cli.wallet(), &descriptor.filename).await else { return };
    let Ok(folder) = Folder::around(&files) else { return };
    if folder.uncovered().is_empty() {
        return;
    }
    cover_wizard(cli, &folder, false).await;
}

/// The wizard proper: explains the change once, then asks per wallet.
/// `all`: include the wallets marked 'never' ('telegram cover').
pub async fn cover_wizard(cli: &Arc<KaspaCli>, folder: &Folder, all: bool) {
    let waiting: Vec<&WalletFiles> = folder
        .wallets
        .iter()
        .filter(|w| w.recipient.is_none() && (all || !w.skipped()) && Some(&w.name) != folder.open.as_ref())
        .collect();
    if waiting.is_empty() {
        if all {
            tprintln!(cli, "Every wallet in {} is covered.", folder.path.display());
        }
        return;
    }
    let names: Vec<String> = waiting.iter().map(|w| w.name.clone()).collect();
    let covered: Vec<String> = folder.covered().iter().map(|w| w.name.clone()).collect();
    tprintln!(cli, "");
    if !covered.is_empty() {
        tprintln!(cli, "{}", crate::ui::dim(format!("Covered already: {}.", covered.join(", "))));
    }
    tprintln!(
        cli,
        "The backups now cover every wallet on this computer, each sealed with its own 24 words. Since this change, {} no backup key yet: {}.",
        if names.len() == 1 { "one wallet has" } else { "these wallets have" },
        names.join(", ")
    );
    tprintln!(
        cli,
        "{}",
        crate::ui::dim(
            "A wallet gets its key the first time it is opened with its password — or now: type its 24 words (checked against the wallet; nothing is kept but a public key), or 'view' to give its password instead, see its words and have them used. Enter skips it for now; 'never' leaves it out of the backups ('telegram cover' asks again)."
        )
    );
    for wallet in waiting {
        tprintln!(cli, "");
        loop {
            let answer = match cli
                .term()
                .ask(
                    true,
                    &format!("The 24 words of '{}' (Enter to skip, 'never' to leave it out, 'view' to see them): ", wallet.name),
                )
                .await
            {
                Ok(a) => a.trim().to_string(),
                Err(_) => return,
            };
            if answer.is_empty() || matches!(answer.to_lowercase().as_str(), "n" | "no" | "skip" | "later") {
                tprintln!(cli, "{}", crate::ui::dim(format!("'{}' is covered once you open it.", wallet.name)));
                break;
            }
            if answer.eq_ignore_ascii_case("never") {
                match skip(folder, &wallet.name) {
                    Ok(()) => tprintln!(cli, "{}", crate::ui::dim(format!("'{}' is left out of the backups.", wallet.name))),
                    Err(err) => tprintln!(cli, "{}", crate::ui::warn(err.to_string())),
                }
                break;
            }
            if matches!(answer.to_lowercase().as_str(), "view" | "show" | "password") {
                let secret = match cli.term().ask(true, &format!("The password of '{}': ", wallet.name)).await {
                    Ok(p) => Secret::new(p.trim().as_bytes().to_vec()),
                    Err(_) => return,
                };
                if secret.as_ref().is_empty() {
                    continue;
                }
                match cover_with_password(folder, &wallet.name, &secret).await {
                    Ok((words, new)) => {
                        tprintln!(cli, "");
                        if new {
                            tprintln!(
                                cli,
                                "{}",
                                crate::ui::dim(format!(
                                    "'{}' had no 24 words yet — it was made before wallets had them. These are its words now.",
                                    wallet.name
                                ))
                            );
                            tprintln!(cli, "");
                        }
                        crate::ui::recovery_words(cli, &words);
                        tprintln!(cli, "");
                        tprintln!(
                            cli,
                            "{}",
                            style(format!(
                                "'{}' is covered from the next backup on. Those are its words — write them down.",
                                wallet.name
                            ))
                            .green()
                        );
                        break;
                    }
                    Err(err) => {
                        tprintln!(cli, "{}", crate::ui::warn(format!("{err} — try again, or Enter to skip.")));
                        continue;
                    }
                }
            }
            match cover(folder, &wallet.name, &answer) {
                Ok(()) => {
                    tprintln!(cli, "{}", style(format!("'{}' is covered from the next backup on.", wallet.name)).green());
                    break;
                }
                Err(err) => tprintln!(
                    cli,
                    "{}",
                    crate::ui::warn(format!("'{}' is not covered: {err} — try again, 'view', or Enter to skip.", wallet.name))
                ),
            }
        }
    }
    tprintln!(cli, "");
}

/// At 'close': a change not yet posted goes out now rather than at the next open.
pub async fn flush_before_close(cli: &Arc<KaspaCli>) {
    let Some(secret) = cli.tidying_secret() else { return };
    let Some(descriptor) = cli.store().descriptor() else { return };
    let Ok(cfg_path) = telegram_config_path(cli) else { return };
    let say = |line: String| tprintln!(cli, "{}", crate::ui::dim(line));
    let warn = |line: String| tprintln!(cli, "{}", crate::ui::warn(line));
    flush_for(&cli.wallet(), &cfg_path, &descriptor.filename, &secret, &say, &warn).await;
}
