# The phone wallet: a step-by-step plan (iOS and Android)

Decided 2026-10-12 (DECISIONS "The phone wallet: one Rust core, a React Native screen"). This is the executable plan: steps `M0`–`M12`, each with what to do, how it is verified, and when it is done, written so that one agent can take them one at a time in order. Tick a step only when its verify line passes. Keep the basic look for now; the screens are tweaked later, the structure is not.

## Rules that hold for every step

- **One wallet implementation.** Every wallet operation on the phone goes through the Rust service (`WalletService`, today in `cli/src/serve.rs`, moved to its own crate in M1). No JavaScript, Swift or Kotlin ever signs, derives a key, builds a transaction, reads a note file or parses a payment code; the phone side only calls the service and shows what it answers. Where the service lacks an operation the phone needs, the operation is added to the service, tested there, and then called.
- **Test against the public testnet.** The phone talks to `wss://rpc1.marigold.cash` (testnet-10) exactly as the terminal's `connect public` does; faucet notes come from the faucet. Nothing is mocked that can be real.
- **Every step leaves the desktop and the Telegram bot working.** They use the same service; after each step `scripts/build-gui.sh` builds and the terminal drivers pass.
- **Words a grandmother understands** on every screen and in every message (DECISIONS "Grandma safe"); technical detail only behind a settings switch.
- **No secret in the repository.** Signing keys, store credentials and the updater key live in the organisation's accounts and the workflow secrets; the plan names where, never what.
- **Builds:** Rust for the phone targets builds on the macOS runner the desktop already uses (`gui-binaries.yaml`) and on Ubuntu for Android; pve3 is not involved. Local iOS work needs a Mac.

## Architecture in one paragraph

The Rust wallet core (`wallet/core`, with the light wallet in `light.rs`) is compiled natively for `aarch64-apple-ios`, the iOS simulator and `aarch64-linux-android` (plus `x86_64-linux-android` for the emulator), wrapped by UniFFI, which generates the Swift and Kotlin bindings, and exposed to React Native through one Expo native module, `@marigold/core`. The app is TypeScript under Expo, with a development client (not Expo Go, which cannot load native modules). The service runs inside the app process; the light copy of the pool lives in the app's own storage; keys stay in the app's sandbox with the password in the platform keystore behind biometrics. The node is a public computer by default and configurable.

## The steps

### M0 — Accounts and identifiers (the founder's, done outside the code)

Do: the Apple Developer account enrolled as the organisation Marigold.cash Ltd (D-U-N-S arrives by itself for a UK company); the Google Play developer account; the bundle identifier `cash.marigold.wallet` reserved on both; Charly given an App Store Connect role and Play Console access.

Verify: both consoles show the organisation and the identifier. Done when: a TestFlight and a Play internal-testing track exist, even empty.

### M1 — The service in its own crate

Do: move `WalletService`, `Session`, `SessionOptions`, `open_session` and what they need (`bundle.rs`, `tgbackup.rs`, `backup.rs`, the miner host interface, the error type) out of `cli/src/` into a new crate `wallet/service` (`kaspa-wallet-service`), without the terminal (`workflow-terminal`) and without the embedded node as a hard dependency: the node inside the process stays behind a feature `embedded-node`, as in the cli today. The cli and the desktop (`gui/src/main.rs`) import the crate; `cli/src/serve.rs` becomes the `serve` subcommand only. `say` stays a plain callback; nothing prints.

Verify: `cargo build --release -p kaspa-cli -p marigold-gui`; `NO_BUMP=1 scripts/build-gui.sh`; the terminal drivers `tlight.py`, `tpassword2.py` and the Telegram backup driver pass unchanged; `cargo tree -p kaspa-wallet-service` shows no `workflow-terminal`. Done when: the desktop opens a wallet through the new crate and CI is green.

### M2 — The core builds for the phones

