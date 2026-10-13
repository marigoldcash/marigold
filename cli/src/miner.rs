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

use kaspa_addresses::Address;
use kaspa_consensus_core::block::Block;
use kaspa_consensus_core::header::Header;
use kaspa_hashes::Hash;
use kaspa_pow::State;
use kaspa_rpc_core::api::miner::MinerControl;
use kaspa_rpc_core::model::RpcRawBlock;
use kaspa_rpc_core::{RpcError, RpcMinerStatus, RpcResult};
use kaspa_wallet_core::rpc::DynRpcApi;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use workflow_log::{log_info, log_warn};

/// How often to ask the node for fresh work.
pub const TEMPLATE_REFRESH_MS: u64 = 400;

/// Nonces hashed between checks of the stop flag and the job generation.
/// Small enough that abandoning stale work is prompt, large enough that the
/// atomic loads are lost in the noise of hashing.
const CHUNK: u64 = 512;

/// What a solution turns into once a thread finds one.
enum Work {
    /// A template from our own node: the solution is a block to submit.
    Block { header: Box<Header>, transactions: Arc<Vec<kaspa_consensus_core::tx::Transaction>> },
    /// A job from a pool: the solution is a share, matched to its job id by
    /// generation in `recent_share_jobs`.
    Share,
}

/// One unit of work: everything a thread needs to search, plus what it would
/// hand back if it found something.
///
/// A pool reserves the top bytes of the nonce for each worker (the stratum
/// "extranonce"), so two workers never search the same nonces. `nonce_mask`
/// and `nonce_prefix` express that: a thread's next nonce is
/// `(nonce + 1) & mask | prefix`. Solo work has no reservation, so the mask
/// is all ones and the prefix zero.
struct Job {
    generation: u64,
    state: State,
    work: Work,
    nonce_prefix: u64,
    nonce_mask: u64,
}

/// The nonce space a stratum extranonce leaves to the worker: the prefix the
/// top bytes must carry and the mask of the bytes that are the worker's own.
/// Up to seven bytes of extranonce; the bridge hands out two.
pub fn nonce_space_for_extranonce(extranonce_hex: &str) -> Option<(u64, u64)> {
    let hex = extranonce_hex.trim().trim_start_matches("0x");
    if hex.is_empty() {
        return Some((0, u64::MAX));
    }
    if !hex.len().is_multiple_of(2) || hex.len() > 14 {
        return None;
    }
    let value = u64::from_str_radix(hex, 16).ok()?;
    let own_bits = 64 - 4 * hex.len() as u32;
    Some((value << own_bits, (1u64 << own_bits) - 1))
}

/// How the pool is treating this miner: shown by 'mine status', kept by the
/// stratum session, readable from anywhere the miner is.
pub struct PoolStats {
    pub pool: String,
    pub worker: String,
    connected: AtomicBool,
    difficulty_bits: AtomicU64,
    accepted: AtomicU64,
    rejected: AtomicU64,
    last_rejection: std::sync::Mutex<Option<String>>,
    recent_accepts: std::sync::Mutex<std::collections::VecDeque<std::time::Instant>>,
}

impl PoolStats {
    pub fn new(pool: String, worker: String) -> Self {
        Self {
            pool,
            worker,
            connected: AtomicBool::new(false),
            difficulty_bits: AtomicU64::new(0),
            accepted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            last_rejection: std::sync::Mutex::new(None),
            recent_accepts: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Relaxed);
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn set_difficulty(&self, difficulty: f64) {
        self.difficulty_bits.store(difficulty.to_bits(), Ordering::Relaxed);
    }

    /// The share difficulty the pool has set, zero before it has said.
    pub fn difficulty(&self) -> f64 {
        f64::from_bits(self.difficulty_bits.load(Ordering::Relaxed))
    }

    pub fn record_accepted(&self) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
        let now = std::time::Instant::now();
        let mut recent = self.recent_accepts.lock().unwrap();
        recent.push_back(now);
        while recent.front().is_some_and(|t| now.duration_since(*t).as_secs() >= 60) {
            recent.pop_front();
        }
    }

    pub fn record_rejected(&self, reason: &str) {
        self.rejected.fetch_add(1, Ordering::Relaxed);
        *self.last_rejection.lock().unwrap() = Some(reason.to_string());
    }

