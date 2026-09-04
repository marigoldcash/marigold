# Reviewer 1's confirmation pass on the updated spec (v1.1-draft with all artifacts)

Received 2026-08-15, relayed by the user. This is the confirmation pass the triage
process was waiting for: reviewer 1 read the **updated** spec — including the review-2
rigor artifacts (field matrix, authorization theorem, canonicalization rules, IBD
anchor ratchet, exact equivocation definition) that neither reviewer had seen when
their prior documents were written. **Verdict, quoted: "None of these should block
tagging v1.1 and closing P5.9."**

---

This is a good position to be in. Let me assess what the coder said and what the
revised spec shows.

The coder's three points are all accurate:

1. Timing artifact — confirmed. Looking at the v1.1 spec, the signed/unsigned field
matrix (the thing I called "the most valuable contribution beyond what I covered" from
review 2) is already present as a complete table in P5.2. The authorization theorem,
the threat model, the canonicalization rules — all there. Review 2's recommendations
weren't open work items; they were confirming what the revision already did.

2. Reviewer convergence — the strongest signal. Two independent reviews with different
methodologies (mine found concrete bugs, O'Connell's demanded systematic frameworks)
arrived at the same priority list. And the v1.1 spec addresses both styles: the
concrete fix (transparent outputs in the signing hash) and the systematic framework
(the field matrix, the theorem, the encoding rules). The one point of divergence
between the reviews — whether the P5.7 graph-study is over-calibrated — the
coder triaged exactly as I would have: a gate on future marketing claims, not a spec
change.

3. The IBD question — the v1.1 spec's IBD section directly answers my question about
stale-anchor eclipse attacks. The anchor ratchet (persist highest-scoring valid
anchor, never accept a conflicting chain) is the right mechanism, and the honest
acknowledgment that "a fully eclipsed fresh node fed an attacker chain plus no recent
anchors is the same residual eclipse risk every PoW chain's IBD already carries, now
with an alarm attached" is exactly the framing I'd want. It names the risk rather than
claiming the mechanism eliminates it.

On the revised spec itself, a few observations:

The equivocation definition is now crisp: `|anchored_daa_score_1 −
anchored_daa_score_2| < interval` with the interval being the active stage's cadence.
The structural exclusion of honest partition behavior is well-argued — a partition
brief enough that both sides' DAA scores stay within one interval can't produce two
honest signatures from one trustee (they're on one side of a partition at a time), and
a longer partition produces scores outside the rule. Clean.

The consumed-group-set malleability analysis in the field matrix is the one genuinely
open item (`[Open]` flag), and it's correctly characterized: fee-stripping (removing
another party's group) is a griefing vector, not theft, and the deliberate trade-off
(allowing collaborative fee attachment) is reasonable. Worth the specialist
confirmation pass, but not alarming.

The T/M/K sensitivity model being promoted from a "calibration note" to a hard
pre-launch gate (gated into P9.5) is the right elevation. Review 2's framing —
"compare attack cost against post-retirement extractable value, not just declare the
window expensive" — is the question that actually matters, and it's now explicitly
named as a requirement.

My assessment of where things stand: the spec is in good shape for a confirmation
pass. The two P0 items from review 2 (authorization theorem, field audit) are present
as written artifacts. The two P0 items I'd add (the Redeem fix and the equivocation
precision) are addressed. The remaining open items are: the consumed-group-set
malleability (needs specialist sign-off, not a redesign), the gas-semantics
confirmation (small, implementation-phase), and the T/M/K quantitative model
(correctly deferred to pre-launch, not pre-implementation). None of these should block
tagging v1.1 and closing P5.9.

---

## Disposition note (added at filing time)

Acting on the explicit verdict: `pool-spec-v1.1` is tagged and P5.9 closed. The three
items reviewer 1 lists as remaining-but-non-blocking are carried forward as standing
obligations, each with a home:

- **Consumed-group-set malleability** — stays `[Open]`-flagged in the P5.2 field
  matrix; carried into Phase 6 as a standing review item (specialist sign-off wanted;
  redesign not expected). Any Phase 6 implementation choice touching group/fee
  semantics re-raises it.
- **Gas semantics for user-lane txs** — implementation-phase confirmation, flagged in
  the field matrix; resolves during P6.1-P6.4 against real code.
- **T/M/K quantitative model** — pre-launch hard gate at P9.5, unchanged.

Review 2's author (O'Connell) has not separately counter-signed the updated draft;
the plan's requirement ("at least one person with applied-cryptography background
outside the project") is met by reviewer 1's full cycle (review → revision →
confirmation). If O'Connell later reviews the updated draft, any findings fold into a
v1.2 by the same triage process — the tag doesn't foreclose that.
