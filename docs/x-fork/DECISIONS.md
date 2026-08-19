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
| Inherited seed-phrase wallet stack (P7.0) | **Keep as the transparent-tier wallet tool**, with the legacy-Kaspa import surfaces removed in the same step (executed 2026-08-16, not left to the P8.7 deadline): `compat/gen0.rs` (KDX) and `compat/gen1.rs` (Go `kaspawallet`) deleted; the four `import_kaspawallet_golang_*` API functions, `import_legacy_keydata`, `import_gen1_keydata`, and their wire-file types removed from `wallet/core`; the CLI's `account import legacy-data` arm removed, `account import mnemonic legacy` hard-guarded with an explanatory refusal, help/hint text updated, and the already-dead `cli/src/modules/import.rs` deleted. The legacy account *storage* variant and gen0 derivation code are retained so pre-existing wallet files still open — compatibility without an import path. | The transparent tier is permanent infrastructure (mining rewards, mint funding, redeem outputs, integrator fee keys), and the inherited stack is the only transparent wallet — working, rebranded (P2.8), and load-bearing for existing tests. Stripping it would be large churn plus upstream-merge friction for negative user value; feature-gating adds a build matrix without removing the maintenance reality. Options considered: keep (chosen), feature-gate, strip. User-ratified 2026-08-16. The import removal is unconditional per the 2026-08-15 row above (key-reuse/phishing hazard on a fair-launch chain); doing it now rather than at P8.7 removes the hazard at the earliest coherent moment. | 2026-08-16 |

| Public positioning / terminology | **Never market Marigold as a "privacy coin."** Official positioning: a **transparent bearer-note chain** ("digital cash with a fully auditable ledger"). The three claims that carry the pitch, all mechanically verifiable: (1) every unit of supply is publicly accountable at every block (consensus-enforced `Σ pool + transparent == emitted`); (2) nothing on the chain is encrypted or obfuscated — the pool is plaintext, there are no ring signatures, stealth addresses, confidential amounts or mixers, and the only cryptography is signatures authorizing spends (wallet-vault encryption is local key storage, not chain data); (3) exchanges and auditors only ever touch the transparent tier, and every entry/exit to the note pool is a visible, value-conserving public event. Equally binding in the other direction: never deny or downplay the unlinkability either — the denomination ladder and rotation are essential for fungibility like cash, and claiming the property is accidental would be false. The honest sentence is: *"nothing is hidden — the ledger is complete about value by construction, like physical cash."* | User-initiated (2026-08-18), refined in discussion. Rationale: "privacy coin" is a regulatory/exchange classification magnet (Monero delistings, EU AMLR 2027 — see P1.9's recorded posture) and misdescribes the mechanism used by Marigold — Monero-class coins *hide recorded data* cryptographically; Marigold *never records* identity linkage in the first place. Positioning by what it verifiably is, beats positioning by category. Requires a terminology pass over all public-facing docs before the repo goes public. | 2026-08-18 |

| AI-assistance disclosure | **Keep the development record exactly as it is — no scrub, no banner.** The repo publishes with its FORK-PLAN's session-sized step structure and model-capability flags, NOTES.md/STATE.md session handoffs — all as-is. Disclosure is matter-of-fact: one honest sentence in the README/FAQ (lands with the pre-publication README rewrite) — *developed by a human founder using AI-assisted engineering, with all consensus-critical work externally reviewed: spec review by external cryptographers (P5.9, `docs/x-fork/reviews/`), full security audit before mainnet (P8.8), months of public testnet soak (P8.7)*.
. The security argument always points at review/audit/soak, never at authorship in either direction. | User-ratified 2026-08-19, with the founder's own stated reason on record: the assistance was real and material ("could not have done it without it, at least not in this timeframe"), so the record stands because it's true. Options considered: (a) scrub — rejected as disqualifying, not merely risky: requires rewriting the entire git history plus the development docs whose organizing principle is session-sized AI-executable steps, directly contradicts the project's own documented don't-rewrite-history convention and the "nothing is hidden" brand, and creates a fiction that must be maintained forever; decided that the scrub window closes permanently at publication. (b) Marketing banner — rejected: "first AI-programmed coin" is neither true nor verifiable, sits on the rugpull-token shelf, misdirects attention from the actual design to the tooling, and promotional use of a trademark invites a cease-and-desist. (c) Matter-of-fact (chosen): in 2026 the stigma attaches to *unreviewed* AI code; this repo's visible discipline (phase gates, live verification, recorded corrections, external reviews) is the differentiator, and for a consensus system trust should rest on tests/review/audit/soak regardless of who typed the code. | 2026-08-19 |

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

### P5.6 — Key-algorithm-deprecation mechanism

New design, not transcribed from anywhere: the plan asks P5.6 to "include the
network-signaled key-rotation upgrade story from the architecture paragraph," but no such
paragraph exists anywhere else in this repo (checked via grep before writing anything).
Designed one from scratch, reusing only mechanisms already specified elsewhere rather
than inventing new ones: a future key-algorithm deprecation is a `ForkActivation`-gated
consensus rule (the identical mechanism `crescendo_activation`/`toccata_activation`
already use) that blocks *new* notes from using a deprecated key format while leaving
existing deprecated-format notes fully spendable via ordinary rotation — mirroring how
Kaspa addresses already support multiple coexisting key formats via a `Version` field.
Wallets learn of the schedule via software updates or RPC query, then proactively
self-sweep using the exact same rotate mechanism already specified for the
same-key-in-two-wallets hazard. Deliberately did not design a second key format to
support today — P5.1 fixes exactly one (secp256k1 Schnorr) at launch; this section specs
only the *mechanism* a migration would use when one is eventually needed.

### P5.8 — Finality anchor parameters (k, n, cadence, depth, T, M, K, sunset schedule)

The plan's own text decided the *mechanism* (federated finality guard, sunsetting) and
recommended some values; this session chose the exact numbers, each with reasoning
recorded here since they're genuinely consequential (real chain security parameters, not
formatting choices) and P5.9's external cryptography review will need to evaluate them
specifically, not just the mechanism shape.

