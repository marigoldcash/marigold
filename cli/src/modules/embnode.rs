use crate::imports::*;

#[derive(Default, Handler)]
#[help("The old name: 'connect' starts the network sync, 'connect status' shows it, 'disconnect' stops it")]
pub struct Node;

impl Node {
    async fn main(self: Arc<Self>, ctx: &Arc<dyn Context>, _argv: Vec<String>, _cmd: &str) -> Result<()> {
        let ctx = ctx.clone().downcast_arc::<KaspaCli>()?;
        // There is no node to think about any more (founder, 2026-09-16):
        // 'connect' is the network. The old verb only says where things went.
        tprintln!(ctx, "'node' is now part of 'connect': 'connect' starts syncing the network here, 'connect status'");
        tprintln!(ctx, "shows progress, 'connect logs' shows what it is doing, and 'disconnect' stops it.\r\n");
        Ok(())
    }

    /// How far our own node has got, as a step out of three and a percentage.
    ///
    /// Best effort, and read out of the node's own log records rather than its
    /// RPC: during the first phase it reports zero blocks and zero headers
    /// however you ask it, because the work is going into a staging consensus
    /// that is not committed until the very end. The log is the only place the
    /// progress exists at all.
    ///
    /// Three steps because there are three waits, not because the node has
    /// three phases — it checks the chain's proof, fetches the headers in two
    /// internal stages, then fetches the blocks. Someone watching a bar wants
    /// to know how many more times it fills.
    fn sync_step(ctx: &Arc<KaspaCli>) -> String {
        use crate::log_sink::SyncProgress;
        let (step, percent) = match crate::log_sink::sync_progress() {
            // Levels count DOWN from 250, so progress is how far it has come.
            Some(SyncProgress::VerifyingProof { level }) => (1, 250u32.saturating_sub(level) * 100 / 250),
            Some(SyncProgress::ChainSegment { headers }) => {
                let params = kaspa_consensus_core::config::params::Params::from(
                    ctx.wallet().network_id().unwrap_or(NetworkId::with_suffix(NetworkType::Testnet, 10)),
                );
                let max = params.finality_depth() + 2 * params.ghostdag_k as u64 + 1;
                (2, ((headers as f64 / max as f64) * 100.0) as u32)
            }
            Some(SyncProgress::Headers { percent, .. }) => (2, percent),
            Some(SyncProgress::Blocks { percent, .. }) => (3, percent),
            None => (1, 0),
        };
        // Never 100: a figure that says done while the wallet is still not
        // on its own copy reads as stuck. 99 reads as almost there, which is
        // what it is (founder, 2026-09-16).
        format!("Step {step} of 3, {}% done", percent.min(99))
    }

    /// How long the sync has been stuck, if long enough to be worth saying.
    ///
    /// Ten minutes: header and block batches land far more often than that
    /// when a node is healthy, and the proof phase logs each level as it goes.
    /// Anything quieter is either a peer problem or a wedged IBD, and either
    /// way the person watching deserves to be told rather than left guessing
    /// at a percentage that has stopped.
    fn stalled_for(_ctx: &Arc<KaspaCli>) -> Option<String> {
        let seconds = crate::log_sink::seconds_since_progress()?;
        if seconds < 600 {
            return None;
        }
        let minutes = seconds / 60;
        Some(if minutes < 120 { format!("{minutes} minutes") } else { format!("{} hours", minutes / 60) })
    }

