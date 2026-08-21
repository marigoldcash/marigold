# Launch plan — testnet standup, publication, and the road to mainnet

The single place where the technical track (FORK-PLAN P8/P9) and everything around it — whitepaper, website, explorer, announcements, community — come together as one sequenced program. FORK-PLAN.md remains the authority on the numbered engineering steps; this document adds the execution checklist for the immediate testnet standup, the publication gate, and the entire communications/ecosystem workstream that the plan previously covered only as a bullet inside P9.7. Items marked 🧑‍⚖️ need the founder's decision; recommendations are given for each.

Drafted 2026-08-20. Status of each item lives here; decisions, once made, get DECISIONS.md rows as usual.

## Part 1 — Testnet standup (the immediate checklist)

Everything below is executable now; nothing waits on anything outside this list. Reference runbook: [TESTNET.md](TESTNET.md). Target: a public, joinable testnet with blocks flowing, within days not weeks.

- [ ] **T1 — Deploy method decision.** 🧑‍⚖️ Recommendation: **native Ansible for our own 3 Hetzner hosts now** (the playbook is finished and validated; the docker-compose harness is deliberately local-only and production-hardening it — host networking, the Docker/iptables-vs-manual-firewall interaction — is real extra work), with **Docker images published for third parties** at repo-publication time (T10). The half-finished multi-instance Ansible work (uncommitted `group_vars` changes) is not needed for standup — all three hosts are testnet-only until mainnet exists — park it until the mainnet-coexistence phase.
- [ ] **T2 — Provision the 3 Hetzner hosts.** `inventory.ini` from the example with real IPs; bootstrap play (deploy user), build play on the big rig, deploy plays for `marigoldd` (all 3) + `stratum-bridge` (big + medium). Manually open `26211/tcp` on all three and `5555/tcp` on the two bridge hosts (manual-firewall decision, STATE.md 2026-08-20 — `ufw_manage` stays unset). ✅ *Verify:* `systemctl status marigoldd` green on all three; gRPC confirmed loopback-only from outside (`nc -zv <ip> 26210` fails).
- [ ] **T3 — DNS seed records.** Three DNS-only (NOT proxied) `A` records in Cloudflare: `tn-seed1/2/3.marigold.cash` → the fixed IPs. The hostnames are already committed in `TESTNET_PARAMS.dns_seeders`, so this is purely a Cloudflare-dashboard task. ✅ *Verify:* `dig +short tn-seed1.marigold.cash` from an unrelated network returns the right IP; a fresh node with no `--addpeer` finds peers.
- [ ] **T4 — Point the ASICs.** ASIC #1 → big rig `:5555`, ASIC #2 → medium rig `:5555` (over the internet from home/Starlink — outbound only, no home-side config). ✅ *Verify:* bridge logs show accepted shares from both; blocks land `via submit block` and relay to the other nodes.
- [ ] **T5 — Testnet CPU miner on the 8-core.** Decided earlier (2026-08-20): runs now, not at mainnet cutover. Fastest path given T1: run the patched `marigold-miner` docker image on the medium rig (`docker run` against loopback gRPC), or screen/systemd the bare binary; the unfinished Ansible `cpu-miner` role can formalize it later. ✅ *Verify:* continuous block production survives both ASICs being pointed away for an hour.
- [ ] **T6 — Monitoring.** Prometheus scrape of the bridges (`:2114`, loopback — scrape locally, push or tunnel out) + node perf metrics; a simple Grafana on the big rig; alert on: no new blocks N minutes, node process down, peer count zero. P8.7's incident log starts the day this is live. ✅ *Verify:* kill a node, alert fires.
- [ ] **T7 — Faucet (the one genuinely new build).** Smallest viable: a single small service (axum + wallet-core, or even the CLI driven by a wrapper) holding a funded testnet wallet, one endpoint: address in → small MAGLD out; per-IP + per-address rate limit; static page front end on the site. Runs on the big rig. No captcha initially (testnet coins, rate limits suffice); revisit if farmed. ✅ *Verify:* a stranger with only the website can fund a wallet and send a transaction.
- [ ] **T8 — Miner/tester onboarding docs.** A "Join the testnet" page (website + repo): binaries or docker image, `--testnet` quickstart, faucet link, MINING-COMPAT.md pointer for anyone porting tools, where to report issues. This is P9.4's "third party can mine unassisted" condition, pulled forward. ✅ *Verify:* someone who isn't the founder follows it cold and succeeds.

## Part 2 — Publication gate (repo goes public)

Trigger per your own ground rule 5: overdue already; per your stated intent, it happens **as soon as the testnet runs properly and the deploy systems are in place** — i.e. after T1–T8.