- **3-of-5 trustees**: adopted the plan's own recommendation as-is — tolerates 2
  simultaneous unavailable/uncooperative trustees, requires majority collusion/compromise
  for any misbehavior, small enough that "independent orgs/geos" stays a checkable
  property rather than diffuse to meaninglessness.
- **30-second launch cadence, 600-DAA-score (~1 minute) anchor depth**: the tight end of
  the plan's own 30-60s range, chosen deliberately — the chain is most vulnerable exactly
  at launch (smallest honest hashrate, largest relative size of any external ASIC fleet
  that could be redirected against it), so the strongest protection belongs there; it
  eases via the staged decay schedule as real security arrives, not by starting loose.
- **T = 10⁶ × genesis difficulty target, not a fraction of Kaspa mainnet's difficulty.**
  This is a genuine refinement over the plan's own literal framing ("any sliver of
  Kaspa's ASIC fleet" suggested comparing to Kaspa's real difficulty), caught while
  writing the spec: a threshold referencing another chain's difficulty isn't on-chain
  data Marigold's own consensus can deterministically verify, which would directly
  violate the plan's own "exact deterministic function of on-chain data" requirement (a
  fuzzy/external definition is explicitly called out as a chain-split bug). Redefined
  purely against Marigold's own genesis difficulty instead — fully self-contained,
  no oracle, no off-chain input, and a million-fold sustained hashrate increase from a
  cold launch is still a strong organic-adoption signal in its own right. The mechanism
  (a fixed multiplier of genesis difficulty) is the durable part; the exact `10⁶`
  multiplier is a calibration point, same treatment as P1.8's stamp sizing and P2.5's
  genesis timestamp.
