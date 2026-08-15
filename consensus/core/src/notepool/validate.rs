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
//!
//! [`validate_stateful`] (P6.4) is the second half: full P5.3 validation against a live
//! composed [`PoolStateView`].

use super::{FreshnessAnchor, MintOp, POOL_FRESHNESS_WINDOW, PoolDiff, PoolOp, RedeemOp, SignedGroup, TransferOp};
use super::{PoolStateView, hashing};
use crate::Hash;
use crate::errors::notepool::{PoolOpContextError, PoolOpValidationError};
use crate::tx::TransactionOutput;
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

/// The outcome of full stateful validation: the op's pool-state mutation plus its petal
/// totals (consumed/produced), which P6.6's value binding and fee crediting will consume.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidatedPoolOp {
    pub diff: PoolDiff,
    pub consumed_petals: u64,
    pub produced_petals: u64,
}

/// Full stateful validation of a `PoolOp` against a composed pool view — POOL-SPEC.md
/// P5.3's validation orders, executed inside the virtual pipeline's mergeset walk
/// (FORK-PLAN P6.4). Assumes [`validate_stateless`] already passed (enforced at block
/// body validation); debug-asserts it.
///
/// `skip_signature_and_freshness` mirrors `TxValidationFlags::SkipScriptChecks`'s role
/// for the selected-parent replay: when a chain block replays its selected parent's own
/// transactions (already fully validated during that parent's chain qualification,
/// against the identical state basis), signatures need no re-verification — and
/// freshness MUST not be re-checked, because unlike maturity/sequence-locks it is
/// non-monotonic in the POV DAA score (an op fresh at the parent's POV can be stale at
/// the child's), so re-imposing it at replay would make the child's acceptance data
/// diverge from what the parent already committed. Existence, conservation, and diff
/// construction always run (they are needed to build the diff, and hold identically on
/// the replay basis).
///
/// Value binding to the transparent side (mint input sums, redeem output sums, fee
/// crediting) is deliberately NOT checked here — FORK-PLAN P6.6 owns it. `Transfer`
/// conservation (pool-side only, needs nothing transparent) IS enforced.
pub fn validate_stateful<V: PoolStateView>(
    op: &PoolOp,
    tx_id: Hash,
    tx_outputs: &[TransactionOutput],
    pool_view: &V,
    pov_daa_score: u64,
    skip_signature_and_freshness: bool,
) -> Result<ValidatedPoolOp, PoolOpContextError> {
    debug_assert_eq!(validate_stateless(op), Ok(()), "stateless validity is enforced at block body validation");
    match op {
        PoolOp::Mint(mint) => validate_mint_stateful(mint, tx_id, pool_view),
        PoolOp::Transfer(transfer) => {
            validate_transfer_stateful(transfer, tx_id, tx_outputs, pool_view, pov_daa_score, skip_signature_and_freshness)
        }
        PoolOp::Redeem(redeem) => validate_redeem_stateful(redeem, tx_outputs, pool_view, pov_daa_score, skip_signature_and_freshness),
    }
}

fn validate_mint_stateful<V: PoolStateView>(mint: &MintOp, tx_id: Hash, pool_view: &V) -> Result<ValidatedPoolOp, PoolOpContextError> {
    let mut validated = ValidatedPoolOp::default();
    for (i, note) in mint.new_notes.iter().enumerate() {
        let sn = hashing::serial_hash(&tx_id, i as u32);
        // P5.3 Mint step 4 calls serial uniqueness "guaranteed by construction ... not an
        // active check" — but that reasoning assumes P6.6's value binding (a mint MUST
        // spend transparent inputs, so the same mint tx can never be accepted twice: its
        // second instance is a UTXO double-spend). Until P6.6 lands, a zero-input mint
        // duplicated across parallel blocks would otherwise validate in both contexts, so
        // this check is REAL consensus for now; after P6.6 it degrades to the cheap
        // insurance the spec permits.
        check_produced_serial_is_fresh(sn, pool_view)?;
        validated.diff.add_note(sn, *note)?;
        validated.produced_petals += note.d.petals();
    }
    Ok(validated)
}

/// Rejects a produced serial that already exists in the composed view. One map lookup per
/// produced note; deterministically excludes the "same op accepted twice in one mergeset"
/// class instead of letting it surface as a diff-accumulation panic downstream.
fn check_produced_serial_is_fresh<V: PoolStateView>(sn: Hash, pool_view: &V) -> Result<(), PoolOpContextError> {
    if pool_view.get_note(&sn).is_some() {
        return Err(PoolOpContextError::SerialAlreadyExists(sn));
    }
    Ok(())
}

