# Reviewer 1's response to review 2 (cross-review concurrence)

Received 2026-08-15, relayed by the user. Reviewer 1 read review 2
([pool-spec-v1.1-review-2-james-o-connell.md](pool-spec-v1.1-review-2-james-o-connell.md))
against their own review and the spec. **Provenance note**: reviewer 1 was reading the
spec as of the v1.1-draft *before* commit `8226721a` — i.e. before the review-2 rigor
artifacts (field matrix, theorem, canonicalization, IBD ratchet, equivocation
lifecycle, threat model) were folded in. Several items this response asks for were
therefore already done by the time it arrived; see the disposition note at the end.

---

This is a strong, methodically structured review — arguably more systematically
rigorous than mine, in that it frames the specific issues I found as instances of
broader classes that need systematic treatment. Let me compare it against what I found
and against the spec itself.

On the Redeem signature gap (my P0, their P0): The reviewer notes this is already
fixed in "v1.1" — the signing hash now includes transparent_outputs_hash. So it looks
like the spec was revised after my review to address exactly the vulnerability I
flagged. Good. The reviewer then goes further than I did: instead of just saying
"cover the transparent outputs," they recommend a complete unsigned-field audit —
listing every transaction and pool-operation field excluded from the signing hash and
establishing for each that varying it cannot alter the economic meaning of the
authorization. This is the right generalization. My finding was one instance of the
class; the audit catches all of them. That recommendation alone justifies the review.

On equivocation (my second finding, their P1): The reviewer notes the spec already
defines overlap as "anchor DAA scores closer than the active cadence interval" — which
is the concrete proposal I suggested. So that was also addressed in revision. The
reviewer still correctly flags that the full lifecycle (evidence format, relay/storage,
acceptance ordering, false-evidence rejection, exact activation point of
disqualification) needs complete specification. That's an implementation-readiness
concern, not a design flaw.

On the freshness anchor: The reviewer's framing of pre-execution bearerability is
sharper than mine. I said the two-property defense (post-execution state change +
pre-execution expiry window) is "generally sound." The reviewer pushes further: a
captured signed operation is a bearer authorization before execution, and the design
needs to explicitly state whether early broadcast by a thief, relay, or malware is
acceptable as an inherent property of offline signed payment authorization. This is
the right question — it reframes the freshness anchor not as "anti-replay" (which
implies it fully prevents replay) but as "bounds the window of bearerability," which
is the honest description. The spec's P5.5 actually does discuss the settlement rule
and the shared-key window, but the reviewer is right that a formal threat model
stating "early broadcast by a holder of the signed payload is acceptable/unacceptable"
is needed.

On canonical encodings: This is a legitimate P1 the reviewer raises that I didn't
emphasize. The signing preimage includes sorted(group.serials) and op.produced, but
"sorted" without specifying the sort key, duplicate treatment, byte encoding, and
field widths is a consensus and interoperability risk. Different implementations
serializing the preimage differently would produce different hashes and cause silent
chain splits. The spec references borsh encoding for the payload but doesn't fully
specify the signing preimage's canonicalization. This is a real gap.

On domain separation: The reviewer suggests adding operation type and protocol version
to the signing domain (H("NotePoolSig" || pool_protocol_version || operation_type ||
...)). This is cheap defensive practice. The spec currently uses "NotePoolTransferSig"
for both Transfer and Redeem (since Redeem is structurally a Transfer variant). No
current exploit exists because transparent outputs are now bound and consensus rules
constrain operations, but adding operation-type domain separation prevents
cross-operation reinterpretation if future op types are added. Reasonable.

On T = 10⁶ × genesis difficulty: The reviewer and I raised the same concern — that a
relative multiplier can be numerically large while representing weak absolute security
if genesis difficulty is low. The reviewer's recommendation to model multiple
multipliers (10⁴ through 10⁸) against sustained hash-rate cost, energy cost, rentable
hash power, and attacker capital requirements is the right way to calibrate this.
Neither the spec nor my review supplied those estimates; the reviewer correctly flags
this as P0 work that must happen before implementation lock.

