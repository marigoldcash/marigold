//! Marigold trustee signer (POOL-SPEC.md P5.8, FORK-PLAN P6.12): "a small tool that
//! watches its own node, signs the depth-D block on the P5.8 cadence, aggregates
//! k-of-n partial signatures, and submits the anchor tx."
//!
//! One instance per trustee key (the production topology — one live signer per key,
//! cold standby only, per the spec's own operator guidance). Instances exchange
//! *partial attestations* — a single trustee's BIP340 signature over one
//! (anchored_block, anchored_daa_score) pair — over a deliberately minimal transport:
//! length-prefixed borsh frames on TCP, fire-and-forget. Whoever holds ≥ k partials
//! for the identical pair assembles the canonical `FinalityAnchor` (ascending trustee
//! index) and submits it as a zero-input anchor-lane transaction through its node's
//! RPC; identical assemblies dedup by transaction id, differing-but-valid assemblies
//! all ratchet the same (block, score) — both harmless. The P9.1 trustee ceremony may
//! replace the transport; the signing discipline below is the part that must survive.
//!
//! **The equivocation-safety invariant, load-bearing:** an honest signer signs at most
//! one attestation per cadence interval, advancing only with DAA score — this tool
//! refuses to sign a target whose score is less than one full interval past the last
//! *persisted* signed score. Persistence matters: a restart mid-interval must not
//! re-sign a different block (a reorg away from the previous target inside one
//! interval would otherwise make an honest restart produce a valid equivocation proof
//! against its own key — the exact "misconfigured failover" hazard the spec calls
//! out). State is one small file holding the last signed (score, block).

use kaspa_consensus_core::finality_anchor::{
    ANCHOR_QUORUM, AnchorAttestation, AnchorPayload, FinalityAnchor, TRUSTEE_COUNT, TrusteeKeys, signing_hash,
};
use kaspa_consensus_core::{
    config::params::MAINNET_PARAMS, constants::TX_VERSION_TOCCATA, mass::MassCalculator, subnets::SUBNETWORK_ID_FINALITY_ANCHOR,
    tx::Transaction,
};
use kaspa_core::{debug, info, warn};
use kaspa_grpc_client::GrpcClient;
type DynRpcApi = dyn kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_hashes::Hash;
use kaspa_rpc_core::notify::mode::NotificationMode;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// A single trustee's signed (block, score) contribution, exchanged between signers.
#[derive(Debug, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct PartialAttestation {
    pub trustee_index: u8,
    pub attestation: AnchorAttestation,
}

#[derive(Debug, Clone)]
pub struct SignerConfig {
    /// The node's RPC address: a bare `host:port` (gRPC), or a `ws://` / `wss://`
    /// URL for a node that speaks only wRPC — the wallet's embedded node exposes
    /// nothing else.
    pub rpc_server: String,
    /// This signer's trustee index (0..5) into the network's pinned key set.
    pub trustee_index: u8,
    /// This trustee's BIP340 secret key.
    pub secret_key: [u8; 32],
    /// TCP address to receive peer partials on; `None` disables the listener (a
    /// signer can still contribute by sending partials outward).
    pub listen_address: Option<String>,
    /// Peer signer partial-exchange addresses.
    pub peer_addresses: Vec<String>,
    /// Anchoring depth in DAA-score units (must match the network's params).
    pub depth: u64,
    /// Cadence interval in DAA-score units (must match the network's active stage).
    pub interval: u64,
    /// Node poll period.
    pub poll_millis: u64,
    /// The full pinned trustee key set, used to verify incoming partials before
    /// aggregation. `None` skips partial verification (the node still verifies the
    /// assembled anchor — an invalid partial then only wastes one assembly attempt).
    pub trustee_keys: Option<TrusteeKeys>,
    /// Path for the last-signed persistence file (the equivocation-safety state).
    pub state_file: PathBuf,
}

#[derive(Debug, Clone, Copy, Default)]
struct LastSigned {
    score: u64,
    block: Hash,
}

fn read_state(path: &PathBuf) -> Option<LastSigned> {
    let content = std::fs::read_to_string(path).ok()?;
    let (score, block) = content.trim().split_once(' ')?;
    Some(LastSigned { score: score.parse().ok()?, block: block.parse().ok()? })
}

