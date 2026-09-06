use kaspa_wallet_core::account::BIP32_ACCOUNT_KIND;
use kaspa_wallet_core::account::LEGACY_ACCOUNT_KIND;
use kaspa_wallet_core::account::MULTISIG_ACCOUNT_KIND;

use crate::imports::*;
use crate::wizards;

#[derive(Default, Handler)]
#[help("Account management operations")]
pub struct Account;

impl Account {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, mut argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let wallet = ctx.wallet();

        // Help is readable without an open wallet — discoverability
        // shouldn't require unlocking anything.
        if argv.is_empty() || argv.first().map(|s| s.to_lowercase()).as_deref() == Some("help") {
            return self.display_help(ctx, argv).await;
        }

        if !wallet.is_open() {
            return Err(Error::WalletIsNotOpen);
        }

        let action = argv.remove(0);

        match action.as_str() {
            "name" => {
                if argv.len() != 1 {
                    tprintln!(ctx, "usage: 'account name <name>' or 'account name remove'");
                    return Ok(());
                } else {
                    let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                    let _ = ctx.notifier().show(Notification::Processing).await;
                    let account = ctx.select_account().await?;
                    let name = argv.remove(0);
                    if name == "remove" {
                        account.rename(&wallet_secret, None).await?;
                    } else {
                        account.rename(&wallet_secret, Some(name.as_str())).await?;
                    }
                }
            }
            "create" => {
                let account_kind = if argv.is_empty() {
                    BIP32_ACCOUNT_KIND.into()
                } else {
                    let kind = argv.remove(0);
                    kind.parse::<AccountKind>()?
                };

                let account_name = if argv.is_empty() {
                    None
                } else {
                    let name = argv.remove(0);
                    let name = name.trim().to_string();

                    Some(name)
                };

                let prv_key_data_info = ctx.select_private_key().await?;

                let account_name = account_name.as_deref();
                wizards::account::create(&ctx, prv_key_data_info, account_kind, account_name).await?;
            }
            "import" => {
                if argv.is_empty() {
                    tprintln!(ctx, "usage: 'account import <import-type> <key-type> [extra keys]'");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "examples:");
                    tprintln!(ctx, "");
                    ctx.term().help(
                        &[
                            (
                                "account import mnemonic bip32",
                                "Import a Bip32 account from a 12 or 24 word mnemonic",
                            ),
                            (
                                "account import mnemonic multisig [additional keys]",
                                "Import mnemonic and additional keys for a multisig account",
                            ),
                        ],
                        None,
                    )?;

                    return Ok(());
                }

                let import_kind = argv.remove(0);
                match import_kind.as_ref() {
                    "mnemonic" => {
                        if argv.is_empty() {
                            tprintln!(ctx, "usage: 'account import mnemonic <bip32|multisig>'");
                            tprintln!(ctx, "please specify the mnemonic type\r\n");
                            return Ok(());
                        }

                        let account_kind = argv.remove(0);
                        let account_kind = account_kind.parse::<AccountKind>()?;

                        match account_kind.as_ref() {
                            // The legacy (KDX/kaspanet) import path was removed in
                            // FORK-PLAN P7.0 — typing real Kaspa key material into
                            // Marigold software is a key-reuse hazard on a
                            // fair-launch chain (see docs/marigold/DECISIONS.md)
                            LEGACY_ACCOUNT_KIND => {
                                tprintln!(ctx, "legacy (KDX/kaspanet) account import has been removed: importing");
                                tprintln!(ctx, "real Kaspa key material into Marigold would be a key-reuse hazard\r\n");
                                return Ok(());
                            }
                            BIP32_ACCOUNT_KIND => {
                                if !argv.is_empty() {
                                    tprintln!(ctx, "too many arguments: {}\r\n", argv.join(" "));
                                    return Ok(());
                                }
                                crate::wizards::import::import_with_mnemonic(&ctx, account_kind, &argv).await?;
                            }
                            MULTISIG_ACCOUNT_KIND => {
                                crate::wizards::import::import_with_mnemonic(&ctx, account_kind, &argv).await?;
                            }
                            _ => {
                                tprintln!(ctx, "account import is not supported for this account type: '{account_kind}'\r\n");
                                return Ok(());
                            }
                        }

                        return Ok(());
                    }
                    _ => {
                        tprintln!(ctx, "unknown account import type: '{import_kind}'");
                        tprintln!(ctx, "supported import types are: 'mnemonic' or 'multisig-watch'\r\n");
                        return Ok(());
                    }
                }
            }
            "watch" => {
                if argv.is_empty() {
                    tprintln!(ctx, "usage: 'account watch <watch-type> [account name]'");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "examples:");
                    tprintln!(ctx, "");
                    ctx.term().help(
                        &[
                            ("account watch bip32", "Import a extended public key for a watch-only bip32 account"),
                            ("account watch multisig", "Import extended public keys for a watch-only multisig account"),
                        ],
                        None,
                    )?;

                    return Ok(());
                }

