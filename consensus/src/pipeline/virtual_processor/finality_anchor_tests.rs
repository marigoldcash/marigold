//! Consensus-level finality-anchor tests (FORK-PLAN P6.11's verify criteria,
//! POOL-SPEC.md P5.8): a heavier attacker chain lacking the latest anchor loses to the
//! anchored chain; an equivocating quorum is ignored after its proof is processed;
//! anchors past the hard sunset score are rejected; and anchor-free/stale operation
//! degrades to plain PoW without halting.
//!
//! Context-free verification rules (quorum shape, signature validity, the exact
//! equivocation overlap boundaries) are unit-tested in `consensus-core`'s
//! `finality_anchor` module; these tests cover the pipeline wiring end to end —
//! isolation validation, acceptance-time application (ratchet + deny-list), and the
//! fork-choice override in `sink_search_algorithm`.
//!
//! DAG-acceptance timing note, load-bearing for every test here: a block's own
//! transactions are accepted by its chain *descendants* (its own acceptance data
//! covers its mergeset, never its own body). An anchor mined into a block therefore
//! takes effect exactly one chain block later — hence the carrier-block + confirming
//! block pattern (`mine_anchor_and_confirm`).

use crate::config::ConfigBuilder;
use crate::consensus::test_consensus::TestConsensus;
use kaspa_consensus_core::{
    api::ConsensusApi,
    config::params::{FinalityAnchorParams, MAINNET_PARAMS},
    constants::TX_VERSION_TOCCATA,
    finality_anchor::{AnchorAttestation, AnchorPayload, EquivocationEvidence, FinalityAnchor, TRUSTEE_COUNT, signing_hash},
    mass::MassCalculator,
    subnets::SUBNETWORK_ID_FINALITY_ANCHOR,
    tx::Transaction,
};
use kaspa_hashes::Hash;

/// Deterministic trustee keypairs (seeds chosen not to collide with
/// `notepool_tests`'s wallet seeds — irrelevant in practice, tidy in principle).
fn trustee_keypairs() -> Vec<secp256k1::Keypair> {
    (101..=100 + TRUSTEE_COUNT as u8)
        .map(|seed| secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[seed; 32]).unwrap())
        .collect()
}

fn trustee_keys(keypairs: &[secp256k1::Keypair]) -> kaspa_consensus_core::finality_anchor::TrusteeKeys {
    let keys: Vec<[u8; 32]> = keypairs.iter().map(|kp| kp.public_key().x_only_public_key().0.serialize()).collect();
    keys.try_into().unwrap()
}

/// Signs the finality-anchor message for one trustee (deterministic, no aux rand —
/// same reasoning as `notepool_tests::sign_deterministic`).
fn attest(keypair: &secp256k1::Keypair, block: Hash, score: u64) -> [u8; 64] {
    let msg = secp256k1::Message::from_digest(signing_hash(&block, score).into());
    *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, keypair).as_ref()
}

fn sign_anchor(keypairs: &[secp256k1::Keypair], signers: &[u8], block: Hash, score: u64) -> FinalityAnchor {
    let mut sorted = signers.to_vec();
    sorted.sort_unstable();
    FinalityAnchor {
        anchored_block: block,
        anchored_daa_score: score,
        signer_bitmap: sorted.iter().fold(0u8, |bitmap, &i| bitmap | (1 << i)),
        signatures: sorted.iter().map(|&i| attest(&keypairs[i as usize], block, score)).collect(),
    }
}

/// A zero-input, zero-output anchor-lane transaction carrying `payload` — the
/// trustee-signature-authorized shape the isolation rules explicitly allow, mirroring
/// pure pool Transfers (see `check_transaction_inputs_count`).
fn anchor_tx(payload: &AnchorPayload) -> Transaction {
    let tx = Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_FINALITY_ANCHOR, 0, payload.encode_payload());
    let populated = kaspa_consensus_core::tx::PopulatedTransaction::new(&tx, vec![]);
    let storage_mass =
        MassCalculator::new_with_consensus_params(&MAINNET_PARAMS).calc_contextual_masses(&populated).unwrap().storage_mass;
    tx.set_storage_mass(storage_mass);
    tx
}

