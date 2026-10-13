# The phone wallet: a step-by-step plan (iOS and Android)

Decided 2026-10-12 (DECISIONS "The phone wallet: one Rust core, a React Native screen"). This is the executable plan: steps `M0`–`M12`, each with what to do, how it is verified, and when it is done, written so that one agent can take them one at a time in order. Tick a step only when its verify line passes. Keep the basic look for now; the screens are tweaked later, the structure is not.

## Rules that hold for every step

- **One wallet implementation.** Every wallet operation on the phone goes through the Rust service (`WalletService`, today in `cli/src/serve.rs`, moved to its own crate in M1). No JavaScript, Swift or Kotlin ever signs, derives a key, builds a transaction, reads a note file or parses a payment code; the phone side only calls the service and shows what it answers. Where the service lacks an operation the phone needs, the operation is added to the service, tested there, and then called.
- **Test against the public testnet.** The phone talks to `wss://rpc1.marigold.cash` (testnet-10) exactly as the terminal's `connect public` does; faucet notes come from the faucet. Nothing is mocked that can be real.
- **Every step leaves the desktop and the Telegram bot working.** They use the same service; after each step `scripts/build-gui.sh` builds and the terminal drivers pass.
- **Words a grandmother understands** on every screen and in every message (DECISIONS "Grandma safe"); technical detail only behind a settings switch.
- **No secret in the repository.** Signing keys, store credentials and the updater key live in the organisation's accounts and the workflow secrets; the plan names where, never what.
- **Builds:** Rust for the phone targets builds on the macOS runner the desktop already uses (`gui-binaries.yaml`) and on Ubuntu for Android; pve3 is not involved. Local iOS work needs a Mac.

## How an agent works a step

Read [REPOS.md](REPOS.md) first: it says which repository a piece of work belongs in and how a session behaves in a checkout. One step, or one lettered sub-step, is one branch `mobile/M3a` and so on, in your own worktree (`git worktree add ../mobile-M3a -b mobile/M3a origin/main`), pushed; it reaches `main` by fast-forward when its box is ticked, in the same session. Read the step, the rules above, and the files it names before touching anything. Do only what the step says, and before every commit read `git show --stat HEAD` and every hunk: another session's edit that is lying in a shared tree is how a foreign change gets into a commit, and it happened on M1a; something missing from the plan that the step needs is added to the plan in the same pull request, as a new lettered sub-step, not done silently. Run the verify line and paste its output into the pull request; CI must be green. Tick the box in this file in that pull request. If the verify line cannot pass, say so in the pull request with the reason and stop; do not loosen the verify line. Build in a directory of your own on the build host, `MARIGOLD_BUILD_DIR=/opt/marigold-build/remote-build-<step> scripts/remote-build.sh ...` (and the same variable for `scripts/build-gui.sh`): the default directory on pve3 is shared, two syncs from two trees interleave, and a build then fails on code that is not yours. Versions of Expo, UniFFI and the phone toolchains are the current stable ones at the time of the first mobile commit, pinned in `mobile/package.json` and `wallet/service/Cargo.toml`, and bumped only in a commit of their own.

## Architecture in one paragraph

The Rust wallet core (`wallet/core`, with the light wallet in `light.rs`, and the service in `wallet/service`) is built in this repository and compiled natively for `aarch64-apple-ios`, the iOS simulator and `aarch64-linux-android` (plus `x86_64-linux-android` for the emulator), wrapped by UniFFI, which generates the Swift and Kotlin bindings, and exposed to React Native through one Expo native module, `@marigold/core`, which lives with the app in `marigoldcash/expo-mobile` and takes the compiled core from the release as a download. The app is TypeScript under Expo, with a development client (not Expo Go, which cannot load native modules). The service runs inside the app process; the light copy of the pool lives in the app's own storage; keys stay in the app's sandbox with the password in the platform keystore behind biometrics. The node is a public computer by default and configurable.

## The steps

### [ ] M0 — Accounts and identifiers (the founder's, done outside the code)

