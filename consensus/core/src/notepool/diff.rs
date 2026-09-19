//! The pool's analog of `UtxoDiff` (`crate::utxo::utxo_diff`) — mutations plus their
//! inverses, so pool state can be applied and unapplied per chain block with the same
//! discipline the UTXO set already uses (POOL-SPEC.md P5.1/P5.4, PLAN P6.2/P6.4).
//!
//! The composition algebra (`with_diff_in_place`) mirrors `UtxoDiff`'s exactly, minus the
//! DAA-score dimension: a live serial's `(d, pk)` value is immutable (invariants I1/I3,
//! POOL-SPEC.md P5.1), so where `UtxoDiff` must compare entries' DAA scores to tell "same
//! logical UTXO" from "recreated at a different score", the pool needs only the serial.

use super::{NewNote, PoolEntry};
use crate::Hash;
use crate::errors::notepool::PoolAlgebraError;
use kaspa_utils::mem_size::MemSizeEstimator;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::Entry::Vacant;

/// `sn -> (d, pk)` — mirrors `crate::utxo::utxo_collection::UtxoCollection`'s role
/// exactly (`TransactionOutpoint -> UtxoEntry`), just keyed by note serial instead of
/// a transaction outpoint.
pub type PoolCollection = HashMap<Hash, PoolEntry>;

/// Read-only view over a diff's two sides — the pool analog of
/// `crate::utxo::utxo_diff::ImmutableUtxoDiff`, existing for the same reason: reorg
/// walk-downs apply stored diffs *in reverse* without cloning them
/// ([`PoolDiff::as_reversed`]).
pub trait ImmutablePoolDiff {
    fn added(&self) -> &PoolCollection;
    fn removed(&self) -> &PoolCollection;
}

impl<T: ImmutablePoolDiff> ImmutablePoolDiff for &T {
    fn added(&self) -> &PoolCollection {
        (*self).added()
    }
    fn removed(&self) -> &PoolCollection {
        (*self).removed()
    }
}

/// A pool-state mutation: entries to add, entries to remove. Reversing a `PoolDiff`
/// (swap `add`/`remove`) yields its exact inverse — the same trick `UtxoDiff::to_reversed`
/// uses to unapply a diff when a chain block is removed from the virtual chain (a reorg
/// past a pool op; wired up by the virtual processor per P6.4).
///
/// Carries `serde` derives (bincode) for the per-chain-block diff store
/// (`consensus/src/model/stores/notepool_diffs.rs`) — a second, independent
/// serialization from the borsh wire format, same split `UtxoDiff` has.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolDiff {
    pub add: PoolCollection,
    pub remove: PoolCollection,
}

/// A diff as the v1.1 stores wrote it — `NewNote` values, no lock. Read from
/// the pre-P5.9 prefixes when a block's diff is not under the new one, so a
/// node upgraded mid-chain can still walk and unwind its old blocks.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPoolDiff {
    pub add: HashMap<Hash, NewNote>,
    pub remove: HashMap<Hash, NewNote>,
}

impl MemSizeEstimator for LegacyPoolDiff {
    fn estimate_mem_bytes(&self) -> usize {
        size_of::<Self>() + (self.add.len() + self.remove.len()) * (size_of::<Hash>() + size_of::<NewNote>())
    }
}

impl From<LegacyPoolDiff> for PoolDiff {
    fn from(legacy: LegacyPoolDiff) -> Self {
        Self {
            add: legacy.add.into_iter().map(|(sn, note)| (sn, PoolEntry::unlocked(note))).collect(),
            remove: legacy.remove.into_iter().map(|(sn, note)| (sn, PoolEntry::unlocked(note))).collect(),
        }
    }
}

impl MemSizeEstimator for PoolDiff {
    fn estimate_mem_bytes(&self) -> usize {
        size_of::<Self>() + (self.add.len() + self.remove.len()) * (size_of::<Hash>() + size_of::<PoolEntry>())
    }
}

impl ImmutablePoolDiff for PoolDiff {
    fn added(&self) -> &PoolCollection {
        &self.add
    }
    fn removed(&self) -> &PoolCollection {
        &self.remove
    }
}

/// Borrowed reversed view — `added()`/`removed()` swapped (see
/// [`PoolDiff::as_reversed`]).
pub struct ReversedPoolDiff<'a> {
    inner: &'a PoolDiff,
}

impl ImmutablePoolDiff for ReversedPoolDiff<'_> {
    fn added(&self) -> &PoolCollection {
        &self.inner.remove
    }
    fn removed(&self) -> &PoolCollection {
        &self.inner.add
    }
}