fn evidence_tx(
    keypairs: &[secp256k1::Keypair],
    trustee_index: u8,
    block_1: Hash,
    score_1: u64,
    block_2: Hash,
    score_2: u64,
) -> Transaction {
    let evidence = EquivocationEvidence {
        trustee_index,
        first: AnchorAttestation {
            anchored_block: block_1,
            anchored_daa_score: score_1,
            signature: attest(&keypairs[trustee_index as usize], block_1, score_1),
        },
        second: AnchorAttestation {
            anchored_block: block_2,
            anchored_daa_score: score_2,
            signature: attest(&keypairs[trustee_index as usize], block_2, score_2),
        },
    };
    anchor_tx(&AnchorPayload::Equivocation(evidence))
}

/// Test params: trustees pinned, small depth/interval so tests need few blocks. The
/// staleness bound is `depth + 3×interval`.
fn config(anchor_params: FinalityAnchorParams) -> crate::config::Config {
    ConfigBuilder::new(MAINNET_PARAMS)
        .skip_proof_of_work()
        .edit_consensus_params(|p| {
            p.max_block_parents = 4;
            p.mergeset_size_limit = 10;
            p.coinbase_maturity = 0;
            p.finality_anchor = anchor_params;
        })
        .build()
}

fn keyed_params(keypairs: &[secp256k1::Keypair], depth: u64, interval: u64, hard_expiry: u64) -> FinalityAnchorParams {
    FinalityAnchorParams {
        trustees: Some(trustee_keys(keypairs)),
        depth,
        launch_interval: interval,
        hard_expiry_daa_score: hard_expiry,
        ..FinalityAnchorParams::LAUNCH_UNKEYED
    }
}

/// Builds a linear chain of empty blocks `first_hash..first_hash+count` on `tip`,
/// returning the new tip.
async fn extend_chain(consensus: &TestConsensus, tip: Hash, first_hash: u64, count: u64) -> Hash {
    let mut tip = tip;
    for i in 0..count {
        let hash: Hash = (first_hash + i).into();
        consensus.add_utxo_valid_block_with_parents(hash, vec![tip], vec![]).await.unwrap();
        tip = hash;
    }
    tip
}

/// Mines `txs` into a carrier block (`carrier_hash` on `tip`) plus one confirming
/// block on top — the block whose acceptance actually processes the carrier's
/// transactions (see the module doc's acceptance-timing note). Returns the new tip.
async fn mine_and_confirm(consensus: &TestConsensus, tip: Hash, carrier_hash: u64, txs: Vec<Transaction>) -> Hash {
    consensus.add_utxo_valid_block_with_parents(carrier_hash.into(), vec![tip], txs).await.unwrap();
    let confirm: Hash = (carrier_hash + 1).into();
    consensus.add_utxo_valid_block_with_parents(confirm, vec![carrier_hash.into()], vec![]).await.unwrap();
    confirm
}

fn daa_score_of(consensus: &TestConsensus, block: Hash) -> u64 {
    consensus.get_block(block).unwrap().header.daa_score
}

