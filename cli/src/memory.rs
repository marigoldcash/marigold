//! How much memory this machine has, and whether the wallet is about to
//! take too much of it.
//!
//! The wallet's own copy of the network is a full node with caches sized
//! for a server: about a gigabyte nominal at scale 1.0, several times that
//! in practice, and doubled during the first sync, when a second consensus
//! instance is staged beside the first. On a laptop that took the whole
//! machine down twice in two days — a macOS tester's spinning beach ball, a
//! Linux tester's out-of-memory killer (2026-09-22). Two answers here: size
//! the node to the machine before it starts, and watch free memory while it
//! runs, so the wallet backs off before the operating system has to.
//!
//! Like the disk checks, everything degrades to silence: a platform that
//! cannot be asked runs as before.

/// Bytes this machine has, and bytes nobody is using right now.
#[derive(Clone, Copy, Debug)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

#[cfg(not(target_arch = "wasm32"))]
pub fn read() -> Option<Memory> {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let system = System::new_with_specifics(RefreshKind::new().with_memory(MemoryRefreshKind::everything()));
    let total = system.total_memory();
    if total == 0 {
        return None;
    }
    Some(Memory { total, available: system.available_memory() })
}

#[cfg(target_arch = "wasm32")]
pub fn read() -> Option<Memory> {
    None
}

/// What a node at scale 1.0 takes during its first sync, in practice.
const NODE_AT_FULL_SCALE_GB: f64 = 4.0;

/// The node's cache scale for this machine: enough of the machine for the
/// sync to be quick, never enough to starve everything else. A third of the
/// memory, or what is free minus a margin for the wallet and the rest of
/// the machine, whichever is smaller; scale 1.0 is four gigabytes of that.
pub fn ram_scale_for(memory: Memory) -> f64 {
    let total = memory.total as f64 / GB;
    let available = memory.available as f64 / GB;
    let budget = (total / 3.0).min(available - 1.5).max(0.4);
    (budget / NODE_AT_FULL_SCALE_GB).clamp(0.1, 1.0)
}

/// The scale for the machine this runs on; a full node's when it cannot be
/// measured.
pub fn ram_scale() -> f64 {
    read().map(ram_scale_for).unwrap_or(1.0)
}

/// "8.0 GB" — the way people say it.
pub fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GB)
}

/// Free memory below this is low: the miner stops and the person is told.
pub fn low_watermark(memory: Memory) -> u64 {
    (memory.total / 25).max(512 * 1024 * 1024)
}

/// Free memory below this is critical: the sync is stopped so the machine
/// stays usable — a clean stop, where the alternative is the system killing
/// whatever it likes.
pub fn critical_watermark(memory: Memory) -> u64 {
    (memory.total / 50).max(256 * 1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(total_gb: f64, available_gb: f64) -> Memory {
        Memory { total: (total_gb * GB) as u64, available: (available_gb * GB) as u64 }
    }

    #[test]
    fn the_scale_follows_the_machine() {
        assert!((ram_scale_for(mem(64.0, 50.0)) - 1.0).abs() < 1e-9, "a server gets a full node");
        assert!((ram_scale_for(mem(16.0, 10.0)) - 1.0).abs() < 0.35, "sixteen gigabytes is roughly full");
        let laptop = ram_scale_for(mem(8.0, 5.0));
        assert!(laptop > 0.5 && laptop < 0.75, "eight gigabytes gets about two thirds: {laptop}");
        let busy_laptop = ram_scale_for(mem(8.0, 2.0));
        assert!(busy_laptop <= 0.15, "a busy laptop gets the minimum: {busy_laptop}");
        assert!((ram_scale_for(mem(4.0, 3.0)) - 0.1).abs() < 0.3, "four gigabytes is near the floor");
        assert!(ram_scale_for(mem(2.0, 0.5)) >= 0.1, "never below the floor");
    }

    #[test]
    fn watermarks_scale_with_the_machine_but_not_below_a_floor() {
        assert_eq!(low_watermark(mem(4.0, 1.0)), 512 * 1024 * 1024);
        assert_eq!(low_watermark(mem(64.0, 1.0)), (64.0 * GB) as u64 / 25);
        assert_eq!(critical_watermark(mem(4.0, 1.0)), 256 * 1024 * 1024);
        assert!(critical_watermark(mem(64.0, 1.0)) < low_watermark(mem(64.0, 1.0)));
        assert_eq!(gigabytes((8.0 * GB) as u64), "8.0 GB");
    }
}
