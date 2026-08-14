# Orientation notes

Running log of build/test findings for future sessions. Append to this file as later
Phase 0 steps run (P0.6 formalizes it further).

## P0.1 — Build

- Platform: Linux (Debian/LMDE), Rust 1.97.1, protoc 3.21.12, clang 19. All prerequisites
  were already installed on this machine — no `apt install` needed.
- `cargo build --release --bin kaspad` — clean, 10m03s, zero errors.
- Note: `kaspad --version` prints the version string but exits with code 1 (harmless
  quirk of upstream's arg parsing — not fork-caused; don't rely on its exit code in scripts).

## P0.2 — Test suite baseline (2026-08-14)

`cargo test --release` (cargo-nextest not installed; plain `cargo test` used instead,
plan allows either). Full log saved at test time in the session scratchpad.

- **Result: 0 failed, 0 compile errors**, across unit + integration + doctests.
- 26 tests ignored workspace-wide (pre-existing, not fork-caused) — recorded so future
  red/green diffs are attributable. Notable ones:
  - `processes::coinbase::tests::verify_crescendo_emission_schedule` — marked
    "ignored, long"; relevant later for P3.2 (emission table changes) — run it
    explicitly (`cargo test -- --ignored`) after touching coinbase.rs.
  - `daemon_integration_tests::daemon_toccata_activation_log_file_test` — ignored.
  - `subscription::context::tests::test_hash_map_u32_u16_size` — "ignored, measuring
    consumed memory" (perf/size assertion, not correctness).
  - `config::bps::tests::gen_ghostdag_table`, `cache::tests::print_cache_entry_byte_sizes`
    — generator/printer tests, ignored by design.
  - Remainder are doctests marked `ignored` (example code not meant to run standalone)
    and a couple of hasher/cache micro-tests.
- **Baseline established: any test that goes red after this point in the fork is ours
  to fix** (Ground rule / P0.2 verify instruction).

## P0.3 — Devnet node (2026-08-14)

Command from the plan:
```
cargo run --release --bin kaspad -- --devnet --enable-unsynced-mining --rpclisten-borsh=127.0.0.1 --utxoindex
```
- `cargo run` partially recompiled `kaspad`/`kaspa-wrpc-server`/`kaspa-build-info` (1m45s)
  even though `cargo build --release --bin kaspad` had just succeeded in P0.1 — different
  codegen flags between `cargo build` and `cargo run` invalidated those 3 crates' cache.
  Not a problem, just don't be surprised by it.
- Default devnet app dir: `~/.rusty-kaspa/kaspa-devnet/` (datadir + logs subdirs). Not yet
  rebranded (P2.7 will change this to a Marigold-named dir).
- Devnet default ports actually bound: GRPC `127.0.0.1:16610`, P2P `0.0.0.0:16611`,
  WRPC(borsh) `127.0.0.1:17610`. (Mainnet defaults, for reference when writing P2.2, are
  the `161*0` family — e.g. GRPC 16110 — which is what upstream examples hardcode.)
- UPnP port-mapping attempt fails harmlessly in this dev environment ("Resource
  temporarily unavailable") — expected, not an error to chase.
- **`kaspa-cli` is a full interactive REPL (crossterm raw-mode terminal), not a
  one-shot command tool** — `cli/src/main.rs` ignores argv entirely and always drops into
  the `$` prompt. It cannot be driven non-interactively (crossterm needs a real TTY;
  under a piped/`/dev/null` stdin it fails with `Cli error No such device or address (os
  error 6)`). For scripted/CI-style RPC checks, use a gRPC client instead.
- Building `kaspa-cli` from scratch (wallet-core, terminal, wrpc client, etc.) takes
  ~5 minutes — much bigger dependency tree than the daemon alone.
- **Verified RPC answers** using the existing example crate
  `rpc/grpc/examples/simple_client` (`kaspa-grpc-simple-client-example`), pointed at the
  devnet's actual GRPC port (16610; the example hardcodes mainnet's 16110, so the URL
  needs adjusting when pointing it at devnet — do this as a throwaway edit + `git
  checkout --` revert, not a committed change). Output confirmed: server version 2.0.1,
  network `devnet`, UTXO indexing on, genesis-only DAG (block/header count 0, DAA score
  0) — expected for a node with no mined blocks yet.
