# Triage — pool-spec v1.1-draft review 2 (James O'Connell)

Review: [pool-spec-v1.1-review-2-james-o-connell.md](pool-spec-v1.1-review-2-james-o-connell.md),
received 2026-08-15, reviewing the v1.1-draft (i.e. including the review-1 fixes, which
it independently confirms — notably the Redeem output binding). Triaged same day.

**Character of this review**: unlike review 1 (which found a concrete vulnerability),
review 2 found **no new flaw**. It validates the architecture section by section, then
demands *rigor artifacts*: written proofs, exhaustive field audits, precise
canonicalization, explicit threat-model statements, and quantitative economic modeling
— the "show your work so specialists can check it" layer. Its P0/P1/P2 priorities are
about what must exist before implementation lock / launch, not about broken designs.

## Disposition, finding by finding

| # | Finding (reviewer's priority) | Disposition | Where |
|---|---|---|---|
| 1 | P5.2: write the central authorization theorem + proof sketch (P0) | **Done** — theorem stated with four explicit premises each tied to its enforcement point, plus a proof sketch, marked "for specialist verification, not its own confirmation." | Spec P5.2, "The authorization theorem" |
| 2 | P5.2: signed/unsigned field audit for every tx + op field (P0, "most important artifact") | **Done** — complete matrix. Every field is Committed, Harmless (with argument), or Analyzed. One genuine deviation surfaced and documented: **consumed-group-set malleability** (third parties can add their own validly-signed groups, or strip others' — bounded to fee effects, never redirection/re-denomination; full-set binding deliberately rejected since it would forbid collaborative fee attachment). Marked **[Open]** for specialist sign-off. Also confirmed `Mint` is secured by the *existing* transparent sighash, which already commits to payload + outputs (verified against `sighash.rs`). | Spec P5.2, "Signed/unsigned field matrix" |
| 3 | P5.2: define the authorization threat model — is pre-execution bearerability acceptable? (P0) | **Done — accepted explicitly, with reasoning**: a signed op authorizes exactly one transition; early broadcast executes precisely the intended payment, so bearerability is inherent-and-acceptable for offline signed authorization. Binding wallet-UX consequence recorded: a signed op is spent-on-signing, never a revocable draft; the only kill switch is self-rotating the consumed serials first. | Spec P5.2, "Authorization threat model" |
| 4 | P5.2: bind op type + protocol version into the signing domain (P1) | **Done** — `NotePoolSigningHash` now includes `pool_protocol_version` (u8=1) and `op_type` (the PoolOp borsh tag). Renamed from `NotePoolTransferSigningHash` accordingly. | Spec P5.2 (preimage), P5.3 step 2 |
| 5 | P5.2: full canonicalization (sort key, duplicates, encoding, limits, ordering) (P1) | **Done** — lexicographic byte order over raw serials; duplicates anywhere in the consumed set = invalid (also added to P5.3 step 1); produced/outputs committed in serialized order; 1,000-item caps mirroring `max_tx_inputs`/`max_tx_outputs`; P5.1 gained a global encodings rule (LE integers, borsh collections, Blake2b domain-separated hashes, malformed = rejected not coerced). | Spec P5.1 + P5.2 |
| 6 | P5.2: complete anchor acceptance semantics (what's read, reorgs, delays, edges) (P1) | **Done** — key clarification: the anchor is a *pure integer*, no block reference, so "must it be in the selected-parent chain" and anchor-side reorg cases dissolve by construction; the only comparison is against the P5.3 context's POV DAA score; expiry is monotone; verdicts identical across nodes from `(payload, pov_daa_score)` alone. | Spec P5.2, "Anchor acceptance semantics" |
| 7 | P5.2: treat 36,000 as liveness/UX parameter; model 5min–24h regimes (P1) | **Done (classification + gate)** — explicitly classified as liveness/UX not security-derived; the five-regime modeling written into the P6.6 calibration requirement. The modeling itself is future work by design (needs a running testnet). | Spec P5.2 (freshness paragraph) |
| 8 | P5.1: qualify "uniqueness" as computational (P2) | **Done.** | Spec P5.1 |
| 9 | P5.1: canonical encodings consensus-defined (P1) | **Done** (see #5). | Spec P5.1 |
| 10 | P5.1: enumerate state invariants as premises for P5.2/P5.3 (P1) | **Done** — invariants I1–I5, explicitly referenced by the theorem's premises. | Spec P5.1 |
| 11 | P5.3: make the P5.2↔P5.3 dependency explicit (P1) | **Done** — "one argument split across two headings" paragraph; any Phase 6 change to ordering/diff semantics must re-check the theorem. | Spec P5.3 |
| 12 | P5.4: implementation-detail review (proof format, atomicity, bounds) (P1) | **Deferred to Phase 6 by design** — these are implementation-review items the reviewer himself labels "implementation-critical rather than objections"; the spec already fixes the architecture. Recorded as a Phase 6 review obligation. | This triage |
| 13 | P5.5: surface expiry window + early-broadcast risk in UX (P1) | **Done via #3's binding UX consequence** (signed = spent; invoice = payment-in-flight). | Spec P5.2 threat model (binding on P5.5/P5.6) |
| 14 | P5.6: recovery drills before launch (device loss, partial restore, divergent wallets…) (P1) | **Accepted as a launch-gate obligation** — flagged into FORK-PLAN's P8.5 (wallet threat pass). Not performable at spec time. | FORK-PLAN P8.5 flag |
| 15 | P5.7: graph-analysis study before privacy-forward positioning (P2) | **Accepted as a positioning gate** — no protocol change; recorded that public claims stay at P5.7's current qualified level until such a study exists. | This triage |
| 16 | P5.8: state the 3-of-5 trust boundary prominently (P0) | **Done** — promoted to the first paragraph of P5.8, including the IBD trust-root aspect. | Spec P5.8 (top) |
| 17 | P5.8: quantitative T/M/K sensitivity model, 10⁴–10⁸ sweep; don't finalize 10⁶ without it (P0) | **Accepted as a hard pre-launch gate** — the spec's calibration note escalated from "caveat" to "required model before parameter freeze," with the reviewer's estimate list verbatim and the attack-cost-vs-post-retirement-value comparison named. Gated into FORK-PLAN P9.5. The model itself requires real-world data (hardware/energy/rental markets near launch) and is deliberately not fabricated now. | Spec P5.8 + FORK-PLAN P9.5 flag |
| 18 | P5.8: define the IBD/bootstrap trust root, suppression/rollback resistance (P0) | **Done** — new subsection: software distribution as the (only) trust root, multi-peer anchor discovery, the **anchor ratchet** (persist highest-seen anchor, never accept a conflicting chain), suppression degrades toward the fail-open floor with alerting and never silently, residual eclipse risk named honestly. | Spec P5.8, "IBD / bootstrap trust root" |
| 19 | P5.8: operationalize trustee independence beyond geography (P1) | **Accepted — flagged into P9.1** (trustee ceremony): admission criteria across operators, jurisdictions, hosting, key custody/HSMs, signing software, incident response. | FORK-PLAN P9.1 flag |
| 20 | P5.8: 20-year sunset transition UX (P1) | **Done (consensus side) + flagged (UX side)** — stage visibility over RPC, post-expiry anchors have zero bootstrap relevance (ratchet stops applying); wallet/UX presentation deferred to P9.x launch materials. | Spec P5.8 (hard-maximum paragraph) |
| 21 | P5.8: equivocation evidence + disqualification lifecycle (P1) | **Done** — format, relay, objective verification (false evidence = just an invalid tx), exact activation point (from the including block onward, no retroactivity), storage as committed consensus state, permanence. | Spec P5.8, "Evidence and disqualification lifecycle" |
| 22 | Build conformance/property tests from the P5.2/P5.8 matrices (P1) | **Accepted** — the reviewer's 8-case P5.2 test matrix and the P5.8 boundary cases become Phase 6 conformance-test requirements; recorded here and in the P5.9 plan note. | This triage + FORK-PLAN P5.9 note |
| 23 | Fail-open visibility for wallets/users, not just node operators (P1) | **Done** — `finality_anchor_stale` required to be queryable over public RPC for wallet surfacing. | Spec P5.8 (fail-open) |

## What remains genuinely open after this triage

1. **Specialist sign-off** on the theorem (#1), the field matrix's one analyzed
   deviation (#2, group-set malleability), and the domain-separation adequacy — the
   artifacts now exist; a human cryptographer confirms or challenges them. Natural
   candidates: the same two reviewers, on the updated draft.
2. **The T/M/K quantitative model** (#17) — pre-launch gate at P9.5, needs real-world
   market data near launch time.
3. **Recovery drills** (#14) — P8.5, needs working wallet software.
4. **Graph-analysis study** (#15) — only if privacy claims are ever to go beyond
   P5.7's current qualified text.

Items 2–4 are launch-phase gates, not spec gates. Item 1 is the only thing between
v1.1-draft and tagging `pool-spec-v1.1` / closing P5.9.