/// P6.11 verify criterion 1: a heavier attacker chain lacking the latest anchor loses
/// to the anchored chain — and (anti-vacuity) the identical DAG WITHOUT the anchor
/// reorgs to the attacker chain, proving the anchor is what made the difference.
#[tokio::test]
async fn heavier_attacker_chain_lacking_anchor_loses() {
    let keypairs = trustee_keypairs();

    // Anchored node: sees the honest chain, an anchor for a block on it, then a
    // heavier conflicting chain.
    let consensus = TestConsensus::new(&config(keyed_params(&keypairs, 2, 50, u64::MAX)));
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    // Honest chain: blocks 10..17. The anchor certifies block 12 (well behind the tip).
    let honest_tip = extend_chain(&consensus, genesis, 10, 8).await;
    let anchored_block: Hash = 12.into();
    let anchored_score = daa_score_of(&consensus, anchored_block);

    let anchor = sign_anchor(&keypairs, &[0, 1, 2], anchored_block, anchored_score);
    let honest_tip = mine_and_confirm(&consensus, honest_tip, 30, vec![anchor_tx(&AnchorPayload::Anchor(anchor))]).await;

    let status = consensus.get_finality_anchor_status();
    assert_eq!(status.latest_anchor, Some((anchored_block, anchored_score)), "the mined anchor must ratchet");
    assert!(status.enforcing, "a fresh anchor must be enforced");
    assert!(!status.stale);

    // Attacker chain: forks from block 11 (before the anchored block), far heavier
    // than the honest chain (25 blocks vs 10).
    let mut attacker_tip: Hash = 11.into();
    for i in 0..25u64 {
        let hash: Hash = (100 + i).into();
        consensus.add_utxo_valid_block_with_parents(hash, vec![attacker_tip], vec![]).await.unwrap();
        attacker_tip = hash;
    }

    // The sink must still be on the anchored chain, work notwithstanding.
    assert_eq!(consensus.get_sink(), honest_tip, "the anchored chain must remain selected against a heavier anchor-free chain");
    assert!(consensus.get_finality_anchor_status().enforcing);

    // Anti-vacuity control: the identical DAG on a node with NO trustee keys pinned
    // (mechanism inert) reorgs to the attacker chain — plain PoW behavior.
    let control = TestConsensus::new(&config(FinalityAnchorParams::LAUNCH_UNKEYED));
    let control_handles = control.init();
    let control_genesis = control.params().genesis.hash;
    let control_honest_tip = extend_chain(&control, control_genesis, 10, 8).await;
    control.add_utxo_valid_block_with_parents(30.into(), vec![control_honest_tip], vec![]).await.unwrap();
    control.add_utxo_valid_block_with_parents(31.into(), vec![30.into()], vec![]).await.unwrap();
    let mut control_attacker_tip: Hash = 11.into();
    for i in 0..25u64 {
        let hash: Hash = (100 + i).into();
        control.add_utxo_valid_block_with_parents(hash, vec![control_attacker_tip], vec![]).await.unwrap();
        control_attacker_tip = hash;
    }
    assert_eq!(
        control.get_sink(),
        control_attacker_tip,
        "without the anchor mechanism the heavier chain must win — the anchor is what made the difference above"
    );

    consensus.shutdown(join_handles);
    control.shutdown(control_handles);
}

/// P6.11 verify criterion 2: an equivocating quorum is ignored after its proof is
/// processed — evidence permanently disqualifies the keys, later anchors from them
/// (in any signer combination leaving fewer than 3 countable signatures) have no
/// effect, and the ratchet freezes.
#[tokio::test]
async fn equivocating_quorum_ignored_after_proof() {
    let keypairs = trustee_keypairs();
    let consensus = TestConsensus::new(&config(keyed_params(&keypairs, 2, 50, u64::MAX)));
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let tip = extend_chain(&consensus, genesis, 10, 8).await;
    let anchored_block: Hash = 12.into();
    let anchored_score = daa_score_of(&consensus, anchored_block);
    let anchor = sign_anchor(&keypairs, &[0, 1, 2], anchored_block, anchored_score);
    let tip = mine_and_confirm(&consensus, tip, 30, vec![anchor_tx(&AnchorPayload::Anchor(anchor))]).await;
    assert_eq!(consensus.get_finality_anchor_status().latest_anchor, Some((anchored_block, anchored_score)));

    // Trustees 0, 1, 2 each double-sign: two different (synthetic) blocks at scores
    // within one cadence interval — the exact self-contained proof shape. The
    // equivocation blocks need not exist anywhere; the proof is pure signed data.
    let evidence_txs: Vec<Transaction> =
        (0..3u8).map(|i| evidence_tx(&keypairs, i, Hash::from(777u64), 1_000, Hash::from(778u64), 1_010)).collect();
    let tip = mine_and_confirm(&consensus, tip, 40, evidence_txs).await;

    let status = consensus.get_finality_anchor_status();
    assert_eq!(status.disqualified, vec![0, 1, 2], "all three equivocating keys must be permanently disqualified");
    assert_eq!(
        status.latest_anchor,
        Some((anchored_block, anchored_score)),
        "prior accepted anchors are unaffected (no retroactive re-validation)"
    );

    // A fresh anchor signed by the disqualified quorum: cryptographically valid, so
    // the transaction is accepted — but with zero countable signers it has no effect.
    let later_block: Hash = 15.into();
    let later_score = daa_score_of(&consensus, later_block);
    let dead_anchor = sign_anchor(&keypairs, &[0, 1, 2], later_block, later_score);
    let tip = mine_and_confirm(&consensus, tip, 50, vec![anchor_tx(&AnchorPayload::Anchor(dead_anchor))]).await;
    assert_eq!(
        consensus.get_finality_anchor_status().latest_anchor,
        Some((anchored_block, anchored_score)),
        "an anchor from a fully disqualified quorum must not ratchet"
    );

    // A mixed anchor — one disqualified signer plus the two clean keys — leaves only
    // 2 countable signatures, below the k=3 quorum: also no effect. With 3 of 5 keys
    // dead, no valid quorum can ever form again short of a hard fork.
    let mixed_anchor = sign_anchor(&keypairs, &[2, 3, 4], later_block, later_score);
    let _tip = mine_and_confirm(&consensus, tip, 60, vec![anchor_tx(&AnchorPayload::Anchor(mixed_anchor))]).await;
    assert_eq!(
        consensus.get_finality_anchor_status().latest_anchor,
        Some((anchored_block, anchored_score)),
        "an anchor with fewer than 3 countable (non-disqualified) signers must not ratchet"
    );

    consensus.shutdown(join_handles);
}

