use crate::imports::*;
use kaspa_consensus_core::tx::TransactionId;
use kaspa_wallet_core::error::Error as WalletError;
use kaspa_wallet_core::storage::Binding;
#[derive(Default, Handler)]
#[help("Display transaction history")]
pub struct History;

impl History {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, mut argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        let guard = ctx.wallet().guard();
        let guard = guard.lock().await;

        if argv.is_empty() {
            self.display_help(ctx, argv).await?;
            return Ok(());
        }

        // Handled before an account is required: this is a setting, not a
        // query, and it should work whether or not a wallet is open.
        if argv[0] == "detail" {
            let current: bool = ctx.wallet().settings().get(WalletSettings::HistoryDetail).unwrap_or(false);
            match argv.get(1).map(|s| s.as_str()) {
                Some("on") => {
                    ctx.wallet().settings().set(WalletSettings::HistoryDetail, true).await?;
                    tprintln!(ctx, "History detail is on — the wallet's internal bookkeeping is recorded too.");
                    tprintln!(ctx, "{}", style("Reorgs, change and batch entries. Useful for diagnosis, noisy otherwise.").dim());
                }
                Some("off") => {
                    ctx.wallet().settings().set(WalletSettings::HistoryDetail, false).await?;
                    tprintln!(ctx, "History detail is off — only money arriving and leaving is recorded.");
                }
                _ => {
                    tprintln!(ctx, "History detail is {}.", if current { "on" } else { "off" });
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Off records only what you would recognise as a transaction: money in,");
                    tprintln!(ctx, "money out, transfers between your own accounts.");
                    tprintln!(ctx, "On also records the wallet's own bookkeeping — reorgs, change and the");
                    tprintln!(ctx, "batches a large payment is assembled from. Diagnosis only; it is a lot.");
                    tprintln!(ctx, "");
                    tprintln!(ctx, "usage: 'history detail on' | 'history detail off'");
                }
            }
            return Ok(());
        }

        if argv[0] == "clear" {
            return Self::clear(&ctx, argv.get(1).map(|s| s.as_str()) == Some("all")).await;
        }

        let account = ctx.account().await?;
        let network_id = ctx.wallet().network_id()?;
        let binding = Binding::from(&account);
        let current_daa_score = ctx.wallet().current_daa_score();

        let (last, include_utxo) = match argv.remove(0).as_str() {
            "lookup" => {
                let transaction_id = if argv.is_empty() {
                    tprintln!(ctx, "usage: history lookup <transaction id>");
                    return Ok(());
                } else {
                    argv.remove(0)
                };

                let txid = TransactionId::from_hex(transaction_id.as_str())?;
                let store = ctx.wallet().store().as_transaction_record_store()?;
                match store.load_single(&binding, &network_id, &txid).await {
                    Ok(tx) => {
                        let lines = tx
                            .format_transaction_with_args(
                                &ctx.wallet(),
                                None,
                                current_daa_score,
                                true,
                                true,
                                Some(account.clone()),
                                &guard,
                            )
                            .await;
                        lines.iter().for_each(|line| tprintln!(ctx, "{line}"));
                    }
                    Err(_) => {
                        tprintln!(ctx, "transaction not found");
                    }
                }

                return Ok(());
            }
            "list" => {
                let last = if argv.is_empty() { None } else { argv[0].parse::<usize>().ok() };
                (last, false)
            }
            "details" => {
                let last = if argv.is_empty() { None } else { argv[0].parse::<usize>().ok() };
                (last, true)
            }
            v => {
                tprintln!(ctx, "unknown command: '{v}'");
                self.display_help(ctx, argv).await?;
                return Ok(());
            }
        };

        let store = ctx.wallet().store().as_transaction_record_store()?;
        let mut ids = match store.transaction_id_iter(&binding, &network_id).await {
            Ok(ids) => ids,
            Err(err) => {
                if matches!(err, WalletError::NoRecordsFound) {
                    tprintln!(ctx);
                    tprintln!(ctx, "No transactions found for this account.");
                    tprintln!(ctx);
                } else {
                    terrorln!(ctx, "{err}");
                }
                return Ok(());
            }
        };
        let length = ids.size_hint().0;
        let skip = if let Some(last) = last { length.saturating_sub(last) } else { 0 };
        let mut index = 0;
        let page = 25;