- **M = 6 months, K = 5 years**: M balances catching a fleeting difficulty spike (needs
  to be long enough that sustaining it is a real, expensive commitment) against not
  delaying legitimate easing once real security has arrived; K is an independent time
  floor long enough that any attacker patiently mining honestly toward the T threshold
  has sunk years of real resources with no guaranteed payoff, short enough not to
  indefinitely extend the trust period for a chain that's clearly already succeeded.
  Both are the plan's named parameters with this session's chosen concrete values.
- **5-stage cadence decay (30s → 1h → 1d → 1wk → advisory-only) and a 20-year hard
  maximum DAA score (6,311,520,000) for unconditional trustee-key expiry**: the plan's
  own example shape (30s → hourly → daily → weekly → never), given concrete triggers —
  Stage 1 on the difficulty condition alone (an early, partial signal), Stage 2 on the
  full T+M+K retirement trigger, Stage 3 two years after that, Stage 4 at a fixed
  20-year mark regardless of any network condition, per the plan's explicit "trust must
  end even if growth disappoints" requirement. 20 years was chosen as a multiple of the
  K=5-year floor with real margin (allows the full staged decay to play out even for a
  chain that only just barely clears retirement near the K floor) while still being a
  genuinely finite, non-indefinite commitment.
- **Honesty about the limits of a k-of-n federation**: recorded explicitly in the spec,
  not glossed over — a genuinely compromised 3-of-5 majority *can* sign a false anchor
  endorsing an attacker's chain. This is the same trust model every k-of-n federation
  carries; the mitigation is trustee independence (a practical barrier, not a
  cryptographic guarantee) and the sunset itself (bounding how long that trust is ever
  extended, not eliminating the need for it during the young-chain phase where it's
  genuinely the best available option per the plan's own rationale).

### Note vault, backup, and restore-rotation policy (P7.6, decided ahead of execution, 2026-08-17)

Design worked out with the user before P7.6's implementation, replacing P5.6's original
"paper QR is the backup" framing with a **note vault** — the vault is the primary,
day-to-day backup mechanism; the paper QR (P5.6, already specced) survives as one
printable *export* of the same encrypted entries, not a separate design. Recorded here
ahead of execution per this file's own convention (P1.2, P1.3, ...) — POOL-SPEC.md P5.6
gets the corresponding spec-text update alongside this entry.

**Storage format — one file per note, plaintext filename, encrypted contents.** A
`notes/` directory with one status subdirectory per `NoteStatus` (`active/`,
`handed-over/`, `superseded/`) — a status change is an atomic file rename, and the
directory tree is self-describing without opening a single file. Each note's file is
named for its public, already-on-chain-visible metadata (denomination/value; serial),
and its *contents* — `sk`, plus enough to reconstruct the row — are encrypted under one
per-wallet **vault key K** (XChaCha20Poly1305, per-file nonce; the same primitive the
existing wallet encryption already uses, not a new one). This is a deliberate reversal
of today's implementation (P7.1's single encrypted map, decrypted-and-reencrypted in
full on every touch — a real weakness the user identified: every single-note operation
today transiently holds *every* note key in memory, not just backup/restore). Balance
and coin selection read filenames only, zero decryption; a spend decrypts exactly the
selected files. The exposure window shrinks from "every key, every operation" to "only
the notes being spent, only while spending" — it cannot reach zero (signing needs the
plaintext `sk` momentarily and K must exist in memory for that moment), but this is the
smallest that window gets without a hardware signer.

**Accepted trade-off, stated openly**: plaintext filenames (value, and implicitly serial)
leak local inventory metadata to anyone who can list the directory — more than today's
single-blob format leaks (its size alone is a much coarser signal). Judged acceptable:
a local observer that far in is usually local compromise regardless of file layout, and
blinding filenames breaks the compute-without-decrypting property this format exists for.
Primitive and legible beats clever here.

