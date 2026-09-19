//!
//! # Finality anchors
//!
//! Wire types and verification for Marigold's launch finality anchors
//! (POOL-SPEC.md P5.8, PLAN P6.11). **Not a note-pool feature** — a chain-level
//! security mechanism: a 3-of-5 trustee quorum periodically countersigns a block
//! already settled ~600 DAA-score-units deep, and a chain conflicting with the latest
//! valid anchor is invalid regardless of accumulated work. The trustees can only veto
//! — they produce no blocks, hold no reward, and their authority is bounded in time by
//! a staged sunset and an unconditional 20-year hard expiry.
//!
//! Naming note, straight from the spec: this module's *finality anchors* are unrelated
//! to the note pool's *freshness anchors* (`notepool::FreshnessAnchor`, replay
//! protection) — same word, different mechanism, never conflate them.
//!
//! Everything here is deliberately context-free: an anchor or an equivocation proof is
//! a self-contained signed statement, verifiable from its own bytes plus the trustee
//! keys pinned in [`crate::config::params::Params`]. Chain-contextual rules (depth,
//! chain membership of the anchored block, the deny-list's POV scoping, the fail-open
//! staleness bound) live in the virtual processor, which decides what *effect* a
//! verified statement has — see `consensus/src/pipeline/virtual_processor`.

use crate::Hash;
use borsh::{BorshDeserialize, BorshSerialize};
use kaspa_hashes::{FinalityAnchorSigningHash, HasherBase};
use thiserror::Error;

/// The trustee quorum shape: n = 5 keys, any k = 3 of which must sign (POOL-SPEC.md
/// P5.8's "3-of-5"). Fixed by the spec; changing either is a hard fork.
pub const TRUSTEE_COUNT: usize = 5;
pub const ANCHOR_QUORUM: usize = 3;

/// The trustee public-key set as pinned in params: BIP340 x-only keys, index i = bit i
/// of every [`FinalityAnchor::signer_bitmap`].
pub type TrusteeKeys = [[u8; 32]; TRUSTEE_COUNT];

/// P5.8's launch-stage anchor parameters, in DAA-score units (never wall-clock — the
/// v1.1 spec redefinition that makes the equivocation rule exactly decidable).
pub const FINALITY_ANCHOR_DEPTH: u64 = 600;
pub const FINALITY_ANCHOR_LAUNCH_INTERVAL: u64 = 300;

/// The fail-open staleness bound is `depth + STALENESS_INTERVALS × interval` where
/// `interval` is the currently-active decay stage's cadence (POOL-SPEC.md P5.8,
/// "Fail-open liveness": `600 + 3×300 = 1,500` at launch).
pub const ANCHOR_STALENESS_INTERVALS: u64 = 3;

/// The hard maximum DAA score (POOL-SPEC.md P5.8, "The hard-coded sunset"): 20 years
/// from genesis at 10 BPS (`20 × 12 × 2,629,800 × 10`). At and beyond this score the
/// trustee keys are consensus-expired unconditionally — no anchor, however validly
/// signed, has any consensus effect. Extending trustee life past this score requires an
/// explicit hard fork; the default, unforced outcome is always expiry.
pub const FINALITY_ANCHOR_HARD_EXPIRY_DAA_SCORE: u64 = 6_311_520_000;

/// The staged cadence-decay schedule's intervals (stages 1–3 of the P5.8 table;
/// stage 0 is [`FINALITY_ANCHOR_LAUNCH_INTERVAL`], stage 4 is the hard expiry).
/// The *triggers* for stages 1–2 are difficulty-dependent (sustained ≥ T for M months,
/// plus the K-year floor) and are represented as `ForkActivation` scores in params —
/// see `Params::finality_anchor_decay_stages` for why their evaluation is deferred.
pub const ANCHOR_STAGE_INTERVALS: [u64; 3] = [36_000, 864_000, 6_048_000];

