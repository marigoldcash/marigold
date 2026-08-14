# Orientation notes

Start here. This file has two parts: a **quick-start** block below with the exact
commands that work on this machine today, and a **detailed step log** (P0.1 onward)
with the reasoning, gotchas, and dead ends behind them — read the log when a quick-start
command surprises you. Append to the log as later steps run; keep the quick-start block
in sync when commands change (e.g. once P2 rebrands ports/prefixes/app-dir).

## Environment

- Platform: Linux (Debian/LMDE). Windows was abandoned (MSVC Build Tools installer
  failures) — this fork is developed on Linux.
- Toolchain: Rust 1.97.1 (≥1.91 required), protoc 3.21.12, clang 19. Prereqs installed via:
  ```
  sudo apt install -y curl git build-essential libssl-dev pkg-config protobuf-compiler libprotobuf-dev clang libclang-dev
  ```
  then rustup (stable). No Windows AR.exe/LIBCLANG quirks apply here.
- CPU miner: `kaspa-miner` (elichai/kaspa-miner) installed separately at
  `~/.cargo/bin/kaspa-miner` — not part of this workspace's `cargo build`.

## Quick start (copy-paste, in order)

Everything below targets **devnet**, using the rebranded `marigold`/`marigolddev`
address prefixes (P2.1), `26*10`-family ports (P2.2), and `marigold-` P2P handshake
name (P2.3). Kaspa addresses/peers are no longer valid on this network.

```bash
# Build the node
cargo build --release --bin kaspad

# Run the full test suite (cargo-nextest not installed; plain `cargo test` works fine)
cargo test --release

# Start a devnet node (GRPC :26610, P2P :26611, WRPC-borsh :27610; app dir ~/.rusty-kaspa/marigold-devnet/)
target/release/kaspad --devnet --enable-unsynced-mining --rpclisten-borsh=127.0.0.1 --utxoindex

# Mine to an address (kaspa-cli can't generate one — see "kaspa-cli is REPL-only" below;
# use rothschild --network devnet with no --private-key against a running node instead,
# or the throwaway kaspa-addresses example described under P0.4 if you just need bytes
# with no real key)
kaspa-miner --mining-address <devnet-address> --kaspad-address 127.0.0.1 --port 26610 --threads 4 --mine-when-not-synced

# Generate a real keypair + address, then (after funding + maturity) send transactions
target/release/rothschild --network devnet
target/release/rothschild --network devnet --private-key <hex> --to-addr <addr> --tps 1

# Ad-hoc RPC checks (kaspa-cli is not usable for this — see below): point
# rpc/grpc/examples/simple_client at grpc://localhost:26610 (devnet) instead of its
# hardcoded mainnet default of 16110, `cargo run --release -p kaspa-grpc-simple-client-example`
```

**`kaspa-cli` is REPL-only and cannot be scripted** (`cli/src/main.rs` ignores argv,
always opens an interactive `$` prompt that needs a real TTY). Don't reach for it in
any non-interactive workflow — use a gRPC/wRPC client, or `rothschild` for anything
requiring a signed transaction. Full detail under P0.3.

## Detailed step log

### P0.1 — Build

- Platform: Linux (Debian/LMDE), Rust 1.97.1, protoc 3.21.12, clang 19. All prerequisites
  were already installed on this machine — no `apt install` needed.
- `cargo build --release --bin kaspad` — clean, 10m03s, zero errors.
- Note: `kaspad --version` prints the version string but exits with code 1 (harmless
  quirk of upstream's arg parsing — not fork-caused; don't rely on its exit code in scripts).

### P0.2 — Test suite baseline (2026-08-14)

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

### P0.3 — Devnet node (2026-08-14)

Command from the plan:
```
cargo run --release --bin kaspad -- --devnet --enable-unsynced-mining --rpclisten-borsh=127.0.0.1 --utxoindex
```
- `cargo run` partially recompiled `kaspad`/`kaspa-wrpc-server`/`kaspa-build-info` (1m45s)
  even though `cargo build --release --bin kaspad` had just succeeded in P0.1 — different
  codegen flags between `cargo build` and `cargo run` invalidated those 3 crates' cache.
  Not a problem, just don't be surprised by it.
- Default devnet app dir (as of P0.3, before any rebranding): `~/.rusty-kaspa/kaspa-devnet/`
  (datadir + logs subdirs). **Correction (P2.3, see below): the `kaspa-devnet` subfolder
  part was renamed to `marigold-devnet` by P2.3's network-name change, not P2.7 as
  originally guessed here** — only the top-level `~/.rusty-kaspa` app-dir base name
  remains P2.7's job.
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

