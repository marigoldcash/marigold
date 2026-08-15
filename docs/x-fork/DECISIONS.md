# Design decisions

Every parameter Phase 1+ code will encode, recorded here before it's implemented. One
row per decision, added when the step that owns it (P1.2, P1.3, ...) executes — this
file doesn't get filled in ahead of that. Read together with
[FORK-PLAN.md](../../FORK-PLAN.md), which is the authority on *why* each decision is
needed; this file is the authority on *what was chosen* and *when*.

| Decision | Choice | Rationale | Date |
|---|---|---|---|
| Coin name | Marigold | Companion-plant story fits the finality-anchor narrative; "Mary's gold" etymology carries the money association. Collision-checked: no coin/CMC listing; nearest name-neighbors are marigold.dev (a Tezos dev company) and marigold.com (a martech firm), neither a coin. | 2026-08-14 |
| Ticker | MAGLD | See "P1.2 — ticker collision check" below for the full search trail. First pick MGLD had a minor collision (dead/unverified BSC token); alternate MCASH had a thematic collision (a dead 2019 project pitching itself as private+feeless digital cash). MAGLD verified fully clean on CoinGecko (zero results) and CoinMarketCap. | 2026-08-14 |
| Address prefix (mainnet) | `marigold` | Matches the coin name; lowercase, short, human-readable as the bech32 address prefix (e.g. `marigold:qq...`). | 2026-08-14 |
| Address prefix (testnet) | `marigoldtest` | Mirrors upstream's `kaspa`/`kaspatest` convention (mainnet prefix + `test` suffix). | 2026-08-14 |
| Decimal precision | 8 decimals (1 marigold = 10⁸ base units) | Keep Kaspa's precision unchanged — no reason to diverge; simplifies porting wallet/RPC display code (amount formatting, `SOMPI_PER_KASPA`-style constants) with only a rename, no rescaling logic. | 2026-08-14 |
| Base unit name | `petal` | Fits the marigold/flower branding (a flower's petals are its smallest, most numerous parts — mirrors how sompi are Kaspa's smallest unit); short, pronounceable, not already a unit name in this codebase or in wallet/RPC display code elsewhere. | 2026-08-14 |
| Supply cap | 210,000,000 MAGLD — hard cap, **no tail emission** | 10× Bitcoin's cap. At any given market cap the unit is 10× cheaper than a 21M cap would make it, keeping the smallest pool note (0.01, per the P1.6 recommended set) usable for sub-dollar private payments and letting prices read in whole marigolds — granularity matters more for an everyday-cash coin than maximum scarcity branding. Hard cap kept for fair-launch credibility; a Dogecoin-style tail could only ever be added by explicit future hard fork if circulation fees demonstrably fail to carry security (recorded openly here so it's never a quiet change). | 2026-08-14 |
| Emission curve | Smooth geometric decay from genesis: reward halves every **3 years** via monthly steps (monthly factor 2^(−1/36)); **no pre-deflationary phase** | No cliff moments ever — fee share grows as subsidy fades. ~20.6% of supply mined in year 1 (Bitcoin-comparable front-loading; Kaspa's 1-yr halving shape would have mined ~50% in year 1 into a tiny launch hashrate — stealth-premine optics), ~90% by year 10, per-block reward quantizes below 1 petal around **year ~72**, so subsidy outlives the P5.8 finality-anchor sunset by decades. Deflationary from genesis is also the simplest P3.2 implementation. **Mechanically applied (not re-decided) at P2.6**: `deflationary_phase_daa_score` set to `0` for mainnet/testnet in `params.rs`, since leaving it at real Kaspa's legacy checkpoint value produced a genuine bug (10×-too-high flat subsidy — see NOTES.md's P2.6 writeup). The real subsidy table/curve numbers above remain P3.2's job. | 2026-08-14 |
| Initial block reward | **152,280,842.63 petals/sec ≈ 1.5228084263 MAGLD/sec** (15,228,085 petals/block at 10 BPS, `div_ceil`). Locked in by P3.2's generator: 1016-month table, total emission 20,999,999,999,644,200 petals — **355,800 petals (~0.0036 MAGLD) under the 210,000,000 MAGLD cap**, table tapers to an exact 0 at month 1015 (~84.6 years). Per-block reward reaches its final 1-petal floor at month 862 (~71.8 years), matching the original ~year-72 estimate. | Derived, not independently chosen: found by bisecting for the largest base subsidy whose discrete, rounded monthly table still sums to ≤ cap (not the closed-form continuous estimate, which would slightly overshoot after rounding). P3.2's generator (`consensus/src/processes/coinbase.rs::tests::generate_subsidy_table`, `#[ignore]`d, rerunnable) computes this exactly; `total_emission_stays_under_cap` enforces it permanently. | 2026-08-15 |
| Security endgame posture | Three-phase: **anchors guard youth → emission guards middle age → circulation fees guard maturity** | Marigold is a circulation coin: every payment is an on-chain rotate op paying a fee, so a *successful* cash economy is a permanent fee base — unlike store-of-value coins whose activity (and fee revenue) dries up at maturity. The ~72-year smooth subsidy runway is the bridge to that fee-funded maturity. This is the bet, stated openly. | 2026-08-14 |
| Launch allocation | **Fair launch from zero** — no premine, no dev fund, no airdrop; every MAGLD enters circulation via the P1.4 emission schedule from block 0 | Matches the plan's own defensible-zone guidance ("honesty + large premine is a hard sell") at the clean end of the spectrum; strengthens P1.9's regulatory posture and P9.6 legal review (no allocation to a founding entity to justify); mirrors Kaspa's own no-premine launch, which the project already inherits credibility from by forking. No vesting/governance question to resolve since there's no allocation to vest. | 2026-08-14 |
| Pool denominations | Powers of ten, whole coins: **{0.01, 0.1, 1, 10, 100, 1000, 10000, 100000}** (8 tiers) | Extends the plan's recommended set upward by two tiers (10000, 100000). Since split/merge is always available at an exact 10× factor (per the architecture), adding large denominations costs nothing at the small end — no fragmentation of the anonymity sets for everyday-payment sizes — while saving large holders from managing piles of 1000-notes to represent one big balance which is worse for usability. Also a wallet holding e.g. 50× 1000-notes leaks an approximate balance by note count / UTXO-style clustering that one 100000-note or a few don't. Floor stays at 0.01 (the P5.6/P8.3 smallest-denomination discussion — spam/floor pricing — is unaffected, only the ceiling moved). | 2026-08-14 |
| Transparent tier policy | Confirmed as-is: fork launches **transparent-only** (Phases 2-4), pool added later (Phases 5-7) | No change from the plan's existing phase structure — confirming it here just makes it an explicit recorded decision rather than an implicit one baked into the phase ordering. | 2026-08-14 |
| Fee destination | **The including block's miner**, via normal coinbase fee accounting — never burned, no dev fund | Per-block collection is statistically hashrate-proportional at 10 BPS (a miner with X% of hashrate collects ~X% of fees), which was the stated goal, with zero new machinery. A fee-funded dev fund was considered and rejected: it would reverse P1.5's fair-launch decision through the side door, convert the legal posture from "published software" to "operates a paid service" and drain the P1.4 endgame mechanism (circulation fees fund security). A community treasury remains possible later only via explicit DAO-voted hard fork (same pattern as the tail-emission reserve option). Supply cap unaffected: fees are recycled value, not emission. | 2026-08-14 |
| Regulatory posture | **No legal entity for now.** A Swiss-style nonprofit foundation (Ethereum Foundation model) is the leading candidate *if* one becomes necessary later (e.g. to hold domains/trademark, organize the P9.1 trustee ceremony) — jurisdiction and structure deliberately deferred, not decided today. See the full recorded paragraph below ("P1.9 — Regulatory posture"). **Not legal advice; real counsel required before mainnet (P9.6).** | 2026-08-14 |
| Genesis motto (mainnet) | *"Hell is other people's monetary policy. — Sartre"* | User's choice. Embedded in the P2.5 genesis `coinbase_payload` — 50 UTF-8 bytes, well under the 204-byte limit. **Placeholder, not final**: P9.5 explicitly regenerates mainnet's genesis with the real launch timestamp and motto right before mainnet launch, per the plan's own design — this is the Phase-2-milestone value. | 2026-08-14 |
| Genesis motto (testnet) | Plain identifier: `marigold-testnet` | User's choice, over reusing the same quote — mirrors upstream's own testnet/simnet convention (plain descriptive label, not a quote), keeping the Sartre line unique to mainnet where it carries real weight. | 2026-08-14 |
| Block explorer domain | `explorer.marigold.cash` (used in the P2.8 CLI terminal link-matcher, replacing `explorer.kaspa.org`) | No explorer exists yet — that's P9.4's job — so this is a forward-looking placeholder under the already-canonical `marigold.cash` domain (P1.2), chosen because leaving the CLI pointed at Kaspa's real explorer would show wrong/misleading results (someone else's address or "not found") for a Marigold address. P9.4 should stand up the explorer at this exact subdomain, or this string needs updating again. | 2026-08-14 |
| Legacy-Kaspa wallet import code | **Keep untouched through Phases 2–6; remove or hard-disable at P7.0 (new plan step), before any binaries reach outside users (P8.7 at the latest)** | The inherited wallet stack carries live, user-facing import paths for two deprecated third-party Kaspa wallet formats: KDX (`compat/gen0.rs`, reached via the CLI's `import legacy` / `account import legacy-data` commands, which read the user's real KDX file from disk and prompt for its password) and the Go `kaspawallet` (`compat/gen1.rs` + four `import_kaspawallet_golang_*` API functions). On a fair-launch chain these can never find funds — their only possible real-world effect is inviting users to expose real Kaspa passwords/keys inside Marigold software (key-reuse hazard; normalizes the exact behavior wallet-phishing needs). Not deleted immediately because: the inherited wallet is load-bearing for P4.2's smoke test; deletion is permanent upstream-merge friction best paid once; and it's one corner of the bigger P7.0 question (fate of the whole seed-phrase wallet stack), which deserves a single coherent decision. P2.8's string-rebrand of these modules was correct in the meantime — while the code exists, it must point at KDX's real paths or it's broken code. | 2026-08-15 |
| Network ports | Mirror Kaspa's exact port structure, shifted to the **26xxx/27xxx/28xxx** block: gRPC mainnet **26110**/testnet **26210**/simnet 26510/devnet 26610; borsh-wRPC mainnet **27110**/testnet **27210**/simnet 27510/devnet 27610; JSON-wRPC mainnet **28110**/testnet **28210**/simnet 28510/devnet 28610; P2P mainnet **26111**/testnet **26211** (suffix 10)/26311 (suffix 12)/26411 (other)/simnet 26511/devnet 26611 | No protocol-level collision risk either way (P2.3's handshake already rejects cross-network peers), but Kaspa's own source (`consensus/core/src/network.rs:239-242`) documents *why* they vary P2P ports per network: "avoiding repeatedly failing P2P handshakes between nodes on different networks" — i.e. so multiple network variants can run on one host without an OS-level bind conflict. Reusing Kaspa's exact ports would make it impossible to run a Kaspa node and a Marigold node on the same machine simultaneously (relevant for the P8.6 upstream-merge drill, multi-chain tooling, etc.). Mirroring Kaspa's own offset structure (not inventing a new one) keeps P2.2's implementation a trivial value shift. | 2026-08-14 |
| Fee policy | **Inherit Kaspa's fee model** (near-zero, mass-based) for the transparent tier. Pool ops pay with **fee stamps**: whole small-denomination notes consumed inside the op (an embedded redeem-with-no-transparent-output — the stamp's value becomes the miner fee through Kaspa's native value-in-minus-value-out mechanism). Unified conservation rule across all ops: Σ(note in) + Σ(transparent in) = Σ(note out) + Σ(transparent out) + fee. **REVISED same-day**: supersedes the earlier "small transparent-value input from a wallet-held fee reserve" answer, which violated the core no-wallet principle (it would have required every wallet to hold a transparent balance with an address — reintroducing exactly what the design removes). See "P1.8 — pool-op fee mechanism (revised: fee stamps)" below. | Zero-fee + reward-per-action was already ruled out as a spam vector. Notes are fixed-denomination (can't be fractionally shaved without destroying the anonymity set), so fees must be whole small notes. Stamps keep the wallet a pure key manager (a stamp is just another note key), put no transparent address on any pool op (mint/redeem return to being the only transparent touchpoints, as originally designed), need zero new miner-payment machinery, and still support congestion pricing (attach more stamps = higher fee/mass priority — a denomination-quantized fee market). | 2026-08-14 |
| Transfer modes | **Both bearer key-handover and sign-to-fresh-pk are first-class, supported equally** — same on-chain `TransferOp` (P5.2) for both; the mode is a wallet-level choice invisible to consensus. Universal settlement rule: a note is finally yours when a rotation to a key only you know confirms on-chain. Formalized against the concrete wire format at P5.2/P5.3 — see [POOL-SPEC.md](POOL-SPEC.md)'s P5.5 section for the full flows and the shared-key-window vs. pk-freshness-window distinction. | Originally decided at the plan-authoring stage (no single "correct" transfer UX for a cash-like coin — different situations call for different modes: passive receipt for granny-at-a-market, private-key-never-shared for point-of-sale), reconfirmed once the transaction format existed to check the decision against — both modes reduce to the identical wire shape with zero special-casing needed, which is itself evidence the earlier decision was sound rather than aspirational. | 2026-08-15 |
| Note serial number (`sn`) | **`sn = H(creating_tx_id \|\| output_index)`** — a domain-separated hash, not an incrementing counter or a reuse of `pk` | `pk` can't serve as the pool-map key: rotation replaces it, and (P5.6) it's not even required to be unique across notes. `sn` needed to be stable-for-life and unique-per-note simultaneously. Chose to mirror Kaspa's existing `(transaction_id, output_index)` UTXO-outpoint pattern rather than invent a counter — zero extra consensus state to "assign" serials, collision resistance inherited from transaction-ID hashing. Applies uniformly to notes created by mint, split, or merge. | 2026-08-15 |
| Pool state commitment | **A new, dedicated 32-byte `Header` field (`pool_commitment`), not a reuse of `accepted_id_merkle_root`** | Seq-commit (KIP-21) already repurposes `accepted_id_merkle_root` post-toccata for its own SMT root. Overloading that same field for a second, unrelated commitment would make one field mean two different things depending on which of two independent forks activated. A dedicated field costs nothing extra in return (headers already carry multiple 32-byte commitment fields) and keeps each commitment legible on its own. Implementing this (new block version, `ForkActivation`) is explicitly Phase 6's job, not P5.1's — this is a spec-level byte-layout decision only. | 2026-08-15 |

