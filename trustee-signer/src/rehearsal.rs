//! The anchor forgery rehearsal (PLAN P8.4a, 2026-09-22): every shape of bad
//! anchor a stranger could submit, sent to a live node through the same RPC the
//! signers use, each expected to be refused. No trustee key is needed, and no
//! forgery can be mistaken for an anchor by a correct node — that is the point of
//! running it against the real testnet: the node in front of it must not move its
//! ratchet by a single block.
//!
//! What is submitted, in order:
//! - an anchor signed by three strangers (the pinned keys are not theirs);
//! - one signed by two strangers (below the quorum, refused before any signature is
//!   looked at);
//! - a bitmap naming a sixth trustee; a bitmap and signature count that disagree;
//! - a payload that is not an anchor at all;
//! - equivocation evidence that accuses a trustee of certifying the same block
//!   twice, and evidence signed by strangers;
//! - the newest real anchor found on the chain, replayed as it is (valid, but stale:
//!   no effect), with its score moved by one (the real signatures no longer match),
//!   and with its signatures cut to two (a real quorum reduced below quorum).
//!
//! The report names each case, what the node answered, and whether the anchor
//! status changed across the run. The exit code is non-zero if anything was
//! accepted that should not have been, or if the ratchet moved.

use crate::{DynRpcApi, anchor_transaction};
use kaspa_consensus_core::finality_anchor::{AnchorAttestation, AnchorPayload, EquivocationEvidence, FinalityAnchor, signing_hash};
use kaspa_consensus_core::{
    config::params::MAINNET_PARAMS, constants::TX_VERSION_TOCCATA, mass::MassCalculator, subnets::SUBNETWORK_ID_FINALITY_ANCHOR,
    tx::Transaction,
};
use kaspa_hashes::Hash;
use std::sync::Arc;

/// How far back along the selected chain the rehearsal looks for a real anchor to
/// replay: a few cadence intervals at launch cadence.
const REAL_ANCHOR_SEARCH_BLOCKS: usize = 1_500;

fn stranger(seed: u8) -> secp256k1::Keypair {
    secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[seed; 32]).expect("a fixed non-zero seed is a valid key")
}

fn attest(keypair: &secp256k1::Keypair, block: Hash, score: u64) -> [u8; 64] {
    let msg = secp256k1::Message::from_digest(signing_hash(&block, score).into());
    *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, keypair).as_ref()
}

fn raw_anchor_lane_transaction(payload: Vec<u8>) -> Transaction {
    let tx = Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_FINALITY_ANCHOR, 0, payload);
    let populated = kaspa_consensus_core::tx::PopulatedTransaction::new(&tx, vec![]);
    let storage_mass =
        MassCalculator::new_with_consensus_params(&MAINNET_PARAMS).calc_contextual_masses(&populated).unwrap().storage_mass;
    tx.set_storage_mass(storage_mass);
    tx
}

/// One submission and its verdict.
pub struct CaseResult {
    pub name: &'static str,
    pub expect_refusal: bool,
    pub refused: bool,
    pub answer: String,
}

impl CaseResult {
    pub fn passed(&self) -> bool {
        self.refused == self.expect_refusal
    }
}

pub struct Report {
    pub cases: Vec<CaseResult>,
    pub status_before: String,
    pub status_after: String,
    pub ratchet_moved: bool,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.cases.iter().all(CaseResult::passed) && !self.ratchet_moved
    }
}

async fn submit(client: &Arc<DynRpcApi>, name: &'static str, expect_refusal: bool, tx: Transaction) -> CaseResult {
    match client.submit_transaction((&tx).into(), false).await {
        Ok(id) => CaseResult { name, expect_refusal, refused: false, answer: format!("accepted as {id}") },
        Err(err) => CaseResult { name, expect_refusal, refused: true, answer: format!("refused: {err}") },
    }
}

/// Walks the selected chain back from the sink looking for the newest anchor
/// transaction; returns it with the block it was found in.
async fn find_real_anchor(client: &Arc<DynRpcApi>, sink: Hash) -> Option<(FinalityAnchor, Hash)> {
    let mut cursor = sink;
    for _ in 0..REAL_ANCHOR_SEARCH_BLOCKS {
        let block = client.get_block(cursor, true).await.ok()?;
        for tx in &block.transactions {
            if tx.subnetwork_id == SUBNETWORK_ID_FINALITY_ANCHOR
                && let Some(AnchorPayload::Anchor(anchor)) = AnchorPayload::decode_payload(&tx.payload)
            {
                return Some((anchor, cursor));
            }
        }
        cursor = block.verbose_data?.selected_parent_hash;
    }
    None
}

async fn anchor_status_line(client: &Arc<DynRpcApi>) -> Result<(String, u64), String> {
    let status = client.get_finality_anchor_status().await.map_err(|e| e.to_string())?;
    Ok((
        format!(
            "anchor at DAA {} ({}), enforcing {}, stale {}, disqualified {:?}",
            status.latest_anchored_daa_score,
            status.latest_anchored_block,
            status.enforcing,
            status.stale,
            status.disqualified_trustees
        ),
        status.latest_anchored_daa_score,
    ))
}

/// The drill's one-line status: where the node's sink is and what it anchors to.
pub async fn drill_status_line(client: &Arc<DynRpcApi>) -> Result<String, String> {
    let dag = client.get_block_dag_info().await.map_err(|e| e.to_string())?;
    let sink_score = client.get_block(dag.sink, false).await.map_err(|e| e.to_string())?.header.daa_score;
    let (anchor, _) = anchor_status_line(client).await?;
    Ok(format!("sink {} at DAA {} (virtual {}), {} blocks; {}", dag.sink, sink_score, dag.virtual_daa_score, dag.block_count, anchor))
}