### P0.4 — Mine devnet blocks (2026-08-14)

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

### P0.5 — Exercise a wallet on devnet (2026-08-14)

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

### P2.1 — Address prefix rebrand (2026-08-14)

**Technique for recomputing bech32 test vectors** (will recur at every step that
rebrands a prefix/network-name string covered by a checksum — P2.2/P2.3 don't need
this since ports/handshake names aren't checksummed, but keep it in mind for anything
address-adjacent later): don't hand-compute bech32 checksums. Change the source
string, run the test, and the assertion failure prints the *actual* correct value as
`left:` (or panics with just the correct value in `check_from_string`'s case, since it
uses `.expect()` rather than a two-sided assert) — paste that back in, rerun, repeat
for the next failing vector. `check_into_string` only reports one mismatch per run
(the test loop stops at first panic), so this is genuinely iterative — budget one
test-run per test-vector, not one run total.

**Gotcha — the same prefix string can hide in files the step description doesn't
mention.** P2.1's own text names `lib.rs` and `wasm.rs`, but
`crypto/addresses/benches/bench.rs` also hardcoded a `"kaspa:..."` address string
(parsed via `.expect("Should work")` — would have panicked at bench time with the old
prefix now rejected). Caught by `grep -rn "kaspa" <crate-dir>` across the whole crate
rather than trusting the plan's file list literally. Worth doing this grep-the-whole-
crate check at every P2.x rebrand step, not just the files named in the plan text.

### P2.2 — Network ports (2026-08-14)

Mechanical: four functions in `consensus/core/src/network.rs` edited to the P1.10
26xxx/27xxx/28xxx scheme. Verified live on a devnet node: GRPC 26610, P2P 26611,
WRPC(borsh) 27610.

### P2.3 — P2P network isolation (2026-08-14)

`NetworkId::to_prefixed()`/`from_prefixed()` in `network.rs` changed `kaspa-` →
`marigold-`; everything downstream (`Config::network_name()`, gRPC's `network_name`
field, `RpcNetworkId`) picks it up automatically since they all funnel through this
one function — no separate edits needed there.

**Side effect worth knowing**: `kaspad/src/daemon.rs` and `database/rocknroll/src/db.rs`
both derive the per-network data/log **subfolder** name from `network.to_prefixed()`
too, so this one change silently renamed `~/.rusty-kaspa/kaspa-devnet/` →
`~/.rusty-kaspa/marigold-devnet/` (and `kaspa-mainnet` → `marigold-mainnet`, etc.) as
a side effect. **This corrects an earlier note in this file** (written during P0.3)
that guessed this rename was P2.7's job — it isn't; it already happened here. P2.7
still owns the top-level `~/.rusty-kaspa` app-dir base name itself.

**Gotcha — a stale binary gave a false pass on the first live-test attempt.** After
editing `network.rs`, I ran `cargo test` (which rebuilds test binaries) but then
directly executed the *already-built* `target/release/kaspad` for the live integration
check without rebuilding it first. Result: the node still identified as `kaspa-mainnet`
under the hood, connected successfully to a real Kaspa mainnet peer, and started
downloading real chain history (IBD) before the mistake was caught by checking file
timestamps (`stat` on the binary vs. the edited source file). **Always rebuild the
actual binary you're about to run after a source edit — a green `cargo test` does not
imply the binary on disk reflects the latest source.** This will matter even more from
here on, since Phase 2+ steps increasingly verify via live `kaspad` runs, not just
`cargo test`.

**Gotcha — a pre-existing, unrelated real-mainnet datadir exists on this machine** at
`~/.rusty-kaspa/kaspa-mainnet/` (dated March 2025, predates this project). Running
plain `kaspad` with no network flag defaults to mainnet and will find it, prompt an
interactive "database is from an older version, downgrade? (y/n)" question on stdin
(which a backgrounded process can't answer, so it just exits), and would touch real
data if forced through. **Use `--appdir=<scratch-dir>` for any mainnet-mode testing**
to sandbox it away from this — never delete or interact with the pre-existing
directory without understanding what it is first.

**Live network test used a real, currently-online Kaspa mainnet peer.** Kaspa's
configured `dns_seeders` (still present in `params.rs`, since P2.4 hasn't stripped
them yet) resolve to hosts that often run a full node alongside the DNS-seeder role.
Checked several for an open port 16111 with a plain `/dev/tcp` probe before picking
one (`seeder2.kaspad.net`); several of the *other* configured seeder hostnames no
longer resolve at all (stale/decommissioned volunteer infrastructure) — not a problem
for us since P2.4 removes this whole list regardless, but don't assume every
configured seeder is still alive if this comes up again before P2.4 runs.
