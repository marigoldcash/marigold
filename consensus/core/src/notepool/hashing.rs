//! Hash functions for note-pool state and authorization (POOL-SPEC.md P5.1/P5.2,
//! PLAN P6.2/P6.4).
//!
//! Separate hasher types per purpose, mirroring `consensus/seq-commit`'s split between
//! `SeqCommitActiveLeaf` (external leaf value) and `SeqCommitActiveNode` (SMT internal
//! branch combination): `NotePoolLeafHash`/`NotePoolSerialHash`/`NotePoolSigningHash`/
//! `NotePoolOutputsHash` here, while `NotePoolSmt`/`NotePoolSmtCollapsed` (registered in
//! `crypto/smt/build.rs`) handle the tree's internal hashing and are never called
//! directly from this module.

use super::{DenominationTag, NewNote, POOL_PROTOCOL_VERSION, PoolEntry, ProducedLock};
use crate::Hash;
use crate::tx::TransactionOutput;
use kaspa_hashes::{HasherBase, NotePoolLeafHash, NotePoolOutputsHash, NotePoolSerialHash, NotePoolSigningHash};

/// The pool SMT's leaf value hash: `H(d || pk)` (POOL-SPEC.md P5.1).
#[inline]
pub fn leaf_hash(d: DenominationTag, pk: &[u8; 32]) -> Hash {
    let mut hasher = NotePoolLeafHash::new();
    hasher.update([d as u8]).update(pk);
    hasher.finalize()
}

/// The leaf for a pool entry: `H(d || pk)` unlocked — unchanged from v1.1, so every
/// existing commitment stands — and `H(d || pk || refund_pk || until_daa)` locked
/// (POOL-SPEC.md P5.9). The lengths differ, so the two can never collide.
#[inline]
pub fn leaf_hash_entry(entry: &PoolEntry) -> Hash {
    match entry.lock {
        None => leaf_hash(entry.note.d, &entry.note.pk),
        Some(lock) => {
            let mut hasher = NotePoolLeafHash::new();
            hasher.update([entry.note.d as u8]).update(entry.note.pk).update(lock.refund_pk).update(lock.until_daa.to_le_bytes());
            hasher.finalize()
        }
    }
}

/// A note's serial: `sn = H_serial(creating_tx_id || index)` where `index` is the note's
/// position among all notes its op creates (u32 LE, POOL-SPEC.md P5.1/P5.2). Never stored
/// in any payload — every node derives it independently, which both saves 32 wire bytes
/// per note and removes any possibility of a payload lying about a note's `sn`.
#[inline]
pub fn serial_hash(creating_tx_id: &Hash, index: u32) -> Hash {
    let mut hasher = NotePoolSerialHash::new();
    hasher.update(creating_tx_id.as_bytes()).update(index.to_le_bytes());
    hasher.finalize()
}

/// `transparent_outputs_hash` — `H_outputs` over the enclosing transaction's outputs
/// (POOL-SPEC.md P5.2): for each output, in order, `amount (u64 LE) || script version
/// (u16 LE) || script bytes`. Over an empty output list (any pure `Transfer`) this is the
/// domain's empty-input hash. The per-output serialization deliberately mirrors the field
/// order the existing transaction sighash commits outputs with
/// (`crate::hashing::sighash`'s outputs hash) — same data, pool-domain-separated rather
/// than reusing the transparent sighash function, which carries `SigHashType` semantics
/// this scheme doesn't want.
pub fn transparent_outputs_hash(outputs: &[TransactionOutput]) -> Hash {
    let mut hasher = NotePoolOutputsHash::new();
    for output in outputs {
        hasher
            .update(output.value.to_le_bytes())
            .update(output.script_public_key.version().to_le_bytes())
            .update(output.script_public_key.script());
    }
    hasher.finalize()
}

/// `NotePoolSigningHash` — the message every `SignedGroup`'s BIP340 Schnorr signature
/// covers (POOL-SPEC.md P5.2, v1.1 preimage, pinned exactly):
///
/// ```text
/// H( "NotePoolSig"                 // domain tag (the hasher's blake3 key)
///    || pool_protocol_version      // u8 = 1
///    || op_type                    // u8 = the PoolOp borsh tag: 1=Transfer, 2=Redeem
///    || sorted(group.serials)      // ascending lexicographic byte order, 32 bytes each
///    || op.produced                // EVERY note the whole op creates, d||pk each (empty for Redeem)
///    || transparent_outputs_hash   // 32 bytes (see [`transparent_outputs_hash`])
///    || freshness.anchor_daa_score // u64 LE
/// )
/// ```
///
/// Each group signs over the **entire** op's `produced` list and the transaction's
/// transparent outputs, not merely its own serials — this is what makes a multi-group op
/// atomic and closes review 1's Redeem transaction-malleability vector (the signature
/// binds where redeemed value lands, not just that notes were consumed). No `tx_id` is
/// signed (it would be circular — the tx id hashes this very payload); replay safety
/// comes from current-`pk` verification plus the freshness window (P5.3).
///
/// `group_serials` may be passed in any order — this function sorts a local copy into the
/// canonical ascending byte order, so signer and validator always hash the same message
/// regardless of wire order.
pub fn signing_hash(
    op_type: u8,
    group_serials: &[Hash],
    produced: &[NewNote],
    transparent_outputs_hash: Hash,
    anchor_daa_score: u64,
) -> Hash {
    signing_hash_with_locks(op_type, group_serials, produced, &[], transparent_outputs_hash, anchor_daa_score)
}

