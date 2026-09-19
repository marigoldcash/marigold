//! Read-only pool-state views and diff composition — the pool analog of
//! `crate::utxo::utxo_view` (`UtxoView`/`ComposedUtxoView`), existing for the same
//! reason: the virtual pipeline validates each merged block against the selected
//! parent's state *plus* the mergeset diff accumulated so far, without materializing
//! intermediate states (POOL-SPEC.md P5.3's composed view, PLAN P6.4).

use super::PoolEntry;
use super::diff::{ImmutablePoolDiff, PoolCollection};
use crate::Hash;

/// An abstraction for read-only queries over pool state (`sn -> (d, pk)`).
pub trait PoolStateView {
    fn get_note(&self, sn: &Hash) -> Option<PoolEntry>;
}

/// Composes a pool view from a base view and a pool diff. Nests, like
/// `ComposedUtxoView`, to stack any number of diff layers.
pub struct ComposedPoolView<V: PoolStateView, D: ImmutablePoolDiff> {
    base: V,
    diff: D,
}

impl<V: PoolStateView, D: ImmutablePoolDiff> ComposedPoolView<V, D> {
    pub fn new(base: V, diff: D) -> Self {
        Self { base, diff }
    }
}

impl<V: PoolStateView, D: ImmutablePoolDiff> PoolStateView for ComposedPoolView<V, D> {
    fn get_note(&self, sn: &Hash) -> Option<PoolEntry> {
        if let Some(note) = self.diff.added().get(sn) {
            return Some(*note);
        }
        if self.diff.removed().contains_key(sn) {
            return None;
        }
        self.base.get_note(sn)
    }
}

impl<T: PoolStateView> PoolStateView for &T {
    fn get_note(&self, sn: &Hash) -> Option<PoolEntry> {
        (*self).get_note(sn)
    }
}

/// A bare `PoolCollection` is a valid base view (used by tests and any in-memory state).
impl PoolStateView for PoolCollection {
    fn get_note(&self, sn: &Hash) -> Option<PoolEntry> {
        self.get(sn).copied()
    }
}

pub trait PoolViewComposition: PoolStateView + Sized {
    fn compose<D: ImmutablePoolDiff>(self, diff: D) -> ComposedPoolView<Self, D>;
}

impl<T: PoolStateView> PoolViewComposition for T {
    fn compose<D: ImmutablePoolDiff>(self, diff: D) -> ComposedPoolView<Self, D> {
        ComposedPoolView::new(self, diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notepool::{DenominationTag, PoolDiff};

    fn note(byte: u8) -> PoolEntry {
        PoolEntry::unlocked(crate::notepool::NewNote { d: DenominationTag::D1, pk: [byte; 32] })
    }

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    #[test]
    fn composition_layers_add_remove_and_base() {
        let base: PoolCollection = [(hash(1), note(1)), (hash(2), note(2))].into();
        let diff = PoolDiff::new([(hash(3), note(3))].into(), [(hash(2), note(2))].into());
        let view = (&base).compose(&diff);

        assert_eq!(view.get_note(&hash(1)), Some(note(1))); // untouched base entry
        assert_eq!(view.get_note(&hash(2)), None); // removed by diff
        assert_eq!(view.get_note(&hash(3)), Some(note(3))); // added by diff
        assert_eq!(view.get_note(&hash(4)), None); // never existed
    }

    #[test]
    fn nested_composition() {
        let base: PoolCollection = [(hash(1), note(1))].into();
        let inner = PoolDiff::new([(hash(2), note(2))].into(), Default::default());
        let outer = PoolDiff::new(Default::default(), [(hash(2), note(2))].into());
        let view = (&base).compose(&inner).compose(&outer);

        assert_eq!(view.get_note(&hash(1)), Some(note(1)));
        assert_eq!(view.get_note(&hash(2)), None); // added by inner, removed by outer
    }
}
