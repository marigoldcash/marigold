extern crate self as kaspa_cli;

pub mod backup;

/// The wallet password policy (founder, 2026-10-10): a password, or at least a
/// PIN, of a reasonable length is required of a regular user; a wallet without
/// a password is a deliberate advanced choice — `advanced on` in the terminal,
/// the advanced box on the desktop's Create screen, or
/// `MARIGOLD_ALLOW_EMPTY_PASSWORD=1` for scripts — whatever the reason. Nothing
/// here changes at mainnet; the advanced choice stays.
pub const MIN_WALLET_PASSWORD_CHARS: usize = 4;

/// Scripts and tests say so in the environment rather than in a terminal mode.
pub fn empty_password_allowed_by_env() -> bool {
    std::env::var("MARIGOLD_ALLOW_EMPTY_PASSWORD").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// What a wallet password typed at creation gets: the one place the policy
/// lives, so the terminal wizard and the desktop agree, and so a test can
/// hold the policy to the requirement (GitHub #20). `allow_empty` is the
/// advanced choice made for this creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordVerdict {
    /// Empty, by an advanced choice: accepted, with a note.
    EmptyAllowed,
    /// Empty without that choice: ask again, and say where the choice is.
    EmptyRefused,
    /// Shorter than `MIN_WALLET_PASSWORD_CHARS`: ask again.
    TooShort,
    Fine,
}

pub fn password_verdict(password: &str, allow_empty: bool) -> PasswordVerdict {
    let chars = password.chars().count();
    if chars == 0 {
        if allow_empty { PasswordVerdict::EmptyAllowed } else { PasswordVerdict::EmptyRefused }
    } else if chars < MIN_WALLET_PASSWORD_CHARS {
        PasswordVerdict::TooShort
    } else {
        PasswordVerdict::Fine
    }
}

#[cfg(test)]
mod password_policy_tests {
    use super::*;

    /// The requirement (founder, 2026-10-10): a regular user needs a password
    /// or a PIN of reasonable length; no password is an advanced choice only.
    #[test]
    fn no_password_is_an_advanced_choice_only() {
        assert_eq!(password_verdict("", false), PasswordVerdict::EmptyRefused);
        assert_eq!(password_verdict("", true), PasswordVerdict::EmptyAllowed);
    }

    #[test]
    fn a_pin_of_four_is_enough_and_three_is_not() {
        assert_eq!(password_verdict("123", false), PasswordVerdict::TooShort);
        assert_eq!(password_verdict("123", true), PasswordVerdict::TooShort);
        assert_eq!(password_verdict("1234", false), PasswordVerdict::Fine);
        assert_eq!(password_verdict("héhé", false), PasswordVerdict::Fine, "characters, not bytes");
        assert_eq!(password_verdict("correct horse battery staple", false), PasswordVerdict::Fine);
    }

    #[test]
    fn the_environment_switch_is_off_unless_set() {
        // SAFETY: the test owns this variable; nothing else in the suite reads it.
        unsafe { std::env::remove_var("MARIGOLD_ALLOW_EMPTY_PASSWORD") };
        assert!(!empty_password_allowed_by_env());
        unsafe { std::env::set_var("MARIGOLD_ALLOW_EMPTY_PASSWORD", "1") };
        assert!(empty_password_allowed_by_env());
        unsafe { std::env::remove_var("MARIGOLD_ALLOW_EMPTY_PASSWORD") };
    }
}

pub mod bundle;
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
pub mod selfupdate;
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
                                     your Telegram bot ('telegram link' pairs it)
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
