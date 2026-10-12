//! Mining for a pool over stratum (docs/marigold/MINING-POOL-PLAN.md, M8).
//!
//! The pool, not our node, hands out the work: `mining.notify` carries a
//! pre-PoW hash and a timestamp, `mining.set_difficulty` the share target,
//! and `mining.set_extranonce` the top bytes of the nonce reserved for this
//! worker. The hashing threads in `miner.rs` are the same ones the solo miner
//! uses; this file is the conversation around them. The dialect is the one
//! this repository's own stratum bridge speaks to a miner it does not
//! recognise as an ASIC: one JSON message per line.
//!
//! The payout address goes to the pool as the login (`address.worker`), which
//! is why the solo miner's own-node rule does not apply here: the pool pays
//! by its own rules, and knowing our address is the point.

use crate::miner::{Miner, PoolStats, Solution};
use kaspa_hashes::Hash;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use workflow_log::{log_info, log_warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const SUBMIT_TICK: Duration = Duration::from_millis(50);
/// Shares sent per tick at most. A pool that sets the difficulty far too low
/// gets the first few of a flood and its vardiff fixes the rest; it never
/// gets the flood.
const SUBMITS_PER_TICK: usize = 8;
const MAX_BACKOFF_SECS: u64 = 30;
/// Submits still waiting for an answer that are remembered.
const REMEMBERED_SUBMITS: usize = 256;
const REJECTION_LOG_SPACING: Duration = Duration::from_secs(10);

/// A job as the bridge's standard `mining.notify` carries it.
#[derive(Debug, Clone, PartialEq)]
pub struct Notify {
    pub job_id: String,
    pub pre_pow_hash: Hash,
    pub timestamp: u64,
}

/// The standard job: `[job id, the pre-PoW hash as four little-endian u64s, timestamp]`.
/// The job id arrives as a string, or as a number from other bridges.
pub fn parse_notify(params: &[Value]) -> Option<Notify> {
    let job_id = match params.first()? {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    let words = params.get(1)?.as_array()?;
    if words.len() != 4 {
        return None;
    }
    let mut arr = [0u64; 4];
    for (slot, word) in arr.iter_mut().zip(words) {
        *slot = word.as_u64()?;
    }
    let timestamp = params.get(2)?.as_u64()?;
    Some(Notify { job_id, pre_pow_hash: Hash::from_le_u64(arr), timestamp })
}

/// The nonce as the bridge wants it: sixteen hex digits, the extranonce in the leading ones.
pub fn nonce_hex(nonce: u64) -> String {
    format!("{nonce:016x}")
}

/// What the pool said to one of our requests.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Accepted,
    Refused(String),
}

/// A response (a message with an id and no method) as `(id, reply)`.
/// Errors come as the bridge sends them, `[code, "message", null]`, or as an
/// object with a `message`; either way the words are kept for 'mine status'.
pub fn parse_reply(message: &Value) -> Option<(u64, Reply)> {
    if message.get("method").is_some() {
        return None;
    }
    let id = message.get("id")?;
    let id = id.as_u64().or_else(|| id.as_str().and_then(|text| text.parse().ok()))?;
    if let Some(error) = message.get("error").filter(|error| !error.is_null()) {
        let text = match error {
            Value::Array(parts) => parts.get(1).and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| error.to_string()),
            Value::Object(fields) => {
                fields.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| error.to_string())
            }
            other => other.to_string(),
        };
        return Some((id, Reply::Refused(text)));
    }
    match message.get("result") {
        Some(Value::Bool(false)) => Some((id, Reply::Refused("the pool said no".to_string()))),
        Some(Value::Null) | None => Some((id, Reply::Refused("the pool gave no answer".to_string()))),
        Some(_) => Some((id, Reply::Accepted)),
    }
}

/// Everything the pool has told us so far that the next job depends on.
struct Session {
    extranonce: String,
    difficulty: f64,
    job: Option<Notify>,
    pending: VecDeque<u64>,
    last_rejection_log: Option<Instant>,
    unreadable_job_said: bool,
}

impl Session {
    fn new() -> Self {
        Self {
            extranonce: String::new(),
            difficulty: 1.0,
            job: None,
            pending: VecDeque::new(),
            last_rejection_log: None,
            unreadable_job_said: false,
        }
    }

    /// Hand the threads the current job under the current difficulty and extranonce.
    fn issue(&self, miner: &Miner) {
        if let Some(job) = &self.job {
            miner.set_share_job(job.job_id.clone(), job.pre_pow_hash, job.timestamp, self.difficulty, &self.extranonce);
        }
    }