fn validate_transfer_stateful<V: PoolStateView>(
    transfer: &TransferOp,
    tx_id: Hash,
    tx_outputs: &[TransactionOutput],
    pool_view: &V,
    pov_daa_score: u64,
    skip_signature_and_freshness: bool,
) -> Result<ValidatedPoolOp, PoolOpContextError> {
    if !skip_signature_and_freshness {
        check_freshness(&transfer.freshness, pov_daa_score)?;
    }

    let mut validated = ValidatedPoolOp::default();
    let outputs_hash = hashing::transparent_outputs_hash(tx_outputs);
    validated.consumed_petals = validate_consumed_groups_stateful(
        &transfer.consumed,
        /* op_type */ 1, // PoolOp borsh tag: Transfer
        &transfer.produced,
        outputs_hash,
        transfer.freshness.anchor_daa_score,
        pool_view,
        skip_signature_and_freshness,
        &mut validated.diff,
    )?;

    for (i, note) in transfer.produced.iter().enumerate() {
        let sn = hashing::serial_hash(&tx_id, i as u32);
        check_produced_serial_is_fresh(sn, pool_view)?;
        validated.diff.add_note(sn, *note)?;
        validated.produced_petals += note.d.petals();
    }

    // Conservation (P5.3 Transfer step 5): the difference is the op's fee (crediting it
    // to the miner is P6.6's job; the inequality is consensus now).
    if validated.consumed_petals < validated.produced_petals {
        return Err(PoolOpContextError::InsufficientConsumedValue {
            consumed: validated.consumed_petals,
            produced: validated.produced_petals,
        });
    }
    Ok(validated)
}

fn validate_redeem_stateful<V: PoolStateView>(
    redeem: &RedeemOp,
    tx_outputs: &[TransactionOutput],
    pool_view: &V,
    pov_daa_score: u64,
    skip_signature_and_freshness: bool,
) -> Result<ValidatedPoolOp, PoolOpContextError> {
    if !skip_signature_and_freshness {
        check_freshness(&redeem.freshness, pov_daa_score)?;
    }

    let mut validated = ValidatedPoolOp::default();
    let outputs_hash = hashing::transparent_outputs_hash(tx_outputs);
    validated.consumed_petals = validate_consumed_groups_stateful(
        &redeem.consumed,
        /* op_type */ 2, // PoolOp borsh tag: Redeem
        /* produced (always empty for Redeem) */ &[],
        outputs_hash,
        redeem.freshness.anchor_daa_score,
        pool_view,
        skip_signature_and_freshness,
        &mut validated.diff,
    )?;
    // Conservation vs transparent outputs (`Σ consumed >= Σ outputs + fee`) is the
    // transparent-side value binding P6.6 owns — not checked here.
    Ok(validated)
}

/// P5.3 steps 1-2 for `Transfer`/`Redeem`: every serial exists in the composed view, all
/// serials in a group share one current `pk`, and the group's signature verifies against
/// that `pk` over the exact P5.2 signing hash. Records every consumed note in `diff` and
/// returns the total consumed petal value.
#[allow(clippy::too_many_arguments)]
fn validate_consumed_groups_stateful<V: PoolStateView>(
    groups: &[SignedGroup],
    op_type: u8,
    produced: &[super::NewNote],
    outputs_hash: Hash,
    anchor_daa_score: u64,
    pool_view: &V,
    skip_signature: bool,
    diff: &mut PoolDiff,
) -> Result<u64, PoolOpContextError> {
    let mut consumed_petals = 0u64;
    for (i, group) in groups.iter().enumerate() {
        let mut group_pk: Option<[u8; 32]> = None;
        for &sn in &group.serials {
            let note = pool_view.get_note(&sn).ok_or(PoolOpContextError::SerialNotFound(sn))?;
            match group_pk {
                None => group_pk = Some(note.pk),
                Some(pk) if pk != note.pk => return Err(PoolOpContextError::MixedKeysInGroup(i)),
                Some(_) => {}
            }
            diff.remove_note(sn, note)?;
            consumed_petals += note.d.petals();
        }
        if !skip_signature {
            let pk = group_pk.expect("groups are non-empty (stateless validation)");
            let xonly = secp256k1::XOnlyPublicKey::from_slice(&pk).map_err(|_| PoolOpContextError::BadPublicKey(i))?;
            let sig = secp256k1::schnorr::Signature::from_slice(&group.signature).expect("[u8; 64] is always length-valid");
            let msg_hash = hashing::signing_hash(op_type, &group.serials, produced, outputs_hash, anchor_daa_score);
            let msg = secp256k1::Message::from_digest(msg_hash.into());
            sig.verify(&msg, &xonly).map_err(|_| PoolOpContextError::BadSignature(i))?;
        }
    }
    Ok(consumed_petals)
}