    /// Which node the wallet is talking to, and — if it is not yet your own —
    /// how far off that is.
    ///
    /// Two facts, in this order: which node is answering, and what that means
    /// for who can see your notes. Everything else a node knows about itself
    /// is behind 'node details', because block counts and DAA scores change
    /// nobody's next move and reading them is a skill this wallet should not
    /// require.
    pub(crate) async fn status(&self, ctx: &Arc<KaspaCli>) {
        let mine = ctx.embedded_node_in_use();
        let pending = ctx.embedded_node_pending();
        let connected = ctx.wallet().is_connected();

        tprintln!(ctx, "");
        if mine {
            // "All in sync" was said whenever the wallet was on its own node,
            // syncing or not — beside a prompt that said SYNC (tester,
            // 2026-09-20). The node's progress decides the sentence.
            if ctx.wallet().utxo_processor().is_synced() {
                tprintln!(ctx, "Using: {}", style("your own copy of the network, all in sync.").bold());
            } else {
                tprintln!(
                    ctx,
                    "Using: {} Its sync: {}.",
                    style("your own copy of the network, still catching up.").bold(),
                    Self::sync_step(ctx)
                );
                tpara!(
                    ctx,
                    "A copy that is behind holds only part of the pool, so paying, receiving and minting wait until it has caught up. 'connect public' uses a public computer meanwhile."
                );
                if let Some(stalled) = Self::stalled_for(ctx) {
                    tprintln!(ctx, "");
                    tprintln!(ctx, "{}", style(format!("It has not moved for {stalled}. That is longer than expected.")).yellow());
                    tprintln!(ctx, "{}", style("Leaving it running usually recovers; 'connect logs' shows what it is doing.").dim());
                }
            }
            tprintln!(ctx, "Your wallet notes are not announced to anyone.");
            tprintln!(ctx, "");
            return;
        }
        if pending {
            // Two different situations, and calling both of them "using a
            // public node" was a lie in the second: the public node may have
            // refused us, in which case there is no ledger connection at all
            // until our own node is ready.
            if connected {
                tprintln!(ctx, "Using: {} Your own sync: {}.", style("a public computer.").bold(), Self::sync_step(ctx));
                tprintln!(ctx, "Your wallet notes are announced through a public computer until the sync");
                tprintln!(ctx, "is complete.");
            } else {
                tprintln!(ctx, "Using: {} Your own sync: {}.", style("nothing yet.").bold(), Self::sync_step(ctx));
                tpara!(
                    ctx,
                    "Your notes are safe on this disk, but the ledger stays unread and some things wait until the sync has caught up. Nothing is being announced to anyone in the meantime."
                );
            }
            // The one thing worth interrupting for. A stalled sync looks
            // identical to a working one — the numbers simply stop — and
            // without this the node's own warnings were the only clue, which
            // is why they used to be printed at everybody all the time.
            if let Some(stalled) = Self::stalled_for(ctx) {
                tprintln!(ctx, "");
                tprintln!(ctx, "{}", style(format!("It has not moved for {stalled}. That is longer than expected.")).yellow());
                tprintln!(ctx, "{}", style("Leaving it running usually recovers; 'connect logs' shows what it is doing.").dim());
            }
            tprintln!(ctx, "");
            return;
        }
        if connected && ctx.remote_miner_present() {
            tprintln!(ctx, "Using: {}", style("the miner program running in the background on this machine.").bold());
            tprintln!(ctx, "Its copy of the network is yours; nobody else sees your notes. 'mine status' for the miner.");
            tprintln!(ctx, "");
            return;
        }
        if connected && ctx.connected_to_local_node() {
            // A marigoldd the person runs beside the wallet. It used to be
            // reported as a public computer, privacy warning and all (founder,
            // 2026-09-19).
            let url = ctx.wallet().try_wrpc_client().and_then(|c| c.url()).unwrap_or_default();
            tprintln!(ctx, "Using: {}", style(format!("a node running on this machine ({url}).")).bold());
            tprintln!(ctx, "Its copy of the network is yours; nobody else sees your notes. 'mine start' mines through it.");
            tprintln!(ctx, "");
            return;
        }
        if connected {
            tprintln!(ctx, "Using: {}", style("a public computer.").bold());
            tprintln!(ctx, "Whoever runs it can see which notes your wallet asks about.");
            tprintln!(ctx, "Type 'connect' to sync the network here instead.");
            tprintln!(ctx, "");
            return;
        }
        tprintln!(ctx, "Not connected to the network.");
        tprintln!(ctx, "");
        tprintln!(ctx, "Type 'connect' to get started.");
        tprintln!(ctx, "");
    }

    /// The node's own figures. Kept out of 'node status' on purpose — see
    /// there — but a node that will not sync cannot be diagnosed without them.
    pub(crate) async fn details(&self, ctx: &Arc<KaspaCli>) {
        tprintln!(ctx, "");
        let connected = ctx.wallet().is_connected();
        if !connected && !ctx.embedded_node_running() {
            tprintln!(ctx, "Not connected to the network, and no sync of your own is running.");
            tprintln!(ctx, "");
            return;
        }
        if connected {
            let synced = ctx.wallet().utxo_processor().is_synced();
            tprintln!(ctx, "Reports:      {}", if synced { "caught up" } else { "not caught up" });
        } else {
            // The interesting case: our own node is running and is the only
            // thing there is to report on.
            tprintln!(ctx, "Wallet:       not connected to anything yet");
        }
        if ctx.embedded_node_running() {
            tprintln!(ctx, "Your sync:    {}", Self::sync_step(ctx));
            tprintln!(ctx, "Adopted:      {}", if ctx.embedded_node_in_use() { "yes" } else { "not yet" });
            // The raw figures behind the step-of-three. 'node status' does not
            // carry them because they answer nothing anyone asks; here they are
            // the whole point, since a node that will not sync is diagnosed by
            // watching which of these stops moving.
            use crate::log_sink::SyncProgress;
            match crate::log_sink::sync_progress() {
                Some(SyncProgress::VerifyingProof { level }) => {
                    tprintln!(ctx, "{}", style(format!("              proof level {level} of 250, counting down")).dim())
                }
                Some(SyncProgress::ChainSegment { headers }) => {
                    tprintln!(ctx, "{}", style(format!("              {} chain headers", headers.separated_string())).dim())
                }
                Some(SyncProgress::Headers { headers, block_time, .. }) => {
                    let at = block_time.map(|t| format!(", reached blocks from {t}")).unwrap_or_default();
                    tprintln!(ctx, "{}", style(format!("              {} headers{at}", headers.separated_string())).dim())
                }
                Some(SyncProgress::Blocks { blocks, .. }) => {
                    tprintln!(ctx, "{}", style(format!("              {} blocks", blocks.separated_string())).dim())
                }
                None => {}
            }
        }
        let suppressed = crate::log_sink::suppressed_count();
        if suppressed > 0 {
            tprintln!(
                ctx,
                "{}",
                style(format!("Hidden:       {suppressed} sync warnings — 'connect logs' shows them as they arrive")).dim()
            );
        }
        if !connected {
            tprintln!(ctx, "");
            return;
        }
        match ctx.wallet().rpc_api().get_block_dag_info().await {
            Ok(info) => {
                tprintln!(
                    ctx,
                    "{}",
                    style(format!(
                        "{} blocks, {} headers, virtual DAA score {}",
                        info.block_count.separated_string(),
                        info.header_count.separated_string(),
                        info.virtual_daa_score.separated_string()
                    ))
                    .dim()
                );
            }
            Err(err) => tprintln!(ctx, "{}", style(format!("(could not read the figures: {err})")).dim()),
        }
        tprintln!(ctx, "");
    }
}
