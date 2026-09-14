use crate::imports::*;
use crate::ui;
use kaspa_consensus_core::notepool::DENOMINATION_PETALS;
use kaspa_wallet_core::storage::NoteStatus;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;

#[derive(Default, Handler)]
#[help("Show everything you hold: ledger balance and notes, in one view")]
pub struct Balance;

impl Balance {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let account = ctx.wallet().account()?;

        tprintln!(ctx, "");
        let note_key_store = ctx.wallet().store().as_note_key_store()?;
        let mut stream = note_key_store.iter().await?;
        let mut counts = [0u64; DENOMINATION_PETALS.len()];
        let mut total = 0u64;
        let mut mirrored = 0u64;
        let mut mirrored_count = 0usize;
        while let Some(info) = stream.try_next().await? {
            match info.status {
                NoteStatus::Active => {
                    counts[info.d as usize] += 1;
                    total += DENOMINATION_PETALS[info.d as usize];
                }
                // Still yours — this wallet holds the key — but carried on the
                // phone and untouchable here. Listing it separately rather than
                // omitting it: money that silently disappears from the balance
                // when you fund a phone reads as money lost.
                NoteStatus::Mirrored => {
                    mirrored += DENOMINATION_PETALS[info.d as usize];
                    mirrored_count += 1;
                }
                _ => {}
            }
        }
        // Columns rather than a frame. A frame is for a moment — the note at
        // startup, a wallet being created; `balance` is run twenty times a
        // day and furniture that often becomes wallpaper. What it does need
        // is the amounts lining up on the decimal point, which is what the
        // right-aligned column is for and what a hand-rolled format! never
        // quite managed.
        let money = |petals: u64| ui::paint(ui::Ink::Petal, sompi_to_kaspa_string(petals));
        let unit = ui::paint(ui::Ink::Moss, "MAGLD");
        // The minimums keep an empty wallet's balance from collapsing to
        // "notes 0 MAGLD" — the widths should not move as money arrives.
        const COLUMNS: [ui::Column; 4] =
            [("", ui::Align::Left, 15), ("", ui::Align::Right, 14), ("", ui::Align::Left, 5), ("", ui::Align::Left, 0)];

        let mut rows: Vec<Vec<String>> = vec![vec![ui::paint(ui::Ink::Cream, "notes"), money(total), unit.clone(), String::new()]];
        for (index, count) in counts.iter().enumerate().rev() {
            if *count > 0 {
                rows.push(vec![
                    ui::paint(ui::Ink::Moss, format!("  {count} × {}", sompi_to_kaspa_string(DENOMINATION_PETALS[index]))),
                    ui::paint(ui::Ink::Moss, sompi_to_kaspa_string(count * DENOMINATION_PETALS[index])),
                    String::new(),
                    String::new(),
                ]);
            }
        }

        // Still yours, but on the phone and untouchable here. On its own row
        // rather than folded into the total: money that silently vanishes
        // from a balance when you fund a phone reads as money lost.
        if mirrored > 0 {
            rows.push(vec![String::new(); 4]);
            rows.push(vec![
                ui::paint(ui::Ink::Cream, "on your phone"),
                money(mirrored),
                unit.clone(),
                ui::paint(ui::Ink::Moss, format!("{mirrored_count} note{}", if mirrored_count == 1 { "" } else { "s" })),
            ]);
        }

        // Every balance is checked against the pool, and says so only when
        // there is something to say. A figure the wallet merely believes is
        // worth no more than the belief — but a line confirming the obvious on
        // every call trains people to stop reading it, so silence means good.
        if total > 0 || mirrored > 0 {
            if ctx.wallet().is_connected() {
                match kaspa_wallet_core::account::notepool::reconcile_held_notes(account.clone()).await {
                    Ok(Some(result)) if !result.moved_to_unknown.is_empty() => {
                        let value: u64 = result.moved_to_unknown.iter().map(|i| DENOMINATION_PETALS[i.d as usize]).sum();
                        tprintln!(ctx, "");
                        tprintln!(
                            ctx,
                            "{}",
                            style(format!(
                                "{} note(s) worth {} MAGLD are not on chain and have stopped being counted.",
                                result.moved_to_unknown.len(),
                                sompi_to_kaspa_string(value)
                            ))
                            .yellow()
                        );
                        tprintln!(ctx, "{}", style("Most often a payment that never landed, in which case the money never").dim());
                        tprintln!(ctx, "{}", style("left your ledger balance. 'note unknown' lists them.").dim());
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tprintln!(ctx, "");
                        tprintln!(ctx, "{}", style(format!("(could not check these against the chain: {err})")).dim());
                    }
                }
            } else {
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style("Not verified on chain — this is what your vault says it holds, unchecked.").dim());
                tprintln!(ctx, "{}", style("Connect a node and run 'balance' again to confirm the notes really exist.").dim());
            }
        }

        // The ledger comes last and only when it holds something — for anyone
        // but an exchange it should be empty most of the time.
        let network_id = ctx.wallet().network_id()?;
        let network_type = NetworkType::from(network_id);
        if let Some(balance) = account.balance() {
            if balance.mature > 0 || balance.pending > 0 {
                let strings = BalanceStrings::from((Some(&balance), &network_type, None));
                rows.push(vec![String::new(); 4]);
                rows.push(vec![
                    ui::paint(ui::Ink::Cream, "ledger"),
                    ui::paint(ui::Ink::Petal, strings.to_string()),
                    String::new(),
                    ui::paint(
                        ui::Ink::Moss,
                        format!(
                            "{} piece{}",
                            balance.mature_utxo_count.separated_string(),
                            if balance.mature_utxo_count == 1 { "" } else { "s" }
                        ),
                    ),
                ]);
            }
        }

        ui::table(&ctx, &COLUMNS, &rows);

        if total == 0 {
            if let Some(balance) = account.balance() {
                if balance.mature > 0 {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "Tip: turn ledger balance into bearer notes with 'note mint <amount>' (or 'note mint all')");
                }
            }
        }
        tprintln!(ctx, "");

        Ok(())
    }
}
