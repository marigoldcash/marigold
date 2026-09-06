use crate::imports::*;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;

/// Default trigger: 1 MAGLD of matured ledger balance.
const DEFAULT_THRESHOLD_PETALS: u64 = 100_000_000;
/// Default sweep trigger: consolidate once the account holds this many
/// mature ledger coins.
pub const DEFAULT_SWEEP_UTXOS: u64 = 2_000;
/// Below this, sweeping cannot keep up with newly mined coins and simply
/// burns fees in a loop.
const MIN_SWEEP_UTXOS: u64 = 500;

#[derive(Default, Handler)]
#[help("Automate the ledger chores: mint arriving balance into notes, and consolidate coins ('auto' for details)")]
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
                if meta.auto_sweep {
                    let sweep_threshold =
                        if meta.auto_sweep_utxo_threshold == 0 { DEFAULT_SWEEP_UTXOS } else { meta.auto_sweep_utxo_threshold };
                    tprintln!(ctx, "auto-sweep is ON above {sweep_threshold} coins");
                } else {
                    tprintln!(ctx, "auto-sweep is OFF");
                }
                tprintln!(ctx, "");
                tprintln!(ctx, "  'auto on'            mint arriving balance into notes (arms whenever you open this wallet)");
                tprintln!(ctx, "  'auto off'           stop minting");
                tprintln!(ctx, "  'auto <amount>'      set the mint threshold, e.g. 'auto 10'");
                tprintln!(ctx, "  'auto sweep [<n>]'   consolidate coins above <n> of them — independent of minting, for");
                tprintln!(ctx, "                       holders (exchanges, say) who want plain ledger balance kept tidy");
                tprintln!(ctx, "  'auto sweep off'     stop consolidating");
                tprintln!(ctx, "");
                tprintln!(ctx, "It signs on your behalf, so while it is armed this wallet's password is held in memory");
                tprintln!(ctx, "for as long as the wallet is open — never written to disk. That is the hot-wallet trade;");
                tprintln!(ctx, "'auto off' or closing the wallet ends it.");
                tprintln!(ctx, "");
            }
            Some("sweep") => {
                let arg = argv.get(1).map(|s| s.to_lowercase());
                match arg.as_deref() {
                    Some("off") => {
                        meta.auto_sweep = false;
                        ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                        ctx.disarm_auto_sweep();
                        tprintln!(ctx, "auto-sweep off. (A consolidation already in flight finishes; nothing new starts.)");
                    }
                    Some("on") | None => {
                        meta.auto_sweep = true;
                        if meta.auto_sweep_utxo_threshold == 0 {
                            meta.auto_sweep_utxo_threshold = DEFAULT_SWEEP_UTXOS;
                        }
                        let threshold = meta.auto_sweep_utxo_threshold;
                        ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                        let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                        ctx.arm_auto_sweep(wallet_secret, threshold);
                        tprintln!(ctx, "auto-sweep on: coins are consolidated once this account holds more than {threshold}.");
                    }
                    Some(count) => {
                        let mut threshold: u64 = count.parse().map_err(|_| Error::custom("usage: 'auto sweep <coin count>'"))?;
                        // A low threshold on a chain that mints coins every
                        // block means sweeping forever and paying fees forever
                        // — consolidation can never get ahead of arrivals.
                        if threshold < MIN_SWEEP_UTXOS {
                            tprintln!(
                                ctx,
                                "A threshold of {threshold} would sweep continuously and burn fees without ever catching up — using {MIN_SWEEP_UTXOS}."
                            );
                            threshold = MIN_SWEEP_UTXOS;
                        }
                        meta.auto_sweep = true;
                        meta.auto_sweep_utxo_threshold = threshold;
                        ctx.store().set_client_metadata(&descriptor.filename, Some(meta)).await?;
                        let (wallet_secret, _) = ctx.ask_wallet_secret(None).await?;
                        ctx.arm_auto_sweep(wallet_secret, threshold);
                        tprintln!(ctx, "auto-sweep on: coins are consolidated once this account holds more than {threshold}.");
                    }
                }
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
