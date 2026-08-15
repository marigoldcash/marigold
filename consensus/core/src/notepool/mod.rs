//!
//! # Note pool
//!
//! Wire types for Marigold's note pool (P6.1 — pure data, no validation logic).
//! Every type here is defined exactly as specified in
//! `docs/x-fork/POOL-SPEC.md`'s P5.1/P5.2 sections — see that document for the
//! full rationale; this module only implements the byte layouts it fixes.
//!

pub mod diff;
pub mod hashing;
pub mod validate;
pub mod view;

pub use diff::{ImmutablePoolDiff, PoolCollection, PoolDiff};
pub use hashing::{leaf_hash, serial_hash, signing_hash, transparent_outputs_hash};
pub use validate::{MAX_POOL_OP_COLLECTION_LEN, ValidatedPoolOp, validate_stateful, validate_stateless};
pub use view::{ComposedPoolView, PoolStateView, PoolViewComposition};

/// The pool protocol version byte committed in every `NotePoolSigningHash` preimage
/// (POOL-SPEC.md P5.2, v1.1). Any future revision changing signing semantics bumps this
/// rather than relying on every other field coincidentally differing.
pub const POOL_PROTOCOL_VERSION: u8 = 1;

/// The freshness window (POOL-SPEC.md P5.2/P5.3): a pool op is valid iff
/// `0 <= pov_daa_score - freshness.anchor_daa_score <= POOL_FRESHNESS_WINDOW`, inclusive
/// on both ends. 36,000 DAA-score units ≈ 1 hour at 10 BPS — a liveness/UX parameter,
/// not a derived security constant (the spec's own classification); P6.6 calibration may
/// retune it before launch.
pub const POOL_FRESHNESS_WINDOW: u64 = 36_000;

use crate::Hash;
use borsh::{BorshDeserialize, BorshSerialize};
use kaspa_utils::mem_size::MemSizeEstimator;
use serde::{Deserialize, Serialize};

/// A denomination tag: a `u8` index into the fixed P1.6 denomination ladder, not the
/// raw petal amount (POOL-SPEC.md P5.1, "`d` — denomination tag"). Tags 8-255 are
/// reserved; assigning one is inherently a hard fork (the tag→value table is
/// referenced by conservation arithmetic and the pool commitment's meaning).
///
/// Borsh discriminants are assigned by declaration order, 0 through 7, matching the
/// table below exactly. Also carries `serde` derives (bincode-based) independent of
/// the borsh wire format — used only for the pool state store's on-disk encoding
/// (P6.2), the same "two independent serializations for two independent purposes"
/// pattern `UtxoEntry` already uses in this codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub enum DenominationTag {
    /// 0.01 MAGLD = 1,000,000 petals
    D0_01,
    /// 0.1 MAGLD = 10,000,000 petals
    D0_1,
    /// 1 MAGLD = 100,000,000 petals
    D1,
    /// 10 MAGLD = 1,000,000,000 petals
    D10,
    /// 100 MAGLD = 10,000,000,000 petals
    D100,
    /// 1,000 MAGLD = 100,000,000,000 petals
    D1000,
    /// 10,000 MAGLD = 1,000,000,000,000 petals
    D10000,
    /// 100,000 MAGLD = 10,000,000,000,000 petals
    D100000,
}

/// The canonical denomination → petal-value table (POOL-SPEC.md P5.1), indexed by a
/// `DenominationTag`'s declaration order. The single source of truth for "what
/// denominations exist" — mirrors `SUBSIDY_BY_MONTH_TABLE`'s (P3.2) role as one
/// canonical const array rather than a formula recomputed ad hoc.
pub const DENOMINATION_PETALS: [u64; 8] =
    [1_000_000, 10_000_000, 100_000_000, 1_000_000_000, 10_000_000_000, 100_000_000_000, 1_000_000_000_000, 10_000_000_000_000];

impl DenominationTag {
    /// The note value this tag represents, in petals.
    pub const fn petals(self) -> u64 {
        DENOMINATION_PETALS[self as usize]
    }
}

