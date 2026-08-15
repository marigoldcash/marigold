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
