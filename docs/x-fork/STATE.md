# Project state — session handoff

Snapshot of everything decided and built so far, so any fresh coding session on any
machine can continue from the repo alone. Read this together with [FORK-PLAN.md](../../FORK-PLAN.md).
Update this file whenever off-repo state changes (domains, accounts, infra).

Last updated: 2026-08-15 (Phase 5 complete — spec tagged pool-spec-v1.1, P5.9 closed, Phase 6 unblocked)

## What this project is

**Marigold** — a transparent (non-ZK) fixed-denomination bearer-note "digital cash" coin,
forked from rusty-kaspa. Full roadmap and all design decisions: [FORK-PLAN.md](../../FORK-PLAN.md)
(phases P0–P9 + future governance; work strictly by step IDs; "X Coin" naming inside it is
the historical working title, kept deliberately).

## Repo layout

- Private GitHub repo **marigoldcash/marigold-node** = `origin`. Private during Phases 0–4,
  public at P5.9/Phase 6 (ground rule 5).
- `master` mirrors upstream kaspanet/rusty-kaspa (remote `upstream`). Never commit fork work to it.
- All fork work on branch **`x-fork`**.
- GitHub account with org access: `leancode`. Orgs: `marigoldcash` (primary so far) and
  `marigoldcoin` (defensive).

## Locked parameters (Phase 1 complete, 2026-08-14)

All ten Phase 1 decisions are recorded, with full rationale and rejected alternatives,
in [DECISIONS.md](DECISIONS.md) — this is a summary for quick reference, not a
substitute for it.

- **Name / ticker**: Marigold / **MAGLD** (not the originally pre-decided MGLD —
  collision found on execution-time recheck).
- **Prefixes**: `marigold` / `marigoldtest`. **Domain**: marigold.cash.
- **Units**: 8 decimals, base unit **petal** (1 marigold = 10⁸ petals).
- **Supply**: 210,000,000 MAGLD hard cap, no tail emission. Smooth geometric decay,
  halving every 3 years from genesis, ~20.6%/yr1, ~90%/yr10, fades below 1 petal
  ~year 72. Initial reward ≈1.5228 MAGLD/sec (exact value fixed by P3.2).
  Security-endgame posture: anchors → emission → circulation fees.
- **Launch**: fair launch from zero — no premine, no dev fund, no airdrop.
- **Pool denominations**: {0.01, 0.1, 1, 10, 100, 1000, 10000, 100000}, 8 tiers
  (extended above the plan's recommendation since split/merge at exact 10× makes
  extra headroom free).
- **Transparent tier**: confirmed as-is — transparent-only Phases 2-4, pool in 5-7.
- **Fees**: inherit Kaspa's fee model for the transparent tier. Pool ops (rotate/
  split/merge) pay with **fee stamps** — whole small-denomination notes consumed
  in-op via an embedded redeem-with-no-transparent-output, so no wallet ever holds
  transparent value or an address (a same-day-rejected earlier design would have
  required a transparent "fee reserve" balance — ruled out as violating the core
  no-wallet principle). Stamp value goes to the including block's miner (never
  burned, never a dev fund) — this is a **major, non-negotiable design constraint**
  going forward: any future step touching fees/wallets must preserve "the wallet
  holds nothing but note keys."
- **Regulatory posture**: no legal entity for now (Swiss-style foundation the leading
  candidate if one's ever needed); distribution expected to depend on CEX's willingness
  to accept the coin and DEXs/P2P.
- **Network ports**: mirror Kaspa's structure shifted to 26xxx/27xxx/28xxx (gRPC
  26110/26210, borsh-wRPC 27110/27210, JSON-wRPC 28110/28210, P2P 26111/26211 —
  mainnet/testnet; simnet/devnet follow the same +X00 pattern). Chosen over reusing
  Kaspa's ports so a Kaspa node and a Marigold node can run on one host at once.

## Live infrastructure (as of 2026-08-14)

- **Website**: https://marigold.cash — GitHub Pages, repo `marigoldcash/marigoldcash.github.io`,
  HTTPS enforced, RFC 9116 security.txt (Contact: security@marigold.cash, Expires 2027-08-13 —
  renew annually alongside domains).
