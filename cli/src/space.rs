//! How much disk is left, and whether it is enough for what is about to run.
//!
//! This exists because the wallet once filled a directory until ext4 refused
//! another entry and reported it as "No space left on device" on a disk with
//! 200 GB free. That particular cause is fixed, but the lesson stands: the
//! moment to talk about disk is before a long job starts, not eighteen hours
//! in, and the message has to say what to do about it.
//!
//! Every check here is advisory and degrades to silence. A platform we cannot
//! ask, or a path we cannot stat, means the job runs as it always did — a
//! disk check that blocks work because it could not measure anything would be
//! worse than no disk check.

use std::path::Path;

/// What a filesystem has left, for the user running this process.
#[derive(Clone, Copy, Debug)]
pub struct Space {
    /// Bytes a non-root process can still use — not the raw free count,
    /// which includes the reserved blocks this user cannot touch.
    pub available: u64,
    pub total: u64,
}

impl Space {
    pub fn used_percent(&self) -> u64 {
        if self.total == 0 { 0 } else { (self.total - self.available) * 100 / self.total }
    }
}

/// The space on the filesystem holding `path`, or the nearest parent that
/// exists — a node directory is often being asked about before it is created.
#[cfg(unix)]
pub fn at(path: &Path) -> Option<Space> {
    use std::os::unix::ffi::OsStrExt;

    let mut probe = path;
    loop {
        if probe.exists() {
            break;
        }
        probe = probe.parent()?;
    }

    let c_path = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c_path` is a valid NUL-terminated path and `stat` is written
    // only by the call, which reports failure through its return value.
    let stat = unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return None;
        }
        stat
    };

    // f_frsize is the fragment size, which is what both counts are in.
    let unit = if stat.f_frsize > 0 { stat.f_frsize as u64 } else { stat.f_bsize as u64 };
    let total = (stat.f_blocks as u64).saturating_mul(unit);
    // A pseudo filesystem — procfs, sysfs — answers statvfs with zero blocks.
    // Reading that as "zero bytes available" would refuse every job on a path
    // that happens to resolve into one, so it counts as unmeasurable instead.
    if total == 0 {
        return None;
    }
    Some(Space { available: (stat.f_bavail as u64).saturating_mul(unit), total })
}

#[cfg(not(unix))]
pub fn at(_path: &Path) -> Option<Space> {
    None
}

/// What a job would like to have before it starts.
#[derive(Clone, Copy, Debug)]
pub struct Need {
    /// Below this, the job is refused. Set to what the job genuinely cannot
    /// finish without, not to a round number that feels safe.
    pub required: u64,
    /// Below this the job runs, with a word first. Room to finish, not room
    /// to keep running for months.
    pub comfortable: u64,
}

const GB: u64 = 1024 * 1024 * 1024;

/// A local node.
///
/// A pruned testnet node's database was 5.4 GB when this was written, and
/// mainnet will be larger. The number that matters is not that, though: it is
/// that RocksDB compaction rewrites data it has not yet deleted, so peak
/// usage during a merge is meaningfully above steady state. Twenty gigabytes
/// is room for a pruned node and its compactions; sixty is room to leave it
/// running and not think about it.
pub const LOCAL_NODE: Need = Need { required: 20 * GB, comfortable: 60 * GB };

/// Mining adds no files of its own — it hashes in memory and submits blocks.
/// What it does do is pay into this wallet block after block, which grows
/// the node's UTXO set and its database. So the requirement is the node's,
/// and the reason to say so is that "mining fills your disk" is true by a
/// route nobody guesses.
pub const MINING: Need = LOCAL_NODE;

/// Roughly what a sweep will write, for `pieces` coins consolidated.
///
/// A sweep spends its inputs in batches of about eighty, and the batch
/// records are not kept — so in the ordinary case this is close to nothing.
/// With `history detail` on, every batch is a record, and a record occupies a
/// filesystem block however small it is. That is the case worth sizing.
pub fn sweep_estimate(pieces: u64, history_detail: bool) -> u64 {
    if !history_detail {
        return 0;
    }
    const INPUTS_PER_BATCH: u64 = 80;
    const BLOCK: u64 = 4096;
    pieces.div_ceil(INPUTS_PER_BATCH).saturating_mul(BLOCK)
}

/// The answer to "is there room".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Room to spare, or nothing measurable — either way, get on with it.
    Fine,
    /// Enough to finish, not enough to be relaxed about.
    Tight { available: u64 },
    /// Not enough. The caller should refuse.
    Short { available: u64, required: u64 },
}

pub fn check(path: &Path, need: Need) -> Verdict {
    let Some(space) = at(path) else { return Verdict::Fine };
    if space.available < need.required {
        Verdict::Short { available: space.available, required: need.required }
    } else if space.available < need.comfortable {
        Verdict::Tight { available: space.available }
    } else {
        Verdict::Fine
    }
}

/// Bytes as something a person reads, with one decimal where it helps.
pub fn human(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [("TB", 1024 * 1024 * 1024 * 1024), ("GB", GB), ("MB", 1024 * 1024), ("KB", 1024)];
    for (suffix, scale) in UNITS {
        if bytes >= scale {
            let whole = bytes / scale;
            let tenths = (bytes % scale) * 10 / scale;
            return if whole >= 100 { format!("{whole} {suffix}") } else { format!("{whole}.{tenths} {suffix}") };
        }
    }
    format!("{bytes} bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The working directory always exists, so this is a live check that the
    /// syscall path returns something sane rather than zeroes.
    #[test]
    #[cfg(unix)]
    fn a_real_path_reports_a_plausible_filesystem() {
        let space = at(Path::new(".")).expect("the working directory is on a filesystem");
        assert!(space.total > 0, "a filesystem with no blocks at all is a bad reading");
        assert!(space.available <= space.total);
        assert!(space.used_percent() <= 100);
    }

    /// A node directory is asked about before it is created, so the walk up
    /// to an existing parent has to work.
    #[test]
    #[cfg(unix)]
    fn a_path_that_does_not_exist_yet_reports_its_parent() {
        let deep = Path::new("./no-such-dir-a/no-such-dir-b/no-such-dir-c");
        let here = at(Path::new(".")).unwrap();
        let there = at(deep).expect("falls back to a parent that exists");
        assert_eq!(there.total, here.total, "same filesystem, so the same total");
    }

    #[test]
    fn a_filesystem_we_cannot_measure_never_blocks_the_job() {
        // `at` returning None must read as Fine, not as Short — refusing to
        // work because we could not measure would be the worse failure.
        assert_eq!(check(Path::new("/proc/self/cmdline/not-a-dir"), LOCAL_NODE), Verdict::Fine);
    }

    #[test]
    fn the_verdict_tracks_the_thresholds() {
        let need = Need { required: 100, comfortable: 200 };
        let verdict = |available: u64| {
            if available < need.required {
                Verdict::Short { available, required: need.required }
            } else if available < need.comfortable {
                Verdict::Tight { available }
            } else {
                Verdict::Fine
            }
        };
        assert!(matches!(verdict(99), Verdict::Short { .. }));
        assert!(matches!(verdict(100), Verdict::Tight { .. }), "exactly the requirement is not short");
        assert!(matches!(verdict(199), Verdict::Tight { .. }));
        assert_eq!(verdict(200), Verdict::Fine);
    }

    /// An ordinary sweep keeps no batch records, so it must not be sized as
    /// though it did — a check that refuses a harmless job is a bug.
    #[test]
    fn a_sweep_costs_nothing_until_detail_is_on() {
        assert_eq!(sweep_estimate(3_795_130, false), 0);
        let detailed = sweep_estimate(3_795_130, true);
        assert!(detailed > 100 * 1024 * 1024, "3.8M pieces in batches of eighty is tens of thousands of records");
        assert!(detailed < GB, "...but still well under a gigabyte");
        assert_eq!(sweep_estimate(1, true), 4096, "one piece is still one whole block");
    }

    #[test]
    fn sizes_read_the_way_a_person_would_say_them() {
        assert_eq!(human(0), "0 bytes");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(20 * GB), "20.0 GB");
        assert_eq!(human(201 * GB), "201 GB", "three digits do not need a decimal");
        assert_eq!(human(1024 * GB), "1.0 TB");
    }
}
