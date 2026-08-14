# Project state — session handoff

Snapshot of everything decided and built so far, so any fresh coding session on any
machine can continue from the repo alone. Read this together with [FORK-PLAN.md](../../FORK-PLAN.md).
Update this file whenever off-repo state changes (domains, accounts, infra).

Last updated: 2026-08-14 (P0.5 complete)

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

## Naming (pre-filled P1.2/P1.3 — formalize in DECISIONS.md when Phase 1 runs)

Coin **Marigold**, ticker **MGLD** (provisional — re-verify unclaimed), prefixes
`marigold`/`marigoldtest`, base unit **petal** (1 marigold = 10⁸ petals).
Canonical domain **marigold.cash**.

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

- **Next step: P0.6** (write the orientation notes file — mostly already satisfied by
  the running [NOTES.md](NOTES.md) built up across P0.1-P0.5; just review/tidy it).
  P0.1-P0.5 are all done on Linux (LMDE/Debian). P0.5 was executed via RPC instead of
  the literal `kaspa-cli` wallet flow — see NOTES.md and the FORK-PLAN.md P0.5 entry for
  the rationale (kaspa-cli is REPL-only and unscriptable, and its backing
  `kaspa-wallet-core` seed/key-DB layer is exactly what this project's wallet redesign
  replaces, so it wasn't worth proving out). See [NOTES.md](NOTES.md) for full details:
  test baseline (0 failed, 26 pre-existing ignored), devnet ports/app-dir, the
  `kaspa-cli` REPL gotcha, generating throwaway devnet addresses without a wallet,
  `kaspa-miner` invocation, and using the built-in `rothschild` tx generator (note its
  2×-coinbase-maturity requirement and that confirmed balance needs a block mined
  *after* the tx, not just mempool submission). Windows build was abandoned (MSVC Build
  Tools installer failures) — development moved to Linux. Linux prereqs:
  `sudo apt install -y curl git build-essential libssl-dev pkg-config protobuf-compiler libprotobuf-dev clang libclang-dev`
  then rustup (stable, ≥1.91). No Windows AR.exe/LIBCLANG quirks apply on Linux.
- wasm-pack / wasm32 target only needed for WASM SDK steps, not for `kaspad`.
- Phases 0–4 are bite-size sessions by design; use stronger models for Phase 5 spec
  work, ⚠️ HARD steps (P6.4, P6.11), and review gates.

## Suggested session opener on a new machine

> Read FORK-PLAN.md and docs/x-fork/STATE.md, then do step P0.1.
