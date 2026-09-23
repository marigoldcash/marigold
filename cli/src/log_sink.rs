//! Routing the `log` crate through the terminal.
//!
//! The wallet talks to its user with `tprintln!`, but everything inside the
//! embedded node uses the `log` facade. Left to log4rs those records go to
//! stdout with a bare LF, and a raw-mode terminal never returns the cursor to
//! column zero — so the output walks diagonally across the screen and the
//! prompt is trampled.
//!
//! This logger hands each record to the terminal, which emits CRLF and repaints
//! the prompt afterwards. Before a terminal exists it falls back to stdout with
//! an explicit CRLF, so early startup lines are still readable.

use crate::cli::KaspaCli;
use std::sync::{Arc, OnceLock, RwLock};

static TERMINAL: OnceLock<RwLock<Option<Arc<KaspaCli>>>> = OnceLock::new();
static PROGRESS: OnceLock<RwLock<Option<SyncProgress>>> = OnceLock::new();

/// How far through its first sync a node is.
///
/// None of this is available over RPC: the whole initial sync runs inside a
/// staging consensus, and `get_block_dag_info` answers from the ACTIVE one — so
/// it reports zero blocks and zero headers throughout, however much work has
/// been done. The node does say where it is, but only in its log records, and
/// those pass through here on their way to the terminal. So this reads them.
///
/// Scraping log text is not a nice way to learn this, and it is written down as
/// what it is: the alternative is a status display that shows three zeroes for
/// an hour and looks broken.
#[derive(Clone, Debug)]
pub enum SyncProgress {
    /// Validating the pruning point proof, counting DOWN from level 250.
    VerifyingProof { level: u32 },
    /// Downloading the selected-chain headers between the pruning point and the
    /// syncer's tip. Bounded by finality depth, so a maximum is known.
    ChainSegment { headers: u64 },
    /// Processing the header DAG. The node's own percentage, by DAA score.
    Headers { headers: u64, percent: u32, block_time: Option<String> },
    /// Downloading block bodies, after the headers are in.
    Blocks { blocks: u64, percent: u32 },
}

fn progress_slot() -> &'static RwLock<Option<SyncProgress>> {
    PROGRESS.get_or_init(|| RwLock::new(None))
}

