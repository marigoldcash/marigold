use crate::cli::KaspaCli;
use crate::imports::*;
use crate::result::Result;
use kaspa_bip32::{Language, Mnemonic};
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
        // Say up front if that name is taken. The overwrite warning came later,
        // after the name was accepted, so the wizard first announced where the
        // wallet "will be stored" as though the path were free — and the path
        // in question held a wallet with money in it.
        let taken = ctx.store().exists(Some(file.trim_end_matches(".wallet"))).await.unwrap_or(false);
        tprintln!(ctx, "This wallet will be stored as: {}", style(format!("{folder}/{file}")).cyan());
        if taken {
            tprintln!(ctx, "{}", style("That name is taken. Type a different one — a wallet is never overwritten.").yellow());
        }
        tprintln!(ctx, "(change the folder for all wallets with 'settings set folder <path>' before creating)");
        let input = term.ask(false, "Press <enter> to accept, or type a different wallet name: ").await?.trim().to_string();
        if input.is_empty() {
            if taken {
                // Nothing here overwrites a wallet. The file it would replace
                // can hold the only copy of somebody's notes, and there is no
                // undo — so the wizard has no path to it at all, deliberately.
                // Deleting a wallet is 'wallet destroy', which checks the
                // balance first and makes you type the name.
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("That name is taken, and a wallet is never overwritten.").red());
                tprintln!(ctx, "Pick another name, or remove the old one first with 'wallet destroy <name>'.");
                continue;
            }
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

    // Belt and braces: the loop above will not let a taken name through, but a
    // caller reaching here by some other route must still not destroy a wallet.
    let filename = make_filename(&name.map(String::from), &custom_filename);
    if wallet.exists(Some(&filename)).await? {
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style(format!("A wallet named '{filename}' already exists, and is never overwritten.")).red());
        tprintln!(ctx, "Choose another name, or remove that one first with 'wallet destroy {filename}'.");
        tprintln!(ctx, "");
        return Ok(());
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
            You can add an extra passphrase on top of the recovery phrase below. It is optional, \
            and most people should press ENTER to skip it. \
            \
            If you do add one: it becomes part of the recovery phrase. You will be asked for it \
            every time you spend from the ledger, and recovering that account needs BOTH the phrase \
            and the passphrase. Lose it and the ledger side of this wallet is unrecoverable — your \
            notes are unaffected, they are protected by your wallet password and the vault phrase, \
            not by this. \
            \
            Press ENTER to skip it.\
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

    // Note-vault ceremony — at wallet creation, where it belongs, not as a
    // lazy auto-create that logs the 24 words mid-command (which is exactly
    // how the founder's vault words ended up in scrollback, 2026-09-05).
    tprintln!(ctx, "");
    tprintln!(ctx, "---");
    tpara!(
        ctx,
        "\
        Your note vault holds the keys to your bearer notes — your money. It has \
        a 24-word recovery phrase, and this is the one to write down. You can \
        supply your own 24 words or have them generated now.\
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
    // The vault phrase is settled BEFORE any key is made, because the account
    // key is now derived from it: one phrase recovers both sides. Choosing it
    // afterwards would mean generating an account key that the phrase could not
    // reproduce.
    let vault_words = match vault_words {
        Some(words) => words,
        None => kaspa_wallet_core::storage::local::notevault::new_vault_words()?,
    };

    let prv_key_data_args = if import_with_mnemonic {
        let words = crate::wizards::import::prompt_for_mnemonic(&term).await?;
        PrvKeyDataCreateArgs::new(None, payment_secret.clone(), Secret::from(words.join(" ")), PrvKeyDataVariantKind::Mnemonic)
    } else {
        // Derived, not random: the vault phrase reproduces it, so there is one
        // phrase to keep rather than two.
        let account = kaspa_wallet_core::storage::local::notevault::account_mnemonic_from_vault_words(&vault_words)?;
        PrvKeyDataCreateArgs::new(None, payment_secret.clone(), Secret::from(account.phrase_string()), PrvKeyDataVariantKind::Mnemonic)
    };

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

    // The account's own bip39 phrase is deliberately NOT shown.
    //
    // It lives inside the wallet file, encrypted under the wallet password, and
    // recovering that file recovers it — so it is not a second thing to write
    // down, it is the same thing. Printing it here put a third secret in front
    // of someone who had just been given two, and every extra secret shown is
    // one more that gets photographed, pasted, or written on the wrong piece of
    // paper. 'export mnemonic' produces it from an open wallet on the rare
    // occasion something outside Marigold needs it.
    if !import_with_mnemonic {
        tprintln!(ctx, "");
        tpara!(
            ctx,
            "This wallet also holds an ordinary account key for the transparent ledger. It is \
            derived from the 24 words below, so those words bring it back too — there is nothing \
            separate to write down. If another program ever needs it, 'export mnemonic' shows it. \
            ",
        );
    }

    let store = wallet.store().as_note_key_store()?;
    {
        {
            let words = vault_words.clone();
            store.vault_restore_from_words(&words, &wallet_secret).await?;
            tprintln!(ctx, "");
            tprintln!(ctx, "{}", style("Your note vault recovery phrase — write these 24 words down NOW:").red());
            tprintln!(ctx, "");
            term.writeln(style(&words).cyan().to_string());
            tprintln!(ctx, "");
            tpara!(
                ctx,
                "\
                These words bring back your ledger balance on their own. Your NOTES need \
                the words AND a copy of the vault files ('note vault backup <dir>' makes \
                one) — nothing can derive a note, which is exactly what makes it cash. \
                The words will not be shown again.\
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
        // Automation on by default — the ledger is plumbing a person should
        // never have to manage (2026-09-05).
        auto_mint: true,
        auto_mint_threshold_petals: 100_000_000,
        auto_sweep: true,
        auto_sweep_utxo_threshold: 0,
        auto_configured: false,
    };
    wallet.store().set_client_metadata(&wallet_descriptor.filename, Some(meta)).await.ok();
    ctx.wallet().settings().set(WalletSettings::Wallet, wallet_descriptor.filename.clone()).await.ok();

    Ok(())
}
