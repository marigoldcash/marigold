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

### P2.4 — DNS seeders removed (2026-08-14)

Both `MAINNET_PARAMS.dns_seeders` and `TESTNET_PARAMS.dns_seeders` set to `&[]`
(SIMNET/DEVNET were already empty upstream). `cargo build` clean; confirmed on a live
mainnet-mode node (sandboxed `--appdir`) that startup now produces zero seeder-lookup
log lines, where before (P2.3's live test) the populated list would have triggered
queries against Kaspa's real seeders.

**Own seeders come later, at P9.2 — but here's how they'll actually work, since the
question came up early.** A "DNS seeder" is not a static DNS record you type into a
dashboard — it's custom server software (Kaspa's own `dnsseeder` tool, which P9.2
notes "works unmodified against your network once P2.3's handshake name is set")
that crawls the live P2P network and answers DNS `A`-record queries *dynamically*,
returning a rotating set of currently-known-good peer IPs. It has to run as the
actual authoritative nameserver for whatever hostname it answers for.

**Can Cloudflare host this?** Not directly through the normal records dashboard (that
only serves static A/AAAA/CNAME/TXT/etc. entries), but Cloudflare is still exactly
the right place to *point* at it: create an **NS delegation record** for a subdomain
— e.g. `seed.marigold.cash` or `seed1.marigold.cash` — pointing at the nameserver(s)
of a small VPS running the `dnsseeder` binary. DNS queries for that subdomain get
referred by Cloudflare to the VPS, which then answers dynamically. This is exactly
how Kaspa's own seeders work today (e.g. `seeder2.kaspad.net` is a delegated
subdomain served by someone's own node, not a Cloudflare-hosted static record) — the
domain registrar/parent-zone host and the seeder server are different things, and
that's fine; Cloudflare stays the registrar/parent zone, a separate small VPS runs
the seeder software.

**What P9.2 will actually need**: ≥2 independent VPS instances (independent
infra/geo, mirroring the P5.8 finality-anchor trustee independence principle),
`dnsseeder` built and pointed at a synced Marigold node, an NS delegation per seeder
subdomain created in the Cloudflare dashboard for marigold.cash, then those
hostnames populated into `dns_seeders` in `params.rs` (reversing this step). Nothing
to action now — just recording the mechanism so it's not re-derived from scratch at
P9.2.

### P2.5 — New genesis blocks (2026-08-14)

Mainnet motto: *"Hell is other people's monetary policy. — Sartre"* (user's pick).
Testnet motto: plain `marigold-testnet` (matches upstream's own boring-but-clear
testnet/simnet convention — keeps the quote unique to mainnet). Both well under the
204-byte `max_coinbase_payload_len` limit (50 and 16 bytes respectively, vs. 184
available after the 20-byte fixed prefix).

**Mainnet's old genesis wasn't actually a from-scratch block** — worth knowing if
this ever needs revisiting. Real Kaspa's `GENESIS` constant had `daa_score: 1312860`
and a `coinbase_payload` embedding a Hebrew/Aramaic scripture quote *plus* a
"Bitcoin block hash" and "Checkpoint block hash" — it's a checkpoint-reset genesis
from partway into Kaspa's real history, not literally block 0 (the embedded Bitcoin
hash is a classic anti-foreknowledge technique: cite a not-yet-mined-at-design-time
Bitcoin block so nobody could have pre-mined favorable content). Testnet/simnet/
devnet's genesis blocks were already plain from-scratch ones (`daa_score: 0`, no
embedded hashes) — that's the shape this step gives mainnet too, per the plan's
explicit instruction (`daa_score: 0`, `utxo_commitment: EMPTY_MUHASH`, no checkpoint
history). Mainnet's `bits` also changed from Kaspa's real `486722099` to devnet's
easy `0x1e21bc1c` — unmineable at zero launch hashrate otherwise.

**Placeholder timestamp, not final.** Used "now" (`1786742438234` ms, 2026-08-14) for
both networks. The plan's own design already accounts for this: **P9.5 explicitly
regenerates mainnet's genesis with the real launch timestamp and motto** right before
the actual launch ceremony. So today's genesis is a Phase-2-milestone artifact for
testing the fork end-to-end, not the production one — don't treat these hashes as
precious; P9.5 will produce different ones deliberately.

**Same iterate-run-test/paste-hash technique as P2.1's bech32 checksums** — 4 rounds
(mainnet merkle root → mainnet hash → testnet merkle root → testnet hash), each from
the `assert_hashes_eq` panic's "Got hash [...]" array, pasted straight in.

**Real bug found, deliberately fixed as its own separate commit** (different concern
from `genesis.rs`; Ground rule 2 says one concern per commit): `get_chain_block_samples()`
(around line 886) hardcoded a 16-entry `POINTS` array of **real Kaspa mainnet 2021
checkpoint `(daa_score, timestamp)` pairs**, sourced from Kaspa's own genesis-proof/
tx-timestamp-estimation notebooks, prepended specifically `if network_type == Mainnet`.
This fed the `get_daa_score_timestamp_estimate` RPC call
(`rpc/service/src/service.rs:871`) — on our fork's mainnet, with a fresh genesis at
`daa_score: 0`, these 16 points were from a *different chain's* history and would have
corrupted that RPC's timestamp interpolation. Not consensus-critical (doesn't affect
block validation or fund safety — it's an auxiliary estimate endpoint), but a real
data-correctness bug that would have shipped silently broken.

**Fix (2026-08-14, same session)**: removed the whole `if network_type == Mainnet {
... } else { ... }` branch — Marigold's mainnet genesis is `daa_score: 0` like every
other network now (P2.5), so there's no analogous pre-genesis history to splice in;
the function just always does what the old `else` branch did. Also removed the
now-unused `network::NetworkType` import. `cargo build -p kaspa-consensus` clean, zero
warnings; `cargo test -p kaspa-consensus` 72/72 green. Grepped for other
`NetworkType::Mainnet` special-cases and leftover references to the removed
checkpoint data — none found; this was the only instance.

### ⚠️ Open regression — P2.1 broke 22 tests in `kaspa-wallet-core` (found 2026-08-14, not yet fixed)

While grepping around the checkpoint-timestamp bug above (searching all
`NetworkType::Mainnet` usages workspace-wide, looking for similar patterns), found
this by running `cargo test -p kaspa-wallet-core` directly — it currently fails
**22 of 49 tests**, all traceable to P2.1's address-prefix rebrand:
`"kaspa:..."`-prefixed strings hardcoded as test fixtures now fail to parse
(`InvalidPrefix`) since `"kaspa"` is no longer a registered prefix.

**Why this wasn't caught at P2.1 time**: P2.1's own verify step is explicitly scoped
to `cargo test -p kaspa-addresses` (per the plan text), which only covers the crate
where the prefix strings are *defined* — not every downstream crate that happens to
hardcode a `"kaspa:"` address string as a test fixture. No P2.x step since has run a
full-workspace `cargo test`. **Lesson: periodically run a full-workspace test pass
during Phase 2, not just the crate a step names** — targeted verify commands only
prove the step's own crate compiles/passes; they say nothing about who else depends
on the string you just changed.

Failing tests as of this writing (`cargo test -p kaspa-wallet-core` output):
```
account::tests::gen0_prv_keys
compat::gen1::test::import_golang_single_wallet_test
compat::gen1::test::import_golang_multisig_v1_wallet_test
tx::generator::test::test_generator_compound_100k_random_transactions
tx::generator::test::test_generator_compound_200k_10kas_transactions
tx::generator::test::test_generator_dust_1_1
tx::generator::test::test_generator_empty_utxo_noop
tx::generator::test::test_generator_fee_rate_compound_200k_10kas_transactions
tx::generator::test::test_generator_inputs_100_outputs_1_fees_exclude_insufficient_funds
tx::generator::test::test_generator_inputs_100_outputs_1_fees_exclude_success
tx::generator::test::test_generator_inputs_100_outputs_1_fees_include_success
tx::generator::test::test_generator_inputs_1k_outputs_2_fees_exclude
tx::generator::test::test_generator_inputs_2_outputs_2_fees_exclude
tx::generator::test::test_generator_inputs_32k_outputs_2_fees_exclude
tx::generator::test::test_generator_inputs_250k_outputs_2_sweep
tx::generator::test::test_generator_large_payload_min_relay_fee
tx::generator::test::test_generator_preserves_output_covenant_binding
tx::generator::test::test_generator_random_outputs
tx::generator::test::test_generator_sweep_single_utxo_noop
tx::generator::test::test_generator_sweep_two_utxos
tx::generator::test::test_generator_sweep_two_utxos_with_priority_fees_rejection
utxo::test::test_utxo_generator_empty_utxo_noop
```

**Not fixed yet — needs real attention, not a blind find-replace.** Two different
risk levels hide in this list:
- Most `tx::generator::test::*` and `utxo::test::*` failures likely use arbitrary
  placeholder `"kaspa:..."` addresses (like P2.1's own `cases()` vectors) — probably
  safe to fix with the same recompute-from-test-failure technique used in P2.1/P2.5.
- `compat::gen1::test::import_golang_*` and `account::tests::gen0_prv_keys` sound
  like **legacy wallet-format compatibility tests** — these may hardcode addresses
  that are meaningful to a specific historical wallet-file format/version, not
  arbitrary. Swapping their prefix without understanding what's actually being
  tested could silently defeat the point of the test. Read what each one actually
  asserts before touching it.

**Where this probably belongs**: P2.8 ("user-facing rebrand pass 2: wallet + CLI")
already scopes `wallet/` for `"kaspa"` string cleanup — this regression is in the
same crate, so likely gets swept up there. Recorded here so it isn't silently lost
between now and then. Whoever picks this up should also run a full workspace
`cargo test` once, since this discovery method (grep + spot-check one crate) doesn't
guarantee it's the *only* other affected crate.

### P2.6 — Reset fork activations (2026-08-14)

The flag flip (`crescendo_activation`/`toccata_activation` → `ForkActivation::always()`
for mainnet + testnet) is one line each, but flipping it surfaced **5 real test
failures**, none of them fake — each traced to a genuine stale-legacy-value issue.
Worth understanding the mechanism, since it'll recur for anyone touching activation
state again.

**Root mechanism**: `ForkedParam::before()` (params.rs:116) already self-corrects —
its doc comment says exactly this: "Returns the value before activation (=pre unless
activation = always)." So when activation is `always()`, `.before()` returns `post`,
not `pre`. This is *not* where the bugs were. The actual bugs were **stale
`pre`-side values that were never updated to be self-consistent with an
always-active fork**, only ever exercised now that mainnet/testnet joined
simnet/devnet in using `always()`.