/// A note, as a self-contained unit (e.g. in a wallet's key database — POOL-SPEC.md
/// P5.6). 65 bytes: `d` (1) + `pk` (32) + `sn` (32). Inside the pool state map itself,
/// `sn` is the map key, so only `d`+`pk` (33 bytes) are the stored value per entry.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Note {
    pub d: DenominationTag,
    /// x-only BIP340 Schnorr public key — the same 32-byte format as address
    /// `Version::PubKey` (`crypto/addresses`).
    pub pk: [u8; 32],
    /// Stable for the note's lifetime; never appears as mutable state. Derived per
    /// `sn = H_serial(creating_tx_id || output_index)`, P5.1 — not stored in any
    /// `PoolOp` payload, since every node computes it independently.
    pub sn: Hash,
}

/// A note about to be created by a `PoolOp` — everything except its `sn`, which every
/// node derives independently from `(creating_tx_id, index_among_this_op's_notes)`
/// rather than trusting the payload to state it (POOL-SPEC.md P5.2). 33 bytes.
///
/// Also doubles as the pool state map's *value* type (`sn -> (d, pk)`, P5.1) — a live
/// pool entry is exactly "the (d, pk) of some note," the same shape whether it just
/// arrived in a `PoolOp` payload or has been sitting in the committed state for years.
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct NewNote {
    pub d: DenominationTag,
    pub pk: [u8; 32],
}

impl MemSizeEstimator for NewNote {
    fn estimate_mem_bytes(&self) -> usize {
        size_of::<Self>()
    }
}

/// One or more serials currently sharing a single `pk`, authorized in one `PoolOp` by
/// one Schnorr signature from that shared key (POOL-SPEC.md P5.2). Multiple groups in
/// one op exist because a wallet may need to spend notes under different keys in a
/// single transaction; one group with many serials is what a merchant's POS sweep
/// (P5.6) uses to move many same-`pk` notes in one signature.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SignedGroup {
    /// Must currently share one `pk` (checked at validation, not by this type).
    /// Canonical order is ascending lexicographic byte order over the raw serials,
    /// and duplicates — within a group or across an op's groups — are invalid
    /// (P5.2 canonicalization rules; enforced by validation, not this type).
    pub serials: Vec<Hash>,
    /// BIP340 Schnorr signature over `NotePoolSigningHash` (P5.2).
    pub signature: [u8; 64],
}

/// The DAA-score anti-replay anchor every pool-op signature covers (POOL-SPEC.md
/// P5.2). Valid iff `0 <= pov_daa_score - anchor_daa_score <= 36_000` inclusive, per
/// P5.3's validation rule — not enforced by this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct FreshnessAnchor {
    pub anchor_daa_score: u64,
}

/// Deposit transparent coins into the pool as new notes (POOL-SPEC.md P5.2). The
/// enclosing transaction's ordinary transparent inputs must sum to at least
/// `Σ(new_notes petal values)` — mint needs no note-level signature, since nothing
/// pre-existing in the pool is touched; it self-funds its fee the same way any
/// transparent Kaspa transaction always has.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct MintOp {
    pub new_notes: Vec<NewNote>,
}

/// Consume some notes, produce some notes, under one conservation check
/// (`Σ(consumed) >= Σ(produced)`, difference = fee) — the unified wire shape for
/// rotate/split/merge (POOL-SPEC.md P5.2). "Rotate", "split", and "merge" are purely
/// descriptive labels for what a given `TransferOp`'s multiset happened to do, not
/// distinct protocol formats.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct TransferOp {
    pub consumed: Vec<SignedGroup>,
    pub produced: Vec<NewNote>,
    pub freshness: FreshnessAnchor,
}

/// Withdraw notes from the pool as transparent coins (POOL-SPEC.md P5.2) — the
/// transparent-side mirror of `MintOp`. The enclosing transaction's ordinary
/// transparent outputs hold what the redeemed notes become;
/// `Σ(consumed) >= Σ(transparent outputs) + fee`. Self-funding like mint: no fee
/// stamp required, since redeem already produces transparent value to pay from.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct RedeemOp {
    pub consumed: Vec<SignedGroup>,
    pub freshness: FreshnessAnchor,
}

