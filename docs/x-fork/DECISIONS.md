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
