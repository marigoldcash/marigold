# Project state — session handoff

Snapshot of everything decided and built so far, so any fresh coding session on any
machine can continue from the repo alone. Read this together with [FORK-PLAN.md](../../FORK-PLAN.md).
Update this file whenever off-repo state changes (domains, accounts, infra).

Last updated: 2026-08-17 (P7.7 complete — wallet UX & docs pass, WALLET.md written and
validated against a real interactive kaspa-cli session; next: P7.8, end-to-end smoke
extension)

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
- **Planned public-testnet topology (agreed 2026-08-18, hardware in hand, not yet
  provisioned)** — three machines on fixed IPv4s plus two kHeavyHash ASICs. Confirmed
  prerequisites: PoW is unchanged kHeavyHash (ASICs mine Marigold natively), and the
  in-repo `stratum-bridge` is both rebranded (bc43c3d4) and P6.5-aware — its hasher
  mirrors the `pool_commitment` header field (dc32a708), without which every ASIC
  share would hash wrong. `TESTNET_PARAMS` already has `pool_activation: always()` —
  no consensus change needed to launch. Roles:
  - **Big rig (96 cores / 256 GB)**: public seed node 1 (`--testnet --utxoindex
    --archival` — the network's archival node from day one, doubling as the rehearsal
    for the T360 partner's archival+Caddy setup below); stratum bridge #1 → ASIC #1;
    build host (only machine with the toolchain — others get binaries); later: faucet,
    status page, trustee-signer rehearsal, spare capacity for P8.4 drill nodes.
  - **Medium rig (8 cores / 32 GB)**: public seed node 2; stratum bridge #2 → ASIC #2
    (ASICs deliberately split across nodes: mining survives either machine dying, and
    blocks genuinely propagate between independently-mining nodes — better soak data).
  - **Small VM (2 cores / 2 GB)**: public seed node 3 with `--ram-scale` turned down —
    a deliberate low-end viability probe (an OOM here is a P8 finding, not a failure);
    real value is a third independent IP. Fallback role: faucet/status-page frontend.
  - **DNS**: three DNS-only (NOT Cloudflare-proxied — proxying breaks P2P) A records,
    `tn-seed1/2/3.marigold.cash`, pointing at the fixed IPs, committed into
    `TESTNET_PARAMS.dns_seeders`. Static seed hostnames suffice at this scale; the
    NS-delegated crawler `dnsseeder` software remains P9.2.
  - **Sequence**: P7.8 (smoke extension) → TESTNET.md deployment runbook (systemd
    units; firewall: P2P public, RPC localhost-only, `--unsaferpc` never on a public
    node; bridge config; DNS records) → commit `dns_seeders` → provision → faucet
    (the one genuinely new build item) → invite testers, start the P8.7 incident log.
  - Two ASICs = ~100% of initial testnet hashrate — fine and expected for a testnet
    the founder controls; the DAA ramps difficulty to whatever they output.
- **Planned integration (agreed 2026-08-16, post-launch)**: a Time & Attendance SaaS
  provider (personal contact of the founder) will anchor monthly Merkle roots of
  customer PDF signatures on chain — one tiny transaction per month in his own
  user-lane subnetwork, each customer receiving an inclusion proof with their PDF
  (the OpenTimestamps pattern; design discussion in session history 2026-08-16). In
  exchange **he has committed to running a full archival node** (closing the
  pruning caveat on "independently verifiable") and already has a verification
  tool. Marigold-side work, all P9-era: a documented Caddy-on-443 reverse-proxy
  recipe for constrained runtimes (his app is Cloudflare Workers — gRPC is
  impossible from Workers and `connect()` is blocked toward Cloudflare IPs, which
  our own DNS uses), and a small standalone HTTP anchoring-gateway tool (Worker
  POSTs a 32-byte root; the gateway, holding the funded fee key off Cloudflare,
  builds/signs/submits) — sidecar binary like `trustee-signer`, never a fourth RPC
  surface on the node.
- **Future (P9.2)**: DNS seeders will be ≥2 independent VPS instances running Kaspa's
  `dnsseeder` software, reached via an NS delegation record per seeder subdomain
  (e.g. `seed1.marigold.cash`) created in this Cloudflare account — Cloudflare stays
  registrar/parent zone, the seeder software runs elsewhere. Mechanism detail in
  NOTES.md's P2.4 entry. Nothing to provision yet.

## Where execution stands

- **Next step: Phase 8 (Hardening), starting with the TESTNET.md deployment
  runbook** for the topology recorded under "Live infrastructure" below
  (systemd units, firewall posture, bridge config, DNS `dns_seeders` commit) —
  the logical next step after P7.8, though not yet formally started. Phase 8's
  own items (P8.1-P8.7, including the calendar-time testnet soak) run partly in
  parallel with this.
- **P7.8 is done — and with it, all of Phase 7 (the full note-pool wallet).**
  End-to-end smoke extension: taught `scripts/x-testnet-local.sh`/`.ps1` a
  `NETWORK=simnet` mode (every node now also gets `--rpclisten-borsh` and
  `--unsaferpc` explicitly — P7.7's finding, neither on by default), then
  extended [SMOKE.md](SMOKE.md) with the full note lifecycle across a live
  3-node simnet testnet: cross-node payment propagation (confirmed via node2's
  own `NotesChanged` subscription, nothing pushed directly between wallet
  sessions), a mid-flow node restart with clean re-sync, and a cross-node vault
  restore. **Real bug found and fixed**: `note vault restore`'s own documented
  idempotent-retry path ("mine a confirmation and re-run") was actually blocked
  by P7.7's `vault_exists()` safety check — it copies files and recovers `K`
  *before* attempting rotation, so a rotation-batch failure leaves a real vault
  in place that the check couldn't distinguish from a genuinely different one,
  refusing every same-words retry. Fixed with `NoteVault::words_match_existing_key`
  (unlocks the on-disk vault with the wallet secret already at hand and compares
  the recovered `K` against what the given words decode to) plus a
  `NoteKeyStore::vault_words_match` trait method; the CLI now resumes instead of
  refusing when the words match. Confirmed live on the identical retry command:
  refused pre-fix, resumed and completed further rotation batches post-fix.
  Final cross-node check (after everything above): all three nodes reported
  identical `get_pool_stats()`, sink block, and `pool_commitment`. Full writeup
  in NOTES.md's P7.8 entry.
- **P7.7 is done — wallet UX & docs pass, the first Phase 7 step to drive the
  real interactive `kaspa-cli` against a live daemon** rather than testing
  wallet-core directly. Audited `cli/src/modules/note.rs`: fixed a real
  unit-mixing bug (`note redeem`'s fee printed in raw sompi against every
  sibling's `sompi_to_kaspa_string`), lowercased 3 messages that had drifted to
  Title Case against the module's own 9-message majority, moved `note vault
  import`'s password from a plain CLI arg to an interactive masked prompt (a
  gap flagged when P7.6 landed), and added a real safety check — `note vault
  restore` now refuses if the destination wallet already has its own vault,
  rather than silently overwriting its `vault.key`. Wrote
  [WALLET.md](WALLET.md), validated via `pexpect` (a Python pty-driving
  library) genuinely driving `kaspa-cli`'s interactive REPL — confirmed P0.3's
  "can't be scripted" finding still holds for piped stdin, but a real pty
  works fine, a new precedent for this project. **Found along the way**:
  devnet is the one network shape with `pool_activation:
  ForkActivation::never()` (mainnet/testnet/simnet are all `always()`, per
  P2.6/P6.5 — P7.7's docs originally over-claimed testnet/mainnet as `never()`
  too; corrected 2026-08-18) — so SMOKE.md/P4.1's devnet-based testnet
  structurally cannot run a single `note` command; WALLET.md is simnet-based
  throughout. **Two more real bugs found live** (on top of P7.6's four): (1)
  `note vault restore`'s rotation loop aborted entirely on one batch's
  failure, leaving unrelated later batches unexecuted — now continues past a
  failed batch. (2) `deep_verify`'s `stale` findings weren't reconciled to
  local status, so `rotate_notes` kept re-proposing the same chain-dead serial
  as a fee-source spare forever — `note vault restore` now marks every stale
  serial `Superseded` locally right after `deep_verify` reports it. Full
  writeup in NOTES.md's P7.7 entry.
- **P7.6 is done — note vault, backup, and restore, live-verified, the largest
  single step of Phase 7.** File-per-note encrypted vault
  (`storage::local::notevault::NoteVault`) replaces P7.1's single-blob store —
  plaintext filename (value/serial), contents encrypted under a per-wallet
  vault key K (raw-key XChaCha20Poly1305, no Argon2 stretch — K is already
  CSPRNG entropy), status as a directory with atomic-rename transitions.
  24-word ceremony for K via `kaspa_bip32::Mnemonic` (explicitly not a
  note-deriving seed — recovery needs the words **and** the vault files);
  auto-provisions silently on first use so every pre-P7.6 flow keeps working,
  with `note vault create` as the proper explicit ceremony. `manifest.tsv`
  deliberately does double duty as both the mandatory fast in-memory index and
  DECISIONS.md's human-facing manifest (a recorded simplification, not a spec
  violation). Two verify tiers (`account::notepool::light_verify`/`deep_verify`),
  `light_verify_vault` for checking a standalone backup directory with no wallet
  open at all, `plan_restore_rotation` for the default-on/overridable batched
  restore-rotation (2-5 randomly-composed batches), and a paper QR export/import
  codec reusing `BearerNote`'s exact wire format. CLI: `note vault
  create/backup/verify/restore/export/import`. **Four real bugs found and fixed
  by the live daemon test, none by unit tests**: (1) `try_create`/`try_import`
  never wiped a stale `vault.key` from an earlier same-named wallet, breaking
  every P7.1-P7.5 regression test the moment this step's wiring landed; (2)
  resident wallets all shared one default vault location, harmless for the old
  in-memory store but catastrophic for a secret-keyed one — fixed with a random
  per-instance temp location; (3) `restore_key_from_words` didn't invalidate an
  already-cached (e.g. empty) in-memory index, silently shadowing freshly
  copied-in files forever; (4) `deep_verify` propagated a decrypt failure as a
  hard error instead of classifying it `corrupted`. Also documented: naively
  executing pre-planned restore-rotation batches in order can hit a real
  node-level rejection, since an earlier batch's fee-sourcing can legitimately
  consume a note a later batch was targeting — correct pool-op economics, not a
  bug, but the orchestration loop must re-check liveness per batch. Full
  writeup in NOTES.md's P7.6 entry.
- **P7.5 is done — POS landing-pad mode, live-verified, smallest step of the
  five.** `pos_checkout()` (`account::notepool`) is three existing P7.3/P7.4
  calls chained (`create_payment_request` → `await_payment_request` →
  `rotate_notes` on the claimed serials) — the spec's "one `SignedGroup`" sweep
  requirement falls out automatically since every claimed note already shares
  the landing-pad key. New: an `on_request` callback so the CLI can show the QR
  before the (long) payment wait, since `pos_checkout` only returns once the
  whole sale is done. Design call: the sweep re-decomposes by value (canonical
  ladder form), not a strict 1:1 per-note rekey — both satisfy "no note stays
  shared," and value-based reshaping opportunistically consolidates a
  merchant's dust. Deliberately deferred: the static day-`pk` fallback for
  printed/no-register QR codes (P5.6's own secondary form; a genuinely
  different repeat-watch shape, not exercised by FORK-PLAN's verify text).
  CLI: `note pos <amount>`. Live test: customer holding one 0.1 note pays a
  0.04 checkout (splitting), merchant sweeps to 3 fresh Cold notes under
  distinct keys the instant it confirms, none remaining on the checkout pk.
  All 4 notepool live tests green; wallet-core 56 green; workspace check +
  clippy clean. Full writeup in NOTES.md's P7.5 entry.
- **P7.4 is done — split spends + bearer export, live-verified.**
  `select_covering` (exact-first, else smallest-first sweep — organic dust
  consolidation via change), with payment/split/change/fee in ONE `TransferOp`;
  the separate P7.3 fee-source stage dissolved into the covering target. New
  `NoteStatus::HandedOver` for bearer-exported notes (excluded from balance and
  all selection; flips to Superseded via the ordinary NotesChanged-removal path
  when the receiver rotates). `bearer_export` enforces P5.6's solo-key invariant
  structurally — shared or Hot keys are auto-isolated via `rotate_notes` first
  and only the isolated fresh key is ever exported; CLI `note export` waits for
  the isolation to confirm before displaying the QR. Live test: one 0.1 note
  pays a 0.03 request in a single split+pay+change+fee tx; a landing-pad
  (shared-pk) export demonstrably isolates first — two on-chain txs for the
  note's journey — and a solo export skips isolation. Full writeup in NOTES.md's
  P7.4 entry.
- **P7.3 is done — both receive flows live-verified between two wallet
  instances.** The wallet's first `TransferOp` construction
  (`account::notepool::submit_transfer` + `rotate_notes`/`pay_payment_request`),
  with a recorded fee design: pure pool transfers pay fees in 0.01-MAGLD quanta
  (value math forces this — every value is a multiple of the smallest
  denomination), sourced via P5.2 fee stamps from spare notes (excess back as
  change) or slack-withheld from the rotation itself when the wallet holds
  nothing else (bootstrap case). Deliberately NOT transparent-funded — that would
  link transparent identity to note rotations. `PaymentRequest` QR/text in both
  spec forms (40B pinned-amount / 32B payer-fills-in), `BearerNote` = the paper
  backup's `(sn, sk, d)` triple; `qrcode` crate (CLI-only) renders terminal QRs.
  Payment-request keys are a new persisted store (encrypted, written before the
  QR is displayed — crash safety). CLI: `note request/pay/import`. **Real
  pre-existing bug found by the live test**: the wRPC client never registered a
  handler for `NotesChangedNotification` — P6.9 wired the server, and every wRPC
  client silently dropped the notification; one-line fix in
  `rpc/wrpc/client/src/client.rs`. Known deliberate gap: offline-landed payments
  need a query-by-pk RPC to discover — lands with P7.6's restore (which needs it
  anyway). Full writeup in NOTES.md's P7.3 entry.
- **P7.2 is done — live-verified against a real daemon, not just unit tests.**
  `note mint <amount>` / `note redeem <serial>...|amount <amount>` /
  `note balance` / `note list` (`cli/src/modules/note.rs`). Mint funds through the
  ordinary `Generator`/`Signer` pipeline (`GeneratorSettings::with_subnetwork_id()`,
  new, applied only to the final transaction; minted value withheld from change via
  `PaymentDestination::PaymentOutputs(vec![])` + `Fees::SenderPays(amount_petals)`,
  no output ever represents it). Redeem is hand-built (zero transparent inputs,
  mirrors `trustee-signer::anchor_transaction`'s pattern) since it's self-funding by
  design and doesn't fit `Generator`'s aggregate-toward-a-target model at all. New
  live daemon+wallet test infra (`testing/integration/src/
  notepool_wallet_integration_tests.rs`, `kaspa-wallet-core` now a
  `testing/integration` dependency for the first time — reusable by P7.3-P7.5)
  found and fixed a genuine `Generator` bug along the way, not notepool-specific:
  Toccata-version (≥1) transactions need `ComputeBudget`-based input mass, not
  legacy `SigopCount` — nothing before P7.2 had ever combined a non-native
  subnetwork with real transparent inputs, so the mismatch had no way to surface
  until mint needed both. Live-verified reconciliation: mint drops balance by
  exactly `amount + real fee`, redeem raises it by exactly `redeemed value - real
  fee`, net cost across both is exactly the two real fees, nothing more. Full
  writeup (including the two test-harness maturity-timing bugs found along the
  way) in NOTES.md's P7.2 entry.
- **P7.1 is done.** `wallet/core::storage::notekeys` — `NoteKeyEntry{sn,sk,d,
  provenance}` (encrypted, zeroized on drop) split from `NoteKeyInfo{sn,pk,d,
  provenance,status}` (plaintext index — nothing but `sk` is actually sensitive,
  the pool being plaintext means `sn`/`pk`/`d`/`provenance` are already public or
  wallet-internal). `NoteKeyStore` trait wired through `Interface`/`Payload`/
  `Cache`/`LocalStoreInner` exactly like `PrvKeyDataStore`. `import_bearer_key`
  takes no provenance argument — bearer imports are structurally always `Hot`.
  `apply_notes_changed(wallet_secret: Option<&Secret>, notification)` is the
  P6.9-subscription landing point, split so the safe half (superseding a removed
  serial's `status`) never needs the wallet secret and can run from a passive
  background listener even while locked; the half that inserts a new row (a
  rotation landing on a `pk` we hold) needs the secret and is `deferred` if none
  is supplied. `UtxoProcessor` gained `register_note_serials`/
  `unregister_note_serials` + `NotesChanged` dispatch, forwarded to `Wallet` via a
  new `WalletBusMessage::NotesChanged` arm, mirroring the existing
  `UtxosChanged`/`Discovery` wiring exactly. 3 new unit tests, full wallet-core
  suite green (46), workspace check + wallet-core clippy clean. Full design
  writeup (including why serial-keyed over POOL-SPEC's key-keyed sketch) in
  NOTES.md's P7.1 entry.
- **P7.0 is done.** Decision (user-ratified, recorded in DECISIONS.md): the
  inherited seed-phrase wallet stack **stays as the transparent-tier wallet tool**;
  every legacy-Kaspa import surface was removed in the same step (compat/gen0+gen1
  deleted, golang import API + wire types removed, CLI legacy arms
  removed/refused, help text scrubbed; storage variant + derivation kept so old
  wallet files still open). Also: the T&A anchoring-gateway API contract draft was
  published at docs/x-fork/ANCHORING-GATEWAY.md for the partner's integration.
- **P6.12 is done — and with it, all of Phase 6 (the full note-pool consensus layer
  plus the finality-anchor security layer).** Anchor gossip over P2P (on-connect
  request from every peer + hub-wide relay of improvements), the pending-anchor slot
  with automatic promotion, anchor-aware IBD refusal at the three-IBD-types
  convergence point, the anchor lane's mempool fee exemption + forced template
  inclusion, the `GetFinalityAnchorStatus` RPC (spec's wallet-visible
  `finality_anchor_stale`), and the `kaspa-trustee-signer` crate (per-key signer with
  persisted-before-shared equivocation-safe state, TCP partial exchange, canonical
  k-of-n assembly). **One real security bug found by the adversarial test and
  fixed:** fail-open's staleness clock was virtual's mergeset-inclusive DAA score,
  which a refused-but-merged attacker branch could inflate to trip fail-open — now
  the sink's own header score. Full writeup in NOTES.md's P6.12 entry.
- **P6.11 is done (⚠️ HARD, done under a stronger model per standing discipline).**
  The P5.8 finality-anchor consensus rule end to end: `consensus-core::finality_anchor`
  (types/hashing/k-of-n + equivocation verification), the "ANCR" subnetwork,
  `FinalityAnchorParams` in params (trustees `None` everywhere until the P9.1
  ceremony — mechanism ships inert; decay stages 1-3 are `ForkActivation::never()`
  hooks pending the P9.5 T-calibration, deliberately), `DbFinalityAnchorStore`
  (monotone latest-anchor ratchet + reachability-keyed deny-list), the fork-choice
  override in `sink_search_algorithm` (second trigger next to the finality-point
  refusal — staleness judged, since P6.12's bug fix, against the sink's own header
  DAA score), fail-open with
  edge-detected loud alerting, and `ConsensusApi::get_finality_anchor_status()`.
  Two structural decisions worth re-reading before touching this code — context-free
  transaction validity vs contextual anchor *effect* (an acceptance-divergence hazard
  was designed out), and monotone-by-keying deny-list semantics — plus the
  one-block anchor application lag (a block's own txs are accepted by its chain
  descendants) are all documented in NOTES.md's P6.11 entry.
- **P6.10 is done.** New split/merge/`BadPublicKey`/`MalformedNotePoolPayload`/deep-
  reorg/cross-op-conflict tests in `consensus/src/pipeline/virtual_processor/
  notepool_tests.rs` (reusing its existing harness rather than duplicating it into
  `testing/integration` — see NOTES.md for why); a new three-daemon
  `daemon_notepool_multi_node_agreement_test`; `ConsensusApi::get_pool_root()`; and
  simpa taught to self-target mint/rotate/merge/redeem pool ops per miner, gated by a
  new `pool_op_probability` flag, with a `run_and_verify_pool_root_agreement` check.
  Found and fixed three real bugs along the way, all in simpa's own harness (not the
  pool implementation): missing `toccata_activation`/`pool_activation` in simpa's
  config, an opaque `unimplemented!()` masking real validation errors (plus a latent
  `None.unwrap()` it was hiding), and two heavy simpa tests racing for the same
  process-wide file-descriptor budget when run concurrently. Full writeup in NOTES.md.
- **P6.9 is done.** RPC methods `get_notes_by_serial`/`get_pool_stats` plus a
  `NotesChanged` subscription (through the notify system, scoped to watched
  serials/pks), wired through rpc/core, grpc proto, and wrpc, plus a new
  `kaspa-cli` `rpc notify-notes-changed` command. Full design rationale — why
  `NotesChanged` needed no `UtxosChanged`-style second index stage, why its
  subscription skips the `Tracker` machinery, the wrpc-vs-grpc plumbing asymmetry —
  is in NOTES.md's P6.9 entry.
- **P6.8 is done (done under a stronger model — the prior session scoped it and
  correctly judged it HARD-caliber despite the plan not flagging it).** New nodes
  syncing from a pruning point download the pool state, verify it against the
  header-committed root, and end with a genuinely usable pool.
  **The real design problem, absent from the plan text**: the server must serve the
  pool state *at the pruning point*, but only virtual's pool state existed (P6.4).
  Solved exactly the way UTXO solves it (confirmed by reading, not analogy): a new
  pruning-position pool store in `PruningMetaStores` (prefix 95), advanced by the
  pruning processor from the per-chain-block `notepool_diffs` store **in the same
  WriteBatch** as the pruning utxoset, so the existing `utxoset_position` recovery
  marker and crash semantics cover both. New pool stable flag (prefix 96) folded
  into `is_in_transitional_ibd_state`.
  **Deliberate deviation from both plan hints, documented in NOTES.md**: wire shape
  mirrors the UTXO flow (4 messages, Done-sentinel, no metadata, no inline proofs
  — the pool root is the header's `pool_commitment` verbatim, already PoW-validated
  locally, unlike seq-commit's folded lanes_root; inline proofs would force every
  node to maintain a second pruning-positioned SMT forever). Verification uses
  `crypto/smt`'s generic `StreamingSmtBuilder` directly (new
  `DbNotePoolSmtStore::rebuild_from_sorted_leaves` + a small pool `MergeSink`) —
  NOT `consensus/smt-store`'s `streaming_import`, which is hard-coded to
  seq-commit's versioned multi-lane apparatus. O(n) single pass; RocksDB's native
  key order is exactly the sorted input the builder needs.
  Import populates virtual's pool map + SMT in the same pass, resets the stored
  virtual pool diff, and runs BEFORE the utxoset sync in all three IBD branches
  (the utxo import validates the pruning point's own pool ops against virtual's
  pool state) — closing P6.4's "pool state empty at pruning import" caveat, which
  was a genuine silent-wrongness gap (empty pool + non-empty commitment, nothing
  comparing them).
  **Two real bugs found by reading actual code**: pruned blocks' `notepool_diffs`
  were never deleted (permanent disk leak since P6.4 — fixed); and the above import
  gap. Pool commitment added to the pruning processor's sanity checks
  (`assert_pool_commitment`).
  ✅ *Verify*: new `daemon_ibd_pool_state_sync_test` (two real daemons): mint →
  rotate → bury past pruning depth → fresh node IBDs → pruning point commits a
  non-empty pool, **the syncee's own mempool accepts a rotate consuming notes that
  exist only in the imported state**, and the syncee follows post-IBD blocks.
  Passed first run, full-log confirmed. Tamper rejection pinned by store unit
  tests (`streaming_rebuild_detects_tampered_leaf`, rebuild-vs-incremental
  agreement). Full workspace + integration suites green; workspace build clean.
- **P6.7 is done.** Pool ops (Mint/Transfer/Redeem) are live in the real mempool —
  before this step, a `PoolOp` transaction could only ever enter the system by
  being handed straight to a test block builder; no real user could submit one to
  a running node. A research pass first (before writing code) found the mempool
  crate already input/output-count-agnostic almost everywhere (standardness,
  mass/fee ordering, template selection all correctly treat a zero-input/
  zero-output Transfer as vacuous) — the real work was three new pieces: (1)
  lifted P6.6's `NotePoolTxNotYetSupportedInMempool` mempool-entry guard, added a
  `pool_view` parameter to `validate_mempool_transaction_in_utxo_context`,
  mirroring the block-validation path's existing decode → `validate_stateful` →
  real `pool_value` sequence; (2) `mining/src/mempool/model/pool_note_set.rs`
  (new): `MempoolPoolNoteSet`, a serial-keyed conflict index structurally
  identical to `MempoolUtxoSet`'s outpoint index, but unconditional (no RBF
  variant — confirmed this is just "run the existing `RbfPolicy::Forbidden`
  branch's logic always", not a new mechanism) and tracking only consumed
  serials, not produced ones; (3) `handle_new_block_transactions.rs` gained
  `remove_serial_conflicts`, the serial-keyed sibling of `remove_double_spends`.
  `ConsensusMock` (every mempool crate test's harness — confirmed zero
  `TestConsensus` usage) gained real pool-op support, reusing the actual
  `validate_stateful` consensus-core function against a `PoolCollection` (which
  already implements `PoolStateView` directly) rather than a second hand-rolled
  reimplementation.
  **A real, plan-text-silent gap found and deliberately scoped out, not missed**:
  `populate_mempool_entries` lets a transaction spending an unconfirmed mempool
  transaction's output validate by pre-populating UTXO entries positionally —
  there's no positional equivalent for pool state (`PoolStateView::get_note` is
  queried by serial hash, not an array index), so a pool op consuming a serial
  another still-unconfirmed mempool pool op is about to produce is hard-rejected
  today (`SerialNotFound`), not orphaned. Chaining two unconfirmed pool ops
  back-to-back doesn't work yet — closing it needs a `PoolDiff`-based mempool
  overlay (composed via the same `PoolViewComposition::compose` the block
  pipeline already uses) plus a serial-keyed sibling to `OrphanPool`, real
  additional scope with no verify condition demanding it now. Recorded explicitly
  as a known MVP limitation (sub-second UX cost given the pool's target cadence),
  not left to be rediscovered by surprise.
  New `mining/src/notepool_mempool_tests.rs` (3 tests: this step's own two verify
  criteria plus a third closing the loop on the eviction mechanism itself).
  First draft of every test used a zero-fee `consumed == produced` rotate and
  failed standardness checks — a reminder, not a bug, that a real pool-op tx
  needs a genuine fee (a small "fee stamp" note consumed alongside the note being
  moved) the same as any other transaction.
  ✅ *Verify*: all 3 new tests pass. `cargo test -p kaspa-mining` 59 passed, 0
  failed. Full `cargo test --workspace --exclude kaspa-testing-integration`: 0
  failed across every crate. Integration suite: 42/42 passed. `cargo build
  --workspace` clean.
- **P6.6 is done.** Mint/redeem value binding, unified into
  `validate_populated_transaction_and_get_fee` via one `pool_value:
  Option<(u64, u64)>` parameter (`consumed_petals`, `produced_petals`) rather than
  three separate per-op checks: `available = total_in + consumed`,
  `spent = total_out + produced`, `available >= spent` — this single formula
  derives Mint's "transparent inputs cover new notes", Redeem's "consumed notes
  cover transparent outputs + fee", and pure Transfer's `consumed - produced`
  fee-stamp rule all at once, matching POOL-SPEC.md P5.2 exactly for every op.
  **A real mass-costing gap found, not called out by the plan text**:
  `calc_non_contextual_masses`'s signature-verification cost was computed entirely
  from `tx.inputs`, which is always empty for Transfer/Redeem — meaning their
  `SignedGroup` Schnorr verification was completely uncosted (a genuine fee-evasion
  gap) before this step added `GRAMS_PER_COMPUTE_BUDGET_UNIT * num_signed_groups`,
  gated on the pool subnetwork and decoded op type.
  **The same class of bug already found once in P6.4 recurred and was fixed
  file-wide this time**: two pre-existing tests compare outcomes across
  independently-run `TestConsensus` instances by rebuilding "the same" transaction
  in each — safe when mints were zero-input, broken once a real mint's funding UTXO
  became instance-specific, since BIP340's randomized aux-nonce makes a rebuilt
  signature hash differently every time. Fixed by switching every signature in
  `notepool_tests.rs` to `sign_schnorr_no_aux_rand` (both `Wallet::rotate` and a new
  local, deterministic reimplementation of `sign()` for the mint's transparent
  input — the real `sign()` couldn't be changed, since real signing paths elsewhere
  depend on its genuine randomization).
  Test infrastructure needed building from scratch (no precedent existed for a
  real, coinbase-funded spend in a `TestConsensus` unit test): confirmed Kaspa/
  Marigold's mergeset reward mechanism pays a block's subsidy through its CHILD's
  coinbase, never its own, and confirmed the correct P2PK script shape
  (`OP_DATA_32 <x-only pk> OP_CHECKSIG` via `pay_to_address_script`) — explicitly
  NOT the raw-pubkey shape `pipeline/virtual_processor/tests.rs`'s pre-existing
  `new_miner_data()` helper uses, which is invalid and must not be copied.
  Added the three tests this step's own verify condition calls for:
  `mint_with_insufficient_transparent_inputs_rejected`,
  `redeem_with_excessive_transparent_outputs_rejected`, and
  `value_conservation_across_mint_transfer_redeem` (the literal
  `Σ pool notes + transparent supply == emitted supply` check across a real mint →
  transfer → redeem sequence).
  ✅ *Verify*: all 9 `notepool_tests.rs` tests pass (6 pre-existing rewired to real
  funding + 3 new). `cargo test -p kaspa-consensus` 90 passed, 0 failed. Full
  `cargo test --workspace --exclude kaspa-testing-integration`: 0 failed across
  every crate. Full `cargo build --workspace` clean.
- **P6.5 is done.** `Header` gained `pool_commitment: Hash`, hashed right after
  `utxo_commitment` in `hashing::header::hash_override_nonce_time` — confirmed via
  direct code reading (not assumed) that this is the FIRST genuinely new field this
  fork has ever added to the header hash preimage; every prior change (seq-commit,
  `CompressedParents`) reinterpreted or re-encoded an existing field instead. Gated
  by a new independent `pool_activation: ForkActivation` (always() mainnet/testnet/
  simnet, never() devnet, matching `toccata_activation`'s own precedent) and a third
  block-version tier `NOTE_POOL_BLOCK_VERSION = 3` (`ForkedParam<u16>` couldn't
  express a 3-way chain, so `block_version()` now returns a small dedicated
  `BlockVersionParam`).
  **The real design problem, found via a failing test, not anticipated**: verifying
  `pool_commitment` for an arbitrary chain block during a reorg's exploratory walk
  needs branch-node structure consistent with THAT block's own position — but
  `DbNotePoolSmtStore` (P6.2's deliberately single-current-state design) only ever
  reflects whichever branch was most recently committed to virtual. Reading it for
  an off-canonical-branch block would silently return a stale root — confirmed real,
  not theoretical, since it broke this step's own new cross-check test. UTXO's
  `utxo_commitment` avoids this because MuHash is an algebraic accumulator (branch-
  independent composition); SMT roots have no equivalent property. Fixed via
  `recompute_pool_commitment`: materialize the full live pool-entry set (persisted
  flat map + accumulated diff — correctly branch-independent, unlike SMT structure)
  and rebuild a fresh in-memory SMT from scratch, O(pool size) per verified block —
  a documented, correctness-first tradeoff for a fresh/early network, not silently
  punted (a proper incremental multi-branch store is named future work).
  `DbNotePoolSmtStore` is kept for virtual's own fast root query, which has no
  branch-divergence risk (virtual only ever advances linearly). A second bug found
  by the same investigation: `build_block_template_from_virtual_state` originally
  read `pool_smt.current_root()` directly — correct for real mining but wrong for
  `TestBlockBuilder`'s "template for arbitrary parents" path every reorg test uses,
  where the hypothetical virtual state may not match what's persisted. Fixed by
  making `pool_commitment` an explicit per-caller parameter.
  Full wire propagation was required for the workspace to compile at all (not
  deferrable to P6.9 as originally hoped): p2p.proto's `BlockHeader`, `rpc-core`'s
  `RpcRawHeader`/`RpcHeader`/`RpcOptionalHeader`/`RpcHeaderVerbosity`, `rpc-grpc-core`'s
  two proto messages + converters, `rpc-service`'s verbosity adapter, the WASM SDK's
  `IHeader`/`IRawHeader` (genuinely hash-affecting — `finalize_js` calls the real
  canonical hash function), and `bridge/src/hasher.rs`'s hand-rolled preimage
  (updated in the identical field position, or real miners' shares would
  hash-mismatch and get silently rejected).
  Genesis regenerated for all four networks via the established test-and-paste loop;
  confirmed via P9.5's own entry this is explicitly another placeholder pass, not
  final. New `incremental_and_full_rebuild_commitments_agree` test pins the two
  commitment mechanisms to agree exactly. Three MAINNET_PARAMS-based tests (one in
  `consensus`, two in the integration suite) needed `pool_activation =
  ForkActivation::never()` added to isolate their own toccata-version assertions
  from the now-also-default-active pool fork.
  ✅ *Verify*: `cargo test -p kaspa-consensus-core` 121 passed (all 4 genesis hashes
  regenerated and verified). Full `cargo test --workspace` (minus integration): 1,207
  passed across 142 binaries. Integration suite: 42/42 passed. Real `kaspad --devnet`
  binary verified live via gRPC — regenerated genesis hash matches, `pool_commitment`
  served correctly over the wire as a well-formed value. Deep correctness (mint/
  rotate/reorg/cross-mechanism agreement) covered by the `skip_proof_of_work()`
  `TestConsensus` suite — this project's established methodology — rather than
  solving real devnet PoW, a deliberate, documented scoping call. Full `cargo build
  --workspace` clean.
- **P6.4 is done (⚠️ HARD — done with a stronger model per the plan's flag).** Pool
  ops are live in the virtual pipeline end-to-end. The core: validation happens
  inside `validate_transaction_in_utxo_context` against a composed pool view
  (`PoolStateView`/`ComposedPoolView`, mirrors `UtxoView`), so **first-accepted-wins
  is the existing composed-view mergeset mechanism** — no new conflict rule, exactly
  as P5.3 designed. `UtxoProcessingContext` accumulates a `mergeset_pool_diff` in
  lockstep with the UTXO diff; per-chain-block diffs persist in a new
  `notepool_diffs` store (prefix 93, same batch as `utxo_diffs`); virtual pool state
  (map + SMT root) lives in `VirtualStores` (`pool_state`/`pool_smt`/`pool_diff`,
  prefix 94) and is applied/unapplied via the same accumulated-diff walks as the
  UTXO set, including reorg walk-downs. New hashers `NotePoolSerialHash`/
  `NotePoolSigningHash`/`NotePoolOutputsHash` implement P5.2's exact preimages.
  **Consensus-critical subtlety, documented in code**: freshness is non-monotonic in
  POV DAA score, so signature+freshness checks are skipped on the selected-parent
  replay (riding `SkipScriptChecks`) — re-imposing them would fork acceptance data.
  **Two real fixes**: a div-by-zero in `calc_storage_mass` for zero-input txs (pure
  transfers are the first-ever such shape; KIP-9's |I|/A(I) term vanishes → mass =
  max(0, harmonic_outs)); and the mint produced-serial existence check made an
  *active* rule until P6.6's value binding makes duplicated zero-input mints
  impossible (the spec's "guaranteed by construction" reasoning presumed P6.6).
  Body-level `check_block_double_serials` mirrors the UTXO double-spend rule
  (parallel per-tx validation requires intra-block conflicts to be block-invalid).
  **Deferred with in-code notes**: transparent value binding/fees/mass → P6.6;
  mempool rejects pool txs until P6.7 (no conflict policy yet → template self-DoS
  risk); pool state empty at pruning import until P6.8. All three P6.4 verify
  criteria have passing consensus tests
  (`consensus/src/pipeline/virtual_processor/notepool_tests.rs`), including
  double-rotate determinism under reversed insertion order and reorg convergence to
  a never-forked reference node's exact pool root. consensus-core 121 / consensus 86
  / workspace 1,206 tests passing; integration suite green.
- **P6.3 is done.** [validate.rs](../../consensus/core/src/notepool/validate.rs)'s
  `validate_stateless(&PoolOp)` — checks that hold with zero pool/consensus state.
  Notable finding: three of P6.3's own named checks ("denominations from the P1.6
  set", "freshness-anchor field present", "signature well-formed") turned out to
  already be fully guaranteed by the type system once a `PoolOp` decodes at all —
  confirmed the "signature well-formed" claim by reading `secp256k1` v0.29.1's actual
  source rather than assuming, found `schnorr::Signature::from_slice` only checks
  length (already guaranteed by the `[u8; 64]` field type), and removed the dead
  check rather than ship code that can provably never fail. What's actually enforced
  at runtime: non-empty collections, the 1,000-item cap deferred from P6.1
  (`MAX_POOL_OP_COLLECTION_LEN`, a standalone constant chosen to match — not read
  from — mainnet's `max_tx_inputs`/`max_tx_outputs`), and no duplicate serial within
  or across an op's `SignedGroup`s (P5.3 step 1's stateless half; the same-`pk`
  requirement and actual signature verification are P6.4's job). New
  `PoolOpValidationError` in `consensus/core/src/errors/notepool.rs`, matching
  `TxRuleError`'s existing `thiserror` convention. 15 new table-driven tests pass.
  Full `cargo test -p kaspa-consensus-core` (94 passed) and full `cargo build
  --workspace` clean.
- **P6.2 is done.** Two new RocksDB stores mirror `DbUtxoSetStore`/`UtxoDiff`
  exactly: [notepool.rs](../../consensus/src/model/stores/notepool.rs)
  (`DbNotePoolStore`, flat `sn -> NewNote` map) and
  [notepool_smt.rs](../../consensus/src/model/stores/notepool_smt.rs)
  (`DbNotePoolSmtStore`, `BranchKey -> Node` branch storage implementing
  `crypto/smt`'s `SmtStore`, plus a `CachedDbItem<Hash>` root singleton).
  `apply_diff`/`unapply_diff` take a `PoolDiff`
  ([diff.rs](../../consensus/core/src/notepool/diff.rs), mirrors `UtxoDiff`
  minimally: `add`/`remove` + `to_reversed()`) and call `crypto/smt`'s
  `compute_root_update` (the production incremental path, not the
  `#[cfg(test)]`-gated in-memory tree). Three new hasher types
  (`NotePoolLeafHash`, `NotePoolSmt`, `NotePoolSmtCollapsed`) added to
  `crypto/hashes/src/hashers.rs`'s `blake3_hasher!` block — **corrected a
  spec-vs-reality gap found during implementation**: P5.1's prose calls Blake2b
  "the codebase's single hashing convention", but there are actually two
  coexisting families (legacy Blake2b, Toccata-era Blake3); as a new Toccata-era
  feature the pool follows Blake3 (the seq-commit precedent), not Blake2b — worth
  a v1.2 spec addendum at some point. `NotePoolSmt`/`NotePoolSmtCollapsed`
  registered in `crypto/smt/build.rs`'s `KNOWN_HASHERS` for the generated
  `SmtHasher` impl. Deliberately did NOT reuse `consensus/smt-store`'s
  `SmtProcessor`/`BranchVersionKey` apparatus (despite P5.4's prose citing it) —
  that crate solves a harder, block-versioned multi-lane problem the pool doesn't
  have. Three new `DatabaseStorePrefixes` entries: `NotePoolState = 90`,
  `NotePoolSmtBranches = 91`, `NotePoolSmtRoot = 92`. 7 new store tests pass,
  including both of P6.2's exact verify criteria (apply/unapply round-trips
  restore the exact prior root; commitment deterministic across insertion
  orders) plus a real-RocksDB-reopen persistence check. Full `cargo test -p
  kaspa-consensus` (80 passed) and full `cargo build --workspace` clean.
- **P6.1 is done.** [consensus/core/src/notepool.rs](../../consensus/core/src/notepool.rs)
  implements `pool-spec-v1.1`'s P5.1/P5.2 wire types verbatim — `DenominationTag`,
  `Note`, `NewNote`, `SignedGroup`, `FreshnessAnchor`, and the 3-variant `PoolOp`
  (`Mint`/`Transfer`/`Redeem` — note this supersedes FORK-PLAN's own older "5-op"
  P6.1 phrasing, written before Phase 5 unified rotate/split/merge into one
  `TransferOp`; implemented against the spec, the frozen source of truth, not the
  plan's pre-Phase-5 prose). `SUBNETWORK_ID_NOTE_POOL` added to `subnets.rs`. 17 new
  tests pass, including exact byte-size assertions matching all seven of P5.2's worked
  examples (38, 170, 150, 182, 1,305, 1,385, 177 bytes) — a second, independent
  confirmation of the spec's own arithmetic. SMT/store work explicitly deferred to
  P6.2; the 1,000-item collection cap deferred to P6.3 (validation-time, not data-layer).
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
- Phases 0–4 were bite-size sessions by design; Phase 5 needs spec work and the
  ⚠️ HARD steps (P6.4, P6.8 by judgment, P6.11, P6.12). The next ⚠️ HARD step is 
  P8.3; P8.7/P8.8 are 🧑‍⚖️ gates needing the user. Standing rule: if a step turns out 
  materially bigger than its label (the P6.8 precedent), stop and flag for a model 
  switch instead of pushing through.

## Suggested session opener on a new machine

> Read FORK-PLAN.md and docs/x-fork/STATE.md, then do step P0.1.
