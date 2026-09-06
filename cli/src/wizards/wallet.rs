use crate::cli::KaspaCli;
use crate::imports::*;
use crate::result::Result;
use kaspa_bip32::{Language, Mnemonic, WordCount};
use kaspa_wallet_core::storage::keydata::PrvKeyDataVariantKind;
use kaspa_wallet_core::{
    storage::{Hint, make_filename},
    wallet::WalletGuard,
};

pub(crate) async fn create(
    ctx: &Arc<KaspaCli>,
    wallet_guard: Option<WalletGuard<'_>>,
    name: Option<&str>,
    import_with_mnemonic: bool,
) -> Result<()> {
    let term = ctx.term();
    let wallet = ctx.wallet();
    let local_guard = ctx.wallet().guard();

    let guard = match wallet_guard {
        Some(locked_guard) => locked_guard,
        None => local_guard.lock().await,
    };
    // TODO @aspect
    let word_count = WordCount::Words12;

    if let Err(err) = wallet.network_id() {
        tprintln!(ctx);
        tprintln!(ctx, "Before creating a wallet, you need to select a Marigold network.");
        tprintln!(ctx, "Please use 'network <name>' command to select a network.");
        tprintln!(ctx, "Available networks: {}", kaspa_consensus_core::network::NetworkId::supported_list());
        tprintln!(ctx);
        return Err(err.into());
    }
    // Storage-location step: the wallet file is the user's money — its
    // location must be proposed up front, not revealed after the fact
    // (wallet-UX refinements, 2026-09-05).
    let mut name: Option<String> = name.map(String::from);
    let folder: String = ctx
        .wallet()
        .settings()
        .get(WalletSettings::Folder)
        .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
    let mut custom_filename: Option<String> = None;
    loop {
        let file = match &custom_filename {
            Some(file) => kaspa_wallet_core::storage::local::wallet_file_name(file),
            None => kaspa_wallet_core::storage::local::wallet_file_name(&make_filename(&name, &None)),
        };
        tprintln!(ctx);
        tprintln!(ctx, "This wallet will be stored as: {}", style(format!("{folder}/{file}")).cyan());
        tprintln!(ctx, "(change the folder for all wallets with 'settings set folder <path>' before creating)");
        let input = term.ask(false, "Press <enter> to accept, or type a different wallet name: ").await?.trim().to_string();
        if input.is_empty() {
            break;
        }
        if input.to_lowercase() == "wallet" {
            tprintln!(ctx, "Wallet name cannot be 'wallet'");
            continue;
        }
        if input.contains('.') {
            // A name with an extension is used verbatim — powerful, with sharp edges.
            tprintln!(ctx);
            tprintln!(ctx, "{}", style("Custom extension — two things to know:").yellow());
            tprintln!(ctx, "  1. Files without the .wallet extension do NOT appear in 'wallet list' or the");
            tprintln!(ctx, "     open picker — you must remember to 'open {input}' by name.");
            tprintln!(ctx, "  2. Other programs may claim the extension: double-clicking a wallet named");
            tprintln!(ctx, "     notes.doc opens a word processor, not your money. Oops-resistant it is not.");
            let keep = term.ask(false, "Keep this file name anyway? (type 'y' to keep): ").await?.trim().to_lowercase();
            if keep != "y" {
                continue;
            }
            custom_filename = Some(input.clone());
            name = Some(input);
        } else {
            custom_filename = None;
            name = Some(input);
        }
    }
    let name = name.as_deref();

    let filename = make_filename(&name.map(String::from), &custom_filename);
    if wallet.exists(Some(&filename)).await? {
        tprintln!(ctx, "{}", style("WARNING - A previously created wallet already exists!").red().to_string());
        tprintln!(ctx, "NOTE: You can create a differently named wallet by using 'wallet create <name>'");
        tprintln!(ctx);

        let overwrite =
            term.ask(false, "Are you sure you want to overwrite it (type 'y' to approve)?: ").await?.trim().to_string().to_lowercase();
        if overwrite.ne("y") {
            return Ok(());
        }
    }

    let account_name = term.ask(false, "Default account title: ").await?.trim().to_string();
    let account_name = account_name.is_not_empty().then_some(account_name);

    tpara!(
        ctx,
        "\n\
        \"Phishing hint\" is a secret word or a phrase that is displayed \
        when you open your wallet. If you do not see the hint when opening \
        your wallet, you may be accessing a fake wallet designed to steal \
        your private key. If this occurs, stop using the wallet immediately, \
        check the browser URL domain name and seek help on social networks \
        (the official Marigold channels). \
        \n\
        ",
    );

    let hint = term.ask(false, "Create phishing hint (optional, press <enter> to skip): ").await?.trim().to_string();
    let hint = hint.is_not_empty().then_some(hint).map(Hint::from);
    //if hint.is_empty() { None } else { Some(hint) };

    let wallet_secret = Secret::new(term.ask(true, "Enter wallet encryption password: ").await?.trim().as_bytes().to_vec());
    if wallet_secret.as_ref().is_empty() {
        return Err(Error::WalletSecretRequired);
    }
    let wallet_secret_validate =
        Secret::new(term.ask(true, "Re-enter wallet encryption password: ").await?.trim().as_bytes().to_vec());
    if wallet_secret_validate.as_ref() != wallet_secret.as_ref() {
        return Err(Error::WalletSecretMatch);
    }

    tprintln!(ctx, "");
    if import_with_mnemonic {
        tpara!(
            ctx,
            "\
            \
            If your original wallet has a bip39 recovery passphrase, please enter it now.\
            \
            Specifically, this is not a wallet password. This is a secondary mnemonic passphrase\
            used to encrypt your mnemonic. This is known as a 'payment passphrase'\
            'mnemonic passphrase', or a 'recovery passphrase'. If your mnemonic was created\
            with a payment passphrase and you do not enter it now, the import process\
            will generate a different private key.\
            \
            If you do not have a bip39 recovery passphrase, press ENTER.\
            \
            ",
        );
    } else {
        tpara!(
            ctx,
            "\
            PLEASE NOTE: The optional bip39 mnemonic passphrase, if provided, will be required to \
            issue transactions. This passphrase will also be required when recovering your wallet \
            in addition to your private key or mnemonic. If you lose this passphrase, you will not \
            be able to use or recover your wallet! \
            \
            If you do not want to use bip39 recovery passphrase, press ENTER.\
            ",
        );
    }

    let payment_secret = term.ask(true, "Enter bip39 mnemonic passphrase (optional): ").await?;
    let payment_secret =
        if payment_secret.trim().is_empty() { None } else { Some(Secret::new(payment_secret.trim().as_bytes().to_vec())) };

    if let Some(payment_secret) = payment_secret.as_ref() {
        let payment_secret_validate =
            Secret::new(term.ask(true, "Please re-enter mnemonic passphrase: ").await?.trim().as_bytes().to_vec());
        if payment_secret_validate.as_ref() != payment_secret.as_ref() {
            return Err(Error::PaymentSecretMatch);
        }
    }

    tprintln!(ctx, "");

    let prv_key_data_args = if import_with_mnemonic {
        let words = crate::wizards::import::prompt_for_mnemonic(&term).await?;
        PrvKeyDataCreateArgs::new(None, payment_secret.clone(), Secret::from(words.join(" ")), PrvKeyDataVariantKind::Mnemonic)
    } else {
        PrvKeyDataCreateArgs::new(
            None,
            payment_secret.clone(),
            Secret::from(Mnemonic::random(word_count, Language::default())?.phrase()),
            PrvKeyDataVariantKind::Mnemonic,
        )
    };

    let mnemonic_phrase = prv_key_data_args.secret.clone();

    let notifier = ctx.notifier().show(Notification::Processing).await;

    // suspend commits for multiple operations
    wallet.store().batch().await?;

    let wallet_args = WalletCreateArgs::new(name.map(String::from), custom_filename.clone(), EncryptionKind::XChaCha20Poly1305, hint, true);
    let (wallet_descriptor, storage_descriptor) = ctx.wallet().create_wallet(&wallet_secret, wallet_args).await?;
    let prv_key_data_id = wallet.create_prv_key_data(&wallet_secret, prv_key_data_args).await?;

    let account_args = AccountCreateArgsBip32::new(account_name, None);
    let account = wallet.create_account_bip32(&wallet_secret, prv_key_data_id, payment_secret.as_ref(), account_args).await?;

    // flush data to storage
    wallet.store().flush(&wallet_secret).await?;

    notifier.hide();

    if !import_with_mnemonic {
        tprintln!(ctx, "");
        tprintln!(ctx, "---");
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style("IMPORTANT:").red());
        tprintln!(ctx, "");

        tpara!(
            ctx,
            "Your mnemonic phrase allows you to re-create your private key. \
            The person who has access to this mnemonic will have full control of \
            the Marigold stored in it. Keep your mnemonic safe. Write it down and \
            store it in a safe, preferably in a fire-resistant location. Do not \
            store your mnemonic on this computer or a mobile device. This wallet \
            will never ask you for this mnemonic phrase unless you manually \
            initiate a private key recovery. \
            ",
        );

        // descriptor

        ["", "Never share your mnemonic with anyone!", "---", "", "Your default wallet account mnemonic:", mnemonic_phrase.as_str()?]
            .into_iter()
            .for_each(|line| term.writeln(line));
    }

    // Note-vault ceremony — at wallet creation, where it belongs, not as a
    // lazy auto-create that logs the 24 words mid-command (which is exactly
    // how the founder's vault words ended up in scrollback, 2026-09-05).
    tprintln!(ctx, "");
    tprintln!(ctx, "---");
    tpara!(
        ctx,
        "\
        Your note vault holds the keys to your bearer notes. It has its own \
        24-word recovery phrase — a second, independent secret from the \
        account mnemonic above. You can supply your own 24 words or have \
        them generated now.\
        ",
    );
    tprintln!(ctx, "");
    let vault_words = loop {
        let input = term
            .ask(false, "Enter your own 24-word vault recovery phrase, or press <enter> to generate one: ")
            .await?
            .trim()
            .to_string();
        if input.is_empty() {
            break None;
        }
        let words: Vec<&str> = input.split_whitespace().collect();
        if words.len() != 24 {
            tprintln!(ctx, "Expected 24 words, got {} — try again (or press <enter> to generate)", words.len());
            continue;
        }
        match Mnemonic::new(words.join(" "), Language::default()) {
            Ok(_) => break Some(words.join(" ")),
            Err(err) => {
                tprintln!(ctx, "Not a valid 24-word phrase ({err}) — try again (or press <enter> to generate)");
                continue;
            }
        }
    };
    let store = wallet.store().as_note_key_store()?;
    match vault_words {
        Some(words) => {
            store.vault_restore_from_words(&words, &wallet_secret).await?;
            tprintln!(ctx, "Note vault created from your recovery phrase.");
        }
        None => {
            let words = store.vault_create(&wallet_secret).await?;
            tprintln!(ctx, "");
            tprintln!(ctx, "{}", style("Your note vault recovery phrase — write these 24 words down NOW:").red());
            tprintln!(ctx, "");
            term.writeln(style(&words).cyan().to_string());
            tprintln!(ctx, "");
            tpara!(
                ctx,
                "\
                Recovering your notes on another machine requires BOTH these 24 words \
                AND the vault files ('note vault backup <dir>' copies them). The words \
                will not be shown again.\
                ",
            );
            term.ask(false, "Press <enter> once you have written them down: ").await?;
        }
    }

    term.writeln("");
    term.writeln(format!("Your wallet is stored in: {}", storage_descriptor));
    term.writeln("");

    let receive_address = account.receive_address()?;
    term.writeln("Your default account deposit address:");
    term.writeln(style(receive_address).blue().to_string());
    term.writeln("");

    wallet.open(&wallet_secret, custom_filename.clone().or_else(|| name.map(String::from)), WalletOpenArgs::default_with_legacy_accounts(), &guard).await?;
    wallet.activate_accounts(None, &guard).await?;

    // Remember this wallet: plaintext client metadata in the wallet file
    // itself (travels with the file) + last-opened pointer in settings.
    // 'wallet remember off' opts out later.
    let meta = kaspa_wallet_core::storage::local::ClientMetadata {
        network: wallet.network_id().ok().map(|n| n.to_string()),
        server: None,
        last_opened: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs()),
        remember: true,
        hidden: false,
        auto_mint: false,
        auto_mint_threshold_petals: 0,
    };
    wallet.store().set_client_metadata(&wallet_descriptor.filename, Some(meta)).await.ok();
    ctx.wallet().settings().set(WalletSettings::Wallet, wallet_descriptor.filename.clone()).await.ok();

    Ok(())
}
