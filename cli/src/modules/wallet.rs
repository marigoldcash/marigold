use crate::imports::*;
use crate::wizards;
use std::str::FromStr;

#[derive(Default, Handler)]
#[help("Wallet management operations")]
pub struct Wallet;

impl Wallet {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, mut argv: Vec<String>, cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        let guard = ctx.wallet().guard();
        let guard = guard.lock().await;

        if argv.is_empty() {
            return self.display_help(ctx, argv).await;
        }

        let op = argv.remove(0);
        match op.as_str() {
            "list" => {
                let wallets = ctx.store().wallet_list().await?;
                if wallets.is_empty() {
                    tprintln!(ctx, "No wallets found");
                } else {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Wallets:");
                    tprintln!(ctx, "");
                    for wallet in wallets {
                        let hidden = ctx
                            .store()
                            .client_metadata(&wallet.filename)
                            .await
                            .ok()
                            .flatten()
                            .map(|m| m.hidden)
                            .unwrap_or(false);
                        let mark = if hidden { "   (hidden from the picker)" } else { "" };
                        if let Some(title) = wallet.title {
                            tprintln!(ctx, "  {}: {}{mark}", wallet.filename, title);
                        } else {
                            tprintln!(ctx, "  {}{mark}", wallet.filename);
                        }
                    }
                    tprintln!(ctx, "");
                }
            }
            "create" | "import" => {
                let wallet_name = if argv.is_empty() {
                    None
                } else {
                    let name = argv.remove(0);
                    let name = name.trim().to_string();
                    let name_check = name.to_lowercase();
                    if name_check.as_str() == "wallet" {
                        return Err(Error::custom("Wallet name cannot be 'wallet'"));
                    }
                    Some(name)
                };

                let wallet_name = wallet_name.as_deref();
                let import_with_mnemonic = op.as_str() == "import";
                wizards::wallet::create(&ctx, guard.into(), wallet_name, import_with_mnemonic).await?;
            }
            "open" => {
                let name = if let Some(name) = argv.first().cloned() {
                    let name_check = name.to_lowercase();

                    if name_check.as_str() == "wallet" {
                        tprintln!(ctx, "you can not have a wallet named 'wallet'...");
                        tprintln!(ctx, "perhaps you are looking to use 'open <name>'");
                        return Ok(());
                    }
                    Some(name)
                } else {
                    // No name given: enumerate. One wallet opens directly; several
                    // get a numbered picker with the last-used one on <enter>.
                    let all = ctx.store().wallet_list().await?;
                    // Hidden wallets stay out of the picker (still openable by
                    // name; 'wallet list' shows them marked).
                    let mut wallets = Vec::with_capacity(all.len());
                    for w in all {
                        let hidden = ctx.store().client_metadata(&w.filename).await.ok().flatten().map(|m| m.hidden).unwrap_or(false);
                        if !hidden {
                            wallets.push(w);
                        }
                    }
                    match wallets.len() {
                        0 => {
                            tprintln!(ctx, "No wallets to show — create one with 'wallet create <name>' (hidden ones: 'wallet list')");
                            return Ok(());
                        }
                        1 => Some(wallets[0].filename.clone()),
                        _ => {
                            let last: Option<String> = ctx.wallet().settings().get(WalletSettings::Wallet);
                            let last = last.filter(|l| wallets.iter().any(|w| &w.filename == l));
                            tprintln!(ctx, "");
                            for (i, w) in wallets.iter().enumerate() {
                                // 1-based: humans count from one.
                                let n = i + 1;
                                let marker = if Some(&w.filename) == last.as_ref() { "  (last used)" } else { "" };
                                match &w.title {
                                    Some(title) => tprintln!(ctx, "{n}: {title} ({}){marker}", w.filename),
                                    None => tprintln!(ctx, "{n}: {}{marker}", w.filename),
                                }
                            }
                            tprintln!(ctx, "");
                            let default = last.unwrap_or_else(|| wallets[0].filename.clone());
                            let selection = ctx
                                .term()
                                .ask(false, &format!("Select wallet [1..{}] or <enter> for '{default}': ", wallets.len()))
                                .await?
                                .trim()
                                .to_string();
                            if selection.is_empty() {
                                Some(default)
                            } else {
                                match selection.parse::<usize>() {
                                    Ok(i) if i >= 1 && i <= wallets.len() => Some(wallets[i - 1].filename.clone()),
                                    _ => {
                                        tprintln!(ctx, "No such wallet: '{selection}'");
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                };

                // Plaintext metadata is readable before the password: apply the
                // wallet's remembered network first, since account activation
                // derives addresses for whatever network is current.
                let meta = match &name {
                    Some(name) => ctx.store().client_metadata(name).await.ok().flatten(),
                    None => None,
                };
                if let Some(network) = meta.as_ref().and_then(|m| m.network.clone()) {
                    if let Ok(network_id) = NetworkId::from_str(&network) {
                        if ctx.wallet().network_id().ok() != Some(network_id) {
                            match ctx.wallet().set_network_id(&network_id) {
                                Ok(_) => tprintln!(ctx, "Network set to {network_id} (remembered by this wallet)"),
                                Err(err) => {
                                    tprintln!(ctx, "This wallet remembers network {network}, which can't be applied now: {err}")
                                }
                            }
                        }
                    }
                }

                let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                let _ = ctx.notifier().show(Notification::Processing).await;
                let args = WalletOpenArgs::default_with_legacy_accounts();
                ctx.wallet().open(&wallet_secret, name.clone(), args, &guard).await?;
                ctx.wallet().activate_accounts(None, &guard).await?;

                if let Some(name) = &name {
                    let remember = meta.as_ref().map(|m| m.remember).unwrap_or(true);
                    if remember {
                        let mut updated = meta.clone().unwrap_or_default();
                        updated.remember = true;
                        updated.network = ctx.wallet().network_id().ok().map(|n| n.to_string());
                        updated.last_opened =
                            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs());
                        ctx.store().set_client_metadata(name, Some(updated.clone())).await.ok();
                        ctx.wallet().settings().set(WalletSettings::Wallet, name.clone()).await.ok();

                        if let Some(server) = &updated.server {
                            if !ctx.wallet().is_connected() {
                                let answer =
                                    ctx.term().ask(false, &format!("Connect to {server}? [Y/n]: ")).await?.trim().to_lowercase();
                                if answer.is_empty() || answer == "y" || answer == "yes" {
                                    ctx.term().exec(format!("connect {server}")).await?;
                                }
                            }
                        }
                    }
                }
            }
            "where" => {
                let folder: String = ctx
                    .wallet()
                    .settings()
                    .get(WalletSettings::Folder)
                    .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
                tprintln!(ctx, "");
                if ctx.wallet().is_open() {
                    if let Some(descriptor) = ctx.store().descriptor() {
                        tprintln!(ctx, "Wallet file:    {folder}/{}.wallet", descriptor.filename);
                        tprintln!(ctx, "Note vault:     {folder}/{}.notes/", descriptor.filename);
                        tprintln!(ctx, "Transactions:   {folder}/{}.transactions/", descriptor.filename);
                    }
                } else {
                    tprintln!(ctx, "Wallet folder:  {folder}  (no wallet open — 'wallet list' shows the files)");
                }
                tprintln!(
                    ctx,
                    "Settings file:  {}/marigold.settings",
                    kaspa_wallet_core::storage::local::default_storage_folder()
                );
                tprintln!(ctx, "");
                tprintln!(ctx, "These files ARE your money and your keys — back them up accordingly.");
                tprintln!(ctx, "");
            }
            "remember" => {
                if !ctx.wallet().is_open() {
                    tprintln!(ctx, "Open a wallet first");
                    return Ok(());
                }
                let Some(descriptor) = ctx.store().descriptor() else {
                    tprintln!(ctx, "Unable to resolve the open wallet's file");
                    return Ok(());
                };
                let arg = argv.first().map(|s| s.to_lowercase());
                match arg.as_deref() {
                    Some("off") => {
                        // Incognito: strip recorded details and stop recording.
                        let meta = kaspa_wallet_core::storage::local::ClientMetadata { remember: false, ..Default::default() };
                        ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                        tprintln!(ctx, "This wallet will no longer record network/server/usage details (stored details removed).");
                    }
                    Some("on") => {
                        let meta = kaspa_wallet_core::storage::local::ClientMetadata {
                            network: ctx.wallet().network_id().ok().map(|n| n.to_string()),
                            server: None,
                            last_opened: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .ok()
                                .map(|d| d.as_secs()),
                            remember: true,
                            hidden: false,
                        };
                        ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                        tprintln!(ctx, "This wallet now remembers its network and connection details.");
                    }
                    _ => {
                        let meta = ctx.store().client_metadata(&descriptor.filename).await.ok().flatten();
                        let state = meta.map(|m| m.remember).unwrap_or(true);
                        tprintln!(ctx, "remember is {} — 'wallet remember on|off' to change", if state { "on" } else { "off" });
                    }
                }
            }
            "rename" => {
                if !ctx.wallet().is_open() {
                    tprintln!(ctx, "Open a wallet first");
                    return Ok(());
                }
                if argv.is_empty() {
                    tprintln!(ctx, "usage: 'wallet rename <new display name>'");
                    tprintln!(ctx, "(the display name is what 'wallet list' and the prompt show; the FILE keeps its name —");
                    tprintln!(ctx, " renaming the file would orphan its note-vault and transaction folders, so that stays manual)");
                    return Ok(());
                }
                let title = argv.join(" ");
                let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                ctx.store().rename(&wallet_secret, Some(&title), None).await?;
                tprintln!(ctx, "Wallet display name is now: {title}");
            }
            "forget" | "show" => {
                // 'forget' hides a wallet from the picker; 'show' brings it
                // back. The wallet itself is untouched — nothing is deleted
                // (that is 'wallet destroy'), and it stays openable by name.
                let hide = op.as_str() == "forget";
                let Some(name) = argv.first().cloned() else {
                    tprintln!(ctx, "usage: 'wallet {} <name>'", op);
                    tprintln!(ctx, "('forget' hides a wallet from the open picker; 'show' brings it back; neither deletes anything)");
                    return Ok(());
                };
                let existing = ctx.store().wallet_list().await?;
                if !existing.iter().any(|w| w.filename == name) {
                    tprintln!(ctx, "No wallet named '{name}' found");
                    return Ok(());
                }
                let mut meta = ctx.store().client_metadata(&name).await.ok().flatten().unwrap_or_default();
                meta.hidden = hide;
                ctx.store().set_client_metadata(&name, Some(meta)).await?;
                if hide {
                    tprintln!(ctx, "'{name}' is hidden from the picker. It still exists — 'open {name}' opens it, 'wallet show {name}' unhides it.");
                } else {
                    tprintln!(ctx, "'{name}' will appear in the picker again.");
                }
            }
            "hint" => {
                if !argv.is_empty() {
                    let re = regex::Regex::new(r"wallet\s+hint\s+").unwrap();
                    let hint = re.replace(cmd, "");
                    let hint = hint.trim();
                    let store = ctx.store();
                    if hint == "remove" {
                        tprintln!(ctx, "Hint is empty - removing wallet hint");
                        store.set_user_hint(None).await?;
                    } else {
                        store.set_user_hint(Some(hint.into())).await?;
                    }
                } else {
                    tprintln!(ctx, "usage:\n'wallet hint <text>' or 'wallet hint remove' to remove the hint");
                }
            }
            "help" => {
                return self.display_help(ctx, argv).await;
            }
            v => {
                tprintln!(ctx, "unknown command: '{v}'");
                return self.display_help(ctx, argv).await;
            }
        }

        Ok(())
    }

    async fn display_help(self: Arc<Self>, ctx: Arc<KaspaCli>, _argv: Vec<String>) -> Result<()> {
        ctx.term().help(
            &[
                ("list", "List available local wallet files"),
                ("create [<name>]", "Create a new bip32 wallet"),
                ("import [<name>]", "Create a wallet from an existing mnemonic (bip32 only)"),
                ("open [<name>]", "Open an existing wallet (shorthand: 'open [<name>]'; no name shows a picker)"),
                ("close", "Close an opened wallet (shorthand: 'close')"),
                ("where", "Show where the wallet, note vault, and settings files live on disk"),
                ("rename <name>", "Change the wallet's display name (the on-disk file name is unchanged)"),
                ("remember [on|off]", "Whether this wallet records its network/server/last-used details (in the wallet file)"),
                ("forget <name>", "Hide a wallet from the open picker (it is NOT deleted; 'wallet show <name>' undoes it)"),
                ("show <name>", "Un-hide a wallet previously hidden with 'wallet forget'"),
                ("destroy <name> [force]", "Permanently delete a wallet (refuses while it holds live notes, unless forced)"),
                ("hint", "Change the wallet phishing hint"),
            ],
            None,
        )?;

        Ok(())
    }
}
