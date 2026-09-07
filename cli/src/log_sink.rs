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
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Time and level, because these records exist to answer "is it making
        // progress?" — a stream of messages with no clock cannot. Short form:
        // this is a wallet, not a server log.
        let line = format!(
            "{} [{}] {}",
            chrono::Local::now().format("%H:%M:%S"),
            record.level(),
            record.args()
        );
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
