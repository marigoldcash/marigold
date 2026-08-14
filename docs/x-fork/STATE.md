# Project state — session handoff

Snapshot of everything decided and built so far, so any fresh coding session on any
machine can continue from the repo alone. Read this together with [FORK-PLAN.md](../../FORK-PLAN.md).
Update this file whenever off-repo state changes (domains, accounts, infra).

Last updated: 2026-08-14 (P2.8 complete)

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

- **Next step: P2.9 — the last step of Phase 2.** Two-node private network smoke
  test: start two local nodes with `--addpeer` pointing at each other (different
  appdirs/ports), mine on one. Verify: the second node's log shows it syncing blocks
  mined by the first, and both report the same virtual DAA score via RPC.
- **Phase 2 (P2.1-P2.8) is done.** This fork is now a genuinely separate,
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
- **A draft GitHub issue for kaspanet/rusty-kaspa is sitting unposted** — reporting
  the `TestConsensus` block-version test-infra gap found at P2.6 as a potential
  upstream contribution; awaiting the user's go-ahead to actually post it (posting
  to a third-party public repo needs explicit confirmation).
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
