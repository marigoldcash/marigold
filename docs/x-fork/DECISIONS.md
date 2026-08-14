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
| Emission curve | Smooth geometric decay from genesis: reward halves every **3 years** via monthly steps (monthly factor 2^(−1/36)); **no pre-deflationary phase** | No cliff moments ever — fee share grows as subsidy fades. ~20.6% of supply mined in year 1 (Bitcoin-comparable front-loading; Kaspa's 1-yr halving shape would have mined ~50% in year 1 into a tiny launch hashrate — stealth-premine optics), ~90% by year 10, per-block reward quantizes below 1 petal around **year ~72**, so subsidy outlives the P5.8 finality-anchor sunset by decades. Deflationary from genesis is also the simplest P3.2 implementation. | 2026-08-14 |
| Initial block reward | ≈ **1.5228 MAGLD/sec** (≈ 0.15228 MAGLD/block at 10 BPS); exact petal value fixed by the P3.2 generator so total emission ≤ cap | Derived, not independently chosen: cap ÷ Σ(monthly decay series) = 210,000,000 ÷ ~137.9M-seconds-equivalent. P3.2's generator computes the exact table and asserts the cap. | 2026-08-14 |
| Security endgame posture | Three-phase: **anchors guard youth → emission guards middle age → circulation fees guard maturity** | Marigold is a circulation coin: every payment is an on-chain rotate op paying a fee, so a *successful* cash economy is a permanent fee base — unlike store-of-value coins whose activity (and fee revenue) dries up at maturity. The ~72-year smooth subsidy runway is the bridge to that fee-funded maturity. This is the bet, stated openly. | 2026-08-14 |
| Launch allocation | **Fair launch from zero** — no premine, no dev fund, no airdrop; every MAGLD enters circulation via the P1.4 emission schedule from block 0 | Matches the plan's own defensible-zone guidance ("honesty + large premine is a hard sell") at the clean end of the spectrum; strengthens P1.9's regulatory posture and P9.6 legal review (no allocation to a founding entity to justify); mirrors Kaspa's own no-premine launch, which the project already inherits credibility from by forking. No vesting/governance question to resolve since there's no allocation to vest. | 2026-08-14 |
| Pool denominations | Powers of ten, whole coins: **{0.01, 0.1, 1, 10, 100, 1000, 10000, 100000}** (8 tiers) | Extends the plan's recommended set upward by two tiers (10000, 100000). Since split/merge is always available at an exact 10× factor (per the architecture), adding large denominations costs nothing at the small end — no fragmentation of the anonymity sets for everyday-payment sizes — while saving large holders from managing piles of 1000-notes to represent one big balance which is worse for usability. Also a wallet holding e.g. 50× 1000-notes leaks an approximate balance by note count / UTXO-style clustering that one 100000-note or a few don't. Floor stays at 0.01 (the P5.6/P8.3 smallest-denomination discussion — spam/floor pricing — is unaffected, only the ceiling moved). | 2026-08-14 |
| Transparent tier policy | Confirmed as-is: fork launches **transparent-only** (Phases 2-4), pool added later (Phases 5-7) | No change from the plan's existing phase structure — confirming it here just makes it an explicit recorded decision rather than an implicit one baked into the phase ordering. | 2026-08-14 |
| Fee destination | **The including block's miner**, via normal coinbase fee accounting — never burned, no dev fund | Per-block collection is statistically hashrate-proportional at 10 BPS (a miner with X% of hashrate collects ~X% of fees), which was the stated goal, with zero new machinery. A fee-funded dev fund was considered and rejected: it would reverse P1.5's fair-launch decision through the side door, convert the legal posture from "published software" to "operates a paid service" and drain the P1.4 endgame mechanism (circulation fees fund security). A community treasury remains possible later only via explicit DAO-voted hard fork (same pattern as the tail-emission reserve option). Supply cap unaffected: fees are recycled value, not emission. | 2026-08-14 |
| Fee policy | **Inherit Kaspa's fee model** (near-zero, mass-based) for the transparent tier. Pool ops pay with **fee stamps**: whole small-denomination notes consumed inside the op (an embedded redeem-with-no-transparent-output — the stamp's value becomes the miner fee through Kaspa's native value-in-minus-value-out mechanism). Unified conservation rule across all ops: Σ(note in) + Σ(transparent in) = Σ(note out) + Σ(transparent out) + fee. **REVISED same-day**: supersedes the earlier "small transparent-value input from a wallet-held fee reserve" answer, which violated the core no-wallet principle (it would have required every wallet to hold a transparent balance with an address — reintroducing exactly what the design removes). See "P1.8 — pool-op fee mechanism (revised: fee stamps)" below. | Zero-fee + reward-per-action was already ruled out as a spam vector. Notes are fixed-denomination (can't be fractionally shaved without destroying the anonymity set), so fees must be whole small notes. Stamps keep the wallet a pure key manager (a stamp is just another note key), put no transparent address on any pool op (mint/redeem return to being the only transparent touchpoints, as originally designed), need zero new miner-payment machinery, and still support congestion pricing (attach more stamps = higher fee/mass priority — a denomination-quantized fee market). | 2026-08-14 |

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
