use crate::imports::*;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;

/// Default trigger: 1 MAGLD of matured ledger balance.
const DEFAULT_THRESHOLD_PETALS: u64 = 100_000_000;

#[derive(Default, Handler)]
#[help("Automatically turn arriving ledger balance into notes ('auto on|off|<amount>')")]
pub struct Auto;

impl Auto {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;

        if !ctx.wallet().is_open() {
            tprintln!(ctx, "Open a wallet first");
            return Ok(());
        }
        let Some(descriptor) = ctx.store().descriptor() else {
            tprintln!(ctx, "Unable to resolve the open wallet's file");
            return Ok(());
        };
        let mut meta = ctx.store().client_metadata(&descriptor.filename).await.ok().flatten().unwrap_or_default();

        match argv.first().map(|s| s.to_lowercase()).as_deref() {
            None => {
                let threshold =
                    if meta.auto_mint_threshold_petals == 0 { DEFAULT_THRESHOLD_PETALS } else { meta.auto_mint_threshold_petals };
                tprintln!(ctx, "");
                if meta.auto_mint {
                    tprintln!(
                        ctx,
                        "auto-mint is ON above {} MAGLD — {}",
                        sompi_to_kaspa_string(threshold),
                        if ctx.auto_mint_armed() { "armed for this session" } else { "not armed (re-open the wallet to arm it)" }
                    );
                } else {
                    tprintln!(ctx, "auto-mint is OFF");
                }
                tprintln!(ctx, "");
                tprintln!(ctx, "When on, ledger balance that arrives (mining rewards, incoming transfers) is minted into");
                tprintln!(ctx, "notes once it passes the threshold — so your money ends up as cash without you asking,");
                tprintln!(ctx, "and the ledger never accumulates the dust that makes 'sweep' necessary.");
                tprintln!(ctx, "");
                tprintln!(ctx, "  'auto on'          turn it on (arms now and every time you open this wallet)");
                tprintln!(ctx, "  'auto off'         turn it off");
                tprintln!(ctx, "  'auto <amount>'    set the threshold, e.g. 'auto 10'");
                tprintln!(ctx, "");
                tprintln!(ctx, "It signs on your behalf, so while it is armed this wallet's password is held in memory");
                tprintln!(ctx, "for as long as the wallet is open — never written to disk. That is the hot-wallet trade;");
                tprintln!(ctx, "'auto off' or closing the wallet ends it.");
                tprintln!(ctx, "");
            }
            Some("on") => {
                meta.auto_mint = true;
                if meta.auto_mint_threshold_petals == 0 {
                    meta.auto_mint_threshold_petals = DEFAULT_THRESHOLD_PETALS;
                }
                let threshold = meta.auto_mint_threshold_petals;
                ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                ctx.arm_auto_mint(wallet_secret, threshold);
                tprintln!(ctx, "auto-mint on: ledger balance above {} MAGLD becomes notes automatically.", sompi_to_kaspa_string(threshold));
            }
            Some("off") => {
                meta.auto_mint = false;
                ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                ctx.disarm_auto_mint();
                tprintln!(ctx, "auto-mint off.");
            }
            Some(_) => {
                let threshold = try_parse_required_nonzero_kaspa_as_sompi_u64(argv.first())?;
                meta.auto_mint_threshold_petals = threshold;
                let armed = meta.auto_mint;
                ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                if armed && ctx.auto_mint_armed() {
                    let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                    ctx.arm_auto_mint(wallet_secret, threshold);
                }
                tprintln!(ctx, "auto-mint threshold set to {} MAGLD.", sompi_to_kaspa_string(threshold));
                if !armed {
                    tprintln!(ctx, "(auto-mint is still off — 'auto on' to enable it)");
                }
            }
        }

        Ok(())
    }
}