/// A 3-of-5 trustee certification of an already-settled block (POOL-SPEC.md P5.8's
/// exact struct, field for field). Carried as the borsh payload of a transaction in
/// [`crate::subnets::SUBNETWORK_ID_FINALITY_ANCHOR`] (and, from P6.12, gossiped raw
/// over P2P — anchors are just signed data; mining is one distribution channel).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct FinalityAnchor {
    /// The block being certified — per the spec, ≥ `depth` DAA-score-units behind the
    /// signer's view of the tip at signing time (a receiving node enforces depth
    /// against its own accepting context, the deterministically checkable analog).
    pub anchored_block: Hash,
    /// That block's DAA score, for unambiguous depth/cadence arithmetic. Cross-checked
    /// against the actual stored header at application time.
    pub anchored_daa_score: u64,
    /// Which of the 5 trustee keys signed: bit i = trustee i. A bitmap cannot express
    /// a repeated signer, which is exactly why the spec chose it.
    pub signer_bitmap: u8,
    /// BIP340 Schnorr signatures, one per set bit of `signer_bitmap`, in ascending bit
    /// order, each over [`signing_hash`] of (`anchored_block`, `anchored_daa_score`).
    pub signatures: Vec<[u8; 64]>,
}

/// One signed (block, score) attestation — the unit the equivocation rule compares.
/// The `FinalityAnchor` signing hash covers exactly these two fields, so a single
/// trustee's contribution to any anchor is fully captured by this triple.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct AnchorAttestation {
    pub anchored_block: Hash,
    pub anchored_daa_score: u64,
    pub signature: [u8; 64],
}

/// Self-contained equivocation proof (POOL-SPEC.md P5.8, "Evidence format"): two
/// validly-signed attestations from the *same* trustee key certifying different blocks
/// at DAA scores less than one cadence interval apart — behavior no honest,
/// once-per-interval signing procedure produces. Verifiable by any node from these
/// bytes plus params alone; submittable by anyone (enforcement never depends on the
/// honest trustees' cooperation).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct EquivocationEvidence {
    /// The accused trustee (0..5, an index into the pinned key set).
    pub trustee_index: u8,
    pub first: AnchorAttestation,
    pub second: AnchorAttestation,
}

/// The anchor subnetwork's transaction payload: either an anchor or an equivocation
/// proof ("broadcast as a transaction in the same dedicated anchor subnetwork", P5.8).
/// Borsh discriminants by declaration order, same convention as `notepool::PoolOp`.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum AnchorPayload {
    Anchor(FinalityAnchor),
    Equivocation(EquivocationEvidence),
}

impl FinalityAnchor {
    /// The anchor's canonical borsh bytes — used for DB persistence and P2P transport
    /// (P6.12). One wire format everywhere; `AnchorPayload` wraps the same encoding
    /// with a lane discriminant for the transaction channel.
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("borsh serialization of FinalityAnchor cannot fail")
    }

    pub fn from_wire_bytes(bytes: &[u8]) -> Option<Self> {
        Self::try_from_slice(bytes).ok()
    }
}

impl AnchorPayload {
    /// Decodes an anchor-subnetwork transaction payload. `None` on any malformed
    /// encoding, trailing bytes included — same "malformed encodings are
    /// consensus-invalid, not coerced" rule as `PoolOp::decode_payload`.
    pub fn decode_payload(payload: &[u8]) -> Option<Self> {
        Self::try_from_slice(payload).ok()
    }

    pub fn encode_payload(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("borsh serialization of AnchorPayload cannot fail")
    }
}

/// The message every trustee signature covers:
/// `H("FinalityAnchor" || anchored_block || anchored_daa_score (u64 LE))` — a new
/// domain-separated hash, same macro convention as every other purpose-specific hash
/// in this codebase (the domain tag is the hasher's blake3 key).
pub fn signing_hash(anchored_block: &Hash, anchored_daa_score: u64) -> Hash {
    let mut hasher = FinalityAnchorSigningHash::new();
    hasher.update(anchored_block.as_bytes()).update(anchored_daa_score.to_le_bytes());
    hasher.finalize()
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum FinalityAnchorError {
    #[error("signer bitmap {0:#04x} has bits set beyond the {TRUSTEE_COUNT} trustee keys")]
    BitmapOutOfRange(u8),

    #[error("anchor carries {actual} signatures but its bitmap has {expected} bits set")]
    SignatureCountMismatch { expected: usize, actual: usize },

    #[error("anchor has {0} signers, below the required quorum of {ANCHOR_QUORUM}")]
    BelowQuorum(usize),

    #[error("signature by trustee {0} does not verify")]
    BadSignature(u8),

    #[error("trustee public key {0} is not a valid BIP340 x-only key")]
    BadTrusteeKey(u8),

    #[error("evidence accuses trustee index {0}, beyond the {TRUSTEE_COUNT} trustee keys")]
    TrusteeIndexOutOfRange(u8),

    #[error("evidence attestations certify the same block — not an equivocation")]
    SameBlock,

    #[error("evidence attestation scores are {0} apart, not within one cadence interval of {1}")]
    ScoresNotWithinInterval(u64, u64),

    #[error("anchored DAA score {0} is at or beyond the hard trustee-expiry score {1}")]
    PastHardExpiry(u64, u64),
}

/// The outcome of offering an externally-received (gossiped) anchor to consensus
/// (PLAN P6.12). Drives the P2P flow's rebroadcast decision: `Ratcheted` and
/// `Pending` anchors improved local state and are worth relaying; `Ignored` ones are
/// stale, invalid, or quorum-dead and propagate no further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAnchorOutcome {
    /// Verified, block locally known, ratchet advanced — enforced from the next
    /// virtual resolution.
    Ratcheted,
    /// Verified and newer than anything held, but the anchored block is not locally
    /// verifiable yet — held pending (promoted once the block syncs; meanwhile it
    /// still guards IBD chain selection).
    Pending,
    /// No improvement or no consensus effect (stale score, invalid signatures,
    /// disqualified quorum, expired keys, or the mechanism is unkeyed).
    Ignored,
}