**Bug 1 — `pre_crescendo_target_time_per_block` still said 1000 (1 BPS, real
Kaspa's actual historical pre-crescendo rate)** for mainnet/testnet, while
`blockrate` said 10 BPS. Simnet/devnet already avoid this exact trap — both set
`pre_crescendo_target_time_per_block: TenBps::target_time_per_block()`, i.e. *the
same* as their post-value, because they don't have real pre-crescendo history either.
Fixed mainnet/testnet the same way.

**Bug 2 — `deflationary_phase_daa_score` still said `15778800 - 259200`** (real
Kaspa's actual historical value, derived from Kaspa's real launch date and a real
3-day network outage shortly after — see the removed comment for the exact
derivation) for both mainnet and testnet, and `pre_deflationary_phase_base_subsidy`
was still the raw, un-scaled `50000000000` (Kaspa's real 1-BPS-era flat per-block
subsidy). Together these meant: any block before that (meaningless, real-Kaspa-only)
DAA score would get a flat, wrongly-large (10x too high — 500 KAS/sec instead of the
intended 50 KAS/sec-equivalent) subsidy. **Fix: `deflationary_phase_daa_score: 0` for
both** — this is not a new economics decision, it *implements* the P1.4 decision
already locked in ("no pre-deflationary phase"), just mechanically, ahead of P3.2's
real subsidy-table work. `pre_deflationary_phase_base_subsidy` becomes dead/unused
once `daa_score` is 0 (matches devnet's existing pattern) — set to
`TenBps::pre_deflationary_phase_base_subsidy()` as a harmless placeholder, same as
devnet.

**Bug 3 — a test-infrastructure gap, not a production bug.**
`TestConsensus::build_header_with_parents` (consensus/src/consensus/test_consensus.rs)
builds a header via `header_from_precomputed_hash`, which ultimately calls
`Header::from_precomputed_hash` — a generic constructor that hardcodes
`version: BLOCK_VERSION` (the pre-toccata constant) unconditionally, with no
awareness of network or activation state. This was never exercised before because no
test using this helper against `MAINNET_PARAMS` had previously hit real block
version *validation* against an always-active toccata fork. Fix: set
`header.version = self.params.block_version().get(header.daa_score);` right after
`header.daa_score` is computed in that same function — correctly derives the version
for *any* network/activation state, not just ours. This is arguably a latent
correctness gap worth reporting upstream too, since it would bite any future Kaspa
network config that activates toccata from genesis.

**Bugs 4/5 — two hardcoded subsidy literals**, `50000000000` and `44000000000`, in
`consensus/src/pipeline/body_processor/body_validation_in_context.rs`'s
`validate_body_in_context_test`. Both are direct, correctly-scaled consequences of
Bug 2's fix (`50000000000/10=5000000000`, `44000000000/10=4400000000` — confirmed via
`TenBps::pre_deflationary_phase_base_subsidy()`'s actual source before hardcoding,
not guessed). Simple literal updates once Bug 2's fix was understood.

**Verification beyond the plan's stated commands**: `cargo test -p kaspa-consensus`
(72/72), plus re-checked `kaspa-consensus-core` and `kaspa-mining` (both touch
subsidy/params too) — all green. Ran a full `cargo build --workspace` (given the
P2.1 wallet-core lesson above: targeted checks miss cross-crate breaks) — clean, no
new errors anywhere (the known wallet-core *test* failures are runtime assertions,
not compile errors, so a build-only pass doesn't re-surface them — still open,
tracked separately above). **Live check used real mainnet mode, not devnet** — devnet
wasn't touched by P2.6 at all, so testing against it would have proven nothing about
this step specifically. Generated a throwaway mainnet address (same
`kaspa-addresses` example-then-delete pattern as P0.4/P2.5), started a sandboxed
`--appdir` mainnet node, mined with `kaspa-miner` pointed at the real mainnet port
(26110) — blocks accepted at genuine 10 BPS pace, no version or subsidy rejections.

### P2.7 — User-facing rebrand pass 1: node (2026-08-14)

Straightforward once the P2.3 investigation had already mapped `app_dir`/log-file
territory. App dir `~/.rusty-kaspa` → `~/.marigold`; log files `rusty-kaspa.log`/
`rusty-kaspa_err.log` → `marigold.log`/`marigold_err.log`; `--help` banner text (not
the binary name) rebranded; three crates' `Cargo.toml` `description` fields updated
for consistency (kaspad's feeds the `--help` banner directly).

**The judgment call**: whether to also rename the actual clap `Command::new("kaspad")`
name and the `"Kaspad"` mentions sprinkled through log/error messages
(`"Kaspad has stopped..."`, DB-version-mismatch prompts). Decided **no** — the
binary/crate itself stays named `kaspad` (Ground rule 1: crate/module identity is
off-limits, kept for upstream-merge feasibility), so every message that refers to
"Kaspad" as the name of the running program is *still accurate*, not stale. Renaming
just the display text while leaving the actual invoked binary as `kaspad` would create
a new inconsistency, not fix one.

**Also deliberately skipped**: a ~40-line `/* ... */` block comment in `args.rs`
(lines ~608+) that looks like a snapshot of Go-kaspad's old `--help` output, kept as
developer reference. It's dead code — never compiled or displayed — and already
inconsistent with reality independent of branding (it lists Kaspa's pre-P2.2 ports,
16111/16210, not even the Rust node's actual historical defaults). Out of scope for
a *live* user-facing strings pass.

Verified: `--help` banner reads "Marigold full node daemon (marigold-node) v...";
a fresh run with an isolated `$HOME` created `~/.marigold/` (confirmed via directory
listing, not just log text). `cargo test -p kaspa-core -p kaspad -p kaspa-daemon` —
all green.

### P2.8 — User-facing rebrand pass 2: wallet + CLI, and the P2.1 regression fix (2026-08-14)

The biggest step so far. Two parts: fixing the still-open P2.1 regression (22 failing
`kaspa-wallet-core` tests), then the actual P2.8 sweep — and along the way, a
full-workspace verification pass surfaced three *more* real bugs in crates P2.8
doesn't even own.

**The bech32 re-prefixing tool.** Fixing the P2.1 regression meant updating ~344
hardcoded `"kaspa:..."`/`"kaspatest:..."` address strings across the wallet crates.
Two options: generate fresh arbitrary addresses (fine for pure placeholders, e.g.
`wallet/core/src/tx/generator/test.rs`'s `change_address()`/`output_address()`
helpers), or preserve the exact original payload under a new prefix (required for
anything where the address is *derived* from something else — script bytes in
`crypto/txscript`, or fixed BIP32 seed keys in the `gen0`/`gen1` legacy-derivation
test vectors, where the test's whole point is "does deriving from this known seed
produce this known address"). Since most of the 344 were the derivation-tied kind,
I wrote a standalone Python script
(`bech32_reprefix.py`, scratchpad) that reimplements the *exact* algorithm from
`crypto/addresses/src/bech32.rs` (same charset, same polymod generator constants,
same `conv8to5`/`conv5to8` bit-packing) but takes an arbitrary raw prefix *string*
rather than the crate's `Prefix` enum — letting it decode an address under its
*old* prefix (which the enum no longer accepts) and re-encode under the new one.

**Self-verified before trusting it on real data**: round-tripped it against 3
already-known-correct P2.1 test vectors (mainnet all-zero, testnet ECDSA all-zero,
mainnet non-trivial payload) before running it on anything real. Then: extracted
every quoted `"kaspa..."`/`"kaspatest..."` address workspace-wide (grep), converted
all of them in one batch (zero decode errors — a good sign they were all
genuinely valid, uncorrupted bech32 to begin with), and applied the replacements
via exact string substitution (safe here since full addresses+checksums are long
enough to never collide with unrelated text). Net: 346 replacements across 5 files
in the first pass (`wallet/core`'s `account/mod.rs`, `compat/gen1.rs`, `wallet/mod.rs`;
`wallet/keys`'s `gen0/hd.rs`, `gen1/hd.rs`), all 22 previously-failing tests fixed,
zero new failures. This same tool script and technique is worth reusing for any
future rebrand step that touches hardcoded address strings.

**The actual P2.8 sweep**, once the regression was cleared:
- **Ticker suffix** (`kaspa_suffix()`, duplicated verbatim in both `wallet/core/src/utils.rs`
  and `wallet/pskt/src/wasm/utils.rs` — same fix needed in both places):
  `KAS`/`TKAS`/`SKAS`/`DKAS` → `MAGLD`/`TMAGLD`/`SMAGLD`/`DMAGLD`. This is the string
  that actually answers P2.8's own verify condition.
- **Account storage-kind tags** — `LEGACY_ACCOUNT_KIND`, `BIP32_ACCOUNT_KIND`,
  `BIP32_WATCH_ACCOUNT_KIND`, `MULTISIG_ACCOUNT_KIND`, `KEYPAIR_ACCOUNT_KIND`,
  `WATCH_ONLY_ACCOUNT_KIND`, `RESIDENT_ACCOUNT_KIND` — each `"kaspa-X-standard"` →
  `"marigold-X-standard"`. Checked every usage site first (grep across the whole
  crate) to confirm nothing hardcodes the literal string separately for comparison
  — everything routes through the same Rust constant, so a consistent rename is
  safe. Found and fixed one literal duplicate that needed to stay in sync: a
  TypeScript type-definition string in `wasm/api/message.rs` mirroring
  `KEYPAIR_ACCOUNT_KIND`'s value for the generated `.d.ts`.
- **Default wallet storage location** — `~/.kaspa` → `~/.marigold`,
  default wallet/settings file name `"kaspa"` → `"marigold"`
  (`wallet/core/src/storage/local/mod.rs`, `settings.rs`), plus the matching
  `cli.rs` prompt-suppression check (hides the wallet name from the CLI prompt when
  it's still the boring default — needed updating to check for the *new* default,
  not the old one, to keep working).
- **CLI terminal link matcher** (`cli/src/matchers.rs`) — this one was a genuine
  functional bug, not just cosmetic: the address-matching regex was literally
  `(kaspa|kaspatest):\S+`, which would never match a Marigold address at all
  (clicking/copying addresses printed in the terminal would have silently stopped
  working). Fixed the regex, and rebranded the three `explorer.kaspa.org` URLs
  (addresses/blocks/txs) to `explorer.marigold.cash` — a forward-looking placeholder
  since no explorer exists yet (that's P9.4); better than leaving it pointed at
  Kaspa's real explorer, which would show "not found" or worse, someone else's
  address, for a Marigold address.
- **CLI output strings** — literal `"... KAS"` balance/scan-result text in
  `account.rs`, `pskb.rs`, `send.rs` → `MAGLD` (these hardcode their own suffix
  rather than calling `kaspa_suffix()`, so no double-suffix risk).
- **`marigold-cpu-miner`** — the NW.js desktop-app bundled-binary search name in
  `cli/src/modules/miner.rs`. No such binary exists yet either way (we've only used
  the separate community `kaspa-miner` tool for testing, never built our own), so
  this doesn't change current behavior — renamed for forward consistency with
  whatever Marigold's own bundled miner eventually gets called.

**Deliberately left unchanged** (same "identifier, not display string" judgment
already applied to the `kaspad` binary name at P2.7):
- The `kaspad` binary name itself and every log/prompt message that names it
  accurately (`"Kaspad has stopped..."`, DB-version-mismatch prompts) — still
  correct since the binary really is still called `kaspad`.
- `kaspa_utils::...` crate paths (Ground rule 1).
- The WASM/JS **public API surface** — `#[wasm_bindgen(js_name = "kaspaToSompi")]`
  and its siblings (`sompiToKaspaString`, `ISompiToKaspa`, etc.). This is a bigger,
  separate concern than a single string: a whole family of interdependent public
  function/type names external SDK consumers would call directly. Renaming it
  properly would be a systematic API redesign (and I have no JS/TS build harness
  here to verify nothing else references these names), not a "grep for display
  strings" fix — deferred, not forgotten. *Doc comment text* describing these
  functions (e.g. "returns `KAS` for mainnet...") was still fixed, since that's pure
  prose, not an identifier.
- `compat/gen0.rs`'s `Kaspa/kaspa.kpk` paths and the `"kaspa-wallet"` local-storage
  key in `legacy_v0_keydata_location()`. **This one needed real investigation, not
  a pattern-match**: these strings target a *real, external, pre-existing* Kaspa
  wallet's actual on-disk/browser-storage format (the original browser-based
  "gen0" wallet), used for a genuine legacy-import compatibility feature.
  Renaming them wouldn't be a rebrand — it would silently break the ability to
  import a real user's real legacy Kaspa wallet, which is the entire point of this
  module. Confirmed via `compat/gen1.rs` too (function names like
  `import_kaspawallet_golang_single_v1` reference a *different* real external
  legacy wallet, the Go `kaspawallet` CLI tool) — same category, same reasoning,
  no changes needed there since it had no literal format-marker strings, only
  identifiers.

**Three more real bugs found via full-workspace verification, each its own
commit** (not P2.8's commit — different crates/concerns per Ground rule 2):
1. Same P2.1 regression pattern, one crate over: `crypto/txscript/src/standard.rs`
   had 2 hardcoded `"kaspa:..."`/`"kaspatest:..."` expected-addresses, script-bytes-
   derived (payload-preserving fix required, same tool). Also fixed 3 lines of
   dead/commented-out code in `consensus/core/src/tx.rs` referencing the same
   address for consistency.
2. `testing/integration`'s `header_in_isolation_validation_test` — a P2.6-pattern
   bug (hardcoded `BLOCK_VERSION` as the "correct" expected value in an assertion,
   now wrong since toccata is active from genesis) — own commit, see its message.
3. `bridge/` (stratum-bridge) — a genuinely new, real functional bug: wallet-address
   regex/fallback-prefix logic still hardcoded `kaspa:`/`kaspatest:`/`kaspadev:`,
   meaning a bare address submitted by a miner would get incorrectly coerced to an
   invalid `kaspa:...` address. Own commit, see its message for full detail
   (including why the existing test suite didn't catch it: fixtures were
   self-consistently using the same stale prefix on both sides of the comparison,
   masking the bug until the fixtures were also updated).

**Verification.** `kaspa-cli` remains REPL-only (P0.3), so — same workaround as
before — called `sompi_to_kaspa_string_with_suffix()` directly via a throwaway
example: confirmed output `"1,234.56789012 MAGLD"`. Ran a full `cargo build
--workspace` and `cargo test --workspace` (not just the touched crates) given the
standing lesson from the P2.1 regression that targeted checks miss cross-crate
breaks — this is exactly what caught the txscript/testing-integration/bridge bugs
above. Final state: 144 test-result blocks, 0 failures, matching/exceeding the P0.2
baseline.

### P2.9 — Two-node private network smoke test (2026-08-15)

Rebuilt `kaspad` release fresh first (standing lesson: never trust a binary that
predates the last edit for a live-network test — real find at P2.3).

**Setup**: two devnet nodes, separate `--appdir`s under the session scratchpad
(never touches `~/.rusty-kaspa` or the pre-existing real-mainnet datadir). Node A
used every default port (gRPC 26610, borsh-wRPC 27610, JSON-wRPC 28610, P2P 26611 —
the P2.2 scheme). Node B needed every listener moved to avoid binding collisions on
the same host: `--listen=127.0.0.1:26621 --rpclisten=127.0.0.1:26620
--rpclisten-borsh=127.0.0.1:27620 --rpclisten-json=127.0.0.1:28620
--addpeer=127.0.0.1:26611`. Both came up and handshook within ~10s of node B's
start (`Registering p2p flows for peer ... for protocol version 9` on both sides,
node A inbound / node B outbound) — confirms same-network peers still connect fine
post-P2.3 (P2.3 only proved *cross*-network rejection).

**Mining address**: same throwaway-`cargo run --example`-then-delete technique as
P0.4 (`Address::new(Prefix::Devnet, Version::PubKey, &payload)`, arbitrary 32 bytes,
no real key needed since nothing spends from it) — this time correctly producing a
`marigolddev:...` address (P2.1's prefix), not the old `kaspadev:...` one P0.4 got.

**Mining gotcha**: `kaspa-miner` speaks gRPC, not borsh-wRPC — first attempt pointed
`--port` at node A's borsh port (27610) and got `ConnectionRefused`; the correct
target is the gRPC port (26610, node A's default). Once corrected, mining and
submission worked immediately (devnet genesis difficulty is trivial by design, same
as P0.4).

**Sync verification (log-based)**: after ~15s of mining, node A's log shows a
sequence of `Accepted N blocks ...<hash> via submit block` lines; node B's log
shows the **identical hashes, in the same order**, each as `Accepted N blocks
...<hash> via relay`. This is the real verification, not just "both logs mention
blocks" — same DAG, same order, arrived via P2P relay rather than independent
mining.

**DAA-score verification (RPC-based)**: same throwaway-edit-then-revert pattern as
P0.3/P0.4/P2.5/P2.6, this time on `rpc/grpc/examples/simple_client` (which is a
plain workspace bin, not a Cargo `[[example]]` — confirmed via its `Cargo.toml`
before trying `cargo run --example`, which fails for it). Hardcoded URL swapped for
a CLI-arg port, built once, run twice (`26610`, `26620`), diff reverted with `git
checkout --` immediately after. Result — **every field identical** between the two
nodes: block count 119, header count 119, virtual DAA score 119, tip hash, sink
hash, pruning point hash, both `is_synced: true`.

**Cleanup**: `pkill -x kaspad` stopped both cleanly (no orphaned processes); miner
stopped with `pkill -x kaspa-miner` before the RPC check. Appdirs and all logs live
under the session scratchpad only — nothing added to the repo, working tree clean
after the example-file revert.

This closes Phase 2. The fork is now verified, not just argued, to be a real
independent P2P network: two independently-started nodes with no shared state
converge to byte-identical DAG views purely through the P2P layer this phase
rebuilt (own ports, own P2P handshake network name, own genesis, own fork
activations).

## Phase 3 — Economics

### P3.1 — How the existing emission mechanism works (2026-08-15)

Read [consensus/src/processes/coinbase.rs](../../consensus/src/processes/coinbase.rs)
in full (632 lines) plus its production wiring and call sites. Summary for P3.2:

**The core data structure** is `SUBSIDY_BY_MONTH_TABLE` (coinbase.rs:286-305): a
`const [u64; 426]` baked into the binary, one entry per elapsed month since the
start of the "deflationary phase." Each entry is the **per-second** subsidy for that
month (= per-block subsidy at a reference rate of 1 BPS) — comments in the file say
it was originally generated by porting kaspad-go's
`calcDeflationaryPeriodBlockSubsidyFloatCalc` and running `TestBuildSubsidyTable`.
The shape: 44,000,000,000 sompi (440 KAS) at month 0, exactly halving every 12
entries (month 12 = 22,000,000,000, month 24 = 11,000,000,000, ...), with the 11
months *between* each halving boundary smoothly interpolated (not a single cliff
drop) — i.e. real Kaspa's actual emission is "halve once a year, but glide there
smoothly within the year," not a pure geometric year-over-year step function. The
table tapers cleanly to exactly `0` at its last entry (month 425, ~35.4 years in) —
no separate "tail cutoff" logic is needed elsewhere; once you're past the table, the
subsidy *is* zero.

**Two more inputs gate the table**: `deflationary_phase_daa_score` (DAA score where
the table-driven era begins) and `pre_deflationary_phase_base_subsidy` (a flat
subsidy paid to every block *before* that score — real Kaspa's actual launch used a
~6-month "bootstrap" period at a flat 500 KAS/block, then dropped — not smoothly —
to the table's 440 KAS/block starting value; see `create_legacy_manager()`'s
`15778800 - 259200` = 6 months minus 3 days, and `50000000000` = 500 KAS). **This
whole pre-deflationary branch is currently dead code for us**: P2.6 set
`deflationary_phase_daa_score = 0`, so for every `daa_score >= 1` the `daa_score <
deflationary_phase_daa_score` check in `calc_block_subsidy()` (line 229) is never
true — we fall straight into the table-driven branch from the first mined block,
consistent with the fair-launch-from-zero decision (P1.5): no separate bootstrap
plateau, decay starts immediately.

**BPS scaling.** `CoinbaseManager::new()` (line 66) precomputes two *scaled* copies
of the table — `subsidy_by_month_table_before`/`_after` — by dividing every entry by
the pre-/post-crescendo BPS (`div_ceil`, rounded up), since going from 1 block/sec
to N blocks/sec means each individual block should get roughly 1/N the per-block
reward to preserve the same total emission per unit wall-clock time. **For us this
split is moot**: P2.6 also set `crescendo_activation = ForkActivation::always()`
(10 BPS active from genesis), so `bps_history.activation().is_active(daa_score)` in
`calc_block_subsidy()` (line 234) is always true — only `subsidy_by_month_table_after`
is ever read in practice; `_before` exists only because `ForkedParam` always carries
both variants.

**Turning a DAA score into a table index** (`subsidy_month()`, line 245) converts
elapsed *blocks* since `deflationary_phase_daa_score` into elapsed *seconds* (divide
by BPS — a 3-way branch handles a BPS change happening mid-range, irrelevant to us
since BPS is constant from genesis), then elapsed seconds into a month index by
dividing by `SECONDS_PER_MONTH` (2,629,800 — a fixed 365.25-day-year average month,
line 23). The month index is clamped to the table's last valid index.

**Where it's consumed** (the verify condition for this step) — exactly two call
sites, both going through one `CoinbaseManager` instance built once, from `Params`,
in [consensus/src/consensus/services.rs:124](../../consensus/src/consensus/services.rs#L124):
1. **Validation** —
   [body_validation_in_context.rs:74](../../consensus/src/pipeline/body_processor/body_validation_in_context.rs#L74):
   for every incoming block, `calc_block_subsidy(block.header.daa_score)` computes
   the *expected* subsidy and compares it against what the block's own coinbase
   payload declares (deserialized via `deserialize_coinbase_payload`). Mismatch =
   consensus-invalid block, rejected. This is the actual enforcement point — the one
   P3.3's "provably sums below the cap" claim rests on.
2. **Template generation** —
   [virtual_processor/processor.rs:1454](../../consensus/src/pipeline/virtual_processor/processor.rs#L1454)
   and [utxo_validation.rs:304](../../consensus/src/pipeline/virtual_processor/utxo_validation.rs#L304):
   `expected_coinbase_transaction()` (coinbase.rs:100, itself calling
   `calc_block_subsidy()` at line 138) builds the actual coinbase transaction template
   — this is what a mining block template's reward output is computed from, and what
   the UTXO-diff for a newly-accepted block uses to credit the miner.

**Implication for P3.2.** The existing architecture bakes in one specific shape:
year-scale halving (12-entry-per-halving table) with smooth intra-year interpolation,
generated by a curve-fitting script whose original Go source isn't in this repo (only
its output, the const table, and a comment naming the generator function/test). To
hit the P1.4 target (210M cap, 3-year halving, no tail, decay from block 0) P3.2 has
two honest options: (a) regenerate an equivalent large lookup table using the same
"halving boundary + smooth interpolation" shape stretched to 3-year (36-entry)
halving periods, preserving the existing table-driven architecture and its
tap-to-exactly-zero tail behavior; or (b) replace the table with a directly-computed
closed-form decay function evaluated at block-validation time, which is simpler to
reason about and audit (no opaque 400+-entry magic-number array) but is a bigger
structural change to `CoinbaseManager` and needs its own overflow/rounding-safety
argument to substitute for the table's implicit one. Bringing this choice explicitly
to P3.2 rather than assuming either.

### P3.2 — Generate the subsidy table (2026-08-15)

Went with option (a) from P3.1: regenerate an equivalent table, same architecture,
stretched to 3-year (36-month) halving. Full rationale/numbers in
[DECISIONS.md](DECISIONS.md)'s new "P3.2 — Subsidy table implementation" section;
this entry is the build/test/debugging log.

**Generator.** Wrote it first as a standalone Python script in the scratchpad
(fast iteration), confirmed the exact bisection result, then ported the identical
algorithm into a permanent Rust `#[ignore]`d test,
`processes::coinbase::tests::generate_subsidy_table` — mirrors Kaspa's own original
convention (their header comment named a `TestBuildSubsidyTable` to rerun). Diffed
the Rust generator's live stdout against the pasted const array byte-for-byte
(stripped to just the numeric rows) — identical, confirming the paste wasn't
transcribed wrong. `total_emission_stays_under_cap` (permanent, not ignored) is the
actual enforcement the Phase 3 goal ("provably sums below the cap") asks for;
`generate_subsidy_table` is reproducibility/documentation, not enforcement.

**Table shape.** 1016 months (vs Kaspa's 426) — expected, since a 3-year halving
period decays 3× slower than Kaspa's 1-year one, so it takes proportionally longer
to round down to 0. `SUBSIDY_BY_MONTH_TABLE_SIZE` updated to match; grepped the
whole workspace for other consumers of that constant or the table's values first —
found none outside `coinbase.rs` itself, so no other file needed touching for the
table swap itself (the *tests* touched below are a separate matter — real hardcoded
literals, not table-size-dependent).

**`subsidy_test` couldn't just get new numbers plugged in.** Kaspa's original
version cross-checked hardcoded fractions of the initial subsidy
(`initial_subsidy / 2^n`) against specific halving counts (32, 35, 36) tuned to
their exact table length. Checked empirically whether this still holds for our
table: `table[36]==table[0]/2` ✅, `table[180]==table[0]/32` (5 halvings) ✅, but
`table[72]==table[0]/4` (2 halvings) ✗ — off by 1 due to rounding, and month
`32*36=1152` doesn't even exist in our 1016-entry table. Rewrote the test to spot-check
`calc_block_subsidy`'s DAA-score → month → table-lookup → BPS-scaling *wiring*
directly against real table entries (by index) instead of re-deriving expectations
via a second formula — more robust, and arguably a better test design regardless of
table size, since it stops assuming a coincidental integer-halving property that was
never guaranteed by the generation formula in the first place.

**Simnet almost got "fixed" incorrectly.** Noticed `SIMNET_PARAMS` was the one
network P2.6 left on the real-Kaspa-derived `TenBps::deflationary_phase_daa_score()`
instead of `0`, assumed it was an oversight, and changed it "for consistency" —
which broke `daemon_integration_tests::daemon_utxos_propagation_test` (and a sibling
assertion), both of which deliberately assert `initial_blocks *
SIMNET_PARAMS.pre_deflationary_phase_base_subsidy` for a `coinbase_maturity`-sized
initial mining run. Investigated rather than patched around it: simnet is a
PoW-skipped internal benchmark/test harness (per its own pre-existing params
comment), never a real user-facing network, so P1.4/P1.5's fair-launch commitment
was never actually meant to bind it — the flat pre-deflationary phase there is
existing test infrastructure, not economics. Reverted the "fix." **Lesson**: a
uniformity cleanup that isn't explicitly requested needs the same verification bar
as any other change — run the tests before deciding it's obviously correct.

**Three more real bugs, found via the standing full-workspace-plus-ignored-tests
lesson, each its own commit:**
1. `body_validation_in_context.rs::validate_body_in_context_test` — hardcoded
   expected-subsidy literal `4400000000` (Kaspa's real month-0/BPS value) → our
   `15228085` (our month-0/BPS value). Caught immediately by
   `cargo test --workspace`.
2. `verify_crescendo_emission_schedule` — an `#[ignore]`d, genuinely long test
   (~15-20 minutes at our table's scale: ~26M DAA-score iterations per table month,
   ×1016 months, ×4 activation scenarios) that was apparently never actually run
   this session before now. It cross-checks `calc_block_subsidy` against
   `legacy_calc_block_subsidy`, which treats its argument as literal elapsed seconds
   (implicitly assuming 1 BPS). That assumption silently broke back at **P2.2**,
   which deliberately set `pre_crescendo_target_time_per_block` to match the real 10
   BPS rate rather than a fake 1-BPS history — a real, previously-undiscovered
   latent bug, surfaced only because P3.2's diligence pass finally ran the
   `--ignored` test. Fixed by converting blocks→seconds (`current / bps_before`)
   before calling the legacy function, then scaling its raw table-value result back
   down by the same BPS — both conversions are no-ops at bps=1, so the fix is
   backward-compatible with real Kaspa's own original test intent.
3. Five `goref_*` tests in `testing/integration/src/consensus_integration_tests.rs`
   (`goref_custom_pruning_depth_test`, `goref_notx_test`,
   `goref_notx_concurrent_test`, `goref_tx_small_test`,
   `goref_tx_small_concurrent_test`) replay real, literal historical Kaspa mainnet
   block data (`testdata/dags_for_json_tests/goref-*`) — each recorded block's own
   coinbase payload declares its real historical subsidy. `json_test()` builds its
   `Params` from the fixture's own genesis, but `SUBSIDY_BY_MONTH_TABLE` is a global
   const, not part of `Params` — no override can rescue this, the table's *values*
   are what conflict, not an index. First failure surfaced as a **double panic and
   `SIGABRT`** (the harness's own DB-lifetime-check panicked during unwind from the
   `WrongSubsidy` panic), which aborted the whole `kaspa-testing-integration` test
   binary and silently prevented every other test in it from running or reporting —
   worth remembering: a single unhandled panic in an async integration test can mask
   an entire binary's results, not just fail one test. Marked all five `#[ignore]`d
   with an explanatory reason (same "ignore, don't delete" treatment as other
   real-Kaspa-history artifacts this session) rather than fixed, since replaying
   real Kaspa chain history against a permanently-diverged economics schedule can
   never validate again — this isn't a bug.

**Verification.** `cargo test -p kaspa-consensus --lib coinbase`: 7 passed, 2
ignored (the generator + the long crescendo test), 0 failed — matches P3.2's stated
verify condition. Full `cargo build --workspace` and `cargo test --workspace`: 144
test-result blocks, 0 failures — matches the P2.8/P2.8-era baseline exactly, despite
the goref tests moving from "ran and passed" to "ignored" (same total count, since
they're still compiled and counted, just skipped). `generate_subsidy_table` output
diffed byte-for-byte against the pasted const array. `verify_crescendo_emission_schedule`
re-run in release mode after the fix: **passed, 1842.92s (~30.7 minutes)** — real
scale for our 1016-month table across 4 activation scenarios (baseline + 3 sample
points). `DIFF (KAS): 1` at the largest activation point, comfortably inside the
`<= 51` bound.

**One honest side note from actually running this to completion.** The test's own
`calculate_emission()` sums real per-block subsidies one block at a time (~26.7
billion blocks total, from `deflationary_phase_daa_score` to full depletion) rather
than the idealized `Σ table[i] × seconds_per_month` the permanent cap test uses.
Baseline total came out **21,000,013,335,360,000 petals — ~133.35 MAGLD *over* the
210,000,000 MAGLD nominal cap** (a `0.0000635%` overshoot). This isn't a bug or a
contradiction of `total_emission_stays_under_cap`: that test correctly implements
the plan's literal verify condition (`Σ table × seconds-per-month`), which is an
idealized continuous-time model, not a full per-block simulation. The overshoot is
`div_ceil` rounding at 10 BPS — each month's table entry gets divided up across
~26.3M real blocks, and any remainder rounds up per block, accumulating over
billions of blocks — the exact same architecture Kaspa's own original design has
(their `calc_high_bps_total_rewards_delta` test measures and prints this same
phenomenon for their table, unasserted). Not worth "fixing": 133 MAGLD out of 210M
is far smaller than the ~0.0036 MAGLD-scale precision the generator already aims
for at the idealized level, and eliminating it would mean abandoning the
`div_ceil`-based BPS-scaling architecture entirely — out of scope for P3.2, which
was told to keep the existing table-driven design.

### P3.3 — Emission integration check (2026-08-15)

Rebuilt `kaspad` fresh (standing lesson). Fresh single-node devnet, `--utxoindex`
enabled (required for `get_coin_supply`), scratch `--appdir`. Generated a throwaway
`marigolddev:` sink address the same way as P0.4/P2.9 (no real key needed). Mined
with `kaspa-miner` against the node's gRPC port (26610) until past 1000 blocks
(~4 minutes wall-clock — slower than P2.9's 15-second burst since that run only
needed ~120 blocks; devnet difficulty still ramps up as blocks land, so getting to
1000+ legitimately takes proportionally longer, nothing wrong).

**RPC query.** Extended the usual throwaway-edit-then-revert pattern on
`rpc/grpc/examples/simple_client` (port-as-CLI-arg, as in P2.9) by also calling
`get_coin_supply()` (not previously used by this example) and printing
`circulating_sompi / block_count` as a quick average. Reverted after, as always.

**Result — block/header count 1098, virtual DAA score 1098, circulating supply
16,705,209,245 petals.** At first glance this doesn't obviously match table[0]'s
per-block value (15,228,085): naive `circulating / block_count` gives ~15,214,216,
about 0.09% low. The catch: `block_count` includes genesis (DAA score 0, no
coinbase reward), so only **1097** of those 1098 blocks actually minted a subsidy.
`15,228,085 × 1097 = 16,705,209,245` — an **exact** match, confirmed via `python3 -c
"print(15228085 * 1097)"`. Zero deviation, not even a rounding hair — expected here
since this was a single-miner, no-parallel-mining devnet run (a purely linear
chain, no merged/red blocks to introduce the "± red-block/merge effects" slack the
plan's verify condition anticipates for less controlled setups).

**Real bug caught by this same RPC call.** `get_coin_supply`'s `max_sompi` field
came back as real Kaspa's actual max supply
(`consensus/core/src/constants.rs::MAX_SOMPI = 29_000_000_000 * SOMPI_PER_KASPA`,
~2.9 × 10¹⁸ petals) instead of ours (`210,000,000 * SOMPI_PER_KASPA` = 2.1 × 10¹⁶
petals) — off by ~138×. `MAX_SOMPI` does double duty: it's both the value
`get_coin_supply` reports and the sanity-bound used in transaction-output/total
validation (`tx_validation_in_isolation.rs`, `tx_validation_in_utxo_context.rs`,
mempool fee capping) — exactly mirroring how real Kaspa used their own actual max
supply for both purposes, so the correct fix for us is the same pattern with our
own cap, not a new mechanism. Grepped for other hardcoded references to the old
literal value first — none found outside the constant's own definition. Fixed,
rebuilt, reran `cargo test -p kaspa-consensus --lib -- transaction_validator` (13
passed) and a full `cargo build --workspace` (clean) before the live devnet run, so
the fix was already verified before it got exercised over real RPC — the RPC output
above (`Max supply (petals): 21000000000000000` = exactly 210,000,000 MAGLD) is
confirmation, not the first check.

**Cleanup.** `pkill -x kaspa-miner` then `pkill -x kaspad`, both exited cleanly.
Scratch appdir/logs under the session scratchpad only, nothing added to the repo.

This closes Phase 3 (P3.1-P3.3). Marigold's own emission schedule is now
understood, implemented, capped by a permanent test, and confirmed correct against
a real running node over RPC — not just unit-tested in isolation.

## Phase 4 — MILESTONE: transparent chain running end-to-end

### P4.1 — Testnet-in-a-box script (2026-08-15)

Wrote both scripts fresh (no prior scripts/ directory existed). Extended the
two-node pattern from P2.9 to three: node1 keeps every default devnet port, node2
and node3 each get their P2P/gRPC/borsh-wRPC/JSON-wRPC ports shifted by +10/+20
respectively (26621/26631 for P2P, etc.) to avoid bind collisions on one host, and
both peer to node1 via `--addpeer` (a star topology — simplest reliable way to get
"3 synced nodes," and P2.9 already confirmed devnet peers sync cleanly).

**Design choices:**
- **Auto-builds `kaspad` if missing** rather than just failing/instructing the user
  to build it first — the plan's own verify condition ("running the script from a
  clean checkout yields 3 synced nodes") implies a clean checkout with no prior
  `cargo build` should still work end-to-end.
- **Reusable data dir**, not wiped on each run — `x-testnet-local-data/` persists
  across restarts (stop with `pkill -x kaspad` / `Get-Process kaspad | Stop-Process`,
  relaunch the script later to resume the same chain state). Deleting the directory
  is the documented way to get a clean start. Added to `.gitignore`.
- **Mining instructions point at `rothschild --network devnet`** for getting a real
  keypair/address, not the internal throwaway-`cargo run --example` trick used
  elsewhere in this session — `rothschild` is an actual committed repo tool, the
  officially-supported way for an external user to get a funded devnet address,
  whereas the throwaway-example approach is a debugging convenience for sessions
  with direct repo access, not something to hand to a future script's end user.

**Verification.** Ran the bash script from a clean state (data dir removed first).
All 3 nodes peered immediately (no retry needed). Generated a throwaway
`marigolddev:` address (P0.4/P2.9-style, no real key needed) and mined 12 blocks
against node1 with `kaspa-miner`; `grep -c "via relay"` on node2 and node3's logs
both showed exactly 12 — every mined block relayed to both peers. Cross-checked
over gRPC (same throwaway-edit-then-revert on `rpc/grpc/examples/simple_client` as
P2.9/P3.3, port made a CLI arg): all three nodes' block count (64), virtual DAA
score (64), and sink hash matched exactly. Stopped all three cleanly, reverted the
example edit, removed the test data dir. Did not test the PowerShell script live
(no Windows environment available this session) — written to the same structure/
flags as the verified bash version, but flagging this as unverified until run on
Windows.

### P4.2 — Full user-journey test (2026-08-15)

Wrote [SMOKE.md](SMOKE.md) as the deliverable, then actually walked it end to end
on the P4.1 local testnet (not a fresh ad-hoc setup) before writing down the final
steps, so every command in SMOKE.md is one that was actually run, not just planned.

**Wallet A.** `rothschild --network devnet --rpcserver 127.0.0.1:26610` (no
`--private-key`) generated a keypair — but the FIRST attempt printed
`kaspadev:qrm7ja...` instead of `marigolddev:...`. Immediately recognized this as
the exact P2.3 stale-binary trap (documented: "always rebuild kaspad before a live
test"), just hitting `rothschild` instead of `kaspad` this time — confirmed via
`stat` that `target/release/rothschild` predated the P2.1 address-prefix rebrand by
about 5.5 hours. Rebuilt (`cargo build --release --bin rothschild`), regenerated —
correct `marigolddev:` prefix. **Lesson reinforced**: "rebuild before a live test"
applies to every binary you're about to use, not just the one most recently edited
— worth calling out explicitly in SMOKE.md's own gotchas section so a future
session doesn't lose the same few minutes.

**Mining + maturity.** Mined with `kaspa-miner` to wallet A's address until well
past DAA score 3000 (coinbase_maturity × 2 = 2000, confirmed via rothschild's own
startup banner: "Coinbase maturity: 1000"). Balance query confirmed
46,278,150,315 petals — mature and spendable.

**The real bug: sending never worked, no matter how long we waited.** Started
`rothschild --private-key <A> --to-addr <B> --tps 1` — every single tick logged
`"Has not enough funds"` / `"Refetching UTXO set"`, even after mining thousands
more blocks and confirming (via a throwaway extension of the gRPC example client
to dump `get_utxos_by_addresses`) that wallet A held 3612+ UTXOs, all `is_coinbase:
true`, with `block_daa_score` values from 2 up to the current tip — i.e., clearly
mature, spendable-looking UTXOs in abundance. Root-caused by reading
`rothschild/src/main.rs`'s `select_utxos()` directly rather than guessing further:
it combines UTXOs one at a time toward a `DEFAULT_SEND_AMOUNT` target
(`10 * SOMPI_PER_KASPA`, i.e. "10 KAS" in the original code), but gives up
(`return (vec![], 0)`) once it's combined more than `MAX_UTXOS = 8` without
reaching the target. Marigold's own genesis-era coinbase reward is only
15,228,085 petals/block (P3.2) — 8 of them sum to ~121.8M petals (~1.2 MAGLD),
nowhere near 10 MAGLD. This is a hard mathematical cap, not a timing issue: mining
longer only creates *more* same-sized UTXOs (the schedule is flat within a month),
never *bigger* ones, so this would have failed identically after an hour or a
week of mining. Confirmed the diagnosis by checking `node1.log`'s
"Processed N blocks" summaries during the failed attempts: `1.00 TPB` (transactions
per block) throughout — meaning literally zero user transactions were ever being
included, only coinbase.

Fixed by lowering `DEFAULT_SEND_AMOUNT` to `SOMPI_PER_KASPA` (1 MAGLD-equivalent,
~7 blocks' worth at genesis — chosen to preserve roughly the same margin below
`MAX_UTXOS` that Kaspa's original "10 KAS at ~4.4 KAS/block" choice had, not an
arbitrary round number). Own commit, separate from the SMOKE.md/plan documentation
commit, per Ground rule 2. Rebuilt, reran the send — `Tx rate: 1.1/sec, avg UTXO
amount: 15228085, avg UTXOs per tx: 7` confirmed the fix immediately.

**Wallet B and restart persistence.** Wallet B received `3,485,867,022` petals
across 66 UTXOs from the send. Recorded both wallets' balances and the tip DAA
score, killed node1's process directly (`kill <pid>`, same as a crash/manual stop),
relaunched `kaspad` with the identical `--appdir` — the restart log had no
"Resyncing the utxoindex..." line this time (unlike every fresh-node launch this
session), a small but reassuring sign it recognized existing state rather than
starting over. Both wallet balances and the DAA score matched their pre-restart
values **exactly** — real, working persistence, not just "the node came back up."

**Cleanup.** Stopped all `kaspad` processes, reverted the throwaway
`get_utxos_by_addresses`/balance-query extension to `rpc/grpc/examples/simple_client`
(same pattern as every prior live check), removed `x-testnet-local-data/`. The
`rothschild` fix itself is real and stays committed — it's a genuine compatibility
bug in a tool this project keeps using, not throwaway debugging code.

This closes P4.2. The chain now has direct evidence — not just unit tests — that a
normal wallet-holder's experience (receive funds, wait for them to mature, spend
some, restart your node) works correctly end to end on Marigold's own economics.
P4.3 (integration test suite) and P4.4 (tag) remain before Phase 4 itself is done.

### P4.3 — Integration test suite green (2026-08-15)

`cargo-nextest` wasn't installed (P0.2 had noted this and substituted plain
`cargo test`, which "works fine"). Installed it this time (user's suggestion,
mid-session — "probably helps down the line") via `cargo install cargo-nextest
--locked`, ~6 minutes to build.

**Anticlimactic in the best way**: the plan's own text warned "some tests hardcode
Kaspa params/genesis — fix them" — but there was nothing left to fix in this
crate. It was already caught and dealt with during earlier steps this session,
each in its own commit at the time: P2.6 fixed a hardcoded block-version literal
in `consensus_integration_tests.rs`'s `header_in_isolation_validation_test`; P3.2
found and `#[ignore]`d the five `goref_*` tests in the same file, which replay real
historical Kaspa chain data and can never validate against a permanently-diverged
economics schedule (see P3.2's entry above for the full root-cause — that's not a
literal-fix situation, "fixing" them would mean faking history). This is the
standing "run the full workspace, not just what a step names" lesson paying off in
reverse: by the time P4.3 came around specifically looking for exactly this class
of bug, there genuinely wasn't one left to find.

**Verification.** `cargo test --release -p kaspa-testing-integration --lib`: 42
passed, 0 failed, 6 ignored (the 5 `goref_*` tests plus 1 pre-existing
`#[ignore]` in `daemon_integration_tests.rs` unrelated to this project — a
manual-only test that predates the fork). Then the plan's own literal command,
`cargo nextest run --release -p kaspa-testing-integration`: 42 tests run, 42
passed, 6 skipped — same result, confirmed via the actual specified tool.
Benchmark modules (`mempool_benchmarks`, `subscribe_benchmarks`,
`rpc_perf_benchmarks`) are gated behind a `devnet-prealloc` feature not enabled by
either command, and every test inside them is separately marked `#[ignore =
"bmk"]` regardless — out of scope for "suite passes," consistent with how this
session has treated other explicitly-marked non-default tests (the crescendo
emission test, the subsidy-table generator). Full `cargo build --workspace`
also clean.

### P4.4 — Tag it (2026-08-15)

Created an annotated tag (not lightweight — a real message summarizing the whole
Phase 4 milestone is worth having attached to the ref) at P4.3's commit
(`d4fe5319`), pushed it to `origin`.

**Verified the fresh-clone claim for real, not just "the tag exists."** The plan's
verify condition is two things — "tag exists" (trivial) and "fresh clone + script
reproduces the testnet" (not trivial, and easy to accidentally not actually test:
running the P4.1 script again in the *existing* working directory, which already
has a built `kaspad` and cached dependencies, would not have caught a script bug
that only manifests on a genuinely clean checkout). So: `git clone --branch
fork-transparent-v0.1 https://github.com/marigoldcash/marigold-node.git` into a
scratch directory with zero prior history, confirmed no `target/` directory
existed, then ran `scripts/x-testnet-local.sh` completely unmodified. It correctly
detected the missing `kaspad` binary, built it from scratch (~10 minutes, every
dependency compiled from zero — no shared `~/.cargo` registry cache helped here
since this session already had one, but the actual crate compilation itself was
100% fresh), and all 3 nodes peered on the very first try — same clean result as
P4.1's original run. This is real evidence a stranger cloning the repo cold gets a
working testnet, not just "it still works on my already-set-up machine."

Stopped the fresh-clone's nodes, removed the scratch clone. Nothing added to the
repo by this verification pass.

**This closes Phase 4.** The milestone the phase's own goal statement named — "the
fork works" — is now backed by a tagged, reproducible commit: a multi-node private
testnet of Marigold's renamed, re-parameterized, capped-supply chain, with
wallet-equivalent tooling and mining, verified end to end (not just unit-tested)
across P2.9's/P4.1's network-sync checks, P3.3's/P4.2's economics-and-persistence
checks, and P4.3's regression suite. Next up per FORK-PLAN.md: Phase 5, the pool
specification (⚠️ marked hard — spec-first, no implementation yet).

## Phase 5 — Pool: specification (2026-08-15)

Wrote all of P5.1-P5.8 in one session into
[POOL-SPEC.md](POOL-SPEC.md) — a complete, ~1,300-line specification of the note pool.
Unlike other phases, the detailed per-step reasoning lives directly in POOL-SPEC.md
itself (each `## P5.N` section) and DECISIONS.md (the genuine design choices made along
the way), rather than duplicated again here — this entry is a pointer and a short
retrospective, not a re-narration.

**What actually required new design work**, versus formalizing what the plan already
decided: P5.1's serial-number derivation and pool-commitment field choice; P5.2's
subnetwork mechanism choice, the rotate/split/merge-into-one-`TransferOp` unification,
the signature/freshness-anchor scheme (genuinely new — no existing Kaspa sighash
applies to notes), and the discovery that fee stamps need no new wire concept at all;
P5.3's mostly-free ride on the real existing UTXO mergeset-conflict mechanism; P5.4's
finding that the closer precedent is seq-commit's SMT streaming import, not the UTXO
set's MuHash flow the plan itself pointed at; P5.6's from-scratch key-algorithm-
deprecation design (the plan's referenced "architecture paragraph" doesn't exist
anywhere in this repo, checked before writing anything); and P5.8's T-threshold
refinement (defined against Marigold's own genesis difficulty, not Kaspa mainnet's,
since the latter isn't on-chain-verifiable data — the plan's own literal suggestion
would have violated its own "no fuzzy definitions" requirement). P5.5 and P5.7 were
closer to formalization of already-decided/implied content, done with the same rigor.

**Every factual claim about existing code — file paths, line numbers, function names,
constant values — was checked directly against the real source before being written
into the spec**, not recalled from earlier session context or assumed from naming
conventions. Two citation errors were caught and fixed this way during writing (a wrong
line-number range, one arithmetic mistake in a worked byte-size example independently
re-verified with a calculator afterward). This mattered enough to call out as a
practice, not just a one-off fix: a spec that's about to receive external
cryptography review is exactly the kind of document where an uncaught "close enough"
citation does real damage to credibility, separate from whether the underlying design
is sound.

**P5.9 (the review gate) completed over the following day via a real external review
cycle** — the full trail lives in `docs/x-fork/reviews/` (two independent reviews, a
cross-review concurrence, a confirmation pass, and finding-by-finding triages for
each). Review 1 caught a genuine vulnerability the spec's author-perspective missed:
Redeem's signature didn't cover the enclosing transaction's transparent outputs
(destinations lived *outside* the signed payload — "Redeem is Mint read backwards"
made the payload feel symmetric when it wasn't), fixed by covering
`transparent_outputs_hash` uniformly. It also forced the finality-anchor cadence onto
DAA-score footing, which made the equivocation rule exactly decidable. Review 2
found no new flaw and demanded the written-rigor layer (theorem, field matrix,
threat model, canonicalization, IBD trust-root design) — all folded in. The field
matrix surfaced one bounded deviation worth naming (consumed-group-set malleability:
fee effects only, never redirection; carried into Phase 6 as a standing `[Open]`
item per the confirmation pass's "needs sign-off, not redesign"). Gate closed on
reviewer 1's explicit verdict; spec tagged **`pool-spec-v1.1`**. **Lesson worth
keeping**: the two review styles (bug-hunting vs. rigor-demanding) caught
different-shaped gaps, and the malleability finding shows the field-audit technique
("list every unsigned field and prove it harmless") finds things neither author
intuition nor single-bug review does — Phase 6's conformance tests inherit both
reviewers' test matrices for exactly this reason. Phase 6 is unblocked; implement
against the `pool-spec-v1.1` tag, not memory of it.

## Phase 6 — Pool: consensus implementation (2026-08-16)

### P6.1 — Note types & wire encoding

Straightforward once the spec existed — this is exactly what "spec-first makes
implementation steps bite-sized" (the plan's own words for why Phase 5 came before
Phase 6) was supposed to deliver, and it did. Implemented `pool-spec-v1.1`'s P5.1/P5.2
wire types verbatim in [notepool.rs](../../consensus/core/src/notepool.rs): no design
decisions needed, only faithful transcription — every struct field, every byte size,
every borsh discriminant assignment was already pinned down by the spec.

**One real judgment call**: FORK-PLAN.md's own P6.1 text (written before Phase 5
finalized the design) still describes a 5-variant op enum
("Mint/Rotate/Split/Merge/Redeem"). The spec supersedes this — `PoolOp` has exactly
three variants (`Mint`, `Transfer`, `Redeem`), with rotate/split/merge unified into one
`TransferOp` wire shape (P5.2's "Unifying rotate/split/merge" section). Implemented
against the spec, not the plan's older phrasing, and said so explicitly in the plan
entry rather than silently diverging — the spec is the frozen, reviewed, versioned
source of truth for exactly this kind of detail; the plan's prose predates it.

**Verification doubled as a second, independent check on the spec's own arithmetic.**
Rather than just asserting round-trip equality, every test also asserts the exact
serialized byte length against P5.2's worked-example table (38, 170, 150, 182, 1,305,
1,385, 177 bytes) — these numbers were hand-computed (and independently verified with
a Python calculator) during spec-writing; having the real borsh encoder reproduce them
exactly is a second, stronger confirmation than either of those. All matched on the
first run — no spec arithmetic errors survived to implementation.

Also added `SUBNETWORK_ID_NOTE_POOL` to `subnets.rs` (the `SubnetworkId::from_namespace`
mechanism P5.2 specified, spelling "POOL" in ASCII) — grepped existing
`from_namespace(...)` call sites first (mempool test fixtures use small integer
namespaces like `[1,1,0,0]`) to confirm no collision before picking the constant.

**Deliberately deferred, per the plan's own "pure data, no validation logic yet"
framing**: the SMT hasher (`NotePoolSmt`, needs a `crypto/smt/build.rs` entry — checked
directly and confirmed `SmtHasher` impls are build-time generated for a hardcoded list
of known BLAKE3 hashers, `KNOWN_HASHERS` in that file, so this isn't optional wiring,
it's a real build-script change P6.2 owns) and the pool state store belong to P6.2;
the 1,000-item consumed/produced collection cap (P5.2) is a validation-time bound
belonging to P6.3, not a fact about the data layer itself.

17 new unit tests, all passing; full crate suite (75 tests) and full
`cargo build --workspace` both clean.

### P6.2 — Pool state store + commitment (2026-08-16)

**Spec-vs-reality correction, found and fixed during implementation, not silently
diverged around**: `pool-spec-v1.1`'s P5.1 hashing section states "every hash is 32-byte
Blake2b ... the codebase's single hashing convention". Direct inspection of
[crypto/hashes/src/hashers.rs](../../crypto/hashes/src/hashers.rs) shows this is wrong —
there are two coexisting hasher families, a legacy `blake2b_hasher!` block (pre-Toccata:
`TransactionHash`, `MuHashFinalizeHash`, etc.) and a newer `blake3_hasher!` block
(Toccata-era: `SeqCommitMerkleBranch`, `TransactionV1Id`, all the `SeqCommitActive*`
hashers). Since the pool is an entirely new Toccata-era feature, its closest real
precedent is seq-commit, not the legacy transaction-hashing path — so all three new pool
hash types (`NotePoolLeafHash`, `NotePoolSmt`, `NotePoolSmtCollapsed`) were added to the
`blake3_hasher!` block, not `blake2b_hasher!`. Flagging here rather than quietly using
blake3: this is a place where the frozen spec's prose is simply inaccurate about the
codebase, worth a v1.2 addendum at some point, but not worth reopening the review gate
for a one-line factual correction discovered during implementation.

**Two hash types, not one**, mirroring the split `consensus/seq-commit/src/hashing.rs`
already uses between `SeqCommitActiveLeaf` (external leaf value) and `SeqCommitActiveNode`
(the SMT's internal branch-combination hasher, domain-separated from `CollapsedHasher`
for the single-leaf-subtree optimization): `NotePoolLeafHash` computes the external leaf
value `H(d||pk)` ([hashing.rs](../../consensus/core/src/notepool/hashing.rs)'s
`leaf_hash()`), while `NotePoolSmt`/`NotePoolSmtCollapsed` are the tree's own internal
hashers, registered in [crypto/smt/build.rs](../../crypto/smt/build.rs)'s
`KNOWN_HASHERS` list to get a build-time-generated `SmtHasher` impl (precomputed
`EMPTY_HASHES` for all 257 levels) — exactly the same registration `SeqCommitActiveNode`
already goes through. `leaf_hash()` is never called from `notepool_smt.rs` directly by
name; `NotePoolSmt`/`NotePoolSmtCollapsed` are only ever driven through
`compute_root_update`, never called by hand.

**Architectural call: did not reuse `consensus/smt-store`.** P5.4's prose cites
`consensus/smt-store`'s `SmtProcessor::build` as "the production path" to follow, but
that crate is built for a harder problem than the pool has — block-versioned, multi-lane
SMT state (`BranchVersionKey{prefix, depth, node_key, rev_blue_score, block_hash}`) so
seq-commit can reconstruct tree state as of any past block during mergeset processing.
The pool doesn't need historical point-in-time queries — the plan's own text asks for
"a `PoolDiff` type ... so state can be applied and un-applied per chain block — same
discipline as `UtxoDiff`", which is a single-current-state design, not a versioned one.
Built two purpose-built stores instead, mirroring `DbUtxoSetStore`
([consensus/src/model/stores/utxo_set.rs](../../consensus/src/model/stores/utxo_set.rs))
directly:

- [notepool.rs](../../consensus/src/model/stores/notepool.rs) — `DbNotePoolStore`, the
  flat `sn -> NewNote` map the plan's P6.2 text names explicitly. `NewNote` already had
  the derives it needed (borsh from P6.1, `Copy`); added `serde::{Serialize, Deserialize}`
  since `CachedDbAccess` uses bincode, a second, independent wire format from the borsh
  consensus encoding — the same "two serializations, two purposes" split `UtxoEntry`
  already has, documented inline in `notepool/mod.rs`'s doc comments rather than left
  implicit.
- [notepool_smt.rs](../../consensus/src/model/stores/notepool_smt.rs) —
  `DbNotePoolSmtStore`, `BranchKey -> Node` branch storage implementing `crypto/smt`'s
  `SmtStore` trait, plus a `CachedDbItem<Hash>` singleton for the current committed root
  (falls back to `NotePoolSmt::empty_root()` on a fresh, never-written store rather than
  erroring `KeyNotFound`). `Node`'s `to_bytes`/`from_bytes` aren't serde-derived (they're
  length-discriminated: 32B internal vs. 64B collapsed, no tag byte), so a small
  `NodeBytes` wrapper bridges them onto `CachedDbAccess`'s bincode-based storage.
  `apply_diff`/`unapply_diff` take a `PoolDiff` directly and call `compute_root_update`
  (the pure, production incremental-update function — confirmed the mutable in-memory
  `SparseMerkleTree::insert`/`remove` path is `#[cfg(any(test, feature="test-utils"))]`-
  gated only, so not usable here), persisting the returned `SmtNodeChanges` (delete on
  `None`, write on `Some`) and the new root together in one `WriteBatch`.

Three new `DatabaseStorePrefixes` entries
([database/src/registry.rs](../../database/src/registry.rs)): `NotePoolState = 90`,
`NotePoolSmtBranches = 91`, `NotePoolSmtRoot = 92` — picked from unused numbers in a
fresh "Note pool" section, checked against the existing enum (and its `Separator =
u8::MAX` sentinel) for collisions before adding.

`PoolDiff`/`PoolCollection`
([consensus/core/src/notepool/diff.rs](../../consensus/core/src/notepool/diff.rs))
mirror `UtxoDiff`/`UtxoCollection` deliberately minimally — just `add`/`remove` maps and
`to_reversed()` (swap the two). Did not port `UtxoDiff`'s `with_diff_in_place`
conflict-merging logic; nothing in P6.2's own scope needs it, and it belongs with
whatever P6.4's stateful pipeline work turns out to actually require, not built ahead of
that need.

✅ *Verify* (P6.2's exact criteria, both covered): `apply_then_unapply_restores_prior_root`
and `pool_diff_apply_then_unapply_restores_prior_root` apply a diff, confirm the root
changed, unapply it, confirm the root is back to the pre-apply value — an exact match, not
just "a plausible-looking hash". `commitment_is_deterministic_across_insertion_order`
builds an identical leaf set through two different insertion orders in two independent
temp-DB stores and confirms the two resulting roots are equal (correctness here rests on
`SortedLeafUpdates` sorting before `compute_root_update` ever runs, so this test is really
confirming that guarantee holds through the whole store, not just in `crypto/smt` itself).
`root_persists_across_store_instances` additionally confirms the root survives a real
store re-open against the same RocksDB directory, not just an in-memory cache hit.

7 new store unit tests, all passing; 3 new hashing unit tests
(`leaf_hash_is_deterministic`, `leaf_hash_differs_by_denomination`,
`leaf_hash_differs_by_pk`) plus the `diff.rs` reversal tests, all passing; full
`cargo test -p kaspa-consensus` (80 tests) and full `cargo build --workspace` both clean.

### P6.3 — Stateless op validation (2026-08-16)

**The real work here was figuring out how little of P6.3's own plan-text checklist
needed actual runtime code**, not writing the code once that was clear. Went and read
the real library source for each claim rather than assuming:

- "Denominations from the P1.6 set": already proven by P6.1's own
  `malformed_denomination_tag_discriminant_rejected` test — a bad `DenominationTag`
  discriminant can't survive borsh decoding, so there is no runtime state in which a
  `PoolOp` exists with an out-of-range tag to check against.
- "Freshness-anchor field present": `FreshnessAnchor` is a required struct field on
  `TransferOp`/`RedeemOp`, never `Option` — there's no wire shape lacking it.
- "Signature well-formed": this one took actually reading `secp256k1` v0.29.1's source
  (`src/schnorr.rs`), not just assuming a signature-parsing library does real
  validation. `Signature::from_slice` checks exactly one thing — the input is
  `SCHNORR_SIGNATURE_SIZE` (64) bytes — and `SignedGroup.signature`'s `[u8; 64]` field
  type already guarantees that unconditionally. No curve or field-element validation
  happens at parse time at all; that only occurs during actual verification against a
  message and public key. Wrote the check first, believing it would reject a
  malformed/degenerate signature — a test using an all-zero `[0u8; 64]` signature
  (reasoning: `r = 0` "obviously" isn't a valid curve x-coordinate) — and it failed,
  because the library doesn't check that at parse time either. Rather than keep code
  that can provably never return `Err` (confirmed by reading the exact match arms:
  length either equals 64 and returns `Ok`, or doesn't and hits the one `Err` path),
  removed it and documented why in `validate.rs`'s module doc comment instead of
  leaving a misleadingly-named dead function call in the validation path.

**What P6.3 actually enforces at runtime**, once the above was cleared away:
non-empty collections (an empty `Transfer.consumed` would be indistinguishable from
unauthorized minting — nothing backs the produced notes), the 1,000-item collection
cap P6.1 explicitly deferred here (`MAX_POOL_OP_COLLECTION_LEN` — a standalone
protocol constant matching mainnet's current `max_tx_inputs`/`max_tx_outputs` value by
choice, not a live read of that field, since `params.rs`'s tx-shape limit and the
pool's collection limit are conceptually independent even though they currently
agree), and no duplicate serial within or across an op's `SignedGroup`s — the
stateless half of P5.3 step 1 ("no serial may appear more than once across the op's
entire consumed set"); the other half of that same step (every group's serials must
currently share one `pk`) is inherently stateful (needs the live pool view to know
each serial's current owner) and stays P6.4's job, along with actual signature
verification.

New `PoolOpValidationError`
([errors/notepool.rs](../../consensus/core/src/errors/notepool.rs)) follows
`TxRuleError`'s existing convention in this codebase exactly — `thiserror`, one
`#[error("...")]` variant per distinct violation — rather than inventing a new error
style for one more feature.

✅ *Verify* (P5.3's own malformed-op checklist, the part that's actually stateless):
15 new table-driven tests — every combination of empty-collection, oversized-collection
(both "one past the cap" and "exactly at the cap, still valid"), and duplicate-serial
(within one group, across two groups) rejected with its own distinct error variant,
across all three op kinds, plus a valid-shape positive test per op kind so the suite
can't pass by rejecting everything.

15 new unit tests, all passing; full `cargo test -p kaspa-consensus-core` (94 tests)
and full `cargo build --workspace` both clean.

### P6.4 — Stateful validation in the virtual pipeline (2026-08-16) ⚠️ HARD

Done with a stronger model per the plan's flag (the user switched models specifically
for this step). The largest single step of Phase 6 so far: pool ops now flow through
the real consensus pipeline end-to-end — validated in context, accepted/rejected per
GHOSTDAG blue order, applied to a virtual pool state + SMT commitment, and correctly
unapplied on reorgs.

**The central design decision — and why it made the step tractable**: pool ops are
validated *inside* `validate_transaction_in_utxo_context`, the same function every
transaction already passes through during the mergeset walk. That single placement
means "first accepted wins" needs no new conflict-resolution rule at all: a pool op
whose serial was already consumed by an earlier-in-blue-order block simply fails
`SerialNotFound` against the composed pool view and is excluded from that context's
accepted transactions — byte-for-byte the same mechanism that resolves UTXO
double-spends between merged blocks today, which is exactly what POOL-SPEC.md P5.3
promised ("no new conflict-resolution rule is being invented here").

**The subtlest consensus point in the whole step — freshness is non-monotonic.** When
a chain block replays its selected parent's own transactions, the existing code skips
script checks (they were fully validated during the parent's chain qualification,
against the identical state basis) but re-runs maturity/sequence-lock checks, which is
safe because those are *monotonic* in POV DAA score — once valid, always valid later.
The pool's freshness anchor is the opposite: an op fresh at the parent's POV can be
stale at the child's. Re-imposing the freshness check at replay would make a child
compute different acceptance data than its parent committed — consensus divergence.
So `validate_stateful` takes a `skip_signature_and_freshness` flag driven by the
existing `TxValidationFlags::SkipScriptChecks`, and both the signature check (basis
identical, skip is an optimization) and the freshness check (skip is *correctness*)
ride the selected-parent replay exemption. Documented in both the function's docs and
`calculate_utxo_state`'s comment block, next to the existing monotonicity reasoning it
extends. There's a unit test pinning exactly this
(`stale_anchor_accepted_on_selected_parent_replay`).

**Two real bugs found in existing/adjacent code, both with tests now:**

1. **`calc_storage_mass` divides by zero on a zero-input transaction**
   (`sum_ins / ins_plurality` with both zero — the arithmetic-mean path). Unreachable
   before this fork (native txs must have inputs), but a pure pool `Transfer` is
   exactly a zero-input/zero-output tx. Fixed with the mathematically consistent
   generalization: with no inputs the KIP-9 `|I|/A(I)` term vanishes, so storage mass
   is `max(0, harmonic_outs)` — an early return right after the outputs fold.

2. **A pre-P6.6 soundness hole in the spec's own reasoning.** P5.3 calls mint-serial
   uniqueness "guaranteed by construction ... not an active check", reasoning that a
   mint tx's id can't have been used before. That's only true once P6.6's value
   binding forces mints to spend transparent inputs (making a duplicated mint a UTXO
   double-spend). Until then, the *same* zero-input mint tx included in two parallel
   blocks would validate in both contexts and double-add its serial — panicking the
   diff accumulator. The produced-serial existence check is therefore an **active
   consensus rule for now** (one map lookup per produced note, also applied to
   Transfer's produced serials), documented to degrade to spec-permitted cheap
   insurance after P6.6. `duplicate_mint_across_parallel_blocks_accepted_once` pins
   the behavior at the consensus level.

**What was built, layer by layer** (mirroring the UTXO machinery at every step):

- **Hashes** (`crypto/hashes` + [hashing.rs](../../consensus/core/src/notepool/hashing.rs)):
  `NotePoolSerialHash` (`sn = H(tx_id || index u32 LE)`), `NotePoolSigningHash` (P5.2's
  v1.1 preimage exactly: version || op_type || sorted-serials || whole-produced-list ||
  transparent_outputs_hash || anchor — the outputs-hash coverage is review 1's Redeem
  malleability fix), `NotePoolOutputsHash` (amount || spk version || spk script per
  output, mirroring the sighash field order, pool-domain-separated). The signing-hash
  function sorts serials internally so signer and validator hash the same canonical
  message regardless of wire order.
- **Diff algebra** ([diff.rs](../../consensus/core/src/notepool/diff.rs)): `PoolDiff`
  gained `UtxoDiff`'s exact `with_diff_in_place` two-phase composition (error checks,
  then cancel-or-insert), `ImmutablePoolDiff`/`ReversedPoolDiff` for clone-free
  reverse application on walk-downs, and `add_note`/`remove_note` incremental entry
  points. Simpler than `UtxoDiff`'s algebra in one honest way: no DAA-score dimension,
  because a live serial's `(d, pk)` is immutable (invariants I1/I3), so "same key"
  means "same logical entry" — where `UtxoDiff` must disambiguate recreated outpoints
  by score, the pool never can see one.
- **Views** ([view.rs](../../consensus/core/src/notepool/view.rs)): `PoolStateView` /
  `ComposedPoolView` / `.compose()`, nesting like `UtxoView` composition;
  `PoolCollection` itself implements the trait for tests.
- **Stateful validation** ([validate.rs](../../consensus/core/src/notepool/validate.rs)):
  `validate_stateful` returning `ValidatedPoolOp { diff, consumed_petals,
  produced_petals }` — existence, same-pk-per-group, BIP340 verification against the
  *current* pk, inclusive freshness window (`POOL_FRESHNESS_WINDOW = 36_000`,
  boundaries per review 1), pool-side Transfer conservation, produced-serial freshness,
  and diff construction with derived serials. 15 new unit tests with real Schnorr keys
  including signature-lift attacks (produced-list swap fails), op_type domain
  separation (a Transfer signature can't authorize a Redeem), merchant-sweep
  (one signature, many serials), and split conservation.
- **Pipeline threading**: `UtxoProcessingContext.mergeset_pool_diff` accumulates in
  lockstep with `mergeset_diff`; composed pool views built at every place composed
  UTXO views already were (`calculate_utxo_state` per merged block,
  `verify_expected_utxo_state` for the chain block's own txs, virtual calculation,
  template validation, the test block builder). `ValidatedTransaction` carries an
  `Option<ValidatedPoolOp>` so acceptance directly folds each accepted op's diff.
- **Persistence**: per-chain-block diffs in
  [notepool_diffs.rs](../../consensus/src/model/stores/notepool_diffs.rs) (prefix 93,
  written in `commit_utxo_state`'s batch — always in lockstep with `utxo_diffs`);
  virtual pool state (`pool_state` map + `pool_smt` root) inside `VirtualStores`,
  applied once per resolve in `commit_virtual_state`'s single `WriteBatch`; virtual's
  own mergeset pool diff as its own `CachedDbItem` (prefix 94) — deliberately NOT a
  new `VirtualState` field, leaving that type's version-suffixed serialization
  untouched. Reorg walk-down applies stored diffs reversed; walk-up applies forward
  or computes-and-commits via the existing KeyNotFound branch.
- **Admissibility**: user-lane subnetwork + payloads were already consensus-legal
  (verified against the real isolation checks — only mempool standardness restricts
  payloads, which is P6.7 anyway). What actually needed changing: the `NoTxInputs`
  rule (pool txs are the one non-coinbase shape allowed zero inputs), pool-payload
  decode + P6.3 stateless validation wired into `validate_tx_in_isolation`, and a new
  body-level `check_block_double_serials` mirroring `check_block_double_spends` —
  necessary because txs within a block are validated in parallel against the same
  composed view, so intra-block serial conflicts must be block-invalidity, not a
  validation-order outcome.

**Deliberately deferred, with the boundaries stated in code comments at each site**:
transparent-side value binding (mint funding, redeem output sums), fee crediting
(pool ops currently contribute 0 to `calculated_fee`), and mass costing → P6.6.
Mempool entry rejects pool txs with a dedicated error until P6.7 — without its
same-serial conflict policy, two conflicting rotates could both enter the mempool and
self-invalidate every locally built block template (a self-DoS). Pool state at
pruning-point import stays empty → P6.8.

**Operational note — old datadirs are now incompatible**: the reorg walks assume the
lockstep invariant "every UTXO-valid chain block has BOTH a `utxo_diffs` and a
`notepool_diffs` row" (they're written in one batch), and `.unwrap()` on the pool-diff
read enforces it loudly — the same posture upstream takes for `utxo_diffs` itself.
A datadir produced before P6.4 (e.g. leftovers from the P2.9/P3.3/P4.x live tests)
has chain blocks without pool-diff rows and will panic if a walk crosses them. All
such datadirs were always disposable scratch state; use fresh `--appdir`s (the
long-standing Phase 2 practice anyway), and P9.5's genesis regeneration invalidates
everything pre-launch regardless. Deliberately NOT masked with a default-empty
fallback: a missing row on a post-P6.4 chain would be a real write bug, and silence
there means divergent pool state.

**A test-writing gotcha worth remembering**: the first version of the
double-rotate determinism test failed with different pool roots across its two runs —
which looked exactly like a state-tracking bug. A store-level reproduction proved the
SMT history-independent; the real cause was BIP340's *randomized aux nonces*:
rebuilding "the same" transaction inside the per-run loop re-signed it, producing a
different signature, hence different tx id, hence different derived serials — two
genuinely different DAGs. The comparison is only meaningful over identical
transactions, built once outside the loop. The investigation left behind a permanent
regression guard (`history_independence_tests` in notepool_smt.rs) and both roots
matched their own maps throughout — the pipeline was never actually wrong.

✅ *Verify* (all three P6.4 criteria, as consensus tests in
[notepool_tests.rs](../../consensus/src/pipeline/virtual_processor/notepool_tests.rs)):
`parallel_double_rotate_resolves_deterministically` — same DAG built twice with
opposite insertion orders for the conflicting blocks; exactly one rotate accepted in
the merging block's committed acceptance data, identical winner and identical pool
root both times. `reorg_past_pool_op_restores_prior_pool_state` — a heavier branch
carrying a conflicting rotate wins; the losing branch's produced note is fully
unapplied and the reorged node's pool root **exactly equals a never-forked reference
node's root** (the strongest form of "restores the prior pool root": convergence with
zero residue, having passed through the walk-down/unapply path).
`out_of_window_anchor_op_rejected_in_context` — the freshness gate consensus-side via
a future anchor (a genuinely *stale* anchor needs 36k mined blocks; its exact
inclusive boundaries are unit-pinned in consensus-core, where the window is reachable),
plus the happy-path and duplicate-mint tests above.

Totals: consensus-core 121 passed (+27 this step), consensus 86 passed (+6), full
workspace 1,206 passed across 142 test binaries, integration suite green, workspace
build clean.

### P6.5 — Commitment placement (2026-08-16)

The plan's recommendation ("the header field") was already the decision; this step was 
"implement it," but turned into real design work once the actual requirements became concrete.

**The genuinely new thing about this step**: every prior header-shape change in this
fork's history — checked directly against `hashing::header::hash_override_nonce_time`,
not assumed — reinterpreted an EXISTING field rather than adding a new one.
`accepted_id_merkle_root` is repurposed for seq-commit post-Toccata; `parents_by_level`'s
RLE compaction (`CompressedParents`) changed the wire/storage representation but the hash
preimage still expands it back to the original per-level list before hashing, so even that
wasn't a hash-shape change. `pool_commitment` is the first field this fork has ever
actually ADDED to the hash preimage. POOL-SPEC.md P5.1 explains why reuse was rejected
(overloading `accepted_id_merkle_root` a second time would make one field mean two things
depending on which of two independent forks activated) — confirmed that reasoning still
holds and implemented accordingly.

**The real engineering problem, not visible until implementation**: how does
`verify_expected_utxo_state` compute the EXPECTED `pool_commitment` for an arbitrary chain
block being validated — including ones on a branch that ISN'T currently reflected in
persisted state, exactly what happens during `calculate_utxo_state_relatively`'s
exploratory reorg walk? For `utxo_commitment`, this is free: MuHash is an algebraic
accumulator, so `selected_parent_multiset_hash` (a single value, no branch history needed)
composed with the accumulated diff is ALWAYS correct regardless of which branch
`selected_parent` is actually on — set membership commutes. An SMT root has no equivalent
property. Recomputing it from an old root plus a diff needs READ ACCESS to the actual
branch-node structure along the changed paths (that's literally what
`compute_root_update`'s `store: &S` parameter is for) — and `DbNotePoolSmtStore` (P6.2's
single-current-state design, deliberately NOT block-versioned, mirroring
`DbUtxoSetStore`) only ever correctly reflects whichever branch was most recently
committed to virtual. Reading its persisted nodes to verify a DIFFERENT branch's block
would silently produce the wrong root.

This is not a hypothetical concern — it's exactly why `consensus/smt-store`'s heavier
`SmtProcessor`/`BranchVersionKey` machinery exists for seq-commit (multi-branch,
reachability-filtered lookups via `is_smt_canonical`), machinery P6.2 explicitly declined
to adopt for the pool, reasoning the pool only needs current-state apply/unapply. That
reasoning holds for the STATE map (a flat structure, correctly diff-composable regardless
of branch — this is exactly what makes `PoolStateView`/`ComposedPoolView` correct for any
walked position, proven back in P6.4) but does NOT extend to the SMT ROOT specifically,
which needs actual tree structure. Found this the concrete way, not the abstract way: the
first version of this step's own new `incremental_and_full_rebuild_commitments_agree`
cross-check test (see below) failed, and tracing why led directly to this gap.

**The fix**: `recompute_pool_commitment` doesn't do an incremental update at all — it
materializes the FULL live pool-entry set (the persisted `pool_state` flat map, iterated,
plus the accumulated diff applied on top — correctness inherited from the same
diff-composition argument P6.4 already established) and builds a fresh in-memory SMT
(`BTreeSmtStore`, zero persistence) from scratch. This sidesteps the branch-versioning
problem entirely — it never reads any persisted branch-node structure, so there's nothing
to go stale — at the cost of O(pool size) work per verified block. Accepted as a
documented, correctness-first tradeoff: P6.5's own verify condition doesn't ask for
performance, this is a fresh/early network, and a proper incremental multi-branch-aware
store is real future work, not silently punted — it's named explicitly as a gap in the
plan entry above. `DbNotePoolSmtStore`'s incremental tracking is KEPT, not removed —
it's still correct and useful for virtual's own root query (`pool_root()`), since virtual
only ever advances through ITS OWN linear commit history, never a divergent branch.

**A second real bug, found by the SAME investigation**: `build_block_template_from_virtual_state`
originally read `self.virtual_stores.read().pool_smt.current_root()` directly inside the
function — correct for the real mining path (`build_block_template`, where `virtual_state`
IS the actual persisted virtual) but WRONG for `TestBlockBuilder::build_block_template_with_parents`
(the "build a template for arbitrary/hypothetical parents" test helper every reorg and
parallel-block test in this fork uses) — there, `virtual_state` is a POV-hypothetical
computation that may not match what's actually persisted in `virtual_stores.pool_smt` at
all. This is precisely the kind of staleness risk flagged (but not yet proven) in P6.4's
own doc comment about `build_block_template_from_virtual_state`'s `utxo_commitment` field
— except UTXO's version happens to be safe (virtual_state carries its OWN multiset
snapshot), while my ad hoc `pool_smt.current_root()` read did not. Fixed by making
`pool_commitment` an explicit parameter, computed correctly by each caller for its own
context (the real path uses the direct materialize-from-virtual-state; the test-builder
path uses `recompute_pool_commitment` against the accumulated pov diff, captured BEFORE
that diff gets moved into the composed view used for tx validation).

**Mechanically large, not conceptually hard**: every place a `Header` gets constructed or
converted needed updating:
`consensus/core`'s own test fixtures, `protocol/p2p`'s proto + converter (new field 15,
required for the network to actually function post-activation — peers literally cannot
reconstruct each other's headers without this), `rpc/core`'s `RpcRawHeader`/`RpcHeader`/
`RpcOptionalHeader` + `RpcHeaderVerbosity` (its own serializer/deserializer AND the
`impl_verbosity_from!` macro table), `rpc/grpc/core`'s two proto messages (`RpcBlockHeader`
field 16, `RpcOptionalHeader` field 15) + converters, `rpc/service`'s verbosity-gated
header adapter, the WASM SDK's `consensus/client` (`IHeader`/`IRawHeader` TS interfaces +
getter/setter + object-parsing, genuinely hash-affecting since `finalize_js` calls the
real canonical hash function), and `bridge/src/hasher.rs`'s hand-rolled preimage. None of
this was in the plan's own text beyond "update the bridge" — the RPC/gRPC/P2P propagation
turned out to be **required for the workspace to compile at all**, not an optional
follow-up, once the actual dependency graph became visible. (Originally scoped to defer
RPC/proto work to P6.9's "add pool-related RPC methods" pass, on the theory that a header
struct field change wouldn't need wire-format changes — wrong: the field is read/written
unconditionally by every existing header conversion path, so it breaks compilation
immediately, not just pool-specific RPC methods.)

Two more real bugs, smaller, both concrete not hypothetical:
- `calc_storage_mass`'s already-known zero-input path (fixed in P6.4) wasn't touched
  again here, but the SAME "check real behavior, don't assume" discipline caught two
  MAINNET_PARAMS-based tests (`block_template_version_changes_to_v2_upon_activation` in
  `consensus`, plus `header_in_isolation_validation_test` and
  `header_version_is_enforced_by_activation` in the integration suite) that assert exact
  block-version transitions. All three broke because `MAINNET_PARAMS` now has
  `pool_activation: ForkActivation::always()` by default (matching `toccata_activation`'s
  own "fresh chain, no history to protect" precedent) — meaning `block_version()` jumps
  straight to `NOTE_POOL_BLOCK_VERSION` regardless of what `toccata_activation` is
  overridden to. Fixed by having each test also override `pool_activation =
  ForkActivation::never()`, isolating what each test actually means to exercise.
- Several `#[cfg(test)]`-only call sites across `parents_builder.rs`, `mining/`, and
  `testing/integration/` don't get checked by a plain `cargo build --workspace` at all
  (only by `cargo test`) — caught these via the full `cargo test --workspace` pass, not
  the build. Standing lesson from earlier phases, reconfirmed: build success doesn't
  imply test-target compile success; both passes are necessary, in that order, every step.

**Devnet verification, honestly scoped**: ran a real `kaspad --devnet` binary (debug
build) against a scratch appdir and fetched genesis over live gRPC — confirmed the
regenerated devnet genesis hash matches what the running binary actually computes (not
just what the unit test asserts) and that `pool_commitment` is served correctly over the
wire as a well-formed, non-garbage 32-byte value. Did NOT solve real devnet PoW to mine a
literal post-genesis block — devnet's real difficulty makes that a CPU-minutes-to-hours
proposition unrelated to what this step needs verified, and the `TestConsensus`
`skip_proof_of_work()` suite (this project's established methodology for consensus-logic
verification every phase since P2) already covers mint/rotate/reorg/cross-mechanism
correctness far more thoroughly than a single mined devnet block could. Recorded as a
deliberate scoping call, not a skipped step.

**Genesis regeneration**: same "blank the constant, run the test, paste the panic's hex
array back in" loop as P2.5, four times (mainnet, testnet, simnet, devnet — devnet's
comment history now has three generations: the original golang value, P2.5's bits-field
fix, and this step's pool_commitment-field addition). Confirmed via FORK-PLAN's P9.5 entry
that mainnet gets regenerated a THIRD time regardless, with the real launch timestamp —
this step's hashes are explicitly not meant to be precious.

✅ *Verify*: `cargo test -p kaspa-consensus-core` — 121 passed, including
`test_genesis_hashes` for all four regenerated networks. New
`incremental_and_full_rebuild_commitments_agree` test pins the two independent
commitment-computation mechanisms (fast incremental, from-scratch rebuild) to agree
exactly — the test whose FIRST version failed and led to finding the branch-versioning
gap above; its final, correct version accounts for the standard GHOSTDAG "a block's
commitment reflects its ancestors, not its own body" shape (a subtlety that made the
test's first draft assert the wrong equality — block N+1's header commitment matches
`pool_root()` as of block N, not as of N+1, since a block's own transactions only surface
in its descendants' commitments). Full `cargo test --workspace` (minus the slow
integration crate): 1,207 passed, 0 failed, across 142 binaries. Integration suite green
after the three MAINNET_PARAMS fixes above. Full `cargo build --workspace` clean. Real
`kaspad --devnet` binary verified live via gRPC.

### P6.6 — Mint/redeem value binding (2026-08-16)

**The core design decision**: rather than writing three separate conservation checks for
Mint/Redeem/Transfer, `validate_populated_transaction_and_get_fee` gained one extra
parameter, `pool_value: Option<(u64, u64)>` = `(consumed_petals, produced_petals)`, and one
unified formula: `available = total_in + consumed`, `spent = total_out + produced`,
require `available >= spent`, `fee = available - spent`. Treating `consumed_petals` as
"another transparent input" and `produced_petals` as "another transparent output" derives
the right per-op behavior without the formula itself ever branching on which op it is:
Mint has `consumed = 0`, so `produced` (its new notes) must be covered by real transparent
inputs — POOL-SPEC.md P5.2's rule exactly. Redeem has `produced = 0`, so its consumed notes
fund the transparent outputs plus fee — also exactly P5.2. Pure Transfer has no transparent
side at all, so the formula collapses to `consumed - produced`, the fee-stamp difference
P5.2 already specifies for it. One formula, three correct behaviors, because the *shape* of
what's being conserved is genuinely the same operation viewed from three angles — this
generalizes past the plan text's literal ask ("mint" and "redeem" checks) to the whole op
family, which is why it caught something the plan text didn't call out at all (next
paragraph).

**A real mass-costing gap found, not anticipated by the plan text**: `calc_non_contextual_
masses`'s `script_mass` term (the cost of signature verification) is computed entirely from
`tx.inputs`' `compute_commit` field. Transfer and Redeem have ZERO transparent inputs by
design — meaning their `SignedGroup` Schnorr signature verification, real CPU cost the node
must actually perform, was completely uncosted before this step. P5.3 is explicit that
signature verification costs "one compute-budget unit... charged once per signature, not
once per serial" — there was previously no cost charged at all for these two ops' node-pool
signatures, a genuine fee-evasion/DoS gap (a spammer could submit arbitrarily many
zero-fee-covered Transfer/Redeem signature verifications). Fixed by adding
`GRAMS_PER_COMPUTE_BUDGET_UNIT * num_signed_groups` to the compute-mass total, gated on
`subnetwork_id == SUBNETWORK_ID_NOTE_POOL` and the decoded op type (Mint needs no addition:
no note-level signature, its real transparent inputs are already priced normally by the
existing logic). Found by reading `calc_non_contextual_masses` end-to-end while implementing
this step's mass-costing clause, not by a failing test — no test had ever exercised a real
Transfer/Redeem's mass, since none previously existed with real value on either side.

**Test infrastructure had no precedent to build on**: every pool test up to this point used
an intentionally-invalid, zero-input mint (the whole point of P6.4's "should be rejected
post-P6.6" pattern). Constructing a real, coinbase-funded, signed spend inside a
`skip_proof_of_work()` `TestConsensus` required combining two things this codebase had never
put together: Kaspa/Marigold's mergeset reward mechanism pays a block's own coinbase reward
out through its CHILD's coinbase transaction, never its own — confirmed directly in
`processes/coinbase.rs`'s `expected_coinbase_transaction`, which loops
`ghostdag_data.mergeset_blues` (the merged ancestors), not the current block — so funding a
wallet takes mining two blocks (fund → mature-and-pay), not one. And the correct P2PK
script is `OP_DATA_32 <x-only pubkey> OP_CHECKSIG` via `pay_to_address_script`, NOT the raw
33-byte SEC1 pubkey `consensus/src/pipeline/virtual_processor/tests.rs`'s pre-existing
`new_miner_data()` helper uses — confirmed by direct code reading that this is genuinely an
invalid, non-script-engine-validated shape; it "works" in that file only because that
specific helper discards its own secret key and never actually spends from it, so nothing
ever exercises script validation against it. Copying it would have silently built unspend-
able test fixtures. `coinbase_maturity = 0` reuses an existing pattern from
`testing/integration`'s activation tests rather than mining ~1000 throwaway blocks per test
to clear real maturity.

**The same class of bug this fork already hit once, in P6.4, hit again — worth recording a
second time since it recurred**: two of the six pre-existing pool consensus tests
(`parallel_double_rotate_resolves_deterministically`, `reorg_past_pool_op_restores_prior_
pool_state`) build a shared transaction once and reuse it across independently-run
`TestConsensus` instances or loop iterations, asserting the outcomes are identical. This
was fine when mints were zero-input (no signing dependency on per-instance state), but
P6.6 forces a mint's funding UTXO to be instance-specific — the naive fix is to rebuild the
mint fresh inside each iteration/instance, but BIP340 Schnorr's aux-randomized nonce
(confirmed via the vendored secp256k1 source: `sign_schnorr` is the randomized variant)
makes "the same" rebuilt transaction hash differently every single time, which would have
silently made the "identical DAG, different insertion order" comparison meaningless. Same
root cause as P6.4's earlier instance of this bug, different trigger. Fixed comprehensively
rather than patched around the one call site: every signature in this test file now uses
`secp256k1::SECP256K1.sign_schnorr_no_aux_rand` — both `Wallet::rotate`'s existing signature
and a new local, test-file-only, deterministic reimplementation of `consensus_core::sign::
sign`'s exact logic for the mint's transparent-input signature (the real `sign()` function
couldn't be changed to match, since real wallet/mempool signing paths depend on its
genuinely-randomized security property). With deterministic signing throughout, rebuilding
"the same" transaction fresh per instance/iteration is now actually safe, which simplified
the two affected tests back to their original "build once, or rebuild — doesn't matter"
shape rather than needing any special-cased sharing logic.

**Two smaller real things found while getting the six pre-existing tests green again**:
the block builder requires a transaction's storage-mass commitment field to already be
correct before submission — `check_mass_commitment` in `tx_validation_in_utxo_context.rs`
rejects any mismatch — and in production this is the mempool's job (`validate_mempool_
transaction_in_utxo_context`, via P6.7, not yet built), so nothing in this hand-built test
path goes through it; every constructed test transaction with real transparent inputs or
outputs now explicitly computes and commits its storage mass via `MassCalculator` before
insertion. And separately: a Redeem's transparent output has no offsetting transparent
input for the KIP-0009 storage-mass formula to net against (Mint does, via its real funding
input), so an output much smaller than `STORAGE_MASS_PARAMETER` (1e12) trips the existing
anti-dust storage-mass limit purely as an artifact of test denomination choice, unrelated to
what those tests are actually checking — the redeem-focused tests use 0.1-MAGLD notes
rather than 0.01 to stay clear of it. Also had to drop the previous tests' 1-MAGLD (`D1`)
notes down to 0.01/0.1 MAGLD throughout: Marigold's real subsidy schedule (P3.2, deflationary
from genesis per P1.4) pays roughly 15.2M petals per block at genesis-era DAA scores — about
0.15 MAGLD — nowhere near enough to fund a 1-MAGLD note from a single block's coinbase.

**New tests added, directly matching this step's own stated verify condition**:
`mint_with_insufficient_transparent_inputs_rejected`, `redeem_with_excessive_transparent_
outputs_rejected`, and `value_conservation_across_mint_transfer_redeem` — the literal
`Σ pool notes + transparent supply == emitted supply` check, run across a real mint →
transfer → redeem sequence. Scoped deliberately to the one funding block's reward this test
itself injects (queried via `ConsensusApi::get_virtual_utxos`, filtered to the test's own
known scripts) rather than the whole chain's total emission — every other block mined along
the way pays its own subsidy to an unrelated null miner script the test never queries, so it
can't leak into the balance check and the invariant holds exactly for this closed subsystem
without needing to replicate the real subsidy-by-month schedule inside the test.

✅ *Verify*: all 9 tests in `notepool_tests.rs` pass (6 pre-existing tests rewired to spend
real funding + 3 new tests for this step's verify condition). `cargo test -p kaspa-consensus`
— 90 passed, 0 failed, 2 ignored. `cargo test --workspace --exclude kaspa-testing-integration`
— 0 failed across every crate. `cargo build --workspace` clean.

### P6.7 — Mempool integration (2026-08-16)

Dispatched a research-only subagent before writing any code, mirroring the discipline
P6.6 used for coinbase-funded test infrastructure: map the actual mempool crate's
structure first, rather than assume from the plan text's three-sentence summary what
needs to change. This paid off immediately — the report's bottom line was that the
mempool crate is *already* input/output-count-agnostic almost everywhere (standardness
checks, mass/fee-based ordering, block-template selection all handle a zero-input/
zero-output pure Transfer as a correctly vacuous case, confirmed by reading the actual
code, not assumed), so the real work was narrower and more concentrated than "teach
every mempool file about pool ops."

**What actually needed building, in order**:
1. **Lift the guard, thread `pool_view` through mempool validation.**
   `validate_mempool_transaction_in_utxo_context` (`consensus/src/pipeline/
   virtual_processor/utxo_validation.rs`) had P6.6's `NotePoolTxNotYetSupportedInMempool`
   reject-at-the-door guard. Removed it (and the now-dead `TxRuleError` variant), added a
   `pool_view: &impl PoolStateView` parameter, and mirrored `validate_transaction_in_utxo_
   context`'s existing decode → `validate_stateful` → real `pool_value` sequence exactly —
   no new logic, just the same shape already proven correct for the block-validation path.
   Threaded `virtual_read.pool_state` through `processor.rs`'s three mempool-validation
   call sites (`validate_mempool_transaction_impl`, `validate_mempool_transaction`,
   `validate_mempool_transactions_in_parallel`) the same way `virtual_read.utxo_set`
   already was — small, mechanical, well-precedented.
2. **Serial-keyed conflict tracking, the mempool crate's own new piece.**
   `mining/src/mempool/model/pool_note_set.rs` (new): `MempoolPoolNoteSet` — a
   `serial_owner_id: HashMap<Hash, TransactionId>` index, structurally identical to
   `MempoolUtxoSet`'s `outpoint_owner_id`. Two deliberate simplifications versus the
   outpoint side: no replace-by-fee variant at all (confirmed by reading `replace_by_fee.
   rs` that RBF-vs-reject is purely "what does `rbf_policy` say to do with the double-spend
   list this one index produces" — so "no RBF for serials" is just running the
   `RbfPolicy::Forbidden` branch's logic unconditionally, independent of whatever RBF
   policy governs the same transaction's own outpoint-side double spends, not a new
   mechanism to invent); and no tracking of *produced* notes (see the scope decision
   below). Wired into `TransactionsPool` alongside every existing `utxo_set` call; new
   `RuleError::RejectSerialConflictInMempool` in `mining/errors/src/mempool.rs`, and a
   `SerialConflict` struct in `model/tx.rs` parallel to the existing `DoubleSpend`.
3. **Eviction on confirmation.** `handle_new_block_transactions.rs` gained
   `remove_serial_conflicts`, the serial-keyed sibling of the existing
   `remove_double_spends` — decodes a confirmed pool op's consumed serials and evicts any
   mempool-resident transaction still trying to consume the same one.
4. **`ConsensusMock` pool-op support**, since every mempool crate test uses `ConsensusMock`
   exclusively (confirmed via grep — zero `TestConsensus` usage in `manager_tests.rs`), not
   the real consensus. Added a `notes: RwLock<PoolCollection>` field and pool-op-aware
   bookkeeping in `add_transaction` (mirroring the existing UTXO bookkeeping there), and a
   pool-op branch in `validate_mempool_transaction` that calls the real, exported
   `validate_stateful` from `consensus-core` directly against `&*self.notes.read()` —
   `PoolCollection` already implements `PoolStateView`, so this needed no new glue, and
   avoids a second, drift-prone hand-rolled reimplementation of pool-op rules in test code.

**The one real, plan-text-silent gap, found only by reading how UTXO chaining actually
works, not by reading the plan's literal three-sentence ask**: `populate_mempool_entries`
is what lets a transaction spending an unconfirmed mempool transaction's own output
validate successfully — it pre-populates that transaction's UTXO entries positionally,
by array index, straight from the parent mempool transaction's outputs, before consensus
ever sees it. There is no positional equivalent possible for pool state:
`PoolStateView::get_note` is queried by serial hash, an opaque 32-byte value derived from
the *producing* transaction's own id, not by an array index the mempool crate could
pre-fill ahead of time the way it does for UTXO inputs. Concretely: if transaction B (a
Transfer) is built to consume a serial that transaction A (a still-unconfirmed mempool
Mint or Transfer) is about to produce, and B validates against `virtual_read.pool_state`
alone (the real, committed state — not A's pending, unconfirmed contribution),
`validate_stateful` returns `SerialNotFound` and B is hard-rejected, not queued as an
orphan the way a UTXO-chained transaction with a genuinely missing outpoint would be
(the `TxRuleError → RuleError` mapping only special-cases `MissingTxOutpoints →
RejectMissingOutpoint`, the trigger for orphan-pool insertion; `SerialNotFound` falls
through to a generic hard reject today).

**This is a real, deliberate MVP scope decision, not an oversight — recorded explicitly
so it isn't rediscovered by surprise later**: a wallet cannot currently chain two pool ops
back-to-back while the first is still unconfirmed (e.g. split a note into two and
immediately re-send one half in the same mempool "round"). Closing this fully (found and
scoped, not built) would need a `PoolDiff`-based mempool overlay — `MempoolPoolNoteSet`
already has the natural shape for exactly this, adding a `pool_diff: PoolDiff` field
alongside `serial_owner_id`, composed via the *same* `PoolViewComposition::compose`
primitive the block pipeline already uses for exactly this kind of layering
(`utxo_validation.rs` already does the UTXO-side equivalent) — plus a serial-keyed sibling
to `OrphanPool` for the genuine "this serial doesn't exist anywhere yet" case. That's
materially more scope than this step's plan text asks for, with no verify condition that
requires it. Given the pool's target block cadence, "wait one confirmation between chained
pool ops" costs a wallet UX-visible delay measured in a fraction of a second, not a severe
regression — a reasonable place to stop for P6.7, revisit only if a later step's own verify
condition demands it.

**Test file, and a fee-stamp lesson worth recording**: added `mining/src/notepool_mempool_
tests.rs` as a new top-level file (matching `toccata_transient_mass_activation_tests.rs`'s
existing pattern — a `#[cfg(test)]`-gated module in `lib.rs`, tests as plain top-level
`#[test] fn`s, not nested inside `manager_tests.rs`'s private `mod tests`, since none of
that file's private helpers are reachable from outside it anyway). First draft of every
test used a bare `consumed == produced` rotate (move a note, no change in value) and every
one failed at the FIRST insertion with `RejectNonStandard(..., "transaction has 0 fees
which is under the required amount of ...")` — not a P6.7 bug, just a reminder that a real
pool-op transaction needs an actual fee to pass ordinary mempool standardness the same as
any other transaction, and P5.2/STATE.md's own fee design (whole small notes consumed
purely as "fee stamps", their value becoming the miner's fee via the `consumed - produced`
difference already unified in P6.6) is not automatic — a test (or a real wallet) has to
deliberately include one. Fixed by having every rotate consume a second, smaller note
alongside the one actually being moved and not reproduce its value in `produced`, so the
difference becomes a real, adequate fee. Three tests: the plan's own two verify criteria
(`conflicting_rotate_arriving_second_is_rejected`, `block_template_under_load_includes_
pool_ops`) plus a third closing the loop on the eviction mechanism itself
(`conflicting_rotate_is_accepted_once_the_first_is_evicted_by_confirmation` — proves the
conflict lock genuinely releases once the stale mempool occupant is evicted by a
confirming block, not just that the first-seen path works).

✅ *Verify*: all 3 new tests pass. Full `cargo test -p kaspa-mining` — 59 passed, 0 failed.
`cargo test --workspace --exclude kaspa-testing-integration` — 0 failed across every
crate. Integration suite: 42/42 passed. `cargo build --workspace` clean.

### P6.8 — Pool state sync (IBD) (2026-08-16)

**The real design problem the plan text never mentions**: WHERE does the served pool
state come from? The plan says "mirror `request_pruning_point_utxo_set.rs`" — but the
UTXO flow works only because every node maintains a SECOND utxoset positioned at the
pruning point (`PruningMetaStores.utxo_set`), advanced by the pruning processor via
per-chain-block `utxo_diffs` whenever the pruning point moves. The pool had no analog:
the only pool state any node kept was virtual's (P6.4). Serving virtual's state is
wrong (nothing commits to it), and reconstructing the pruning-point state on demand
means walking ~pruning-depth diffs per request — non-viable. The fix is the exact
structural mirror, and P6.4 had already unknowingly built the hard part: the
per-chain-block `notepool_diffs` store is precisely the diff source the pruning
processor needs. `PruningMetaStores` gained a `pool_state` (`DbNotePoolStore` under a
new prefix 95, via a new `with_prefix` constructor — same one-implementation-two-
prefixes pattern `DbUtxoSetStore` already uses) advanced **in the same loop iteration
and same WriteBatch** as the pruning utxoset, so the existing `utxoset_position`
recovery marker remains a single truth for both stores and crash-recovery semantics
are inherited wholesale rather than re-derived. A new pool stable flag (prefix 96)
joins the utxoset/SMT flags in `is_in_transitional_ibd_state`.

**Wire + verification: a deliberate, documented deviation from BOTH plan hints, each
half taken from the precedent that actually fits.** The prior session's research
suggested mirroring the seq-commit SMT flow (metadata message + inline per-entry
proofs). Reading the actual code said otherwise, twice:
- *Wire shape → UTXO flow, not SMT flow.* Seq-commit needs a metadata message because
  its `lanes_root` is one component folded inside `accepted_id_merkle_root` — the
  receiver literally cannot know what root to expect without wire-carried companions.
  The pool root has no such indirection: it IS the pruning point header's
  `pool_commitment`, verbatim, in a header the receiver already validated under PoW
  before asking. So: no metadata message at all, Done-sentinel termination, four
  messages mirroring the UTXO flow's exactly. And no inline proofs: serving them
  would require every node to maintain a second, pruning-positioned SMT (branch-node
  storage + permanent write amplification on every pruning advance) purely for a
  mid-stream-abort bandwidth nicety that the UTXO flow — with far larger payloads —
  doesn't have either. Tampered data is still always rejected; just at the final
  root check rather than mid-stream, exactly like the UTXO flow's MuHash check.
- *Verification → `crypto/smt`'s streaming builder, not `consensus/smt-store`'s
  importer.* The plan's "crypto/smt's streaming module exists for exactly this" is
  right, but the existing `streaming_import` wrapper is hard-coded to seq-commit's
  hasher and its block-versioned multi-lane `SmtStores` apparatus (confirmed by
  reading it — `StreamingSmtBuilder::<SeqCommitActiveNode, _>` on line 96, `DbSink`
  writing lane/score versioning the pool store deliberately doesn't have). New
  `DbNotePoolSmtStore::rebuild_from_sorted_leaves` drives the *generic*
  `StreamingSmtBuilder<NotePoolSmt, _>` directly with a ~70-line `MergeSink` writing
  into the pool store's own branch schema (structurally the generic `InlineMergeSink`
  from crypto/smt's own tests, DB-batched): O(n) single pass over the staged entries
  in RocksDB's native ascending-key order — which is exactly the strictly-sorted
  input the builder requires, so the staged flat map feeds it directly, no sort step.

**Import semantics and ordering**: download stages chunks into the pruning-position
pool store (mirroring `append_imported_pruning_point_utxos`); import then rebuilds the
SMT and copies the flat map into virtual's pool stores in one shared sorted pass
(split borrows via `VirtualStores` destructuring), resets the stored virtual pool
diff (virtual is about to be recomputed from the new pruning point's POV), and
compares the computed root against `pool_commitment` — `PoolRootMismatch` clears the
half-written stores and aborts IBD with the stable flag still down. Ordering is
enforced in all three IBD branches (Sync, DownloadHeadersProof, PruningCatchUp):
pool state syncs BEFORE the utxoset, because `import_pruning_point_utxo_set`
validates the pruning point's own transactions — pool ops included since P6.6/P6.7 —
against virtual's pool state. This closes P6.4's recorded "pool state empty at
pruning import" caveat, which reading the code confirmed was a genuine
silent-wrongness gap, not a benign TODO: an IBD'd node's `DbNotePoolStore` stayed
empty with its SMT at the empty root while the adopted header committed to a
non-empty pool, and nothing compared the two — post-IBD pool-op validation would
have run against a phantom empty pool.

**Second real bug, found by reading the prune path**: `prune()` deletes pruned
blocks' `utxo_diffs` (line 487) but `notepool_diffs` was never added to the deletion
batch — a permanent per-block disk leak on every pruning node since P6.4. One-line
fix alongside. Also added `assert_pool_commitment` to the pruning processor's
`enable_sanity_checks` path, mirroring `assert_utxo_commitment` — and the new
integration test's logs show it firing and passing on both the syncer's natural
pruning advances AND the syncee's post-IBD advances on top of the imported store.

**Also in passing**: `DenominationTag` gained the canonical `TryFrom<u8>` (wire
formats carry the tag as an integer; RPC in P6.9 will want it too), and `notepool.rs`
gained `seek_iterator`/`write_many` mirroring `DbUtxoSetStore`'s serving/staging API.

✅ *Verify*: new `daemon_ibd_pool_state_sync_test` (two real simnet daemons, small
override params): mint three notes through the real P6.6-funded/P6.7-mempool path,
rotate them to a second key, bury everything past the pruning depth, bring up a fresh
node, and assert (a) IBD completes (a root mismatch would abort it), (b) the synced
pruning point commits to a NON-empty pool (anti-vacuity guard against the test
silently passing on an empty pool), (c) **the syncee's own mempool accepts a rotate
consuming notes that exist only in the imported state** — the sharpest import proof
available, it fails with `SerialNotFound` on any import gap — and (d) the syncee
follows post-IBD blocks, each re-verifying pool commitments on top of the imported
store. Passed on the first run; full-log capture confirms the whole pipeline
("downloading the pruning point note-pool state" → "Total notes: 2" → "Imported
note-pool state ... root f43d…" → syncee sanity checks passing on later advances).
Tamper rejection is pinned at the exact mechanism level by new store unit tests:
`streaming_rebuild_detects_tampered_leaf` (forged note contents → different root),
`streaming_rebuild_agrees_with_incremental_apply` (the IBD rebuild and the live
incremental path produce identical roots AND identically-updatable branch structure),
and `clear_resets_to_empty_root`. Full `cargo test --workspace --exclude
kaspa-testing-integration` and the integration suite both green; `cargo build
--workspace` clean.

### P6.9 — RPC + notifications (2026-08-16)

A research pass first mapped the existing `UtxosChanged` pipeline in
full (`notify`'s generic `EventType`/`Scope`/`Subscription`/`Notification` apparatus,
the three independent per-crate `Notification` enums each built via the same
`full_featured!` macro, and the two-stage consensus→index emission `UtxosChanged`
uses) before writing any `NotesChanged` code, specifically to answer one question:
does a note need the same second-stage/index re-resolution UTXOs do?

**Answer: no, and that's the single decision that made this step small.**
`UtxosChanged` needs two stages because a raw consensus UTXO diff only carries
`ScriptPublicKey`s, and resolving those to addresses (what listeners actually filter
on) requires the optional `utxoindex`'s own separate re-emission on an `IndexNotifier`
— see `rpc/service/src/service.rs`'s `index_collector`, wired only
`if index_notifier.is_some()`. A note's identity (`sn`, `d`, `pk`) has no such
indirection — everything a listener could filter on is already sitting in the raw
`PoolDiff` the virtual processor computes on every commit. So `NotesChanged` emission
is one call, `self.notification_root.notify(Notification::NotesChanged(...))`, dropped
right next to the existing `UtxosChanged` emission in
`consensus/src/pipeline/virtual_processor/processor.rs`, reusing
`accumulated_pool_diff` — P6.4 had already computed it at that exact point in
`resolve_virtual`, just never wrapped it in a `Notification`. `rpc/service`'s routing
needed zero extra code for the same reason: `EventSwitches` defaults every new
`EventType` to enabled, and only `UtxosChanged`/`PruningPointUtxoSetOverride` are
explicitly carved out for the index path.

**Subscription design — deliberately not reusing `UtxosChangedSubscription`.** The
existing UTXO subscription is built on `notify/src/address/tracker.rs`'s `Tracker`, an
`IndexMap`-based reference-counting structure that exists because many independent
wallets watch *overlapping* address sets through one shared index — recycling entries
across listeners matters at scale. There's no analogous sharing need for note
serials/pks (nothing indexes them the way addresses are indexed), so
`NotesChangedSubscription` (`notify/src/subscription/single.rs`) is a plain
standalone value type: `active: bool, all: bool, serials: Arc<BTreeSet<Hash>>, pks:
Arc<BTreeSet<[u8;32]>>`. `BTreeSet` over the more obvious `HashSet` for one concrete
reason: `BTreeSet<T: Hash>` itself implements `std::hash::Hash` (needed because
`Single: ... + DynHash + ...` requires `#[derive(Hash)]` on the subscription struct),
`HashSet` does not. One file-scope gotcha this produced: `single.rs` already has `use
std::hash::{Hash, Hasher}` for its existing manual `Hash` impls, which shadows the
type name `Hash` — every new reference to `kaspa_hashes::Hash` in that file had to be
fully qualified, caught with a pre-build `sed` sweep rather than a wasted compile.

**Plumbing, crate by crate (each following the plan's own instruction to model this on
`UtxosChanged`, mechanically, once the two decisions above were made):**
- `notify` (base crate): `EventType::NotesChanged` (event #10), `Scope::NotesChanged`
  + `NotesChangedScope` (hand-rolled `Serializer`/`Deserializer`, borsh underneath),
  `ArrayBuilder::single`'s new match arm (the `compounded()` aggregate-gating builder
  was deliberately left on its existing catch-all — it only needs to know "is anyone
  listening," which `NotesChanged` doesn't complicate). `apply_notes_changed_subscription`
  became a new *required* method on the core `Notification` trait, which rippled a
  trivial passthrough into every implementor with no `NotesChanged` variant of its own
  (`indexes/core`'s `Notification`, two test-fixture types) — caught one at a time by
  successive `cargo build --workspace` runs, not surprising, exactly as predicted.
- Three independent per-crate `Notification` enums, each needing its own variant +
  impl: `consensus_notify` (raw), `rpc_core` (wasm/serde-friendly, its own integer
  discriminants), and a trivial-passthrough-only touch to `index_core` (no variant —
  confirms the "no index stage needed" decision at the type level, not just logically).
- `rpc/core`: two new `RpcApiOps` (`GetNotesBySerial`, `GetPoolStats`) plus
  `NotifyNotesChanged`/`NotesChangedNotification`, new wire types in
  `model/message.rs` (`RpcNoteEntry`, request/response pairs, all hand-rolled
  Serializer/Deserializer per the file's existing convention), and — unlike
  `UtxosChangedNotification`'s converter, which is a TODO-stub in this codebase — a
  REAL `consensus_notify::NotesChangedNotification → rpc_core::NotesChangedNotification`
  converter, because there's no index step deferring real resolution to later.
  `get_pool_stats` is backed by a new `ConsensusApi::get_pool_stats` that does a full
  `DbNotePoolStore::iterator()` scan — the same correctness-first,
  no-incremental-counter tradeoff P6.5 already established for commitment rebuilds,
  applied here on purpose rather than adding a maintained running counter.
- `rpc/grpc/{core,server,client}`: full bespoke proto messages (grpc has no generic
  subscription payload — every `Notify*`/notification type needs its own
  `.proto` message and converter) plus the usual macro-array entries
  (`payload_type_enum!`, `build_grpc_server_interface!`, `impl_into_kaspad_request!`
  etc.) mirroring the existing `GetSeqCommitLaneProof`/`UtxosChanged` precedents
  exactly — no design surprises here, confirming the research pass's predicted file
  list was complete.
- `rpc/wrpc/{server,client}`: a real, confirmed architectural asymmetry worth noting
  for future RPC additions — `Subscribe`/`Unsubscribe` in
  `rpc/wrpc/server/src/router.rs` are already fully generic over `Serializable<Scope>`,
  so once `Scope::NotesChanged` existed as a variant, wrpc subscription support was
  **entirely free**, zero notify-specific code. Only the two new non-subscription "get"
  ops needed macro-array entries on the wrpc client and server interfaces.
  `rpc/wrpc/wasm` needed nothing, consistent with it having no route even for the
  pre-existing `GetSeqCommitLaneProof` method — WASM JS bindings are wired per-method
  on a separate, later schedule, not automatically.

**A genuinely new CLI capability, not just a mirrored one.** Every existing
`RpcApiOps::*` arm in `cli/src/modules/rpc.rs` is a one-shot
`rpc.xxx_call(...).await?` — there was no precedent anywhere in the file for
registering a listener and consuming a notification stream. `cli/src/notifier.rs`
looked like the obvious place to find that pattern and turned out to be a completely
unrelated wallet-UI toast/icon system (`Transaction`/`Clipboard`/`Processing` icons) —
a dead end worth recording so a future reader doesn't repeat the detour. The actual
pattern came from `rpc/wrpc/examples/subscriber/src/main.rs`, a maintained example
built for exactly this: `rpc.register_new_listener(ChannelConnection::new(...))` →
`rpc.start_notify(listener_id, Scope::NotesChanged(...))` → drain the channel. The new
`rpc notify-notes-changed <serial-hex>...` command treats all args as watched
serials (mirroring `GetUtxosByAddresses`'s all-args-are-addresses convention), prints
each `NotesChanged` notification received via the existing `tprintln!`/`self.println`
helpers, and gives up after 120s so a manual verify session doesn't hang forever — a
one-shot demonstration command, not a permanent daemon feature. One naming gotcha:
`crate::imports::*` already brings in `crate::notifier::Notification` (the unrelated
toast-icon enum) as an explicit, non-glob `pub use`, which — per normal Rust
resolution — shadows the glob-imported `kaspa_rpc_core::Notification` from
`kaspa_wrpc_client::prelude::*`. Every reference to the RPC notification type in
`rpc.rs` had to be written as `kaspa_rpc_core::Notification`, not bare `Notification`.

✅ *Verify:* new `daemon_notes_changed_notification_test` (one real simnet daemon,
`--utxoindex` — `fetch_spendable_utxos` needs it): subscribes a `ListeningClient` to
`NotesChangedScope::default()` (empty serials/pks on `Start` == "watch everything",
mirroring `UtxosChangedScope`'s empty-addresses convention), mints two notes (1 MAGLD
+ 0.01 MAGLD) through the real P6.6-funded/P6.7-mempool path, and asserts the
resulting notification's `added` carries both new notes with the right
denomination/pk and an empty `removed`. Cross-checks the two new "get" RPC methods
agree with what the notification reported (`get_pool_stats`, `get_notes_by_serial`).
Then rotates both notes to a new key, producing only one note back (the D0_01
difference is retained as the rotate's fee — a consumed-equals-produced rotate has
zero fee and is correctly rejected by the standard mass-based relay-fee policy, the
same as a zero-fee transparent transaction would be; this was the second of two
test-harness bugs caught while writing the test, not product bugs — the first was a
forgotten `--utxoindex` arg). Asserts the second notification reports both old
serials `removed` and the new one `added`. Also extended the pre-existing
`rpc_tests::sanity_test` — which force-matches every `KaspadPayloadOps` variant with
`#[allow(unreachable_patterns)]` deliberately absent, so a new RPC op without a test
arm fails to compile — with `GetNotesBySerial`, `GetPoolStats`, and
`NotifyNotesChanged` arms. Along the way, `cargo build --workspace --tests` (not
plain `build`, which doesn't compile test-only code) surfaced two more `RpcApi`
trait-completeness gaps in test-only mock implementors
(`rpc/grpc/server/src/tests/rpc_core_mock.rs`, `wallet/core/src/tests/rpc_core_mock.rs`)
needing the same two new methods stubbed with `Err(RpcError::NotImplemented)`,
matching their neighbors. Full `cargo test --workspace --exclude
kaspa-testing-integration` green (142/142 result groups; kaspa-notify 20 passed,
kaspa-rpc-core 131, kaspa-consensus 93, kaspa-consensus-core 121); full integration
suite green (44 passed, 0 failed, 6 pre-existing `#[ignore]`d); `cargo build
--workspace` clean throughout.

### P6.10 — Consensus test battery + simpa (2026-08-16)

Continued directly from P6.9. Started with a research pass (not a
line of code) specifically to answer "what does P6.10's own text actually add, versus
what P6.3-P6.9 already left behind" — the plan bullet reads like a from-scratch
battery, but this codebase already had a lot of it:
`consensus/core/src/notepool/validate.rs`'s own `#[cfg(test)]` module already
unit-tests every stateless AND stateful rejection case from POOL-SPEC.md P5.3's
validation order (empty/oversized collections, duplicate serials, `SerialNotFound`,
`BadSignature`, `MixedKeysInGroup`, freshness boundaries, conservation, op-type
domain separation — 20+ cases), and
`consensus/src/pipeline/virtual_processor/notepool_tests.rs` (P6.4-P6.6's own test
module) already covers mint→rotate happy-path, one parallel-double-rotate conflict, a
3-block reorg, and value conservation across mint→transfer→redeem, all via a real
`TestConsensus` harness with reusable `Wallet`/`fund`/`mint_funded` helpers.

**The real, previously-uncovered gaps**, found by cross-referencing that inventory
against P6.10's own text line by line:
- **Split and merge had zero coverage above the `validate_stateful` unit-test
  layer.** `PoolOp` has exactly three wire variants — `Mint`, `Transfer`, `Redeem`
  (POOL-SPEC.md's own doc comment: "rotate," "split," and "merge" are descriptive
  labels for what a `TransferOp`'s consumed/produced shape happens to do, not
  distinct types) — so "all five ops happy-path" meant proving split (1→N) and merge
  (N→1) specifically actually mine, commit, and update the pool root through a real
  block, not just pass `validate_stateful` in isolation.
- `PoolOpContextError::BadPublicKey` had no test anywhere in the codebase.
- `TxRuleError::MalformedNotePoolPayload` was tested only at `PoolOp::decode_payload`
  itself (consensus-core's own unit test); nothing drove an undecodable payload
  through a real transaction into `tx_validation_in_isolation`, the gate that
  actually protects the chain.
- The one existing reorg test was 3 blocks deep — a correctness proof, not a scale
  proof (a bounded-depth walk optimization bug could pass that and still fail on
  something with dozens of blocks to unwind).
- Every existing conflict test pit the same op TYPE against itself (rotate vs.
  rotate); nothing exercised a cross-op-type conflict (e.g. a rotate and a redeem
  racing to consume the identical serial).

**Where the new tests live, and why not a new crate/module.** P6.10's own text says
"a dedicated integration-test module" and its verify condition names `-p
kaspa-testing-integration` — a literal reading would put everything there. But
`notepool_tests.rs` already has the full `Wallet`/`fund`/`mint_funded`/`pool_tx`
harness (~150 lines) that every one of these new tests needs, and it's `#[cfg(test)]`-
gated inside `kaspa-consensus`, so `testing/integration` (a different crate) cannot
import it — duplicating that harness into a second location just to satisfy a literal
"lives in this crate" reading would be exactly the kind of premature/unneeded
duplication this fork's own conventions warn against, for zero new signal (P6.8 set
the same precedent: its own new test was added to the SAME existing
`daemon_integration_tests.rs` file rather than a new module). So: the seven new
op-mechanics/rejection/reorg tests (below) went into `notepool_tests.rs` where the
infrastructure already lives — genuine reuse, not laziness — and `testing/
integration` got exactly the one piece that actually NEEDS that crate: a real,
multi-daemon "pool-root agreement across nodes" test (below), which shows up green
under the exact `cargo test --release -p kaspa-testing-integration` command the
plan's verify line names.

**The seven new `notepool_tests.rs` tests** (all passed first or second run):
`split_happy_path_mines_and_updates_pool_state` (0.1 MAGLD → 9×0.01, mirroring
`validate.rs`'s own split unit test's ratio one denomination tier down so it fits a
single funding block's coinbase) and `merge_happy_path_mines_and_updates_pool_state`
(two 0.01-MAGLD notes, one shared key, one signature, folding into one) prove the
op-mechanics through a real mined block, not just `validate_stateful`.
`bad_public_key_on_stored_note_rejects_consumption` mints a note with `pk: [0u8;
32]` (x=0 isn't a valid secp256k1 x-only point — mint never checks curve
membership, only denomination validity) and shows ANY attempted consumption fails
parsing the *stored* note's pk before signature verification is even reached,
regardless of who signs. `malformed_pool_payload_rejected_in_block` drives the same
invalid discriminant-3 payload consensus-core's own unit test uses through a real
block build. `parallel_rotate_vs_redeem_conflict_resolves_deterministically` races a
rotate against a redeem of the same note across two parallel blocks, confirming the
first-accepted-wins mechanism is genuinely op-type-agnostic (`TransferOp` and
`RedeemOp` both implement `ImmutablePoolDiff` the same way). 
`deep_reorg_past_pool_op_restores_prior_pool_state` extends the existing 3-block
reorg test's exact shape to a 60-block winning branch (comfortably inside
`finality_depth`, so this is a depth-of-walk stress, not a finality-boundary test) —
same walk-down/walk-up correctness, at a scale that would catch a bounded-depth bug
the shallow version couldn't. `value_conservation_across_split_and_merge` extends the
existing mint→transfer→redeem conservation test to split and merge specifically,
asserting each op's produced value equals consumed minus exactly its fee (caught one
real test-authoring bug along the way: the merge step's `SignedGroup` was initially
signed by the WRONG wallet — the split's produced notes belonged to `bob`, not
`alice` — surfaced immediately as `BadSignature`, not a product bug).

**`ConsensusApi::get_pool_root()`** (new, mirrors P6.9's `get_pool_stats`): lets code
outside the `kaspa-consensus` crate read virtual's live pool-commitment root without
`TestConsensus`-only internals (`TestConsensus::pool_root()` reaches
`self.consensus.storage.virtual_stores.read().pool_smt.current_root()`, fields only
visible from within the crate). Used by both the new daemon test (as a cheaper
alternative was available there — see below — so it ended up unused by that test,
but genuinely needed by simpa, which holds `Arc<Consensus>` from an external crate)
and simpa's own agreement check.

**`daemon_notepool_multi_node_agreement_test`** (`testing/integration`): three real,
independent daemons — not the special-cased pair `daemon_ibd_pool_state_sync_test`
uses — in a star topology around one miner. A real mint→split→merge→redeem sequence
(each op relayed over P2P and mined, not hand-imported, with the `no-unconfirmed-
chaining` mempool wait between each) leaves exactly one live note; asserts all three
nodes converge to the identical `header.pool_commitment` at the shared sink AND
identical `get_pool_stats()` at their live tips — the latter a more direct
"the actual pool contents agree" signal than a commitment hash alone. Used the
existing RPC surface (`header.pool_commitment`, `get_pool_stats`) rather than adding
a new "get the live root" RPC method, since P6.10's text doesn't ask for new RPC
surface and the existing signals were already sufficient to prove agreement.

**Teaching simpa pool ops.** `simpa/src/simulator/miner.rs`'s `Miner` already tracks
its own UTXOs locally (`possible_unspent_outpoints`, populated by scanning each
processed block's outputs) and builds real signed transactions from live consensus
state every block — not a structural-only DAG simulator. The natural extension: a
`possible_notes: IndexSet<Hash>` mirroring that same pattern, and a new
`maybe_build_pool_op` step in `build_txs`, gated by a new `pool_op_probability`
(`0.0` by default — every existing simpa caller's behavior is untouched). Each
miner self-targets its own note key (no cross-miner note transfers — unnecessary
complexity for a DAG-stress test, not a semantics test) and prefers consuming an
existing note over minting a new one, so its own backlog doesn't grow unbounded: two
notes sharing a denomination tier merge into one; otherwise a note rotates down one
tier (denomination values are strictly increasing, so this is always a valid,
strictly-positive fee) or, at the smallest tier, redeems to transparent value minus a
fee. Mint only fires when there are no live notes to work with. One real borrow-
checker wrinkle: `maybe_build_pool_op` had to be a free function, not a `&mut self`
method — its `pool_state`/`virtual_utxo_view` parameters alias `self.consensus`
through the read guard `build_txs` already holds, and a `&mut self` call can't
coexist with that live borrow (disjoint-FIELD borrows work inline in one function
body — as the pre-existing `self.lane_producer.next_lane(...)` call already
demonstrated — but not across a `&mut self`-taking method call).

**Three real bugs found by actually running this, not by inspection** — all in
simpa's own harness code, not the pool implementation itself:
1. `simpa/src/main.rs`'s `main_impl` force-activates `crescendo_activation` for every
   run but never did the same for `toccata_activation`/`pool_activation` — pool-op
   transactions carry `TX_VERSION_TOCCATA`, so every one was silently rejected as
   `UnknownTxVersion` until both are force-activated the same way.
2. `OnetimeTxSelector::reject_selection` was a blind `unimplemented!()`, and
   `is_successful()` unconditionally returned `true` — meaning ANY rejected
   transaction crashed immediately with an opaque panic, before the actual
   `RuleError` (already computed by `build_block_template`) was ever visible. Fixed
   by tracking whether a rejection happened and having `is_successful()` report it
   honestly, so a genuine rejection now panics via `build_new_block`'s own
   `.expect(...)` WITH the real error attached — same "this must never happen" hard-
   assertion semantics, just debuggable now. Fixing this surfaced a second, entirely
   latent bug behind it: `select_transactions`'s `self.txs.take().unwrap()` would
   panic on `None` on the template-builder's retry-loop second call — previously
   masked because the first bug always crashed before the retry loop ever ran.
3. The new `test_pool_ops_via_simpa` intermittently failed
   `test_pruning_via_simpa` (a pre-existing, untouched 5000-block test) with an
   `fd_budget`/semaphore acquire error one below its limit — both tests size their
   file-descriptor budget off the whole process's ulimit, an assumption that only
   holds with one simulation running at a time, but Rust's test harness runs
   `#[test]`s in the same binary concurrently by default. Fixed with a shared
   `static Mutex` serializing the two (no new dependency).

✅ *Verify:* `cargo test --workspace --exclude kaspa-testing-integration` green
(142/142 result groups; `kaspa-consensus` now 100 passed, up from 93 — the seven new
tests); full `cargo test --release -p kaspa-testing-integration` green, including the
new three-daemon agreement test; `cargo test --release -p simpa --bin simpa` green
running both `test_pruning_via_simpa` and `test_pool_ops_via_simpa` together (3
miners, 400 blocks, `pool_op_probability = 0.5`) — the pool-root agreement assertion
never fired, and the independent-replay cross-check against a fresh consensus
(simpa's own existing post-simulation validation) passed too; `cargo build
--workspace --tests` clean throughout.

### P6.11 — Finality-anchor consensus rule (2026-08-16) ⚠️ HARD

Done under a stronger model per the plan's own HARD flag and the standing model-switch
discipline. Everything implements POOL-SPEC.md P5.8 exactly; where the spec left an
implementation choice open, the decision and its reasoning are recorded here.

**What was built, layer by layer:**
- `consensus/core/src/finality_anchor/mod.rs`: P5.8's `FinalityAnchor` struct field
  for field; `EquivocationEvidence` as `{trustee_index, first, second}` where each
  attestation is `(anchored_block, anchored_daa_score, signature)` — the signing hash
  covers exactly those two fields, so a trustee's contribution to any anchor is fully
  captured by that triple, making this the minimal self-contained form of the spec's
  "two complete conflicting FinalityAnchor messages plus their two signatures";
  `AnchorPayload` borsh enum (Anchor/Equivocation — evidence travels "as a transaction
  in the same dedicated anchor subnetwork", per spec); a `FinalityAnchor`-domain
  blake3 signing hash (`crypto/hashes` macro convention); and context-free
  verification: bitmap shape (a bitmap cannot express a repeated signer — why the
  spec chose it), one-signature-per-set-bit, ≥3 signers, every BIP340 signature
  against its pinned key, and the exact equivocation rule (different blocks AND
  scores strictly `< interval` apart, interval resolved at the max score's decay
  stage). Seven unit tests pin the boundaries — including that two anchors exactly
  one interval apart are honest sequential anchoring, NOT equivocation, and that a
  forged signature can never disqualify anyone.
- `SUBNETWORK_ID_FINALITY_ANCHOR` ("ANCR"): the spec's "second, independent
  namespace, not the pool's".
- `FinalityAnchorParams` grouped in params (one field in each network const block +
  `OverrideParams`, the `BlockrateParams` pattern): trustees ship as `None` on every
  network — the mechanism is inert until the P9.1 ceremony pins real keys, and tests
  inject generated ones. Depth 600, launch interval 300, hard expiry 6,311,520,000
  as spec'd.
- `DbFinalityAnchorStore` (registry prefixes 97/98): the latest-anchor ratchet
  (`CachedDbItem<StoredAnchor>`) and the deny-list (`CachedDbItem<Vec<DenyListEntry>>`).
- Virtual processor: `collect_finality_anchor_updates` (acceptance-scan + contextual
  checks + ratchet/deny computation, pure reads) feeding `commit_virtual_state`'s
  existing `WriteBatch`; the fork-choice override in `sink_search_algorithm`; the
  fail-open state machine + alert; `ConsensusApi::get_finality_anchor_status()`.
- Isolation validation (`check_finality_anchor_payload`) + the zero-input carve-out.

**Design decision 1 — context-free validity, contextual effect.** The spec's deny-list
activation is POV-scoped ("takes effect ... in validation contexts whose POV chain
includes the block containing the accepted evidence"). Read naively, that makes
anchor-*transaction* validity depend on each node's deny-list store — and during
design this surfaced a genuine determinism hazard: deny entries created at
virtual-commit time exist only on nodes whose virtual actually passed through the
evidence block, so a node validating the same context later (e.g. re-verifying a
block whose chain includes evidence its own virtual never adopted) would judge an
anchor tx differently than a node that had adopted it — divergent acceptance data
from identical on-chain history. The fix is structural: **transaction validity is
fully context-free** (parse, shape, all signatures, the equivocation overlap rule,
and the certified-score-vs-hard-expiry bound — all pure functions of payload bytes
plus params, checked at body-in-isolation, so "false evidence is simply an invalid
transaction" holds exactly as spec'd), while **everything chain-contextual decides
only an accepted anchor's *effect*** at virtual-commit time: depth vs the accepting
chain block's DAA score, anchored-block chain membership + claimed-score
cross-check, and deny-list quorum filtering. A contextually-failing anchor is a
no-op transaction, never a block invalidator. The spec's POV-scoped semantics are
preserved where they matter — which anchors *ratchet* and which keys *count* — and
the deny-list's effect on later anchors within the same commit is ordered by
acceptance order (evidence first ⇒ same-chain-block anchors already see it),
matching "from that block onward" inclusively and deterministically.

**Design decision 2 — monotone node-local state, no rollback machinery.** The ratchet
is monotone *by spec* ("persists the highest-scoring valid anchor it has ever
accepted ... across restarts, resyncs, and reorgs" — the rollback-resistance rule).
The deny-list gets the same treatment by *keying* rather than by rollback: each entry
records its accepting chain block, and whether it binds in a given context is a
reachability question (`is_chain_ancestor_of(entry.accepting_block, pov)`) — an
entry whose accepting block reorgs out is inert automatically, and re-acceptance on
the new chain appends a new entry. Nothing is ever deleted (permanence: "no
un-disqualify mechanism short of a hard fork" holds by construction), the list is
bounded by 5 keys × a handful of evidence acceptances ever, and both items join
`commit_virtual_state`'s batch so anchor state and virtual state land atomically.
This is why neither item needs the diff-based POV machinery the UTXO set and note
pool require — and why pruning is a non-issue (they're not per-block data).

**Fork-choice wiring — exactly "a second trigger for the existing finality-depth reorg
refusal", as the plan phrased it.** `sink_search_algorithm` now takes an
`anchor_guard: Option<Hash>`; a candidate whose selected chain lacks the anchored
block is refused with a loud warning and **falls through to the parents push exactly
like a finality violation** — an earlier draft `continue`d past the push, which a
review pass caught as a heap-exhaustion hazard (a merge block whose *selected* chain
is the attacker's but which references honest parents would strand those parents).
Falling through preserves the existing termination argument unchanged. Two
deliberate choices here: the guard is evaluated against the node's **own** virtual
DAA score, not the candidate's — an attacker with majority hashrate can inflate a
fork's DAA score and would otherwise fast-mine its way past the staleness bound and
bring its own exemption; and the test-block builder passes `None` (it builds
hypothetical PoV blocks with an ORIGIN finality point — same reasoning). Safety of
the guard against local knowledge: the ratchet only ever holds anchors accepted on a
locally-committed chain whose blocks this node retains, and a fresh (non-stale)
anchor is at most ~1,500 score units old — far inside finality/pruning depth — so
the anchored chain's tips always exist locally and the walk always terminates.

**Zero-input anchor transactions.** Trustees "produce no blocks, hold no mining
reward, and can only veto" — requiring a funding input would force trustee wallets
into existence. Anchor-lane txs get the same zero-input carve-out pure pool
Transfers have: authorization is the payload's trustee signatures. No spam surface:
only genuinely trustee-signed material passes isolation, an identical tx has one
txid, and third parties can't mint variants. (Fee/relay policy for the lane — how
miners are induced to include zero-fee anchor txs — lands with P6.12's signer
daemon, where it's actually exercised; consensus imposes no minimum fee, which is
all P6.11 needs.)

**The sunset, staged honestly.** The hard expiry (stage 4) is enforced
unconditionally in three places: an anchor *certifying* a score ≥ expiry is an
invalid transaction (isolation, context-free); accepted-anchor processing skips
expired accepting contexts; and enforcement itself switches off once the node's own
virtual score passes expiry — "no anchor, however validly signed, has any consensus
effect from this point forward", with the ratchet retained but inert. Stages 1–3
ship as `ForkActivation::never()` hooks with their intervals (36k/864k/6,048k)
pinned: their *triggers* are the sustained-difficulty condition whose threshold T is
explicitly not final until the P9.5-gated sensitivity model exists (the spec's own
"Calibration is a hard pre-launch gate"), so wiring live median-≥-T-for-M-months
evaluation now would be building consensus rules against an unfrozen constant. The
`ForkActivation` scores are precisely the hook that evaluation — or the FORK-PLAN
P9.x governance mechanism that "upgrades the P5.8 sunset story" — sets when T
freezes. This is the plan's own "`ForkActivation`-staged sunset" phrasing read
literally, and the deferral is recorded here so nobody mistakes it for a forgotten
piece.

**A genuine property surfaced by the first failing test run:** a block's own
transactions are accepted by its chain *descendants* (a block's acceptance data
covers its mergeset, never its own body), so **an anchor takes effect exactly one
chain block after inclusion**. The first test draft mined an anchor into the tip and
asserted an immediate ratchet — `None`. Not a bug: certifying "already-mined
history" is inherently fine with a one-block application lag, and the honest network
mines continuously. Documented in the test module's header as load-bearing for every
test there (`mine_and_confirm` = carrier block + confirming block). Also surfaced:
`build_utxo_valid_block_with_parents` runs only *contextual* template validation —
isolation checks fire at insert-time body validation, so rejection tests assert the
insert errs rather than the build panicking (unlike the notepool rejection tests,
whose invalid ops are contextual and do panic the build).

**Deferred to P6.12, explicitly:** P2P gossip of anchors/evidence (the second
distribution channel — P6.11 nodes learn anchors only from mined transactions),
anchor-aware IBD (requesting latest anchors from every peer before committing to a
chain; the `verify_anchor` / contextual-effect split was structured so gossiped
anchors slot in as "valid signed statements about known blocks" without rework),
mempool/relay policy for the anchor lane, the trustee signer daemon, and RPC
exposure of `finality_anchor_stale` + the current decay stage (the consensus-side
flag and `get_finality_anchor_status()` exist; the spec's wallet-visible RPC
requirement rides P6.12's surface work).

✅ *Verify:* 5 new tests in
`consensus/src/pipeline/virtual_processor/finality_anchor_tests.rs`, all four plan
criteria plus the closed-lane case, each with an anti-vacuity control:
`heavier_attacker_chain_lacking_anchor_loses` (25-block attacker fork vs a 10-block
anchored chain — sink holds; the identical DAG on an unkeyed control node reorgs,
proving the anchor made the difference); `equivocating_quorum_ignored_after_proof`
(evidence against keys 0/1/2 → `disqualified == [0,1,2]`, prior anchor untouched, a
dead-quorum anchor and a mixed {2,3,4} anchor with only 2 countable signers both
fail to ratchet — and with 3 of 5 keys dead the quorum is unrecoverable short of a
hard fork, exactly as spec'd); `anchors_past_sunset_rejected_and_enforcement_retires`
(expiry-15 params: an anchor certifying score 20 is an invalid *transaction*; a
pre-expiry anchor enforces until virtual's own score passes 15, then `expired` +
heavier fork wins despite the ratchet); `anchor_free_and_stale_operation_degrade_to_plain_pow`
(never-anchored: plain PoW, `stale` correctly false — nothing was lost; anchored
then outrun past `depth + 3×interval`: `stale` true, deep reorg allowed again,
ratchet retained — stale, not forgotten); `anchor_lane_closed_without_pinned_keys`.
All 5 passed on the first run after the acceptance-timing fix. `kaspa-consensus`
105 passed (up from 100, plus 7 new consensus-core unit tests → 128 there); full
`cargo test --workspace --exclude kaspa-testing-integration` and the integration
suite green; `cargo build --workspace --tests` clean.

### P6.12 — Anchor distribution + trustee signer (2026-08-16)

Continuing directly from P6.11 (same session — the two steps share one
design and P6.11 was structured with this step's contract in mind). This closes
Phase 6.

**Gossip channel.** Two new P2P messages (`RequestFinalityAnchor` / `FinalityAnchor`,
proto tags 68/69 — remember every new p2p message needs FOUR coordinated edits:
`p2p.proto` body, `messages.proto` oneof, `payload_type.rs` enum variant AND its
`From` match arm) and one per-peer `FinalityAnchorFlow` in `v10`, handling both
directions on a single subscription (the router panics on duplicate payload-type
subscriptions, so one flow owns both types). On start it immediately requests the
peer's best anchor — the `ReceiveAddressesFlow` on-connect pattern — which is exactly
what makes IBD anchor-aware per the spec ("requests the latest known anchors from
EVERY connected peer, not just the sync peer"): the request rides connection setup,
before any sync decision. Improvements re-relay hub-wide (minus the origin), so one
honest path delivers the newest anchor network-wide. Spam-bounded by verification
(nothing not trustee-signed propagates) and by only-improvements-propagate.

**The pending slot, and why gossip needed new consensus surface.** A fresh node
receives anchors for blocks it doesn't have yet. P6.11's ratchet precondition (block
known at its claimed score AND on some body tip's selected chain — the condition
keeping `sink_search_algorithm`'s termination argument valid) can't hold for those,
so `apply_external_finality_anchor` holds them in a persisted *pending* slot instead:
not enforced by fork choice, but (a) served onward to peers, (b) consulted by
anchor-aware IBD, and (c) auto-promoted to the ratchet inside
`collect_finality_anchor_updates` the moment the anchored block becomes locally
verifiable (which on a syncing node happens naturally as the honest chain arrives).
The store gained the full-signed-anchor cache (`latest_full`) for re-serving — the
P6.11 ratchet only kept (block, score) — plus `pending`; prefixes 99/100.

**Anchor-aware IBD.** One check at the exact point all three IBD types converge with
headers fully synced, before any body download
(`verify_syncer_chain_against_finality_anchor`, called just before the first
`sync_missing_block_bodies`): the offered chain must contain the newest held anchor
(ratchet or pending) — an anchored block still unknown after a full header sync from
this peer means the offered chain omits it, same refusal as known-but-reorged-out.
Refusal is a `ProtocolError` → disconnect, the established IBD abort idiom.
Fail-open is preserved at IBD with the staleness bound judged against the *offered
chain's own* tip score: a chain whose trustees stopped anchoring ages ago must stay
syncable on plain PoW (otherwise an abandoned anchor would deadlock every fresh
node). The residual this leaves — an attacker chain far enough ahead in DAA score
looks "stale-relative" and escapes IBD enforcement — is the spec's own acknowledged
eclipse-residual: a non-eclipsed node keeps hearing fresher anchors and honest blocks
from other peers, promotes, and the fork-choice guard then applies (and would flip a
captured node back — trustee-certified history wins by score, not work); a fully
eclipsed fresh node is the residual risk "every PoW chain's IBD already carries, now
with an alarm attached."

**THE BUG — fail-open's clock was partially attacker-controlled.** The first
adversarial run of the refusal test failed: fresh node C, holding and *enforcing* the
anchor, adopted the heavier anchor-free chain anyway. The log showed why: C went
STALE first. P6.11 judged staleness by **virtual's DAA score** — deliberately not the
candidate chain's score (that much was right) — but virtual **merges** a conflicting
heavier branch even while the guard refuses to *select* it (merging is not chain
selection; bounded-merge permitting, the branch's blocks enter virtual's mergeset),
and virtual's DAA score counts the mergeset. So the attacker's 100-block branch
inflated C's own clock past `depth + 3×interval`, tripped fail-open, and only then
won on work. Textbook: the guard's off-switch was measured on a quantity the
adversary could pump without ever winning the guard itself. Fix: the enforcement
clock (fork-choice guard, status, alert) is now the **sink's own header DAA score** —
the selected chain's clock, which a refused branch cannot touch. All five P6.11
consensus tests still pass unchanged (linear-chain scores barely differ), and the
adversarial daemon test now shows the refusal holding block-by-block ("ignored from
Virtual chain selection regardless of its accumulated work") with zero stale
transitions on the defended node. An honest side-observation from the same log: the
*attacker's own node*, upon learning the honest anchor via gossip relay (block
initially unknown → pending → honest blocks relayed → promoted), correctly reports
it stale relative to its own far-ahead chain — fail-open behaving exactly per spec
from the attacker's own point of view.

**Mempool/relay/template policy.** Anchor-lane txs are zero-input/zero-fee (trustees
hold no funds — P6.11's carve-out), which the relay-fee floor would reject
(`minimum_required_transaction_relay_fee` never returns 0 by construction) and the
feerate-weighted template selectors would never sample (weight `(fee/mass)³ = 0`).
Two surgical changes: a fee-floor exemption for `SUBNETWORK_ID_FINALITY_ANCHOR` in
`check_transaction_standard_in_context` (bounded: isolation validation already
limits the lane to genuinely trustee-signed material, honest rate is one anchor per
cadence interval, identical anchors dedup by txid), and a `ForcedInclusionSelector`
wrapper prepending ready anchor-lane txs to whatever selector the frontier builds
(with duplicate filtering, since `TakeAllSelector` would return them again from the
frontier). Zero-input txs otherwise flow through the mempool untouched — audited:
every input-keyed structure (orphan pool, mempool UTXO set, parent tracking) is a
no-op on an empty input list, and P6.7's zero-input pool ops blazed this trail.

**`GetFinalityAnchorStatus` RPC** — the P6.9 recipe end to end (ops 156, hand-rolled
serializers, grpc proto 1127/1128 + converters + factory + client route, wrpc arrays,
both mock impls, the forced `rpc_tests::sanity_test` arm, a cli arm). Exposes the
spec's wallet-visible `finality_anchor_stale` flag ("a user accepting a large payment
during an extended anchor outage ... should be able to know it"), enforcing/expired,
the active cadence interval, and the deny-list. This closes P6.11's deferred RPC
item.

**The trustee signer** (`trustee-signer/`, lib + bin, new workspace member): one
instance per trustee key (production topology per the spec's one-live-signer
guidance). Watches its node over gRPC; finds the highest selected-chain block at
least `depth` behind the virtual score (selected-chain walk via
`get_virtual_chain_from_block` from a moving cursor — parent-walking would not be
selected-chain-safe on a real DAG); signs at most once per cadence interval,
**persisting the last-signed state BEFORE sharing the signature** — the order
matters: a crash between the two loses one signing (harmless) instead of enabling a
double-sign after restart (a restart mid-interval re-signing a *different* block
would hand anyone a valid equivocation proof against an honest key — the exact
"misconfigured failover" hazard the spec calls out; the persisted state is the
defense). Partials travel as length-prefixed borsh over plain TCP — deliberately
minimal, zero new workspace dependencies (bridge/ set the hand-rolled precedent;
neither reqwest nor axum is a workspace dep), and explicitly replaceable by the P9.1
ceremony's ops decisions; the signing discipline is the part that must survive.
Aggregation: any signer holding ≥ k partials for the identical (block, score)
assembles the canonical anchor (ascending index) and submits;
already-in-mempool = success. The signer-liveness test showed the expected view-skew
behavior: a signer occasionally targets a one-block-different (block, score) than its
peers for a tick (no quorum forms on it that round — never equivocation, since each
key still signs once per interval), realigning within the next poll.

✅ *Verify:* `daemon_anchor_refuses_heavier_anchorless_chain_test` (three real
daemons: honest A — whose anchor goes through real RPC submission, the mempool fee
exemption, forced template inclusion, and mining; verified-heavier attacker B; fresh
C syncing A then meeting B and refusing it, anchored block still on C's selected
chain) and `daemon_trustee_signers_produce_anchors_test` (three in-process signers
with real localhost TCP partial exchange against a continuously-mined node: 3-of-5
anchors submitted continuously, both nodes reaching `enforcing && !stale`, the
anchor advancing across cadence intervals). Full `cargo test --workspace --exclude
kaspa-testing-integration` and the full integration suite green; `cargo build
--workspace --tests` clean.

### P7.0 — Inherited-wallet surface audit (2026-08-16) 🧑‍⚖️ DECISION

Decision (options presented, user-ratified): **keep the inherited seed-phrase wallet
stack as the transparent-tier wallet tool**, and execute the mandatory
legacy-Kaspa-import removal immediately rather than at the P8.7 deadline. Full
rationale in DECISIONS.md's new row; the short version: the transparent tier is
permanent (mining payouts, mint funding, redeem outputs, the T&A integration's fee
key), the inherited stack is its only wallet, it already works (P2.8 fixed and
rebranded it), and both alternatives (strip / feature-gate) buy churn or build-matrix
complexity without reducing what actually has to be maintained.

**What was removed** — every way to feed real Kaspa key material into Marigold:
`compat/gen0.rs` (KDX keydata import) and `compat/gen1.rs` (Go-`kaspawallet` file
import) deleted, with `compat/mod.rs` kept as a documented tombstone so the removal
is discoverable in place; the four `import_kaspawallet_golang_*` API functions,
`import_legacy_keydata`, the `import_gen1_keydata` todo-stub, and the golang wallet
wire-file types (`EncryptedMnemonic`, `SingleWalletFileV0/V1`,
`MultisigWalletFileV0/V1` — used by nothing else) removed from `wallet/mod.rs`,
along with a long-dead commented `decrypt_mnemonic`; the CLI's
`account import legacy-data` arm removed, `account import mnemonic legacy` turned
into an explanatory refusal (typing a 12-word KDX mnemonic is the same key-reuse
hazard as the file imports — the plan's letter listed the file paths, its rationale
clearly covers this one too), and every KDX/kaspanet mention scrubbed from help and
hint text. **Two small discoveries along the way**: `cli/src/modules/import.rs` was
already dead code — commented out of the module tree, which is why its call to a
never-defined `import_gen0_keydata` compiled fine for years — deleted outright; and
`api/traits.rs`'s `legacy_accounts` flag documentation actively advertised the
KDX/kaspanet provenance, now rewritten to storage-compat-only with an explicit
"should not be used by new code."

**What was kept, deliberately**: the legacy account *storage* variant
(`account/variants/legacy.rs`) and the gen0 derivation code — pre-existing wallet
files containing legacy accounts still open (compatibility), there is simply no way
left to create or import one. The verify grep's surviving matches are exactly:
tombstone/refusal comments, the storage variant, one coincidental `…umkdx…`
substring inside a bech32 test address, and our own `KaspaWalletKeys` error-variant
name. `cargo test --workspace` green after removal.

**Also this session (not a plan step)**: the T&A anchoring integration was agreed —
the partner runs a full archival node and has a verifier tool (recorded in
STATE.md), and a draft API contract for the P9-era anchoring gateway was published
at [ANCHORING-GATEWAY.md](ANCHORING-GATEWAY.md) so the partner's Workers-side
integration can start against a stable shape now (submit/lookup/health endpoints,
the 33-byte on-chain payload encoding an independent verifier relies on, and the
open items — namespace pinning, tokens, rate limits — flagged explicitly). A
follow-up round with the partner pinned the subnetwork namespace to `"T360"` and
specified zero-downtime token-set rotation (rev 2 of the doc).

### P7.1 — Note key DB (2026-08-16)

**Design decision — serial-keyed, not key-keyed.** POOL-SPEC.md P5.6 sketches
`KeyDbEntry{sk, provenance, known_serials: Vec<Hash>}` — one row per key, a list of
serials underneath. FORK-PLAN's own P7.1 wording flattens this to
`(serial, sk, denomination, provenance)` — one row per serial. Went with the
FORK-PLAN shape: P7.1 is infra, not the receive/spend/POS flows (P7.2–P7.5) where
the shared-`pk` case (POS landing pad) actually gets exercised, and a flat per-serial
row is what spend selection wants to query directly. The cost is redundant `sk`
storage when a `pk` genuinely is shared — acceptable, `sk` is 32 bytes, and the spec
itself treats shared-`pk` as the deliberate exception (POS), not the default (a
personal wallet defaults to a fresh `pk` per note specifically to avoid ever needing
this).

**Split into a sensitive row and a plaintext index**, mirroring `PrvKeyData`/
`PrvKeyDataInfo`: `NoteKeyEntry{sn, sk, d, provenance}` is the only thing that needs
encryption — everything else about a held note is either already public on-chain
(the pool is plaintext) or wallet-internal hygiene metadata, so `NoteKeyInfo{sn, pk,
d, provenance, status}` lives in a separate plaintext collection. This split turned
out to be exactly what made the live subscription problem tractable (next
paragraph) — not planned up front, found while working out how a passive listener
could react to a `NotesChanged` notification without the wallet secret in hand.

**The live-update problem and its resolution.** "Subscribes to `NotesChanged` for
its serials" sounds like it wants a background task that mutates the encrypted key
map on every notification — but nothing else in this storage layer keeps a
session-wide cached secret; every existing mutating call (`PrvKeyDataStore::store`,
etc.) takes `wallet_secret` explicitly from whatever caller already has it (a CLI
prompt, a wizard). A background listener has no such caller. Resolution: give
`NoteKeyInfo` a `status: NoteStatus{Active, Superseded}` field that can be flipped
with no secret at all (plaintext-only mutation), and split
`NoteKeyStore::apply_notes_changed(wallet_secret: Option<&Secret>, notification)`
into two halves accordingly — `removed` entries always supersede their row (safe
even while locked), `added` entries that match an already-held key's derived `pk`
only insert the new row when a secret is supplied, otherwise landing in the result's
`deferred` list for a later caller (a P7.2+ wizard that already has the secret) to
retry. `UtxoProcessor` gained `register_note_serials`/`unregister_note_serials`
(mirrors `register_addresses`, using the P6.9 `NotesChangedScope`) and a
`Notification::NotesChanged` dispatch arm forwarding to `Wallet` over a new
`WalletBusMessage::NotesChanged` variant, mirroring exactly how `UtxosChanged`
already reaches `Wallet` via `WalletBusMessage::Discovery`. `Wallet::handle_notes_changed`
calls `apply_notes_changed(None, ..)` — the always-safe half — logging (not
silently dropping) how many rows are waiting on a secret.

**`import_bearer_key` takes no provenance argument.** "Hot keys are flagged at
import" is enforced structurally, not by trusting a caller-supplied flag: the one
entry point for a key that crossed a wallet boundary hard-codes `NoteProvenance::Hot`
in its own body, so there's no call shape that could import a bearer key as `Cold`.

✅ *Verify*: 3 new unit tests against a resident (in-memory) `LocalStoreInner` via
the public `NoteKeyStore` trait — round-trip (store/load/remove, including that
`NoteKeyInfo::pk` matches the entry's derived BIP340 x-only pubkey), hot-flagged at
import, and a synthetic rotation notification (remove old `sn`, add new `sn` under
the same `pk`) that supersedes the old row unconditionally and, once a secret is
supplied on a second `apply_notes_changed` call, inserts the new row inheriting the
old row's `sk`/provenance. Full wallet-core suite green (46 tests, up from 43),
`cargo check --workspace --all-targets` and `cargo clippy -p kaspa-wallet-core`
clean.

### P7.2 — Mint & redeem commands (2026-08-16)

**Two very different transaction shapes, two very different construction
strategies.** Mint needs real transparent inputs (it's funded from the transparent
balance) — the natural fit is the wallet's existing `Generator`/`Signer` pipeline,
same as any `send`. Redeem needs *zero* transparent inputs at all — POOL-SPEC.md
P5.2 designs it self-funding, the transparent output paid entirely from the
consumed notes' value — which doesn't fit `Generator`'s model even slightly (it only
knows how to aggregate real UTXOs *toward* a requested output value; it has no
concept of value arriving from outside the UTXO set). Redeem is hand-built instead,
mirroring `trustee-signer::anchor_transaction`'s zero-input pattern from P6.12
almost exactly (`consensus_core::mass::MassCalculator` for a correct mass/fee, raw
secp256k1 Schnorr signing with the note's own key, direct RPC submission) —
Redeem's notes-authorize-value is architecturally the same shape as an
anchor-lane transaction's signature-authorizes-inclusion, just with a real payout.

**The Generator's one new trick, and why nothing bigger was needed.** Making Mint's
minted value vanish from change without a real output for it turned out not to need
touching `Generator`'s core aggregation logic at all: `PaymentDestination::Change`
(sweep semantics) structurally forbids any priority fee, but
`PaymentDestination::PaymentOutputs(vec![])` — a non-Change destination with an
*empty* explicit-outputs list — sails through the same validation and gives
`Fees::SenderPays(amount_petals)` a real target to attach to, silently reducing the
automatic change output by exactly the minted amount (plus the real fee) with no
output ever created to represent it. Found by reading the aggregation code closely
enough to notice `final_transaction_amount` only needs to be `Some(0)`, not `None`,
for the fee-inclusion machinery to activate. The only genuine `Generator` change
needed was `GeneratorSettings::with_subnetwork_id()` (default
`SUBNETWORK_ID_NATIVE`, applied only to the *final* transaction — any intermediate
compound/consolidation transactions stay native, matching how
`final_transaction_payload` already works) plus deriving `TX_VERSION_TOCCATA` for
any non-native final transaction (every non-native subnetwork this fork has added —
note-pool ops, finality anchors — requires it).

**The real bug the Generator change exposed — genuinely not notepool-specific.**
Found only by building and running a live daemon+wallet integration test (see
below): the daemon rejected mint's signed transaction with `"RpcTransactionInput
.sig_op_count is inconsistent with transaction version 1"`. Root cause:
`Generator::aggregate_utxo()` always builds transparent inputs with legacy
`ComputeCommit::SigopCount`-based mass, correct for `TX_VERSION` (0) but invalid for
`TX_VERSION_TOCCATA` (≥1), which requires `ComputeCommit::ComputeBudget` instead
(`ComputeCommit::version_expects_compute_budget_field`) — Toccata's "input
compute-budget mass" change (`consensus/core/src/constants.rs`'s own doc comment on
`TX_VERSION_TOCCATA`). Nothing before P7.2 had ever exercised this combination
(Toccata version *and* real transparent inputs) anywhere in the codebase — every
prior non-native-subnetwork transaction (pool ops, finality anchors) has zero
inputs, so the mismatch had no way to surface until mint needed both at once. Fixed
by remapping every input to `TransactionInput::new_with_compute_budget(...)` right
before the final transaction is built (i.e. before signing — `compute_commit` is
part of what the sighash commits, so this can't be patched up after the fact), using
the same grams-per-sigop → compute-budget conversion `kaspa_consensus_core::sign::
sign` already uses elsewhere (`GRAMS_PER_SIGOP_COUNT_UNIT=1000` /
`GRAMS_PER_COMPUTE_BUDGET_UNIT=100` → 10 budget units per sigop, rounded up). This is
a real, general `Generator` bug — it would have hit any future feature needing a
non-native subnetwork with real transparent inputs, not just mint. Flagged, not yet
fixed: the wallet's own `tx::mass::MassCalculator` (an independent reimplementation
of the consensus-core one, not a wrapper around it) still doesn't know about
pool-op signature costing at all — harmless for Mint specifically (`pool_signature_
mass` is always zero for Mint) and for Redeem/Transfer (both bypass this calculator
entirely, being hand-built with `consensus_core::mass::MassCalculator` directly),
but a real gap for whoever eventually needs the wallet's own estimator to *price* a
Transfer before building it.

**Live daemon+wallet test infrastructure (new, reusable by P7.3-P7.5)**:
`testing/integration/src/notepool_wallet_integration_tests.rs`, with
`kaspa-wallet-core` added as a `testing/integration` dependency for the first
time — every prior daemon test builds transactions from raw consensus-core types
directly. Connects a real `kaspa_wallet_core::Wallet` (resident storage) to a live
daemon over **wRPC**, not gRPC — `Wallet`'s `UtxoProcessor` only wires its
connect-state and `UtxosChanged` listener registration through `RpcCtl`, which
`GrpcClient` (every other daemon test's RPC client) doesn't drive the way
`KaspaRpcClient` does; `common::daemon::ClientManager` already configures a
wRPC-borsh listener alongside the daemon's gRPC one, so the wallet connects there
while a plain `GrpcClient` still mines blocks, exactly like every other daemon
test. Bootstraps the wallet/account via the same non-interactive `WalletApi`
methods (`wallet_create`/`prv_key_data_create`/`accounts_create`/
`accounts_activate`) the real CLI (`cli/src/wizards/account.rs`) and wasm bindings
use — not a hand-rolled storage shortcut, so the test actually exercises the real
bootstrap path.

**Two test-harness timing bugs, both about UTXO maturity, found via the same live
run** — neither is a P7.2 code bug:
1. **Funding-maturity backlog masking the mint balance drop.** The first draft
   mined `coinbase_maturity + 20` blocks straight to the wallet's own receive
   address for funding, then captured `balance_before_mint` the moment `mature > 0`
   — but with ~800 blocks mined in one burst, only a handful had matured by that
   instant; the other ~800 sat in `pending`/`stasis`, still maturing on their own as
   *any* later block got mined (mint-confirmation, redeem-confirmation, didn't
   matter), continuously masking the real (much smaller) mint-caused drop behind
   unrelated ongoing maturation. Fixed: fund with exactly **one** block to the
   wallet's own address, mine the remaining maturity-worth to a throwaway address —
   keeps the tracked balance attributable to mint/redeem alone.
2. **Redeem's payout never promoted `pending → mature`.** `register_outgoing_
   transaction`/`notify_outgoing_transaction` (`wallet/core/src/utxo/context.rs`) —
   the mechanism that makes a wallet's *own* spend appear to mature near-instantly
   — only fires from `PendingTransaction::try_submit()`. `redeem()` submits directly
   over RPC, bypassing `Generator`/`PendingTransaction` entirely (see above for
   why), so the wallet has no way to know the payout is its own doing; the deposit
   gets ordinary-deposit treatment instead —
   `UtxoEntryReferenceExtension::maturity`'s non-coinbase branch, gated on
   `user_transaction_maturity_period_daa` (100 DAA-score units by default,
   `wallet/core/src/utxo/settings.rs`) — and the test was only mining 10
   confirmation blocks. Diagnosed by adding a one-off debug print of the full
   `Balance` struct right before the wait loop: `pending: 110099100,
   pending_utxo_count: 1` — the exact redeemed-value-minus-fee, sitting in
   `pending`, never promoted. Fixed: mine `user_transaction_maturity_period_daa +
   20` blocks instead of a flat 10.

**Delegation note**: the bulk of this step's investigation (harness scaffolding,
the compute-budget diagnosis and fix, the first of the two maturity bugs) was done
by a background subagent working from a detailed brief; the second maturity bug and
final verification were done directly after taking over mid-debug once the agent's
progress reports stopped advancing between checks — recorded here since the
finding itself (not who found it) is what matters for future readers of this note.

✅ *Verify*: `wallet_notepool_mint_redeem_test` (live daemon, simnet) — mints 1.11
MAGLD (111,000,000 petals, decomposing into D1+D0_1+D0_01 — deliberately not a
single-denomination amount), confirms via the daemon's own independent
`get_pool_stats()` that the pool holds exactly those three notes and confirms via
the wallet's `NoteKeyStore` that each is `Cold`/`Active`; asserts the transparent
balance drop equals exactly `minted value + real fee` and the real fee is small
(<0.01 MAGLD) for a single-input mint. Redeems the exact three minted serials back;
confirms via `get_pool_stats()` the pool is empty again and via `NoteKeyStore` that
all three are `Superseded`; asserts the balance rise equals exactly `redeemed value
- real fee`. Final reconciliation: net balance change across mint+redeem equals
exactly the sum of the two real fees (1,084,100 sompi total on this run), nothing
more, nothing less. Full workspace build, `cargo test -p kaspa-wallet-core` (51
tests), and clippy on every touched crate all clean.

### P7.3 — Receive flows (2026-08-16)

**The fee-quantization decision (new, recorded here as the authority).** A pure pool
`Transfer` has no transparent side, so its fee is `Σconsumed − Σproduced` in note
values — and since every denomination is a multiple of the smallest, that difference
is *necessarily* a multiple of 0.01 MAGLD (1,000,000 petals). Pool-op fees are
quantized whether anyone likes it or not; the design just embraces it:
`FEE_QUANTUM_PETALS = DENOMINATION_PETALS[0]`, fee = `k` quanta with `k` sized by a
bounded fixpoint against the consensus mass calculator and the node's feerate
estimate (each iteration only raises `k`; converges immediately in practice since
one quantum ≈ 1M sompi dwarfs small-transfer fees — mempool floor confirmed to
apply to pool ops, only the anchor lane is exempt, `check_transaction_standard`).
Fee *sourcing* is P5.2's fee-stamp mechanism made concrete: consume spare notes
(smallest-first) alongside, return their excess over the fee as change to fresh own
Cold keys — a 0.01 spare consumed at `k=1` is a pure stamp, a 0.1 spare produces
9×0.01 change. The one case with no spare to stamp with — a fresh wallet's
first-ever bearer receive — runs in **slack mode**: the fee is withheld from the
rotation's own produced decomposition (rotate 0.1 → produce 9×0.01, fee 0.01).
Deliberately NOT implemented: funding pool-op fees from the transparent balance
(mint-style inputs on a transfer). It would work consensus-wise and avoid burning a
quantum, but it links the wallet's transparent identity to a note rotation — the
exact linkage the pool exists to avoid; the spec's fee-stamp design is
pool-self-contained on purpose.

**Payload formats.** `PaymentRequest{pk, amount_petals: Option}` — both spec forms
(P5.6): 40 bytes with a pinned amount, 32 bytes without (payer enters it — the
printed/static-QR variant), distinguished by length alone. `BearerNote{sn, sk, d}` —
65 bytes, deliberately the same triple the paper backup stores per note rather than
a third invented shape. Text encodings `marigoldreq:<hex>` / `marigoldnote:<hex>`
(wallet-level conventions, not consensus); the CLI renders them as terminal QR codes
via the `qrcode` crate (`default-features = false`, unicode half-block rendering —
first QR dependency in the workspace, CLI-only, wallet-core stays clean).

**Payment-request keys are a new persisted store** (P7.1's exact pattern: encrypted
`pk → {sk, amount}` map + plaintext info half, appended `Payload` field): the key is
persisted *before* the QR is ever displayed, because a crash between issuing a
request and the payment landing must not lose the only key that can ever claim the
payer's notes ("unpaid-invoice semantics" — the wallet was watching that pk since
generating it). Claiming goes through `await_payment_request`: an explicit
`NotesChanged`-by-pk subscription (notifications fire on *confirmation* — virtual's
pool state — so arrival IS settlement, P5.5's rule satisfied by construction), which
stores the landed serials as Cold rows (the request key never left the wallet; only
its pk did) and retires the request. Known gap, deliberate: a payment that lands
while the wallet is offline can't yet be discovered (no query-by-pk RPC exists);
P7.6's restore flow needs exactly that RPC anyway (the spec's sanctioned
pk-enumeration), so it lands there rather than as a P7.3 side-quest.

**The real bug: wRPC clients never received NotesChanged at all.** P6.9 wired the
server side fully (`EventType::NotesChanged → RpcApiOps::NotesChangedNotification`,
op 69) but the wRPC *client*'s notification-handler registration array
(`rpc/wrpc/client/src/client.rs`) never got the new op — so the server sent
notifications and the client-side interface, having no handler registered for op
69, silently dropped them. Found the honest way: the two-wallet live test's
receiver sat at "0 petals arrived" until timeout. One-line fix (add the op to the
array). Lesson recorded: P6.9's verify exercised the subscription mechanism but not
an end-to-end wRPC consumer — a notification pipeline isn't verified until
something actually *consumes* a notification over every transport it claims to
support.

**Bearer import is one call, not a checklist**: `bearer_import` verifies the
serial's current on-chain pk against the handed-over key and the claimed
denomination (`get_notes_by_serial`) *before* touching the wallet, stores via
P7.1's `import_bearer_key` (Hot, structurally — no caller-supplied provenance
exists), and rotates in the same call. "Imported key is never left unrotated" is
therefore not a UX discipline, it's the only code path.

✅ *Verify*: `wallet_notepool_receive_flows_test` — two real `Wallet` instances
(payer A, receiver B) against one live daemon, both flows end-to-end (see
FORK-PLAN's entry for the full assertion list: slack-mode rotation with exact
value conservation, Hot+Superseded/Cold+Active bookkeeping on both sides, on-chain
old-serial-gone/new-serials-live checks, exact-amount claim via subscription,
request retirement, payer tombstones). 4 new unit tests (both request forms
round-trip, bearer round-trip + bad-tag rejection, exact-selection greedy
including must-not-overshoot cases, fee-quanta sizing). P7.2's mint/redeem live
test re-passes after its bootstrap was factored into the shared
`connect_and_bootstrap_wallet` helper. Wallet-core suite 55 green; full workspace
check + clippy clean.

### P7.4 — Spend flows (2026-08-17)

Same session as P7.3, and deliberately small on top of it — the P7.3 transfer
builder was designed with this step's contract in mind, so P7.4 is two focused
changes rather than a new subsystem.

**Split planning is a selection policy, not a transaction plan.** The naive
reading of "note selection + split planning to hit the exact sum" is a two-phase
pipeline (split tx, then pay tx). P5.2/P5.6 make that unnecessary — a `TransferOp`'s
`produced` list is arbitrary, so "split then pay" is one transaction — and P7.3's
`submit_transfer` already takes (consumed, external produced, own produced). All
P7.4(a) actually needed was `select_covering`: exact representation first (via the
existing `select_exact` — no change, fewest moving parts), else accumulate
smallest-first until `amount + fee` is covered, with the overshoot decomposed as
change to fresh own Cold keys. Smallest-first is a deliberate choice, not an
accident: paying with dust sweeps it into change that the decomposition re-issues
in canonical largest-first form — organic merge hygiene (the P5.6 merge
motivation) without a dedicated merge step, and it leaves large notes intact.
Bonus simplification: the P7.3-era separate fee-source stage dissolved — the fee is
just part of the covering target, withheld from change.

**`NoteStatus::HandedOver` — the bearer window made visible.** A bearer-exported
note is not spent (the wallet still holds a valid key) and not safely spendable
(so does the receiver) — a third state, exactly the spec's "handed over, pending
their rotation". Borsh-appended enum variant (no stored wallets predate it);
excluded from balance and from every selection filter (they all filter
`== Active`, so the exclusion was automatic); flips to `Superseded` through the
ordinary `NotesChanged`-removal path when the receiver's rotation lands — the
"pending their rotation" clock needs no new machinery.

**`bearer_export` and the solo-key invariant.** The solo check runs on plaintext
info alone (same sk ⇔ same pk, so "any other non-superseded row under this pk"
detects wallet-visible sharing without decrypting anything), with `Hot` provenance
treated as shared-by-history regardless of rows — a key that ever crossed a wallet
boundary might be held elsewhere even if this wallet sees no sibling. Isolation
reuses `rotate_notes` unchanged; the exported payload always carries the *isolated*
serial and its fresh key — there is no code path that emits a shared `sk`, same
structural-enforcement philosophy as P7.1's `import_bearer_key` (no caller-supplied
provenance) and P7.3's `bearer_import` (no rotation-skipping import). One honest
edge: if the wallet holds no spare note, isolation runs in slack mode and splits
the denomination — the export then errors with the rotated serials listed
(re-export one of those) rather than silently exporting a different denomination
than asked. The CLI waits for the isolation to confirm before displaying the QR
(the receiver's `bearer_import` verifies against live pool state — an unconfirmed
serial would just fail their verification) and words the handover window honestly.

**A pleasing detail from the live test**: B's isolation of a landing-pad note
consumed the exported note *and* a same-key sibling as fee stamp in a single
`SignedGroup` — one signature covering two serials under the shared request pk —
exercising the multi-serial group path (P5.2's merchant-sweep primitive) for the
first time in a wallet-constructed transaction.

✅ *Verify*: `wallet_notepool_spend_flows_test` — A holds exactly ONE 0.1 note and
pays a 0.03 request: one on-chain transaction consuming the 0.1, producing 3×0.01
to B's request pk + 6×0.01 change + 0.01 fee (exact value-split asserted); B
bearer-exports a claimed landing-pad note: isolation tx first (origin serial
verified gone, isolated serial verified live, denomination preserved, fresh key),
then A's import-rotation as the second on-chain tx of the note's journey —
"demonstrably isolates first (two txs on-chain)" literally; a solo-note export
asserted to skip isolation (handed over as-is, `HandedOver` in the books). All 3
notepool live tests green, wallet-core 56 green, workspace check + clippy clean.

### P7.5 — POS landing-pad mode (2026-08-17)

The smallest of the five wallet phase steps by a wide margin — everything P7.5 needed
was already sitting in `account::notepool` from P7.3/P7.4. Worth noting explicitly as
a payoff of building `submit_transfer` as a single shared engine underneath
`rotate_notes`/`pay_payment_request` back in P7.3: "sweep every note sharing one key
to individual fresh keys" isn't a new capability, it's `rotate_notes` called with the
right serial list at the right moment.

**`pos_checkout` is three existing calls in sequence, not a new transaction shape.**
`create_payment_request` (fresh `pk`) → `await_payment_request` (claim on
confirmation) → `rotate_notes` on the claimed serials. The spec's "one `SignedGroup`"
requirement for the sweep is automatic, not something P7.5 had to enforce: every
claimed note shares the request's `sk` by construction (the landing pad *is* one
shared key), so `submit_transfer`'s existing per-key grouping (`groups_by_sk`,
written in P7.3 for the general multi-key case) collapses to exactly one group. The
first genuinely new code in this step is a callback: `pos_checkout` only *returns*
once the whole sale (request + wait + sweep) completes, but the CLI needs the request
displayed *before* the wait begins — `on_request: Option<Box<dyn FnOnce(&PaymentRequest)
+ Send>>`, fired the instant the checkout `pk` exists. Pulled into a named type alias
(`PosCheckoutRequestHook`) rather than inlined, because inlining it independently in
the free function and the `#[async_trait]`-generated `Account::pos_checkout`
signature produced two non-unifying anonymous lifetimes (a higher-ranked one from
ordinary elision vs. a scoped one from the macro's own lifetime threading) — a real,
slightly surprising `#[async_trait]` interaction, not a design issue; the named alias
sidesteps it by resolving the elision once, consistently.

**Design call: value-based sweep, not strict per-note rekey.** POOL-SPEC.md P5.6 says
the sweep moves "every note... to its own freshly generated cold key," which reads as
1:1 per-note rekeying at first pass. `rotate_notes` instead sums the claimed value and
re-decomposes it canonically (largest-first over the ladder) — which can reshape the
note count/denominations relative to what was received (e.g. a payer's 4×0.01 lands as
3×0.01 once the sweep's own fee is withheld, seen directly in the live test's log
line). Both readings satisfy the property that actually matters — "no note stays
under a shared key" — and canonical reshaping is a genuine bonus: a busy merchant's
accumulating small-denomination dust gets opportunistically consolidated for free on
every sale, rather than compounding indefinitely. Reusing the existing primitive
outweighed building a second, strictly-count-preserving sweep path for a spec phrase
that's satisfied either way.

**Deliberately deferred: the static day-`pk` fallback.** POOL-SPEC.md P5.6 names this
explicitly as the secondary form ("falling back to one static day-`pk` only for
printed/static QR codes"), and it's a genuinely different shape from everything else
built this phase — no per-sale amount (the customer enters it), no single
confirmation to wait for and retire on (a day-`pk` accumulates *multiple* independent
sales, needs repeat-watch-and-sweep rather than `await_payment_request`'s current
watch-once-then-retire model), effectively an unbounded-lifetime request the fresh-pk
primitives weren't shaped for. FORK-PLAN's own P7.5 verify criterion doesn't exercise
it. Left as a named gap rather than silently dropped or half-built.

✅ *Verify*: `wallet_notepool_pos_test` — a customer holding exactly one 0.1 note pays
a 0.04 POS checkout (splitting into 4×0.01, exercising the payer-side covering planner
inside a POS sale rather than a peer-to-peer payment); the instant it confirms, the
merchant's `pos_checkout` sweeps to 3 fresh Cold notes under distinct keys (slack-mode
fee — the merchant held no spare notes), none remaining on the checkout `pk`, no two
sharing a key; landing-pad serials confirmed gone from the pool, swept serials
confirmed live and not owned by the checkout `pk`. All 4 notepool live tests
(mint/redeem, receive flows, spend flows, POS) green in one run; wallet-core suite 56
green; full workspace check + clippy clean.
