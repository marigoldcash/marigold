# Spec reviews

This directory holds written external reviews of frozen specs, per FORK-PLAN.md's
review-gate steps (currently just P5.9).

## P5.9 — Pool spec review (CLOSED 2026-08-15 — spec tagged `pool-spec-v1.1`)

[docs/marigold/POOL-SPEC.md](../POOL-SPEC.md) was frozen at tag `pool-spec-v1`
(commit `9d93ab32`) on 2026-08-15. Per the plan, it needs review by **at least one
person with an applied-cryptography background, outside the project**, before Phase 6
(consensus implementation) begins.

**Status (2026-08-15):**
- **Review 1 received and triaged**: [pool-spec-v1-review-1.md](pool-spec-v1-review-1.md),
  triage in [pool-spec-v1-review-1-TRIAGE.md](pool-spec-v1-review-1-TRIAGE.md). All seven
  substantive findings accepted and folded into the spec. Headline fix: the Redeem
  transaction-malleability vector (signature didn't cover transparent outputs).
- **Review 2 (James O'Connell) received and triaged**:
  [pool-spec-v1.1-review-2-james-o-connell.md](pool-spec-v1.1-review-2-james-o-connell.md),
  triage in [pool-spec-v1.1-review-2-TRIAGE.md](pool-spec-v1.1-review-2-TRIAGE.md). No
  new flaw; independently confirms the review-1 fix. Demanded rigor artifacts (theorem,
  field matrix, threat model, canonicalization, IBD trust-root design, equivocation
  lifecycle) — all now written into the spec; launch-phase demands gated into
  FORK-PLAN (P8.5, P9.1, P9.5).
- **Cross-review concurrence received**:
  [pool-spec-review-1-response-to-review-2.md](pool-spec-review-1-response-to-review-2.md)
  — reviewer 1 read review 2 and endorses its priority list ("the Redeem gap I found
  was one fish; the audit is the net"), confirms the revision cycle is working, and
  mildly pushes back only on review 2's P5.7 graph-analysis calibration (concurring
  with our triage's positioning-gate treatment). Every actionable item it endorses was
  already implemented in commit `8226721a`, which postdates the spec version reviewer 1
  read — see the disposition note appended to the filed response.
- **Confirmation pass received — gate closed**:
  [pool-spec-review-1-confirmation-pass.md](pool-spec-review-1-confirmation-pass.md) —
  reviewer 1 read the updated spec including all the new artifacts and delivered the
  explicit verdict: "None of these should block tagging v1.1 and closing P5.9."
  Tagged `pool-spec-v1.1`; P5.9 closed. Carried-forward non-blocking items (each with
  a home): consumed-group-set malleability (`[Open]` in the P5.2 field matrix → Phase
  6 standing review item), gas semantics (→ Phase 6), T/M/K model (→ P9.5 hard gate).
  If reviewer 2 later reviews the updated draft, findings fold into a v1.2 by the same
  triage process.

What the reviewer should look at:
- The full spec (`POOL-SPEC.md`), all eight sections (P5.1-P5.8).
- Particular attention to P5.2's signature scheme (`NotePoolTransferSigningHash`, the
  freshness anchor as the sole anti-replay mechanism) and P5.8's finality-anchor
  parameters (3-of-5 trustees, the T/M/K sunset trigger, the hard maximum DAA score) —
  the two places genuine new cryptographic/game-theoretic design decisions were made
  rather than existing Kaspa mechanisms extended.
- `docs/marigold/DECISIONS.md`'s P5.1-P5.8 entries for the reasoning behind each
  non-obvious choice, if the "why" behind a specific number or mechanism isn't clear
  from the spec section alone.

When a review comes back, add it here as `pool-spec-v1-review-<reviewer-or-org>.md`
(or similar), triage its findings (what's fixed vs. deferred vs. disputed, with
reasoning), fold accepted changes into the spec as `v1.1`, and only then proceed to
Phase 6.
