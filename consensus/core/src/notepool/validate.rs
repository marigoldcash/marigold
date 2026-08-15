//! Stateless `PoolOp` validation (POOL-SPEC.md P5.3, FORK-PLAN P6.3) — checks that hold
//! for an op in complete isolation, with no pool state, no consensus context, and no
//! enclosing transaction. Everything requiring the live pool view — serial existence,
//! signature verification against a serial's *current* `pk`, the freshness-window
//! comparison against a POV DAA score, and actual value conservation (which needs
//! consumed notes' current denominations, not just their serials) — is P6.4's job, not
//! this module's.
//!
//! Several of P6.3's plan-text checks are already guaranteed by the type system once a
//! `PoolOp` decodes at all, so this module doesn't re-check them at runtime:
//!
//! - **"Denominations from the P1.6 set"** — `DenominationTag` is a fieldless enum
//!   whose only inhabitants are the eight valid tags (P6.1's
//!   `malformed_denomination_tag_discriminant_rejected` test proves borsh rejects any
//!   other discriminant at decode time).
//! - **"Freshness-anchor field present"** — `FreshnessAnchor` is a required struct
//!   field on `TransferOp`/`RedeemOp`, never `Option`; there is no wire shape in which
//!   it's absent.
//! - **"Signature well-formed"** — checked directly against `secp256k1` v0.29.1's
//!   source (`schnorr::Signature::from_slice`, `src/schnorr.rs`): it validates only
//!   that the input is exactly `SCHNORR_SIGNATURE_SIZE` (64) bytes, which
//!   `SignedGroup.signature`'s `[u8; 64]` field type already guarantees unconditionally
//!   — no curve/field validation happens until actual verification against a message
//!   and public key (P6.4, which has the serial's current `pk` to verify against).
//!   Calling `from_slice` here would be dead code that can never return `Err`.
//!
//! This module only adds runtime checks for the parts genuinely *not* implied by
//! successful decoding: collection shape and size, and duplicate serials.

use super::{MintOp, PoolOp, RedeemOp, SignedGroup, TransferOp};
use crate::Hash;
use crate::errors::notepool::PoolOpValidationError;
use std::collections::HashSet;

/// An op's total consumed serials / produced notes are each capped at this many items
/// (POOL-SPEC.md P5.2). Chosen to match the current mainnet `max_tx_inputs`/
/// `max_tx_outputs` value (`consensus/core/src/config/params.rs`) — a deliberately
/// separate protocol constant, not a live read of that field, so a future change to one
/// bound doesn't silently retune the other.
pub const MAX_POOL_OP_COLLECTION_LEN: usize = 1000;

/// Validates a `PoolOp` against every stateless P5.3/P6.3 rule. Returns the first
/// violation found; callers needing every violation (e.g. fuzzing harnesses, P8.1)
/// should call the per-shape helpers directly.
pub fn validate_stateless(op: &PoolOp) -> Result<(), PoolOpValidationError> {
    match op {
        PoolOp::Mint(mint) => validate_mint(mint),
        PoolOp::Transfer(transfer) => validate_transfer(transfer),
        PoolOp::Redeem(redeem) => validate_redeem(redeem),
    }
}

fn validate_mint(mint: &MintOp) -> Result<(), PoolOpValidationError> {
    if mint.new_notes.is_empty() {
        return Err(PoolOpValidationError::EmptyCollection("new_notes"));
    }
    if mint.new_notes.len() > MAX_POOL_OP_COLLECTION_LEN {
        return Err(PoolOpValidationError::TooManyItems(mint.new_notes.len(), "new_notes", MAX_POOL_OP_COLLECTION_LEN));
    }
    Ok(())
}

fn validate_transfer(transfer: &TransferOp) -> Result<(), PoolOpValidationError> {
    // A Transfer with no signing group is unauthorized minting under another name —
    // every produced note must be backed by at least one signature over consumed notes.
    if transfer.consumed.is_empty() {
        return Err(PoolOpValidationError::EmptyCollection("consumed"));
    }
    if transfer.produced.len() > MAX_POOL_OP_COLLECTION_LEN {
        return Err(PoolOpValidationError::TooManyItems(transfer.produced.len(), "produced", MAX_POOL_OP_COLLECTION_LEN));
    }
    validate_consumed_groups(&transfer.consumed)
}

fn validate_redeem(redeem: &RedeemOp) -> Result<(), PoolOpValidationError> {
    if redeem.consumed.is_empty() {
        return Err(PoolOpValidationError::EmptyCollection("consumed"));
    }
    validate_consumed_groups(&redeem.consumed)
}

