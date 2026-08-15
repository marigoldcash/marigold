# Spec reviews

This directory holds written external reviews of frozen specs, per FORK-PLAN.md's
review-gate steps (currently just P5.9).

## P5.9 — Pool spec review (open)

[docs/x-fork/POOL-SPEC.md](../POOL-SPEC.md) was frozen at tag `pool-spec-v1`
(commit `9d93ab32`) on 2026-08-15. Per the plan, it needs review by **at least one
person with an applied-cryptography background, outside the project**, before Phase 6
(consensus implementation) begins.

What the reviewer should look at:
- The full spec (`POOL-SPEC.md`), all eight sections (P5.1-P5.8).
- Particular attention to P5.2's signature scheme (`NotePoolTransferSigningHash`, the
  freshness anchor as the sole anti-replay mechanism) and P5.8's finality-anchor
  parameters (3-of-5 trustees, the T/M/K sunset trigger, the hard maximum DAA score) —
  the two places genuine new cryptographic/game-theoretic design decisions were made
  rather than existing Kaspa mechanisms extended.
- `docs/x-fork/DECISIONS.md`'s P5.1-P5.8 entries for the reasoning behind each
  non-obvious choice, if the "why" behind a specific number or mechanism isn't clear
  from the spec section alone.

When a review comes back, add it here as `pool-spec-v1-review-<reviewer-or-org>.md`
(or similar), triage its findings (what's fixed vs. deferred vs. disputed, with
reasoning), fold accepted changes into the spec as `v1.1`, and only then proceed to
Phase 6.
