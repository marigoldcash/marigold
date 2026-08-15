# X Coin — Fork & Pool Implementation Plan

## Architecture in one paragraph (the "why" behind the coin)

I have an idea for a coin that I want to build either on top off or forked out of Kaspa. 
We keep everything that makes Kaspa valuable — GHOSTDAG parallel-block PoW consensus
(10 blocks/sec), pruning, the wallet/RPC/P2P stack — and potentially change only three 
things: **network identity & economics** (new genesis, prefix, ports, seeders, fixed-cap 
emission), **add a note pool**: Notes inside the pool have a clear id are not opaque and use 
very simply asymmetric cryptography. Some argue that this coin is not private at all because 
all notes and denominations are on chain and because chain observers know exactly when and 
how much was spend at all times. This is true compared with shielded ZK proof type coins.
However, what this brings is very simple to understand asynchronous key encryption without the
need to have a phD in cryptography. Everything here is very simple. Every note has a fixed 
denomination (1/10/100/1000 etc) in the pool with its serial number and public key. The owner of 
the note holds the private key. When the owner spends the note he reveals the private key to
the new owner who then rotates the public key in the pool by signing the request with the
old private key. In addition the holder of a note can break it by the factor 10 of its 
denomination into 10 notes of 1/10th the value and vice versa can merge 10 identical 
denomination notes into one 10 times as valuable. Full amounts and serials are on the chain 
and visible to all but no sender or receiver is ever recorded. The privacy implications are 
simple to understand. Nothing is hidden by complex mathematics which is not easily verifyable 
and could potentially ontain snake oil. Lastly **we change the role of 'Wallets'**. Wallets
have no 24 word recovery key because its just a piece of software that manages your database 
holding your collection of private keys not tied to one identity or one key. The role of the 
wallet software is simply to be able to receive a set of private keys (by QR or otherwise) 
and imediately initiate the public key swap in the pool and receive a confirmation. Further 
its role is to receive a spending amount from the user and prepare coins to fit that spend 
(by breaking them if neccessary) and then creating a private key (QR encoded for easy scan
or tranfer) set. The wallet also rotates keys when signaled by the network that upgrades are 
required for the algorythm and old standards are no longer good enough.

---

Below follows a tick-offable plan for a **non-shielded, very simplistic fixed-denomination 
transparent bearer-note coin** — "digital cash with a fully auditable ledger" build step by step 
as a fork of rusty-kaspa. Written so that each step is small enough to be executed correctly
by a less capable coder in a single coding session.

## How to use this document

- Work **strictly in order** within a phase. Phases 0–4 are sequential. Phase 5 (spec) can
  begin while Phase 4 stabilizes. Phase 6 starts only after the P5.9 review gate; Phase 8
  overlaps late Phase 7 (start the P8.7 testnet soak as early as it can stand); Phase 9
  starts only after the P8.8 audit gate closes.
- Each step has an ID (e.g. `P2.3`). In a session, say *"do step P2.3 of FORK-PLAN.md"*.
  The step contains everything needed.
- A step is **done** only when its ✅ *Verify* command passes. Tick the box, commit with the
  step ID in the message (e.g. `P2.3: rebrand address prefix`).
- Steps marked ⚠️ **HARD** are research-grade: do them with extra care, expect iteration,
  and treat external expert review as mandatory before real value touches the code.
- Steps marked 🧑‍⚖️ **DECISION** need a human choice.

## Ground rules (read once, they prevent expensive mistakes)

1. **Do not rename the `kaspa-*` crates or Rust module paths.** Rebrand only user-facing
   strings (ticker, address prefix, network names, CLI text). Reason: upstream rusty-kaspa
   receives security fixes constantly; keeping internal names identical keeps `git merge
   upstream/master` feasible for years. Cosmetic renames would create thousands of conflicts.
2. **Never touch consensus code and rebranding in the same commit.** Small commits, one
   concern each.
3. **Every consensus change gets a test in the same PR.** No exceptions.
4. **The pool ships on testnet for months before mainnet.** Cryptographic code that guards
   money must soak.
5. Work on a branch (`x-fork`), keep `master` tracking upstream (`git remote add upstream
   https://github.com/kaspanet/rusty-kaspa`). The fork repo is **private during Phases
   0–4** (freedom to be messy, narrative control) and **goes public at spec freeze (P5.9)
   or Phase 6 start at the latest** — external review, public-testnet testers (P8.7), and
   fair-launch credibility all require open source well before mainnet. This is a
   commitment, not a drift: a "verifiable honesty" coin must not be developed in secret
   longer than necessary.


## Phase 0 — Environment & orientation

*Goal: build, test, and run a private network of the unmodified code. Nothing here
is fork-specific; it de-risks everything after.*

- [x] **P0.1 — Build the workspace.**
  Install prerequisites per [README.md](README.md) (Windows section: protoc, LLVM with the
  `AR.exe` copy trick, Rust ≥ 1.91). Then `cargo build --release --bin kaspad`.
  ✅ *Verify:* `cargo build --release --bin kaspad` exits 0.

- [x] **P0.2 — Run the test suite once.**
  `cargo test --release` (or `cargo nextest run --release`). Takes a while; this is your
  baseline — from now on, any red test you cause is yours.
  ✅ *Verify:* suite passes (record any pre-existing skips/failures in a note file).

- [x] **P0.3 — Run a devnet node.**
  `cargo run --release --bin kaspad -- --devnet --enable-unsynced-mining --rpclisten-borsh=127.0.0.1 --utxoindex`
  ✅ *Verify:* node starts, logs show devnet params, RPC answers (e.g. connect `cargo run
  --release -p kaspa-cli` and run `rpc get-info` equivalent).

- [x] **P0.4 — Mine devnet blocks.**
  Use the community CPU miner (github.com/elichai/kaspa-miner or equivalent) pointed at the
  devnet node, or use `simpa` for in-process simulation:
  `cargo run --release --bin simpa -- -t=10 -d=2 -b=8 -n=500`.
  ✅ *Verify:* node log shows accepted blocks / simpa completes and prints DAG stats.

- [x] **P0.5 — Exercise a wallet on devnet.**
  Run `cargo run --release -p kaspa-cli`, create a wallet, get a devnet address, mine to it,
  send a transaction to a second address.
  ✅ *Verify:* second address shows a balance via the CLI.
  **Executed via RPC instead of `kaspa-cli`** (deliberate substitution — full rationale
  and transcript in [NOTES.md](docs/x-fork/NOTES.md)): `kaspa-cli` turned out to be
  REPL-only (P0.3 finding) and can't be scripted; more importantly, the traditional
  `kaspa-wallet-core` seed/key-DB layer it fronts is exactly what this project's wallet
  redesign (see the architecture paragraph and Phase 5-7) replaces, so proving it
  end-to-end wasn't worth the investment. Used `kaspa-addresses` to generate a keyless
  recipient address and `rothschild` (the repo's built-in tx generator) to generate a
  real keypair, mine to it, and send a signed transaction — verified the recipient's
  balance via `get_balance_by_address` RPC.

- [x] **P0.6 — Write the orientation notes file.**
  Create `docs/x-fork/NOTES.md` recording: build quirks encountered, the exact miner used,
  commands that worked. Future sessions start by reading it.
  ✅ *Verify:* file exists and includes the working run commands.

---

## Phase 1 — Design decisions 🧑‍⚖️

*Goal: every parameter that code will encode is decided and written down. Each item is a
decision to record in `docs/x-fork/DECISIONS.md` (create it in P1.1). No code changes.*

- [x] **P1.1 — Create the decisions file** `docs/x-fork/DECISIONS.md` with a table:
  Decision / Choice / Rationale / Date.
  ✅ *Verify:* file exists.

- [x] **P1.2 — Name & ticker.** Working name "X Coin" collides with everything (and with
  X/Twitter). Pick a real name, a 3–5 letter ticker, and check collisions on CoinGecko/
  CoinMarketCap. Record: name, ticker, address prefix string (lowercase, short, e.g.
  `xcn`), testnet prefix (e.g. `xcntest`).
  **Executed 2026-08-14.** Name **Marigold** confirmed as pre-decided (2026-08-13):
  collision-checked, no coin/CMC listing; known non-coin name-neighbors to stay clear
  of are marigold.dev (a Tezos dev company) and marigold.com (a martech firm). Ticker
  ~~MGLD~~ **MAGLD** — the pre-decided MGLD didn't survive execution-time recheck (a
  minor collision with an obscure, dead, unverified BSC token); a considered
  alternative MCASH had a worse thematic collision (a dead 2019 project pitching
  itself as private+feeless digital cash). MAGLD verified fully clean on both
  CoinGecko and CoinMarketCap. Full search trail in
  [DECISIONS.md](docs/x-fork/DECISIONS.md). Address prefixes (unaffected by the ticker
  change, confirmed as pre-decided):
  `marigold` / `marigoldtest`. Canonical domain **marigold.cash** (precedent: Zcash's
  canonical z.cash); already registered and 301-redirecting to it: marigoldcoin.com/.io/
  .net/.org, marigold-coin.com, marigoldcash.com/.net/.org, marigold-cash.com,
  marigoldwallet.com (all auto-renew, expiry Aug 2027; marigoldcash.io deliberately
  deferred — revisit at Phase 9). One handle string, identical on all platforms (GitHub
  org, X, Telegram, Discord, Reddit, YouTube, Docker Hub, npm scope): register both
  `marigoldcash` and `marigoldcoin`, primary = whichever is free everywhere. Brand
  notes: the marigold is the classic companion plant protecting a young
  garden — the finality-anchor story; "Mary's gold" etymology carries the money
  association.

- [x] **P1.3 — Units & precision.** Recommend keeping Kaspa's 8 decimals (1 coin = 10⁸ base
  units, Kaspa's "sompi"). Name your base unit. Record it.
  **Executed 2026-08-14, confirmed as pre-decided (2026-08-13) with no changes:** kept
  Kaspa's 8 decimals; base unit **petal** — 1 marigold = 10⁸ petals. Recorded in
  [DECISIONS.md](docs/x-fork/DECISIONS.md).