- **Domains** (all Cloudflare, auto-renew, expiry 2027-08-13): canonical marigold.cash;
  301-redirecting to it: marigoldcoin .com/.io/.net/.org, marigoldcash .com/.net/.org,
  marigold-coin.com, marigold-cash.com, marigoldwallet.com. All redirect domains have
  SPF `-all` + DMARC `p=reject` + null MX (spoofing lockdown).
- **Email**: Cloudflare Email Routing on marigold.cash (SPF+DKIM present).
  Open items: `_dmarc` record on marigold.cash; create `security@` and `dmarc@` routes.
- Open item: social handles (`marigoldcash` + `marigoldcoin` on X, Telegram, Discord,
  Reddit, YouTube, Docker Hub, npm). marigoldcash.io deliberately not registered (revisit P9).
- **Future (P9.2)**: DNS seeders will be ≥2 independent VPS instances running Kaspa's
  `dnsseeder` software, reached via an NS delegation record per seeder subdomain
  (e.g. `seed1.marigold.cash`) created in this Cloudflare account — Cloudflare stays
  registrar/parent zone, the seeder software runs elsewhere. Mechanism detail in
  NOTES.md's P2.4 entry. Nothing to provision yet.

## Where execution stands

- **Next step: P6.1 — start of Phase 6 (Pool: consensus implementation ⚠️).** Note
  types & wire encoding in `consensus/core`: `Note`, the op enum, borsh serialization,
  the dedicated subnetwork ID — pure data, no validation logic yet. Verify: round-trip
  encode/decode tests including maximum-size instances; byte sizes match the spec's
  P5.1/P5.2 tables. Implement against **`pool-spec-v1.1`** (the tag, not memory).