/// P6.11 verify criterion 3: anchors past the hard sunset score are rejected — as
/// transactions (isolation: a certified score at/beyond expiry is invalid), and the
/// enforcement itself retires unconditionally once the chain's own score passes the
/// hard maximum, letting plain PoW take over ("trust must end even if network growth
/// disappoints").
#[tokio::test]
async fn anchors_past_sunset_rejected_and_enforcement_retires() {
    let keypairs = trustee_keypairs();
    // Hard expiry at DAA score 15; depth 2, interval 100 (staleness never triggers
    // within this test — expiry is what must end enforcement).
    let consensus = TestConsensus::new(&config(keyed_params(&keypairs, 2, 100, 15)));
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let tip = extend_chain(&consensus, genesis, 10, 8).await;

    // An anchor certifying a score at/beyond the expiry is an invalid transaction
    // outright, regardless of signatures — body-in-isolation validation rejects the
    // block carrying it.
    let expired_anchor = sign_anchor(&keypairs, &[0, 1, 2], 15.into(), 20);
    let result = consensus.add_utxo_valid_block_with_parents(98.into(), vec![tip], vec![anchor_tx(&AnchorPayload::Anchor(expired_anchor))]).await;
    assert!(result.is_err(), "an anchor certifying a score at/beyond the hard expiry must be rejected as a transaction");

    // A pre-expiry anchor works normally...
    let anchored_block: Hash = 12.into();
    let anchored_score = daa_score_of(&consensus, anchored_block);
    let anchor = sign_anchor(&keypairs, &[0, 1, 2], anchored_block, anchored_score);
    let tip = mine_and_confirm(&consensus, tip, 30, vec![anchor_tx(&AnchorPayload::Anchor(anchor))]).await;
    let status = consensus.get_finality_anchor_status();
    assert_eq!(status.latest_anchor, Some((anchored_block, anchored_score)));
    assert!(status.enforcing);

    // ...until the chain's own DAA score reaches the hard maximum: the keys are
    // consensus-expired, enforcement is off, and a heavier anchor-free fork now wins
    // on plain PoW despite conflicting with the (still-ratcheted) anchor.
    let _post_expiry_tip = extend_chain(&consensus, tip, 40, 10).await;
    let status = consensus.get_finality_anchor_status();
    assert!(status.expired, "the hard expiry must engage once virtual's DAA score reaches it");
    assert!(!status.enforcing);

    let mut attacker_tip: Hash = 11.into();
    for i in 0..40u64 {
        let hash: Hash = (100 + i).into();
        consensus.add_utxo_valid_block_with_parents(hash, vec![attacker_tip], vec![]).await.unwrap();
        attacker_tip = hash;
    }
    assert_eq!(consensus.get_sink(), attacker_tip, "post-expiry, a heavier chain must win regardless of the retired anchor");

    consensus.shutdown(join_handles);
}