/// A snapshot of the node's finality-anchor state (POOL-SPEC.md P5.8's visibility
/// requirement: fail-open must be observable, "operators must be able to see
/// immediately that the extra protection layer is currently absent"). Served through
/// `ConsensusApi::get_finality_anchor_status`; RPC exposure ships with P6.12.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalityAnchorStatus {
    /// The ratchet: the highest-scoring anchor this node has ever accepted.
    pub latest_anchor: Option<(Hash, u64)>,
    /// True while the anchor-conflict fork-choice rule is actually being enforced.
    pub enforcing: bool,
    /// The spec's `finality_anchor_stale` flag: an anchor exists but has fallen
    /// outside the staleness bound — the chain is on plain-PoW guarantees right now.
    pub stale: bool,
    /// The hard trustee-expiry score has been reached — the mechanism is retired.
    pub expired: bool,
    /// The cadence interval of the currently active decay stage.
    pub current_interval: u64,
    /// Trustee indices disqualified as of the current virtual chain.
    pub disqualified: Vec<u8>,
}

/// Iterates the set bits of a signer bitmap in ascending order.
pub fn bitmap_signers(signer_bitmap: u8) -> impl Iterator<Item = u8> {
    (0..TRUSTEE_COUNT as u8).filter(move |i| signer_bitmap & (1 << i) != 0)
}

/// Stateless shape validation of an anchor: bitmap within range, one signature per set
/// bit, at least a quorum of signers. (No-repeated-signer is inherent to the bitmap.)
pub fn validate_anchor_shape(anchor: &FinalityAnchor) -> Result<(), FinalityAnchorError> {
    if anchor.signer_bitmap >> TRUSTEE_COUNT != 0 {
        return Err(FinalityAnchorError::BitmapOutOfRange(anchor.signer_bitmap));
    }
    let signer_count = anchor.signer_bitmap.count_ones() as usize;
    if signer_count != anchor.signatures.len() {
        return Err(FinalityAnchorError::SignatureCountMismatch { expected: signer_count, actual: anchor.signatures.len() });
    }
    if signer_count < ANCHOR_QUORUM {
        return Err(FinalityAnchorError::BelowQuorum(signer_count));
    }
    Ok(())
}

fn verify_one(key: &[u8; 32], trustee_index: u8, msg_hash: Hash, signature: &[u8; 64]) -> Result<(), FinalityAnchorError> {
    let xonly = secp256k1::XOnlyPublicKey::from_slice(key).map_err(|_| FinalityAnchorError::BadTrusteeKey(trustee_index))?;
    let sig = secp256k1::schnorr::Signature::from_slice(signature).expect("[u8; 64] is always length-valid");
    let msg = secp256k1::Message::from_digest(msg_hash.into());
    sig.verify(&msg, &xonly).map_err(|_| FinalityAnchorError::BadSignature(trustee_index))
}

/// Full context-free verification of an anchor: shape plus every carried signature
/// verifying against its bitmap-designated pinned trustee key. Deliberately does NOT
/// consult the deny-list — disqualification scopes to a chain POV and is applied where
/// the anchor's consensus *effect* is decided (the virtual processor), keeping
/// transaction acceptance identical across nodes regardless of local anchor state.
pub fn verify_anchor(anchor: &FinalityAnchor, trustee_keys: &TrusteeKeys) -> Result<(), FinalityAnchorError> {
    validate_anchor_shape(anchor)?;
    let msg_hash = signing_hash(&anchor.anchored_block, anchor.anchored_daa_score);
    for (signature, trustee_index) in anchor.signatures.iter().zip(bitmap_signers(anchor.signer_bitmap)) {
        verify_one(&trustee_keys[trustee_index as usize], trustee_index, msg_hash, signature)?;
    }
    Ok(())
}

