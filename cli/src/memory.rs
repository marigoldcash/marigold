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
/// What a full-scale node actually takes on a laptop: a tester's 16 GB Linux
/// machine held 6.3 GB in the wallet after its sync was stopped, more while
/// it ran (2026-09-22). The earlier guess of four was half the truth.
const NODE_AT_FULL_SCALE_GB: f64 = 8.0;

/// Below this much memory the wallet does not start a sync of its own by
/// itself: the tester's 4 GB machine was "fighting and not usable". 'connect'
/// still starts one on request.
pub const SMALL_MACHINE_GB: f64 = 6.0;

/// The node's cache scale for this machine: enough of the machine for the
/// sync to be quick, never enough to starve everything else. A third of the
/// memory, or what is free minus a margin for the wallet and the rest of
/// the machine, whichever is smaller; scale 1.0 is four gigabytes of that.
pub fn ram_scale_for(memory: Memory) -> f64 {
    let total = memory.total as f64 / GB;
    let available = memory.available as f64 / GB;
    // A quarter of the machine, and never what the machine does not have
    // free beyond two gigabytes for everything else.
    let budget = (total / 4.0).min(available - 2.0).max(0.5);
    (budget / NODE_AT_FULL_SCALE_GB).clamp(0.1, 1.0)
}

/// Whether this machine is too small for a sync of its own to start unasked.
pub fn too_small_for_own_sync(memory: Memory) -> bool {
    (memory.total as f64 / GB) < SMALL_MACHINE_GB
}

/// How many threads the node inside the wallet gets: half the cores on a
/// laptop (at least two), all but two on a bigger machine. A first sync
/// validates blocks on every core it is given, and on a four-core laptop
/// that was 60% of the machine (tester, 2026-09-22).
pub fn node_threads_for(cores: usize) -> usize {
    if cores <= 8 { (cores / 2).max(2).min(cores.max(1)) } else { cores - 2 }
}

pub fn node_threads() -> usize {
    node_threads_for(crate::miner::cores())
}

/// Put the whole process behind everything else the person is doing: the
/// sync is the one thing here that can wait. The miner's threads already
/// run at idle priority; this covers the node's.
#[cfg(not(target_arch = "wasm32"))]
pub fn lower_process_priority() {
    #[cfg(unix)]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass};
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}

/// Above this much resident memory, a node that has just finished its first
/// sync is restarted in place: the sync fills every cache to its budget and
/// glibc keeps the freed heap, so a tester saw six gigabytes held after
/// "sync complete" that a restart brought down to two hundred megabytes.
pub const RESTART_AFTER_SYNC_ABOVE: u64 = 3 * 1024 * 1024 * 1024 / 2;

/// This process's resident memory, in bytes.
#[cfg(target_os = "linux")]
pub fn process_rss() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
pub fn process_rss() -> Option<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let pid = Pid::from_u32(std::process::id());
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), ProcessRefreshKind::new().with_memory());
    system.process(pid).map(|p| p.memory())
}

#[cfg(target_arch = "wasm32")]
pub fn process_rss() -> Option<u64> {
    None
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
        assert!((ram_scale_for(mem(32.0, 28.0)) - 1.0).abs() < 1e-9, "thirty-two gigabytes is full");
        assert!((ram_scale_for(mem(16.0, 10.0)) - 0.5).abs() < 1e-9, "sixteen gigabytes gets half");
        assert!((ram_scale_for(mem(8.0, 5.0)) - 0.25).abs() < 1e-9, "eight gigabytes gets a quarter");
        assert!((ram_scale_for(mem(8.0, 2.0)) - 0.1).abs() < 1e-9, "a busy laptop gets the minimum");
        assert!((ram_scale_for(mem(4.0, 3.0)) - 0.125).abs() < 1e-9, "four gigabytes is near the floor");
        assert!(ram_scale_for(mem(2.0, 0.5)) >= 0.1, "never below the floor");
        assert!(too_small_for_own_sync(mem(4.0, 3.0)));
        assert!(!too_small_for_own_sync(mem(8.0, 1.0)));
        assert_eq!(node_threads_for(4), 2);
        assert_eq!(node_threads_for(2), 2);
        assert_eq!(node_threads_for(1), 1);
        assert_eq!(node_threads_for(8), 4);
        assert_eq!(node_threads_for(16), 14);
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
