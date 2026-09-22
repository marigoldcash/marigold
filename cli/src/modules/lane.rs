use crate::imports::*;
use kaspa_wallet_core::account::lane::{LANE_REGISTRATION_FEE_PETALS, LaneClaim, claim_lane, registry_address};

#[derive(Default, Handler)]
#[help("Claim a lane of your own on the chain, for anchoring your records")]
pub struct Lane;

impl Lane {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        let ticker = ctx.ticker();
        match argv.first().map(|s| s.as_str()) {
            Some("claim") => self.claim(&ctx, &argv[1..]).await,
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
        let (Some(tag), Some(key)) = (argv.first(), argv.get(1)) else {
            tprintln!(ctx, "usage: lane claim <TAG> <key> [label]");
            return Ok(());
        };
        ctx.node_ready_for_notes().await?;
        let tag = LaneClaim::parse_tag(tag)?;
        let pk: [u8; 32] = hex::decode(key.trim())
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| Error::custom("the key is 64 hex characters (a 32-byte public key)"))?;
        let label = argv[2..].join(" ");
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
}
