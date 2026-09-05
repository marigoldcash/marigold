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
                        if let Some(title) = wallet.title {
                            tprintln!(ctx, "  {}: {}", wallet.filename, title);
                        } else {
                            tprintln!(ctx, "  {}", wallet.filename);
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
                    let wallets = ctx.store().wallet_list().await?;
                    match wallets.len() {
                        0 => {
                            tprintln!(ctx, "No wallets found — create one with 'wallet create <name>'");
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
            "forget" => {
                if !ctx.wallet().is_open() {
                    tprintln!(ctx, "Open a wallet first");
                    return Ok(());
                }
                let Some(descriptor) = ctx.store().descriptor() else {
                    tprintln!(ctx, "Unable to resolve the open wallet's file");
                    return Ok(());
                };
                ctx.store().set_client_metadata(&descriptor.filename, None).await?;
                ctx.wallet().settings().set(WalletSettings::Wallet, "marigold".to_string()).await.ok();
                tprintln!(ctx, "Stored network/server/usage details cleared for this wallet.");
            }
            "close" => {
                ctx.wallet().close().await?;
            }
            "destroy" => {
                let Some(name) = argv.first().cloned() else {
                    tprintln!(ctx, "usage: 'wallet destroy <name> [force]' — permanently deletes a wallet's file, note vault, and transaction history");
                    return Ok(());
                };
                let force = argv.get(1).map(|s| s.to_lowercase()).as_deref() == Some("force");
                if ctx.wallet().is_open() && ctx.store().descriptor().map(|d| d.filename) == Some(name.clone()) {
                    tprintln!(ctx, "'{name}' is currently open — 'close' it first");
                    return Ok(());
                }
                use kaspa_wallet_core::storage::local::notevault::NoteVault as DestroyVault;
                use kaspa_wallet_core::storage::local::{Storage, WalletStorage, wallet_file_name};
                let folder: String = ctx
                    .wallet()
                    .settings()
                    .get(WalletSettings::Folder)
                    .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
                let storage = Storage::try_new_with_folder(&folder, &wallet_file_name(&name))?;
                let target = match WalletStorage::try_load(&storage).await {
                    Ok(target) => target,
                    Err(_) => {
                        tprintln!(ctx, "No wallet named '{name}' found");
                        return Ok(());
                    }
                };
                // Ownership proof: only someone holding the password may shred it.
                let secret =
                    Secret::new(ctx.term().ask(true, &format!("Enter the password for '{name}': ")).await?.trim().as_bytes().to_vec());
                if target.payload(&secret).is_err() {
                    tprintln!(ctx, "Unable to decrypt '{name}' with that password — nothing was destroyed.");
                    return Ok(());
                }
                // Count what dies with it — and verify it against the chain,
                // which requires a connection: without one we can't tell a
                // spendable note from a stale tombstone-to-be.
                if !ctx.wallet().is_connected() && !force {
                    tprintln!(ctx, "Not connected — cannot verify this wallet's notes against the chain.");
                    tprintln!(ctx, "'connect' first, or use 'wallet destroy {name} force' to destroy without verification.");
                    return Ok(());
                }
                let vault = DestroyVault::new(&folder, &name);
                let mut active_notes = 0usize;
                let mut active_petals = 0u64;
                if vault.exists().await? {
                    let mut serials = Vec::new();
                    let mut by_serial = std::collections::HashMap::new();
                    let mut stream = vault.iter().await?;
                    while let Some(info) = stream.try_next().await? {
                        if info.status == kaspa_wallet_core::storage::NoteStatus::Active {
                            serials.push(info.sn);
                            by_serial.insert(info.sn, info.d);
                        }
                    }
                    if ctx.wallet().is_connected() && !serials.is_empty() {
                        // Only chain-live serials count — the vault may hold
                        // rows the chain has already retired.
                        let on_chain = ctx.wallet().rpc_api().get_notes_by_serial(serials).await?;
                        for entry in on_chain {
                            if let Some(d) = by_serial.get(&entry.sn) {
                                active_notes += 1;
                                active_petals += kaspa_consensus_core::notepool::DENOMINATION_PETALS[*d as usize];
                            }
                        }
                    } else {
                        for d in by_serial.values() {
                            active_notes += 1;
                            active_petals += kaspa_consensus_core::notepool::DENOMINATION_PETALS[*d as usize];
                        }
                    }
                }
                if active_notes > 0 && !force {
                    tprintln!(ctx, "");
                    tprintln!(
                        ctx,
                        "{}",
                        style(format!(
                            "Refusing: '{name}' still holds {active_notes} spendable note(s) worth {} MAGLD.",
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(active_petals)
                        ))
                        .red()
                    );
                    tprintln!(ctx, "Open it and 'note move' them to another wallet (or 'note redeem' them), then destroy.");
                    tprintln!(ctx, "Or 'wallet destroy {name} force' to burn them forever.\r\n");
                    return Ok(());
                }
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style(format!("About to permanently destroy '{name}':")).red());
                tprintln!(ctx, "  wallet file, note vault, and transaction history — deleted from disk");
                if active_notes > 0 {
                    tprintln!(
                        ctx,
                        "{}",
                        style(format!(
                            "  ⚠ its vault still holds {active_notes} ACTIVE note(s) worth {} MAGLD — without a backup, NOBODY can ever spend them again",
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(active_petals)
                        ))
                        .red()
                    );
                    tprintln!(ctx, "    ('note move' them to another wallet first if you want to keep them)");
                }
                tprintln!(ctx, "  any LEDGER balance stays recoverable only through this wallet's 12-word account mnemonic");
                tprintln!(ctx, "");
                let confirm = ctx.term().ask(false, &format!("Type the wallet name ('{name}') to confirm destruction: ")).await?.trim().to_string();
                if confirm != name {
                    tprintln!(ctx, "Confirmation did not match — nothing was destroyed.");
                    return Ok(());
                }
                let base = workflow_store::fs::resolve_path(&folder).map_err(|e| Error::custom(e.to_string()))?;
                std::fs::remove_file(base.join(wallet_file_name(&name))).map_err(|e| Error::custom(e.to_string()))?;
                for suffix in [".notes", ".transactions"] {
                    let dir = base.join(format!("{name}{suffix}"));
                    if dir.exists() {
                        std::fs::remove_dir_all(&dir).map_err(|e| Error::custom(e.to_string()))?;
                    }
                }
                let last: Option<String> = ctx.wallet().settings().get(WalletSettings::Wallet);
                if last.as_deref() == Some(name.as_str()) {
                    ctx.wallet().settings().set(WalletSettings::Wallet, "marigold".to_string()).await.ok();
                }
                tprintln!(ctx, "'{name}' destroyed.");
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
                ("forget", "Clear this wallet's recorded network/server/usage details"),
                ("destroy <name> [force]", "Permanently delete a wallet (refuses while it holds live notes, unless forced)"),
                ("hint", "Change the wallet phishing hint"),
            ],
            None,
        )?;

        Ok(())
    }
}