impl PoolDiff {
    pub fn new(add: PoolCollection, remove: PoolCollection) -> Self {
        Self { add, remove }
    }

    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }

    /// Borrowed reversed view, for applying a stored diff in reverse without cloning.
    pub fn as_reversed(&self) -> impl ImmutablePoolDiff + '_ {
        ReversedPoolDiff { inner: self }
    }

    /// The exact inverse of this diff: applying `self` then `self.to_reversed()` (in
    /// that order, to the same state) is a no-op.
    pub fn to_reversed(self) -> Self {
        Self { add: self.remove, remove: self.add }
    }

    /// Applies `other` to this diff in place: the result equals applying first `self`,
    /// then `other`, to the same base pool state. Mirrors `UtxoDiff::with_diff_in_place`,
    /// two-phase (validate, then mutate):
    ///
    /// - `other.removed` ∩ `self.remove` without `self.add` covering it → double remove.
    /// - `other.added` ∩ `self.add` without `other.removed` covering it → double add.
    /// - remove of something `self` added cancels the add; add of something `self`
    ///   removed cancels the remove (the reorg-reversal case).
    pub fn with_diff_in_place(&mut self, other: &impl ImmutablePoolDiff) -> Result<(), PoolAlgebraError> {
        if let Some(&sn) = other.removed().keys().find(|sn| self.remove.contains_key(sn) && !self.add.contains_key(sn)) {
            return Err(PoolAlgebraError::DuplicateRemove(sn));
        }
        if let Some(&sn) = other.added().keys().find(|sn| self.add.contains_key(sn) && !other.removed().contains_key(sn)) {
            return Err(PoolAlgebraError::DuplicateAdd(sn));
        }

        for (sn, note) in other.removed() {
            if let Some(existing) = self.add.remove(sn) {
                debug_assert_eq!(existing, *note, "a serial's (d, pk) is immutable while live (invariants I1/I3)");
            } else {
                self.remove.insert(*sn, *note);
            }
        }
        for (sn, note) in other.added() {
            if let Some(existing) = self.remove.remove(sn) {
                debug_assert_eq!(existing, *note, "a serial's (d, pk) is immutable while live (invariants I1/I3)");
            } else {
                self.add.insert(*sn, *note);
            }
        }
        Ok(())
    }

    /// Records one produced note — mirrors `UtxoDiff::add_entry`.
    pub fn add_note(&mut self, sn: Hash, note: PoolEntry) -> Result<(), PoolAlgebraError> {
        if self.remove.remove(&sn).is_some() {
            Ok(())
        } else if let Vacant(e) = self.add.entry(sn) {
            e.insert(note);
            Ok(())
        } else {
            Err(PoolAlgebraError::DuplicateAdd(sn))
        }
    }

    /// Records one consumed note — mirrors `UtxoDiff::remove_entry`.
    pub fn remove_note(&mut self, sn: Hash, note: PoolEntry) -> Result<(), PoolAlgebraError> {
        if self.add.remove(&sn).is_some() {
            Ok(())
        } else if let Vacant(e) = self.remove.entry(sn) {
            e.insert(note);
            Ok(())
        } else {
            Err(PoolAlgebraError::DuplicateRemove(sn))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notepool::DenominationTag;

    fn note(byte: u8) -> PoolEntry {
        PoolEntry::unlocked(NewNote { d: DenominationTag::D1, pk: [byte; 32] })
    }

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn diff(add: &[u8], remove: &[u8]) -> PoolDiff {
        PoolDiff::new(add.iter().map(|&b| (hash(b), note(b))).collect(), remove.iter().map(|&b| (hash(b), note(b))).collect())
    }

    #[test]
    fn to_reversed_swaps_add_and_remove() {
        let d = diff(&[1], &[2]);
        let reversed = d.clone().to_reversed();
        assert_eq!(reversed.add, d.remove);
        assert_eq!(reversed.remove, d.add);
    }

    #[test]
    fn double_reversal_is_identity() {
        let d = diff(&[1], &[2]);
        assert_eq!(d.clone().to_reversed().to_reversed(), d);
    }

    #[test]
    fn with_diff_composes_disjoint_diffs() {
        let mut a = diff(&[1], &[2]);
        a.with_diff_in_place(&diff(&[3], &[4])).unwrap();
        assert_eq!(a, diff(&[1, 3], &[2, 4]));
    }

    #[test]
    fn remove_of_added_serial_cancels() {
        // Mint sn=1, then a later diff consumes it: net effect is no trace of sn=1.
        let mut a = diff(&[1], &[]);
        a.with_diff_in_place(&diff(&[], &[1])).unwrap();
        assert_eq!(a, PoolDiff::default());
    }

    #[test]
    fn add_of_removed_serial_cancels() {
        // The reorg-reversal case: a diff removed sn=1, its reversal re-adds it.
        let mut a = diff(&[], &[1]);
        a.with_diff_in_place(&diff(&[1], &[])).unwrap();
        assert_eq!(a, PoolDiff::default());
    }

    #[test]
    fn double_remove_is_an_error() {
        let mut a = diff(&[], &[1]);
        assert_eq!(a.with_diff_in_place(&diff(&[], &[1])), Err(PoolAlgebraError::DuplicateRemove(hash(1))));
    }

    #[test]
    fn double_add_is_an_error() {
        let mut a = diff(&[1], &[]);
        assert_eq!(a.with_diff_in_place(&diff(&[1], &[])), Err(PoolAlgebraError::DuplicateAdd(hash(1))));
    }

    #[test]
    fn apply_then_apply_reversed_is_identity() {
        let mut acc = diff(&[1, 2], &[3]);
        let step = diff(&[4], &[1]);
        acc.with_diff_in_place(&step).unwrap();
        acc.with_diff_in_place(&step.as_reversed()).unwrap();
        assert_eq!(acc, diff(&[1, 2], &[3]));
    }

    #[test]
    fn add_and_remove_note_cancel_within_one_diff() {
        // One mergeset: block A mints sn=1, block B (later in blue order) consumes it.
        let mut d = PoolDiff::default();
        d.add_note(hash(1), note(1)).unwrap();
        d.remove_note(hash(1), note(1)).unwrap();
        assert_eq!(d, PoolDiff::default());
    }

    #[test]
    fn duplicate_add_and_remove_note_calls_error() {
        let mut d = PoolDiff::default();
        d.add_note(hash(1), note(1)).unwrap();
        assert_eq!(d.add_note(hash(1), note(1)), Err(PoolAlgebraError::DuplicateAdd(hash(1))));
        let mut d = PoolDiff::default();
        d.remove_note(hash(2), note(2)).unwrap();
        assert_eq!(d.remove_note(hash(2), note(2)), Err(PoolAlgebraError::DuplicateRemove(hash(2))));
    }
}