/// The note-pool operation payload — the entire contents of a pool-op transaction's
/// `payload` field, borsh-encoded, tagged with `subnets::SUBNETWORK_ID_NOTE_POOL`
/// (POOL-SPEC.md P5.2). Only three wire-format variants, not five — see
/// `TransferOp`'s docs for why rotate/split/merge collapse into one shape.
///
/// Borsh assigns discriminants by declaration order: `Mint` = 0, `Transfer` = 1,
/// `Redeem` = 2. The `NotePoolSigningHash` preimage's `op_type` byte (P5.2) reuses
/// these values verbatim — one canonical numbering, defined once, here.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum PoolOp {
    Mint(MintOp),
    Transfer(TransferOp),
    Redeem(RedeemOp),
}

impl PoolOp {
    /// Decodes a pool-op transaction payload. `None` on any malformed encoding —
    /// including trailing bytes after the value ("malformed encodings are
    /// consensus-invalid, not coerced", POOL-SPEC.md P5.1; `try_from_slice` requires
    /// the whole slice consumed). Wrapped here so downstream crates need no direct
    /// borsh dependency.
    pub fn decode_payload(payload: &[u8]) -> Option<Self> {
        Self::try_from_slice(payload).ok()
    }

    /// The inverse of [`Self::decode_payload`] — the exact bytes a pool-op
    /// transaction's `payload` field carries.
    pub fn encode_payload(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("borsh serialization of PoolOp cannot fail")
    }