/// P6.11 verify criterion 4: anchor-free operation degrades to plain PoW without
/// halting — both the never-anchored case and the fail-open case where the latest
/// anchor goes stale, after which a deep reorg is allowed again and blocks keep
/// flowing throughout.
#[tokio::test]
async fn anchor_free_and_stale_operation_degrade_to_plain_pow() {
    let keypairs = trustee_keypairs();
    // depth 2, interval 5 => staleness bound 2 + 3×5 = 17 DAA-score units.
    let consensus = TestConsensus::new(&config(keyed_params(&keypairs, 2, 5, u64::MAX)));
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    // (a) Keys pinned but no anchor ever arrives: nothing is enforced, chains flow,
    // heavier forks win — indistinguishable from a chain without the mechanism.
    let _tip = extend_chain(&consensus, genesis, 10, 6).await;
    let status = consensus.get_finality_anchor_status();
    assert_eq!(status.latest_anchor, None);
    assert!(!status.enforcing);
    assert!(!status.stale, "having never seen an anchor is not the stale condition — nothing was lost");

    let mut fork_tip: Hash = 12.into();
    for i in 0..10u64 {
        let hash: Hash = (200 + i).into();
        consensus.add_utxo_valid_block_with_parents(hash, vec![fork_tip], vec![]).await.unwrap();
        fork_tip = hash;
    }
    assert_eq!(consensus.get_sink(), fork_tip, "with no anchor ever accepted, plain PoW governs");

    // (b) An anchor arrives and is enforced...
    let anchored_block: Hash = (205u64).into();
    let anchored_score = daa_score_of(&consensus, anchored_block);
    let anchor = sign_anchor(&keypairs, &[1, 2, 3], anchored_block, anchored_score);
    let tip = mine_and_confirm(&consensus, fork_tip, 300, vec![anchor_tx(&AnchorPayload::Anchor(anchor))]).await;
    let status = consensus.get_finality_anchor_status();
    assert_eq!(status.latest_anchor, Some((anchored_block, anchored_score)));
    assert!(status.enforcing);

    // ...then no further anchors arrive while the chain keeps growing past the
    // staleness bound: fail-open engages automatically — no halt, no operator action,
    // just a loud alert and plain PoW security.
    let _stale_tip = extend_chain(&consensus, tip, 400, 25).await;
    let status = consensus.get_finality_anchor_status();
    assert!(status.stale, "the anchor must be reported stale once the chain outruns the staleness bound");
    assert!(!status.enforcing);

    // With fail-open engaged, a heavier fork conflicting with the stale anchor is
    // adopted again — the rule "simply stops being enforced", exactly as if the
    // mechanism didn't exist.
    let mut late_attacker_tip: Hash = (204u64).into(); // forks BELOW the anchored block
    for i in 0..50u64 {
        let hash: Hash = (500 + i).into();
        consensus.add_utxo_valid_block_with_parents(hash, vec![late_attacker_tip], vec![]).await.unwrap();
        late_attacker_tip = hash;
    }
    assert_eq!(consensus.get_sink(), late_attacker_tip, "once the anchor is stale, plain PoW must govern again (fail-open)");
    assert_eq!(
        consensus.get_finality_anchor_status().latest_anchor,
        Some((anchored_block, anchored_score)),
        "fail-open never erases the ratchet — the anchor is stale, not forgotten"
    );

    consensus.shutdown(join_handles);
}

/// On a network with no trustee keys pinned (every network until the P9.1 ceremony),
/// the anchor lane is closed outright: any anchor-subnetwork transaction is invalid.
#[tokio::test]
async fn anchor_lane_closed_without_pinned_keys() {
    let keypairs = trustee_keypairs();
    let consensus = TestConsensus::new(&config(FinalityAnchorParams::LAUNCH_UNKEYED));
    let join_handles = consensus.init();
    let genesis = consensus.params().genesis.hash;

    let anchor = sign_anchor(&keypairs, &[0, 1, 2], 5.into(), 1);
    let result =
        consensus.add_utxo_valid_block_with_parents(10.into(), vec![genesis], vec![anchor_tx(&AnchorPayload::Anchor(anchor))]).await;
    assert!(result.is_err(), "the anchor lane must be closed while no trustee keys are pinned");

    consensus.shutdown(join_handles);
}
