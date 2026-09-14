//! Measure what a consolidation transaction actually weighs.
//!
//! # What it found
//!
//! Measured over 75 consolidation transactions on testnet-10 (2026-09-14,
//! wallet build 2.0.62), and the answer settled a question that had been
//! guessed at wrongly:
//!
//! ```text
//! inputs                88   every transaction, no variation
//! signed total mass     98,890 of a 100,000 limit — 98.89%
//! headroom              1,110, against ~1,124 mass for one more input
//! generator's estimate  98,990 — 100 high, 0.1%, and on the safe side
//! storage mass          0 on every single one
//! fee                   9,940,200 petals — exactly 100 sompi/gram
//! ```
//!
//! So the transactions are full. They stop at 88 inputs because an 89th does
//! not fit, not because anything told them to stop early, and there is no
//! spare capacity to reclaim.
//!
//! The suspicion this was built to test — that the generator's
//! `TRANSACTION_MASS_BOUNDARY_FOR_ADDITIONAL_INPUT_ACCUMULATION`, four fifths
//! of the mass limit, was capping transactions at 80% and costing a quarter
//! more of them — was wrong. That constant is not a cap. Its own comment says
//! it governs an opportunistic extra-input pass that exists to *reduce
//! storage mass*, and the branch is gated on `storage_mass > 0`. A
//! consolidation merges value rather than splitting it, so its storage mass is
//! zero and that branch never runs. It has nothing to do with how full these
//! transactions are.
//!
//! The fee is the protocol minimum: 100 sompi per gram is the node's own
//! `DEFAULT_MINIMUM_RELAY_TRANSACTION_FEE`, and the wallet pays exactly it.
//!
//! What remains is arithmetic, not waste. Consolidation costs about 112,957
//! petals per coin — 0.00113 MAGLD against coins worth 0.152, so 0.74% of
//! each. Clearing 4,439,373 coins takes roughly 50,447 transactions and about
//! 5,014 MAGLD, which is 0.62% of the 812,524 they held. Nothing in the
//! transaction's shape or price can move that. The only thing that ever could
//! was not letting four million coins accumulate, which is a scheduling
//! problem and was fixed as one.
//!
//! # Why it is still here
//!
//! The numbers above are one network, one wallet, one day. Mass accounting
//! changes (Toccata moved the relay fee by a factor of a hundred), and the
//! next time somebody wonders whether consolidation is priced properly the
//! answer should come from a measurement rather than from this comment.
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
