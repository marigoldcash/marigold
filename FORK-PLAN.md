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

- [ ] **P0.4 — Mine devnet blocks.**
  Use the community CPU miner (github.com/elichai/kaspa-miner or equivalent) pointed at the
  devnet node, or use `simpa` for in-process simulation:
  `cargo run --release --bin simpa -- -t=10 -d=2 -b=8 -n=500`.
  ✅ *Verify:* node log shows accepted blocks / simpa completes and prints DAG stats.

- [ ] **P0.5 — Exercise a wallet on devnet.**
  Run `cargo run --release -p kaspa-cli`, create a wallet, get a devnet address, mine to it,
  send a transaction to a second address.
  ✅ *Verify:* second address shows a balance via the CLI.

- [ ] **P0.6 — Write the orientation notes file.**
  Create `docs/x-fork/NOTES.md` recording: build quirks encountered, the exact miner used,
  commands that worked. Future sessions start by reading it.
  ✅ *Verify:* file exists and includes the working run commands.

---

## Phase 1 — Design decisions 🧑‍⚖️

*Goal: every parameter that code will encode is decided and written down. Each item is a
decision to record in `docs/x-fork/DECISIONS.md` (create it in P1.1). No code changes.*

- [ ] **P1.1 — Create the decisions file** `docs/x-fork/DECISIONS.md` with a table:
  Decision / Choice / Rationale / Date.
  ✅ *Verify:* file exists.

- [ ] **P1.2 — Name & ticker.** Working name "X Coin" collides with everything (and with
  X/Twitter). Pick a real name, a 3–5 letter ticker, and check collisions on CoinGecko/
  CoinMarketCap. Record: name, ticker, address prefix string (lowercase, short, e.g.
  `xcn`), testnet prefix (e.g. `xcntest`).
  **Pre-decided (2026-08-13), record formally when executing this step:** name
  **Marigold** (collision-checked: no coin/CMC listing; known non-coin name-neighbors to
  stay clear of are marigold.dev, a Tezos dev company, and marigold.com, a martech firm).
  Ticker **MGLD** (provisional — verify unclaimed at execution time). Address prefixes
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

- [ ] **P1.3 — Units & precision.** Recommend keeping Kaspa's 8 decimals (1 coin = 10⁸ base
  units, Kaspa's "sompi"). Name your base unit. Record it.
  **Pre-decided (2026-08-13):** base unit **petal** — 1 marigold = 10⁸ petals.

- [ ] **P1.4 — Supply & emission.** Decision has to be made on fixed cap (any round
  cap works), smooth geometric decay like Kaspa's "chromatic" schedule (halving the
  block reward every N months) rather than Bitcoin cliff-halvings — smoother miner economics.
  Choose: **total cap**, **emission duration** (e.g. ~30 years), **initial per-second 
  reward**. Record all three. (Implementation is P3.2.)

- [ ] **P1.5 — Launch allocation.** Fair launch (mine from zero), premine %, or airdrop?
  This is an economics/credibility/legal decision, not code. If premine: how much, vesting,
  and to what governance. Record it. *Note: honesty + large premine is a hard sell;
  fair launch or small transparent dev fund (<10%) is the defensible zone.*

- [ ] **P1.6 — Denominations.** For the coin pool recommend powers of ten in whole
  coins: {0.01, 0.1, 1, 10, 100, 1000}. Record the set.

- [ ] **P1.7 — Transparent tier policy.** The fork launches transparent-only (Phases 2–4)
  and adds the pool (Phases 5–7).

- [ ] **P1.8 — Fee policy.** Keep fees (near-zero like Kaspa). We established zero-fee +
  reward-per-action is a spam machine; this plan keeps Kaspa's fee market untouched. Record
  simply: "inherit Kaspa fee model."

- [ ] **P1.9 — Regulatory posture.** One paragraph: which jurisdictions you'll operate/
  incorporate in and that distribution will depend on CEXs willingness to adopt the coin. 
  Get real legal counsel before mainnet (P9.6).

- [ ] **P1.10 — Network numbers.** Pick non-colliding defaults, record them:
  P2P port, gRPC port, borsh-wRPC port, JSON-wRPC port for mainnet + testnet
  (Kaspa uses 16110/16210-family — pick a different thousand-block, e.g. 26110-family).

---

## Phase 2 — Fork identity (make it a different network)

*Goal: a node that builds from this repo is a **new network**: it will not connect to Kaspa
peers, Kaspa addresses are invalid on it, and it has its own genesis. Everything here is
small, mechanical, and individually testable.*

