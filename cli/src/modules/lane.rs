use crate::imports::*;
use kaspa_wallet_core::account::lane::{
    ANCHOR_SELF_PAYMENT_PETALS, LANE_REGISTRATION_FEE_PETALS, LaneClaim, anchor, claim_lane, lane_key, lane_key_or_new,
    registry_address,
};

#[derive(Default, Handler)]
#[help("Claim a lane of your own on the chain, for anchoring your records")]
pub struct Lane;

impl Lane {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let ticker = ctx.ticker();
        match argv.first().map(|s| s.as_str()) {
            Some("claim") => self.claim(&ctx, &argv[1..]).await,
            Some("key") => self.key(&ctx, &argv[1..]).await,
            Some("anchor") => self.anchor(&ctx, &argv[1..]).await,
            _ => {
                tprintln!(ctx, "");
                tpara!(
                    ctx,
                    "A lane is a company's own corner of the chain: a tag of up to five letters or digits, like a ticker symbol, under which its anchoring transactions live, so anyone can find and verify them. Claiming one costs {} {ticker}, paid to the registry once; the first valid claim of a tag holds it. See marigold.cash for what a lane is for and what verification takes.",
                    sompi_to_kaspa_string(LANE_REGISTRATION_FEE_PETALS)
                );
                tprintln!(ctx, "");
                tprintln!(ctx, "  lane claim <TAG> <key> [label]   claim a tag, e.g. lane claim ACME 8f3a…c1 \"Acme Ltd\"");
                tprintln!(
                    ctx,
                    "{}",
                    crate::ui::dim(
                        "  <key> is your company's public key, 64 hex characters: the key your anchors will be attributed to."
                    )
                );
                tprintln!(ctx, "");
                Ok(())
            }
        }
    }

    async fn claim(&self, ctx: &Arc<KaspaCli>, argv: &[String]) -> Result<()> {
        let ticker = ctx.ticker();
        let Some(tag_text) = argv.first() else {
            tprintln!(ctx, "usage: lane claim <TAG> [label], or lane claim <TAG> <64 hex characters> [label]");
            return Ok(());
        };
        ctx.node_ready_for_notes().await?;
        let tag = LaneClaim::parse_tag(tag_text)?;
        // The key: one given as 64 hex characters, or the wallet's own for
        // this lane, made now if it has none.
        let given: Option<[u8; 32]> = argv.get(1).and_then(|k| hex::decode(k.trim()).ok()).and_then(|b| b.try_into().ok());
        let (pk, key_text, label) = match given {
            Some(pk) => (pk, argv[1].clone(), argv[2..].join(" ")),
            None => {
                let (wallet_secret, _) = ctx.ask_wallet_secret_for_tidying(None).await?;
                let key = lane_key_or_new(&ctx.wallet(), &wallet_secret, tag_text).await?;
                (key.pk, hex::encode(key.pk), argv[1..].join(" "))
            }
        };
        let key = &key_text;
        let claim = LaneClaim::new(tag, pk, &label)?;
        let account = ctx.ledger_account().await?;
        let network_id = ctx.wallet().network_id()?;
        let network = network_id.network_type();
        let to = registry_address(network)?;
        // Said before the money question: a five-letter tag is refused for
        // its own reason until wide lanes are active on this network.
        if LaneClaim::needs_wide_lanes(&claim.tag) {
            let activation = kaspa_consensus_core::config::params::Params::from(network_id).wide_lanes_activation;
            let now = ctx.wallet().rpc_api().get_server_info().await?.virtual_daa_score;
            if !activation.is_active(now) {
                tprintln!(
                    ctx,
                    "{}",
                    style(format!(
                        "Five-letter lanes open on this network at DAA score {} (it is {} now, about {} away). A tag of up to four letters can be claimed today.",
                        activation.daa_score().separated_string(),
                        now.separated_string(),
                        crate::cli::humanised_wait(activation.daa_score().saturating_sub(now) as f64 / kaspa_consensus_core::config::params::Params::from(network_id).bps() as f64)
                    ))
                    .yellow()
                );
                return Ok(());
            }
        }
        let mature = account.balance().map(|b| b.mature).unwrap_or(0);
        if mature < LANE_REGISTRATION_FEE_PETALS {
            tprintln!(
                ctx,
                "{}",
                style(format!(
                    "Claiming a lane costs {} {ticker} from the ledger, and the ledger holds {} {ticker}. Mining, or 'redeem' from notes, puts coins on the ledger.",
                    sompi_to_kaspa_string(LANE_REGISTRATION_FEE_PETALS),
                    crate::ui::ledger_amount(mature)
                ))
                .yellow()
            );
            return Ok(());
        }
        tprintln!(ctx, "");
        tprintln!(
            ctx,
            "Claiming lane {} for key …{}{}.",
            claim.tag_text(),
            &key[key.len().saturating_sub(8)..],
            if label.is_empty() { String::new() } else { format!(" ({label})") }
        );
        tprintln!(
            ctx,
            "Registration: {} {ticker}, paid once to the registry at …{}, plus the network fee.",
            sompi_to_kaspa_string(LANE_REGISTRATION_FEE_PETALS),
            &to.to_string()[to.to_string().len() - 8..]
        );
        tprintln!(
            ctx,
            "{}",
            crate::ui::dim(
                "The first valid claim of a tag holds it; a tag already claimed by someone else is theirs, and the fee is not returned."
            )
        );
        let answer = ctx.term().ask(false, "Claim it? [y/N]: ").await?.trim().to_lowercase();
        if !answer.starts_with('y') {
            tprintln!(ctx, "Nothing claimed.");
            return Ok(());
        }
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret(Some(&account)).await?;
        let tx = claim_lane(account, wallet_secret, payment_secret, &claim).await?;
        ctx.record("paid", LANE_REGISTRATION_FEE_PETALS, 0, format!("lane {} claimed", claim.tag_text()), tx.to_string());
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style(format!("Lane {} claimed. Transaction {tx}.", claim.tag_text())).green());
        tpara!(
            ctx,
            "Keep the transaction id: it is the reference for your claim. Your anchoring transactions go in the lane tagged {}; marigold.cash explains the encoding verifiers rely on.",
            claim.tag_text()
        );
        tprintln!(ctx, "");
        Ok(())
    }

    async fn key(&self, ctx: &Arc<KaspaCli>, argv: &[String]) -> Result<()> {
        let Some(tag_text) = argv.first() else {
            tprintln!(ctx, "usage: lane key <TAG>");
            return Ok(());
        };
        LaneClaim::parse_tag(tag_text)?;
        let existing = lane_key(&ctx.wallet(), tag_text).await?;
        let key = match existing {
            Some(key) => key,
            None => {
                let (wallet_secret, _) = ctx.ask_wallet_secret_for_tidying(None).await?;
                let key = lane_key_or_new(&ctx.wallet(), &wallet_secret, tag_text).await?;
                tprintln!(ctx, "");
                tprintln!(
                    ctx,
                    "Made a key for lane {}. It lives in this wallet's vault and is recovered with the vault words.",
                    tag_text.to_ascii_uppercase()
                );
                key
            }
        };
        tprintln!(ctx, "");
        tprintln!(ctx, "Lane {}: key {}", tag_text.to_ascii_uppercase(), hex::encode(key.pk));
        tprintln!(
            ctx,
            "{}",
            crate::ui::dim("'lane claim' registers it; 'lane anchor' signs with it. The secret never leaves the wallet.")
        );
        tprintln!(ctx, "");
        Ok(())
    }

    async fn anchor(&self, ctx: &Arc<KaspaCli>, argv: &[String]) -> Result<()> {
        let ticker = ctx.ticker();
        let (Some(tag_text), Some(root_hex)) = (argv.first(), argv.get(1)) else {
            tprintln!(ctx, "usage: lane anchor <TAG> <root: 64 hex characters>");
            return Ok(());
        };
        ctx.node_ready_for_notes().await?;
        LaneClaim::parse_tag(tag_text)?;
        let root: [u8; 32] = hex::decode(root_hex.trim())
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| Error::custom("the root is 64 hex characters (a 32-byte fingerprint)"))?;
        if lane_key(&ctx.wallet(), tag_text).await?.is_none() {
            tprintln!(
                ctx,
                "This wallet has no key for lane {}: 'lane key {}' makes one, and 'lane claim' registers it.",
                tag_text.to_ascii_uppercase(),
                tag_text.to_ascii_uppercase()
            );
            return Ok(());
        }
        let account = ctx.ledger_account().await?;
        let mature = account.balance().map(|b| b.mature).unwrap_or(0);
        // The tenth it pays itself, plus room for the network fee.
        let needed = ANCHOR_SELF_PAYMENT_PETALS + 2_000_000;
        if mature < needed {
            tprintln!(
                ctx,
                "{}",
                style(format!(
                    "An anchor needs a little on the ledger for its own transaction (about {} {ticker}); the ledger holds {} {ticker}.",
                    sompi_to_kaspa_string(needed),
                    crate::ui::ledger_amount(mature)
                ))
                .yellow()
            );
            return Ok(());
        }
        let (wallet_secret, payment_secret) = ctx.ask_wallet_secret_for_tidying(Some(&account)).await?;
        let tx = anchor(account, wallet_secret, payment_secret, tag_text, root).await?;
        ctx.record("anchored", 0, 0, format!("lane {}", tag_text.to_ascii_uppercase()), tx.to_string());
        tprintln!(ctx, "");
        tprintln!(ctx, "{}", style(format!("Anchored in lane {}. Transaction {tx}.", tag_text.to_ascii_uppercase())).green());
        tpara!(
            ctx,
            "Give whoever holds the records this transaction id with their record and its proof. Anyone can check it against an archival node: the transaction is in your lane, its payload carries this root, and it is signed by your lane's key."
        );
        tprintln!(ctx, "");
        Ok(())
    }
}