- **Phase 5 (P5.1-P5.9) is fully done — the spec survived external review.**
  [POOL-SPEC.md](POOL-SPEC.md) is tagged **`pool-spec-v1.1`** after a full review
  cycle: two independent external reviews, a cross-review concurrence, and a
  confirmation pass, all filed with finding-by-finding triages under
  [reviews/](reviews/). Review 1 caught one real vulnerability (Redeem
  transaction-malleability — pool-op signatures didn't cover transparent outputs; now
  covered uniformly) and sharpened the equivocation rule (now exactly decidable via
  DAA-keyed cadence). Review 2 (O'Connell) found no new flaw and demanded the rigor
  layer, now in the spec: authorization theorem + proof sketch, complete
  signed/unsigned field matrix, explicit bearerability threat model ("signed = spent"
  as a binding wallet rule), full canonicalization, version+op-type domain separation,
  invariants I1-I5, prominent trust-boundary statement, IBD anchor-ratchet, complete
  equivocation-evidence lifecycle. Reviewer 1's confirmation pass on the updated spec
  closed the gate: "none of these should block tagging v1.1 and closing P5.9."
  **Standing obligations carried forward**: consumed-group-set malleability (`[Open]`
  in the P5.2 field matrix — Phase 6 standing review item; assessed "needs sign-off,
  not redesign"); gas-semantics confirmation (Phase 6); T/M/K quantitative sensitivity
  model (P9.5 hard gate — the 10⁶ multiplier is explicitly not final without it);
  trustee-independence criteria + one-live-signer rule (P9.1); wallet recovery drills
  (P8.5); reviewer test matrices → Phase 6 conformance tests. Any later reviewer-2
  feedback on the updated draft folds into a v1.2 via the same triage process.
- **Phase 4 (P4.1-P4.4) is fully done — "the fork works," tagged and reproducible.**
  Tag `fork-transparent-v0.1` exists on `origin`, and the fresh-clone claim was
  actually tested, not assumed: a genuinely clean `git clone --branch
  fork-transparent-v0.1` (zero prior build artifacts) auto-built `kaspad` from
  scratch and got all 3 testnet nodes peered on the first try (P4.4).
  [scripts/x-testnet-local.sh](../../scripts/x-testnet-local.sh)/`.ps1` (P4.1)
  launch 3 peered devnet nodes on one machine — verified live twice now (12 mined
  blocks relayed to all peers, block count/DAA score/sink hash matching exactly).
  **The `.ps1` script itself remains unverified** — no Windows environment was
  available this session. [docs/x-fork/SMOKE.md](SMOKE.md) (P4.2) walked a full
  user-journey (wallet creation → mining → maturity → send → restart → balance
  persistence) live and confirmed every balance matched exactly across a node
  restart; found and fixed two real bugs along the way in `rothschild`
  (stale-binary trap, same root cause as P2.3's; and a genuine incompatibility
  where its hardcoded 10-KAS-equivalent default send amount could never be
  satisfied from Marigold's much smaller genesis-era coinbase UTXOs — fixed by
  scaling it to 1 MAGLD-equivalent). The integration test suite is green (P4.3,
  `cargo-nextest` now installed) — nothing needed fixing there, every
  hardcoded-Kaspa-params issue this crate had was already caught by earlier steps.
  Full detail in NOTES.md's P4.1-P4.4 entries and DECISIONS.md where relevant.
- **Phase 3 (P3.1-P3.3) is fully done.** The real emission mechanism was understood
  (P3.1), replaced with Marigold's own P1.4 schedule (P3.2), and confirmed correct
  against a real running node over RPC (P3.3). `SUBSIDY_BY_MONTH_TABLE` is now a
  1016-month table (base ≈1.5228084263 MAGLD/sec, 3-year/36-month halving, tapers to
  an exact 0 at ~84.6 years), enforced under the 210,000,000 MAGLD cap by a permanent
  test (`total_emission_stays_under_cap`). Mainnet/testnet/devnet have no
  pre-deflationary phase (decay from block 0, per P1.5); **simnet is the one
  exception** — it deliberately keeps a real flat pre-deflationary phase, since it's
  an internal PoW-skipped benchmark harness (not a real network) and a real
  integration test depends on that flat subsidy. P3.3's live devnet run (1097 real
  mined blocks) confirmed circulating supply matches the table's month-0 value with
  **zero deviation** (16,705,209,245 petals = exactly 1097 × 15,228,085), and caught
  one more real bug: `get_coin_supply`'s `max_sompi` field (and the same
  `MAX_SOMPI` constant's tx-output/tx-total sanity bound) was still real Kaspa's
  actual max supply — fixed to Marigold's own 210,000,000 MAGLD cap. Four bugs total
  found and fixed across P3.2/P3.3 via full-workspace + `--ignored`-test + live-RPC
  verification (a stale test literal, a latent P2.2-era BPS-assumption bug in a
  never-actually-run ignored test, five `goref_*` tests that replay real historical
  Kaspa chain data now correctly marked `#[ignore]` rather than fixed since that
  scenario can never validate again on a from-scratch economics schedule, and the
  `MAX_SOMPI` mismatch). Full numbers and rationale in [DECISIONS.md](DECISIONS.md)'s
  "P3.2" section and [NOTES.md](NOTES.md)'s P3.1/P3.2/P3.3 entries.
- **Phase 2 (P2.1-P2.9) is fully done**, including the closing smoke test: two
  independently-started devnet nodes (separate appdirs/ports, `--addpeer`-linked)
  converged to byte-identical DAG state — same block count, virtual DAA score, tip
  hash, and sink — purely via the P2P layer this phase rebuilt. Full log evidence in
  NOTES.md's P2.9 entry.
- This fork is now a genuinely separate,
  fully-rebranded network end-to-end: own address prefixes, ports, P2P handshake
  name (confirmed live against a real Kaspa mainnet peer — explicit rejection), no
  Kaspa DNS seeders, own from-scratch genesis (mainnet motto: *"Hell is other
  people's monetary policy. — Sartre"*; testnet: `marigold-testnet`; **P9.5
  regenerates both with the real launch timestamp — today's are placeholders**), all
  forks (10 BPS, covenants, ZK opcodes) active from block 0, and every user-facing
  string rebranded across the node, wallet, and CLI (app dir, log files, `--help`
  banner, ticker `MAGLD`, account storage tags, terminal link matcher + explorer
  URLs). Binary/crate name `kaspad` and a few other identifier-not-display-string
  cases (WASM API surface, `kaspa_utils` paths) deliberately kept, per Ground rule 1.
  Own DNS seeders remain a P9.2 future item (see "Live infrastructure" above for the
  Cloudflare-delegation mechanism).
- **The GitHub issue for kaspanet/rusty-kaspa has been posted** (by the user,
  manually, from the prepared draft) — reporting the `TestConsensus` block-version
  test-infra gap found at P2.6 as a potential upstream contribution.
- **New plan step added: P7.0 — Inherited-wallet surface audit** (🧑‍⚖️ DECISION, head
  of Phase 7). Investigation during P2.8 confirmed the legacy-Kaspa wallet-import
  code (`compat/gen0.rs`/KDX-format, `compat/gen1.rs`/Go-`kaspawallet`-format) is
  live and reachable via real CLI commands (`import legacy`, `account import
  legacy-data`), not dormant — and the user confirmed KDX is deprecated and won't be
  used by Marigold. Decision: this import surface must be removed or hard-disabled
  before any binary reaches outside users (deadline: P8.7 at the latest; P8.5's
  wallet threat pass re-verifies closure). Deliberately not acted on yet — deferred
  to one coherent Phase-7 decision about the whole inherited wallet stack rather than
  piecemeal deletion now, given P4.2's smoke-test dependency and upstream-merge
  friction economics. Full rationale in DECISIONS.md.
- **Six real bugs found and fixed during Phase 2** (all stale real-Kaspa
  legacy values/assumptions left inconsistent with a from-scratch, rebranded chain —
  full root-cause writeups in NOTES.md, most as their own separate commits per
  Ground rule 2): `get_chain_block_samples()`'s hardcoded 2021 checkpoint timestamps
  (P2.5); P2.6's activation flip exposing 5 stale-legacy-value test failures plus
  the `TestConsensus` block-version test-infra gap; the P2.1 address-prefix
  regression recurring in `crypto/txscript` (found via a full-workspace test pass
  during P2.8); a second P2.6-pattern hardcoded-block-version assertion in
  `testing/integration`; and a genuine `bridge/` (stratum-bridge) address-validation
  bug where the wallet-address regex/fallback-prefix logic still expected `kaspa:`.
  **The P2.1 `kaspa-wallet-core` regression (22 failing tests) is now fixed** —
  folded into P2.8 per the user's request, using a self-verified bech32
  re-prefixing tool (see NOTES.md) that correctly preserves exact payloads for
  addresses tied to fixed derivation seeds, rather than generating arbitrary fresh
  ones.
  **Standing lesson, worth repeating**: a full-workspace `cargo build`/`cargo test`
  pass — not just the crate(s) a step names — has now caught real regressions in
  unrelated crates multiple times this phase (`kaspa-wallet-core`, `crypto/txscript`,
  `testing/integration`, `bridge/`). Keep doing this periodically, not just when a
  step's own verify command happens to be narrow.
  **Full gotcha log for all of Phase 2 is in [NOTES.md](NOTES.md)** — worth
  skimming before Phase 3: always rebuild `kaspad` before a live-network test (a
  stale binary gave a false pass once), use `--appdir=<scratch>` for mainnet-mode
  testing (a pre-existing unrelated real mainnet datadir exists on this machine,
  untouched).
- **Phases 0 and 1 are both complete.** Phase 0: P0.1-P0.6, see
  [NOTES.md](NOTES.md) — the single source of truth for build/test/devnet/wallet
  commands and gotchas (notably: `kaspa-cli` is REPL-only and unscriptable, P0.5 was
  done via RPC/rothschild instead). Phase 1: P1.1-P1.10, see "Locked parameters"
  above and [DECISIONS.md](DECISIONS.md) for full rationale — **the fee-stamp
  mechanism and the "wallet holds nothing but note keys" constraint are load-bearing
  for everything from here on**, worth re-reading before any step touching wallets,
  fees, or transaction format (P5.2 onward).
- wasm-pack / wasm32 target only needed for WASM SDK steps, not for `kaspad`.
- Phases 0–4 are bite-size sessions by design; use stronger models for Phase 5 spec
  work, ⚠️ HARD steps (P6.4, P6.11), and review gates.

## Suggested session opener on a new machine

> Read FORK-PLAN.md and docs/x-fork/STATE.md, then do step P0.1.
