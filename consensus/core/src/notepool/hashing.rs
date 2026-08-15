//! Hash functions for note-pool state (POOL-SPEC.md P5.1, FORK-PLAN P6.2).
//!
//! Two separate hasher types are used, mirroring `consensus/seq-commit`'s split between
//! `SeqCommitActiveLeaf` (external leaf value) and `SeqCommitActiveNode` (SMT internal
//! branch combination): `NotePoolLeafHash` here for the leaf value, while `NotePoolSmt`/
//! `NotePoolSmtCollapsed` (registered in `crypto/smt/build.rs`) handle the tree's internal
//! hashing and are never called directly from this module.

use super::DenominationTag;
use crate::Hash;
use kaspa_hashes::{HasherBase, NotePoolLeafHash};

/// The pool SMT's leaf value hash: `H(d || pk)` (POOL-SPEC.md P5.1).
#[inline]
pub fn leaf_hash(d: DenominationTag, pk: &[u8; 32]) -> Hash {
    let mut hasher = NotePoolLeafHash::new();
    hasher.update([d as u8]).update(pk);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
