extern crate self as kaspa_cli;

mod backup;
mod cli;
#[cfg(feature = "embedded-node")]
pub mod embedded;
pub mod error;
pub mod extensions;
#[cfg(feature = "embedded-node")]
pub mod headless;
mod helpers;
mod imports;
// The sync-progress side of the log sink is read only by the embedded node.
#[cfg_attr(not(feature = "embedded-node"), allow(dead_code))]
pub(crate) mod log_sink;
mod matchers;
// Not feature-gated: `cores()`, `format_hashrate` and the `MinerControl`
// plumbing are used by the status paths whether or not a node is compiled
// in, and everything the module needs is an unconditional dependency.
pub mod memory;
pub mod miner;
pub mod modules;
mod notifier;
pub mod platform;
pub mod qrpng;
#[cfg(not(target_arch = "wasm32"))]
pub mod release_check;
pub mod result;
#[cfg(feature = "embedded-node")]
pub mod serve;
pub mod space;
pub mod splash;
#[cfg(feature = "embedded-node")]
pub mod telegram;
#[cfg(feature = "embedded-node")]
pub mod tgbackup;
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

/// `marigold-cli --help`: the whole binary on one screen, for whoever gets it
/// without the docs. The wallet's own commands have 'help' inside.
pub const HELP: &str = "marigold-cli {version} — the Marigold wallet, and the miner

  marigold-cli                       open the wallet
  marigold-cli mine-to <address> [<percent>] [--network <id>]
                                     mine in the background — no wallet needed
  marigold-cli serve <wallet> [--password-file <path>] [--mine <percent>]
                                     keep a wallet open as a service, answering
                                     your Telegram bot ('mobile telegram' pairs it)
  marigold-cli --version             print the version
  marigold-cli --platform            what this build is and what it sees of this machine;
                                     paste it into a bug report

Mining without a wallet open:

  marigold-cli mine-to marigold:qq...your-address...  50

  <address>   where the rewards go; 'address' in your wallet shows yours
  <percent>   share of this machine, 1-100 (default 50). The threads run at
              the lowest priority the system has, so 100 still steps aside
              the moment anything else wants the processor.
  --network   mainnet, testnet-10, ...; by default the one the address is on
  --node      mine against a node already running instead of syncing one
              here: grpc://127.0.0.1:26210 (a marigoldd's default) or
              ws://127.0.0.1:27210

  It first syncs a copy of the network on this machine (several gigabytes,
  a while the first time), then mines, logging to this terminal a line a
  minute. Ctrl-C or SIGTERM stops it cleanly. It stays in the foreground on
  purpose: systemd or Docker keep it running, and there is a unit file and a
  compose service for that in the docs below.

  A wallet on the same machine finds it by itself: 'connect' uses the
  miner's copy of the network instead of syncing a second one, and
  'mine start', 'mine stop' and 'mine status' steer the miner from there.

Docs and downloads: https://github.com/marigoldcash/marigold-wallet
";
