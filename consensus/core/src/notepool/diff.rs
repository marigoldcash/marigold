//! The pool's analog of `UtxoDiff` (`crate::utxo::utxo_diff`) — mutations plus their
//! inverses, so pool state can be applied and unapplied per chain block with the same
//! discipline the UTXO set already uses (POOL-SPEC.md P5.1/P5.4, FORK-PLAN P6.2).

use super::NewNote;
use crate::Hash;
use std::collections::HashMap;

/// `sn -> (d, pk)` — mirrors `crate::utxo::utxo_collection::UtxoCollection`'s role
/// exactly (`TransactionOutpoint -> UtxoEntry`), just keyed by note serial instead of
/// a transaction outpoint.
pub type PoolCollection = HashMap<Hash, NewNote>;

/// A pool-state mutation: entries to add, entries to remove. Reversing a `PoolDiff`
/// (swap `add`/`remove`) yields its exact inverse — the same trick `UtxoDiff::to_reversed`
/// uses to unapply a diff when a chain block is removed from the virtual chain (e.g. a
/// reorg past a pool op, P6.4's job to wire up; this type only needs to make that
/// operation cheap and correct).
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct PoolDiff {
    pub add: PoolCollection,
    pub remove: PoolCollection,
}

impl PoolDiff {
    pub fn new(add: PoolCollection, remove: PoolCollection) -> Self {
        Self { add, remove }
    }

    /// The exact inverse of this diff: applying `self` then `self.to_reversed()` (in
    /// that order, to the same state) is a no-op.
    pub fn to_reversed(self) -> Self {
        Self { add: self.remove, remove: self.add }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notepool::DenominationTag;

    fn note(byte: u8) -> NewNote {
        NewNote { d: DenominationTag::D1, pk: [byte; 32] }
    }

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    #[test]
    fn to_reversed_swaps_add_and_remove() {
        let diff = PoolDiff::new(PoolCollection::from([(hash(1), note(1))]), PoolCollection::from([(hash(2), note(2))]));
        let reversed = diff.clone().to_reversed();
        assert_eq!(reversed.add, diff.remove);
        assert_eq!(reversed.remove, diff.add);
    }

    #[test]
    fn double_reversal_is_identity() {
        let diff = PoolDiff::new(PoolCollection::from([(hash(1), note(1))]), PoolCollection::from([(hash(2), note(2))]));
        assert_eq!(diff.clone().to_reversed().to_reversed(), diff);
    }
}
