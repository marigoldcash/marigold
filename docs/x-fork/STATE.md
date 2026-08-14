# Project state — session handoff

Snapshot of everything decided and built so far, so any fresh coding session on any
machine can continue from the repo alone. Read this together with [FORK-PLAN.md](../../FORK-PLAN.md).
Update this file whenever off-repo state changes (domains, accounts, infra).

Last updated: 2026-08-14 (P1.6 complete)

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

## Naming

Coin **Marigold**, ticker **MAGLD** (formalized P1.2, 2026-08-14 — changed from the
originally pre-decided MGLD after a collision check found MGLD, and later-considered
MCASH, both had prior-project overlaps; full trail in
[DECISIONS.md](DECISIONS.md)), prefixes `marigold`/`marigoldtest`, 8 decimals, base
unit **petal** (1 marigold = 10⁸ petals — formalized P1.3, 2026-08-14, no changes from
pre-decision). Canonical domain **marigold.cash**.

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

## Where execution stands

- **Next step: P1.7** (transparent tier policy — the plan already states the fork
  launches transparent-only in Phases 2-4 and adds the pool in Phases 5-7; likely just
  needs confirming, not a fresh decision). P1.1-P1.6 are done — see DECISIONS.md for
  the ticker collision trail, the P1.4 emission math (210M cap, 3-yr halving, no tail,
  ≈1.5228 MAGLD/s initial — plus a flag that P5.2 must spec how pool ops pay fees),
  P1.5 (fair launch from zero), and P1.6 (denominations extended to
  {0.01...100000}, 8 tiers). Phase 0
  (environment & orientation) is complete: P0.1-P0.6 all done on Linux, LMDE/Debian.
  For environment setup, working commands, and every build/test/devnet/wallet gotcha hit
  along the way, **read [NOTES.md](NOTES.md) first** — it's the orientation doc Phase 0
  exists to produce, kept current as the single source of truth for "how do I actually
  run this." Don't duplicate its content here; update it (not this file) when new
  build/run findings turn up.
- P0.5 was executed via RPC instead of the literal `kaspa-cli` wallet flow — see the
  FORK-PLAN.md P0.5 entry and NOTES.md for the rationale (kaspa-cli is REPL-only and
  unscriptable, and its backing `kaspa-wallet-core` seed/key-DB layer is exactly what
  this project's wallet redesign replaces, so it wasn't worth proving out).
- wasm-pack / wasm32 target only needed for WASM SDK steps, not for `kaspad`.
- Phases 0–4 are bite-size sessions by design; use stronger models for Phase 5 spec
  work, ⚠️ HARD steps (P6.4, P6.11), and review gates.

## Suggested session opener on a new machine

> Read FORK-PLAN.md and docs/x-fork/STATE.md, then do step P0.1.