- [ ] **T8b — Default branch & repo presentation.** 🧑‍⚖️ Decided need (founder, 2026-08-20): visitors to the public repo must land on the actual Marigold code, not the upstream mirror. Recommendation: **rename `x-fork` → `main` and make it the default branch**; keep `master` as the upstream mirror under its conventional name; add one branch-layout sentence to the README's "Relationship to Kaspa" section. Rename sweep: `marigold_ref` in `deploy/ansible/group_vars/all.yml`, CI workflow branch triggers, STATE.md's repo-layout section, FORK-PLAN ground rule 5, plus `git branch -m x-fork main` on the founder's other machines. Rationale for renaming rather than just re-pointing the default: "x-fork" is the historical working-title name and reads as cryptic to every first-time visitor; pre-publication is the cheap rename moment (same logic as the marigoldd rename). ✅ *Verify:* an incognito visit to the public repo lands on Marigold's README on `main`; `git merge upstream/master` into `main` still works (mechanics unchanged).
- [ ] **T9 — Pre-publication sweep, final pass.** The STATE.md open-items list, closed out: README (done), repo terminology (done), AI-disclosure posture (decided — publish as-is), **website repo terminology sweep (still open)**, plus a fresh secrets scan of the full history (nothing sensitive was ever committed by convention — `inventory.ini` and `.fetched/` are gitignored — but verify, don't assume: `gitleaks` or equivalent over all branches). 🧑‍⚖️ STATE item 4 explicitly says the list may be incomplete — **enumerate any remaining "several points" now or bless the list as complete.**
- [ ] **T10 — Publish, all at once.** Make `marigoldcash/marigold-node` **and** `marigoldcash/marigold-miner` public together (the compose file references the miner repo; publishing one without the other breaks the out-of-box experience). Drop the `GITHUB_TOKEN` build secret from `Dockerfile.cpu-miner`/compose. Push images to a registry. 🧑‍⚖️ Registry: recommendation **GHCR** (`ghcr.io/marigoldcash/marigoldd`, `stratum-bridge`, `marigold-miner`) — org-tied, free, trivially automated from Actions; *also* register the `marigoldcash` name on Docker Hub immediately (defensive, like the domains) even if images live on GHCR. Tag binaries: a `testnet` pre-release with checksummed Linux binaries (the deploy.yaml workflow already builds these). ✅ *Verify:* `docker run ghcr.io/marigoldcash/marigoldd --testnet` joins the testnet from a machine we don't control.

## Part 3 — The communications & ecosystem workstream

Runs in parallel with the P8 soak (which needs months of calendar time anyway — this is what the calendar time is *for*).

### 3.1 Whitepaper

The core insight to build it around, per the founder (2026-08-20): **the note pool is chain-agnostic — the value proposition is notes, not Kaspa.** The paper is about transparent bearer notes as a concept; the host chain is a deliberate engineering choice, one section, not the identity. There is already an externally-reviewed formal spec (POOL-SPEC v1.1) — the whitepaper is its readable distillation plus motivation and honest positioning, not a new normative document (it should say so and cite the spec).

Proposed outline:

1. **Motivation** — what physical cash does that digital money doesn't; the two existing answers (transparent chains: no fungibility; ZK privacy coins: complexity you can't personally verify, regulatory categorization) and the third path: *never record* the linkage instead of *hiding* it.
2. **Transparent bearer notes** — serials, fixed denominations, ownership = knowing the key; rotate, split (×10), merge; everything plaintext on chain.
3. **Value conservation** — the one rule (`Σ notes + Σ ledger = emitted`), fee stamps, why the miner never touches a note.
4. **Wallets without seed phrases** — bearer model, handover flows (QR, in-person, POS), the vault ("your files and your key").
5. **What is and is not private** — the P5.7 honesty section, elevated to a selling point: graph visibility, anonymity set = denomination cohort, no claim stronger than the design delivers. This section is the brand.
6. **Host-chain requirements** — what the concept needs from *any* chain (fast confirmation for handover UX, pruning, mature P2P/consensus) and why a 10 BPS GHOSTDAG chain was chosen: sub-second rotation confirmation is what makes bearer handover feel like handing over cash. Credit Kaspa plainly; keep the framing "notes hosted on," never "a Kaspa fork with extras."
7. **Economics** — 210M cap, smooth decay, fee-funded endgame.
8. **Launch security** — finality anchors, trustee sunset, stated openly.
9. **Related work** — physical cash, Chaumian e-cash (the intellectual ancestor — same bearer idea, but ours needs no issuing mint), Monero/Zcash, plain transparent chains.
10. **Future** — note-weighted governance (the Future-work section's Route C).

Practicalities: 8–12 pages; Typst or LaTeX → PDF, plus an HTML rendering on the site; lives in the repo (`whitepaper/`) so it's versioned and citable; published **before or with the first broad announcement** (the announcement links to it — it's the artifact that makes the project legible). 🧑‍⚖️ Decide: external review of the paper by the P5.9 reviewers before publishing (recommended — cheap, they know the spec, and "reviewed by the same cryptographers who reviewed the spec" strengthens it).