    pub fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::Relaxed)
    }

    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    pub fn last_rejection(&self) -> Option<String> {
        self.last_rejection.lock().unwrap().clone()
    }

    /// Shares the pool took in the last minute.
    pub fn accepted_last_minute(&self) -> usize {
        let now = std::time::Instant::now();
        let mut recent = self.recent_accepts.lock().unwrap();
        while recent.front().is_some_and(|t| now.duration_since(*t).as_secs() >= 60) {
            recent.pop_front();
        }
        recent.len()
    }
}

/// This machine's name as the pool will show it: letters, digits, dashes and
/// underscores only, at most 32 of them, "cpu" when the system has no name.
pub fn worker_name() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    let raw = sysinfo::System::host_name().unwrap_or_default();
    #[cfg(target_arch = "wasm32")]
    let raw = String::new();
    let cleaned: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(32).collect();
    if cleaned.is_empty() { "cpu".to_string() } else { cleaned }
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
    /// The node's words for the last block it refused, for 'mine status'.
    last_rejection: std::sync::Mutex<Option<String>>,
    threads: std::sync::Mutex<Vec<JoinHandle<()>>>,
    thread_count: usize,
    percent: u32,
    started: std::time::Instant,
    /// Pool jobs a share may still belong to, newest last: a share found a
    /// moment after the job moved on is still worth sending, and the bridge
    /// keeps hundreds of old jobs around for exactly that.
    recent_share_jobs: std::sync::Mutex<std::collections::VecDeque<(u64, String)>>,
    /// Set when the miner works for a pool rather than its own node.
    pool: std::sync::OnceLock<Arc<PoolStats>>,
}

/// How many past pool jobs a share is still matched against.
const REMEMBERED_SHARE_JOBS: usize = 16;

/// Cores this machine has, as the denominator for the percentage.
/// The anchor drill's override of the synced-node gate; see its use.
pub(crate) fn mine_unsynced() -> bool {
    std::env::var("MARIGOLD_MINE_UNSYNCED").is_ok_and(|v| !v.is_empty() && v != "0")
}

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
        // Zeroed rather than a struct literal: musl's `sched_param` carries
        // sporadic-server fields that glibc's does not, and zero is right for
        // every one of them under SCHED_IDLE.
        let mut param: libc::sched_param = std::mem::zeroed();
        param.sched_priority = 0;
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

/// Any other Unix gets the strongest thing it has, which is weaker.
#[cfg(all(unix, not(target_os = "linux"), not(target_os = "macos")))]
fn deprioritise_current_thread() {
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
}

/// Anywhere else, the browser included, there is no thread to slow down.
#[cfg(not(any(unix, windows)))]
fn deprioritise_current_thread() {}

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
            last_rejection: std::sync::Mutex::new(None),
            threads: std::sync::Mutex::new(Vec::new()),
            thread_count,
            percent,
            started: std::time::Instant::now(),
            recent_share_jobs: std::sync::Mutex::new(std::collections::VecDeque::new()),
            pool: std::sync::OnceLock::new(),
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
                                nonce = (nonce.wrapping_add(1) & current.nonce_mask) | current.nonce_prefix;
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
        let work = Work::Block { header: Box::new(header), transactions: block.transactions.clone() };
        self.job.store(Some(Arc::new(Job { generation, state, work, nonce_prefix: 0, nonce_mask: u64::MAX })));
    }

    /// Hand the threads a pool's job: the pre-PoW hash and timestamp from
    /// `mining.notify`, the share difficulty from `mining.set_difficulty`, and
    /// the extranonce the pool reserved for this worker (none is fine).
    pub fn set_share_job(&self, job_id: String, pre_pow_hash: Hash, timestamp: u64, difficulty: f64, extranonce_hex: &str) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let target = kaspa_pow::stratum::target_for_stratum_difficulty(difficulty);
        let state = State::from_pre_pow(pre_pow_hash, timestamp, target);
        let (nonce_prefix, nonce_mask) = nonce_space_for_extranonce(extranonce_hex).unwrap_or((0, u64::MAX));
        {
            let mut recent = self.recent_share_jobs.lock().unwrap();
            recent.push_back((generation, job_id.clone()));
            while recent.len() > REMEMBERED_SHARE_JOBS {
                recent.pop_front();
            }
        }
        self.job.store(Some(Arc::new(Job { generation, state, work: Work::Share, nonce_prefix, nonce_mask })));
    }

    /// The pool job a solution belongs to, with its nonce, for `mining.submit`.
    /// None when the job is too old to remember, or the work was a block.
    pub fn share_for(&self, solution: &Solution) -> Option<(String, u64)> {
        let recent = self.recent_share_jobs.lock().unwrap();
        recent.iter().rev().find(|(generation, _)| *generation == solution.generation).map(|(_, id)| (id.clone(), solution.nonce))
    }

    /// Make this miner a pool's worker: the stats the session keeps and
    /// 'mine status' reads.
    pub fn attach_pool(&self, stats: Arc<PoolStats>) {
        let _ = self.pool.set(stats);
    }

    pub fn pool(&self) -> Option<Arc<PoolStats>> {
        self.pool.get().cloned()
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
        let Work::Block { header, transactions } = &job.work else { return None };
        let mut header = header.as_ref().clone();
        header.nonce = solution.nonce;
        // The header hash is derived from its fields, so it has to be
        // recomputed once the nonce changes or the node rejects the block.
        header.finalize();
        let block = Block::from_arcs(Arc::new(header), transactions.clone());
        Some((&block).into())
    }

    pub fn record_accepted(&self) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rejected(&self, reason: &str) {
        self.rejected.fetch_add(1, Ordering::Relaxed);
        *self.last_rejection.lock().unwrap() = Some(reason.to_string());
    }

    /// Why the node refused the last block it refused, if it ever did.
    pub fn last_rejection(&self) -> Option<String> {
        self.last_rejection.lock().unwrap().clone()
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

    pub fn uptime(&self) -> std::time::Duration {
        self.started.elapsed()
    }
}