                let watch_kind = argv.remove(0);

                let account_name = argv.first().map(|name| name.trim()).filter(|name| !name.is_empty()).map(|name| name.to_string());

                let account_name = account_name.as_deref();

                match watch_kind.as_ref() {
                    "bip32" => {
                        wizards::account::bip32_watch(&ctx, account_name).await?;
                    }
                    "multisig" => {
                        wizards::account::multisig_watch(&ctx, account_name).await?;
                    }
                    _ => {
                        tprintln!(ctx, "unknown account watch type: '{watch_kind}'");
                        tprintln!(ctx, "supported watch types are: 'bip32' or 'multisig'\r\n");
                        return Ok(());
                    }
                }
            }
            "sweep" => {
                // Disambiguate rather than guess: 'sweep' at top level
                // consolidates tracked coins; 'account recover' hunts the
                // derivation chain. They overlap in effect, so a bare
                // 'account sweep' asks which one was meant.
                tprintln!(ctx, "Did you mean:");
                tprintln!(ctx, "  'sweep'            - consolidate this account's coins into fewer, larger ones");
                tprintln!(ctx, "  'account recover'  - search this account's extended derivation chain for funds and bring them home");
                return Ok(());
            }
            "recover" => {
                // 'dry-run' walks the derivation chain and reports what it
                // finds without moving anything (this was the separate 'scan'
                // verb — same code path, one boolean apart; folded in so
                // 'scan' stays free for QR//camera use and 'sweep' keeps its
                // established meaning).
                let dry_run = argv.first().map(|s| s.to_lowercase()).as_deref() == Some("dry-run");
                if dry_run {
                    argv.remove(0);
                }
                let len = argv.len();
                let mut start = 0;
                let mut count = 100_000;
                let window = 128;
                if len >= 2 {
                    start = argv.remove(0).parse::<usize>()?;
                    count = argv.remove(0).parse::<usize>()?;
                } else if len == 1 {
                    count = argv.remove(0).parse::<usize>()?;
                }

                count = count.max(1);

                let sweep = !dry_run;
                // TODO fee_rate
                let fee_rate = None;
                self.derivation_scan(&ctx, start, count, window, sweep, fee_rate).await?;
            }
            "help" => {
                return self.display_help(ctx, argv).await;
            }
            v => {
                tprintln!(ctx, "unknown command: '{v}'\r\n");
                return self.display_help(ctx, argv).await;
            }
        }

        Ok(())
    }

    async fn display_help(self: Arc<Self>, ctx: Arc<KaspaCli>, _argv: Vec<String>) -> Result<()> {
        ctx.term().help(
            &[
                ("create [<type>] [<name>]", "Create a new account (types: 'bip32' (default), 'multisig')"),
                (
                    "import <import-type> [<key-type> [extra keys]]",
                    "Import accounts from a private key using a 24 or 12 word mnemonic. \
                Use 'account import' for additional help.",
                ),
                ("name <name>", "Name or rename the selected account (use 'remove' to remove the name)"),
                ("watch <watch-type> [<name>]", "Create a watch-only account from an extended public key ('account watch' for help)"),
                (
                    "recover [dry-run] [<start>] [<derivations>]",
                    "Search this account's extended address derivation chain for funds and bring them into its \
                     current address (for legacy or imported accounts whose coins sit beyond the normal scan \
                     window). 'dry-run' reports what it finds and moves nothing. For consolidating the coins \
                     this account already holds, use the top-level 'sweep' instead.",
                ),
                // ("purge", "Purge an account from the wallet"),
            ],
            None,
        )?;

        Ok(())
    }

    async fn derivation_scan(
        self: &Arc<Self>,
        ctx: &Arc<KaspaCli>,
        start: usize,
        count: usize,
        window: usize,
        sweep: bool,
        fee_rate: Option<f64>,
    ) -> Result<()> {
        let account = ctx.account().await?;
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        let _ = ctx.notifier().show(Notification::Processing).await;
        let abortable = Abortable::new();
        let ctx_ = ctx.clone();

        let account = account.as_derivation_capable()?;

        account
            .derivation_scan(
                wallet_secret,
                payment_secret,
                start,
                start + count,
                window,
                sweep,
                fee_rate,
                &abortable,
                true,
                Some(Arc::new(move |processed: usize, _, balance, txid| {
                    if let Some(txid) = txid {
                        tprintln!(
                            ctx_,
                            "Scan detected {} MAGLD at index {}; transfer txid: {}",
                            sompi_to_kaspa_string(balance),
                            processed,
                            txid
                        );
                    } else {
                        tprintln!(ctx_, "Scanned {} derivations, found {} MAGLD", processed, sompi_to_kaspa_string(balance));
                    }
                })),
            )
            .await?;

        Ok(())
    }
}