**Status 2026-08-20: draft v0.2 — [`whitepaper/marigold-whitepaper.md`](../../whitepaper/marigold-whitepaper.md)** (11 sections per the outline above; markdown master, renders on GitHub and feeds the website). Two founder review passes (serial lifecycle, two-senses-of-public) and the founding-inversion addition landed; **first external review round complete same day** — verdict "publishable work with genuine innovation," reviewer retracted their two substantive objections (fee-stamp "burn", liquidity example) in correspondence, six targeted additions adopted, followed by a precision pass responding to the reviewer's meta-point that the paper's compression occasionally undersold the spec's exactness. **Review round closed with the reviewer's final verdict: "This is ready"** — their one closing suggestion (a skimmer-proof one-line callout on fee stamps in Section 3) adopted. PDF toolchain done: [`whitepaper/build-pdf.sh`](../../whitepaper/build-pdf.sh) typesets reproducibly via the pandoc/extra docker image (zero local installs; generated PDFs gitignored). **Author line decided (founder, 2026-08-21): "The Marigold Project" — final.** Consistent with the fair-launch, no-personality posture: the system argues for itself, no individual byline. **The whitepaper track is complete** — remaining work is publication-day mechanics only (final version stamp, publish PDF alongside Moment A).

### 3.2 Website

marigold.cash exists (GitHub Pages). Needed: the terminology sweep (open item), then expansion from placeholder to launch-grade static site: landing with the three verifiable claims + the honest sentence; Get Started (join testnet / mine / faucet); whitepaper; docs links (MINING-COMPAT, wallet guide); FAQ (including the one matter-of-fact AI-disclosure sentence, per the decided posture); security.txt already live. Keep it static GH Pages — no backend to secure. Effort: small; content mostly exists in README/docs.

### 3.3 Explorer and visualizer

🧑‍⚖️ Two different things, different priorities:

- **Block explorer (needed for launch, P9.4):** recommendation: port the community Rust indexer stack (`supertypo/simply-kaspa-indexer` + REST server + `kaspa-explorer` frontend) — it speaks rusty-kaspa wRPC, so it's the same class of port as everything else we've done (prefix/ports/branding, plus eventually pool-op awareness). This is what miners and testers actually need: look up a block, a transaction, an address, supply.
- **KGI-style DAG visualizer (kgi.kaspad.net — marketing-grade, optional):** honest cost note: Kaspa Graph Inspector's ingest component embeds the **Go** kaspad node, which cannot speak our network (the `pool_commitment` header field changes P2P serialization, and the Go codebase knows nothing of it) — porting KGI means either patching Go kaspad minimally or rewriting its ingest against our wRPC. Real work, zero launch-critical value, high demo value. Classify: post-testnet nice-to-have, ideal community-contribution bounty once the repo is public.

### 3.4 Community channels (register now, before publication)

Status 2026-08-20 (founder): **have** — Discord, GitHub, Telegram, X. **Still needed**, in priority order:

- **Active-or-real use**: **Docker Hub** (defensive name even if images live on GHCR), **Reddit** (claim r/marigoldcash), **npm** (org name — the WASM SDK will want it eventually), **YouTube** (defensive now; later genuinely useful — a 60-second video of a QR bearer-note handover at a real POS is the single best marketing artifact this project could produce).
- **Facebook / Instagram / TikTok: register defensively, publish nothing.** No fair-launch-mining audience lives there, and an active retail-flavored presence on those platforms would *hurt* credibility with the audience that matters (it pattern-matches to pump projects). Squat the names like the domains; leave them empty.
- **Skip**: LinkedIn, Farcaster. **Optional if effortless**: Bluesky, Nostr.