/// Run a mining session against `rpc`, paying to `address`, until the miner
/// is stopped or `shutdown` is raised.
///
/// One task owns both halves: fetching work and submitting what comes back.
/// Keeping them together means there is a single place where the job
/// generation is advanced, so a solution can never be matched against a
/// template that has already been replaced. The same loop serves the wallet's
/// own miner and the background miner program (PLAN P8.3c).
pub fn spawn_session(
    rpc: Arc<DynRpcApi>,
    address: Address,
    miner: Arc<Miner>,
    solutions: std::sync::mpsc::Receiver<Solution>,
    shutdown: Arc<AtomicBool>,
) {
    workflow_core::task::spawn(async move {
        let extra = format!("marigold-wallet/{}", env!("CARGO_PKG_VERSION")).into_bytes();
        let mut last_refresh = std::time::Instant::now() - std::time::Duration::from_secs(60);
        // Said once per outage, not every refresh: a node that went away
        // filled the screen with the same line twice a second.
        let mut waiting_for_node = false;
        loop {
            if !miner.is_running() || shutdown.load(Ordering::SeqCst) {
                break;
            }
            if last_refresh.elapsed() >= std::time::Duration::from_millis(TEMPLATE_REFRESH_MS) {
                last_refresh = std::time::Instant::now();
                match rpc.get_block_template(address.clone(), extra.clone()).await {
                    Ok(response) => {
                        // Stopped while the node was away: nothing to announce.
                        if !miner.is_running() {
                            break;
                        }
                        if std::mem::take(&mut waiting_for_node) {
                            log_info!("mine: the node is back, mining goes on");
                        }
                        // Building on a chain the node has not finished
                        // reading produces blocks nobody will accept.
                        // MARIGOLD_MINE_UNSYNCED is the anchor drill's override
                        // (scripts/anchor-drill.sh): its attacker node is cut
                        // off from every peer on purpose, sees no blocks, and
                        // therefore never calls itself synced. Nobody else
                        // should set it; a node that is honestly behind makes
                        // blocks nobody accepts.
                        if response.is_synced || mine_unsynced() {
                            match Block::try_from(response.block) {
                                Ok(block) => miner.set_job(block),
                                Err(err) => log_warn!("mine: unusable block template ({err})"),
                            }
                        }
                    }
                    Err(err) => {
                        if !waiting_for_node {
                            waiting_for_node = true;
                            if err.to_string().contains("ibd") {
                                log_warn!("mine: no work yet — the sync is still catching up; mining begins on its own when it has");
                            } else {
                                log_warn!("mine: could not get work ({err}) — waiting for the node to come back");
                            }
                        }
                    }
                }
            }
            // Drain whatever the threads found since the last pass. With the
            // node away there is nowhere to send a solution, and the template
            // it solves is from before the outage: let them go quietly.
            if waiting_for_node {
                while solutions.try_recv().is_ok() {}
                workflow_core::task::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
            while let Ok(solution) = solutions.try_recv() {
                let Some(rpc_block) = miner.block_for(&solution) else { continue };
                match rpc.submit_block(rpc_block, false).await {
                    Ok(_) => {
                        // Deliberately silent. Announcing each block put a
                        // three-line interruption on the terminal every time
                        // one landed, on top of whatever the person was
                        // typing. 'mine status' reports the total.
                        miner.record_accepted();
                    }
                    Err(err) => {
                        miner.record_rejected(&err.to_string());
                        log_warn!("mine: block not accepted ({err})");
                    }
                }
            }
            workflow_core::task::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
}

/// A miner with a fixed payout address that a node's RPC can read and steer
/// (PLAN P8.3c): what `marigold-cli mine-to` runs, and what a wallet on
/// the same machine finds when it connects.
pub struct MinerHost {
    address: std::sync::OnceLock<Address>,
    rpc: std::sync::OnceLock<Arc<DynRpcApi>>,
    miner: std::sync::Mutex<Option<Arc<Miner>>>,
    shutdown: Arc<AtomicBool>,
    /// A pool to work for instead of a node of our own ("host:port").
    pool: std::sync::OnceLock<String>,
}

impl MinerHost {
    pub fn new(address: Address, shutdown: Arc<AtomicBool>) -> Arc<Self> {
        let host = Self::new_unbound(shutdown);
        host.bind_address(address);
        host
    }

    /// A host whose payout address comes later — the wallet service learns
    /// its own address only once the wallet is open.
    pub fn new_unbound(shutdown: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            address: std::sync::OnceLock::new(),
            rpc: std::sync::OnceLock::new(),
            miner: std::sync::Mutex::new(None),
            shutdown,
            pool: std::sync::OnceLock::new(),
        })
    }

    /// Work for a pool: shares go there over stratum and no node is needed.
    pub fn set_pool(&self, pool: String) {
        let _ = self.pool.set(pool);
    }

    pub fn pool_address(&self) -> Option<&str> {
        self.pool.get().map(String::as_str)
    }

    pub fn bind_address(&self, address: Address) {
        let _ = self.address.set(address);
    }

    pub fn address_bound(&self) -> bool {
        self.address.get().is_some()
    }

    /// The node to mine against, once it exists. The host is created before
    /// the node so the node's RPC can be handed the host at birth.
    pub fn bind(&self, rpc: Arc<DynRpcApi>) {
        let _ = self.rpc.set(rpc);
    }

    pub fn address(&self) -> Option<&Address> {
        self.address.get()
    }

    pub fn miner(&self) -> Option<Arc<Miner>> {
        self.miner.lock().unwrap().clone()
    }

    /// Start at `percent` of the machine; a running miner is replaced by a
    /// fresh thread set at the new share.
    pub fn start(&self, percent: u32) -> RpcResult<RpcMinerStatus> {
        if !(1..=100).contains(&percent) {
            return Err(RpcError::General(format!("{percent}% is not a share of the machine between 1 and 100")));
        }
        let Some(address) = self.address.get().cloned() else {
            return Err(RpcError::General("no address to pay the rewards to".to_string()));
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(pool) = self.pool.get() {
            if let Some(old) = self.miner.lock().unwrap().take() {
                old.stop();
            }
            let (miner, solutions) = Miner::start(percent);
            let stats = Arc::new(PoolStats::new(pool.clone(), worker_name()));
            miner.attach_pool(stats.clone());
            let login = format!("{address}.{}", stats.worker);
            crate::stratum::spawn_pool_session(stats, login, miner.clone(), solutions, self.shutdown.clone());
            self.miner.lock().unwrap().replace(miner);
            return Ok(self.status());
        }
        let Some(rpc) = self.rpc.get().cloned() else {
            return Err(RpcError::General("the node is not up yet".to_string()));
        };
        if let Some(old) = self.miner.lock().unwrap().take() {
            old.stop();
        }
        let (miner, solutions) = Miner::start(percent);
        spawn_session(rpc, address, miner.clone(), solutions, self.shutdown.clone());
        self.miner.lock().unwrap().replace(miner);
        Ok(self.status())
    }

    /// Stop, and report what the run came to. The status is read before the
    /// miner is let go: read after, it was always zero blocks and zero shares
    /// (the log said "0 shares this run" after 27, 2026-10-12).
    pub fn stop(&self) -> RpcMinerStatus {
        let status = self.status();
        if let Some(miner) = self.miner.lock().unwrap().take() {
            miner.stop();
        }
        status
    }

    pub fn status(&self) -> RpcMinerStatus {
        let mut status = RpcMinerStatus {
            available: true,
            cores: cores() as u32,
            address: self.address.get().map(|a| a.to_string()).unwrap_or_default(),
            ..Default::default()
        };
        if let Some(miner) = self.miner() {
            status.mining = true;
            status.percent = miner.percent();
            status.threads = miner.thread_count() as u32;
            status.hashrate = miner.hashrate();
            status.blocks_found = miner.blocks_found();
            status.blocks_accepted = miner.blocks_accepted();
            status.blocks_rejected = miner.blocks_rejected();
            status.uptime_seconds = miner.uptime().as_secs();
            if let Some(pool) = miner.pool() {
                status.pool = pool.pool.clone();
                status.shares_accepted = pool.accepted();
                status.shares_rejected = pool.rejected();
                status.share_difficulty = pool.difficulty();
            }
        }
        status
    }
}

impl MinerControl for MinerHost {
    fn status(&self) -> RpcMinerStatus {
        MinerHost::status(self)
    }

    fn control(&self, mining: bool, percent: Option<u32>) -> RpcResult<RpcMinerStatus> {
        if !mining {
            return Ok(self.stop());
        }
        let current = self.miner().map(|m| m.percent());
        match (percent, current) {
            // Already mining at that share, or asked to keep whatever it is at.
            (Some(wanted), Some(now)) if wanted == now => Ok(self.status()),
            (None, Some(_)) => Ok(self.status()),
            (wanted, _) => self.start(wanted.unwrap_or(50)),
        }
    }
}

pub use kaspa_wallet_service::format::format_hashrate;

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
    fn an_extranonce_reserves_the_top_bytes_of_the_nonce() {
        // Two bytes, as the bridge hands out: the top sixteen bits are the pool's.
        let (prefix, mask) = nonce_space_for_extranonce("ab12").unwrap();
        assert_eq!(prefix, 0xab12_0000_0000_0000);
        assert_eq!(mask, 0x0000_ffff_ffff_ffff);
        for raw in [0u64, 1, u64::MAX, 0x1234_5678_9abc_def0] {
            let nonce = (raw.wrapping_add(1) & mask) | prefix;
            assert!(format!("{nonce:016x}").starts_with("ab12"), "{nonce:016x}");
        }
        // No extranonce: the whole space is ours.
        assert_eq!(nonce_space_for_extranonce(""), Some((0, u64::MAX)));
        assert_eq!(nonce_space_for_extranonce("0x"), Some((0, u64::MAX)));
        // Garbage is refused rather than guessed at.
        assert_eq!(nonce_space_for_extranonce("abc"), None);
        assert_eq!(nonce_space_for_extranonce("zz"), None);
        assert_eq!(nonce_space_for_extranonce("0123456789abcdef"), None, "an eight-byte extranonce leaves nothing to search");
    }

    #[test]
    fn a_share_job_is_solved_against_the_pool_target_and_remembered() {
        let header = easy_header();
        let pre_pow = kaspa_consensus_core::hashing::header::hash_override_nonce_time(&header, 0, 0);
        let (miner, solutions) = Miner::start(1);
        // A difficulty so low the first few nonces clear it; the extranonce pins the top bytes.
        miner.set_share_job("job-7".to_string(), pre_pow, header.timestamp, 1e-9, "ab12");
        let solution = solutions.recv_timeout(std::time::Duration::from_secs(30)).expect("a share at this difficulty");
        miner.stop();
        let (job_id, nonce) = miner.share_for(&solution).expect("the job is remembered");
        assert_eq!(job_id, "job-7");
        assert!(format!("{nonce:016x}").starts_with("ab12"));
        let target = kaspa_pow::stratum::target_for_stratum_difficulty(1e-9);
        assert!(State::from_pre_pow(pre_pow, header.timestamp, target).check_pow(nonce).0, "the share meets the pool target");
        assert!(miner.block_for(&solution).is_none(), "a share is not a block");
    }

    #[test]
    fn stopping_the_host_reports_the_run_it_just_ended() {
        let host = MinerHost::new_unbound(Arc::new(AtomicBool::new(false)));
        let (miner, _solutions) = Miner::start(1);
        miner.record_accepted();
        host.miner.lock().unwrap().replace(miner);
        let status = host.stop();
        assert!(status.mining, "the status is the run that just ended, not the idle host");
        assert_eq!(status.blocks_accepted, 1);
        assert!(host.miner().is_none());
    }

    #[test]
    fn a_pool_worker_name_is_plain() {
        let name = worker_name();
        assert!(!name.is_empty() && name.len() <= 32);
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{name}");
    }
}
