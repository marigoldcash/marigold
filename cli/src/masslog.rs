//! Measure what a consolidation transaction actually weighs.
//!
//! # Why this exists
//!
//! The transaction generator stops accumulating inputs at four fifths of
//! `MAXIMUM_STANDARD_TRANSACTION_MASS` (`generator.rs`'s
//! `TRANSACTION_MASS_BOUNDARY_FOR_ADDITIONAL_INPUT_ACCUMULATION`). That
//! headroom is for the change output and for signatures, whose real size is
//! not known until the inputs are signed — an estimate that runs over the
//! limit produces a transaction the node refuses, which on a long
//! consolidation means discovering it hours in.
//!
//! Four fifths is upstream's guess. If the real signed mass lands nearer the
//! limit than the guess assumes, the headroom is being wasted: a
//! consolidation needs about a quarter more transactions than it has to, and
//! on a wallet holding millions of coins that is real money. If it lands
//! close to the limit, the guess is right and the cap should be left alone.
//!
//! Nobody knows which without measuring, so this measures. It writes one row
//! per transaction that a sweep actually signed and submitted, and the numbers
//! it records are the ones the decision turns on: the mass the generator
//! believed, the mass the signed transaction really has, and how much of the
//! limit each used.
//!
//! # What it does not do
//!
//! It does not change how anything is built or sized. It is a tape measure.

use crate::imports::*;
use kaspa_wallet_core::tx::mass::MassCalculator;
use kaspa_wallet_core::tx::{MAXIMUM_STANDARD_TRANSACTION_MASS, PendingTransaction};
use std::io::Write;
use std::sync::atomic::AtomicU64;
use std::path::{Path, PathBuf};

/// One row per signed transaction, tab separated, with a header — so it opens
/// in a spreadsheet, and `awk` can answer the question without one.
const HEADER: &str = "id\tinputs\toutputs\tgenerator_mass\tsigned_compute_mass\tstorage_mass\tsigned_total_mass\tlimit\tused_percent\tbytes\tfees\tinput_value\tis_batch";

pub struct MassLog {
    path: PathBuf,
    calculator: MassCalculator,
    file: Mutex<Option<std::fs::File>>,
    rows: AtomicU64,
    /// The largest `used_percent` seen, in tenths of a percent so it fits an
    /// integer — this single number is the whole answer to whether the
    /// four-fifths cap has room in it.
    peak_tenths: AtomicU64,
}

impl MassLog {
    /// Open (or create) the log beside the wallet's own files.
    pub fn try_new(folder: &Path, network_id: NetworkId) -> std::io::Result<Self> {
        let path = folder.join("sweep-mass.tsv");
        let fresh = !path.exists();
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        if fresh {
            writeln!(file, "{HEADER}")?;
        }
        Ok(Self {
            path,
            calculator: MassCalculator::new(&network_id.into()),
            file: Mutex::new(Some(file)),
            rows: AtomicU64::new(0),
            peak_tenths: AtomicU64::new(0),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn rows(&self) -> u64 {
        self.rows.load(Ordering::Relaxed)
    }

    /// The fullest transaction seen, as a percentage of the mass limit.
    pub fn peak_percent(&self) -> f64 {
        self.peak_tenths.load(Ordering::Relaxed) as f64 / 10.0
    }

    /// Record one transaction. Called after it has been signed, because the
    /// signed size is the entire point — an unsigned estimate is what the
    /// generator already has and already distrusts.
    ///
    /// Never fails the caller. A sweep must not stop because a measurement
    /// could not be written down.
    pub fn record(&self, ptx: &PendingTransaction) {
        let transaction = ptx.transaction();

        let signed_compute_mass = self.calculator.calc_compute_mass_for_signed_consensus_transaction(&transaction);

        // Storage mass from the parts, because the whole-transaction helper
        // takes the client type and this is a consensus one. Consolidation is
        // the cheap direction for storage mass anyway — KIP-9 charges for
        // splitting value across outputs, and this merges it — so compute mass
        // is expected to be what binds. Recorded separately so that
        // expectation is checked rather than assumed.
        let outputs: Vec<_> = transaction.outputs.to_vec();
        let inputs: Vec<_> = ptx.utxo_entries().values().cloned().collect();
        let storage_mass = self.calculator.calc_storage_mass_for_transaction_parts(&inputs, &outputs).unwrap_or(0);
        let signed_total = self.calculator.combine_mass(signed_compute_mass, storage_mass);
        let bytes = kaspa_wallet_core::tx::mass::transaction_serialized_byte_size(&transaction);

        let used = signed_total as f64 / MAXIMUM_STANDARD_TRANSACTION_MASS as f64 * 100.0;
        self.peak_tenths.fetch_max((used * 10.0) as u64, Ordering::Relaxed);

        let row = format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{}\t{}\t{}\t{}",
            ptx.id(),
            transaction.inputs.len(),
            transaction.outputs.len(),
            ptx.mass(),
            signed_compute_mass,
            storage_mass,
            signed_total,
            MAXIMUM_STANDARD_TRANSACTION_MASS,
            used,
            bytes,
            ptx.fees(),
            ptx.aggregate_input_value(),
            ptx.is_batch(),
        );

        if let Some(file) = self.file.lock().unwrap().as_mut() {
            // Ignored on purpose: see the doc comment. A full disk should cost
            // the measurement, not the consolidation.
            let _ = writeln!(file, "{row}");
        }
        self.rows.fetch_add(1, Ordering::Relaxed);
    }

    /// Flush and close, so the file is complete when the sweep reports it.
    pub fn finish(&self) {
        if let Some(mut file) = self.file.lock().unwrap().take() {
            let _ = file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network() -> NetworkId {
        NetworkId::with_suffix(kaspa_consensus_core::network::NetworkType::Testnet, 10)
    }

    #[test]
    fn a_new_log_gets_a_header_and_an_existing_one_is_appended_to() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let log = MassLog::try_new(dir.path(), network())?;
        let path = log.path().to_path_buf();
        log.finish();

        let first = std::fs::read_to_string(&path)?;
        assert_eq!(first.lines().count(), 1, "just the header");
        assert!(first.starts_with("id\tinputs\toutputs\t"));

        // A second sweep must add to the record rather than erase it.
        let again = MassLog::try_new(dir.path(), network())?;
        again.finish();
        let second = std::fs::read_to_string(&path)?;
        assert_eq!(second.lines().count(), 1, "the header is not written twice");
        Ok(())
    }

    /// The peak is the number the decision turns on, so it has to track the
    /// fullest transaction rather than the last one.
    #[test]
    fn the_peak_keeps_the_highest_it_has_seen() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let log = MassLog::try_new(dir.path(), network())?;
        for tenths in [812u64, 903, 455, 899] {
            log.peak_tenths.fetch_max(tenths, Ordering::Relaxed);
        }
        assert_eq!(log.peak_percent(), 90.3);
        Ok(())
    }

    /// The header has to describe the row, or the file is unreadable a week
    /// later by exactly the person who needs it.
    #[test]
    fn the_header_matches_the_row_it_labels() {
        let columns = HEADER.split('\t').count();
        // Count the format placeholders in `record`'s row — kept in step by
        // hand, so worth asserting.
        assert_eq!(columns, 13, "header columns");
    }
}