fn write_state(path: &PathBuf, state: LastSigned) {
    if let Err(err) = std::fs::write(path, format!("{} {}", state.score, state.block)) {
        warn!("[SIGNER] failed to persist signing state to {}: {err}", path.display());
    }
}

/// Builds the canonical zero-input, zero-output anchor-lane transaction for `anchor`.
pub fn anchor_transaction(anchor: &FinalityAnchor) -> Transaction {
    let tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![],
        vec![],
        0,
        SUBNETWORK_ID_FINALITY_ANCHOR,
        0,
        AnchorPayload::Anchor(anchor.clone()).encode_payload(),
    );
    // The zero-input/zero-output shape has no network-dependent mass inputs, so the
    // mainnet cofactors are universally correct here (matches the consensus tests).
    let populated = kaspa_consensus_core::tx::PopulatedTransaction::new(&tx, vec![]);
    let storage_mass =
        MassCalculator::new_with_consensus_params(&MAINNET_PARAMS).calc_contextual_masses(&populated).unwrap().storage_mass;
    tx.set_storage_mass(storage_mass);
    tx
}

async fn send_partial(peer: String, frame: Arc<Vec<u8>>) {
    match TcpStream::connect(&peer).await {
        Ok(mut stream) => {
            let len = (frame.len() as u32).to_le_bytes();
            let result = async {
                stream.write_all(&len).await?;
                stream.write_all(&frame).await?;
                stream.flush().await
            }
            .await;
            if let Err(err) = result {
                debug!("[SIGNER] sending partial to {peer} failed: {err}");
            }
        }
        Err(err) => debug!("[SIGNER] connecting to peer signer {peer} failed: {err}"),
    }
}

/// Runs the partial-exchange listener, feeding decoded partials into `sink`.
async fn serve_partials(listener: TcpListener, sink: mpsc::UnboundedSender<PartialAttestation>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else { continue };
        let sink = sink.clone();
        tokio::spawn(async move {
            let mut len_bytes = [0u8; 4];
            if stream.read_exact(&mut len_bytes).await.is_err() {
                return;
            }
            let len = u32::from_le_bytes(len_bytes) as usize;
            if len > 4096 {
                return; // a partial is ~110 bytes; anything large is garbage
            }
            let mut payload = vec![0u8; len];
            if stream.read_exact(&mut payload).await.is_err() {
                return;
            }
            if let Ok(partial) = borsh::from_slice::<PartialAttestation>(&payload) {
                let _ = sink.send(partial);
            }
        });
    }
}

pub struct TrusteeSigner {
    config: SignerConfig,
    keypair: secp256k1::Keypair,
    client: Arc<DynRpcApi>,
    last_signed: LastSigned,
    /// The selected-chain cursor for target discovery (always a chain block).
    chain_cursor: Option<Hash>,
    /// Collected partials per certified (block, score) pair, ordered by trustee index.
    partials: HashMap<(Hash, u64), BTreeMap<u8, [u8; 64]>>,
    /// The score of the last anchor this instance submitted (suppresses resubmits).
    last_submitted_score: u64,
    incoming: mpsc::UnboundedReceiver<PartialAttestation>,
    incoming_sink: mpsc::UnboundedSender<PartialAttestation>,
}