Discord is the one channel needing actual daily attention (support hub — #announcements, #testnet, #mining-support, #dev); everything else starts as a low-volume announcement mirror.

### 3.5 Announcements — where, what, when

Venues that actually matter for a fair-launch PoW coin, in priority order:

1. **Bitcointalk ANN thread** (Announcements → Altcoins board) — still the canonical venue miners check for new PoW coins; a fair launch without one reads as suspicious. Standard `[ANN]` format: specs table, no-premine statement, links (repo, whitepaper, explorer, discord, mining guide), and — at mainnet time — the exact genesis timestamp.
2. **miningpoolstats.stream** — where ASIC operators discover coins; submit the coin + any pools once mainnet has stable endpoints.
3. **Pool-operator outreach (direct)** — the Kat Pool/NACHO community, HeroMiners, K1Pool, WoolyPooly etc.: a short email/DM with MINING-COMPAT.md and the patched-miner recipe. Every pool that adds Marigold is distribution we don't have to build.
4. **Hacker News (Show HN)** — for the *whitepaper*, not the coin: the "verifiable transparency instead of ZK, like cash" argument is a genuinely HN-shaped idea. Best single shot at reaching people who read designs.
5. **Reddit** — r/kaspa deserves one respectful, honest post (we forked their work, credit it, invite scrutiny; expect mixed reception and don't argue); crypto-wide subs have low signal, keep expectations at zero.
6. **CoinGecko / CoinMarketCap listing applications** — post-mainnet, once there's market data; free, slow, form-driven.
7. **Not recommended:** paid PR/crypto-media placements (fair-launch credibility is earned in miner communities, not bought), and any "AI-built" marketing angle (already decided against — disclosure stays one matter-of-fact FAQ sentence).

Cadence — two distinct announcement moments, deliberately different in volume:

- **Moment A — "testnet is live, code is public" (soft, at T10):** Bitcointalk pre-ANN ("testnet live, testers wanted"), Discord/X/Telegram go live, pool-operator outreach begins, Show HN when the whitepaper lands. Goal: outside testers for P8.7 (its verify condition literally requires them) — not hype.
- **Moment B — "mainnet genesis at <timestamp>" (loud, post-audit):** fair-launch norms: announce the genesis time **at least 4 weeks ahead**, publish final binaries + mining guide **at least 2 weeks ahead**, so nobody can claim insider head-start. Full ANN update, all channels, genesis-day live status page. This is P9.7's comms-plan bullet, now with content.

### 3.6 Long-lead items to start during the soak (they gate mainnet, not testnet)

- **Trustee recruitment (P9.1)** — five genuinely independent people/orgs with measurable independence criteria; realistically the longest human lead time in the whole plan. Start conversations now.
- **External audit sourcing (P8.8)** — auditors book out 4–8 weeks ahead; get quotes and a slot reserved early in the soak, not at its end. Budget item.
- **P5.8 sunset-threshold model (P9.5 hard gate)** — the quantitative T-model review 2 demanded; needs real testnet difficulty data, which the soak produces.
- **Legal counsel (P9.6)** — engage once the whitepaper draft exists (counsel reviews public claims; give them the actual claims).
- **Exchange posture** — per P1.9, nothing to *do* pre-launch except not promising anything; distribution at launch is mining + P2P, stated honestly everywhere.
- **Wallet UX gap (flagged, not scheduled)** — kaspa-cli's REPL is fine for testers, but the "granny at a market" story eventually needs a GUI wallet (QR flows exist in the CLI already). Post-launch roadmap item / community bounty; the P8.0 single-secret change lands before any of that.

## Part 4 — Sequence at a glance

1. **Now → testnet live:** T1–T8 (days).
2. **Testnet live → public:** T9–T10 + website sweep (days; gated only by your "runs properly" judgment and the T9 blessing).
3. **Moment A announcements**; soak clock starts (P8.7: ≥3 months with outsiders).
4. **During soak, in parallel:** whitepaper (draft → review → publish → Show HN), explorer port, faucet/status polish, P8.1–P8.6 hardening, trustee recruitment, audit booking, sunset model, legal review.
5. **Audit gate (P8.8) closes → P9.x → Moment B (≥4 weeks notice) → mainnet genesis (P9.5 ceremony) → P9.8 first-month watch.**

Realistic earliest mainnet, honestly stated: soak + audit arithmetic puts genesis in **Q1 2027**. The gating resources are calendar time (soak), external humans (auditors, trustees), and the founder's decisions above — not engineering throughput.

## Open decisions collected (ratify → DECISIONS.md rows)

1. T1 deploy method (rec: native Ansible own hosts, Docker images for the world).
2. T9: bless the pre-publication list as complete, or add the unstated "several points".
3. T10 registry (rec: GHCR + defensive Docker Hub name).
4. Whitepaper external review by P5.9 reviewers (rec: yes) and authoring start (I can draft now).
5. Explorer path (rec: Rust indexer port) and KGI visualizer as post-launch bounty.
6. Channel set + who staffs Discord.
7. Announcement venue list + the two-moment cadence as written.