- [ ] **P2.1 — Address prefix.**
  Edit [crypto/addresses/src/lib.rs](crypto/addresses/src/lib.rs) — the `Prefix` enum's
  string mappings (`"kaspa"`, `"kaspatest"`, `"kaspasim"`, `"kaspadev"` → your P1.2
  prefixes) in all three places (serde rename, `Display`, `FromStr`) plus
  [crypto/addresses/src/wasm.rs](crypto/addresses/src/wasm.rs). Fix the address test vectors
  in the same file (tests encode prefix strings; recompute expected bech32 outputs by
  running the tests and updating from failures — the checksum covers the prefix, so all
  vectors change).
  ✅ *Verify:* `cargo test -p kaspa-addresses` passes; a generated address starts with your
  prefix.

- [ ] **P2.2 — Network ports.**
  Edit [consensus/core/src/network.rs](consensus/core/src/network.rs):
  `default_rpc_port`, `default_borsh_rpc_port`, `default_json_rpc_port`, `default_p2p_port`
  → your P1.10 numbers.
  ✅ *Verify:* `cargo test -p kaspa-consensus-core network` passes; started node logs show
  new ports.

- [ ] **P2.3 — P2P network isolation.**
  The P2P handshake exchanges a network name derived from `NetworkId` (e.g.
  `kaspa-mainnet`). Grep `protocol/p2p` and `consensus/core/src/network.rs` for how
  `network_name()` / `to_prefixed()` feed the version handshake; change the base string so
  the handshake name becomes e.g. `xcn-mainnet`.
  ✅ *Verify:* unit tests pass **and** an integration check: start your node, attempt to
  connect it to a public Kaspa node (`--addpeer`), confirm the log shows a
  network-mismatch rejection.