        tprintln!(ctx);

        while let Some(id) = ids.try_next().await? {
            if index >= skip {
                if index > 0 && index % page == 0 {
                    tprintln!(ctx);
                    let prompt = format!(
                        "Displaying transactions {} to {} of {} (press any key to continue, 'Q' to abort)",
                        index.separated_string(),
                        (index + page).separated_string(),
                        length.separated_string()
                    );
                    let query = ctx.term().kbhit(Some(&prompt)).await?;
                    tprintln!(ctx);
                    if query.to_lowercase() == "q" {
                        return Ok(());
                    }
                }

                match store.load_single(&binding, &network_id, &id).await {
                    Ok(tx) => {
                        let lines = tx
                            .format_transaction_with_args(
                                &ctx.wallet(),
                                None,
                                current_daa_score,
                                include_utxo,
                                true,
                                Some(account.clone()),
                                &guard,
                            )
                            .await;
                        lines.iter().for_each(|line| tprintln!(ctx, "{line}"));
                    }
                    Err(err) => {
                        terrorln!(ctx, "Unable to read transaction data: {err}");
                    }
                }
            }
            index += 1;
        }

        tprintln!(ctx);
        tprintln!(ctx, "{} transactions", length.separated_string());
        tprintln!(ctx);

        Ok(())
    }

    /// `history clear [all]` — delete transaction records.
    ///
    /// Records are history, never money: a restore needs the wallet file and
    /// the note vault and nothing from here. Deleting them costs you the
    /// ability to look back, and nothing else.
    async fn clear(ctx: &Arc<KaspaCli>, all: bool) -> Result<()> {
        let configured: String = ctx
            .wallet()
            .settings()
            .get(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
        let folder = workflow_store::fs::resolve_path(&configured)?;

        // Which .transactions folders to remove: this wallet's, or every one.
        let mut targets: Vec<std::path::PathBuf> = Vec::new();
        if all {
            if let Ok(entries) = std::fs::read_dir(&folder) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.ends_with(".transactions") && entry.path().is_dir() {
                        targets.push(entry.path());
                    }
                }
            }
        } else {
            let Some(descriptor) = ctx.store().descriptor() else {
                tprintln!(ctx, "Open a wallet first, or use 'history clear all'.");
                return Ok(());
            };
            let path = folder.join(format!("{}.transactions", descriptor.filename));
            if path.is_dir() {
                targets.push(path);
            }
        }

        if targets.is_empty() {
            tprintln!(ctx, "No history to clear.");
            return Ok(());
        }

        // Counting five million files takes long enough to look like a hang,
        // so report size rather than count and get on with it.
        tprintln!(ctx, "");
        for path in &targets {
            tprintln!(ctx, "  {}", path.display());
        }
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style("This deletes transaction history only. Your wallet and your notes are").dim());
        tprintln!(ctx, "{}", style("untouched — history is not needed to restore anything.").dim());
        let answer = ctx.term().ask(false, &format!("Delete {} history folder(s)? [y/N]: ", targets.len())).await?;
        if !answer.trim().to_lowercase().starts_with('y') {
            tprintln!(ctx, "Left alone.");
            return Ok(());
        }

        let mut removed = 0usize;
        for path in &targets {
            match std::fs::remove_dir_all(path) {
                Ok(()) => removed += 1,
                Err(err) => tprintln!(ctx, "Could not remove {}: {err}", path.display()),
            }
        }
        tprintln!(ctx, "");
        tprintln!(ctx, "Cleared {removed} history folder(s).");
        tprintln!(ctx, "");
        Ok(())
    }

    async fn display_help(self: Arc<Self>, ctx: Arc<KaspaCli>, _argv: Vec<String>) -> Result<()> {
        ctx.term().help(
            &[
                ("list [<last N transactions>]", "List transactions"),
                ("details [<last N transactions>]", "List transactions with UTXO details"),
                ("lookup <transaction id>", "Lookup transaction in the history"),
                ("detail [on|off]", "Whether the wallet's own bookkeeping is recorded too (default off)"),
                ("clear", "Delete this wallet's transaction history (not your notes)"),
                ("clear all", "Delete the transaction history of every wallet in the folder"),
            ],
            None,
        )?;

        Ok(())
    }
}
