//! CPU mining, for someone already running their own node.
//!
//! The node is in this process, so there is no miner to install, no stratum
//! server, no gRPC socket and no payout address to configure — the wallet
//! already holds an `RpcApi` pointing straight at its own node, and already
//! knows the address the coins should go to. What is left is a loop.
//!
//! WHY IT ONLY RUNS ON YOUR OWN NODE
//!
//! `get_block_template` works against anybody's node, but asking a stranger
//! for templates hands them the address your rewards are paid to — the one
//! piece of information the rest of this wallet works to keep off other
//! people's machines. It is also a real cost to whoever runs that node. So
//! mining is tied to running your own, which is the behaviour worth
//! encouraging anyway.
//!
//! WHY THE WORK IS NOT WASTED WHEN A TEMPLATE GOES SLIGHTLY STALE
//!
//! At ten blocks a second a template is out of date almost as soon as it
//! arrives. On a chain that would mean orphans; on a blockDAG a block built on
//! a tip a moment behind is still valid and is merged in. That is the whole
//! point of the structure. So refreshing on a timer is enough, and the loop
//! does not need to react to every new block — it only needs to not drift so
//! far back that the mergeset limit rejects it, which a sub-second refresh is
//! nowhere near.

use kaspa_consensus_core::block::Block;
use kaspa_consensus_core::header::Header;
use kaspa_pow::State;
use kaspa_rpc_core::model::RpcRawBlock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

/// How often to ask the node for fresh work.
pub const TEMPLATE_REFRESH_MS: u64 = 400;

/// Nonces hashed between checks of the stop flag and the job generation.
/// Small enough that abandoning stale work is prompt, large enough that the
/// atomic loads are lost in the noise of hashing.
const CHUNK: u64 = 512;

/// One unit of work: everything a thread needs to search, plus the block it
/// would submit if it found something.
struct Job {
    generation: u64,
    state: State,
    header: Header,
    transactions: Arc<Vec<kaspa_consensus_core::tx::Transaction>>,
}

/// A solved block on its way back to the async side.
pub struct Solution {
    generation: u64,
    nonce: u64,
}

pub struct Miner {
    stop: Arc<AtomicBool>,
    job: Arc<arc_swap::ArcSwapOption<Job>>,
    generation: Arc<AtomicU64>,
    hashes: Arc<AtomicU64>,
    found: Arc<AtomicU64>,
    accepted: Arc<AtomicU64>,
    rejected: Arc<AtomicU64>,
    threads: std::sync::Mutex<Vec<JoinHandle<()>>>,
    thread_count: usize,
    percent: u32,
    started: std::time::Instant,
}

/// Cores this machine has, as the denominator for the percentage.
pub fn cores() -> usize {
    num_cpus::get().max(1)
}

/// Turn "half the machine" into a number of threads.
///
/// People think about how much of their computer they are lending, not about
/// core counts — so the question asked is a percentage and this is the only
/// place that translates it. Always at least one thread: someone who asks for
/// 5% of a four-core laptop means "a little", not "none".
pub fn threads_for_percent(percent: u32) -> usize {
    let cores = cores();
    let scaled = (cores as f64 * percent as f64 / 100.0).round() as usize;
    scaled.clamp(1, cores)
}

/// Make this thread yield to anything else that wants the CPU.
///
/// `SCHED_IDLE` is the whole feature: a thread in that class runs only when
/// the machine would otherwise be idle, so mining costs nothing that anybody
/// notices and stops the instant real work arrives. The kernel does the
/// backing off — there is no polling, no load average to watch, and no way for
/// the wallet to get it wrong. `nice` alone would not do this: at nice 19 a
/// thread still takes a share of a busy CPU, just a small one.
#[cfg(target_os = "linux")]
fn deprioritise_current_thread() {
    unsafe {
        let param = libc::sched_param { sched_priority: 0 };
        // pid 0 means the calling thread.
        libc::sched_setscheduler(0, libc::SCHED_IDLE, &param);
        // Belt and braces for anything that ignores the policy.
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
}

/// macOS: the background quality-of-service class, which is the closest thing
/// to Linux's idle scheduling — the scheduler starves it whenever anything
/// user-facing wants the core. Per thread: `setpriority` here would nice the
/// whole process, wallet included, which is the wrong thread to slow down.
#[cfg(target_os = "macos")]
fn deprioritise_current_thread() {
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
    }
}