impl TrusteeSigner {
    pub async fn new(config: SignerConfig) -> Result<Self, String> {
        assert!((config.trustee_index as usize) < TRUSTEE_COUNT, "trustee index out of range");
        let keypair = secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &config.secret_key)
            .map_err(|e| format!("invalid secret key: {e}"))?;
        let client: Arc<DynRpcApi> = if config.rpc_server.starts_with("ws://") || config.rpc_server.starts_with("wss://") {
            use kaspa_wrpc_client::client::{ConnectOptions, ConnectStrategy};
            let client = Arc::new(
                kaspa_wrpc_client::KaspaRpcClient::new(kaspa_wrpc_client::WrpcEncoding::Borsh, Some(&config.rpc_server), None, None, None)
                    .map_err(|e| format!("{}: {e}", config.rpc_server))?,
            );
            let options = ConnectOptions {
                block_async_connect: true,
                strategy: ConnectStrategy::Retry,
                url: Some(config.rpc_server.clone()),
                ..Default::default()
            };
            client.connect(Some(options)).await.map_err(|e| format!("failed to connect to {}: {e}", config.rpc_server))?;
            client
        } else {
            Arc::new(
                GrpcClient::connect_with_args(
                    NotificationMode::Direct,
                    format!("grpc://{}", config.rpc_server),
                    None,
                    true,
                    None,
                    false,
                    Some(500_000),
                    Default::default(),
                )
                .await
                .map_err(|e| format!("failed to connect to {}: {e}", config.rpc_server))?,
            )
        };
        let last_signed = read_state(&config.state_file).unwrap_or_default();
        let (incoming_sink, incoming) = mpsc::unbounded_channel();
        Ok(Self {
            keypair,
            client,
            last_signed,
            chain_cursor: None,
            partials: HashMap::new(),
            last_submitted_score: 0,
            incoming,
            incoming_sink,
            config,
        })
    }

    /// Runs the signer until the task is dropped/aborted. Spawns the partial listener
    /// (if configured) and loops: poll the node, sign on cadence, aggregate, submit.
    pub async fn run(mut self) {
        if let Some(listen) = self.config.listen_address.clone() {
            match TcpListener::bind(&listen).await {
                Ok(listener) => {
                    info!("[SIGNER {}] listening for peer partials on {listen}", self.config.trustee_index);
                    tokio::spawn(serve_partials(listener, self.incoming_sink.clone()));
                }
                Err(err) => warn!("[SIGNER {}] cannot bind {listen}: {err}", self.config.trustee_index),
            }
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(self.config.poll_millis)).await;
            self.drain_incoming();
            if let Err(err) = self.tick().await {
                debug!("[SIGNER {}] tick error: {err}", self.config.trustee_index);
            }
            if let Err(err) = self.try_submit().await {
                debug!("[SIGNER {}] submit error: {err}", self.config.trustee_index);
            }
        }
    }

    fn drain_incoming(&mut self) {
        while let Ok(partial) = self.incoming.try_recv() {
            self.record_partial(partial);
        }
    }

    fn record_partial(&mut self, partial: PartialAttestation) {
        if partial.trustee_index as usize >= TRUSTEE_COUNT {
            return;
        }
        if let Some(keys) = self.config.trustee_keys.as_ref() {
            let key = &keys[partial.trustee_index as usize];
            let Ok(xonly) = secp256k1::XOnlyPublicKey::from_slice(key) else { return };
            let sig = secp256k1::schnorr::Signature::from_slice(&partial.attestation.signature).expect("length-valid");
            let msg_hash = signing_hash(&partial.attestation.anchored_block, partial.attestation.anchored_daa_score);
            let msg = secp256k1::Message::from_digest(msg_hash.into());
            if sig.verify(&msg, &xonly).is_err() {
                debug!("[SIGNER {}] dropping invalid partial from trustee {}", self.config.trustee_index, partial.trustee_index);
                return;
            }
        }
        self.partials
            .entry((partial.attestation.anchored_block, partial.attestation.anchored_daa_score))
            .or_default()
            .insert(partial.trustee_index, partial.attestation.signature);
    }

    /// One poll: find the deepest selected-chain block at least `depth` behind the
    /// virtual DAA score; sign it iff its score is a full cadence interval past the
    /// last persisted signing.
    async fn tick(&mut self) -> Result<(), String> {
        let dag_info = self.client.get_block_dag_info().await.map_err(|e| e.to_string())?;
        let virtual_daa_score = dag_info.virtual_daa_score;
        // The bound snaps to the cadence grid. Signers on different nodes see tips
        // a few scores apart; an unsnapped bound gave the two hosts of the testnet
        // quorum two different target blocks every interval, so no assembly ever
        // held three partials (2026-09-19). On the grid every signer names the
        // same score, and the same chain block at that depth.
        let target_bound = virtual_daa_score.saturating_sub(self.config.depth) / self.config.interval * self.config.interval;
        if target_bound < self.last_signed.score.saturating_add(self.config.interval) {
            return Ok(()); // cadence: nothing eligible yet
        }

        // No cursor yet (first run, or a reorg reset): walk back from the sink
        // along selected parents to the first chain block at or below the bound,
        // and take that. Walking forward from the pruning point instead came back
        // one page at a time, so a signer joining a synced testnet node signed a
        // block a million scores old and would have crept forward page by page
        // for an hour, attesting stale history the whole way (2026-09-18).
        let cursor = match self.chain_cursor {
            Some(cursor) => cursor,
            None => {
                let mut hash = dag_info.sink;
                let found = loop {
                    let block = self.client.get_block(hash, false).await.map_err(|e| e.to_string())?;
                    if block.header.daa_score <= target_bound {
                        break (hash, block.header.daa_score);
                    }
                    let Some(verbose) = block.verbose_data else { return Err("the node gave a block without verbose data".to_string()) };
                    hash = verbose.selected_parent_hash;
                };
                if found.1 < self.last_signed.score.saturating_add(self.config.interval) || found.0 == self.last_signed.block {
                    return Ok(());
                }
                self.chain_cursor = Some(found.0);
                self.sign_and_share(found.0, found.1).await;
                return Ok(());
            }
        };
        let chain = match self.client.get_virtual_chain_from_block(cursor, false, None).await {
            Ok(res) => res.added_chain_block_hashes,
            Err(_) => {
                // The cursor left the selected chain (deep reorg) — reset and retry next tick
                self.chain_cursor = None;
                return Ok(());
            }
        };
        let mut target: Option<(Hash, u64)> = None;
        for hash in chain.iter().rev() {
            let header = self.client.get_block(*hash, false).await.map_err(|e| e.to_string())?.header;
            if header.daa_score <= target_bound {
                target = Some((*hash, header.daa_score));
                break;
            }
        }
        let Some((target_block, target_score)) = target else { return Ok(()) };
        if target_score < self.last_signed.score.saturating_add(self.config.interval) {
            return Ok(()); // the chain hasn't produced an eligible block a full interval on
        }
        if target_block == self.last_signed.block {
            return Ok(());
        }

        self.chain_cursor = Some(target_block);
        self.sign_and_share(target_block, target_score).await;
        Ok(())
    }

    /// Sign the target, persist FIRST (the equivocation-safety order: better to
    /// lose a signing than to double-sign after a crash), then share the partial.
    async fn sign_and_share(&mut self, target_block: Hash, target_score: u64) {
        let msg = secp256k1::Message::from_digest(signing_hash(&target_block, target_score).into());
        let signature: [u8; 64] = *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, &self.keypair).as_ref();
        self.last_signed = LastSigned { score: target_score, block: target_block };
        write_state(&self.config.state_file, self.last_signed);
        info!(
            "[SIGNER {}] signed anchor attestation for block {} at DAA score {}",
            self.config.trustee_index, target_block, target_score
        );

        let partial = PartialAttestation {
            trustee_index: self.config.trustee_index,
            attestation: AnchorAttestation { anchored_block: target_block, anchored_daa_score: target_score, signature },
        };
        self.record_partial(partial.clone());
        let frame = Arc::new(borsh::to_vec(&partial).expect("borsh serialization cannot fail"));
        for peer in self.config.peer_addresses.clone() {
            let frame = frame.clone();
            tokio::spawn(send_partial(peer, frame));
        }
    }

    /// Assembles and submits an anchor for any pair holding a quorum of partials.
    async fn try_submit(&mut self) -> Result<(), String> {
        let Some(((block, score), sigs)) = self
            .partials
            .iter()
            .filter(|((_, score), sigs)| *score > self.last_submitted_score && sigs.len() >= ANCHOR_QUORUM)
            .max_by_key(|((_, score), _)| *score)
            .map(|(k, v)| (*k, v.clone()))
        else {
            return Ok(());
        };
        let mut bitmap = 0u8;
        let mut signatures = Vec::with_capacity(sigs.len());
        for (index, signature) in sigs.iter() {
            bitmap |= 1 << index;
            signatures.push(*signature);
        }
        let anchor = FinalityAnchor { anchored_block: block, anchored_daa_score: score, signer_bitmap: bitmap, signatures };
        let tx = anchor_transaction(&anchor);
        match self.client.submit_transaction((&tx).into(), false).await {
            Ok(_) => {
                info!(
                    "[SIGNER {}] submitted {}-of-{} anchor for block {} at DAA score {}",
                    self.config.trustee_index,
                    sigs.len(),
                    TRUSTEE_COUNT,
                    block,
                    score
                );
                self.last_submitted_score = score;
                // Old aggregation state below the submitted score is done with
                self.partials.retain(|(_, s), _| *s > score);
            }
            Err(err) => {
                let message = err.to_string();
                if message.contains("already") {
                    // Another signer's identical assembly beat us to the mempool — success
                    self.last_submitted_score = score;
                    self.partials.retain(|(_, s), _| *s > score);
                } else {
                    return Err(message);
                }
            }
        }
        Ok(())
    }
}