- [x] **P1.4 — Supply & emission.** Decision has to be made on fixed cap (any round
  cap works), smooth geometric decay like Kaspa's "chromatic" schedule (halving the
  block reward every N months) rather than Bitcoin cliff-halvings — smoother miner economics.
  Choose: **total cap**, **emission duration** (e.g. ~30 years), **initial per-second 
  reward**. Record all three. (Implementation is P3.2.)
  **Executed 2026-08-14.** Cap **210,000,000 MAGLD** (chosen over the 21M for
  note-denomination granularity — keeps the 0.01 smallest note usable for sub-dollar
  private payments at realistic unit prices), hard cap, **no tail emission**. Smooth
  geometric decay from genesis, **halving every 3 years** (monthly factor 2^(−1/36)),
  no pre-deflationary phase — ~20.6% mined in year 1, ~90% by year 10, reward quantizes below 
  1 petal ≈ year 72 (fade-out, not cliff). Initial reward derived:
  ≈ **1.5228 MAGLD/sec** (exact petal value fixed by P3.2's cap-asserting generator).
  Endgame posture recorded: anchors → emission → circulation fees. Full math and
  considered but rejected alternatives in [DECISIONS.md](docs/x-fork/DECISIONS.md).

- [x] **P1.5 — Launch allocation.** Fair launch (mine from zero), premine %, or airdrop?
  This is an economics/credibility/legal decision, not code. If premine: how much, vesting,
  and to what governance. Record it. *Note: honesty + large premine is a hard sell;
  fair launch or small transparent dev fund (<10%) is the defensible zone.*
  **Executed 2026-08-14: fair launch from zero.** No premine, no dev fund, no airdrop —
  every MAGLD is mined via the P1.4 emission schedule starting at genesis. No
  vesting/governance question applies (nothing is pre-allocated). Recorded in
  [DECISIONS.md](docs/x-fork/DECISIONS.md).

- [x] **P1.6 — Denominations.** For the coin pool recommend powers of ten in whole
  coins: {0.01, 0.1, 1, 10, 100, 1000}. Record the set.
  **Executed 2026-08-14 — extended above the recommended set.** Final set:
  **{0.01, 0.1, 1, 10, 100, 1000, 10000, 100000}** (8 tiers). Since split/merge moves
  exactly 10× either direction, extending the ladder upward costs nothing at the
  small-payment end while sparing large holders from managing piles of 1000-notes.
  Rationale in [DECISIONS.md](docs/x-fork/DECISIONS.md).

- [x] **P1.7 — Transparent tier policy.** The fork launches transparent-only (Phases 2–4)
  and adds the pool (Phases 5–7).
  **Confirmed as-is, 2026-08-14** — no change to the phase structure. Recorded in
  [DECISIONS.md](docs/x-fork/DECISIONS.md).

- [x] **P1.8 — Fee policy.** Keep fees (near-zero like Kaspa). We established zero-fee +
  reward-per-action is a spam machine; this plan keeps Kaspa's fee market untouched. Record
  simply: "inherit Kaspa fee model."
  **Executed 2026-08-14 (revised same day).** Transparent tier: inherit Kaspa's fee
  model as-is. Pool ops: pay with **fee stamps** — whole small-denomination notes
  consumed inside the op via an embedded redeem-with-no-transparent-output, whose
  value becomes the miner fee through Kaspa's native value-in-minus-value-out
  accounting. One conservation rule across all ops: Σ(note in) + Σ(transparent in) =
  Σ(note out) + Σ(transparent out) + fee. The wallet holds nothing but note keys. Bootstrap, 
  stamp sizing (possible 0.001 tier, deferred to P6.6/P8.3 calibration), and the
  stamp-lineage note are in [DECISIONS.md](docs/x-fork/DECISIONS.md).

- [x] **P1.9 — Regulatory posture.** One paragraph: which jurisdictions you'll operate/
  incorporate in and that distribution will depend on CEXs willingness to adopt the coin. 
  Get real legal counsel before mainnet (P9.6).
  **Executed 2026-08-14.** No legal entity for now (a Swiss-style nonprofit foundation
  is the leading candidate if one becomes necessary later — jurisdiction deferred, not
  decided); consistent with P1.5/P1.8 since there's no revenue or treasury for an
  entity to hold. We are not a "Privacy Coin" and with EU's AMLR going into full effect 
  10 July 2027 distribution dependend on CEXs willingness to adopt the coin and DEXs and P2P
  usage — consistent with the project's shape. 
  Full paragraph in [DECISIONS.md](docs/x-fork/DECISIONS.md).

- [x] **P1.10 — Network numbers.** Pick non-colliding defaults, record them:
  P2P port, gRPC port, borsh-wRPC port, JSON-wRPC port for mainnet + testnet
  (Kaspa uses 16110/16210-family — pick a different thousand-block, e.g. 26110-family).
  **Executed 2026-08-14.** Mirrors Kaspa's exact port structure shifted to the
  **26xxx/27xxx/28xxx** block (chosen over reusing Kaspa's ports outright — see
  [DECISIONS.md](docs/x-fork/DECISIONS.md), which cites Kaspa's own source comment
  explaining why they vary ports per network: avoiding same-host bind conflicts, not
  protocol collisions). gRPC: mainnet **26110**, testnet **26210** (simnet 26510,
  devnet 26610). borsh-wRPC: mainnet **27110**, testnet **27210** (simnet 27510,
  devnet 27610). JSON-wRPC: mainnet **28110**, testnet **28210** (simnet 28510,
  devnet 28610). P2P: mainnet **26111**, testnet **26211**/26311/26411
  (suffix-dependent, mirroring Kaspa's own scheme) (simnet 26511, devnet 26611).

---

## Phase 2 — Fork identity (make it a different network)

*Goal: a node that builds from this repo is a **new network**: it will not connect to Kaspa
peers, Kaspa addresses are invalid on it, and it has its own genesis. Everything here is
small, mechanical, and individually testable.*

- [x] **P2.1 — Address prefix.**
  Edit [crypto/addresses/src/lib.rs](crypto/addresses/src/lib.rs) — the `Prefix` enum's
  string mappings (`"kaspa"`, `"kaspatest"`, `"kaspasim"`, `"kaspadev"` → your P1.2
  prefixes) in all three places (serde rename, `Display`, `FromStr`) plus
  [crypto/addresses/src/wasm.rs](crypto/addresses/src/wasm.rs). Fix the address test vectors
  in the same file (tests encode prefix strings; recompute expected bech32 outputs by
  running the tests and updating from failures — the checksum covers the prefix, so all
  vectors change).
  ✅ *Verify:* `cargo test -p kaspa-addresses` passes; a generated address starts with your
  prefix.
  **Executed 2026-08-14.** `marigold`/`marigoldtest` per P1.2; extended `marigoldsim`/
  `marigolddev` for simnet/devnet by the same base+suffix convention (mechanical, no
  new naming stakes — never public-facing). Fixed 4 network-prefixed test vectors in
  `lib.rs` plus `test_errors`' hardcoded strings (which would otherwise fail on
  `InvalidPrefix` before even reaching the specific error each one tests) by iterating
  fix→rerun→capture-checksum-from-failure, and one doc-comment example. Also found and
  fixed a stale prefix in `crypto/addresses/benches/bench.rs` — not mentioned in this
  step's scope, but in the same crate and would have broken the benchmark; caught by
  grepping the whole crate rather than trusting the step description's file list.
  `cargo test -p kaspa-addresses` (3/3 + doctests) and `cargo check --benches` both
  green; confirmed a freshly generated address starts with `marigold:`.

- [x] **P2.2 — Network ports.**
  Edit [consensus/core/src/network.rs](consensus/core/src/network.rs):
  `default_rpc_port`, `default_borsh_rpc_port`, `default_json_rpc_port`, `default_p2p_port`
  → your P1.10 numbers.
  ✅ *Verify:* `cargo test -p kaspa-consensus-core network` passes; started node logs show
  new ports.
  **Executed 2026-08-14.** All four functions updated to the P1.10 26xxx/27xxx/28xxx
  scheme (mainnet/testnet/simnet/devnet, including the testnet suffix-dependent P2P
  cases). `cargo test -p kaspa-consensus-core network` — 2/2 green (neither test
  touches port numbers directly, both are network-ID string parsing, unaffected).
  Rebuilt `kaspad` and started a devnet node: log confirmed GRPC `26610`, P2P `26611`,
  WRPC(borsh) `27610` — matches the devnet row of the P1.10 table exactly.

- [x] **P2.3 — P2P network isolation.**
  The P2P handshake exchanges a network name derived from `NetworkId` (e.g.
  `kaspa-mainnet`). Grep `protocol/p2p` and `consensus/core/src/network.rs` for how
  `network_name()` / `to_prefixed()` feed the version handshake; change the base string so
  the handshake name becomes e.g. `xcn-mainnet`.
  ✅ *Verify:* unit tests pass **and** an integration check: start your node, attempt to
  connect it to a public Kaspa node (`--addpeer`), confirm the log shows a
  network-mismatch rejection.
  **Executed 2026-08-14.** `NetworkId::to_prefixed()`/`from_prefixed()` in
  `consensus/core/src/network.rs` now use `marigold-` instead of `kaspa-` (this is the
  single source of truth — `Config::network_name()`, the gRPC `network_name` field,
  and `RpcNetworkId` (a type alias for `NetworkId`) all inherit it automatically). Also
  fixed the same literal in `protocol/p2p/src/echo.rs`'s example handshake tool
  (outside the step's stated file list, but in `protocol/p2p` as instructed, and would
  otherwise fail to talk to our own real node). `cargo test -p kaspa-consensus-core
  network` and `cargo test -p kaspa-p2p-lib` both green. **Live integration test**:
  found a real Kaspa mainnet node with an open P2P port (`seeder2.kaspad.net`,
  `--addpeer`'d directly, sandboxed to a scratch `--appdir` to avoid touching a
  pre-existing unrelated mainnet datadir on this machine) — confirmed the real peer's
  reject message: `Network mismatch - local: kaspa-mainnet, remote: marigold-mainnet`.
  (First attempt gave a false pass — actually connected and started IBD — because the
  `kaspad` binary hadn't been rebuilt since the source edit; see NOTES.md.)

- [x] **P2.4 — Strip Kaspa's DNS seeders.**
  In [consensus/core/src/config/params.rs](consensus/core/src/config/params.rs), set
  `dns_seeders: &[]` for MAINNET_PARAMS and TESTNET_PARAMS (you'll add your own in P9.2).
  ✅ *Verify:* `cargo build` passes; node starts and logs no Kaspa seeder lookups.
  **Executed 2026-08-14.** Both lists emptied (SIMNET/DEVNET were already `&[]`
  upstream). `cargo build --release --bin kaspad` clean; started a mainnet node
  (sandboxed `--appdir`) and confirmed zero seeder-lookup log lines, vs. the seeder
  queries P2.3's live test incidentally showed happening with the old populated list.
  `cargo test -p kaspa-consensus-core` — 58/58 + 7/7 green, nothing else affected.
  Own seeders come later at P9.2 — see the coder's Cloudflare-delegation question
  answered in NOTES.md, which sketches the mechanism ahead of that step.

- [x] **P2.5 — New genesis blocks (mainnet + testnet).**
  Edit [consensus/core/src/config/genesis.rs](consensus/core/src/config/genesis.rs). For each
  network: write a new `coinbase_payload` message (your genesis motto — this alone changes
  all hashes), set `timestamp` to your launch epoch (ms), keep `bits` (testnet-grade initial
  difficulty is fine for a new network; mainnet: consider an easier initial `bits` than
  Kaspa's since you start with ~zero hashrate — copy the devnet approach), set
  `utxo_commitment` to `EMPTY_MUHASH` and `daa_score: 0` (you have no checkpoint history).
  Then run the genesis test — it **panics with the correct expected hashes printed in hex**
  (see `assert_hashes_eq` in that file); paste the printed `hash_merkle_root` and `hash`
  back into the constants. Two iterations of run-test/paste-hash converge.
  *Note: genesis PoW nonce does not need real mining — genesis is trusted by definition;
  the test only enforces internal hash consistency.*
  ✅ *Verify:* `cargo test -p kaspa-consensus-core genesis` passes.
  **Executed 2026-08-14.** Mainnet motto: *"Hell is other people's monetary policy. —
  Sartre"* (coder's choice). Testnet motto: plain `marigold-testnet` identifier,
  matching upstream's own testnet/simnet convention (coder's choice, to keep the
  quote unique to mainnet). Mainnet also got `bits: 0x1e21bc1c` (devnet's easy value,
  copied per the plan's guidance — real Kaspa's `486722099` would be untouchable at
  zero launch hashrate), `daa_score: 0`, `utxo_commitment: EMPTY_MUHASH` (mainnet
  previously carried a real checkpoint-reset structure with embedded Bitcoin/
  checkpoint block hashes — all removed, this is a genuine from-scratch genesis).
  Testnet's `bits` kept as-is (`0x1e7fffff`, already testnet-grade); its
  `utxo_commitment`/`daa_score` were already `EMPTY_MUHASH`/`0` upstream. Both got a
  placeholder timestamp (`1786742438234`ms, today) — **P9.5 regenerates genesis with
  the real launch timestamp and motto per the plan's own design, so this is
  explicitly a Phase-2-milestone placeholder, not final.** Hashes recomputed via the
  run-test/paste-hash dance (4 iterations: mainnet merkle root, mainnet hash, testnet
  merkle root, testnet hash). `cargo test -p kaspa-consensus-core genesis` green,
  plus full crate suite (58/58 + 7/7). Live sanity check: rebuilt `kaspad`, started
  sandboxed mainnet- and testnet-mode nodes on the new genesis — both bootstrap
  cleanly, no panics. **Found a real bug along the way** — `consensus/src/consensus/mod.rs`
  hardcoded 16 real Kaspa mainnet 2021 checkpoint `(daa_score, timestamp)` pairs
  specifically for `NetworkType::Mainnet`, feeding the `get_daa_score_timestamp_estimate`
  RPC, now stale/wrong data for our fictional mainnet genesis — deliberately not fixed
  in this commit (different file/concern), **fixed immediately after as its own
  commit**: removed the whole special-case branch (Marigold's mainnet genesis is
  `daa_score: 0` like every other network — no pre-genesis history to splice in).
  See [NOTES.md](docs/x-fork/NOTES.md) for the full writeup.

- [x] **P2.6 — Reset fork activations.**
  In [params.rs](consensus/core/src/config/params.rs) set for your mainnet/testnet:
  `crescendo_activation: ForkActivation::always()` and
  `toccata_activation: ForkActivation::always()` — a new chain starts with all upgrades
  active from block 0 (no history to protect). This gives you 10 BPS + covenants + ZK
  opcodes from genesis and deletes an entire class of fork-transition complexity.
  ✅ *Verify:* `cargo test -p kaspa-consensus` passes;
  `grep -n "activation" consensus/core/src/config/params.rs` shows always() for your nets.
  **Executed 2026-08-14.** The flag flip itself was one line each, but it surfaced 5
  real test failures needing genuine investigation, not blind acceptance — full
  root-cause writeup in [NOTES.md](docs/x-fork/NOTES.md). Summary: (1)
  `pre_crescendo_target_time_per_block` was still real Kaspa's 1-BPS legacy value,
  inconsistent with `always()` — fixed to match `blockrate` (mirrors simnet/devnet's
  existing self-consistent pattern). (2) `deflationary_phase_daa_score` was still real
  Kaspa's non-zero legacy checkpoint — set to `0` for both networks, which
  *implements the already-locked P1.4 decision* ("no pre-deflationary phase"), not a
  new economics call; the real subsidy table stays P3.2's job. (3) A test-only helper,
  `TestConsensus::build_header_with_parents`, hardcoded the pre-toccata block version
  via a generic `Header::from_precomputed_hash` default — fixed to derive it from
  `params.block_version().get(daa_score)`; a latent test-infra gap, not a production
  bug, just never exercised against an always()-active mainnet/testnet before. (4)
  Two tests had subsidy literals (`50000000000`→`5000000000`,
  `44000000000`→`4400000000`) needing the same BPS-scaling already applied elsewhere.
  `cargo test -p kaspa-consensus` 72/72, `kaspa-consensus-core` and `kaspa-mining`
  also rechecked green, full `cargo build --workspace` clean. Live check: rebuilt
  `kaspad`, mined on a **sandboxed real mainnet-mode node** (not devnet, which P2.6
  doesn't touch) — 10 BPS acceptance confirmed, no version/subsidy rejections.

- [x] **P2.7 — User-facing rebrand pass 1 (node).**
  Grep `kaspad/src`, `core/src`, `daemon/src` for user-visible strings: application name,
  log banner, `--help` text, default app-dir name (so your node's data dir is
  `~/.xcn` -equivalent, not `~/.rusty-kaspa` — grep `app_dir`/`get_app_dir`). Change
  display strings only — not crate names, not module paths (Ground rule 1).
  ✅ *Verify:* `cargo run --release --bin kaspad -- --help` shows your name; a fresh run
  creates your app dir.
  **Executed 2026-08-14.** App dir: `~/.rusty-kaspa` → `~/.marigold` (Windows:
  `rusty-kaspa` → `marigold`) in `kaspad/src/daemon.rs`. Log file names:
  `rusty-kaspa.log`/`rusty-kaspa_err.log` → `marigold.log`/`marigold_err.log` in
  `core/src/log/consts.rs`. `--help` banner: `.about()` text in `kaspad/src/args.rs`
  now reads "Marigold full node daemon (marigold-node) v{version}". Also updated the
  `description` field in all three touched crates' `Cargo.toml` (kaspad, kaspa-core,
  kaspa-daemon) for consistency, since `kaspad`'s feeds directly into the `--help`
  text. **Deliberately left untouched, per Ground rule 1**: the crate/binary name
  itself (`kaspad`) and every log line/message that refers to it by that name (e.g.
  `"Kaspad has stopped..."`, DB-version-mismatch prompts) — these remain accurate
  since the binary genuinely is still called `kaspad`. Also left a large `/* ... */`
  block comment in `args.rs` untouched — it's dead reference documentation (never
  compiled/displayed), already stale relative to reality even before this fork
  (shows Kaspa's old pre-P2.2 ports), out of scope for a live-strings rebrand pass.
  `cargo build --release --bin kaspad` clean; `--help` output confirmed; a fresh run
  with an isolated `$HOME` confirmed `~/.marigold/` gets created (not
  `~/.rusty-kaspa/`). `cargo test -p kaspa-core -p kaspad -p kaspa-daemon` all green.

- [x] **P2.8 — User-facing rebrand pass 2 (wallet + CLI).**
  Grep `wallet/` and `cli/` for `"KAS"`, `"kaspa"` in display strings, ticker formatting,
  and URLs. Same rule: display strings only.
  ✅ *Verify:* `cargo run --release -p kaspa-cli` shows your ticker in balances.
  **Executed 2026-08-14.** First folded in the still-open P2.1 regression: 22 failing
  `kaspa-wallet-core` tests from hardcoded `"kaspa:..."` test-fixture addresses.
  Batch-fixed all 344 (346 with duplicates) across 5 wallet files by writing and
  self-verifying (against 3 known-good P2.1 vectors) a standalone bech32
  re-prefixing tool — decodes an old address's exact payload and re-encodes it under
  the new prefix, rather than generating fresh arbitrary addresses, since several
  (the gen0/gen1 legacy-derivation test vectors) are cryptographically tied to fixed
  seed keys and must keep their exact derived payload. Then did the actual P2.8
  sweep: ticker suffix `KAS`/`TKAS`/`SKAS`/`DKAS` → `MAGLD`/`TMAGLD`/`SMAGLD`/`DMAGLD`
  (the two duplicate `kaspa_suffix()` implementations, in `wallet/core` and
  `wallet/pskt`); 7 internal account-storage-kind tags (`kaspa-bip32-standard` etc.)
  → `marigold-*`; default wallet storage folder/file (`~/.kaspa`/`kaspa` →
  `~/.marigold`/`marigold`); the terminal link-matcher regex and three
  `explorer.kaspa.org` URLs in `cli/matchers.rs` (was matching the wrong prefix
  entirely and pointing at the wrong network's explorer — now
  `explorer.marigold.cash`, a forward placeholder pending P9.4); CLI balance-display
  strings; the `marigold-cpu-miner` binary search name. **Deliberately left
  unchanged**: the `kaspad` binary name and its own log messages, `kaspa_utils`
  crate paths, the WASM/JS public API surface (`kaspaToSompi` etc. — treated as
  identifiers, not display strings, consistent with the `kaspad` decision), and
  `compat/gen0.rs`'s real external legacy-Kaspa-wallet file paths/storage key (genuine
  interop with actual third-party software, not our own branding). While running a
  full-workspace test pass to verify (given the P2.1 lesson that targeted checks
  miss cross-crate regressions), found and fixed the **same P2.1 regression pattern
  in `crypto/txscript`** (2 real script-derived addresses, same payload-preserving
  fix) and **two more real bugs, each fixed as its own separate commit**: a
  P2.6-pattern test assertion in `testing/integration` expecting the wrong block
  version, and a genuine `bridge/` (stratum-bridge) address-validation bug where the
  wallet-address regex and fallback-prefix logic still expected `kaspa:`. Verified:
  `sompi_to_kaspa_string_with_suffix()` called directly (kaspa-cli is REPL-only, per
  P0.3) confirms output `"1,234.56789012 MAGLD"`. Full workspace clean:
  `cargo build --workspace` and `cargo test --workspace` both green (144 test-result
  blocks, 0 failures) — matches and exceeds the P0.2 baseline. Full writeup in
  [NOTES.md](docs/x-fork/NOTES.md).

- [x] **P2.9 — Two-node private network smoke test.** Executed 2026-08-15.
  Rebuilt `kaspad` fresh (standing lesson). Started two devnet nodes with separate
  `--appdir`s and non-conflicting ports (node A: defaults — gRPC 26610, borsh-wRPC
  27610, P2P 26611; node B: `--listen=127.0.0.1:26621 --rpclisten=127.0.0.1:26620
  --rpclisten-borsh=127.0.0.1:27620 --rpclisten-json=127.0.0.1:28620
  --addpeer=127.0.0.1:26611`). Generated a throwaway `marigolddev:` sink address the
  same way as P0.4 (no real key needed), mined ~15s with `kaspa-miner` against node
  A's gRPC port only. Node B's log showed the identical block hashes node A logged
  "via submit block" being accepted "via relay" in the same order, confirming real
  P2P sync rather than independent mining. Both nodes' handshake logged protocol
  version 9 (`Registering p2p flows ... for protocol version 9`) — the network
  isolation from P2.3 doesn't block same-network peers, only cross-network ones, as
  expected.
  ✅ *Verify:* the same throwaway-edit-then-revert pattern used at P0.3/P0.4/P2.5/P2.6
  (`rpc/grpc/examples/simple_client`, port made a CLI arg, reverted after) queried
  both nodes: **identical** block count 119, header count 119, virtual DAA score 119,
  tip hash, sink, and pruning point hash. Both `is_synced: true`. Both processes
  stopped cleanly after (`pkill -x kaspad`); scratch appdirs and logs kept under the
  session scratchpad, nothing added to the repo. Full log excerpts in
  [NOTES.md](docs/x-fork/NOTES.md).

---

## Phase 3 — Economics (emission with a hard cap)

*Goal: your P1.4 supply schedule is enforced by consensus and provably sums below the cap.*

- [x] **P3.1 — Understand the existing emission.** Executed 2026-08-15.
  Read [consensus/src/processes/coinbase.rs](consensus/src/processes/coinbase.rs) in full
  plus its production wiring/call sites. Key findings for P3.2: the pre-deflationary flat
  subsidy branch is already dead code for us (P2.6 set `deflationary_phase_daa_score = 0` —
  decay starts at block 1, matching P1.5's fair launch); the BPS-scaling "before" table copy
  is likewise unused (P2.6 set `crescendo_activation = always()` — 10 BPS from genesis, only
  `subsidy_by_month_table_after` is ever read); the table already tapers to an exact `0` at
  its last entry with no separate tail-cutoff logic needed. `calc_block_subsidy()` is consumed
  at exactly two sites: block validation
  ([body_validation_in_context.rs:74](consensus/src/pipeline/body_processor/body_validation_in_context.rs))
  and coinbase-template generation
  ([virtual_processor/processor.rs:1454](consensus/src/pipeline/virtual_processor/processor.rs),
  [utxo_validation.rs:304](consensus/src/pipeline/virtual_processor/utxo_validation.rs)), both
  through one `CoinbaseManager` built once in
  [services.rs:124](consensus/src/consensus/services.rs) from `Params`. Flagged the open
  design choice for P3.2: regenerate an equivalent large lookup table (halving boundary +
  smooth intra-period interpolation, stretched to 3-year/36-entry halving periods) vs. replace
  with a directly-computed closed-form decay function — not deciding here, bringing it to P3.2
  explicitly.
  ✅ *Verify:* summary written to
  [docs/x-fork/NOTES.md](docs/x-fork/NOTES.md) (P3.1 entry), correctly identifying both
  consumption sites (validation + template generation) and the one shared `CoinbaseManager`
  instance they both go through.

- [x] **P3.2 — Generate your subsidy table.** Executed 2026-08-15.
  Kept the existing table-driven architecture (option (a) from P3.1) rather than switching to a
  closed-form calculation. Wrote the generator as a permanent `#[ignore]`d test,
  `processes::coinbase::tests::generate_subsidy_table` in
  [coinbase.rs](consensus/src/processes/coinbase.rs) — bisects for the largest base subsidy
  whose discrete, rounded monthly table still sums to ≤ the 210M cap. Result: **1016 months**
  (vs Kaspa's 426 — a 3-year halving decays 3× slower), base ≈ **1.5228084263 MAGLD/sec**, total
  emission 20,999,999,999,644,200 petals, ~0.0036 MAGLD under cap, table tapers to an exact 0.
  `SUBSIDY_BY_MONTH_TABLE_SIZE` updated to 1016 (grepped the workspace — no other consumers of
  the table/constant exist outside this file). `deflationary_phase_daa_score: 0` for
  mainnet/testnet/devnet was already set at P2.6; **left simnet on its real pre-deflationary
  value** after confirming (not assuming) it's load-bearing for
  `daemon_integration_tests::daemon_utxos_propagation_test` — simnet is an internal PoW-skipped
  benchmark harness, not a real network, so P1.4/P1.5's fair-launch commitment doesn't bind it.
  Rewrote `subsidy_test` to spot-check the DAA-score → month → table-lookup → BPS-scaling wiring
  against real table entries by index, rather than re-deriving expectations via Kaspa's original
  `initial_subsidy / 2^n` halving-count shortcut (confirmed empirically that shortcut doesn't
  hold exactly for our table). Added a permanent `total_emission_stays_under_cap` test (the
  actual cap enforcement). Fixed three more real bugs found via full-workspace + ignored-test
  verification: a hardcoded subsidy literal in `body_validation_in_context.rs`'s test; a latent
  P2.2-era bug in `verify_crescendo_emission_schedule` (BPS-unaware legacy-calc cross-check,
  never actually run until this step's diligence pass); and five `goref_*` integration tests
  that replay real historical Kaspa mainnet block data — permanently incompatible with a
  from-scratch economics schedule, so `#[ignore]`d (with reason) rather than fixed. Full
  writeup, numbers, and bug root-causes in [NOTES.md](docs/x-fork/NOTES.md) and
  [DECISIONS.md](docs/x-fork/DECISIONS.md).
  ✅ *Verify:* `total_emission_stays_under_cap` passes as a permanent test;
  `cargo test -p kaspa-consensus --lib coinbase` — 7 passed, 2 ignored (generator + the
  ~15-20-minute `verify_crescendo_emission_schedule`), 0 failed. Full `cargo build --workspace`
  / `cargo test --workspace`: 144 test-result blocks, 0 failures, matching the P2.8 baseline.

- [x] **P3.3 — Emission integration check.** Executed 2026-08-15.
  Rebuilt `kaspad` fresh. Fresh single-node `--utxoindex` devnet, mined 1097 real blocks
  (block/header count 1098 includes genesis, which mints no coinbase) with `kaspa-miner`.
  Queried `get_coin_supply` over gRPC (throwaway-edit-then-revert on
  `rpc/grpc/examples/simple_client`, same pattern as prior live checks): **circulating supply
  16,705,209,245 petals = exactly 1097 × 15,228,085** — the table's month-0 per-block value
  (`SUBSIDY_BY_MONTH_TABLE[0].div_ceil(10)`) with **zero deviation** (single miner, no
  red/merged blocks on a linear devnet chain, so no rounding slack to account for). Also caught
  and fixed a real bug surfaced by this same RPC call: `get_coin_supply`'s `max_sompi` field
  reported real Kaspa's actual historical cap (`MAX_SOMPI = 29_000_000_000 * SOMPI_PER_KASPA`,
  also used as the tx-output/tx-total sanity-bound constant) instead of ours — updated to
  `210_000_000 * SOMPI_PER_KASPA`, confirmed via the same RPC call afterward
  (`Max supply (petals): 21000000000000000`, exactly 210M MAGLD). Node and miner stopped
  cleanly; full `cargo build --workspace` clean. Full log in
  [NOTES.md](docs/x-fork/NOTES.md).
  ✅ *Verify:* observed issuance per block (15,228,085 petals) exactly matches the table's
  month-0 value — no red-block/merge deviation to account for on this single-miner run.

---

## Phase 4 — MILESTONE: transparent chain running end-to-end

*Goal: "the fork works." A multi-node private testnet of your renamed, re-parameterized,
capped-supply chain, with wallet and miner. This is the moment the project is demo-able and
the natural checkpoint before the hard part.*

- [x] **P4.1 — Testnet-in-a-box script.** Executed 2026-08-15.
  Wrote [scripts/x-testnet-local.sh](scripts/x-testnet-local.sh) and
  [scripts/x-testnet-local.ps1](scripts/x-testnet-local.ps1). Both launch 3 devnet nodes on one
  machine with distinct appdirs/ports (node1 on the P2.2 defaults — gRPC 26610, P2P 26611;
  node2/node3 shifted to 26620/26621 and 26630/26631, both `--addpeer`'d to node1), auto-build
  `kaspad` if the release binary is missing (so a clean checkout works unmodified), wait and
  report per-node peering status, then print RPC endpoints, log paths, mining instructions (via
  `rothschild --network devnet` for a real keypair + `kaspa-miner`), and the stop command. Data
  dir defaults to repo-relative `x-testnet-local-data/` (gitignored), reusable across restarts.
  ✅ *Verify:* ran the bash script from a clean state (`rm -rf x-testnet-local-data` first) —
  all 3 nodes peered on the first try. Mined 12 blocks against node1 with `kaspa-miner`; all 12
  relayed to both node2 and node3 (`grep -c "via relay"` = 12 on each). Cross-checked over gRPC
  (throwaway-edit-then-revert on `rpc/grpc/examples/simple_client`, same pattern as P2.9/P3.3):
  all three nodes report identical block count (64), virtual DAA score (64), and sink hash.
  Stopped cleanly, throwaway edit reverted, test data dir removed.

- [x] **P4.2 — Full user-journey test.** Executed 2026-08-15.
  Wrote [docs/x-fork/SMOKE.md](docs/x-fork/SMOKE.md) — 8-step manual script (launch testnet →
  create wallet A → mine to it → wait maturity → check balance → create wallet B → send A→B →
  restart node → confirm balances persist), using `rothschild` for keypair generation/sending
  (`kaspa-cli` remains REPL-only, per P0.3) on the P4.1 local testnet. Found and fixed two real
  bugs while walking it: (1) `rothschild` needed rebuilding — a stale pre-P2.1 binary silently
  printed `kaspadev:` addresses instead of `marigolddev:`, the same stale-binary trap as P2.3's
  `kaspad` gotcha, just hitting a different tool this time. (2) A genuine, non-obvious
  compatibility bug: `rothschild`'s hardcoded `DEFAULT_SEND_AMOUNT` (originally 10 KAS-equivalent)
  could never be satisfied from Marigold's own coinbase UTXOs — `select_utxos()`'s
  `MAX_UTXOS = 8` input cap means 8 combined genesis-era coinbase outputs (15,228,085 petals
  each) sum to only ~1.2 MAGLD, so every send silently failed with "Has not enough funds"
  regardless of mining duration (a hard cap, not a timing issue). Fixed by lowering it to 1
  MAGLD-equivalent, the same proportional margin Kaspa's original constant had. Full root-cause
  writeup in [NOTES.md](docs/x-fork/NOTES.md).
  ✅ *Verify:* walked every SMOKE.md step successfully on the local testnet — wallet A mined and
  matured (65,847,603,983 petals), wallet B funded via a real send (3,485,867,022 petals),
  node1 stopped and restarted with the same `--appdir`, both balances (and the virtual DAA
  score) matched exactly pre- and post-restart.

- [x] **P4.3 — Integration test suite green.** Executed 2026-08-15.
  Installed `cargo-nextest` (not present; P0.2 had substituted plain `cargo test` for it back
  then — now installed per coder's suggestion, for this step and future ones). No test
  literals needed fixing this time: every hardcoded-Kaspa-params/genesis issue this crate had
  was already caught and fixed by prior steps this session (P2.6's block-version literal, P3.2's
  subsidy literal and the 5 `goref_*` real-history tests, correctly `#[ignore]`d rather than
  "fixed" since real historical Kaspa chain data can never validate against Marigold's own
  economics). Confirmed clean with both the plain `cargo test --release -p
  kaspa-testing-integration --lib` and the plan's own specified `cargo nextest run --release -p
  kaspa-testing-integration` (benchmark modules are `devnet-prealloc`-feature-gated off by
  default and separately `#[ignore = "bmk"]`d — out of scope for "suite passes," same as any
  other explicitly-marked benchmark).
  ✅ *Verify:* `cargo nextest run --release -p kaspa-testing-integration` — 42 tests run, 42
  passed, 6 skipped (5 `goref_*` + 1 pre-existing manual-only test), 0 failed. Full `cargo build
  --workspace` also clean.

- [x] **P4.4 — Tag it.** Executed 2026-08-15.
  Created an annotated tag `fork-transparent-v0.1` at
  [d4fe5319](https://github.com/marigoldcash/marigold-node/commit/d4fe5319) (P4.3's commit),
  summarizing the whole Phase 4 milestone in its message, and pushed it to `origin`.
  ✅ *Verify:* tag exists on `origin` (`git push origin fork-transparent-v0.1` — confirmed new
  ref). Fresh-clone reproduction actually tested, not assumed: `git clone --branch
  fork-transparent-v0.1` into a scratch directory with zero pre-existing build artifacts, then
  ran `scripts/x-testnet-local.sh` unmodified from that clone — it auto-built `kaspad` from
  scratch (~10 minutes, no cached dependencies) and all 3 nodes peered immediately on the first
  try. Cleaned up (stopped nodes, removed the scratch clone) after confirming.
  **This closes Phase 4 — the fork works, end to end, verified live.**

---

## Phase 5 — Pool: specification ⚠️

*Goal: a complete written spec of the note pool **before implementation**. Writing spec
first is what makes the implementation steps bite sized. The spec lives in
`docs/x-fork/POOL-SPEC.md` and each step below adds a section to it.*

**The design being specified (fix in mind before starting):**
A **note** is `(d, pk, sn)` — denomination tag `d` (from your P1.6 set), current owner
key's public part `pk`, serial number `sn`. The pool is a **plaintext, consensus-maintained
map** `sn → (d, pk)` that every node holds. Nothing is encrypted and nothing is proven in
zero knowledge — ownership is simply "whoever can sign with the private key matching the
note's current `pk`". There are five operations: **mint** (deposit transparent coins →
notes), **rotate** (transfer: replace a note's `pk`), **split** (one note → 10 notes of
1/10th denomination), **merge** (10 identical notes → one note of 10× denomination), and
**redeem** (notes → transparent coins). Study whether `sn` is actually needed as a separate
field or whether a note can be identified another way — note that `pk` cannot serve as the
identifier because it changes on every rotation, so some stable id is required; the spec
should settle its size and how it is assigned (e.g. hash of the minting tx + output index).

- [x] **P5.1 — Spec: data structures.** Executed 2026-08-15.
  Wrote [docs/x-fork/POOL-SPEC.md](docs/x-fork/POOL-SPEC.md)'s P5.1 section. `Note = (d: 1
  byte, pk: 32 bytes, sn: 32 bytes)`, 65 bytes total. `d` is a `u8` index into a
  consensus-defined denomination table (not a raw amount — headroom for future tiers
  without widening the struct). `pk` reuses Kaspa's exact x-only BIP340 Schnorr pubkey
  format (`Version::PubKey`, `crypto/addresses`), so signing/verification code is directly
  reused, not reinvented. `sn = H(creating_tx_id || output_index)` — answers the plan's
  explicit "is `sn` needed" question: yes, since `pk` isn't stable across rotation and
  (per P5.6) isn't even required to be unique, so it can't double as the map key; `sn`'s
  value mirrors how Kaspa already treats `(tx_id, output_index)` as a unique UTXO handle,
  collapsed into one hash for the SMT's key type. Pool state map: `sn → H(d || pk)` via
  the existing `crypto/smt` sparse Merkle tree — the same crate already proven in
  production for the seq-commit/KIP-21 feature, confirmed via real file/line citations
  (`consensus/smt-store`, `compute_root_update`). Pool commitment: a new dedicated 32-byte
  `Header` field (not reusing `accepted_id_merkle_root`, which seq-commit already
  overloads — one field, one meaning), explicitly flagged as a hard-fork-requiring
  addition for Phase 6. Two new domain-separated hash functions specified
  (`NotePoolSerialHash`, `NotePoolLeafHash`), matching the exact existing
  `crypto/hashes` macro convention. Verified every cited file path/line/constant against
  the actual source before writing it down, not from memory.
  ✅ *Verify:* every field has an exact byte size (stated compactly at the section's end);
  every claim about existing code (`crypto/smt`, `crypto/addresses`, `crypto/hashes`,
  `consensus/smt-store`) checked against real source in this pass, not assumed.

- [x] **P5.2 — Spec: transaction format.** Executed 2026-08-15.
  Wrote [docs/x-fork/POOL-SPEC.md](docs/x-fork/POOL-SPEC.md)'s P5.2 section. Subnetwork:
  a dedicated user-lane namespace (`SubnetworkId::from_namespace`) — the actively-used
  mechanism backing Toccata's "user lanes" feature — not the essentially-unused
  `RegistrySubnetwork` path (confirmed via grep: only test-fixture usages exist). Payload:
  `PoolOp` enum, borsh-encoded. **Unified rotate/split/merge into one `TransferOp`**
  wire shape (consumed notes, produced notes, one conservation check) rather than three —
  the plan's own "split/merge is a transfer with a different multiset" framing taken
  literally; "rotate"/"split"/"merge" become descriptive labels for a `Transfer`'s
  multiset shape, not distinct formats. Designed a new domain-separated
  `NotePoolTransferSigningHash` (no existing Kaspa sighash applies, since notes have no
  transparent script to spend) with a `FreshnessAnchor` (recommended 36,000 DAA-score
  window ≈1 hour, reasoned explicitly, not left implicit) replacing tx-ID binding (which
  would be circular) as the anti-replay mechanism. **Fee-stamp mechanics need no new wire
  concept**: a stamp is just a consumed serial with no matching produced note, and the
  conservation-rule difference automatically becomes fee — this single mechanism covers
  every P1.8 bootstrap case (pure rotate needing an attached stamp, self-funding
  split/merge, bundled handover stamps, mint-produced stamps) without a special-cased
  data type. **Found and flagged a real consensus-rule dependency**: checked
  `check_transaction_inputs_count` directly and confirmed it currently rejects any
  non-coinbase transaction with zero inputs — a pure `Transfer`/`Redeem` needs an explicit
  exception (mirroring the existing coinbase one), flagged for P5.3 to formalize, not
  silently assumed to already work. Worked byte-size table for every op shape, arithmetic
  independently checked with a calculator (caught and fixed one addition error before
  finalizing): tens of bytes to ~1.4 KB, comfortably under the "few KB" target and well
  under 1% of block mass limits.
  ✅ *Verify:* all five ops covered (Mint, Transfer's three descriptive shapes, Redeem)
  with worked byte-size estimates, confirmed against `params.rs`'s actual mass constants;
  fee-stamp mechanics explicitly specified for every named bootstrap case.

- [x] **P5.3 — Spec: consensus rules.** Executed 2026-08-15.
  Wrote [docs/x-fork/POOL-SPEC.md](docs/x-fork/POOL-SPEC.md)'s P5.3 section. Read the real
  existing UTXO double-spend/mergeset-conflict mechanism directly
  (`consensus/src/pipeline/virtual_processor/utxo_validation.rs::calculate_utxo_state`,
  `consensus_ordered_mergeset_without_selected_parent`) rather than assuming how it works:
  blocks in a GHOSTDAG mergeset are validated in blue-topological order against a
  **composed view** accumulated from already-processed blocks earlier in that order — a
  conflicting later transaction simply fails and is excluded from acceptance, no special
  rule invoked, the block itself isn't rejected. Specced a `PoolDiff` as the pool's exact
  analog of the existing `UtxoDiff`/`mergeset_diff`, accumulated the same way, in the same
  pass — meaning **the parallel-blocks double-rotate case needed no new rule at all**,
  just this existing, already-proven mechanism extended to a second kind of state. Wrote
  the exact per-op validation order (existence → signature → freshness → denomination
  validity → conservation, in that order) for all three payload variants (Mint, Transfer,
  Redeem). Mass/fee costing defined from existing cost-model constants (payload bytes
  already cost `mass_per_tx_byte` automatically; one sigop-equivalent charged per
  *signature*, not per serial — making batch sweeps cheap by design, not incidentally).
  ✅ *Verify:* every checklist question answered explicitly; the parallel-blocks case is
  resolved by extending the real, cited existing mechanism rather than inventing one;
  signature replay protection ties directly to P5.2's freshness anchor via a concrete
  consensus check (reject anchors outside `[0, 36000]` DAA-score-units of the block, in
  both directions).

- [ ] **P5.4 — Spec: pool state sync & pruning interaction.** Kaspa nodes prune old
  blocks/UTXO history; the pool map is *current state* (like the UTXO set) and must survive
  pruning. Spec how a fresh node syncing from a pruning point downloads the pool state and
  verifies it against the P5.1 commitment in the pruning-point header — mirror how the
  UTXO set is synced and verified against `utxo_commitment` today.
  ✅ *Verify:* section describes the sync-from-pruning-point flow for a fresh node,
  including what is downloaded and which committed hash checks it.

- [ ] **P5.5 — Spec: transfer modes.** **DECIDED: both modes are supported, first-class.**
  The chain-side rotate op is identical for both — the modes are pure wallet-level flows.
  Neither involves identities: only notes have keys, and a "fresh pk" is a new random
  note-keypair generated on the spot. The universal settlement rule (state it once, applies
  to both): *a note is finally yours when a rotation to a key only you know is confirmed
  on-chain*. A dishonest sender can attempt a conflicting rotation before confirmation in
  either mode (classic double-spend, resolved by the P5.3 first-accepted rule) — waiting
  for confirmation is what settles it, and at 10 BPS that wait is seconds.
  **(a) Bearer key-handover** — sender reveals the note's private key (QR, printed paper
  note, offline handover); receiver can be completely passive at transfer time — the
  cash-like property (granny pays with a QR secret key printed on paper). Receiver redeems
  by rotating; until then the note is not finally theirs (sender still knows the key), so
  point-of-sale use means rotate-and-wait-confirmation before handing over goods.
  **(b) Sign-to-fresh-pk** — receiver hands the sender a fresh pk (e.g. merchant displays
  a QR; customer scans and rotates exact notes to it — avoids the merchant scanning a
  customer's broken phone screen); the private key is never in two hands. A handed-out pk
  is inert — if the sender never broadcasts, the receiver has lost nothing; the wallet
  just watches the pk (unpaid-invoice semantics). Note the P5.2 replay-protection anchor
  (signature covers a recent DAA score) gives signed rotations a shelf life — choose the
  freshness window deliberately, as it doubles as invoice expiry and constrains any
  "sign now, broadcast later" pattern.
  ✅ *Verify:* decision recorded in DECISIONS.md; spec states the settlement/finality rule
  and per-mode flows unambiguously, including the shared-key window of bearer mode and
  the pk-freshness window of sign-to-fresh-pk.

- [ ] **P5.6 — Spec: wallet protocol.** The wallet is a key-database manager, not an
  identity: no 24-word seed tied to one key — it holds one private key per note. Spec: the
  key DB format (and its backup story — losing the DB is losing the notes; state this
  loudly), **paper backup**: the wallet can print its key DB as password-protected QR
  code(s) — serialize `(serial, sk, denomination)` entries, encrypt with Argon2id +
  authenticated encryption (reuse the workspace's existing `argon2`/`chacha20poly1305`
  deps), chunk into numbered QRs with a per-page header (backup id, chunk i/N, format
  version) so multi-page restores detect missing pages (~40 note entries fit per QR);
  the password may be written on the printed page for the safe-storage threat model —
  encryption still protects stray copies (photos, printer spools). Restore is
  self-reconciling against the plaintext pool: for each entry, if the serial's current pk
  matches the printed key the note is still yours (now a hot key → rotate immediately),
  otherwise discard. Two properties to state in the spec: **rotation doubles as backup
  revocation** (a leaked printout only endangers notes not rotated since printing; a
  full self-sweep deliberately invalidates all prior backups) and **backups go stale**
  (notes received after printing are not covered — the wallet should prompt periodic
  re-printing). Also spec the receive flow (import keys or receive rotation per P5.5 mode → verify
  on-chain state → rotate if bearer mode → confirm), the spend flow (given an amount,
  select notes, split as needed to make exact change, then transfer), the QR payload
  format(s), and how the wallet tracks its notes (it knows its serials — it just watches
  the chain for ops touching them; no scanning/trial-decryption exists in this design).
  **Shared-pk policy (decided):** consensus does not require pk uniqueness — many notes
  may share one pk, each stays an independent `sn → (d, pk)` entry, and each remains
  separately spendable via signed rotation with no re-rotation needed (a rotation op
  should be allowed to cover multiple serials under one signature, so merchants can sweep
  cheaply). The one wallet invariant: **bearer handover requires a solo key** — revealing
  a shared sk would hand over every note under that pk, so a note must be isolated onto
  its own fresh key (one rotation) before it can be bearer-spent. Personal wallets should
  default to per-note fresh pks in payment QRs.
  **POS "landing pad" flow (decided):** the register encodes `{pk, amount}` into a
  dynamic QR (fresh pk per checkout when dynamically generated — gives free payment
  matching; a single day-pk only as fallback for static printed QRs, customer enters the
  amount manually). The customer wallet displays the amount for confirmation, selects and
  splits notes to the exact sum, and rotates them all to the QR's pk. The merchant wallet
  watches the pk and, on confirmation, **immediately sweeps**: one multi-serial rotation
  (one signature, since all landed notes share the pk) moving each note to its own fresh
  cold key. The receiving pk is thus only a transient landing pad — steady state is
  always one-note-one-key, and the shared-key window lasts seconds. Sweep per
  confirmation, not end-of-day: notes parked on the POS pk are exposed to a compromised
  register device. (Requires P5.2's rotate op to be a list of `(serial, new_pk)` pairs
  under one signature over serials sharing the old key.)
  **Same-key-in-two-wallets hazard:** shared-pk notes can end up split across wallets that
  don't know of each other (partial key export between own devices, restored old backups,
  bearer handover of a non-solo key) — and since the pool is plaintext, anyone holding a
  pk can enumerate every serial under it, so either wallet can spend (or accidentally
  sweep) the other's notes. Two wallet rules make this state harmless and transient:
  (1) **ownership is tracked by serial, never inferred by pk** — spend/sweep selection
  operates only on the wallet's explicit serial list, pk-scanning is for audit only;
  (2) **key provenance decides laziness** — a key generated locally and never exported is
  "cold" (lazy isolation OK); any key that ever crossed a wallet boundary (bearer import,
  device export, backup restore) is "hot" and all of the wallet's serials under it are
  rotated to fresh cold keys immediately, not lazily.
  Include the network-signaled key-rotation upgrade story from the architecture paragraph:
  how a wallet learns "algorithm X is deprecated, rotate to algorithm Y keys".
  ✅ *Verify:* section lets a wallet dev implement receive-detect-spend without asking
  questions.

- [ ] **P5.7 — Spec: honest privacy statement.** One section stating exactly what is and
  is not private: denominations, serials, and every op are public; no sender/receiver
  addresses exist in the pool, but **mint and redeem edges link transparent coins to
  specific notes** (this is where real-world deanonymization happens),
  and timing/denomination patterns of rotate/split/merge are visible graph structure. The
  anonymity set of a note is roughly "all notes of the same denomination". 
  *(Flag from P1.8: fee stamps put no transparent address on pool ops — mint/redeem
  remain the only transparent touchpoints — but a stamp's lineage is public like any
  note's, so ops sharing stamp ancestry are linkable within the note graph. Same class
  as the disclosed rotate/split/merge graph visibility, not a new category, but name
  it explicitly here; P5.6 wallet hygiene mitigates.)*
  ✅ *Verify:* section exists and makes no claim stronger than the design delivers.

- [ ] **P5.8 — Spec: launch finality anchors.** ⚠️ **DECIDED: the fork launches with a
  federated finality guard.** Rationale: a young PoW network can be 51%-attacked by any
  sliver of Kaspa's ASIC fleet; a veto-only, sunsetting trustee quorum is strictly less
  centralized than one anonymous farm holding 99.9% of hashrate. Precedents: early
  Bitcoin checkpoints, Peercoin/Feathercoin checkpointing, Komodo dPoW, Decred's hybrid.
  Mechanism to spec: a k-of-n trustee quorum (recommend 3-of-5, independent orgs/geos)
  signs **rolling finality anchors** — the hash of a block already ~1 minute deep, every
  30–60s — published as tiny transactions in a designated subnetwork *and* gossiped over
  P2P. Consensus rule: a chain conflicting with the latest valid anchor is invalid
  regardless of accumulated work (a second, faster trigger for the existing
  finality-depth reorg refusal). Trustees produce no blocks, earn nothing, and can only
  veto reorgs of ~minute-old history — they ratify history, not transaction selection.
  The spec must cover: anchor tx format + quorum verification; cadence and depth choices;
  **fail-open liveness** (no anchors → plain PoW security + loud alerts, never a halt);
  equivocation proofs (a quorum signing conflicting anchors) and permanent key
  disqualification; anchor-aware IBD (a fresh node must learn the latest anchor before
  trusting any chain — an anchor-free attacker chain must lose); and the **hard-coded
  sunset**. The sunset exploits the fact that the chain can measure its own security:
  difficulty is on-chain, objective, and proportional to honest hashrate. Retirement
  trigger: sustained difficulty ≥ threshold **T** for **M** months **AND** at least
  **K** years elapsed — both conditions required, because a patient attacker could mine
  honestly to inflate difficulty, trip a difficulty-only threshold early, then attack
  the newly unprotected chain; the time floor blunts that, and the hysteresis (the
  threshold must hold for months, not moments) blunts it further. "Sustained" must be
  specified as an exact deterministic function of on-chain data (e.g. the sampled
  difficulty-window median staying above T across a defined DAA-score span) so every
  node computes the identical retirement state — a fuzzy definition is a chain-split
  bug. Decay is **gradual, not cliff-edge**: the anchor cadence stretches in
  `ForkActivation`-staged steps as conditions are met (e.g. 30s → hourly → daily →
  weekly → never), so the protection thins as it exits — with fail-open alerting
  thresholds scaling alongside — ending with anchors advisory-only, then trustee keys
  consensus-expired at a hard maximum DAA score regardless of network growth (trust
  must end even if growth disappoints). Extending trustee life must require an explicit
  hard fork. Note the naming collision to avoid in all documents: the P5.2 *freshness
  anchor* (replay protection) is unrelated to these *finality anchors*.
  ✅ *Verify:* section states exact values for k, n, cadence, depth, T, M, K, the
  cadence-decay schedule, and the hard maximum DAA score, and answers every attack case:
  k-key compromise, equivocation, trustee DoS, difficulty-inflation-then-attack, and an
  anchor-free chain offered to a syncing node.

- [ ] **P5.9 — Spec review gate.** 🧑‍⚖️ Freeze the spec (tag `pool-spec-v1`), then have
  it reviewed by at least one person with applied-cryptography background **outside the
  project**. Fold feedback into v1.1. Do not start Phase 6 before this.
  ✅ *Verify:* written review exists in `docs/x-fork/reviews/`; issues triaged.

---

## Phase 6 — Pool: consensus implementation ⚠️

*Goal: the P5 spec is enforced by every node. Steps follow the data path: types → state
store → validation → pipeline → mempool → sync → RPC. Every step lands with its tests
(Ground rule 3). Do not start before the P5.9 gate.*

- [ ] **P6.1 — Note types & wire encoding.** In `consensus/core`: `Note`, the op enum
  (Mint/Rotate/Split/Merge/Redeem — rotate as a list of `(serial, new_pk)` pairs under one
  signature per P5.2/P5.6), borsh serialization, and a dedicated subnetwork ID in
  [consensus/core/src/subnets.rs](consensus/core/src/subnets.rs). Pure data — no validation
  logic yet.
  ✅ *Verify:* round-trip encode/decode unit tests pass for every op, including maximum-size
  instances; byte sizes match the P5.1/P5.2 spec tables.

- [ ] **P6.2 — Pool state store + commitment.** A RocksDB-backed store (follow the store
  patterns in `consensus/src/model/stores/`) holding `sn → (d, pk)`, with an SMT root over
  it (reuse [crypto/smt](crypto/smt/src/lib.rs)) as the pool commitment, and a `PoolDiff`
  type (mutations + their inverses) so state can be applied and un-applied per chain block
  — same discipline as `UtxoDiff`.
  ✅ *Verify:* store unit tests: apply/unapply round-trips restore the exact prior root;
  commitment is deterministic across insertion orders.

- [ ] **P6.3 — Stateless op validation.** Parse-and-check without any state: payload
  decodes, signature well-formed, denominations from the P1.6 set, split/merge multiset
  arithmetic balances, size limits, freshness-anchor field present. Fail-fast and
  fuzz-friendly (this parser is fuzzed in P8.1).
  ✅ *Verify:* table-driven tests — every malformed-op class from the P5.3 checklist is
  rejected with a distinct error.

- [ ] **P6.4 — Stateful validation in the virtual pipeline.** ⚠️ **HARD.** Wire pool ops
  into [consensus/src/pipeline/virtual_processor](consensus/src/pipeline/virtual_processor/processor.rs)
  (model: how `utxo_validation.rs` resolves UTXO double-spends at the accepted-transaction
  level): serial exists / doesn't (mint), signature verifies against current pk, freshness
  anchor within window, first-accepted-wins for conflicting ops on the same serial in
  merged blocks, and `PoolDiff` application/rollback on virtual-chain changes (reorgs).
  ✅ *Verify:* consensus tests: parallel-block double-rotate resolves deterministically;
  a reorg past a pool op restores the prior pool root; stale-anchor ops rejected.

- [ ] **P6.5 — Commitment placement.** 🧑‍⚖️ **DECISION + implementation.** Where does the
  pool root live: a new header field beside `utxo_commitment` (clean; changes header
  serialization/hashing — fine for a fresh network but touches mining/stratum code and
  requires redoing the P2.5 genesis hashes) or inside the coinbase payload (no header
  change; weaker ergonomics). Recommend the header field. Implement, update the in-repo
  [bridge](bridge/) for the new header, regenerate genesis constants.
  ✅ *Verify:* `cargo test -p kaspa-consensus-core` (genesis + header tests) passes; a
  mined block on local devnet carries the correct pool root.

- [ ] **P6.6 — Mint/redeem value binding.** Mint consumes ordinary transparent outputs
  summing exactly to the notes created; redeem mints transparent outputs from destroyed
  notes. Assign mass costs per op per P5.3 (rotate ≈ 1-input tx; split priced so
  note-inflation spam is uneconomical — final numbers calibrated in P8.3).
  ✅ *Verify:* value-conservation test: after arbitrary op sequences,
  `Σ pool notes + transparent supply == emitted supply`; mint/redeem with wrong sums
  rejected.

- [ ] **P6.7 — Mempool integration.** In [mining/src/mempool](mining/): accept pool-op
  transactions, standardness checks, same-serial conflict policy (first-seen holds, second
  rejected), eviction on confirmation, and inclusion in block templates.
  ✅ *Verify:* mempool tests: conflicting rotate arriving second is rejected; template
  built under load includes pool ops and validates.

- [ ] **P6.8 — Pool state sync (IBD).** New nodes syncing from a pruning point download
  the pool state and verify it against the committed root — mirror
  [request_pruning_point_utxo_set.rs](protocol/flows/src/v7/request_pruning_point_utxo_set.rs)
  (chunked download, hash-verified; `crypto/smt`'s streaming module exists for exactly
  this). Add the messages to the current protocol version's flow registration.
  ✅ *Verify:* integration test: fresh node syncs from a node whose pool is non-empty,
  ends with identical pool root; tampered chunk is rejected.

- [ ] **P6.9 — RPC + notifications.** Add RPC methods: get note(s) by serial, pool stats
  (count per denomination); add a `NotesChanged`-style subscription (model:
  `UtxosChanged` through the [notify](notify/) system) scoped to watched serials/pks —
  this is what wallets poll-free receive/sweep flows (P7) depend on. Wire through
  rpc/core, grpc (proto files), and wrpc.
  ✅ *Verify:* `kaspa-cli`-level manual check: subscribe to a serial, rotate it, receive
  the notification; grpc + wrpc both serve the new methods.

- [ ] **P6.10 — Consensus test battery + simpa.** A dedicated integration-test module
  running the full matrix: all five ops happy-path, every P5.3 rejection case, parallel
  conflicts, deep reorg, value conservation, pool-root agreement across nodes. Teach
  `simpa` to generate random pool ops so DAG-level stress includes the pool.
  ✅ *Verify:* `cargo nextest run --release -p kaspa-testing-integration` green including
  new module; simpa run with pool ops completes with all nodes agreeing on the pool root.

- [ ] **P6.11 — Finality-anchor consensus rule.** ⚠️ **HARD.** Implement per P5.8: anchor
  tx parsing and k-of-n signature verification against trustee pubkeys pinned in
  [params.rs](consensus/core/src/config/params.rs); the fork-choice override in the
  virtual processor (a chain conflicting with the latest valid anchor is invalid
  regardless of work — wire it as a second trigger for the existing finality-depth reorg
  refusal); equivocation-proof handling with permanent key disqualification; fail-open
  behavior when anchors are absent; and the `ForkActivation`-staged sunset
  (mandatory → advisory → keys consensus-expired).
  ✅ *Verify:* consensus tests: a heavier attacker chain lacking the latest anchor loses
  to the anchored chain; an equivocating quorum is ignored after its proof is processed;
  anchors after the sunset score are rejected; anchor-free operation degrades to plain
  PoW without halting.

- [ ] **P6.12 — Anchor distribution + trustee signer.** Gossip anchors over P2P and make
  IBD anchor-aware (a syncing node requests the latest anchor before committing to a
  chain, so an anchor-free attacker chain cannot capture fresh nodes); build the trustee
  signer daemon: a small tool that watches its own node, signs the depth-D block on the
  P5.8 cadence, aggregates k-of-n partial signatures, and submits the anchor tx.
  ✅ *Verify:* integration test: a fresh node offered only an attacker chain refuses it
  once it learns the latest anchor; a 3-of-5 signer setup on the local testnet produces
  anchors continuously and all nodes report finality within one cadence interval.

---

## Phase 7 — Wallet integration

*Goal: a person can hold, receive, spend, back up, and restore notes through the CLI
wallet. WASM/mobile wallets are post-launch — CLI proves the protocol.*

- [ ] **P7.0 — Inherited-wallet surface audit.** 🧑‍⚖️ **DECISION** (added 2026-08-15,
  prompted by the P2.8 investigation of `compat/gen0.rs`). Decide the fate of the
  entire inherited seed-phrase wallet stack (`kaspa-wallet-core`'s BIP32/mnemonic
  accounts, `kaspa-cli`'s wallet commands) now that the note wallet replaces the
  wallet concept — keep as a transparent-tier power-user tool, feature-gate it, or
  strip it. Whatever the wallet-stack decision, **the legacy-Kaspa import surfaces
  must be removed or hard-disabled**: `compat/gen0.rs` (KDX import — a deprecated
  third-party Kaspa wallet), `compat/gen1.rs` + the four
  `import_kaspawallet_golang_*` wallet-API functions (Go `kaspawallet` files), and
  the CLI's `import legacy` / `account import legacy-data` commands. Rationale: on a
  fair-launch chain these can never find funds — their only possible real-world
  effect is inviting users to type real Kaspa wallet passwords and expose real Kaspa
  keys inside Marigold software (key-reuse hazard, and it normalizes exactly the
  behavior wallet-phishing needs). Deliberately deferred from Phase 2 (the inherited
  wallet is load-bearing for P4.2's smoke test, and piecemeal deletion would buy
  upstream-merge friction without settling the real question). **Hard deadline: must
  close before any binaries reach outside users — P8.7 public testnet at the latest;
  the P8.5 wallet threat pass must re-verify it happened.**
  ✅ *Verify:* decision recorded in DECISIONS.md; `grep -ri "kdx\|kaspawallet\|legacy_v0"
  wallet/ cli/` shows no reachable user-facing import path; `cargo test --workspace`
  green after removal.

- [ ] **P7.1 — Note key DB.** In `wallet/core`: a serial-keyed store of
  `(serial, sk, denomination, provenance)` with the hot/cold provenance flag from P5.6,
  persisted with the wallet's existing encrypted-storage machinery; subscribes to
  `NotesChanged` (P6.9) for its serials.
  ✅ *Verify:* unit tests: DB round-trips; a rotation observed on-chain updates note
  status; hot keys are flagged at import.

- [ ] **P7.2 — Mint & redeem commands.** CLI: `note mint <amount>` (splits into P1.6
  denominations, pays from transparent balance) and `note redeem <serials|amount>`.
  ✅ *Verify:* on local testnet: mint from mined funds, redeem back, transparent balance
  reconciles minus fees.

- [ ] **P7.3 — Receive flows.** (a) bearer import: scan/paste a key QR → verify serial's
  on-chain pk matches → **immediately rotate to a fresh cold key** → report confirmed;
  (b) sign-to-fresh-pk: generate fresh keypair(s), emit payment-request QR, watch for the
  rotation, confirm. Enforce the P5.6 hot-key rule on every import path (incl. restore).
  ✅ *Verify:* both flows succeed on local testnet between two wallet instances; imported
  key is never left unrotated after confirmation.

- [ ] **P7.4 — Spend flows.** Given an amount: note selection + split planning to hit the
  exact sum, then (a) rotate to a supplied payment-request pk, or (b) bearer export —
  which must enforce solo-key: auto-isolate the note first if its key is shared, then
  emit the key QR and mark the note "handed over, pending their rotation".
  ✅ *Verify:* spends of amounts requiring splits succeed; bearer-exporting a shared-key
  note demonstrably isolates first (two txs on-chain).

- [ ] **P7.5 — POS landing-pad mode.** Implement P5.6's flow: merchant side generates
  `{pk, amount}` payment-request QRs (fresh pk per checkout) and auto-sweeps on
  confirmation (one multi-serial rotation to per-note fresh cold keys); payer side scans,
  displays amount for confirmation, pays exact.
  ✅ *Verify:* scripted two-wallet POS demo passes; merchant wallet ends with
  one-note-one-key state within seconds of payment.

- [ ] **P7.6 — Paper backup.** Print/export per P5.6: Argon2id + authenticated encryption,
  chunked QRs with page headers, password optionally printed; restore = decrypt →
  reconcile each serial against the pool → rotate all still-owned notes (hot keys).
  ✅ *Verify:* backup → wipe wallet → restore on local testnet recovers exactly the
  still-owned notes and rotates them; a missing page is detected and reported.

- [ ] **P7.7 — Wallet UX & docs pass.** Consistent CLI command naming, human-readable
  errors for every rejection case, and `docs/x-fork/WALLET.md` walking through every flow
  (mint, pay, receive, POS, bearer, backup, restore).
  ✅ *Verify:* a reader can execute every WALLET.md flow on the local testnet verbatim.

- [ ] **P7.8 — End-to-end smoke extension.** Extend `docs/x-fork/SMOKE.md` (P4.2) with the
  full note lifecycle across the 3-node local testnet, including a node restart mid-flow
  and a wallet restore.
  ✅ *Verify:* every SMOKE.md step passes from a clean checkout via the P4.1 script.

---

## Phase 8 — Hardening ⚠️

*Goal: the code survives adversaries, crashes, spam, and time. Runs partly in parallel
with late Phase 7. The testnet soak (P8.7) is calendar time — start it as early as it can
stand.*

- [ ] **P8.1 — Fuzz the op parser.** `cargo fuzz` targets for the P6.3 stateless parser
  and the paper-backup/QR decoders; run to coverage plateau; fix every panic/OOM.
  ✅ *Verify:* fuzz corpus committed; 24h fuzz run with zero crashes; fuzz job wired into
  CI (short run per PR).

- [ ] **P8.2 — Adversarial consensus tests.** Beyond P6.10 happy-path-adjacent tests:
  freshness-window boundary ops (exactly at/past the edge), replayed rotations after a
  key is re-used, duplicate serials within one block and across a mergeset, signature
  malleability probes, ops referencing pruned freshness anchors. Finality-anchor attacks:
  trustee DoS (verify fail-open + alerting), equivocation split attempts, an attacker
  holding k−1 trustee keys plus majority hashrate, and stale-anchor replay.
  ✅ *Verify:* each attack has a named test that fails on a deliberately-broken validator
  (mutation-check) and passes on real code.

- [ ] **P8.3 — Spam & state-growth economics.** ⚠️ The pool map, like the UTXO set, never
  shrinks *on its own*: merge and redeem do shrink it, but they are voluntary and only the
  key-holder can perform them — an attacker bloating state via splits never merges, and
  lost-key notes occupy their slots forever — so worst-case analysis must assume no
  voluntary cleanup. Split is a 10× note multiplier. Model worst-case pool growth
  under mass pricing from P6.6, calibrate split/mint costs so sustained note-inflation is
  uneconomical, and decide whether the smallest denomination needs a higher floor price.
  Add pool-size metrics to the node's perf monitoring.
  ✅ *Verify:* written analysis in `docs/x-fork/NOTES.md` with the calibrated numbers; a
  simpa spam-scenario run stays within projected growth bounds.

- [ ] **P8.4 — Crash & deep-reorg drills.** Kill nodes mid-sweep/mid-IBD and restart:
  state must recover to a consistent pool root. Simpa-driven deep reorgs crossing
  mint/split/redeem boundaries.
  ✅ *Verify:* scripted kill/restart matrix passes; post-reorg pool roots agree across
  all simulated nodes.

- [ ] **P8.5 — Wallet threat pass.** Review the wallet flows against: payment-request QR
  tampering (amount/pk swapped — does the payer's confirm screen bind what's signed?),
  bearer QR shoulder-surfing/photograph, clipboard scraping, malicious restore files,
  and the hot-key rules under every import path. Fix what falls out.
  *(Flag from P7.0: re-verify here that the legacy-Kaspa import surfaces — KDX/gen0,
  golang-kaspawallet/gen1, `import legacy` CLI flows — were actually removed or
  hard-disabled before any binaries ship to outside users.)*
  ✅ *Verify:* written threat checklist in `docs/x-fork/reviews/` with each item tested
  or explicitly accepted.

- [ ] **P8.6 — Upstream merge drill.** Merge current `upstream/master` into the fork
  branch; measure the conflict surface; document the resolution playbook in NOTES.md
  (this validates Ground rule 1 in practice — if conflicts are ugly, fix the fork's
  divergence style *now*).
  ✅ *Verify:* merge completed, full test suite green afterwards; playbook written.

- [ ] **P8.7 — Public testnet soak.** 🧑‍⚖️ Stand up the public testnet: a few seed nodes,
  a faucet, invite outside testers, run for **months** (Ground rule 4). Keep an incident
  log; every consensus-relevant incident gets a regression test.
  ✅ *Verify:* testnet has run ≥ 3 months with outside participants; incident log exists;
  zero unexplained pool-root divergences.

- [ ] **P8.8 — External code audit gate.** 🧑‍⚖️ Independent security review of the pool
  consensus code, finality-anchor code, and wallet crypto (complements the P5.9 spec
  review). Fix criticals,
  re-review the fixes. Do not schedule mainnet before this closes.
  ✅ *Verify:* audit report in `docs/x-fork/reviews/`; all criticals/highs closed with
  linked commits.

---

## Phase 9 — Launch preparation

*Goal: from "hardened testnet" to "mainnet exists and survives its first month."*

- [ ] **P9.1 — Launch security posture: trustee ceremony & anchor operations.**
  **DECIDED:** the fork keeps kHeavyHash and launches under the P5.8 finality-anchor
  guard (implemented in P6.11–P6.12) — chosen over a PoW change because kHeavyHash has
  no liquid rental market (attacks require owning attributable ASICs, unlike the
  NiceHash-rental fate of small GPU-algo coins), and anchors neutralize the reorg threat
  outright while hashrate grows. Record the full threat analysis in DECISIONS.md. This
  step is the operational half: select the n trustees (genuinely independent orgs, geos,
  jurisdictions — feeds the P9.6 legal review), run a documented key ceremony (HSMs
  recommended; generation, backup, and rotation procedures written down), pin the
  trustee pubkeys + sunset schedule in [params.rs](consensus/core/src/config/params.rs),
  and stand up the k-of-5 signer daemons on independent infrastructure. Residual
  mitigations still apply: a friendly-hashrate floor at launch (own/rented ASICs plus
  committed known miners — anchors protect against reorgs, not censorship or difficulty
  whiplash), keep value-at-stake low early, and anchor-liveness + reorg monitoring
  (P9.8).
  ✅ *Verify:* DECISIONS.md records the analysis; key-ceremony document exists; the
  public testnet runs under real trustee infrastructure (not localhost signers) for its
  final soak period with continuous anchors.

- [ ] **P9.2 — DNS seeders.** Stand up ≥ 2 DNS seeders on independent infrastructure
  (Kaspa's dnsseeder software works unmodified against your network once P2.3's handshake
  name is set), then populate `dns_seeders` in
  [params.rs](consensus/core/src/config/params.rs) for mainnet + testnet (reversing
  P2.4's empty list).
  ✅ *Verify:* a fresh node with no `--addpeer` discovers peers through your seeders on
  both networks.

- [ ] **P9.3 — Release engineering.** Adapt [.github/workflows](.github/workflows/ci.yaml)
  CI + deploy pipelines for the fork: multi-platform binaries (Linux/Windows/macOS),
  checksummed release artifacts, Docker images, and a `stable` branch discipline matching
  upstream's.
  ✅ *Verify:* CI green on the fork repo; a tagged release produces installable binaries
  that run the P4.1 testnet script from scratch.

- [ ] **P9.4 — Ecosystem minimum.** The smallest viable surrounding infrastructure: a
  block explorer (existing Kaspa explorers can be adapted — the RPC surface is nearly
  identical plus P6.9 additions), the testnet faucet, stratum-bridge configuration and
  miner-setup docs for your network.
  ✅ *Verify:* explorer shows blocks + pool stats on testnet; a third party can follow the
  miner docs to mine on testnet unassisted.

- [ ] **P9.5 — Mainnet params freeze & genesis ceremony.** Final pass over every P1
  decision as implemented: emission table re-verified against the cap (P3.2's assertion),
  ports, prefixes, denominations; then regenerate the mainnet genesis with the real launch
  timestamp and motto, and tag `mainnet-rc1`.
  ✅ *Verify:* fresh clone of the tag builds and starts a mainnet node that idles
  correctly (no peers yet); all tests green.

- [ ] **P9.6 — Legal counsel review.** 🧑‍⚖️ Real counsel reviews the P1.9 posture against
  the launch plan: entity/jurisdiction, asset regulations in target markets
  (EU AMLR 2027 horizon), exchange/DEX distribution constraints, and the project's public
  claims (P5.7's honest statement helps here).
  ✅ *Verify:* written legal memo received; launch plan adjusted where required.

- [ ] **P9.7 — Launch runbook & emergency plan.** Write `docs/x-fork/LAUNCH.md`: launch-day
  sequence (seeders live → binaries published → genesis time → first blocks), a go/no-go
  checklist, comms plan, a responsible-disclosure security policy, and — most importantly —
  the **consensus-emergency playbook**: who can ship a hotfix, how a chain-halting bug is
  triaged, how an emergency release reaches node operators, and the **trustee-emergency
  procedures**: suspected key compromise (rotate via the P9.1 ceremony process),
  quorum-loss response (network runs fail-open — degraded, not down), and equivocation
  response.
  ✅ *Verify:* runbook reviewed end-to-end in a dry run against the public testnet
  (simulated emergency included).

- [ ] **P9.8 — Launch & first-month watch.** Execute LAUNCH.md at the P9.5 timestamp.
  Monitoring rota for the first weeks: pool-root agreement across independent nodes,
  **anchor liveness and depth-to-finalization** (an anchor gap means the 51% guard is
  down — page someone), hashrate and reorg depth, mempool health, seeder health. Establish the upstream-tracking
  cadence (periodic P8.6-style merges) as ongoing practice.
  ✅ *Verify:* mainnet running ≥ 1 month with no consensus incidents; first routine
  upstream merge completed post-launch.

---

## Future work (post-launch, deliberately out of launch scope)

**Governance / DAO — DECIDED direction: native note-weighted governance ops** (the
"Route C" pattern already used twice — the note pool and the finality anchors are both
purpose-built consensus mechanisms with enumerable action spaces; governance becomes the
third). Concept: a governance subnetwork with proposal + vote ops; a vote is a signature
by a note's current key over `(proposal, choice)`, weighted by denomination, one vote per
serial per proposal — sybil-resistance is coin-weight, double-vote prevention is serial
uniqueness (machinery that already exists), and no identity is involved: a note votes,
not a person (public cost: a serial is linked to a stance, never to a holder; rotate
after voting). The governed action space stays tiny and enumerable: elect/remove trustee
keys, tune anchor cadence within hard-coded bounds. This upgrades the P5.8 sunset story
from *mandatory → advisory → keys expire* to *founder trustees → **DAO-elected
trustees** → expiry* — privileges don't just decay, they transfer from founders to
holders first. Sequencing: launch under the founder-trustee model; post-launch, run a
"Phase 10" of the same shape as Phase 5 (spec → external review gate → implement →
harden), then adopt via an explicit hard fork — a DAO adoption *should* be a visible
community opt-in event. Rejected alternatives, recorded so they aren't relitigated: an
EVM/WASM global-state VM (fights GHOSTDAG's commutativity-based parallelism, permanent
upstream divergence); building the DAO on the inherited Toccata covenant+ZK substrate
(already active from genesis via P2.6 and remains available as a fallback, but the ZK
toolchain contradicts the project's simplicity ethos).

---

