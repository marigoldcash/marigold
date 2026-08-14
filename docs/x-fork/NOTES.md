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

## P0.4 — Mine devnet blocks (2026-08-14)

Used the installed `kaspa-miner` (elichai/kaspa-miner, at `~/.cargo/bin/kaspa-miner`)
against a P0.3-style devnet node, rather than `simpa`.

- **Mining address**: `kaspa-cli` can't produce one non-interactively (P0.3's REPL
  finding), and no wallet exists yet (P0.5 territory). Generated a syntactically valid
  address directly with the `kaspa-addresses` crate (`Address::new(Prefix::Devnet,
  Version::PubKey, &payload)` with an arbitrary 32-byte payload — no real keypair needed,
  since nothing will ever spend from it) via a throwaway `cargo run --example`, deleted
  immediately after. Result used for this run:
  `kaspadev:qqxkanesj8e98dq4wmtn3x06tw7p6lklgzssyc7yykrwwj9fpf4uc9j5wmw8s` (no known
  private key — coinbase-only sink address, don't reuse it as a real wallet address).
- **Miner invocation**: `kaspa-miner` has no devnet-specific port default (only
  mainnet=16110/testnet=16210), so pass `--port 16610` explicitly (devnet's GRPC port,
  per P0.3). Also needs `--mine-when-not-synced` on the miner side to pair with the
  node's `--enable-unsynced-mining`:
  ```
  kaspa-miner --mining-address <addr> --kaspad-address 127.0.0.1 --port 16610 --threads 4 --mine-when-not-synced
  ```
- **Result**: miner found 221 blocks in well under a minute (devnet's genesis difficulty
  is trivial by design); node log showed matching `Accepted N blocks ... via submit
  block` lines throughout. Cross-checked via RPC (same gRPC example client as P0.3):
  block count 221, header count 221, virtual DAA score 221, `is_synced: true`.
- **Gotcha**: `pgrep -f <pattern>` / `kill $(pgrep -f ...)` can self-match the wrapping
  shell command that contains the same pattern text (the whole `kill $(pgrep -f "...")`
  string is itself searched), killing the wrong thing or the shell itself. Prefer
  `pgrep -x <exact-binary-name>` (matches `/proc/*/comm`, not the full cmdline) when
  killing a process spawned in this workflow.

## P0.5 — Exercise a wallet on devnet (2026-08-14)

**Substituted RPC-direct testing for the `kaspa-cli` wallet flow** (user-approved
deviation from the plan's literal text). Two reasons: (1) P0.3 already established
`kaspa-cli` is REPL-only and unscriptable; (2) more fundamentally, `kaspa-cli`'s wallet
is backed by `kaspa-wallet-core` — a traditional seed-phrase/key-database layer that
this project's architecture (see FORK-PLAN.md's opening paragraph, and Phase 5-7)
deliberately replaces with a different model (per-note keys, no seed, wallet = key
manager). Proving out a component slated for replacement wasn't worth the time; what
actually matters at this stage is the RPC/consensus path underneath it — mining,
building/signing a real transaction, submitting it, and observing a balance change.

**Tools used** (both already in the workspace, no new code written):
- `kaspa-addresses` — generate a recipient address with no known private key (a valid
  bech32 devnet address is just prefix + version + 32-byte payload + checksum; nothing
  requires the payload to be a real curve point if nobody will ever sign with it).
  Same throwaway-example-then-delete pattern as P0.4.
- `rothschild` (`rothschild/src/main.rs`) — the repo's own transaction-generator tool.
  Run with no `--private-key` on a live devnet node, it generates a real secp256k1
  keypair, prints the address, and exits ("send funds and rerun"). Run again with
  `--private-key <hex> --to-addr <addr> --tps <n>`, it continuously builds, signs, and
  submits real transactions from that key's UTXOs. **Note: it requires a reachable RPC
  endpoint even just to generate a throwaway keypair** — start the node first.

**Addresses used this run** (devnet-only, no value, safe to leave in this file):
- Sender A (rothschild-generated, has known private key
  `5dd09baab7b23e026e93dd6f0dff67aa60d1eb2a727382e23a45f5b7c7a4ff2e`):
  `kaspadev:qpctsq0w4ekmz23pz8v8f85x70pyflx7cxj40fcnftfgrhn3c4kschr2q3lhs`
- Recipient B (keyless, kaspa-addresses-generated):
  `kaspadev:qrfss0tj5lwpz3nmkrj35nuyh8hzxkydctmjccvkevqr265l6synu3m7qjaqv`

**Gotcha — rothschild needs 2× coinbase maturity, not 1×.** Its `is_utxo_spendable`
check (`rothschild/src/main.rs:527-533`) uses `coinbase_maturity * 2` as the required
confirmation depth for coinbase-sourced UTXOs (comment in the source: `TODO: We should
compare with sink blue score in the case of coinbase` — this is upstream's own
acknowledged approximation, not a fork bug). Devnet's `coinbase_maturity` is 1000
blocks (`BPS(10) * COINBASE_MATURITY_SECONDS(100)`), so mining had to reach DAA score
> ~2000 past a UTXO's block before rothschild would spend it — mining only to 1000-1300
left it stuck logging "Has not enough funds" in an infinite retry loop. Mined to DAA
~2200 to clear it comfortably.

**Gotcha — a submitted transaction needs a block mined *after* it to show up in
balance queries.** `get_balance_by_address` reads confirmed UTXO state, not the
mempool. After rothschild reported successful submissions ("Tx rate: 1.1/sec..."),
the recipient's balance was still 0 until a few more blocks were mined to include
those transactions in the accepted chain.

**Verified end-to-end**: mined ~2200 devnet blocks to address A (`kaspa-miner`,
reusing the P0.4 invocation), ran `rothschild --network devnet --private-key <A> --to-addr
<B> --tps 1`, confirmed ~11 successful submissions in its log, mined a few more blocks
to confirm them, then queried B's balance via a temporary extension of the P0.3/P0.4
`kaspa-grpc-simple-client-example` (added a `get_balance_by_address` call + the
`kaspa-addresses` dep, reverted both plus `Cargo.lock` afterward — same
throwaway-edit-then-`git checkout --`-revert pattern as before):
**B's balance: 52,797,283,440 sompi** (~527.97 coins), confirming the send worked.

**Process-management note**: this run used two more short-lived background processes
(`rothschild`, a second `kaspa-miner` invocation) than P0.4 — same stop pattern
(`pgrep -x <name>` + `kill`) worked fine throughout.
