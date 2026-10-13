# GitHub Actions Workflows

This directory contains the following GitHub Actions workflows:

| Workflow | Purpose | Triggers |
| --- | --- | --- |
| [`ci.yaml`](ci.yaml) — **Tests** | Checks the Rust workspace, runs its test suites and lints, checks `no_std` and WASM32 targets, runs WASM tests, builds the WASM SDK, and builds Linux release binaries. | Automatically on every push and pull request. |
| [`musl-toolchain.yaml`](musl-toolchain.yaml) — **Build musl toolchain** | Builds and packages the x86_64 musl cross-compilation toolchain, then publishes it as the `musl-toolchain-v1` GitHub release. | Manually via **Run workflow**, or automatically when `musl-toolchain/preset.sh` changes on `master`. |
| [`binaries.yaml`](binaries.yaml) — **Wallet binaries** | Builds the command-line wallet for Linux, Windows, and macOS, including macOS universal binaries and an additional trustee kit for Linux ARM64. Uploads the builds as workflow artifacts and can optionally attach them to a `marigold-wallet` release. | Manually via **Run workflow**. Set the optional `release_tag` input to publish to that release; leave it empty to keep the files as workflow artifacts. |
| [`gui-binaries.yaml`](gui-binaries.yaml) — **Desktop wallet binaries** | Builds the desktop wallet and platform installers for Linux, macOS, and Windows. Uploads the builds as workflow artifacts and can optionally attach them to a `marigold-wallet` release. | Manually via **Run workflow**. Set the optional `release_tag` input to publish to that release; leave it empty to keep the files as workflow artifacts. |
| [`mobile-core.yaml`](mobile-core.yaml) — **Mobile core** | Builds the wallet core as an Android `.so` (phone and emulator) and an iOS XCFramework (iPhone and simulator), through `scripts/mobile-core.sh`, and keeps both as workflow artifacts. | Manually via **Run workflow**, and on every push to a `mobile/` branch. |
| [`deploy.yaml`](deploy.yaml) — **Build and upload assets** | Manually builds the legacy command-line binaries for Linux, Windows, and macOS, as well as the WASM32 SDK, and uploads the resulting assets to a GitHub release. | Manually via **Run workflow**; it is not triggered automatically by a release. |