/// Windows: idle thread priority, the lowest the scheduler offers a thread.
#[cfg(windows)]
fn deprioritise_current_thread() {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_IDLE};
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE);
    }
}

/// Anything else gets the strongest thing it has, which is weaker.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn deprioritise_current_thread() {
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
}

impl Miner {
    /// Start hashing. The caller has already established that the wallet is on
    /// its own node and that the node is synced.
    pub fn start(percent: u32) -> (Arc<Self>, std::sync::mpsc::Receiver<Solution>) {
        let thread_count = threads_for_percent(percent);
        let (tx, rx) = std::sync::mpsc::channel::<Solution>();

        let miner = Arc::new(Self {
            stop: Arc::new(AtomicBool::new(false)),
            job: Arc::new(arc_swap::ArcSwapOption::empty()),
            generation: Arc::new(AtomicU64::new(0)),
            hashes: Arc::new(AtomicU64::new(0)),
            found: Arc::new(AtomicU64::new(0)),
            accepted: Arc::new(AtomicU64::new(0)),
            rejected: Arc::new(AtomicU64::new(0)),
            threads: std::sync::Mutex::new(Vec::new()),
            thread_count,
            percent,
            started: std::time::Instant::now(),
        });

        let mut handles = Vec::with_capacity(thread_count);
        for index in 0..thread_count {
            let stop = miner.stop.clone();
            let job = miner.job.clone();
            let generation = miner.generation.clone();
            let hashes = miner.hashes.clone();
            let found = miner.found.clone();
            let tx = tx.clone();
            handles.push(
                std::thread::Builder::new()
                    .name(format!("marigold-mine-{index}"))
                    .spawn(move || {
                        deprioritise_current_thread();
                        // Each thread starts somewhere different in the nonce
                        // space, so N threads do N times the work rather than
                        // the same work N times.
                        let mut nonce: u64 = rand::random();
                        while !stop.load(Ordering::Relaxed) {
                            let Some(current) = job.load_full() else {
                                std::thread::sleep(std::time::Duration::from_millis(50));
                                continue;
                            };
                            if current.generation != generation.load(Ordering::Relaxed) {
                                continue;
                            }
                            for _ in 0..CHUNK {
                                nonce = nonce.wrapping_add(1);
                                if current.state.check_pow(nonce).0 {
                                    found.fetch_add(1, Ordering::Relaxed);
                                    // A closed channel means the miner is
                                    // shutting down; nothing to report to.
                                    let _ = tx.send(Solution { generation: current.generation, nonce });
                                    break;
                                }
                            }
                            hashes.fetch_add(CHUNK, Ordering::Relaxed);
                        }
                    })
                    .expect("mining thread"),
            );
        }
        *miner.threads.lock().unwrap() = handles;
        (miner, rx)
    }

    /// Hand the threads a fresh template.
    pub fn set_job(&self, block: Block) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let header = (*block.header).clone();
        let state = State::new(&header);
        self.job.store(Some(Arc::new(Job { generation, state, header, transactions: block.transactions.clone() })));
    }

    /// Rebuild the block a solution belongs to, with the winning nonce in it.
    ///
    /// Returns None when the job has moved on — the thread found something for
    /// work we have already replaced, which is harmless and simply discarded.
    pub fn block_for(&self, solution: &Solution) -> Option<RpcRawBlock> {
        let job = self.job.load_full()?;
        if job.generation != solution.generation {
            return None;
        }
        let mut header = job.header.clone();
        header.nonce = solution.nonce;
        // The header hash is derived from its fields, so it has to be
        // recomputed once the nonce changes or the node rejects the block.
        header.finalize();
        let block = Block::from_arcs(Arc::new(header), job.transactions.clone());
        Some((&block).into())
    }

    pub fn record_accepted(&self) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rejected(&self) {
        self.rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let handles = std::mem::take(&mut *self.threads.lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
    }

    pub fn is_running(&self) -> bool {
        !self.stop.load(Ordering::Relaxed)
    }

    pub fn thread_count(&self) -> usize {
        self.thread_count
    }

    pub fn percent(&self) -> u32 {
        self.percent
    }

    pub fn blocks_found(&self) -> u64 {
        self.found.load(Ordering::Relaxed)
    }

    pub fn blocks_accepted(&self) -> u64 {
        self.accepted.load(Ordering::Relaxed)
    }

    pub fn blocks_rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    /// Average hashes per second since starting.
    pub fn hashrate(&self) -> f64 {
        let seconds = self.started.elapsed().as_secs_f64();
        if seconds <= 0.0 { 0.0 } else { self.hashes.load(Ordering::Relaxed) as f64 / seconds }
    }
}