pub async fn rehearse_forgeries(client: &Arc<DynRpcApi>) -> Result<Report, String> {
    let (status_before, _) = anchor_status_line(client).await?;
    let dag = client.get_block_dag_info().await.map_err(|e| e.to_string())?;
    let sink = dag.sink;
    let sink_score = client.get_block(sink, false).await.map_err(|e| e.to_string())?.header.daa_score;
    let strangers: Vec<_> = (101u8..=103).map(stranger).collect();
    let sigs = |n: usize| -> Vec<[u8; 64]> { strangers.iter().take(n).map(|k| attest(k, sink, sink_score)).collect() };
    let mut cases = Vec::new();

    let anchor = |bitmap: u8, signatures: Vec<[u8; 64]>| FinalityAnchor {
        anchored_block: sink,
        anchored_daa_score: sink_score,
        signer_bitmap: bitmap,
        signatures,
    };
    cases.push(submit(client, "three strangers sign", true, anchor_transaction(&anchor(0b00111, sigs(3)))).await);
    cases.push(submit(client, "two strangers sign (below quorum)", true, anchor_transaction(&anchor(0b00011, sigs(2)))).await);
    cases.push(submit(client, "bitmap names a sixth trustee", true, anchor_transaction(&anchor(0b100111, sigs(4)))).await);
    cases.push(submit(client, "bitmap and signature count disagree", true, anchor_transaction(&anchor(0b00111, sigs(2)))).await);
    cases.push(submit(client, "payload is not an anchor", true, raw_anchor_lane_transaction(b"not an anchor at all".to_vec())).await);

    let evidence = |first: (Hash, u64), second: (Hash, u64)| {
        let tx = Transaction::new(
            TX_VERSION_TOCCATA,
            vec![],
            vec![],
            0,
            SUBNETWORK_ID_FINALITY_ANCHOR,
            0,
            AnchorPayload::Equivocation(EquivocationEvidence {
                trustee_index: 0,
                first: AnchorAttestation {
                    anchored_block: first.0,
                    anchored_daa_score: first.1,
                    signature: attest(&strangers[0], first.0, first.1),
                },
                second: AnchorAttestation {
                    anchored_block: second.0,
                    anchored_daa_score: second.1,
                    signature: attest(&strangers[0], second.0, second.1),
                },
            })
            .encode_payload(),
        );
        let populated = kaspa_consensus_core::tx::PopulatedTransaction::new(&tx, vec![]);
        let storage_mass =
            MassCalculator::new_with_consensus_params(&MAINNET_PARAMS).calc_contextual_masses(&populated).unwrap().storage_mass;
        tx.set_storage_mass(storage_mass);
        tx
    };
    cases.push(submit(client, "evidence accusing the same block twice", true, evidence((sink, sink_score), (sink, sink_score))).await);
    let other = dag.pruning_point_hash;
    cases.push(
        submit(client, "evidence signed by a stranger", true, evidence((sink, sink_score), (other, sink_score.saturating_sub(10))))
            .await,
    );

    match find_real_anchor(client, sink).await {
        Some((real, found_in)) => {
            let signer_count = real.signatures.len();
            cases.push(CaseResult {
                name: "a real anchor found on the chain",
                expect_refusal: false,
                refused: false,
                answer: format!(
                    "DAA {} block {} with {} signatures, in block {}",
                    real.anchored_daa_score, real.anchored_block, signer_count, found_in
                ),
            });
            // Replayed as it is: valid, and the node may take the duplicate or say it
            // has it already; either way the ratchet does not move (checked below).
            let replay = submit(client, "the real anchor replayed unchanged", false, anchor_transaction(&real)).await;
            let replay_known = replay.answer.contains("already") || replay.answer.contains("duplicate");
            cases.push(CaseResult { refused: replay.refused && !replay_known, ..replay });
            let mut moved = real.clone();
            moved.anchored_daa_score += 1;
            cases.push(submit(client, "the real anchor with its score moved by one", true, anchor_transaction(&moved)).await);
            if signer_count >= 3 {
                let mut cut = real.clone();
                let keep = 2;
                let mut bitmap = 0u8;
                let mut kept = 0;
                for i in 0..8u8 {
                    if cut.signer_bitmap & (1 << i) != 0 {
                        if kept < keep {
                            bitmap |= 1 << i;
                        }
                        kept += 1;
                    }
                }
                cut.signer_bitmap = bitmap;
                cut.signatures.truncate(keep);
                cases.push(submit(client, "the real anchor cut to two of its signatures", true, anchor_transaction(&cut)).await);
            }
        }
        None => cases.push(CaseResult {
            name: "a real anchor found on the chain",
            expect_refusal: false,
            refused: true,
            answer: format!("none within {REAL_ANCHOR_SEARCH_BLOCKS} blocks of the sink"),
        }),
    }

    // Give the node a moment to mine anything it accepted, then compare.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let (status_after, score_after) = anchor_status_line(client).await?;
    // A real anchor landing during the run advances the ratchet legitimately; what
    // must not happen is a jump to the forgeries' target, the sink score itself.
    let ratchet_moved = score_after == sink_score || score_after == sink_score + 1;
    Ok(Report { cases, status_before, status_after, ratchet_moved })
}