/// Full context-free verification of an equivocation proof against P5.8's exact rule:
/// same trustee key, `anchored_block_1 ≠ anchored_block_2`, and
/// `|score_1 − score_2| < interval`, where `interval` is the cadence interval of the
/// decay stage active at `max(score_1, score_2)` — supplied by the caller as a
/// function of params (`interval_at`), keeping this module params-agnostic.
///
/// Anything failing any check is simply an invalid proof (and, at the transaction
/// layer, an invalid transaction): false evidence can never disqualify anyone.
pub fn verify_equivocation_evidence(
    evidence: &EquivocationEvidence,
    trustee_keys: &TrusteeKeys,
    interval_at: impl Fn(u64) -> u64,
) -> Result<(), FinalityAnchorError> {
    if evidence.trustee_index as usize >= TRUSTEE_COUNT {
        return Err(FinalityAnchorError::TrusteeIndexOutOfRange(evidence.trustee_index));
    }
    if evidence.first.anchored_block == evidence.second.anchored_block {
        return Err(FinalityAnchorError::SameBlock);
    }
    let gap = evidence.first.anchored_daa_score.abs_diff(evidence.second.anchored_daa_score);
    let interval = interval_at(evidence.first.anchored_daa_score.max(evidence.second.anchored_daa_score));
    if gap >= interval {
        return Err(FinalityAnchorError::ScoresNotWithinInterval(gap, interval));
    }
    let key = &trustee_keys[evidence.trustee_index as usize];
    for attestation in [&evidence.first, &evidence.second] {
        let msg_hash = signing_hash(&attestation.anchored_block, attestation.anchored_daa_score);
        verify_one(key, evidence.trustee_index, msg_hash, &attestation.signature)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trustee_keypair(seed: u8) -> secp256k1::Keypair {
        secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[seed; 32]).unwrap()
    }

    fn trustee_keys() -> (Vec<secp256k1::Keypair>, TrusteeKeys) {
        let keypairs: Vec<_> = (1..=TRUSTEE_COUNT as u8).map(trustee_keypair).collect();
        let keys: Vec<[u8; 32]> = keypairs.iter().map(|kp| kp.public_key().x_only_public_key().0.serialize()).collect();
        (keypairs, keys.try_into().unwrap())
    }

    fn attest(keypair: &secp256k1::Keypair, block: Hash, score: u64) -> [u8; 64] {
        let msg = secp256k1::Message::from_digest(signing_hash(&block, score).into());
        *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, keypair).as_ref()
    }

    fn sign_anchor(keypairs: &[secp256k1::Keypair], signers: &[u8], block: Hash, score: u64) -> FinalityAnchor {
        let mut bitmap = 0u8;
        let mut signatures = Vec::new();
        let mut sorted = signers.to_vec();
        sorted.sort_unstable();
        for &i in sorted.iter() {
            bitmap |= 1 << i;
            signatures.push(attest(&keypairs[i as usize], block, score));
        }
        FinalityAnchor { anchored_block: block, anchored_daa_score: score, signer_bitmap: bitmap, signatures }
    }

    #[test]
    fn valid_three_of_five_anchor_verifies() {
        let (keypairs, keys) = trustee_keys();
        let anchor = sign_anchor(&keypairs, &[0, 2, 4], 7.into(), 1_000);
        assert_eq!(verify_anchor(&anchor, &keys), Ok(()));
        // All five signing also verifies
        let anchor = sign_anchor(&keypairs, &[0, 1, 2, 3, 4], 7.into(), 1_000);
        assert_eq!(verify_anchor(&anchor, &keys), Ok(()));
    }

    #[test]
    fn below_quorum_anchor_rejected() {
        let (keypairs, keys) = trustee_keys();
        let anchor = sign_anchor(&keypairs, &[0, 2], 7.into(), 1_000);
        assert_eq!(verify_anchor(&anchor, &keys), Err(FinalityAnchorError::BelowQuorum(2)));
    }

    #[test]
    fn bitmap_signature_mismatches_rejected() {
        let (keypairs, keys) = trustee_keys();
        let mut anchor = sign_anchor(&keypairs, &[0, 1, 2], 7.into(), 1_000);
        anchor.signatures.pop();
        assert_eq!(verify_anchor(&anchor, &keys), Err(FinalityAnchorError::SignatureCountMismatch { expected: 3, actual: 2 }));
        let anchor = sign_anchor(&keypairs, &[0, 1, 2], 7.into(), 1_000);
        let mut bad = anchor.clone();
        bad.signer_bitmap = 0b100111; // bit 5 doesn't exist
        assert_eq!(verify_anchor(&bad, &keys), Err(FinalityAnchorError::BitmapOutOfRange(0b100111)));
    }

    #[test]
    fn wrong_key_signature_rejected() {
        let (keypairs, keys) = trustee_keys();
        // Trustee 3 signs, but the bitmap claims the slot belongs to trustee 1
        let mut anchor = sign_anchor(&keypairs, &[0, 1, 2], 7.into(), 1_000);
        anchor.signatures[1] = attest(&keypairs[3], 7.into(), 1_000);
        assert_eq!(verify_anchor(&anchor, &keys), Err(FinalityAnchorError::BadSignature(1)));
    }

    #[test]
    fn signature_does_not_transfer_across_messages() {
        let (keypairs, keys) = trustee_keys();
        // Signed over one (block, score), presented over another
        let mut anchor = sign_anchor(&keypairs, &[0, 1, 2], 7.into(), 1_000);
        anchor.anchored_daa_score = 1_001;
        assert_eq!(verify_anchor(&anchor, &keys), Err(FinalityAnchorError::BadSignature(0)));
    }

    #[test]
    fn equivocation_rule_is_exactly_decidable() {
        let (keypairs, keys) = trustee_keys();
        let interval = 300u64;
        let make = |block: Hash, score: u64| AnchorAttestation {
            anchored_block: block,
            anchored_daa_score: score,
            signature: attest(&keypairs[2], block, score),
        };

        // Different blocks, scores within one interval: equivocation.
        let ev = EquivocationEvidence { trustee_index: 2, first: make(7.into(), 1_000), second: make(8.into(), 1_299) };
        assert_eq!(verify_equivocation_evidence(&ev, &keys, |_| interval), Ok(()));

        // Exactly one interval apart: honest sequential anchoring, NOT equivocation
        // (the rule is a strict `< interval`).
        let ev = EquivocationEvidence { trustee_index: 2, first: make(7.into(), 1_000), second: make(8.into(), 1_300) };
        assert_eq!(
            verify_equivocation_evidence(&ev, &keys, |_| interval),
            Err(FinalityAnchorError::ScoresNotWithinInterval(300, 300))
        );

        // Same block twice: not an equivocation, whatever the scores.
        let ev = EquivocationEvidence { trustee_index: 2, first: make(7.into(), 1_000), second: make(7.into(), 1_100) };
        assert_eq!(verify_equivocation_evidence(&ev, &keys, |_| interval), Err(FinalityAnchorError::SameBlock));

        // A forged signature can never disqualify: trustee 2 accused with trustee 3's
        // signatures fails verification outright.
        let forged = AnchorAttestation {
            anchored_block: 8.into(),
            anchored_daa_score: 1_100,
            signature: attest(&keypairs[3], 8.into(), 1_100),
        };
        let ev = EquivocationEvidence { trustee_index: 2, first: make(7.into(), 1_000), second: forged };
        assert_eq!(verify_equivocation_evidence(&ev, &keys, |_| interval), Err(FinalityAnchorError::BadSignature(2)));
    }

    #[test]
    fn payload_roundtrip_and_malformed_rejection() {
        let (keypairs, _) = trustee_keys();
        let anchor = sign_anchor(&keypairs, &[0, 1, 2], 7.into(), 1_000);
        let payload = AnchorPayload::Anchor(anchor.clone()).encode_payload();
        assert_eq!(AnchorPayload::decode_payload(&payload), Some(AnchorPayload::Anchor(anchor)));

        // Unknown discriminant and trailing bytes are both malformed.
        assert_eq!(AnchorPayload::decode_payload(&[2u8]), None);
        let mut trailing = AnchorPayload::Equivocation(EquivocationEvidence {
            trustee_index: 0,
            first: AnchorAttestation { anchored_block: 1.into(), anchored_daa_score: 1, signature: [0; 64] },
            second: AnchorAttestation { anchored_block: 2.into(), anchored_daa_score: 2, signature: [0; 64] },
        })
        .encode_payload();
        trailing.push(0);
        assert_eq!(AnchorPayload::decode_payload(&trailing), None);
    }
}
