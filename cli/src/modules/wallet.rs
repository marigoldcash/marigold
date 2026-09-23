use crate::imports::*;
use crate::wizards;
use kaspa_wallet_core::wallet::WalletGuard;
use std::str::FromStr;

#[derive(Default, Handler)]
#[help("Wallet management operations")]
pub struct Wallet;

/// Written into a restored wallet's vault by 'wallet restore'; removed once
/// the first open with a node has rotated every note.
const ROTATE_ON_OPEN: &str = "rotate-on-open";

impl Wallet {
    /// The restore marker of the open wallet, if it has one.
    fn rotate_on_open_marker(ctx: &Arc<KaspaCli>) -> Option<std::path::PathBuf> {
        let descriptor = ctx.wallet().store().descriptor()?;
        let folder: String = ctx
            .wallet()
            .settings()
            .get(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
        let folder = workflow_store::fs::resolve_path(&folder).ok()?;
        let marker =
            folder.join(kaspa_wallet_core::storage::local::wallet_dir_name(&descriptor.filename)).join("notes").join(ROTATE_ON_OPEN);
        marker.exists().then_some(marker)
    }

    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, mut argv: Vec<String>, cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let ticker = ctx.ticker();

        let guard = ctx.wallet().guard();
        let guard = guard.lock().await;

        if argv.is_empty() {
            return self.display_help(ctx, argv).await;
        }

        let op = argv.remove(0);
        match op.as_str() {
            "list" => {
                // A permission error names the folder it happened in. In a
                // container that folder is a mount, and "Permission denied"
                // on its own sent a tester looking at the wrong thing.
                let wallets = match ctx.store().wallet_list().await {
                    Ok(wallets) => wallets,
                    Err(err) => {
                        let folder: String = ctx
                            .wallet()
                            .settings()
                            .get(WalletSettings::Folder)
                            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
                        tprintln!(ctx, "Could not read the wallet folder {folder}: {err}");
                        return Ok(());
                    }
                };
                if wallets.is_empty() {
                    tprintln!(ctx, "No wallets found");
                } else {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Wallets:");
                    tprintln!(ctx, "");
                    for wallet in wallets {
                        let hidden =
                            ctx.store().client_metadata(&wallet.filename).await.ok().flatten().map(|m| m.hidden).unwrap_or(false);
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
                            // Someone who types 'open' with no wallet is at the
                            // very start, and the useful thing is to take them
                            // to the next step rather than name a command and
                            // leave them to type it.
                            tprintln!(ctx, "");
                            tprintln!(ctx, "You do not have a wallet yet.");
                            tprintln!(ctx, "");
                            let answer = ctx.term().ask(false, "Create one now? [Y/n]: ").await?.trim().to_lowercase();
                            if answer.starts_with('n') {
                                tprintln!(ctx, "");
                                tprintln!(ctx, "Nothing opened. 'wallet create' when you are ready.");
                                tprintln!(
                                    ctx,
                                    "{}",
                                    style("(a wallet you have hidden with 'wallet forget' still shows in 'wallet list')").dim()
                                );
                                tprintln!(ctx, "");
                                return Ok(());
                            }
                            tprintln!(ctx, "");
                            return wizards::wallet::create(&ctx, guard.into(), None, false).await;
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
                                .ask_digits(&format!("Select wallet [1..{}] or <enter> for '{default}': ", wallets.len()))
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

                // Two programs on one wallet is refused, and it is known from
                // the lock on the file before any password — so say it now,
                // not after asking for one (founder, 2026-09-17).
                if let Some(name) = &name {
                    use kaspa_wallet_core::storage::local::{Storage, wallet_file_name};
                    let folder: String = ctx
                        .wallet()
                        .settings()
                        .get(WalletSettings::Folder)
                        .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
                    if let Ok(storage) = Storage::try_new_with_folder(&folder, &wallet_file_name(name)) {
                        // The same goes for a wallet that is not there at all:
                        // a tester typed a name that did not exist and was asked
                        // for a password first, then told "No wallet named
                        // 'destini.wallet' found" (2026-09-17). Say it before.
                        if !storage.exists_sync().unwrap_or(true) {
                            tprintln!(ctx, "");
                            tprintln!(ctx, "No wallet named '{name}' in {folder}. 'wallet list' shows the ones that are there.");
                            tprintln!(ctx, "");
                            return Ok(());
                        }
                        if kaspa_wallet_core::storage::local::interface::wallet_is_open_elsewhere(storage.filename()) {
                            tprintln!(ctx, "");
                            tprintln!(ctx, "'{name}' is open in another Marigold program on this machine. Close it there first.");
                            tprintln!(ctx, "");
                            return Ok(());
                        }
                    }
                }

                // Plaintext metadata is readable before the password: apply the
                // wallet's remembered network first, since account activation
                // derives addresses for whatever network is current.
                let meta = match &name {
                    Some(name) => ctx.store().client_metadata(name).await.ok().flatten(),
                    None => None,
                };
                if let Some(network) = meta.as_ref().and_then(|m| m.network.clone())
                    && let Ok(network_id) = NetworkId::from_str(&network)
                    && ctx.wallet().network_id().ok() != Some(network_id)
                {
                    match ctx.wallet().set_network_id(&network_id) {
                        Ok(_) => tprintln!(ctx, "Network set to {network_id} (remembered by this wallet)"),
                        Err(err) => {
                            tprintln!(ctx, "This wallet remembers network {network}, which can't be applied now: {err}")
                        }
                    }
                }

                let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                let _ = ctx.notifier().show(Notification::Processing).await;
                let args = WalletOpenArgs::default_with_legacy_accounts();
                ctx.wallet().open(&wallet_secret, name.clone(), args, &guard).await?;
                ctx.wallet().activate_accounts(None, &guard).await?;
                // The phone, if one is paired: answered from here, with the
                // password just typed, until 'close'.
                ctx.start_telegram_bot(wallet_secret.clone()).await;

                // Automation defaults ON: the ledger is plumbing, and a
                // person should not have to learn about it. Both arm with the
                // password just typed — no second prompt, nothing persisted.
                // On unless the user has explicitly configured otherwise.
                // A bip39 passphrase on the account key can't be guessed from
                // the wallet password, and asking for a second password at
                // every open would tax the majority who have none. Those
                // wallets arm through 'auto on' instead, which asks for both.
                let needs_passphrase = match ctx.wallet().account() {
                    Ok(account) => ctx.wallet().is_account_key_encrypted(&account).await.ok().flatten().unwrap_or(false),
                    Err(_) => false,
                };
                let configured = meta.as_ref().map(|m| m.auto_configured).unwrap_or(false);
                let auto_mint_on = if configured { meta.as_ref().map(|m| m.auto_mint).unwrap_or(true) } else { true };
                let auto_sweep_on = if configured { meta.as_ref().map(|m| m.auto_sweep).unwrap_or(true) } else { true };
                if needs_passphrase && (auto_mint_on || auto_sweep_on) {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "This account's key has its own passphrase, so the automatic housekeeping");
                    tprintln!(ctx, "cannot sign on its own. Run 'auto on' to turn it on for this session.");
                }
                // Whatever the automation settings, tidying this session — a
                // 'sweep' or 'mint' typed by hand — does not ask for the
                // password again; only a key with its own passphrase does.
                if !needs_passphrase {
                    ctx.hold_tidying_secret(wallet_secret.clone());
                }
                if auto_sweep_on && !needs_passphrase {
                    let threshold = meta
                        .as_ref()
                        .map(|m| m.auto_sweep_utxo_threshold)
                        .filter(|t| *t > 0)
                        .unwrap_or(crate::modules::auto::DEFAULT_SWEEP_UTXOS);
                    ctx.arm_auto_sweep(wallet_secret.clone(), None, threshold);
                }
                if auto_mint_on && !needs_passphrase {
                    let threshold = meta.as_ref().map(|m| m.auto_mint_threshold_petals).filter(|t| *t > 0).unwrap_or(100_000_000);
                    ctx.arm_auto_mint(wallet_secret.clone(), None, threshold);
                }

                // A wallet restored from a backup rotates every note to fresh
                // keys on its first open with a node; the marker is removed
                // once that has been done, so it asks again if it could not.
                if let Some(marker) = Self::rotate_on_open_marker(&ctx) {
                    if ctx.wallet().is_connected() && ctx.wallet().utxo_processor().is_synced() {
                        tprintln!(ctx, "");
                        tpara!(
                            ctx,
                            "This wallet was restored from a backup. Every note is now rotated to fresh keys, so no other copy of that backup can spend them. This costs the network fee per group of notes."
                        );
                        let store = ctx.wallet().store().as_note_key_store()?;
                        let mut serials = Vec::new();
                        let mut stream = store.iter().await?;
                        while let Some(info) = stream.try_next().await? {
                            if info.status == kaspa_wallet_core::storage::NoteStatus::Active {
                                serials.push(info.sn);
                            }
                        }
                        if serials.is_empty() {
                            tprintln!(ctx, "No notes to rotate.");
                        } else {
                            crate::modules::note::Note.rotate_serials(&ctx, &ctx.wallet(), &wallet_secret, serials).await?;
                        }
                        std::fs::remove_file(&marker).ok();
                        tprintln!(ctx, "");
                    } else {
                        tprintln!(ctx, "");
                        tprintln!(
                            ctx,
                            "{}",
                            style("This wallet was restored from a backup. Its notes are rotated to fresh keys the first time it opens with a synced node; until then any other copy of the backup can spend them.").yellow()
                        );
                    }
                }

                // Show what is held, do the ledger housekeeping out loud (the
                // backlog can be large if the wallet has been closed a while),
                // then show the result. Sequential by construction.
                if ctx.wallet().is_connected() {
                    // Runs on the first balance event — the moment the wallet
                    // actually knows its coins. Doing it inline raced the
                    // asynchronous account selection and initial scan.
                    ctx.request_open_housekeeping();
                } else {
                    // Offline is a perfectly good state to open in: notes live
                    // in the local vault and can be counted without a node.
                    ctx.report_holdings().await;
                    if ctx.has_ledger_account().await {
                        tprintln!(ctx, "To see whether you hold anything on the ledger, a network connection is needed.");
                    }
                    // One dialog for this decision, not two. 'connect' owns
                    // it — public node first, then the single question about
                    // running your own — and this path just calls that, with
                    // whatever server the wallet remembers. A menu here as
                    // well meant the same choice was asked twice, in two
                    // different shapes, depending on which command you reached
                    // it through.
                    let target = meta
                        .as_ref()
                        .and_then(|m| m.server.clone())
                        .or_else(|| ctx.wallet().settings().get::<String>(WalletSettings::Server))
                        .filter(|server| server != "public" && !crate::modules::connect::is_local_target(server));
                    tprintln!(ctx, "");
                    let answer = ctx.term().ask(false, "Connect now? [Y/n]: ").await?.trim().to_lowercase();
                    if answer.starts_with('n') {
                        tprintln!(ctx, "Not connected. Type 'connect' when you are ready.");
                    } else {
                        match target {
                            Some(server) => ctx.exec_within(&format!("connect {server}")).await?,
                            None => ctx.exec_within("connect").await?,
                        }
                        ctx.request_open_housekeeping();
                    }
                }

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
                    }
                }
            }
            "close" => {
                ctx.disarm_automation();
                ctx.wallet().close().await?;
                tprintln!(ctx, "Wallet closed.");
            }
            "destroy" => {
                let Some(name) = argv.first().cloned() else {
                    tprintln!(
                        ctx,
                        "usage: 'wallet destroy <name> [force]' — permanently deletes a wallet's file, note vault, and transaction history"
                    );
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
                            "Refusing: '{name}' still holds {active_notes} spendable note(s) worth {} {ticker}.",
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
                            "  ⚠ its vault still holds {active_notes} ACTIVE note(s) worth {} {ticker} — without a backup, NOBODY can ever spend them again",
                            kaspa_wallet_core::utils::sompi_to_kaspa_string(active_petals)
                        ))
                        .red()
                    );
                    tprintln!(ctx, "    ('note move' them to another wallet first if you want to keep them)");
                }
                tprintln!(ctx, "  any LEDGER balance stays recoverable only through this wallet's 12-word account mnemonic");
                tprintln!(ctx, "");
                let confirm = ctx
                    .term()
                    .ask(false, &format!("Type the wallet name ('{name}') to confirm destruction: "))
                    .await?
                    .trim()
                    .to_string();
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
            "where" => {
                // Absolute paths, always. "~/.marigold" is a true answer that
                // helps nobody inside a container, where it means a directory
                // in a Docker volume the person has never seen — and this
                // command exists precisely so somebody can find their money.
                let configured: String = ctx
                    .wallet()
                    .settings()
                    .get(WalletSettings::Folder)
                    .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
                let folder = workflow_store::fs::resolve_path(&configured)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| configured.clone());
                let settings_folder = workflow_store::fs::resolve_path(kaspa_wallet_core::storage::local::default_storage_folder())
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| kaspa_wallet_core::storage::local::default_storage_folder().to_string());

                tprintln!(ctx, "");
                if ctx.wallet().is_open() {
                    if let Some(descriptor) = ctx.store().descriptor() {
                        let name = &descriptor.filename;
                        let dir = kaspa_wallet_core::storage::local::wallet_dir_name(name);
                        tprintln!(ctx, "Your wallet:    {}", style(format!("{folder}/{dir}/")).bold());
                        tprintln!(ctx, "");
                        // Padded to the longest of the three, which varies
                        // with the wallet's name.
                        let keys = kaspa_wallet_core::storage::local::keys_file_name(name);
                        let width = keys.len().max("transactions/".len());
                        tprintln!(ctx, "  {:<width$}   your keys, encrypted", keys);
                        tprintln!(ctx, "  {:<width$}   your notes — the money", "notes/");
                        tprintln!(ctx, "  {:<width$}   history only", "transactions/");
                        tprintln!(ctx, "");
                        tprintln!(ctx, "That one directory is the whole wallet. Copy it and you have copied");
                        tprintln!(ctx, "everything.");
                    }
                } else {
                    tprintln!(ctx, "Wallet folder:  {folder}  (no wallet open — 'wallet list' shows the files)");
                }
                tprintln!(ctx, "Settings file:  {settings_folder}/marigold.settings");
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("'wallet backup <file>' packs it into one encrypted file, leaving out").dim());
                tprintln!(ctx, "{}", style("the history, which a restore does not need.").dim());

                // In a container that path is inside the image unless somebody
                // mounted something over it, and saying so is the difference
                // between a person finding their keys and losing them.
                if std::path::Path::new("/.dockerenv").exists() {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "{}", style("This wallet is running inside a container. That path is only real on").yellow());
                    tprintln!(ctx, "{}", style("the host if a folder was mounted over it — check your compose file.").yellow());
                }
                tprintln!(ctx, "");
            }
            "autoconnect" => {
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
                        tprintln!(
                            ctx,
                            "Autoconnect off: this wallet no longer stores its network or node, and won't offer to reconnect (stored details removed)."
                        );
                    }
                    Some("on") => {
                        let meta = kaspa_wallet_core::storage::local::ClientMetadata {
                            network: ctx.wallet().network_id().ok().map(|n| n.to_string()),
                            server: None,
                            last_opened: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs()),
                            remember: true,
                            hidden: false,
                            auto_mint: true,
                            auto_mint_threshold_petals: 100_000_000,
                            auto_sweep: true,
                            auto_sweep_utxo_threshold: 0,
                            auto_configured: false,
                            mined: false,
                        };
                        ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                        tprintln!(
                            ctx,
                            "Autoconnect on: this wallet remembers its network and node, and offers to reconnect when opened."
                        );
                    }
                    _ => {
                        let meta = ctx.store().client_metadata(&descriptor.filename).await.ok().flatten();
                        let state = meta.map(|m| m.remember).unwrap_or(true);
                        tprintln!(
                            ctx,
                            "autoconnect is {} — 'wallet autoconnect on|off' to change (it remembers this wallet's network and node)",
                            if state { "on" } else { "off" }
                        );
                    }
                }
            }
            "rename" => {
                if !ctx.wallet().is_open() {
                    tprintln!(ctx, "Open a wallet first");
                    return Ok(());
                }
                if argv.is_empty() {
                    tprintln!(ctx, "usage: 'wallet rename <new name>'");
                    return Ok(());
                }
                let title = argv.join(" ");
                let Some(descriptor) = ctx.store().descriptor() else {
                    tprintln!(ctx, "Open a wallet first");
                    return Ok(());
                };
                let old_filename = descriptor.filename.clone();

                // The file on disk gets a name derived from the title, the same
                // way 'wallet create' derives it — nobody wants a wallet called
                // "Savings" living in a file called "marigold.wallet".
                let new_filename = title
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
                    .collect::<String>()
                    .trim_matches('-')
                    .to_string();
                if new_filename.is_empty() {
                    tprintln!(ctx, "'{title}' has no letters or digits in it — pick a name that does.");
                    return Ok(());
                }

                let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                ctx.store().rename(&wallet_secret, Some(&title), None).await?;

                if new_filename == old_filename {
                    tprintln!(ctx, "Wallet is now called: {title}");
                    return Ok(());
                }

                let existing = ctx.store().wallet_list().await?;
                if existing.iter().any(|w| w.filename == new_filename) {
                    tprintln!(ctx, "Renamed to '{title}', but the file stays '{old_filename}' — '{new_filename}' is already taken.");
                    return Ok(());
                }

                tprintln!(ctx, "");
                tprintln!(ctx, "Renaming the file closes the wallet — you will need to open it again as '{title}'.");
                let answer = ctx.term().ask(false, "Rename the file too? [Y/n]: ").await?.trim().to_lowercase();
                if answer.starts_with('n') {
                    tprintln!(ctx, "Renamed to '{title}'. The file stays '{old_filename}'.");
                    return Ok(());
                }

                // Close first: the open wallet holds paths to the old file, the
                // old note vault and the old transaction folder, and moving
                // them out from under it would leave every one of those handles
                // pointing at nothing.
                ctx.disarm_automation();
                ctx.wallet().close().await?;
                match ctx.store().rename_storage(&old_filename, &new_filename).await {
                    Ok(()) => {
                        tprintln!(ctx, "");
                        tprintln!(ctx, "Renamed. The wallet, its notes and its history are now '{new_filename}'.");
                        tprintln!(ctx, "Open it with 'open {new_filename}'.");
                    }
                    Err(err) => {
                        // Nothing moved — rename_storage puts back anything it
                        // managed to move before failing.
                        tprintln!(ctx, "");
                        tprintln!(ctx, "Could not rename the file: {err}");
                        tprintln!(
                            ctx,
                            "Nothing was moved. The wallet is still '{old_filename}' — open it with 'open {old_filename}'."
                        );
                    }
                }
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
                    tprintln!(
                        ctx,
                        "'{name}' is hidden from the picker. It still exists — 'open {name}' opens it, 'wallet show {name}' unhides it."
                    );
                } else {
                    tprintln!(ctx, "'{name}' will appear in the picker again.");
                }
            }
            "tidy" => {
                if !ctx.wallet().is_open() {
                    tprintln!(ctx, "Open a wallet first");
                    return Ok(());
                }
                // Keys with no accounts hold nothing and can receive nothing —
                // they are leftovers from an interrupted 'account create'.
                let store = ctx.store().as_prv_key_data_store()?;
                let wallet = ctx.wallet();
                let mut ids = Vec::new();
                let mut stream = store.iter().await?;
                while let Some(info) = stream.try_next().await? {
                    let mut accounts = wallet.accounts(Some(info.id), &guard).await?;
                    if accounts.try_next().await?.is_none() {
                        ids.push(info.id);
                    }
                }
                if ids.is_empty() {
                    tprintln!(ctx, "Nothing to tidy — every recovery key in this wallet is in use.");
                    return Ok(());
                }
                tprintln!(ctx, "");
                tprintln!(ctx, "{} unused recovery key(s) can be removed.", ids.len());
                tprintln!(ctx, "They were created but never used by an account, so they hold no funds and no address can");
                tprintln!(ctx, "have received any. (If you ever imported a mnemonic that failed to finish, you can simply");
                tprintln!(ctx, "import it again.)");
                let confirm = ctx.term().ask(false, "Remove them? (type 'y' to confirm): ").await?.trim().to_lowercase();
                if confirm != "y" {
                    tprintln!(ctx, "Nothing was removed.");
                    return Ok(());
                }
                let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                let mut removed = 0usize;
                for id in ids {
                    if store.remove(&wallet_secret, &id).await.is_ok() {
                        removed += 1;
                    }
                }
                ctx.store().commit(&wallet_secret).await?;
                tprintln!(ctx, "Removed {removed} unused key(s).");
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
            "backup" => {
                if argv.first().map(|s| s.as_str()) == Some("verify") {
                    return self.backup_verify(&ctx, argv[1..].to_vec()).await;
                }
                if argv.first().map(|s| s.as_str()) == Some("telegram") {
                    #[cfg(feature = "embedded-node")]
                    return self.backup_telegram(&ctx, argv[1..].to_vec()).await;
                    #[cfg(not(feature = "embedded-node"))]
                    {
                        tprintln!(ctx, "Telegram backups need the full wallet build.");
                        return Ok(());
                    }
                }
                return self.backup(&ctx, argv).await;
            }
            "restore" => {
                if argv.first().map(|s| s.as_str()) == Some("telegram") {
                    #[cfg(feature = "embedded-node")]
                    return self.restore_telegram(&ctx, argv[1..].to_vec(), &guard).await;
                    #[cfg(not(feature = "embedded-node"))]
                    {
                        tprintln!(ctx, "Telegram backups need the full wallet build.");
                        return Ok(());
                    }
                }
                return self.restore(&ctx, argv, &guard).await;
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
                ("create [<name>]", "Create a new wallet"),
                ("import [<name>]", "Create a wallet from an existing mnemonic (bip32 only)"),
                ("open [<name>]", "Open an existing wallet (shorthand: 'open [<name>]'; no name shows a picker)"),
                ("close", "Close an opened wallet (shorthand: 'close')"),
                ("where", "Show where the wallet, note vault, and settings files live on disk"),
                ("destroy <name> [force]", "Permanently delete a wallet (refuses while it holds live notes, unless forced)"),
                ("rename <name>", "Rename the wallet, file and all"),
                ("autoconnect [on|off]", "Whether this wallet remembers its network and node, and offers to reconnect when opened"),
                ("forget <name>", "Hide a wallet from the open picker (it is NOT deleted; 'wallet show <name>' undoes it)"),
                ("show <name>", "Un-hide a wallet previously hidden with 'wallet forget'"),
                ("hint", "Change the wallet phishing hint"),
                ("backup [<file-or-folder>]", "Write the whole wallet — keys, notes and all — to one encrypted file"),
                ("backup verify <file>", "Check that a backup file still opens and what is inside it"),
                ("restore <file> [<name>]", "Rebuild a wallet from a backup file"),
            ],
            None,
        )?;

        Ok(())
    }

    /// `wallet backup [<file-or-folder>]` — the whole wallet in one encrypted
    /// file: the wallet file, the vault key, every note key, the manifest.
    ///
    /// This is the copy you can put somewhere you do not control. `note vault
    /// backup` writes the vault as loose files, which is right for a USB stick
    /// in a drawer and wrong for anywhere else: `manifest.tsv` is plaintext and
    /// lists every note you hold. The archive passphrase is what makes the
    /// difference, which is why this command insists on one and will not reuse
    /// the wallet password for it.
    async fn backup(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        use crate::backup as archive;

        if !ctx.wallet().is_open() {
            tprintln!(ctx, "Open a wallet first — 'wallet backup' backs up the wallet you have open.");
            return Ok(());
        }
        let descriptor = ctx.store().descriptor().ok_or_else(|| Error::custom("no wallet is open"))?;
        let name = descriptor.filename.clone();

        // vault_folder() is the resolved on-disk path, so its parent is the
        // real storage folder — no second guess at where '~' points.
        // <storage>/<name>.wallet/notes -> <storage>/<name>.wallet
        let vault_folder = ctx.wallet().store().as_note_key_store()?.vault_folder().await?;
        let wallet_dir = vault_folder.parent().ok_or_else(|| Error::custom("cannot work out the wallet folder"))?.to_path_buf();
        let wallet_file = wallet_dir.join(kaspa_wallet_core::storage::local::keys_file_name(&name));
        if !wallet_file.exists() {
            return Err(Error::custom(format!("{} is missing — nothing to back up", wallet_file.display())));
        }

        let target = Self::backup_target(argv.first().map(|s| s.as_str()))?;
        if target.exists() {
            tprintln!(ctx, "{} already exists. Choose another name — a backup never overwrites one.", target.display());
            return Ok(());
        }

        tprintln!(ctx, "");
        tpara!(
            ctx,
            "This writes your whole wallet — the keys, every note, the lot — into one file, \
            encrypted under a passphrase you choose now. It is safe to keep somewhere you do \
            not control: a cloud drive, a chat with yourself, a stranger's USB stick. \
            "
        );
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "Choose a passphrase you do not use anywhere else, and write it down. Nobody can \
            reset it and nobody keeps a copy: lose it and this file is noise, however much \
            money it holds. \
            "
        );
        tprintln!(ctx, "");

        let pass = ctx.term().ask(true, "Passphrase for this backup: ").await?.trim().to_string();
        if pass.is_empty() {
            tprintln!(ctx, "No passphrase — nothing written.");
            return Ok(());
        }
        // Short enough to brute-force is the same as no passphrase, for a file
        // whose whole purpose is to sit somewhere you do not control.
        if pass.len() < 8 {
            tprintln!(ctx, "That is under 8 characters. This file may sit on someone else's server — nothing written.");
            return Ok(());
        }
        let again = ctx.term().ask(true, "Again: ").await?.trim().to_string();
        if pass != again {
            tprintln!(ctx, "Those did not match — nothing written.");
            return Ok(());
        }
        let passphrase = Secret::from(pass.as_bytes().to_vec());
        let (entries, packed) = Self::pack_wallet(&name, &wallet_file, &vault_folder, &passphrase)?;
        let file_count = entries.len();
        Self::write_private(&target, &packed)?;

        // Read it back and open it. A backup that was never opened is a guess,
        // and this is the cheapest moment to find out it is a bad one.
        let reread = archive::read_file(&target)?;
        let restored = archive::unpack(&reread, &passphrase)?;
        if restored.len() != file_count {
            return Err(Error::custom("the backup did not read back correctly — do not rely on it"));
        }
        for (a, b) in entries.iter().zip(restored.iter()) {
            if a.path != b.path || a.data != b.data {
                return Err(Error::custom("the backup did not read back correctly — do not rely on it"));
            }
        }

        let (active, retired) = Self::note_counts(&entries);
        tprintln!(ctx, "");
        tprintln!(ctx, "Wrote {}", style(target.display().to_string()).bold());
        tprintln!(ctx, "{} files, {} — opened again to check it.", file_count.separated_string(), archive::human_size(packed.len()));
        tprintln!(ctx, "{} note keys you can spend, {} retired.", active.separated_string(), retired.separated_string());
        if retired > active.saturating_mul(4) {
            tprintln!(ctx, "");
            // Retired keys are the bulk of every mature vault, and leaving them
            // out is not an option: a note is marked retired when its spending
            // transaction is SENT, not when it lands, and nothing ever marks one
            // back. A backup without them could be a backup without your money.
            tpara!(
                ctx,
                "Most of that is retired notes — spent, but kept, because a note is written off \
                when its payment is sent rather than when it confirms. Dropping them to save \
                space is how a backup quietly stops being one. \
                "
            );
        }
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "Restore it with 'wallet restore <file>' on any machine. It needs this passphrase \
            and nothing else — not your 24 words, not your wallet password, though the wallet \
            password is still what opens the wallet afterwards. \
            "
        );
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style("This one file is enough to spend your money. Treat it as cash.").red());
        tprintln!(ctx, "");
        Ok(())
    }

    /// `wallet backup verify <file>` — open an archive and say what is in it,
    /// without writing anything.
    ///
    /// The point is to be able to answer "is that old file still good?" at a
    /// moment of your choosing rather than the moment you need it.
    async fn backup_verify(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        use crate::backup as archive;

        let Some(path) = argv.first() else {
            tprintln!(ctx, "usage: 'wallet backup verify <file>'");
            return Ok(());
        };
        let path = std::path::PathBuf::from(path);
        let bytes = archive::read_file(&path)?;

        let pass = ctx.term().ask(true, "Passphrase for this backup: ").await?.trim().to_string();
        if pass.is_empty() {
            tprintln!(ctx, "No passphrase — nothing checked.");
            return Ok(());
        }
        let entries = archive::unpack(&bytes, &Secret::from(pass.as_bytes().to_vec()))?;

        let name = Self::wallet_name_in(&entries)?;
        let (active, retired) = Self::note_counts(&entries);
        let total: usize = entries.iter().map(|e| e.data.len()).sum();

        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style("The passphrase is right and the file is intact.").green());
        tprintln!(ctx, "");
        tprintln!(ctx, "  Wallet:  {name}");
        tprintln!(ctx, "  Files:   {}", entries.len().separated_string());
        tprintln!(ctx, "  Notes:   {} spendable, {} retired", active.separated_string(), retired.separated_string());
        tprintln!(ctx, "  Size:    {} on disk, {} inside", archive::human_size(bytes.len()), archive::human_size(total));
        tprintln!(ctx, "");
        tprintln!(ctx, "'wallet restore {}' would rebuild it.", path.display());
        tprintln!(ctx, "");
        Ok(())
    }

    /// `wallet restore <file> [<name>]` — rebuild a wallet from an archive.
    ///
    /// Never overwrites. If anything it would write is already there it writes
    /// nothing at all, and says which file stopped it: restoring an old backup
    /// over a live wallet is the one way this command could cost somebody
    /// money, so it is not possible by accident.
    async fn restore(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>, guard: &WalletGuard<'_>) -> Result<()> {
        use crate::backup as archive;

        let Some(path) = argv.first() else {
            tprintln!(ctx, "usage: 'wallet restore <file> [<name>]'");
            tprintln!(ctx, "(<name> restores it under a different name, so it can sit beside a wallet you already have)");
            return Ok(());
        };
        let path = std::path::PathBuf::from(path);
        let bytes = archive::read_file(&path)?;

        let pass = ctx.term().ask(true, "Passphrase for this backup: ").await?.trim().to_string();
        if pass.is_empty() {
            tprintln!(ctx, "No passphrase — nothing restored.");
            return Ok(());
        }
        let entries = archive::unpack(&bytes, &Secret::from(pass.as_bytes().to_vec()))?;

        let original = Self::wallet_name_in(&entries)?;
        let name = argv.get(1).cloned().unwrap_or_else(|| original.clone());
        if name.to_lowercase() == "wallet" {
            return Err(Error::custom("a wallet cannot be named 'wallet'"));
        }
        // Renaming a wallet is a file move and nothing else — no path is stored
        // inside any of these files — so restoring under a new name is the same
        // operation done a moment earlier.
        let entries = if name == original { entries } else { archive::rename_entries(entries, &original, &name)? };

        let folder: String = ctx
            .wallet()
            .settings()
            .get(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
        let folder = workflow_store::fs::resolve_path(&folder)?;

        let written = archive::extract(&entries, &folder)?;
        // A backup is a copy of the keys, and any other copy of it can spend
        // the same notes. The first open of the restored wallet rotates every
        // note to fresh keys (POOL-SPEC.md P5.6), which needs the wallet open
        // and a node: this marker asks for it (threat pass, 2026-09-20).
        let marker = folder.join(kaspa_wallet_core::storage::local::wallet_dir_name(&name)).join("notes").join(ROTATE_ON_OPEN);
        if let Some(dir) = marker.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(&marker, b"restored from a backup; rotate every note on the first open\n").ok();

        tprintln!(ctx, "");
        tprintln!(ctx, "Restored {written} files into {}", style(folder.display().to_string()).bold());
        tprintln!(
            ctx,
            "When it is next opened with a node, every note is rotated to fresh keys, so no other copy of the backup can spend them."
        );
        if name == original {
            tprintln!(ctx, "");
            tprintln!(ctx, "Open it with 'open {name}' — it wants the wallet password it had when the backup was made.");
            tprintln!(ctx, "");
            return Ok(());
        }

        // The restore itself is finished and on disk. Everything below is
        // tidying: the title lives inside the wallet file under the wallet
        // password, so correcting it needs a second secret and an open wallet.
        // Doing it here rather than as part of the restore means a wrong
        // password, a refusal, or a crash costs nothing — the files are
        // already safe, and 'wallet rename' can finish the job any time.
        tprintln!(ctx, "The backup called it '{original}'; the files are '{name}' here.");
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "Inside, it still calls itself '{original}' — that is the name 'wallet list' shows and the \
            one on the prompt. Correcting it means opening the wallet, which needs the password it had \
            when the backup was made. "
        );
        tprintln!(ctx, "");
        if ctx.wallet().is_open() {
            tprintln!(ctx, "{}", style("This will close the wallet you have open.").dim());
        }
        let answer = ctx.term().ask(false, &format!("Fix the name to '{name}' now? [Y/n]: ")).await?.trim().to_lowercase();
        if answer.starts_with('n') {
            tprintln!(ctx, "");
            tprintln!(ctx, "Left as it is. 'open {name}' works either way; 'wallet rename' fixes the name later.");
            tprintln!(ctx, "");
            return Ok(());
        }

        let secret = Secret::new(ctx.term().ask(true, "Wallet password: ").await?.trim().as_bytes().to_vec());
        if secret.as_ref().is_empty() {
            tprintln!(ctx, "");
            tprintln!(ctx, "No password — left as '{original}'. 'wallet rename' fixes it later.");
            tprintln!(ctx, "");
            return Ok(());
        }

        // Automation holds the previous wallet's keys; disarm before swapping,
        // exactly as 'wallet rename' does.
        ctx.disarm_automation();
        if ctx.wallet().is_open() {
            ctx.wallet().close().await?;
        }
        let opened = ctx.wallet().open(&secret, Some(name.clone()), WalletOpenArgs::default_with_legacy_accounts(), guard).await;
        if let Err(err) = opened {
            tprintln!(ctx, "");
            tprintln!(ctx, "Could not open it: {err}");
            tprintln!(ctx, "The restore stands — the files are in place. 'open {name}' when you have the password.");
            tprintln!(ctx, "");
            return Ok(());
        }
        ctx.wallet().activate_accounts(None, guard).await?;

        match ctx.store().rename(&secret, Some(&name), None).await {
            Ok(()) => {
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style(format!("Restored and open as '{name}'.")).green());
                tprintln!(ctx, "");
            }
            Err(err) => {
                tprintln!(ctx, "");
                tprintln!(ctx, "Opened, but the name inside is still '{original}': {err}");
                tprintln!(ctx, "'wallet rename {name}' fixes it.");
                tprintln!(ctx, "");
            }
        }
        Ok(())
    }

    /// Spendable and retired note keys in an archive, counted by which folder
    /// of the vault they came from. Read off the paths rather than the manifest
    /// so it works on an archive without parsing anything inside it.
    fn note_counts(entries: &[crate::backup::ArchiveEntry]) -> (usize, usize) {
        let mut active = 0;
        let mut retired = 0;
        for entry in entries.iter().filter(|e| e.path.ends_with(".note")) {
            if entry.path.contains("/active/") || entry.path.contains("/mirrored/") {
                active += 1;
            } else {
                retired += 1;
            }
        }
        (active, retired)
    }

    /// The wallet's name, read off the one top-level `.wallet` entry.
    fn wallet_name_in(entries: &[crate::backup::ArchiveEntry]) -> Result<String> {
        // `<name>.wallet/<name>.keys` — one level down, and the directory
        // name is the authority since the keys file is named after it.
        let mut found = entries.iter().filter_map(|e| {
            let (dir, file) = e.path.split_once('/')?;
            let name = dir.strip_suffix(".wallet")?;
            (file == format!("{name}.keys")).then(|| name.to_string())
        });
        let name = found.next().ok_or_else(|| Error::custom("that archive holds no wallet file"))?;
        if found.next().is_some() {
            return Err(Error::custom("that archive holds more than one wallet file"));
        }
        Ok(name)
    }

    /// Work out where to write. A folder gets a dated filename; anything else
    /// is taken literally.
    ///
    /// The default name carries no wallet name, because the filename is the one
    /// part of a backup that whoever stores it can read.
    /// The open wallet — its keys file and every note file — as archive
    /// entries and the sealed archive, checked to read back before anything
    /// is done with it.
    fn pack_wallet(
        name: &str,
        wallet_file: &std::path::Path,
        vault_folder: &std::path::Path,
        passphrase: &Secret,
    ) -> Result<(Vec<crate::backup::ArchiveEntry>, Vec<u8>)> {
        use crate::backup as archive;
        let dir = kaspa_wallet_core::storage::local::wallet_dir_name(name);
        let mut entries = vec![archive::ArchiveEntry {
            path: format!("{dir}/{}", kaspa_wallet_core::storage::local::keys_file_name(name)),
            data: std::fs::read(wallet_file).map_err(|e| Error::custom(format!("cannot read the wallet file: {e}")))?,
        }];
        if vault_folder.exists() {
            archive::collect_tree(vault_folder, &format!("{dir}/notes"), &mut entries)?;
        }
        let packed = archive::pack(&entries, passphrase)?;
        let restored = archive::unpack(&packed, passphrase)?;
        if restored.len() != entries.len() || entries.iter().zip(restored.iter()).any(|(a, b)| a.path != b.path || a.data != b.data) {
            return Err(Error::custom("the backup did not read back correctly — do not rely on it"));
        }
        Ok((entries, packed))
    }

    /// The passphrase for a backup, asked twice, with the same rules as a
    /// file backup: eight characters at least, nothing written on an empty one.
    #[cfg(feature = "embedded-node")]
    async fn ask_backup_passphrase(ctx: &Arc<KaspaCli>) -> Result<Option<Secret>> {
        let pass = ctx.term().ask(true, "Passphrase for this backup: ").await?.trim().to_string();
        if pass.is_empty() {
            tprintln!(ctx, "No passphrase — nothing sent.");
            return Ok(None);
        }
        if pass.len() < 8 {
            tprintln!(ctx, "That is under 8 characters. This will sit on Telegram's servers — nothing sent.");
            return Ok(None);
        }
        let again = ctx.term().ask(true, "Again: ").await?.trim().to_string();
        if pass != again {
            tprintln!(ctx, "Those did not match — nothing sent.");
            return Ok(None);
        }
        Ok(Some(Secret::from(pass.as_bytes().to_vec())))
    }

    /// 'wallet backup telegram [<chat id>]': the same encrypted archive as a
    /// file backup, posted to a private group as parts the bot can read back
    /// (founder, 2026-09-23, after the nightly backups of other systems that
    /// work this way). Needs the bot from 'mobile telegram <token>'; the chat
    /// is given once and kept.
    #[cfg(feature = "embedded-node")]
    async fn backup_telegram(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>) -> Result<()> {
        use crate::backup as archive;
        use crate::telegram::{BACKUP_PART_BYTES, TelegramConfig, part_file_name, send_document, send_plain};
        if !ctx.wallet().is_open() {
            tprintln!(ctx, "Open a wallet first — 'backup telegram' backs up the wallet you have open.");
            return Ok(());
        }
        let descriptor = ctx.store().descriptor().ok_or_else(|| Error::custom("no wallet is open"))?;
        let name = descriptor.filename.clone();
        let folder: String = ctx
            .wallet()
            .settings()
            .get(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
        let cfg_path = TelegramConfig::path(&folder, &name);
        let Some(mut cfg) = TelegramConfig::load(&cfg_path) else {
            tprintln!(
                ctx,
                "No Telegram bot is set up for this wallet. 'mobile telegram <token>' first, with a token from @BotFather."
            );
            return Ok(());
        };
        let chat_id = match argv.first() {
            Some(arg) => {
                let id: i64 = arg.replace(',', "").parse().map_err(|_| Error::custom("the chat id is a number, like -603049415"))?;
                cfg.backup_chat_id = Some(id);
                cfg.save(&cfg_path).map_err(|e| Error::custom(format!("cannot save the bot settings: {e}")))?;
                id
            }
            None => match cfg.backup_chat_id {
                Some(id) => id,
                None => {
                    tprintln!(ctx, "Where to? 'backup telegram <chat id>' the first time: a private group the bot is a member of.");
                    tprintln!(ctx, "{}", crate::ui::dim("The id is shown in the group's info; a group's id is negative."));
                    return Ok(());
                }
            },
        };
        let vault_folder = ctx.wallet().store().as_note_key_store()?.vault_folder().await?;
        let wallet_dir = vault_folder.parent().ok_or_else(|| Error::custom("cannot work out the wallet folder"))?.to_path_buf();
        let wallet_file = wallet_dir.join(kaspa_wallet_core::storage::local::keys_file_name(&name));
        if !wallet_file.exists() {
            return Err(Error::custom(format!("{} is missing — nothing to back up", wallet_file.display())));
        }
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "This posts your whole wallet — the keys, every note, the lot — to that Telegram chat, \
            encrypted under a passphrase you choose now. Telegram keeps the messages; the passphrase \
            is the only thing between them and your money. \
            "
        );
        tprintln!(ctx, "");
        let Some(passphrase) = Self::ask_backup_passphrase(ctx).await? else { return Ok(()) };
        let (entries, packed) = Self::pack_wallet(&name, &wallet_file, &vault_folder, &passphrase)?;
        let digest = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(&packed);
            faster_hex::hex_string(&h.finalize())
        };
        let stamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
        let backup = format!("marigold-{name}-{stamp}.mgb");
        let parts: Vec<&[u8]> = packed.chunks(BACKUP_PART_BYTES).collect();
        let count = parts.len();
        let token = cfg.token.clone();
        tprintln!(
            ctx,
            "Sending {} ({} files, {}) as {} part(s)…",
            backup,
            entries.len().separated_string(),
            archive::human_size(packed.len()),
            count
        );
        send_plain(
            &token,
            chat_id,
            &format!("----- Marigold backup {backup}: {count} part(s), {} bytes, sha256 {digest}", packed.len()),
        )
        .await
        .map_err(Error::custom)?;
        for (i, chunk) in parts.iter().enumerate() {
            let index = i + 1;
            let file_name = part_file_name(&backup, index, count);
            let caption = format!("Part {index} of {count} of {backup} · sha256 {}…", &digest[..16]);
            send_document(&token, chat_id, &file_name, chunk.to_vec(), &caption).await.map_err(Error::custom)?;
            tprintln!(ctx, "  part {index} of {count} sent ({})", archive::human_size(chunk.len()));
        }
        send_plain(&token, chat_id, &format!("----- End of Marigold backup {backup}")).await.map_err(Error::custom)?;
        let (active, retired) = Self::note_counts(&entries);
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style(format!("Sent {backup}: {count} part(s), {}.", archive::human_size(packed.len()))).green());
        tprintln!(ctx, "{} note keys you can spend, {} retired.", active.separated_string(), retired.separated_string());
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "To bring it back on any machine: 'wallet restore telegram <bot token>', then forward the \
            part messages from that chat to the bot. It needs this passphrase and nothing else. \
            "
        );
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style("Those messages are enough to spend your money. Keep that chat private.").red());
        tprintln!(ctx, "");
        Ok(())
    }

    /// 'wallet restore telegram <bot token> [<name>]': collects the parts of
    /// one backup forwarded to the bot and restores the wallet from them.
    #[cfg(feature = "embedded-node")]
    async fn restore_telegram(&self, ctx: &Arc<KaspaCli>, argv: Vec<String>, guard: &WalletGuard<'_>) -> Result<()> {
        use crate::backup as archive;
        use crate::telegram::{collect_backup_parts, download_file};
        let Some(token) = argv.first().cloned() else {
            tprintln!(ctx, "usage: 'wallet restore telegram <bot token> [<name>]'");
            tprintln!(
                ctx,
                "{}",
                crate::ui::dim("The token of the bot the backup was sent with, from @BotFather; <name> restores under another name.")
            );
            return Ok(());
        };
        let new_name = argv.get(1).cloned();
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "Now forward the backup's part messages from the backup chat to the bot (select them all, \
            forward, pick the bot). The wallet waits up to ten minutes for all of them. \
            "
        );
        tprintln!(ctx, "");
        let ctx_ = ctx.clone();
        let progress = move |line: String| tprintln!(ctx_, "  {line}");
        let parts =
            collect_backup_parts(&token, None, None, std::time::Duration::from_secs(600), &progress).await.map_err(Error::custom)?;
        let backup = parts[0].backup.clone();
        let mut bytes = Vec::new();
        for part in &parts {
            tprintln!(ctx, "  fetching part {} of {}…", part.index, part.count);
            bytes.extend(download_file(&token, &part.file_id).await.map_err(Error::custom)?);
        }
        tprintln!(ctx, "Received {} ({}).", backup, archive::human_size(bytes.len()));
        let tmp = std::env::temp_dir().join(format!("{backup}.restore"));
        crate::backup::write_owner_only(&tmp, &bytes).map_err(|e| Error::custom(format!("cannot write the archive: {e}")))?;
        let mut restore_args = vec![tmp.to_string_lossy().to_string()];
        if let Some(name) = new_name {
            restore_args.push(name);
        }
        let outcome = self.restore(ctx, restore_args, guard).await;
        let _ = std::fs::remove_file(&tmp);
        outcome
    }

    fn backup_target(arg: Option<&str>) -> Result<std::path::PathBuf> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M").to_string();
        let generated = format!("marigold-backup-{stamp}.mgb");
        Ok(match arg {
            None => std::path::PathBuf::from(generated),
            Some(arg) => {
                let path = std::path::PathBuf::from(arg);
                if path.is_dir() { path.join(generated) } else { path }
            }
        })
    }

    /// Write with owner-only permissions from the start, rather than creating
    /// a world-readable file and narrowing it afterwards.
    fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|e| Error::custom(format!("cannot create {}: {e}", path.display())))?;
        file.write_all(bytes).map_err(|e| Error::custom(format!("cannot write {}: {e}", path.display())))?;
        file.sync_all().map_err(|e| Error::custom(format!("cannot flush {}: {e}", path.display())))?;
        Ok(())
    }
}