**24-word vault key ceremony, explicitly not a derivation seed.** K itself — the file
encryption key, not a BIP32/BIP39-style master key deriving note keys — is presented
once, at vault creation, as 24 words (the classic wallet-onboarding shape users already
recognize, reused for its ceremony familiarity, not its cryptographic properties). For
daily use K is additionally stored wrapped under the ordinary wallet password, exactly
like every other secret this wallet already protects that way; the 24 words exist purely
as the out-of-band recovery path. This does not weaken P5.6's opening line ("no 24-word
seed tied to one master key") — note keys remain independently generated, one per note,
undiscoverable from K or the 24 words alone. Recovery therefore needs **both** the words
*and* the files: an encrypted vault copy on fully untrusted storage (cloud, a found USB
drive) is safe without the words; the words alone recover nothing, since bearer note keys
aren't derivable. Stated as a strength, not just a caveat — but the wallet UX must say it
loudly, since users trained on HD wallets will assume the words alone are sufficient.

**Manifest**: an optional plaintext companion file — `(serial, value, last-rotated-at)`
per note, nothing else — riding alongside the encrypted vault copy for human and tooling
legibility. Requires a `last_rotated_at` field the current schema doesn't yet carry
(small addition alongside P7.6's implementation, not a design change).

**Two-tier verification, the light tier needing no secrets at all.** A serial's
`(denomination, pk)` binding is immutable for its life — rotation consumes a serial and
mints a new one, it never re-points an existing one — so "serial still exists in the
pool" is exactly equivalent to "note still unspent," and serials sit in plaintext
filenames/the manifest already. **Light verify**: check every manifest serial against
live pool state (`get_notes_by_serial`, any node) — zero decryption, zero secrets in
memory, no rotation, works even without the 24 words. Lets a user (or an automated
watchdog they've deliberately pointed at a manifest, accepting the inventory-leak
trade-off to that watchdog) confirm a backup's health without ever restoring. **Deep
verify**: additionally decrypt and re-derive each `pk` to confirm ciphertext integrity —
catches a corrupted file light verify can't — the mandatory first step of an actual
restore, not something a passive health check needs.

**Restore-time rotation: default on, explicitly overridable — reversing P5.6's original
"always rotate immediately" rule for restore specifically** (bearer *receive* keeps
rotating unconditionally — a different threat model, argued below). The original
always-rotate rule was written against the paper-backup threat model, where the password
may be printed on the same page — a leaked backup there *is* a leaked wallet, so
immediate rotation is the only reasonable default. The vault breaks that coupling: an
encrypted copy is safe on fully untrusted storage without the words, so "I restored from
a backup I know never left my control" (migrating to a new machine, wiping an old device)
no longer needs the same urgency. But device *loss* is the more common restore trigger,
and a lost device carries the password-wrapped copy of K — exactly the case where prompt
rotation still matters — so the default stays on. Flow: **deep-verify against live chain
first (report: N notes still live, M already gone) → offer a batched, randomly-spaced,
randomly-composed rotation (2-5 transactions, mixed denominations per batch — sorted-by-
value batches would leak structure the mixing is meant to hide) → user may accept
(default), defer, or decline → nag while deferred (notes remain fully spendable
meanwhile — Hot is an urgency flag, not a lock) → prompt for a fresh backup copy the
moment rotation completes**, since the whole point was invalidating the old one. The
dialog states both sides plainly: rotating invalidates every old backup copy including
any stolen one; deferring keeps old backups valid including any stolen one.

**Accepted trade-off, stated openly**: batching/spacing *reduces* the "entire wealth
rotated at one timestamp" fingerprint, it does not eliminate linkage — each batch's own
consumed-serials list is still an explicit on-chain link, and a patient observer
correlating rotation-shaped transactions across the spacing window can still cluster
them. This is a genuine improvement over one all-at-once sweep, not a privacy guarantee;
documented as such rather than oversold.

**Implementation note — this is the existing full self-sweep, not new machinery.** P5.6
already names "rotate everything" a deliberate full self-sweep ("a real recovery action,
not just hygiene") for exactly this revocation purpose. Restore-time rotation should be
built as that same `sweep` primitive with a confirmation dialog in front, giving the
wallet a standalone panic button ("I think my backup leaked") for free alongside the
restore flow, not a second implementation of the same idea.