## Notes

### P1.2 — Ticker collision check

Checked on CoinGecko and CoinMarketCap, per the plan's instruction, before locking in.

- **MGLD** (the original pre-decision, dated 2026-08-13): CoinGecko shows 0 results
  (clean) and there's no verified CoinMarketCap listing, but an obscure, essentially
  dead BSC token called "Metallurgy" trades under MGLD on unverified DEX-scan pairs
  (PancakeSwap v2, Biswap v2 — ~$21-22/24h combined volume, no market cap). Flagged to
  the user as a weak but non-zero collision.
- Two clean alternates identified as backups: **MRGD**, **MRGL** (both 0 results on
  CoinGecko).
- User additionally asked to check **MAGLD**, **MAR**, **MARI**, **MARIG**, **MAG**.
  All came back with no *exact*-ticker match on CoinGecko. MAR, MARI, and MAG sit in
  crowded naming neighborhoods (MarsCoin/Dogelon Mars; Marinade/Marina Protocol/Marie
  Rose AI; MAGA-adjacent tokens respectively) even without an exact collision. MAGLD
  and MARIG were the two fully clean, unambiguous options.
- User then proposed **MCASH** as a further alternative: 0 results on CoinGecko, but
  CoinMarketCap has two prior (both dead/untracked, zero volume) projects on that
  ticker — **Mcashchain** (2019, BEP2; described itself as "a foundation for instant,
  feeless transactions... privacy, governance" — thematically close enough to
  Marigold's own pitch to be worth avoiding) and the unrelated **MMScash**. Flagged as
  a thematic near-miss even though technically inactive.
- **Final choice: MAGLD** — zero collisions of any kind (name or ticker, active or
  dead) found on either CoinGecko or CoinMarketCap.

### P1.4 — Emission math & the security endgame

The working formula (P3.2 implements this exactly): with initial rate `R` MAGLD/sec,
monthly decay factor `r = 2^(−1/36)` (halving every 36 months), and
`S = 2,629,800` seconds per month (365.25-day year), total emission is the convergent
geometric series `R·S/(1−r) ≈ R × 137.9M`. Solving for the 210M cap gives
`R ≈ 1.5228 MAGLD/sec`. Front-loading: year 1 mines `1 − 2^(−1/3) ≈ 20.6%`,
10 years ≈ 90.1%. The per-block reward (R/10 at 10 BPS) falls below 1 petal (10⁻⁸)
around year 72, which is where emission effectively ends — a quantization fade-out,
not a cliff, and the hard cap holds throughout because the series converges.

Alternatives considered and rejected:
- **Tail emission (Dogecoin-style)** — guarantees a perpetual security floor and
  replaces lost bearer notes, but forfeits the hard-cap credibility line at launch.
  Held in reserve: adding a tail later via explicit hard fork remains possible if
  circulation fees demonstrably fail; the reverse (launching with a tail, later
  claiming scarcity) is not. Lost-note deflation is accepted as cash-like (physical
  cash economies lose notes too).
- **Kaspa's 1-year halving shape** — would mine ~50% of supply in year 1 into a tiny
  launch hashrate (stealth-premine optics) and leave only ~22 years of runway.

**⚠️ Flag for P5.2/P5.3 (spec phase):** the P5.2 sketch says rotate/split/merge ops
"touch no transparent value" — but the endgame posture above depends on pool ops
actually *paying fees*. The spec must define the fee-payment mechanism for pool ops.
Resolved to a recommended default under P1.8 below — see that section for the
mechanism.

### P3.2 — Subsidy table implementation (clarifies P1.4)

Implemented P1.4's formula by replacing Kaspa's `SUBSIDY_BY_MONTH_TABLE` (426 entries,
1-year halving) with Marigold's own (1016 entries, 3-year/36-month halving),
preserving the existing table-driven `CoinbaseManager` architecture rather than
switching to a closed-form runtime calculation (the option flagged as open at P3.1) —
simpler diff, keeps the exact-zero-tail behavior "for free." The generator bisects
for the largest base subsidy whose *discrete, rounded* table sums to ≤ cap (not the
continuous closed-form estimate, which overshoots slightly once you round each
month) — see the table row above for the exact final numbers.

**One clarification to P1.4's "no pre-deflationary phase" language**: that's true for
mainnet, testnet, and devnet (`deflationary_phase_daa_score: 0`, unchanged from
P2.6), but **not** for simnet, which keeps a real flat pre-deflationary phase
(`TenBps::deflationary_phase_daa_score()`, a real-Kaspa-derived value, unchanged from
before P2.6 too). This was checked, not assumed: setting simnet's
`deflationary_phase_daa_score` to 0 "for consistency" was tried first and broke a
real, passing test
(`testing/integration/src/daemon_integration_tests.rs::daemon_utxos_propagation_test`,
plus a sibling assertion), which mines `coinbase_maturity` blocks and asserts the
resulting balance as `initial_blocks * SIMNET_PARAMS.pre_deflationary_phase_base_subsidy`
— i.e. it deliberately relies on simnet paying a flat, predictable subsidy for its
initial mining run rather than the decaying table. Simnet is a PoW-skipped internal
benchmark/test harness (per its own existing params comment, built for "mempool
benchmarks out of the box"), never a real user-facing network, so P1.4/P1.5's
fair-launch commitment was never meant to bind it — reverted to keep that test
correct rather than force uniformity where it isn't the actual decision.

**Three more real bugs found via full-workspace + ignored-test verification, each
its own commit**: (1) `body_validation_in_context.rs`'s `validate_body_in_context_test`
had a hardcoded expected-subsidy literal (`4400000000`, Kaspa's real month-0 value)
that needed updating to ours (`15228085`). (2) `verify_crescendo_emission_schedule`
(an `#[ignore]`d, ~15-20-minute test at our table's scale) cross-checks
`calc_block_subsidy` against `legacy_calc_block_subsidy`, which assumes a 1-BPS
reference rate; this assumption silently broke back at P2.2 (which deliberately made
`pre_crescendo_target_time_per_block` match the real 10 BPS rate instead of a fake
historical 1 BPS), but went uncaught until now because the test is `#[ignore]`d and
was never actually run this session before P3.2 — fixed the comparison to convert
blocks→seconds and scale the legacy result by the real pre-crescendo BPS, rather
than assuming 1:1. (3) Five `goref_*` integration tests
(`testing/integration/src/consensus_integration_tests.rs`) replay real, literal
historical Kaspa mainnet block data with real historical coinbase subsidies baked
into the recorded fixtures — permanently incompatible with a from-scratch chain's
own economics, not a bug to fix. Marked `#[ignore]` with an explanatory reason
rather than deleted, so the fixtures/test code stay available for reference.

### P1.8 — Pool-op fee mechanism (revised: fee stamps)

Direct question from the user: pool ops touch no transparent value, so *which
denomination pays their fee*? First answer (same day) was a wallet-held transparent
"fee reserve" — **rejected by the user as violating the non-negotiable core
principle** that the wallet is a pure key manager holding nothing but note keys: no
transparent balance, no address, no second thing to back up. Revised to the mechanism
below, which formalizes the user's own proposals (wallet splits bills to fee-payable
size / sender adds a small fee coin on top so the receiver's note arrives intact).

**Constraint (unchanged):** the fee cannot be shaved off a note. Notes are fixed
denominations by design — every note of size X must be indistinguishable from every
other — so an off-denomination 99.999-note can never exist. Fees must therefore be
paid in *whole* small notes.

**Mechanism — fee stamps.** A pool-op transaction consumes one or more whole
small-denomination notes ("stamps") via an embedded **redeem with no transparent
output**: the stamp is destroyed and its value automatically becomes the miner's fee
through Kaspa's native `fee = value-in − value-out` accounting. No transparent
address appears anywhere; the payer never holds transparent value; miners need no new
payment machinery. This yields one conservation rule unifying all five ops (the shape
the P6.6 value-conservation test already anticipates):

> Σ(note inputs) + Σ(transparent inputs) = Σ(note outputs) + Σ(transparent outputs) + fee

Congestion pricing works natively: attaching more/larger stamps raises the tx's
fee-per-mass in the existing mempool ordering — a denomination-quantized fee market,
no protocol-fixed fee needed.

**Fee-inclusive vs fee-additive is wallet UX, not protocol.** "Receiver gets 99.99 in
valid change denominations" (fee taken from the amount) and "sender attaches a stamp
on top, receiver gets the intact 100" (fee added) are the same chain mechanism; the
wallet exposes the toggle, like cash registers vs stamped envelopes.

**Bootstrap (first stamp problem):** a fresh receiver holding one bearer note needs a
stamp to rotate it. Three composing answers, for P5.2/P5.6 to formalize:
1. **Value-touching ops self-fund** — any op that changes the denomination multiset
   can pay its fee from the value passing through, e.g. deep-split
   100 → 9×10 + 9×1 + 9×0.1 + 9×0.01 (= 99.99) + 0.01 fee. One split yields stamps
   forever; only the pure rotate requires a pre-existing stamp.
2. **Handovers include a stamp** — the paper/QR bearer bundle carries the note key
   plus a stamp key (cash etiquette: the stamped return envelope).
3. **Mint produces stamps** — wallets mint a strip of stamps alongside big notes by
   default.

**Stamp sizing (deliberately open until P6.6/P8.3 fee calibration):** with inherited
relay params (~100 base-units/gram, small-tx mass) a pool op's fee lands around
0.001–0.003 MAGLD, so either the 0.01 note is the standard stamp (clears comfortably)
or the ladder gains a 0.001 tier. Relay-fee constants are ours to tune in the fork, so
this is a calibration decision, not a design decision — the P1.6 set stands unchanged
until then, with the 0.001 fee tier recorded as a live option.

**True to principle (strictly better than the rejected fee-reserve).** No transparent 
address attaches to any pool op — mint and redeem return to being the only transparent
touchpoints, exactly as the architecture originally claimed. What remains: a stamp's
lineage is public like any note's, so ops sharing stamp ancestry are linkable within
the note graph. That is the same class of visibility as the already-disclosed
rotate/split/merge graph structure (not a new category of leak), but P5.7 should name
it explicitly, and P5.6 wallet hygiene can mitigate (don't pay for unrelated ops from
one linkable stamp strip).

**Destination (decided, see "Fee destination" row):** the stamp's value is never
burned — "no transparent output" means none *inside the op transaction*; the value
re-materializes in the including miner's coinbase through Kaspa's standard
fee-collection path and returns to circulation. Per-block collection at 10 BPS is
statistically hashrate-proportional with low variance (small miners collect fees
continuously — a quiet virtue of GHOSTDAG's block frequency), so no fee-splitting
machinery is needed. A fee-funded dev fund (even a capped, quota-then-miners one) was
explicitly rejected — see the row's rationale.

### P1.9 — Regulatory posture

**Not legal advice — a recorded, eyes-open position. Real counsel required before
mainnet (P9.6).**

Marigold is published as open-source software by an individual/informal group, with
no legal entity at this stage. This isn't a placeholder oversight: P1.5 (fair launch,
no premine) and P1.8 (no dev fund, fees go to miners, never to a project-controlled
address) mean there is no revenue, no treasury, and no commercial activity for an
entity to hold — the "just published the code" posture is a factual description, not
a legal fiction wrapped around a business. A Swiss-style nonprofit foundation
(Ethereum Foundation's model) is the leading candidate *if* an entity becomes
necessary later — e.g. to hold the domains/trademark, or organize the P9.1 finality-
anchor trustee ceremony — but jurisdiction and structure are deliberately deferred,
not decided now.

The project understands that the crypto landscape is changing and some coins 
face a hostile regulated-exchange environment and it is expected to worsen on a known 
timeline. Monero was delisted by Binance in February 2024, and by Kraken for EEA
users in late 2024 (citing MiCA); OKX, Huobi, and Bitstamp took similar action;
73 platforms delisted coins in 2024 alone. The EU's Anti-Money Laundering Regulation 
(Regulation (EU) 2024/1624, "AMLR") takes full effect **10 July 2027** and will bar 
regulated crypto-asset service providers (CASPs) from listing, storing, or
processing certain coins and anonymous accounts, enforced by a new authority (AMLA).
Notably, the AMLR targets *regulated intermediaries*, not individual self-custody or
peer-to-peer use — there is no mechanism to ban a wallet or a DEX trade between two
people, which is the activity Marigold is actually built around. Consistent with
that reality (and consistent with the plan's original transparent-tier design),
**distribution is expected to depend on CEXs willingness to accept the coin, DEXs and 
peer-to-peer channels**. This posture will need real legal review before mainnet, 
particularly once P1.5/P1.8 are cross-checked against P9.1's trustee-ceremony 
organizing (which may itself imply some jurisdictional footprint even without a 
formal entity).

### P5.2 — Transaction format decisions

Several smaller, genuine design choices bundled into one spec section
([POOL-SPEC.md](POOL-SPEC.md)'s P5.2), recorded together since they're tightly coupled:

**Subnetwork mechanism**: a dedicated user-lane namespace
(`SubnetworkId::from_namespace`), not the reserved `RegistrySubnetwork` path. Checked
directly (grep) that `SUBNETWORK_ID_REGISTRY` has no active registration/dispatch
mechanism anywhere in the codebase today — only test-fixture usages — while
`from_namespace` user lanes are the real, already-implemented mechanism backing
Toccata's "non-native/non-coinbase subnetworks" feature. Using the mechanism that's
actually load-bearing today, not the one that merely sounds more official.

**Unifying rotate/split/merge into one `TransferOp`**: the plan's own text already
frames split/merge as "a transfer with a different multiset in vs. out" — taking that
literally collapses three near-identical wire shapes into one (consumed notes,
produced notes, one conservation check), with "rotate"/"split"/"merge" surviving only
as descriptive labels for what a given `Transfer`'s multiset happened to do. Simpler
spec, simpler future implementation, and it lets one transaction freely mix e.g.
split-and-partial-rotate without a fourth wire shape ever being needed.

**Freshness window: 36,000 DAA-score units (≈1 hour at 10 BPS)**, the anti-replay
anchor every pool-op signature covers. Chosen, not left as a placeholder: long enough
that no realistic in-person or remote payment flow (which settle in seconds at 10 BPS)
risks the signature expiring mid-transaction; short enough that a leaked or abandoned
signed op — a stale invoice, a bearer QR handed over late — stops being a live
liability within the same session it was created. Same category as P1.8's stamp-sizing
note: a concrete recommended default subject to real-world calibration at Phase 6/P6.6,
not a first-principles-derived constant.

**No new "fee stamp" data type**: fee stamps (P1.8) turned out to need zero new wire
format once `Transfer`'s conservation rule existed — a stamp is simply a consumed
serial with no matching produced note, and the resulting value gap is fee, by the same
rule that makes self-funding split/merge work. Discovered while writing the spec, not
planned in advance; recorded here because it simplifies P1.8's mechanism further than
that decision's own text anticipated (no "embedded redeem" sub-structure needed — it's
the same conservation check already required for every other reason).

**Real consensus-rule gap found, not assumed away**: `check_transaction_inputs_count`
(`consensus/src/processes/transaction_validator/tx_validation_in_isolation.rs:78-80`)
currently rejects any non-coinbase transaction with zero inputs. A pure
`Transfer`/`Redeem` has zero transparent inputs by design, so this needs an explicit
exception before Phase 6 implementation — checked directly against the real validation
code rather than assuming "touches no transparent value" already worked under existing
rules. Flagged for P5.3 to formalize as a consensus rule change.