On IBD/bootstrap trust root: This is an area where the reviewer goes deeper than I
did. I noted that the IBD protection ("reject chains not building at-or-beyond the
latest anchored block") is the critical property, and that hardcoded trustee keys
mirror Kaspa's DNS seeder trust. The reviewer correctly points out that multiple
bootstrap peers improve availability but don't create cryptographic independence, and
asks for a design that handles adversarial peer scenarios — a fresh node presented
with only an old valid anchor, or a network adversary suppressing newer anchors. This
is a genuine design question: during IBD, what prevents an attacker who controls the
node's peer connections from feeding it a stale-but-valid anchor and a higher-work
chain that forks after that point? The spec says the node learns the "latest valid
anchor" from "hardcoded trustee keys, cross-checked against multiple independent
bootstrap peers/seeders" — but the reviewer is right that this needs more rigor.

Where the reviewer overreaches slightly: The P5.7 recommendations (P2 —
"obtain a graph-analysis study before any privacy-forward positioning") are reasonable but
perhaps over-calibrated given that P5.7 already explicitly says the system is "closer to 
cash than to Zcash's or Monero's" and doesn't overclaim. The
spec's honesty here is the correct posture; a graph-analysis study would strengthen
marketing claims, but the spec itself isn't making aggressive privacy claims that need
to be walked back. Still, as a "before you tell users this is private, measure how
private" recommendation, it's sound advice.

My overall take: This review complements mine well. My review found two specific
issues (Redeem coverage, equivocation precision); this review found the same issues at
a higher level of abstraction and then broadened them into systematic frameworks
(unsigned-field audit, authorization theorem, T/M/K sensitivity model, IBD trust-root
design) that catch more than the individual instances. The P0 items are all
legitimate. The priority ordering is defensible — the authorization theorem and field
audit come first because everything else in P5.2/P5.3 depends on them being correct.

If I had to pick the single most valuable contribution beyond what I covered, it's the
unsigned-field audit recommendation. The Redeem gap I found was one fish; the audit is
the net. The spec should implement that audit as a documented artifact before P5.9
closes.

The good news is that the two issues I flagged (Redeem coverage, equivocation overlap
definition) appear to have already been addressed in the v1.1 revision the reviewer
worked from — so the revision cycle is working.

---

## Disposition note (added at filing time)

Every actionable item this response endorses was already implemented in commit
`8226721a` (which predates this response's arrival but postdates the spec version
reviewer 1 read):

- **Unsigned-field audit** ("the net") → the signed/unsigned field matrix, P5.2.
- **Authorization threat model** (bearerability accept/reject statement) → P5.2,
  explicitly accepted with reasoning and the binding "signed = spent" wallet rule.
- **Canonical encodings for the signing preimage** → P5.1 global encoding rules +
  P5.2 canonicalization (sort key, duplicates, caps).
- **Domain separation** (version + op type) → `NotePoolSigningHash`, exactly the
  proposed shape.
- **Equivocation lifecycle** → P5.8, end to end.
- **IBD adversarial-peer rigor** → P5.8's anchor-ratchet subsection. Reviewer 1's
  sharpened question — "what prevents a peer-controlling attacker feeding a
  stale-but-valid anchor plus a higher-work chain forking after it?" — is answered
  there honestly: the node still enforces at-or-beyond the newest anchor it holds,
  staleness alerting fires (the presented tips are far ahead of the stale anchor), and
  protection degrades toward the plain-PoW fail-open floor, never below it and never
  silently; full eclipse of a fresh node remains the same residual risk all PoW IBD
  carries, named rather than claimed away.
- **T/M/K modeling** → hard pre-launch gate at P9.5 (not performable at spec time).
- **P5.7 calibration pushback** → concurs with our triage: the graph-analysis study
  was gated as a *positioning* requirement, not a spec change, matching reviewer 1's
  "the spec's honesty is the correct posture."

Both reviewers have now converged on the same priority list, and every
spec-addressable item on it is in the current draft. Remaining for P5.9 closure:
sign-off on the new artifacts themselves (theorem, field matrix — including the one
`[Open]` item, consumed-group-set malleability) by either or both reviewers.