    /// This op's canonical type byte — the borsh enum tag, reused verbatim as
    /// `NotePoolSigningHash`'s `op_type` preimage byte (P5.2).
    pub fn op_type(&self) -> u8 {
        match self {
            PoolOp::Mint(_) => 0,
            PoolOp::Transfer(_) => 1,
            PoolOp::Redeem(_) => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn sample_pk(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn sample_sig(byte: u8) -> [u8; 64] {
        [byte; 64]
    }

    fn round_trip(op: &PoolOp) -> PoolOp {
        let bytes = borsh::to_vec(op).unwrap();
        PoolOp::try_from_slice(&bytes).unwrap()
    }

    #[test]
    fn denomination_petals_table_matches_spec() {
        // POOL-SPEC.md P5.1's table, verbatim.
        assert_eq!(DenominationTag::D0_01.petals(), 1_000_000);
        assert_eq!(DenominationTag::D0_1.petals(), 10_000_000);
        assert_eq!(DenominationTag::D1.petals(), 100_000_000);
        assert_eq!(DenominationTag::D10.petals(), 1_000_000_000);
        assert_eq!(DenominationTag::D100.petals(), 10_000_000_000);
        assert_eq!(DenominationTag::D1000.petals(), 100_000_000_000);
        assert_eq!(DenominationTag::D10000.petals(), 1_000_000_000_000);
        assert_eq!(DenominationTag::D100000.petals(), 10_000_000_000_000);
    }

    #[test]
    fn denomination_tag_borsh_discriminants_match_declaration_order() {
        // The NotePoolSigningHash's op_type byte and the spec's tag table both assume
        // borsh assigns 0..=7 by declaration order — pin this down explicitly so a
        // future reordering of the enum can't silently break it.
        let tags = [
            DenominationTag::D0_01,
            DenominationTag::D0_1,
            DenominationTag::D1,
            DenominationTag::D10,
            DenominationTag::D100,
            DenominationTag::D1000,
            DenominationTag::D10000,
            DenominationTag::D100000,
        ];
        for (expected_discriminant, tag) in tags.iter().enumerate() {
            let bytes = borsh::to_vec(tag).unwrap();
            assert_eq!(bytes, vec![expected_discriminant as u8]);
        }
    }

    #[test]
    fn pool_op_borsh_discriminants_match_op_type_numbering() {
        // NotePoolSigningHash's op_type: 0=Mint (unused, no note signature), 1=Transfer, 2=Redeem.
        let mint = PoolOp::Mint(MintOp { new_notes: vec![] });
        let transfer = PoolOp::Transfer(TransferOp {
            consumed: vec![],
            produced: vec![],
            freshness: FreshnessAnchor { anchor_daa_score: 0 },
        });
        let redeem = PoolOp::Redeem(RedeemOp { consumed: vec![], freshness: FreshnessAnchor { anchor_daa_score: 0 } });

        assert_eq!(borsh::to_vec(&mint).unwrap()[0], 0);
        assert_eq!(borsh::to_vec(&transfer).unwrap()[0], 1);
        assert_eq!(borsh::to_vec(&redeem).unwrap()[0], 2);
    }

    #[test]
    fn note_round_trips_and_matches_spec_byte_size() {
        let note = Note { d: DenominationTag::D1, pk: sample_pk(0xAB), sn: sample_hash(0xCD) };
        let bytes = borsh::to_vec(&note).unwrap();
        // 1 (tag) + 32 (pk) + 32 (sn) = 65 bytes, per POOL-SPEC.md P5.1.
        assert_eq!(bytes.len(), 65);
        let decoded = Note::try_from_slice(&bytes).unwrap();
        assert_eq!(decoded, note);
    }

    #[test]
    fn new_note_matches_spec_byte_size() {
        let note = NewNote { d: DenominationTag::D100, pk: sample_pk(0x11) };
        let bytes = borsh::to_vec(&note).unwrap();
        // 1 (tag) + 32 (pk) = 33 bytes, per POOL-SPEC.md P5.1/P5.2.
        assert_eq!(bytes.len(), 33);
    }

    #[test]
    fn mint_op_round_trip_single_note() {
        let op = PoolOp::Mint(MintOp { new_notes: vec![NewNote { d: DenominationTag::D1, pk: sample_pk(0x01) }] });
        let bytes = borsh::to_vec(&op).unwrap();
        // 1 (PoolOp tag) + 4 (Vec len) + 33 (one NewNote) = 38 bytes, per P5.2's worked example.
        assert_eq!(bytes.len(), 38);
        assert_eq!(round_trip(&op), op);
    }

    #[test]
    fn mint_op_round_trip_five_notes() {
        let op = PoolOp::Mint(MintOp {
            new_notes: (0..5).map(|i| NewNote { d: DenominationTag::D1, pk: sample_pk(i) }).collect(),
        });
        let bytes = borsh::to_vec(&op).unwrap();
        // 1 + 4 + 33*5 = 170 bytes, per P5.2's worked example.
        assert_eq!(bytes.len(), 170);
        assert_eq!(round_trip(&op), op);
    }

    #[test]
    fn transfer_op_round_trip_plain_rotate() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials: vec![sample_hash(0x01)], signature: sample_sig(0x02) }],
            produced: vec![NewNote { d: DenominationTag::D0_1, pk: sample_pk(0x03) }],
            freshness: FreshnessAnchor { anchor_daa_score: 12345 },
        });
        let bytes = borsh::to_vec(&op).unwrap();
        // 1 + [4+(4+32+64)] + [4+33] + 8 = 150 bytes, per P5.2's worked example.
        assert_eq!(bytes.len(), 150);
        assert_eq!(round_trip(&op), op);
    }

    #[test]
    fn transfer_op_round_trip_rotate_plus_stamp() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials: vec![sample_hash(0x01), sample_hash(0x02)], signature: sample_sig(0x03) }],
            produced: vec![NewNote { d: DenominationTag::D0_1, pk: sample_pk(0x04) }],
            freshness: FreshnessAnchor { anchor_daa_score: 1 },
        });
        let bytes = borsh::to_vec(&op).unwrap();
        // 1 + [4+(4+64+64)] + [4+33] + 8 = 182 bytes, per P5.2's worked example.
        assert_eq!(bytes.len(), 182);
        assert_eq!(round_trip(&op), op);
    }

    #[test]
    fn transfer_op_round_trip_self_funding_split() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials: vec![sample_hash(0x01)], signature: sample_sig(0x02) }],
            produced: (0..36).map(|i| NewNote { d: DenominationTag::D0_01, pk: sample_pk(i) }).collect(),
            freshness: FreshnessAnchor { anchor_daa_score: 99 },
        });
        let bytes = borsh::to_vec(&op).unwrap();
        // 1 + [4+(4+32+64)] + [4+33*36] + 8 = 1305 bytes, per P5.2's worked example.
        assert_eq!(bytes.len(), 1305);
        assert_eq!(round_trip(&op), op);
    }

    #[test]
    fn transfer_op_round_trip_merchant_sweep() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup {
                serials: (0..20).map(sample_hash).collect(),
                signature: sample_sig(0xFF),
            }],
            produced: (0..20).map(|i| NewNote { d: DenominationTag::D1, pk: sample_pk(i) }).collect(),
            freshness: FreshnessAnchor { anchor_daa_score: 7 },
        });
        let bytes = borsh::to_vec(&op).unwrap();
        // 1 + [4+(4+32*20+64)] + [4+33*20] + 8 = 1385 bytes, per P5.2's worked example
        // — the largest of the spec's worked sizes, still comfortably under "a few KB".
        assert_eq!(bytes.len(), 1385);
        assert_eq!(round_trip(&op), op);
    }

    #[test]
    fn redeem_op_round_trip() {
        let op = PoolOp::Redeem(RedeemOp {
            consumed: vec![SignedGroup {
                serials: (0..3).map(sample_hash).collect(),
                signature: sample_sig(0x77),
            }],
            freshness: FreshnessAnchor { anchor_daa_score: 42 },
        });
        let bytes = borsh::to_vec(&op).unwrap();
        // 1 + [4+(4+32*3+64)] + 8 = 177 bytes, per P5.2's worked example.
        assert_eq!(bytes.len(), 177);
        assert_eq!(round_trip(&op), op);
    }

    #[test]
    fn empty_ops_round_trip() {
        // Degenerate but well-formed shapes: no groups/notes at all. Byte-level
        // encoding must still be well-defined and round-trip cleanly, even though
        // validation (P6.3+) will reject these as economically meaningless.
        for op in [
            PoolOp::Mint(MintOp { new_notes: vec![] }),
            PoolOp::Transfer(TransferOp { consumed: vec![], produced: vec![], freshness: FreshnessAnchor { anchor_daa_score: 0 } }),
            PoolOp::Redeem(RedeemOp { consumed: vec![], freshness: FreshnessAnchor { anchor_daa_score: 0 } }),
        ] {
            assert_eq!(round_trip(&op), op);
        }
    }

    #[test]
    fn maximum_size_transfer_round_trips() {
        // A large-but-plausible Transfer: multiple signed groups (distinct keys) each
        // with several serials, and a sizeable produced list — exercises multi-group
        // encoding, which the worked-example tests above (all single-group) don't.
        let consumed: Vec<SignedGroup> = (0..10u8)
            .map(|g| SignedGroup { serials: (0..10).map(|i| sample_hash(g.wrapping_mul(10).wrapping_add(i))).collect(), signature: sample_sig(g) })
            .collect();
        let produced: Vec<NewNote> = (0..50u8).map(|i| NewNote { d: DenominationTag::D10, pk: sample_pk(i) }).collect();
        let op = PoolOp::Transfer(TransferOp { consumed, produced, freshness: FreshnessAnchor { anchor_daa_score: u64::MAX } });
        assert_eq!(round_trip(&op), op);
    }

    #[test]
    fn malformed_pool_op_discriminant_rejected() {
        // A discriminant of 3 doesn't exist (only Mint=0, Transfer=1, Redeem=2) —
        // borsh's derived deserialization must reject it outright, per P5.1's
        // "malformed encodings are consensus-invalid, not coerced" rule.
        let bytes = vec![3u8];
        assert!(PoolOp::try_from_slice(&bytes).is_err());
    }

    #[test]
    fn malformed_denomination_tag_discriminant_rejected() {
        // Tag 8 is reserved/unassigned (P5.1) — must be rejected, not silently accepted.
        let bytes = vec![8u8];
        assert!(DenominationTag::try_from_slice(&bytes).is_err());
    }

    #[test]
    fn trailing_bytes_after_valid_payload_rejected() {
        // "Malformed encodings ... carries trailing bytes after the deserialized
        // value ... is rejected outright" (P5.1). try_from_slice enforces this by
        // requiring the entire slice to be consumed.
        let op = PoolOp::Mint(MintOp { new_notes: vec![] });
        let mut bytes = borsh::to_vec(&op).unwrap();
        bytes.push(0xFF);
        assert!(PoolOp::try_from_slice(&bytes).is_err());
    }
}