- [ ] **P2.4 — Strip Kaspa's DNS seeders.**
  In [consensus/core/src/config/params.rs](consensus/core/src/config/params.rs), set
  `dns_seeders: &[]` for MAINNET_PARAMS and TESTNET_PARAMS (you'll add your own in P9.2).
  ✅ *Verify:* `cargo build` passes; node starts and logs no Kaspa seeder lookups.

- [ ] **P2.5 — New genesis blocks (mainnet + testnet).**
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

- [ ] **P2.6 — Reset fork activations.**
  In [params.rs](consensus/core/src/config/params.rs) set for your mainnet/testnet:
  `crescendo_activation: ForkActivation::always()` and
  `toccata_activation: ForkActivation::always()` — a new chain starts with all upgrades
  active from block 0 (no history to protect). This gives you 10 BPS + covenants + ZK
  opcodes from genesis and deletes an entire class of fork-transition complexity.
  ✅ *Verify:* `cargo test -p kaspa-consensus` passes;
  `grep -n "activation" consensus/core/src/config/params.rs` shows always() for your nets.

- [ ] **P2.7 — User-facing rebrand pass 1 (node).**
  Grep `kaspad/src`, `core/src`, `daemon/src` for user-visible strings: application name,
  log banner, `--help` text, default app-dir name (so your node's data dir is
  `~/.xcn` -equivalent, not `~/.rusty-kaspa` — grep `app_dir`/`get_app_dir`). Change
  display strings only — not crate names, not module paths (Ground rule 1).
  ✅ *Verify:* `cargo run --release --bin kaspad -- --help` shows your name; a fresh run
  creates your app dir.

- [ ] **P2.8 — User-facing rebrand pass 2 (wallet + CLI).**
  Grep `wallet/` and `cli/` for `"KAS"`, `"kaspa"` in display strings, ticker formatting,
  and URLs. Same rule: display strings only.
  ✅ *Verify:* `cargo run --release -p kaspa-cli` shows your ticker in balances.

- [ ] **P2.9 — Two-node private network smoke test.**
  Start two local nodes with `--addpeer` pointing at each other (different appdirs/ports via
  flags), mine on one.
  ✅ *Verify:* second node's log shows it syncing blocks mined by the first; both report the
  same virtual DAA score via RPC.

---

## Phase 3 — Economics (emission with a hard cap)

*Goal: your P1.4 supply schedule is enforced by consensus and provably sums below the cap.*

- [ ] **P3.1 — Understand the existing emission.** Read
  [consensus/src/processes/coinbase.rs](consensus/src/processes/coinbase.rs): subsidy comes
  from `SUBSIDY_BY_MONTH_TABLE` (426 monthly entries, divided by BPS so per-block reward
  scales with block rate) after `deflationary_phase_daa_score`, and a flat
  `pre_deflationary_phase_base_subsidy` before it. Write a summary paragraph into
  `docs/x-fork/NOTES.md`.
  ✅ *Verify:* summary exists and correctly states where the table is consumed.

- [ ] **P3.2 — Generate your subsidy table.**
  Write a small generator (a `#[test]` or `examples/gen_emission.rs` in
  `consensus/core`) that, from your P1.4 choices (cap, duration, decay), computes a monthly
  table (same 426-slot shape or your own length — if length changes, update
  `SUBSIDY_BY_MONTH_TABLE_SIZE` and all its uses), **asserts total emission
  (pre-deflationary + Σ table × seconds-per-month) ≤ cap**, and prints the table as Rust
  code. Paste the output into `coinbase.rs`, replacing Kaspa's table. Set
  `deflationary_phase_daa_score` and `pre_deflationary_phase_base_subsidy` in
  [params.rs](consensus/core/src/config/params.rs) accordingly (simplest: deflationary phase
  from genesis — `deflationary_phase_daa_score: 0` — so the table alone defines emission).
  ✅ *Verify:* the generator's cap assertion passes as a permanent test;
  `cargo test -p kaspa-consensus coinbase` passes (update Kaspa-specific expected-value
  tests to your schedule in the same commit).

- [ ] **P3.3 — Emission integration check.**
  On a fresh single-node devnet-of-your-network, mine ~1000 blocks; query circulating supply
  via RPC (or sum coinbase outputs via the utxoindex).
  ✅ *Verify:* observed issuance per block matches your table's month-0 value ±
  red-block/merge effects.

---

## Phase 4 — MILESTONE: transparent chain running end-to-end

*Goal: "the fork works." A multi-node private testnet of your renamed, re-parameterized,
capped-supply chain, with wallet and miner. This is the moment the project is demo-able and
the natural checkpoint before the hard part.*

- [ ] **P4.1 — Testnet-in-a-box script.** Write `scripts/x-testnet-local.ps1` (and `.sh`)
  that launches 3 nodes on one machine (distinct ports/appdirs, peered), plus miner
  instructions.
  ✅ *Verify:* running the script from a clean checkout yields 3 synced nodes.

- [ ] **P4.2 — Full user-journey test.** Documented manual script in `docs/x-fork/SMOKE.md`:
  create wallet → mine to it → wait maturity → send to second wallet → restart node →
  balances persist.
  ✅ *Verify:* every step of SMOKE.md passes on the local testnet.

- [ ] **P4.3 — Integration test suite green.**
  `cargo nextest run --release -p kaspa-testing-integration` against your params (some tests
  hardcode Kaspa params/genesis — fix them to use your constants as part of this step).
  ✅ *Verify:* suite passes.

- [ ] **P4.4 — Tag it.** `git tag fork-transparent-v0.1`. Update NOTES.md with current state.
  ✅ *Verify:* tag exists; fresh clone + script reproduces the testnet.

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

- [ ] **P5.1 — Spec: data structures.** Write the Note definition and the pool-state map
  with exact byte layouts (field sizes for `d`, `pk`, `sn`; key scheme — recommend
  secp256k1 Schnorr, same as Kaspa addresses, so existing crypto code is reused). Define
  the **pool commitment**: a hash root over the pool state (e.g. via the existing
  `crypto/smt` sparse merkle tree) that blocks commit to, so any two nodes can check they
  agree and fresh nodes can verify a downloaded pool state.
  ✅ *Verify:* every field has a byte size; a colleague-level reader could implement the
  structs from this section alone.

- [ ] **P5.2 — Spec: transaction format.** How a pool op rides in a Kaspa transaction:
  recommend a dedicated subnetwork ID (see `consensus/core/src/subnets.rs`) with the op
  borsh-encoded in the payload. Mint consumes ordinary transparent outputs of exactly the
  note sum; redeem creates them. Rotate/split/merge touch no transparent value and carry:
  the target serial(s), the new pubkey(s), and a signature by the **old** key over the
  whole op (must cover new pk + a recent block hash or daa-score to prevent replay).
  Split/merge is a transfer with a different denomination multiset in vs. out.
  ✅ *Verify:* section covers all five ops with worked byte-size estimates; total tx size
  target ≤ a few KB — confirm against mass limits and payload size limits in params.rs.

- [ ] **P5.3 — Spec: consensus rules.** Exact validation order per op: serial exists in
  pool state (rotate/split/merge/redeem) or does not yet exist (mint); signature verifies
  against the note's **current** pk; denomination arithmetic balances per op type; mint's
  transparent input sum equals note sum, redeem's output sum likewise. Define the
  parallel-blocks conflict rule: two rotations of the same serial in parallel blocks are
  resolved at the **accepted-transaction** level, exactly like UTXO double-spends between
  merged blocks — first accepted wins, the loser becomes a no-op/invalid. Define mass/fee
  costing per op (a rotate is one sig verify + one map update — cost it like a normal
  1-input tx; no special proof costs exist in this design).
  ✅ *Verify:* section answers every question in this checklist explicitly, including the
  parallel-blocks double-rotate case and signature replay protection.

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

