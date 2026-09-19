use std::collections::HashMap;

use crate::mempool::{errors::RuleResult, model::tx::SerialConflict};
use kaspa_consensus_core::{
    Hash,
    notepool::PoolOp,
    subnets::SUBNETWORK_ID_NOTE_POOL,
    tx::{MutableTransaction, TransactionId},
};

/// Tracks which mempool transaction is consuming which note-pool serial — the serial-keyed
/// analog of `MempoolUtxoSet`'s `outpoint_owner_id` (PLAN P6.7). Deliberately simpler
/// than the outpoint side: no replace-by-fee variant (the plan specifies "first-seen holds,
/// second rejected" unconditionally for serial conflicts, unlike the outpoint side's
/// configurable RBF policy), and no tracking of *produced* notes — chaining an unconfirmed
/// pool op's produced notes into a second unconfirmed pool op (the way UTXO outputs chain
/// through `populate_mempool_entries`) is a documented, deliberate MVP scope limitation for
/// this step, not something this index needs to support. See NOTES.md's P6.7 entry.
#[derive(Default)]
pub(crate) struct MempoolPoolNoteSet {
    serial_owner_id: HashMap<Hash, TransactionId>,
}

impl MempoolPoolNoteSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// This transaction's consumed serials — empty for a non-pool-op tx, for a `Mint`
    /// (which consumes nothing), or on a malformed payload (body-in-isolation validation
    /// already rejects that before a transaction can reach the mempool).
    fn consumed_serials(transaction: &MutableTransaction) -> Vec<Hash> {
        if transaction.tx.subnetwork_id != SUBNETWORK_ID_NOTE_POOL {
            return Vec::new();
        }
        match PoolOp::decode_payload(&transaction.tx.payload) {
            Some(PoolOp::Transfer(op)) => op.consumed.iter().flat_map(|g| g.serials.iter().copied()).collect(),
            Some(PoolOp::TransferLocked(op)) => op.consumed.iter().flat_map(|g| g.serials.iter().copied()).collect(),
            Some(PoolOp::Redeem(op)) => op.consumed.iter().flat_map(|g| g.serials.iter().copied()).collect(),
            Some(PoolOp::Mint(_)) | None => Vec::new(),
        }
    }

    pub(crate) fn add_transaction(&mut self, transaction: &MutableTransaction) {
        let transaction_id = transaction.id();
        for serial in Self::consumed_serials(transaction) {
            self.serial_owner_id.insert(serial, transaction_id);
        }
    }

    pub(crate) fn remove_transaction(&mut self, transaction: &MutableTransaction) {
        for serial in Self::consumed_serials(transaction) {
            self.serial_owner_id.remove(&serial);
        }
    }

    pub(crate) fn get_serial_owner_id(&self, serial: &Hash) -> Option<&TransactionId> {
        self.serial_owner_id.get(serial)
    }

    /// Make sure no other transaction in the mempool is already consuming a serial this
    /// transaction also consumes.
    pub(crate) fn check_serial_conflicts(&self, transaction: &MutableTransaction) -> RuleResult<()> {
        match self.get_first_serial_conflict(transaction) {
            Some(conflict) => Err(conflict.into()),
            None => Ok(()),
        }
    }

    pub(crate) fn get_first_serial_conflict(&self, transaction: &MutableTransaction) -> Option<SerialConflict> {
        let transaction_id = transaction.id();
        for serial in Self::consumed_serials(transaction) {
            if let Some(existing_transaction_id) = self.get_serial_owner_id(&serial)
                && *existing_transaction_id != transaction_id
            {
                return Some(SerialConflict::new(serial, *existing_transaction_id));
            }
        }
        None
    }
}