/// P5.3 step 3: valid iff `0 <= pov - anchor <= POOL_FRESHNESS_WINDOW`, inclusive on
/// both ends (boundary semantics pinned per review 1 — a difference of exactly 0 and
/// exactly 36,000 are both valid; 36,001 is not; a future anchor is not).
fn check_freshness(freshness: &FreshnessAnchor, pov_daa_score: u64) -> Result<(), PoolOpContextError> {
    let anchor = freshness.anchor_daa_score;
    if anchor > pov_daa_score {
        return Err(PoolOpContextError::AnchorInFuture { anchor, pov: pov_daa_score });
    }
    if pov_daa_score - anchor > POOL_FRESHNESS_WINDOW {
        return Err(PoolOpContextError::StaleAnchor { anchor, pov: pov_daa_score, window: POOL_FRESHNESS_WINDOW });
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

    /// Stateful-validation tests (P6.4) with real Schnorr keys against an in-memory
    /// `PoolCollection` view.
    mod stateful {
        use super::*;
        use crate::notepool::{PoolCollection, hashing};

        struct Wallet {
            keypair: secp256k1::Keypair,
            pk: [u8; 32],
        }

        impl Wallet {
            fn new(seed: u8) -> Self {
                let keypair = secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[seed; 32]).unwrap();
                let pk = keypair.public_key().x_only_public_key().0.serialize();
                Self { keypair, pk }
            }

            fn sign(&self, op_type: u8, serials: &[Hash], produced: &[NewNote], anchor: u64) -> [u8; 64] {
                let outputs_hash = hashing::transparent_outputs_hash(&[]);
                let msg_hash = hashing::signing_hash(op_type, serials, produced, outputs_hash, anchor);
                let msg = secp256k1::Message::from_digest(msg_hash.into());
                *self.keypair.sign_schnorr(msg).as_ref()
            }
        }

        fn pool_with(entries: &[(Hash, NewNote)]) -> PoolCollection {
            entries.iter().copied().collect()
        }

        fn owned_note(wallet: &Wallet, d: DenominationTag) -> NewNote {
            NewNote { d, pk: wallet.pk }
        }

        /// A signed 1-in/1-out rotate of `sn` to `dest`, anchored at `anchor`.
        fn rotate_op(wallet: &Wallet, sn: Hash, dest: NewNote, anchor: u64) -> PoolOp {
            let signature = wallet.sign(1, &[sn], &[dest], anchor);
            PoolOp::Transfer(TransferOp {
                consumed: vec![SignedGroup { serials: vec![sn], signature }],
                produced: vec![dest],
                freshness: FreshnessAnchor { anchor_daa_score: anchor },
            })
        }

        #[test]
        fn valid_rotate_produces_expected_diff() {
            let wallet = Wallet::new(1);
            let sn = hash(10);
            let consumed_note = owned_note(&wallet, DenominationTag::D1);
            let dest = note(2);
            let view = pool_with(&[(sn, consumed_note)]);
            let op = rotate_op(&wallet, sn, dest, 50);

            let validated = validate_stateful(&op, hash(0xAA), &[], &view, 100, false).unwrap();
            assert_eq!(validated.consumed_petals, DenominationTag::D1.petals());
            assert_eq!(validated.produced_petals, DenominationTag::D1.petals());
            assert_eq!(validated.diff.remove, pool_with(&[(sn, consumed_note)]));
            assert_eq!(validated.diff.add, pool_with(&[(hashing::serial_hash(&hash(0xAA), 0), dest)]));
        }

        #[test]
        fn missing_serial_rejected() {
            let wallet = Wallet::new(1);
            let op = rotate_op(&wallet, hash(10), note(2), 50);
            let empty = PoolCollection::default();
            assert_eq!(validate_stateful(&op, hash(0xAA), &[], &empty, 100, false), Err(PoolOpContextError::SerialNotFound(hash(10))));
        }

        #[test]
        fn signature_by_wrong_key_rejected() {
            let owner = Wallet::new(1);
            let thief = Wallet::new(2);
            let sn = hash(10);
            let view = pool_with(&[(sn, owned_note(&owner, DenominationTag::D1))]);
            // Signed by a key that is NOT the serial's current pk.
            let op = rotate_op(&thief, sn, note(2), 50);
            assert_eq!(validate_stateful(&op, hash(0xAA), &[], &view, 100, false), Err(PoolOpContextError::BadSignature(0)));
        }

        #[test]
        fn signature_over_different_produced_list_rejected() {
            // A valid signature lifted into a transaction with a different produced list
            // must fail — the signed message covers the entire produced list.
            let wallet = Wallet::new(1);
            let sn = hash(10);
            let view = pool_with(&[(sn, owned_note(&wallet, DenominationTag::D1))]);
            let signature = wallet.sign(1, &[sn], &[note(2)], 50); // signs produced=[note(2)]
            let op = PoolOp::Transfer(TransferOp {
                consumed: vec![SignedGroup { serials: vec![sn], signature }],
                produced: vec![note(3)], // attacker swapped the destination
                freshness: FreshnessAnchor { anchor_daa_score: 50 },
            });
            assert_eq!(validate_stateful(&op, hash(0xAA), &[], &view, 100, false), Err(PoolOpContextError::BadSignature(0)));
        }

        #[test]
        fn mixed_keys_in_one_group_rejected() {
            let a = Wallet::new(1);
            let b = Wallet::new(2);
            let view = pool_with(&[(hash(10), owned_note(&a, DenominationTag::D1)), (hash(11), owned_note(&b, DenominationTag::D1))]);
            let signature = a.sign(1, &[hash(10), hash(11)], &[note(2)], 50);
            let op = PoolOp::Transfer(TransferOp {
                consumed: vec![SignedGroup { serials: vec![hash(10), hash(11)], signature }],
                produced: vec![note(2)],
                freshness: FreshnessAnchor { anchor_daa_score: 50 },
            });
            assert_eq!(validate_stateful(&op, hash(0xAA), &[], &view, 100, false), Err(PoolOpContextError::MixedKeysInGroup(0)));
        }

        #[test]
        fn freshness_window_boundaries_are_inclusive() {
            let wallet = Wallet::new(1);
            let sn = hash(10);
            let make = |anchor| rotate_op(&wallet, sn, note(2), anchor);
            let view = pool_with(&[(sn, owned_note(&wallet, DenominationTag::D1))]);

            // pov - anchor == 0: valid.
            assert!(validate_stateful(&make(100), hash(0xAA), &[], &view, 100, false).is_ok());
            // pov - anchor == WINDOW exactly: valid (inclusive).
            assert!(validate_stateful(&make(100), hash(0xAA), &[], &view, 100 + POOL_FRESHNESS_WINDOW, false).is_ok());
            // pov - anchor == WINDOW + 1: stale.
            let pov = 100 + POOL_FRESHNESS_WINDOW + 1;
            assert_eq!(
                validate_stateful(&make(100), hash(0xAA), &[], &view, pov, false),
                Err(PoolOpContextError::StaleAnchor { anchor: 100, pov, window: POOL_FRESHNESS_WINDOW })
            );
            // anchor > pov: future anchor.
            assert_eq!(
                validate_stateful(&make(101), hash(0xAA), &[], &view, 100, false),
                Err(PoolOpContextError::AnchorInFuture { anchor: 101, pov: 100 })
            );
        }

        #[test]
        fn stale_anchor_accepted_on_selected_parent_replay() {
            // The consensus-critical replay rule: freshness is NOT re-imposed when a
            // chain block replays its selected parent's already-accepted ops.
            let wallet = Wallet::new(1);
            let sn = hash(10);
            let view = pool_with(&[(sn, owned_note(&wallet, DenominationTag::D1))]);
            let op = rotate_op(&wallet, sn, note(2), 100);
            let stale_pov = 100 + POOL_FRESHNESS_WINDOW + 1;
            assert!(validate_stateful(&op, hash(0xAA), &[], &view, stale_pov, false).is_err());
            assert!(validate_stateful(&op, hash(0xAA), &[], &view, stale_pov, true).is_ok());
        }

        #[test]
        fn transfer_conservation_enforced() {
            // Consume 1 MAGLD, try to produce 10 MAGLD.
            let wallet = Wallet::new(1);
            let sn = hash(10);
            let view = pool_with(&[(sn, owned_note(&wallet, DenominationTag::D1))]);
            let inflated = NewNote { d: DenominationTag::D10, pk: [2; 32] };
            let op = rotate_op(&wallet, sn, inflated, 50);
            assert_eq!(
                validate_stateful(&op, hash(0xAA), &[], &view, 100, false),
                Err(PoolOpContextError::InsufficientConsumedValue {
                    consumed: DenominationTag::D1.petals(),
                    produced: DenominationTag::D10.petals()
                })
            );
        }

        #[test]
        fn transfer_split_under_conservation_passes() {
            // 1 MAGLD -> 9 x 0.1 MAGLD (0.1 to fee): valid split.
            let wallet = Wallet::new(1);
            let sn = hash(10);
            let view = pool_with(&[(sn, owned_note(&wallet, DenominationTag::D1))]);
            let produced: Vec<NewNote> = (0..9).map(|i| NewNote { d: DenominationTag::D0_1, pk: [i; 32] }).collect();
            let signature = wallet.sign(1, &[sn], &produced, 50);
            let op = PoolOp::Transfer(TransferOp {
                consumed: vec![SignedGroup { serials: vec![sn], signature }],
                produced: produced.clone(),
                freshness: FreshnessAnchor { anchor_daa_score: 50 },
            });
            let validated = validate_stateful(&op, hash(0xAA), &[], &view, 100, false).unwrap();
            assert_eq!(validated.consumed_petals - validated.produced_petals, DenominationTag::D0_1.petals());
            assert_eq!(validated.diff.add.len(), 9);
        }

        #[test]
        fn merchant_sweep_one_signature_many_serials() {
            // Many serials under ONE shared pk, authorized by one signature (P5.6's POS flow).
            let pos = Wallet::new(1);
            let entries: Vec<(Hash, NewNote)> = (10..15).map(|i| (hash(i), owned_note(&pos, DenominationTag::D1))).collect();
            let view = pool_with(&entries);
            let serials: Vec<Hash> = entries.iter().map(|(sn, _)| *sn).collect();
            let produced: Vec<NewNote> = (0..5).map(|i| NewNote { d: DenominationTag::D1, pk: [100 + i; 32] }).collect();
            let signature = pos.sign(1, &serials, &produced, 50);
            let op = PoolOp::Transfer(TransferOp {
                consumed: vec![SignedGroup { serials, signature }],
                produced,
                freshness: FreshnessAnchor { anchor_daa_score: 50 },
            });
            let validated = validate_stateful(&op, hash(0xAA), &[], &view, 100, false).unwrap();
            assert_eq!(validated.diff.remove.len(), 5);
            assert_eq!(validated.diff.add.len(), 5);
        }

        #[test]
        fn mint_derives_serials_and_adds_notes() {
            let view = PoolCollection::default();
            let op = PoolOp::Mint(MintOp { new_notes: vec![note(1), note(2)] });
            let validated = validate_stateful(&op, hash(0xAA), &[], &view, 100, false).unwrap();
            assert_eq!(validated.produced_petals, 2 * DenominationTag::D1.petals());
            assert_eq!(validated.diff.remove.len(), 0);
            assert_eq!(
                validated.diff.add,
                pool_with(&[
                    (hashing::serial_hash(&hash(0xAA), 0), note(1)),
                    (hashing::serial_hash(&hash(0xAA), 1), note(2))
                ])
            );
        }

        #[test]
        fn redeem_removes_consumed_serials() {
            let wallet = Wallet::new(1);
            let sn = hash(10);
            let consumed_note = owned_note(&wallet, DenominationTag::D1);
            let view = pool_with(&[(sn, consumed_note)]);
            let signature = wallet.sign(2, &[sn], &[], 50);
            let op = PoolOp::Redeem(RedeemOp {
                consumed: vec![SignedGroup { serials: vec![sn], signature }],
                freshness: FreshnessAnchor { anchor_daa_score: 50 },
            });
            let validated = validate_stateful(&op, hash(0xAA), &[], &view, 100, false).unwrap();
            assert_eq!(validated.consumed_petals, DenominationTag::D1.petals());
            assert_eq!(validated.diff.remove, pool_with(&[(sn, consumed_note)]));
            assert!(validated.diff.add.is_empty());
        }

        #[test]
        fn transfer_signature_cannot_authorize_redeem() {
            // op_type domain separation: a Transfer signature over the same serials must
            // not verify in a Redeem context.
            let wallet = Wallet::new(1);
            let sn = hash(10);
            let view = pool_with(&[(sn, owned_note(&wallet, DenominationTag::D1))]);
            let transfer_sig = wallet.sign(1, &[sn], &[], 50);
            let op = PoolOp::Redeem(RedeemOp {
                consumed: vec![SignedGroup { serials: vec![sn], signature: transfer_sig }],
                freshness: FreshnessAnchor { anchor_daa_score: 50 },
            });
            assert_eq!(validate_stateful(&op, hash(0xAA), &[], &view, 100, false), Err(PoolOpContextError::BadSignature(0)));
        }
    }
}
