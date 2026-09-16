extern crate self as kaspa_cli;

mod backup;
mod cli;
#[cfg(feature = "embedded-node")]
pub mod embedded;
#[cfg(feature = "embedded-node")]
pub mod headless;
pub mod error;
pub(crate) mod log_sink;
pub mod extensions;
mod helpers;
mod imports;
mod matchers;
#[cfg(feature = "embedded-node")]
pub mod miner;
pub mod modules;
mod notifier;
pub mod result;
pub mod space;
pub mod platform;
pub mod splash;
pub mod ui;
pub mod utils;
mod wizards;

pub use cli::{KaspaCli, Options, TerminalOptions, TerminalTarget, kaspa_cli};
pub use workflow_terminal::Terminal;

/// Whether node log records should be printed. Always false without the
/// embedded node, which has no logs of its own to narrate.
#[cfg(feature = "embedded-node")]
pub(crate) fn embedded_logs_wanted() -> bool {
    embedded::logs_wanted()
}

#[cfg(not(feature = "embedded-node"))]
pub(crate) fn embedded_logs_wanted() -> bool {
    false
}