/// Shared by `Transfer` and `Redeem` (POOL-SPEC.md P5.3, "Validation order" steps
/// common to both): total serial count within bounds, every group non-empty, and no
/// serial repeated within or across groups (P5.3's `Transfer` step 1 / `Redeem` steps
/// 1-3, "identical to Transfer's"). Signature well-formedness needs no separate check
/// here — see this module's doc comment.
fn validate_consumed_groups(groups: &[SignedGroup]) -> Result<(), PoolOpValidationError> {
    let total_serials: usize = groups.iter().map(|g| g.serials.len()).sum();
    if total_serials > MAX_POOL_OP_COLLECTION_LEN {
        return Err(PoolOpValidationError::TooManyItems(total_serials, "consumed serials", MAX_POOL_OP_COLLECTION_LEN));
    }

    let mut seen: HashSet<Hash> = HashSet::with_capacity(total_serials);
    for (i, group) in groups.iter().enumerate() {
        if group.serials.is_empty() {
            return Err(PoolOpValidationError::EmptySignedGroup(i));
        }
        for &serial in &group.serials {
            if !seen.insert(serial) {
                return Err(PoolOpValidationError::DuplicateSerial(serial));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notepool::{DenominationTag, FreshnessAnchor, NewNote};

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn sig(byte: u8) -> [u8; 64] {
        [byte; 64]
    }

    fn note(byte: u8) -> NewNote {
        NewNote { d: DenominationTag::D1, pk: [byte; 32] }
    }

    fn anchor() -> FreshnessAnchor {
        FreshnessAnchor { anchor_daa_score: 100 }
    }

    #[test]
    fn valid_mint_passes() {
        let op = PoolOp::Mint(MintOp { new_notes: vec![note(1)] });
        assert_eq!(validate_stateless(&op), Ok(()));
    }

    #[test]
    fn empty_mint_rejected() {
        let op = PoolOp::Mint(MintOp { new_notes: vec![] });
        assert_eq!(validate_stateless(&op), Err(PoolOpValidationError::EmptyCollection("new_notes")));
    }

    #[test]
    fn oversized_mint_rejected() {
        let op = PoolOp::Mint(MintOp { new_notes: (0..=MAX_POOL_OP_COLLECTION_LEN).map(|i| note(i as u8)).collect() });
        assert_eq!(
            validate_stateless(&op),
            Err(PoolOpValidationError::TooManyItems(MAX_POOL_OP_COLLECTION_LEN + 1, "new_notes", MAX_POOL_OP_COLLECTION_LEN))
        );
    }

    #[test]
    fn mint_at_exactly_the_cap_passes() {
        let op = PoolOp::Mint(MintOp { new_notes: (0..MAX_POOL_OP_COLLECTION_LEN).map(|i| note(i as u8)).collect() });
        assert_eq!(validate_stateless(&op), Ok(()));
    }

    #[test]
    fn valid_transfer_passes() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials: vec![hash(1)], signature: sig(0x11) }],
            produced: vec![note(1)],
            freshness: anchor(),
        });
        assert_eq!(validate_stateless(&op), Ok(()));
    }

    #[test]
    fn transfer_with_no_consumed_groups_rejected() {
        let op = PoolOp::Transfer(TransferOp { consumed: vec![], produced: vec![note(1)], freshness: anchor() });
        assert_eq!(validate_stateless(&op), Err(PoolOpValidationError::EmptyCollection("consumed")));
    }

    #[test]
    fn transfer_with_empty_signed_group_rejected() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials: vec![], signature: sig(0x11) }],
            produced: vec![note(1)],
            freshness: anchor(),
        });
        assert_eq!(validate_stateless(&op), Err(PoolOpValidationError::EmptySignedGroup(0)));
    }

    #[test]
    fn transfer_with_duplicate_serial_within_one_group_rejected() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials: vec![hash(1), hash(1)], signature: sig(0x11) }],
            produced: vec![note(1)],
            freshness: anchor(),
        });
        assert_eq!(validate_stateless(&op), Err(PoolOpValidationError::DuplicateSerial(hash(1))));
    }

    #[test]
    fn transfer_with_duplicate_serial_across_groups_rejected() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![
                SignedGroup { serials: vec![hash(1)], signature: sig(0x11) },
                SignedGroup { serials: vec![hash(1)], signature: sig(0x11) },
            ],
            produced: vec![note(1)],
            freshness: anchor(),
        });
        assert_eq!(validate_stateless(&op), Err(PoolOpValidationError::DuplicateSerial(hash(1))));
    }

    #[test]
    fn transfer_with_oversized_produced_rejected() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials: vec![hash(1)], signature: sig(0x11) }],
            produced: (0..=MAX_POOL_OP_COLLECTION_LEN).map(|i| note(i as u8)).collect(),
            freshness: anchor(),
        });
        assert_eq!(
            validate_stateless(&op),
            Err(PoolOpValidationError::TooManyItems(MAX_POOL_OP_COLLECTION_LEN + 1, "produced", MAX_POOL_OP_COLLECTION_LEN))
        );
    }

    #[test]
    fn transfer_with_oversized_consumed_serials_rejected() {
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup {
                serials: (0..=MAX_POOL_OP_COLLECTION_LEN).map(|i| hash(i as u8)).collect(),
                signature: sig(0x11),
            }],
            produced: vec![note(1)],
            freshness: anchor(),
        });
        assert_eq!(
            validate_stateless(&op),
            Err(PoolOpValidationError::TooManyItems(MAX_POOL_OP_COLLECTION_LEN + 1, "consumed serials", MAX_POOL_OP_COLLECTION_LEN))
        );
    }

    #[test]
    fn valid_redeem_passes() {
        let op = PoolOp::Redeem(RedeemOp {
            consumed: vec![SignedGroup { serials: vec![hash(1), hash(2)], signature: sig(0x11) }],
            freshness: anchor(),
        });
        assert_eq!(validate_stateless(&op), Ok(()));
    }

    #[test]
    fn redeem_with_no_consumed_groups_rejected() {
        let op = PoolOp::Redeem(RedeemOp { consumed: vec![], freshness: anchor() });
        assert_eq!(validate_stateless(&op), Err(PoolOpValidationError::EmptyCollection("consumed")));
    }

    #[test]
    fn redeem_with_duplicate_serial_rejected() {
        let op = PoolOp::Redeem(RedeemOp {
            consumed: vec![SignedGroup { serials: vec![hash(1), hash(1)], signature: sig(0x11) }],
            freshness: anchor(),
        });
        assert_eq!(validate_stateless(&op), Err(PoolOpValidationError::DuplicateSerial(hash(1))));
    }
}