Do: add the four phone targets to `rust-toolchain.toml` and to a new workflow `mobile-core.yaml` (macOS runner for the Apple targets, Ubuntu with `cargo-ndk` for Android). Build `kaspa-wallet-service` as a static library (`crate-type = ["staticlib", "cdylib", "rlib"]`) for each target. Fix what does not compile: expect `getrandom` features, `mio`/`tokio` features, and C dependencies (`secp256k1`, `rocksdb` must not be pulled: confirm with `cargo tree`). Produce an XCFramework for Apple and a `jniLibs` tree for Android as workflow artifacts.

Verify: the workflow is green on all four targets and uploads the two artifacts; `nm` on the iOS library shows the symbols. Done when: a tag's workflow run produces downloadable libraries.

### M3 — The UniFFI interface

Do: in `wallet/service/src/ffi.rs`, behind a feature `ffi`, export with UniFFI's proc macros an interface that mirrors the desktop's commands one to one (the thirty-three `#[tauri::command]`s in `gui/src/main.rs` are the list): `probe`, `list_wallets`, `create_wallet`, `restore_from_words`, `open`, `close`, `status`, `balance`, `receive`, `request`, `pay_preview`, `pay`, `history`, `recovery_words`, `backup_status`, `backup_now`, `backup_cover`, `change_password`, `light_status`, plus an event callback interface (`say` lines, notes changed, connection state). Async functions are exported as UniFFI async. Errors cross as one `WalletError` enum with the user-facing text the terminal already produces.

Verify: `cargo test -p kaspa-wallet-service --features ffi` includes a test that creates a wallet in a temp dir through the ffi layer, opens it, reads the status, and closes it; `uniffi-bindgen` generates Swift and Kotlin without warnings; on the macOS runner a ten-line Swift program links the XCFramework and prints `version()`. Done when: those three pass in CI.

### M4 — The Expo app and the native module

Do: `mobile/` at the repository root: an Expo app (TypeScript, Expo SDK current, development client, EAS config checked in without credentials) and a local Expo module `mobile/modules/marigold-core` whose Swift and Kotlin sides wrap the UniFFI bindings from M3 and whose TypeScript side exposes them as typed functions and an event emitter. The iOS podspec pulls the XCFramework; the Android module pulls the `jniLibs`. The first screen shows the core's version.

Verify: `npm test` in `mobile/` (Jest) covers the TypeScript wrapper against a mocked native module; on CI a development client builds for the iOS simulator and for an Android emulator and a Maestro flow launches it and reads the version on screen. Done when: both simulator builds show the version from Rust.

### M5 — Storage, the password and biometrics

Do: the wallet folder is the app's own documents directory (iOS: excluded from iCloud backup by attribute, since the sealed backup of M8 is the backup; Android: internal storage); files owner-only as on the desktop. The password is the PIN the policy already requires (at least four characters); it is kept in the Keychain / Keystore (`expo-secure-store`) and released by `expo-local-authentication` (Face ID, fingerprint), with the PIN typed as the fallback. The password never crosses to JavaScript in the clear longer than the call that opens the wallet.

Verify: Maestro: create a wallet, kill the app, reopen with biometrics (simulator-enrolled), the balance screen appears; a Rust test asserts the created files' modes; a wrong PIN shows "That is not this wallet's password." Done when: the flow passes on both platforms.

### M6 — Connect, and the light copy