/// [`signing_hash`] for a `TransferLockedOp` (POOL-SPEC.md P5.9): the locks follow
/// `produced` in the preimage — `count (u32 LE) || (index u32 LE || refund_pk ||
/// until_daa u64 LE)*` — so a relay can neither strip nor alter them. With no locks
/// the preimage is byte-for-byte the v1.1 one, which is how every existing
/// signature keeps verifying.
pub fn signing_hash_with_locks(
    op_type: u8,
    group_serials: &[Hash],
    produced: &[NewNote],
    locks: &[ProducedLock],
    transparent_outputs_hash: Hash,
    anchor_daa_score: u64,
) -> Hash {
    let mut sorted_serials: Vec<Hash> = group_serials.to_vec();
    sorted_serials.sort_unstable_by_key(|h| h.as_bytes());

    let mut hasher = NotePoolSigningHash::new();
    hasher.update([POOL_PROTOCOL_VERSION]).update([op_type]);
    for serial in &sorted_serials {
        hasher.update(serial.as_bytes());
    }
    for note in produced {
        hasher.update([note.d as u8]).update(note.pk);
    }
    if !locks.is_empty() {
        hasher.update((locks.len() as u32).to_le_bytes());
        for l in locks {
            hasher.update(l.index.to_le_bytes()).update(l.lock.refund_pk).update(l.lock.until_daa.to_le_bytes());
        }
    }
    hasher.update(transparent_outputs_hash.as_bytes()).update(anchor_daa_score.to_le_bytes());
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::ScriptPublicKey;

    #[test]
    fn leaf_hash_is_deterministic() {
        let pk = [7u8; 32];
        assert_eq!(leaf_hash(DenominationTag::D1, &pk), leaf_hash(DenominationTag::D1, &pk));
    }

    #[test]
    fn leaf_hash_differs_by_denomination() {
        let pk = [7u8; 32];
        assert_ne!(leaf_hash(DenominationTag::D1, &pk), leaf_hash(DenominationTag::D10, &pk));
    }

    #[test]
    fn leaf_hash_differs_by_pk() {
        assert_ne!(leaf_hash(DenominationTag::D1, &[1u8; 32]), leaf_hash(DenominationTag::D1, &[2u8; 32]));
    }

    #[test]
    fn serial_hash_differs_by_tx_id_and_index() {
        let tx_a = Hash::from_bytes([1; 32]);
        let tx_b = Hash::from_bytes([2; 32]);
        let base = serial_hash(&tx_a, 0);
        assert_eq!(base, serial_hash(&tx_a, 0));
        assert_ne!(base, serial_hash(&tx_b, 0));
        assert_ne!(base, serial_hash(&tx_a, 1));
    }

    #[test]
    fn signing_hash_is_serial_order_independent() {
        // The wire may carry serials in any order; the signed message is canonical.
        let s1 = Hash::from_bytes([1; 32]);
        let s2 = Hash::from_bytes([2; 32]);
        let out_hash = transparent_outputs_hash(&[]);
        assert_eq!(signing_hash(1, &[s1, s2], &[], out_hash, 5), signing_hash(1, &[s2, s1], &[], out_hash, 5));
    }

    #[test]
    fn signing_hash_binds_every_field() {
        let s = Hash::from_bytes([1; 32]);
        let note = NewNote { d: DenominationTag::D1, pk: [3; 32] };
        let out_hash = transparent_outputs_hash(&[]);
        let base = signing_hash(1, &[s], &[note], out_hash, 5);

        // op_type (Transfer vs Redeem contexts can never share a signature)
        assert_ne!(base, signing_hash(2, &[s], &[note], out_hash, 5));
        // serials
        assert_ne!(base, signing_hash(1, &[Hash::from_bytes([9; 32])], &[note], out_hash, 5));
        // produced list
        assert_ne!(base, signing_hash(1, &[s], &[], out_hash, 5));
        // transparent outputs
        let other_out = transparent_outputs_hash(&[TransactionOutput::new(100, ScriptPublicKey::new(0, Default::default()))]);
        assert_ne!(base, signing_hash(1, &[s], &[note], other_out, 5));
        // freshness anchor
        assert_ne!(base, signing_hash(1, &[s], &[note], out_hash, 6));
    }

    #[test]
    fn outputs_hash_binds_amount_version_and_script() {
        let spk = |version, script: &[u8]| ScriptPublicKey::new(version, script.iter().copied().collect());
        let base = transparent_outputs_hash(&[TransactionOutput::new(100, spk(0, &[1, 2, 3]))]);
        assert_ne!(base, transparent_outputs_hash(&[TransactionOutput::new(101, spk(0, &[1, 2, 3]))]));
        assert_ne!(base, transparent_outputs_hash(&[TransactionOutput::new(100, spk(1, &[1, 2, 3]))]));
        assert_ne!(base, transparent_outputs_hash(&[TransactionOutput::new(100, spk(0, &[1, 2, 4]))]));
        assert_ne!(base, transparent_outputs_hash(&[]));
    }
}