/// Human-readable hash rate. A CPU on this chain is in the kilohash range, so
/// "0.00 MH/s" — which is what the stratum bridge shows — tells nobody
/// anything.
pub fn format_hashrate(rate: f64) -> String {
    if rate >= 1_000_000.0 {
        format!("{:.2} MH/s", rate / 1_000_000.0)
    } else if rate >= 1_000.0 {
        format!("{:.1} kH/s", rate / 1_000.0)
    } else {
        format!("{rate:.0} H/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_consensus_core::header::CompressedParents;
    use kaspa_consensus_core::{BlueWorkType, Hash, blockhash::ORIGIN};

    /// A header with a target so easy that a nonce is found in a few tries.
    fn easy_header() -> Header {
        Header::new_finalized(
            1,
            CompressedParents::try_from(vec![vec![ORIGIN]]).unwrap(),
            Hash::from_bytes([1; 32]),
            Hash::from_bytes([2; 32]),
            Hash::from_bytes([3; 32]),
            Hash::from_bytes([4; 32]),
            1_700_000_000,
            // A very high compact target: almost any hash clears it.
            0x2100_ffff,
            0,
            42,
            BlueWorkType::from_u64(1),
            7,
            Hash::from_bytes([5; 32]),
        )
    }

    #[test]
    fn percentage_maps_to_at_least_one_thread_and_never_more_than_the_machine() {
        let cores = cores();
        assert_eq!(threads_for_percent(100), cores);
        assert!(threads_for_percent(1) >= 1, "1% must still mine with one thread");
        assert!(threads_for_percent(50) <= cores);
        // 50% of anything is never more than the whole machine, and never zero.
        for percent in 1..=100 {
            let n = threads_for_percent(percent);
            assert!((1..=cores).contains(&n), "{percent}% gave {n} threads on {cores} cores");
        }
    }

    /// The invariant the whole miner rests on.
    ///
    /// A thread solves against a `State` built from the template's header, then
    /// the nonce is written into a *copy* of that header and the block is
    /// rebuilt and finalized before submission. If finalizing disturbed
    /// anything the proof-of-work reads, the node would reject every block we
    /// ever found — and it would look like bad luck rather than a bug.
    #[test]
    fn a_rebuilt_block_still_satisfies_the_proof_we_solved() {
        let header = easy_header();
        let state = State::new(&header);

        // From 1, not 0: the header already carries nonce 0, and a solution
        // there would make the hash-changed assertion below vacuous.
        let mut nonce = 1u64;
        let solved = loop {
            if state.check_pow(nonce).0 {
                break nonce;
            }
            nonce += 1;
            assert!(nonce < 500_000, "target was meant to be trivially easy");
        };

        // Exactly what block_for does.
        let mut rebuilt = header.clone();
        rebuilt.nonce = solved;
        rebuilt.finalize();

        assert!(State::new(&rebuilt).check_pow(solved).0, "the block we would submit no longer satisfies its own proof of work");
        assert_eq!(rebuilt.nonce, solved);
        // Finalizing must change the block's identity, or we would be
        // submitting a block claiming a hash it does not have.
        assert_ne!(rebuilt.hash, header.hash, "the header hash must follow the nonce");
    }

    #[test]
    fn hashrate_reads_in_units_a_person_can_use() {
        assert_eq!(format_hashrate(12.0), "12 H/s");
        assert_eq!(format_hashrate(4_500.0), "4.5 kH/s");
        assert_eq!(format_hashrate(2_500_000.0), "2.50 MH/s");
    }
}