Do: on open, the app connects to the public computer (default `wss://rpc1.marigold.cash` for testnet-10, changeable in settings; a `ws://` address for one's own node) and the service starts the light copy as the desktop does; the `say` lines ("Connecting…", "Taking your own copy of the notes…") show on the open screen. On foreground after background the service reconnects and restarts from the latest anchor; on network loss the status says so in plain words. No background execution is requested; the app says incoming payments show when it is open.

Verify: Maestro against testnet: open, the status reads "a public computer, with your own copy of the notes" within thirty seconds; background the app ten minutes (simulator clock), foreground, status is current within thirty seconds; airplane mode gives the plain-words message and recovery when it ends. Done when: those pass on both platforms.

### M7 — The screens, basic

Do: seven screens, plain components, no styling pass: Open or create (list of wallets, create with the words shown once and two asked back as the terminal does, restore from words); Home (balance in notes and ledger, the last movements, connection status line); Receive (a request code with amount, as text and QR); Pay (paste or scan a code, preview with amount and fee, confirm, result); History; Backup (status, back up now, the words again behind biometrics); Settings (node address, PIN change, advanced switch). Every text from the service, not retyped.

Verify: Maestro flows: a faucet payment to a request code lands and shows on Home; a payment to a terminal wallet's request code goes out and the terminal receives it; history shows both; restore from words on a fresh install brings the ledger back. Done when: the four flows pass on both platforms.

### M8 — Backups to the cloud

Do: the sealed backup bundle (the folder backup of `backup.rs`, sealed to the 24 words) written on a schedule to the app's iCloud Drive container on iOS and to Google Drive's app folder on Android, and read back on restore: restore asks for the words, finds the bundle, brings the notes back. The toggle is on by default and explained in one sentence.

Verify: Maestro: wallet with notes, backup runs, app deleted and reinstalled, restore from words plus the cloud bundle, notes back; a Rust test covers the bundle round trip through the service. Done when: the round trip passes on both platforms with real iCloud and Drive test accounts.

### M9 — Camera and sharing

Do: scan a payment or request code with the camera (`expo-camera`), paste from the clipboard, share a request code through the system share sheet, and open `marigoldpay:` / `marigoldreq:` links handed to the app.

Verify: Maestro with a code rendered on screen and scanned from a second device or an injected image; a link opened from a message opens the pay screen prefilled. Done when: both pass.

### M10 — Release builds and testers

Do: `mobile-release.yaml`: signed builds through EAS Build or the runners, with the certificates and keys in the organisation's accounts and workflow secrets; upload to TestFlight and the Play internal track on a tag `mobile-vX`; the testers' group invited; version number from the workspace version.

Verify: a tester installs from TestFlight and from the Play internal track and completes the M7 flows. Done when: both tracks carry a build the testers use.

### M11 — Words and reach

Do: the grandma-safe pass over every string, screen reader labels, large text, dark mode following the system; the terminal's exact phrasings reused where the same thing is said.

Verify: a checklist in the pull request; two testers from the group who are not developers complete the four flows unaided. Done when: they do.

### M11b — Knowing when a payment arrived while the app was closed (after M7, before the stores)

Do, in this order, each its own pull request. (1) A background check: the app asks the operating system for periodic background time (iOS background app refresh, Android WorkManager), takes the light copy's diffs since its last look, checks its own request keys and its ledger address locally, and raises a local notification "A payment arrived. Open the wallet." Nothing leaves the phone; the delay is the operating system's, fifteen minutes to hours. (2) A payer-side ping, opt in: a request code may carry a one-time notify token minted by a small relay we run; a paying wallet that honours it tells the relay "token done" once its transaction is accepted, and the relay pushes the same content-free line to the device behind the token. The relay learns that a device received a payment at a time, never which note or how much; tokens are single use; the wallet trusts nothing in the push and reads its own copy on open. (3) For people with a home machine, the wallet service there, which watches anyway, pings through the same relay. Never (4): registering keys with a public node to be watched.

Verify: (1) on a simulator with background refresh forced, a faucet payment to a request issued before the app was closed raises the local notification within the next refresh; (2) a terminal wallet paying a request with a token produces a push on the device within seconds, and the relay's log holds no key, serial or amount; (3) the Telegram-linked machine's payment produces the push. Done when: the three pass and the FAQ says what the relay learns.

### M12 — The stores

Do: App Store listing and review (privacy manifest, the non-custodial wallet declaration, screenshots), Play listing (data safety form, financial-services declaration), a page on marigold.cash for the app, the FAQ entry, and the release post.

Verify: both apps live, installable from the stores. Done when: the links are on the website.

## Out of scope, on purpose

A public node watching your request keys to notify you (it would learn which keys are yours, the thing the light wallet exists to avoid); mining on the phone (forbidden by Apple, pointless anyway), the Telegram bot's features (the bot stays as the remote for a home machine), assets and lanes (not active on any network), and any visual design beyond the basic components until the structure stands.