/// Node warnings and errors held back since the last time logs were on.
static SUPPRESSED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Unix seconds when the sync last moved. Zero means it never has.
static LAST_PROGRESS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The node has reported block bodies at least once.
static BLOCKS_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Which pass of the sync the node is on, counting from zero.
///
/// A first sync runs in passes — headers, then blocks, then whatever arrived
/// meanwhile, headers first again — and each pass reports its own
/// percentage from zero. Shown as they come, the steps went 3 → 2 → 3 and
/// the figure fell from 77% to 99% to 76% (tester, 2026-09-20). Headers
/// after blocks is a new pass; the status names it rather than counting
/// backwards.
static PASS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// How many times a step fell back to its start within one pass: the node
/// restarts the step when the peer it reads from goes away.
static RESTARTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LAST_HEADERS_PERCENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn sync_restarts() -> u32 {
    RESTARTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The sync pass the node is on: 0 for the first, 1 for the second, and so on.
pub fn sync_pass() -> u32 {
    PASS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Store a parsed progress line and keep track of which pass it belongs to.
fn record_progress(progress: SyncProgress) {
    use std::sync::atomic::Ordering::Relaxed;
    match progress {
        SyncProgress::Blocks { .. } => BLOCKS_SEEN.store(true, Relaxed),
        SyncProgress::Headers { percent, .. } => {
            if BLOCKS_SEEN.swap(false, Relaxed) {
                PASS.fetch_add(1, Relaxed);
                LAST_HEADERS_PERCENT.store(0, Relaxed);
            }
            // Within one pass the figure only ever rises; a fall of more than
            // a few points is the step starting over.
            let last = LAST_HEADERS_PERCENT.swap(percent, Relaxed);
            if percent + 5 < last {
                RESTARTS.fetch_add(1, Relaxed);
            }
        }
        SyncProgress::ChainSegment { .. } => {
            if BLOCKS_SEEN.swap(false, Relaxed) {
                PASS.fetch_add(1, Relaxed);
                LAST_HEADERS_PERCENT.store(0, Relaxed);
            }
        }
        SyncProgress::VerifyingProof { .. } => {}
    }
    *progress_slot().write().unwrap() = Some(progress);
}

/// How many node warnings have been kept off the screen.
pub fn suppressed_count() -> usize {
    SUPPRESSED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Seconds since the sync last advanced, or None if it has not started.
///
/// This is the honest version of "is anything wrong?". A node with a stalled
/// IBD looks exactly like a node that is working, and the difference is
/// visible only in whether the numbers move.
pub fn seconds_since_progress() -> Option<u64> {
    let last = LAST_PROGRESS.load(std::sync::atomic::Ordering::Relaxed);
    if last == 0 {
        return None;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(now.saturating_sub(last))
}

pub fn sync_progress() -> Option<SyncProgress> {
    progress_slot().read().unwrap().clone()
}

pub fn clear_sync_progress() {
    *progress_slot().write().unwrap() = None;
    BLOCKS_SEEN.store(false, std::sync::atomic::Ordering::Relaxed);
    PASS.store(0, std::sync::atomic::Ordering::Relaxed);
    RESTARTS.store(0, std::sync::atomic::Ordering::Relaxed);
    LAST_HEADERS_PERCENT.store(0, std::sync::atomic::Ordering::Relaxed);
    LAST_PROGRESS.store(0, std::sync::atomic::Ordering::Relaxed);
    SUPPRESSED.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Pull progress out of a node log line, if it carries any.
fn parse_progress(line: &str) -> Option<SyncProgress> {
    let after = |prefix: &str| line.split_once(prefix).map(|(_, rest)| rest);
    let first_number = |s: &str| s.split_whitespace().next()?.replace(',', "").parse::<u64>().ok();
    let percent = |s: &str| s.split_once('(')?.1.split_once("%)").and_then(|(p, _)| p.parse::<u32>().ok());

    if let Some(rest) = after("Validating level ") {
        return first_number(rest).map(|level| SyncProgress::VerifyingProof { level: level as u32 });
    }
    // "Downloaded N headers..." and "Finished downloading N headers..." differ
    // in the verb, so key on the phrase that does not change and take the
    // number sitting in front of "headers".
    if line.contains("pruning point chain segment") {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(i) = words.iter().position(|w| *w == "headers")
            && let Some(headers) = i.checked_sub(1).and_then(|j| words[j].replace(',', "").parse::<u64>().ok())
        {
            return Some(SyncProgress::ChainSegment { headers });
        }
    }
    if let Some(rest) = after("IBD: Processed ") {
        let headers = first_number(rest)?;
        let pct = percent(line)?;
        if line.contains("block headers") {
            let block_time = line.split_once("last block timestamp: ").map(|(_, t)| t.trim().to_string());
            return Some(SyncProgress::Headers { headers, percent: pct, block_time });
        }
        return Some(SyncProgress::Blocks { blocks: headers, percent: pct });
    }
    None
}

fn slot() -> &'static RwLock<Option<Arc<KaspaCli>>> {
    TERMINAL.get_or_init(|| RwLock::new(None))
}

/// Give the logger a terminal to write through. Called once the CLI exists;
/// records logged before this still reach stdout.
pub fn attach(cli: &Arc<KaspaCli>) {
    *slot().write().unwrap() = Some(cli.clone());
}

struct TerminalLogger;

impl log::Log for TerminalLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        // Everything reaches us, always. Progress is read out of these records
        // (see SyncProgress), so filtering them at the gate would leave the
        // status display blind exactly when the logs are quiet — which is the
        // normal case. What the user asked to be quiet is the PRINTING, and
        // that is decided below.
        true
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Time and level, because these records exist to answer "is it making
        // progress?" — a stream of messages with no clock cannot. Short form:
        // this is a wallet, not a server log.
        if let Some(progress) = parse_progress(&record.args().to_string()) {
            record_progress(progress);
            if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                LAST_PROGRESS.store(now.as_secs(), std::sync::atomic::Ordering::Relaxed);
            }
        }

        // Nothing from the node reaches the screen unless it was asked for.
        //
        // Warnings used to be exempt, on the reasoning that a warning is worth
        // seeing. It is not, here: a peer timing out and being dropped is
        // ordinary p2p churn, and "SendPingsFlow flow error: timeout expired
        // after 120s, disconnecting from peer 203.0.113.12:26211" landed in the
        // middle of a new user creating their first wallet — between the
        // phishing-hint prompt and the password prompt. It reads like the
        // program breaking. It is the program working.
        //
        // Real trouble is reported as trouble instead: the node's progress is
        // timestamped below, and 'node status' says so when it stops moving.
        // The count is kept so 'node details' can point at 'node logs'.
        if !crate::embedded_logs_wanted() {
            if record.level() <= log::Level::Warn {
                SUPPRESSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }

        let line = format!("{} [{}] {}", chrono::Local::now().format("%H:%M:%S"), record.level(), record.args());
        let cli = slot().read().unwrap().clone();
        match cli.as_ref().and_then(|cli| cli.try_term()) {
            Some(term) => term.writeln(line),
            // \r first: whatever wrote last may have left the cursor mid-line.
            None => println!("\r{line}\r"),
        }
    }

    fn flush(&self) {}
}

pub fn install() {
    // set_logger can only succeed once per process; a second call is a no-op
    // rather than an error, so tests that construct several CLIs are fine.
    let _ = log::set_boxed_logger(Box::new(TerminalLogger));
    log::set_max_level(log::LevelFilter::Info);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real lines from a node's own output. The status display reads progress
    /// out of these because it is not available any other way — so if the node
    /// ever rewords them, this test is what says so.
    #[test]
    fn progress_is_read_from_the_node_s_own_words() {
        match parse_progress("Validating level 47 from the pruning point proof (30 headers)") {
            Some(SyncProgress::VerifyingProof { level }) => assert_eq!(level, 47),
            other => panic!("proof level not parsed: {other:?}"),
        }
        match parse_progress("Downloaded 220000 headers from the pruning point chain segment") {
            Some(SyncProgress::ChainSegment { headers }) => assert_eq!(headers, 220_000),
            other => panic!("chain segment not parsed: {other:?}"),
        }
        match parse_progress("Finished downloading 351596 headers from the pruning point chain segment") {
            Some(SyncProgress::ChainSegment { headers }) => assert_eq!(headers, 351_596),
            other => panic!("finished segment not parsed: {other:?}"),
        }
        match parse_progress("IBD: Processed 113763 block headers (9%) last block timestamp: 2026-09-06 04:09:19.000:-0300") {
            Some(SyncProgress::Headers { headers, percent, block_time }) => {
                assert_eq!(headers, 113_763);
                assert_eq!(percent, 9);
                assert!(block_time.unwrap().starts_with("2026-09-06"));
            }
            other => panic!("header progress not parsed: {other:?}"),
        }
        match parse_progress("IBD: Processed 5000 blocks (42%)") {
            Some(SyncProgress::Blocks { blocks, percent }) => {
                assert_eq!(blocks, 5_000);
                assert_eq!(percent, 42);
            }
            other => panic!("block progress not parsed: {other:?}"),
        }
        // Ordinary chatter must not be mistaken for progress.
    }

    /// Headers after blocks means a new pass; the pass survives the blocks
    /// that follow it, a third set of headers is a third pass, and a fresh
    /// sync starts over.
    #[test]
    fn headers_after_blocks_start_a_new_pass() {
        clear_sync_progress();
        let headers = "IBD: Processed 100 block headers (9%) last block timestamp: 2026-09-20 10:00:00.000:-0300";
        record_progress(parse_progress(headers).unwrap());
        assert_eq!(sync_pass(), 0);
        record_progress(parse_progress("IBD: Processed 5000 blocks (77%)").unwrap());
        assert_eq!(sync_pass(), 0, "the first pass over the blocks is still the first pass");
        record_progress(parse_progress(headers).unwrap());
        assert_eq!(sync_pass(), 1);
        record_progress(parse_progress(headers).unwrap());
        assert_eq!(sync_pass(), 1, "more headers in the same pass do not count again");
        record_progress(parse_progress("IBD: Processed 30 blocks (10%)").unwrap());
        assert_eq!(sync_pass(), 1);
        record_progress(parse_progress(headers).unwrap());
        assert_eq!(sync_pass(), 2);
        clear_sync_progress();
        assert_eq!(sync_pass(), 0);
    }

    #[test]
    fn ordinary_chatter_is_not_progress() {
        assert!(parse_progress("P2P Server starting on: 0.0.0.0:26211").is_none());
        assert!(parse_progress("Querying 3 DNS seeders").is_none());
    }
}
