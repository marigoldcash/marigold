//! The Marigold wallet service.
//!
//! The operations behind every front end the wallet has: the terminal, the
//! desktop app, the Telegram bot and, from PH-M2 on, the phone apps
//! (docs/marigold/MOBILE-PLAN.md). It moves here from `kaspa-cli` in the
//! PH-M1 steps; this crate starts with its error type and nothing else.
//!
//! Nothing in this crate prints, and nothing depends on the terminal library:
//! a message is a `say` line handed to the caller, or a returned value.

pub mod backup;
pub mod bundle;
pub mod error;
pub mod result;
#[cfg(not(target_arch = "wasm32"))]
pub mod telegram;
#[cfg(not(target_arch = "wasm32"))]
pub mod tgbackup;

pub use error::Error;
pub use result::Result;