Do: the Apple Developer account enrolled as the organisation Marigold.cash Ltd (D-U-N-S arrives by itself for a UK company); the Google Play developer account; the bundle identifier `cash.marigold.wallet` reserved on both; Charly given an App Store Connect role and Play Console access.

Verify: both consoles show the organisation and the identifier. Done when: a TestFlight and a Play internal-testing track exist, even empty.

### M1 — The service in its own crate

The service today is `cli/src/serve.rs` (1,143 lines) and it reaches into `cli/src/bundle.rs`, `tgbackup.rs`, `backup.rs`, `miner.rs`, `embedded.rs`, `telegram.rs`, `error.rs` and `ui.rs`. It moves to a new crate `wallet/service` (`kaspa-wallet-service`) in four sub-steps, each leaving the cli, the desktop and the bot building and the drivers passing.

- [ ] **M1a — The crate, empty but wired.** Create `wallet/service` with `kaspa-wallet-service` in the workspace, depending on `kaspa-wallet-core` and the rpc clients, not on `kaspa-cli` and never on `workflow-terminal`. Move `cli/src/error.rs`'s service-facing error into it (or a new `WalletError`), re-exported from the cli so nothing else changes. Verify: `cargo build --release --locked -p kaspa-cli --features embedded-node` and, because the desktop is its own workspace that `-p` cannot reach, `NO_BUMP=1 scripts/build-gui.sh`; `cargo tree -p kaspa-wallet-service | grep -c workflow-terminal` prints 0; `cargo test -p kaspa-wallet-service`, `cargo clippy --tests -- -D warnings` and `cargo fmt --check` clean on the two crates. Status 2026-10-13: the crate is on branch `mobile/M1a` (pull request 36) and passes every check; the box stays open until `main` builds, because main's own `kaspa-cli` has not compiled since 59a0cc65 (its stratum code imports `kaspa_hashes` and the crate's manifest does not declare it, CI red), and the verify cannot pass on a base that does not build. The checks above were run with that one line applied locally and uncommitted.
- [ ] **M1b — Backups move.** Move `bundle.rs`, `backup.rs` and `tgbackup.rs` into the crate; what they printed through the terminal goes through the `say` callback or a returned string. The cli calls them from the crate. Verify: the folder backup driver and `tbundle.py` (Telegram backup) pass; `cargo build` of cli and gui.
- [ ] **M1c — The service moves.** Move `WalletService`, `Session`, `SessionOptions`, `open_session` and the miner host interface (`miner.rs` stays in the cli; the service sees a trait). The embedded node stays behind a feature `embedded-node` on the crate, as on the cli. `cli/src/serve.rs` keeps only the `serve` subcommand and imports the crate; `gui/src/main.rs` imports the crate. Verify: `NO_BUMP=1 scripts/build-gui.sh`; `tlight.py` against the public node; `tpassword2.py`; the desktop opens a wallet (the founder confirms once).
- [ ] **M1d — Nothing prints.** `grep -rn "tprintln\|println" wallet/service/src` prints nothing; every message is a `say` line or a return value. Verify: that grep, and CI green.

### [ ] M2 — The core builds for the phones

Do: add the four phone targets to `rust-toolchain.toml` and to a new workflow `mobile-core.yaml` (macOS runner for the Apple targets, Ubuntu with `cargo-ndk` for Android). Build `kaspa-wallet-service` as a static library (`crate-type = ["staticlib", "cdylib", "rlib"]`) for each target. Fix what does not compile: expect `getrandom` features, `mio`/`tokio` features, and C dependencies (`secp256k1`, `rocksdb` must not be pulled: confirm with `cargo tree`). Produce an XCFramework for Apple and a `jniLibs` tree for Android as workflow artifacts.

Verify: the workflow is green on all four targets and uploads the two artifacts; `nm` on the iOS library shows the symbols. Done when: a tag's workflow run produces downloadable libraries.

### M3 — The UniFFI interface, and the core as a release asset

Decided 2026-10-13 (REPOS.md, DECISIONS "The phone wallet: the Rust core stays here, the app lives in expo-mobile"): the Rust core stays in this repository on `mobile/*` branches, and this repository's release builds it for the phones and attaches it to the release; the app in `marigoldcash/expo-mobile` takes it as a download and never compiles Rust.

Do: in `wallet/service/src/ffi.rs`, behind a feature `ffi`, export with UniFFI's proc macros an interface that mirrors the desktop's commands one to one (the thirty-three `#[tauri::command]`s in `gui/src/main.rs` are the list): `probe`, `list_wallets`, `create_wallet`, `restore_from_words`, `open`, `close`, `status`, `balance`, `receive`, `request`, `pay_preview`, `pay`, `history`, `recovery_words`, `backup_status`, `backup_now`, `backup_cover`, `change_password`, `light_status`, plus an event callback interface (`say` lines, notes changed, connection state). Async functions are exported as UniFFI async. Errors cross as one `WalletError` enum with the user-facing text the terminal already produces.

- [ ] **M3a — The interface, tested in Rust.** The `ffi` feature, the exported functions and the error enum; `cargo test -p kaspa-wallet-service --features ffi` includes a test that creates a wallet in a temp dir through the ffi layer, opens it, reads the status, and closes it. Verify: that test passes.
- [ ] **M3b — The bindings, generated here.** `scripts/mobile-bindings.sh` runs `uniffi-bindgen` and writes the Swift and Kotlin bindings into a directory the release step packages; they are not checked in, they are produced by the build. Verify: the script runs with no warnings, and a second run produces byte-identical files (the workflow compares checksums).
- [ ] **M3c — Swift links it.** On the macOS runner a ten-line Swift program links the XCFramework from M2 with the Swift binding from M3b and prints `version()`. Verify: the workflow step prints the version.
- [ ] **M3d — The release carries the core.** The release workflow this repository already uses to attach binaries to the `marigold-wallet` release of a tag (`binaries.yaml`, started by `scripts/release.sh` with the tag) gains two jobs, on the runners it already has: the macOS job builds `kaspa-wallet-service` for the iOS device and simulator targets, assembles `marigold-core-<version>-ios.zip` (the XCFramework and the Swift bindings), and the Linux job builds the four Android targets with `cargo-ndk`, assembling `marigold-core-<version>-android.zip` (the `jniLibs` tree and the Kotlin bindings). Both are attached to the release with a `.sha256` file each and added to the release's `SHA256SUMS-core.txt`. The jobs build from the tag's own commit and fail the release's workflow if the library does not link. Verify: a tag's workflow run attaches the two zips and the checksum list; the zips downloaded from the release match their checksums; `unzip -l` shows the XCFramework with both slices, the three Android ABIs plus the emulator's, and the bindings. Nothing in these jobs signs: the app signs what it ships.

Done when: M3a to M3d pass, and a release candidate tag carries the two assets.

### M4 — The Expo app and the native module, in expo-mobile

From M4 on the work is in `marigoldcash/expo-mobile`, one Expo project for Android and iOS, in a clone of your own of that repository, not in this one. When M4 is reached, copy M4 and everything after it (M5 to M12) into that repository as its `PLAN.md`; from then on that copy is the plan and the session that owns it amends it, and this file keeps only a pointer. A path written `mobile/` in a later step means the root of that repository, and the release workflow of M10 lives there. No step in that repository installs or runs a Rust toolchain.

Do: an Expo app (TypeScript, Expo SDK current, development client, EAS config checked in without credentials) and a local Expo module `modules/marigold-core` whose Swift and Kotlin sides wrap the UniFFI bindings that the core's release assets carry (M3d) and whose TypeScript side exposes them as typed functions and an event emitter. The first screen shows the core's version.

- [ ] **M4a — The app skeleton.** The Expo app, TypeScript strict, the development client configuration, Jest, Maestro, and `npm test` passing on an empty wrapper. Verify: `npm test`; `npx expo doctor` clean.
- [ ] **M4b — The core, fetched by version and checksum.** One file, `marigold-core.json`, names the `marigold` release tag and, for each of the two assets of M3d, its download URL and SHA-256. `scripts/fetch-core.mjs` reads it, downloads the two zips, verifies each checksum before unpacking, and unpacks the XCFramework and bindings for iOS and the `jniLibs` and bindings for Android into the native module's folders (not checked in). The environment variable `MARIGOLD_CORE_DIR` points the script at a local marigold build's output (the same two zips, or their unpacked folders) instead of downloading, for working on the core and the app together. Verify: a Jest test of the script against a local fixture covers the happy path, a wrong checksum (refused, nothing unpacked), a missing asset, and the local override; against the real release candidate of M3d the fetch succeeds and unpacks both. The script never invokes `cargo`, `rustc` or `rustup`, and a test greps for it.
- [ ] **M4c — The native module, iOS.** The Expo module's Swift side wraps the generated binding; the podspec pulls the XCFramework the fetch unpacked; the first screen shows `version()`. Verify: a development client builds for the iOS simulator on the macOS runner and a Maestro flow reads the version on screen.
- [ ] **M4d — The native module, Android.** The Kotlin side wraps the generated binding; the module pulls the `jniLibs`. Verify: the same on an Android emulator on the Ubuntu runner.
- [ ] **M4e — The TypeScript wrapper.** Typed functions for every exported operation and an event emitter for the callback interface; Jest tests against a mocked native module. Verify: `npm test` covers every function once.

Done when: both simulator builds show the version from Rust, and neither build job installed a Rust toolchain.

### [ ] M5 — Storage, the password and biometrics

Do: the wallet folder is the app's own documents directory (iOS: excluded from iCloud backup by attribute, since the sealed backup of M8 is the backup; Android: internal storage); files owner-only as on the desktop. The password is the PIN the policy already requires (at least four characters); it is kept in the Keychain / Keystore (`expo-secure-store`) and released by `expo-local-authentication` (Face ID, fingerprint), with the PIN typed as the fallback. The password never crosses to JavaScript in the clear longer than the call that opens the wallet.

Verify: Maestro: create a wallet, kill the app, reopen with biometrics (simulator-enrolled), the balance screen appears; a Rust test asserts the created files' modes; a wrong PIN shows "That is not this wallet's password." Done when: the flow passes on both platforms.

### M6 — Connect, and the light copy

Do: on open, the app connects to the public computer (default `wss://rpc1.marigold.cash` for testnet-10, changeable in settings; a `ws://` address for one's own node) and the service starts the light copy as the desktop does; the `say` lines ("Connecting…", "Taking your own copy of the notes…") show on the open screen. On foreground after background the service reconnects and restarts from the latest anchor; on network loss the status says so in plain words. No background execution is requested; the app says incoming payments show when it is open.

- [ ] **M6a — Connect and the light copy.** Verify: Maestro against testnet: open, the status reads "a public computer, with your own copy of the notes" within thirty seconds, on both platforms.
- [ ] **M6b — Background and back.** Verify: background the app ten minutes (simulator clock), foreground, the status is current within thirty seconds; airplane mode gives the plain-words message and recovery when it ends, on both platforms.

Done when: both pass.

### M7 — The screens, basic

Do: seven screens, plain components, no styling pass: Open or create (list of wallets, create with the words shown once and two asked back as the terminal does, restore from words); Home (balance in notes and ledger, the last movements, connection status line); Receive (a request code with amount, as text and QR); Pay (paste or scan a code, preview with amount and fee, confirm, result); History; Backup (status, back up now, the words again behind biometrics); Settings (node address, PIN change, advanced switch). Every text from the service, not retyped.

One screen per sub-step, each with its own Maestro flow, in this order because each needs the one before:

- [ ] **M7a — Open or create.** List, create with the words shown once and two asked back, open with the PIN. Verify: create, close, open; the balance screen appears.
- [ ] **M7b — Home.** Balance in notes and ledger, the connection status line, the last movements. Verify: the faucet's figures match the terminal's `balance` for the same wallet file.
- [ ] **M7c — Receive.** A request code with an amount, as text and QR. Verify: a faucet payment to the request lands and shows on Home.
- [ ] **M7d — Pay.** Paste a code, preview with amount and fee, confirm, result. Verify: a payment to a terminal wallet's request goes out and the terminal receives it.
- [ ] **M7e — History.** Verify: both movements above show, with the terminal's wording.
- [ ] **M7f — Backup.** Status, back up now, the words again behind biometrics. Verify: the words shown equal the ones from creation.
- [ ] **M7g — Settings and restore.** Node address, PIN change, the advanced switch; restore from words on the open screen. Verify: restore from words on a fresh install brings the ledger back; the PIN change is refused with the wrong current PIN.

Done when: all seven pass on both platforms.

### [ ] M8 — Backups to the cloud

Do: the sealed backup bundle (the folder backup of `backup.rs`, sealed to the 24 words) written on a schedule to the app's iCloud Drive container on iOS and to Google Drive's app folder on Android, and read back on restore: restore asks for the words, finds the bundle, brings the notes back. The toggle is on by default and explained in one sentence.

Verify: Maestro: wallet with notes, backup runs, app deleted and reinstalled, restore from words plus the cloud bundle, notes back; a Rust test covers the bundle round trip through the service. Done when: the round trip passes on both platforms with real iCloud and Drive test accounts.

### [ ] M9 — Camera and sharing

Do: scan a payment or request code with the camera (`expo-camera`), paste from the clipboard, share a request code through the system share sheet, and open `marigoldpay:` / `marigoldreq:` links handed to the app.

Verify: Maestro with a code rendered on screen and scanned from a second device or an injected image; a link opened from a message opens the pay screen prefilled. Done when: both pass.

### [ ] M10 — Release builds and testers

Do: `mobile-release.yaml`: signed builds through EAS Build or the runners, with the certificates and keys in the organisation's accounts and workflow secrets; upload to TestFlight and the Play internal track on a tag `mobile-vX`; the testers' group invited; version number from the workspace version.

Verify: a tester installs from TestFlight and from the Play internal track and completes the M7 flows. Done when: both tracks carry a build the testers use.

### [ ] M11 — Words and reach

Do: the grandma-safe pass over every string, screen reader labels, large text, dark mode following the system; the terminal's exact phrasings reused where the same thing is said.

Verify: a checklist in the pull request; two testers from the group who are not developers complete the four flows unaided. Done when: they do.

### [ ] M11b — Knowing when a payment arrived while the app was closed (after M7, before the stores)

Do, in this order, each its own pull request. (1) A background check: the app asks the operating system for periodic background time (iOS background app refresh, Android WorkManager), takes the light copy's diffs since its last look, checks its own request keys and its ledger address locally, and raises a local notification "A payment arrived. Open the wallet." Nothing leaves the phone; the delay is the operating system's, fifteen minutes to hours. (2) A payer-side ping, opt in: a request code may carry a one-time notify token minted by a small relay we run; a paying wallet that honours it tells the relay "token done" once its transaction is accepted, and the relay pushes the same content-free line to the device behind the token. The relay learns that a device received a payment at a time, never which note or how much; tokens are single use; the wallet trusts nothing in the push and reads its own copy on open. (3) For people with a home machine, the wallet service there, which watches anyway, pings through the same relay. Never (4): registering keys with a public node to be watched.

Verify: (1) on a simulator with background refresh forced, a faucet payment to a request issued before the app was closed raises the local notification within the next refresh; (2) a terminal wallet paying a request with a token produces a push on the device within seconds, and the relay's log holds no key, serial or amount; (3) the Telegram-linked machine's payment produces the push. Done when: the three pass and the FAQ says what the relay learns.

### [ ] M12 — The stores

Do: App Store listing and review (privacy manifest, the non-custodial wallet declaration, screenshots), Play listing (data safety form, financial-services declaration), a page on marigold.cash for the app, the FAQ entry, and the release post.

Verify: both apps live, installable from the stores. Done when: the links are on the website.

## Out of scope, on purpose

A public node watching your request keys to notify you (it would learn which keys are yours, the thing the light wallet exists to avoid); mining on the phone (forbidden by Apple, pointless anyway), the Telegram bot's features (the bot stays as the remote for a home machine), assets and lanes (not active on any network), and any visual design beyond the basic components until the structure stands.
