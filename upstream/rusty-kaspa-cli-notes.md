# Upstream notes: rusty-kaspa `cli` crate

Companion to [workflow-terminal/](workflow-terminal/) — findings that live in rusty-kaspa's `cli/` crate rather than the terminal library, discovered while hardening this fork's wallet CLI. To be offered upstream (kaspanet/rusty-kaspa) when this repository is public.

## Open-before-connect leaves the wallet permanently deactivated

`cli/src/cli.rs`, the `Events::SyncState` arm: on reaching synced state with a wallet open, the handler calls `wallet().reload(false, &guard)`. `Wallet::reload`'s own contract (wallet/core `wallet/mod.rs`) says that with `reactivate: false` the *caller* must re-activate accounts — the CLI never does. Net effect: `reload(false)` stops every active account and resets the UTXO processor, so any user who opens their wallet **before** connecting ends up with a permanent `N/A` balance that no amount of connecting fixes. Rarely seen upstream because the public-resolver flow connects before a wallet is typically opened; immediately visible on a local-node workflow. Fix: `reload(true, &guard)` (our commit `2026-09-05`), which restarts accounts and posts fresh discovery/balance events.

## `connect <url>` never persists the server it used

`cli/src/modules/connect.rs` connects to an explicit URL argument without writing `WalletSettings::Server`, while the separate `server` command writes the setting without connecting — so the persisted setting drifts from actual usage and the next session reconnects to the wrong place. Fixed here by persisting the explicitly-given URL on successful connect.

## Notification/prompt interaction

The balance-event handler calls `refresh_prompt()` unconditionally on every `Events::Balance` (`cli/src/cli.rs`); at high BPS with a mining wallet this repaints the prompt many times per second regardless of `mute`, making typing effectively impossible. Throttled to 1/s here. Mostly cosmetic at 1 BPS; structural at 10.