    fn handle(&mut self, line: &str, miner: &Miner, stats: &PoolStats) {
        let Ok(message) = serde_json::from_str::<Value>(line) else { return };
        let params = message.get("params").and_then(Value::as_array).cloned().unwrap_or_default();
        match message.get("method").and_then(Value::as_str) {
            Some("mining.set_extranonce") => {
                if let Some(extranonce) = params.first().and_then(Value::as_str) {
                    self.extranonce = extranonce.to_string();
                    self.issue(miner);
                }
            }
            Some("mining.set_difficulty") => {
                if let Some(difficulty) = params.first().and_then(Value::as_f64).filter(|d| *d > 0.0 && d.is_finite()) {
                    self.difficulty = difficulty;
                    stats.set_difficulty(difficulty);
                    self.issue(miner);
                }
            }
            Some("mining.notify") => match parse_notify(&params) {
                Some(job) => {
                    self.job = Some(job);
                    self.issue(miner);
                }
                None => {
                    if !self.unreadable_job_said {
                        self.unreadable_job_said = true;
                        log_warn!("mine: the pool at {} sends its jobs in a form this wallet does not read", stats.pool);
                    }
                }
            },
            Some(_) => {}
            None => {
                if let Some((id, reply)) = parse_reply(&message)
                    && let Some(position) = self.pending.iter().position(|pending| *pending == id)
                {
                    self.pending.remove(position);
                    match reply {
                        Reply::Accepted => {
                            if stats.accepted() == 0 {
                                log_info!("mine: the pool at {} accepted the first share", stats.pool);
                            }
                            stats.record_accepted();
                        }
                        Reply::Refused(text) => {
                            stats.record_rejected(&text);
                            if self.last_rejection_log.is_none_or(|at| at.elapsed() >= REJECTION_LOG_SPACING) {
                                self.last_rejection_log = Some(Instant::now());
                                log_warn!("mine: the pool at {} refused a share: {text}", stats.pool);
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn send(write: &mut OwnedWriteHalf, id: u64, method: &str, params: Vec<Value>) -> Result<(), String> {
    let mut line = json!({"id": id, "jsonrpc": "2.0", "method": method, "params": params}).to_string();
    line.push('\n');
    write.write_all(line.as_bytes()).await.map_err(|err| format!("could not send to the pool: {err}"))
}

/// Read until the reply to request `id` arrives; notifications that come
/// first (the bridge sends the extranonce and difficulty early) are applied.
async fn wait_reply(
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    id: u64,
    session: &mut Session,
    miner: &Miner,
    stats: &PoolStats,
) -> Result<Reply, String> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("the pool did not answer the handshake".to_string());
        }
        let line = tokio::time::timeout(remaining, lines.next_line())
            .await
            .map_err(|_| "the pool did not answer the handshake".to_string())?
            .map_err(|err| err.to_string())?
            .ok_or_else(|| "the pool closed the connection".to_string())?;
        if let Ok(message) = serde_json::from_str::<Value>(&line)
            && let Some((reply_id, reply)) = parse_reply(&message)
            && reply_id == id
        {
            return Ok(reply);
        }
        session.handle(&line, miner, stats);
    }
}

/// One connection, from the handshake until it drops or we stop.
async fn serve(
    stats: &PoolStats,
    login: &str,
    miner: &Miner,
    solutions: &mut std::sync::mpsc::Receiver<Solution>,
    shutdown: &AtomicBool,
) -> Result<(), String> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&stats.pool))
        .await
        .map_err(|_| "no answer".to_string())?
        .map_err(|err| err.to_string())?;
    let _ = stream.set_nodelay(true);
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let mut session = Session::new();
    let mut next_id: u64 = 1;

    send(
        &mut write,
        next_id,
        "mining.subscribe",
        vec![json!(format!("marigold-cli/{}", env!("CARGO_PKG_VERSION"))), json!("EthereumStratum/1.0.0")],
    )
    .await?;
    if let Reply::Refused(text) = wait_reply(&mut lines, next_id, &mut session, miner, stats).await? {
        return Err(format!("the pool turned the connection away: {text}"));
    }
    next_id += 1;
    send(&mut write, next_id, "mining.authorize", vec![json!(login), json!("x")]).await?;
    if let Reply::Refused(text) = wait_reply(&mut lines, next_id, &mut session, miner, stats).await? {
        return Err(format!("the pool refused the login {login}: {text}"));
    }
    stats.set_connected(true);
    log_info!("mine: working for the pool at {} as {login}", stats.pool);

    let mut tick = tokio::time::interval(SUBMIT_TICK);
    loop {
        if !miner.is_running() || shutdown.load(Ordering::SeqCst) {
            return Ok(());
        }
        tokio::select! {
            line = lines.next_line() => {
                let line = line.map_err(|err| err.to_string())?.ok_or_else(|| "the pool closed the connection".to_string())?;
                session.handle(&line, miner, stats);
            }
            _ = tick.tick() => {
                let mut sent = 0;
                while let Ok(solution) = solutions.try_recv() {
                    // Past the cap the rest of the tick's finds are let go.
                    if sent >= SUBMITS_PER_TICK {
                        continue;
                    }
                    let Some((job_id, nonce)) = miner.share_for(&solution) else { continue };
                    next_id += 1;
                    send(&mut write, next_id, "mining.submit", vec![json!(login), json!(job_id), json!(nonce_hex(nonce))]).await?;
                    session.pending.push_back(next_id);
                    while session.pending.len() > REMEMBERED_SUBMITS {
                        session.pending.pop_front();
                    }
                    sent += 1;
                }
            }
        }
    }
}

/// Run a pool session for `miner` until it stops or `shutdown` is raised,
/// reconnecting with a growing pause whenever the pool goes away.
pub fn spawn_pool_session(
    stats: Arc<PoolStats>,
    login: String,
    miner: Arc<Miner>,
    solutions: std::sync::mpsc::Receiver<Solution>,
    shutdown: Arc<AtomicBool>,
) {
    workflow_core::task::spawn(async move {
        let mut solutions = solutions;
        let mut backoff = 1u64;
        while miner.is_running() && !shutdown.load(Ordering::SeqCst) {
            match serve(&stats, &login, &miner, &mut solutions, &shutdown).await {
                Ok(()) => break,
                Err(err) => {
                    let was_connected = stats.is_connected();
                    stats.set_connected(false);
                    if was_connected {
                        backoff = 1;
                    }
                    log_warn!("mine: pool {}: {err} — trying again in {backoff}s", stats.pool);
                    workflow_core::task::sleep(Duration::from_secs(backoff)).await;
                    // Anything found while there was nobody to send it to is
                    // for a job the pool has moved past.
                    while solutions.try_recv().is_ok() {}
                    backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
                }
            }
        }
        stats.set_connected(false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::net::TcpListener;

    /// What the bridge does to a hash for the standard job: four little-endian u64s.
    fn bridge_job_words(hash: &Hash) -> Vec<Value> {
        hash.as_bytes().chunks(8).map(|chunk| json!(u64::from_le_bytes(chunk.try_into().unwrap()))).collect()
    }

    #[test]
    fn a_standard_job_reads_back_to_the_hash_the_bridge_started_from() {
        let hash = Hash::from_bytes([0x5a; 32]);
        let params = vec![json!("17"), Value::Array(bridge_job_words(&hash)), json!(1_700_000_000_123u64)];
        let job = parse_notify(&params).expect("a readable job");
        assert_eq!(job, Notify { job_id: "17".to_string(), pre_pow_hash: hash, timestamp: 1_700_000_000_123 });
        // A numeric job id is read the same way.
        let numeric = vec![json!(17), Value::Array(bridge_job_words(&hash)), json!(5u64)];
        assert_eq!(parse_notify(&numeric).unwrap().job_id, "17");
        // The IceRiver single-string form is not ours.
        assert!(parse_notify(&[json!("17"), json!("ab".repeat(40)), json!(5u64)]).is_none());
        assert!(parse_notify(&[json!("17"), json!([1, 2, 3]), json!(5u64)]).is_none());
    }

    #[test]
    fn nonces_go_out_as_sixteen_hex_digits() {
        assert_eq!(nonce_hex(0xab12_0000_0000_0001), "ab12000000000001");
        assert_eq!(nonce_hex(1), "0000000000000001");
    }

    #[test]
    fn replies_are_read_the_way_the_bridge_writes_them() {
        assert_eq!(parse_reply(&json!({"id": 7, "result": true, "error": null})), Some((7, Reply::Accepted)));
        assert_eq!(parse_reply(&json!({"id": "7", "result": [true, "EthereumStratum/1.0.0"]})), Some((7, Reply::Accepted)));
        assert_eq!(
            parse_reply(&json!({"id": 9, "result": null, "error": [23, "Invalid difficulty", null]})),
            Some((9, Reply::Refused("Invalid difficulty".to_string())))
        );
        assert_eq!(
            parse_reply(&json!({"id": 9, "error": {"code": 21, "message": "Job not found"}})),
            Some((9, Reply::Refused("Job not found".to_string())))
        );
        assert_eq!(parse_reply(&json!({"id": 3, "result": false})), Some((3, Reply::Refused("the pool said no".to_string()))));
        assert!(parse_reply(&json!({"method": "mining.notify", "params": []})).is_none());
    }

    /// A pool that speaks exactly as the bridge does to an unknown miner.
    async fn fake_pool(listener: TcpListener, login: String, nonces: Arc<Mutex<Vec<String>>>, hash: Hash) {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        let say = |value: Value| {
            let line = value.to_string() + "\n";
            async move { line }
        };
        while let Ok(Some(line)) = lines.next_line().await {
            let message: Value = serde_json::from_str(&line).unwrap();
            let id = message["id"].clone();
            let params = message["params"].as_array().cloned().unwrap_or_default();
            match message["method"].as_str() {
                Some("mining.subscribe") => {
                    assert!(params[0].as_str().unwrap().starts_with("marigold-cli/"));
                    let line = say(json!({"id": id, "result": [true, "EthereumStratum/1.0.0"], "error": null})).await;
                    write.write_all(line.as_bytes()).await.unwrap();
                }
                Some("mining.authorize") => {
                    assert_eq!(params[0], json!(login));
                    let mut out = say(json!({"id": id, "result": true, "error": null})).await;
                    out += &say(json!({"jsonrpc": "2.0", "method": "mining.set_extranonce", "params": ["ab12"]})).await;
                    out += &say(json!({"jsonrpc": "2.0", "method": "mining.set_difficulty", "params": [1e-9]})).await;
                    out += &say(json!({"jsonrpc": "2.0", "id": "1", "method": "mining.notify", "params": ["1", bridge_job_words(&hash), 1_700_000_000_000u64]})).await;
                    write.write_all(out.as_bytes()).await.unwrap();
                }
                Some("mining.submit") => {
                    assert_eq!(params[0], json!(login));
                    assert_eq!(params[1], json!("1"));
                    nonces.lock().unwrap().push(params[2].as_str().unwrap().to_string());
                    let line = say(json!({"id": id, "result": true, "error": null})).await;
                    write.write_all(line.as_bytes()).await.unwrap();
                }
                _ => {}
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_share_travels_to_the_pool_and_comes_back_accepted() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let pool = listener.local_addr().unwrap().to_string();
        let login = "marigoldtest:qqworker.rig7".to_string();
        let nonces = Arc::new(Mutex::new(Vec::new()));
        let hash = Hash::from_bytes([0x33; 32]);
        tokio::spawn(fake_pool(listener, login.clone(), nonces.clone(), hash));

        let (miner, solutions) = Miner::start(1);
        let stats = Arc::new(PoolStats::new(pool, "rig7".to_string()));
        miner.attach_pool(stats.clone());
        let shutdown = Arc::new(AtomicBool::new(false));
        spawn_pool_session(stats.clone(), login, miner.clone(), solutions, shutdown.clone());

        let started = Instant::now();
        while stats.accepted() == 0 && started.elapsed() < Duration::from_secs(30) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        shutdown.store(true, Ordering::SeqCst);
        miner.stop();

        assert!(stats.is_connected() || stats.accepted() > 0, "the session connected");
        assert!(stats.accepted() >= 1, "the pool accepted a share");
        assert_eq!(stats.rejected(), 0);
        assert_eq!(stats.difficulty(), 1e-9);
        let nonces = nonces.lock().unwrap();
        assert!(!nonces.is_empty());
        for nonce in nonces.iter() {
            assert_eq!(nonce.len(), 16, "{nonce}");
            assert!(nonce.starts_with("ab12"), "the extranonce leads the nonce: {nonce}");
            let value = u64::from_str_radix(nonce, 16).unwrap();
            let target = kaspa_pow::stratum::target_for_stratum_difficulty(1e-9);
            assert!(
                kaspa_pow::State::from_pre_pow(hash, 1_700_000_000_000, target).check_pow(value).0,
                "every submitted share meets the target"
            );
        }
    }
}
