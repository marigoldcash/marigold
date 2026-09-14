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

        let network_type = NetworkType::from(ctx.wallet().network_id()?);
        let ticker = kaspa_wallet_core::utils::kaspa_suffix(&network_type);

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
        // The chain check happens BEFORE anything is printed, because it can
        // change the answer. A note that stops being counted stops being
        // counted in this total too — printing the figure first and then
        // saying some of it does not exist leaves a wrong number on screen
        // and a corrected one in the prompt, which is what it did.
        let mut chain_note: Vec<String> = Vec::new();
        if total > 0 || mirrored > 0 {
            if ctx.wallet().is_connected() {
                match kaspa_wallet_core::account::notepool::reconcile_held_notes(account.clone()).await {
                    Ok(Some(result)) if !result.moved_to_unknown.is_empty() => {
                        let mut dropped = 0u64;
                        for info in &result.moved_to_unknown {
                            let petals = DENOMINATION_PETALS[info.d as usize];
                            dropped += petals;
                            total = total.saturating_sub(petals);
                            counts[info.d as usize] = counts[info.d as usize].saturating_sub(1);
                        }
                        chain_note.push(ui::warn(format!(
                            "{} note(s) worth {} {ticker} are not on chain and are no longer counted above.",
                            result.moved_to_unknown.len(),
                            sompi_to_kaspa_string(dropped)
                        )));
                        chain_note.push(ui::dim("Most often a payment that never landed, in which case the money never"));
                        chain_note.push(ui::dim("left your ledger balance. 'note unknown' lists them."));
                    }
                    Ok(_) => {}
                    Err(err) => chain_note.push(ui::dim(format!("(could not check these against the chain: {err})"))),
                }
            } else {
                chain_note.push(ui::dim("Not verified on chain — this is what your vault says it holds, unchecked."));
                chain_note.push(ui::dim("Connect a node and run 'balance' again to confirm the notes really exist."));
            }
        }

        // Columns rather than a frame. A frame is for a moment — the note at
        // startup, a wallet being created; `balance` is run twenty times a
        // day and furniture that often becomes wallpaper. What it does need
        // is the amounts lining up on the decimal point, which is what the
        // right-aligned column is for and what a hand-rolled format! never
        // quite managed.
        let money = |petals: u64| ui::paint(ui::Ink::Petal, sompi_to_kaspa_string(petals));
        let unit = ui::paint(ui::Ink::Moss, ticker);
        // The minimums keep an empty wallet's balance from collapsing to
        // "notes 0 MAGLD" — the widths should not move as money arrives.
        const COLUMNS: [ui::Column; 4] =
            [("", ui::Align::Left, 15), ("", ui::Align::Right, 14), ("", ui::Align::Left, 6), ("", ui::Align::Left, 0)];

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

        // The ledger comes last and only when it holds something — for anyone
        // but an exchange it should be empty most of the time. The amount and
        // its ticker go in their own columns rather than arriving as one
        // pre-formatted string, so the ledger figure lines up with the notes
        // figure above it instead of floating a few characters to the right.
        //
        // And only when the figure is actually known. After a reload that did
        // not finish, the UTXO context is empty, which is indistinguishable
        // from an empty ledger by looking at it — so the wallet says which of
        // the two it is rather than printing a zero it cannot stand behind.
        if !ctx.ledger_is_known() {
            rows.push(vec![String::new(); 4]);
            rows.push(vec![
                ui::paint(ui::Ink::Cream, "ledger"),
                ui::paint(ui::Ink::Moss, "not read yet"),
                String::new(),
                ui::paint(ui::Ink::Moss, "the node has not answered — try 'balance' again shortly"),
            ]);
        } else if let Some(balance) = account.balance() {
            if balance.mature > 0 || balance.pending > 0 {
                let mut aside = format!(
                    "{} piece{}",
                    balance.mature_utxo_count.separated_string(),
                    if balance.mature_utxo_count == 1 { "" } else { "s" }
                );
                if balance.pending > 0 {
                    aside = format!("{} pending · {aside}", ui::ledger_amount(balance.pending));
                }
                rows.push(vec![String::new(); 4]);
                rows.push(vec![
                    ui::paint(ui::Ink::Cream, "ledger"),
                    ui::paint(ui::Ink::Petal, ui::ledger_amount(balance.mature)),
                    unit.clone(),
                    ui::paint(ui::Ink::Moss, aside),
                ]);
            }
        }

        ui::table(&ctx, &COLUMNS, &rows);

        // After the figures, not before them: the explanation of why a number
        // moved is only readable once the number is on screen.
        if !chain_note.is_empty() {
            tprintln!(ctx, "");
            for line in chain_note {
                tprintln!(ctx, "{line}");
            }
        }

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
