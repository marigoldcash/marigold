# Triage — pool-spec-v1 review 1

Review: [pool-spec-v1-review-1.md](pool-spec-v1-review-1.md), received 2026-08-15.
Triaged same day; accepted fixes folded into POOL-SPEC.md as **v1.1-draft** (see the
spec's version header for the change list). A second external review is pending; the
`pool-spec-v1.1` tag waits for it.

| # | Finding (severity per reviewer) | Disposition | Where fixed |
|---|---|---|---|
| 1 | **Redeem signature doesn't cover transparent outputs** — transaction-malleability vector: an interceptor of a signed `RedeemOp` payload can redirect the redeemed funds to their own transparent output (most important finding) | **Accepted, fixed.** Adopted the reviewer's own unification suggestion: the signing hash now always covers a `transparent_outputs_hash` (amount + script version + script per output, mirroring the existing sighash's output serialization, pool-domain-separated as `NotePoolOutputsHash`). For `Redeem` this binds the transparent destinations; for a pure `Transfer` it binds the output list to being empty — uniform, and future-proofs later op shapes. Explicitly documented why this is not circular (outputs don't contain the payload; only the payload contains the signature). | P5.2 (signing-hash section), P5.3 (validation step 2) |
| 2 | **Equivocation "overlap" needs a precise consensus definition** — inferred signing times, partition handling, adjacent-cadence anchors all underspecified (second priority) | **Accepted, fixed — via a stronger change than the review's minimal proposal.** Root cause was the v1 cadence being wall-clock-defined, which made "signing time" an inferred quantity at all. v1.1 redefines the cadence itself in DAA-score units (one anchor per 300-unit interval at launch): an honest trustee's anchors are then ≥ one interval apart in `anchored_daa_score` *by construction*, and the equivocation rule becomes the reviewer's proposed shape, now exactly decidable from on-chain data: same key, different `anchored_block`, `anchored_daa_score`s < one interval apart. Partition disagreement and adjacent-cadence anchoring are structurally excluded (documented case by case in the spec); the residual flaggable case — one key running two live signers — is misbehavior the rule *should* catch, with operational guidance (one live signer per key) flagged for the P9.1 trustee-ceremony docs. Anchor staleness (fail-open trigger) redefined in the same DAA-score terms (`depth + 3×interval`) for consistency. | P5.8 (anchor mechanics, equivocation, fail-open, decay table, attack cases) |
| 3 | Denomination-tag extension: hard fork or soft fork? Spec should state it explicitly (minor) | **Accepted, fixed.** Stated explicitly: assigning any reserved tag is inherently a hard fork — the tag→value table is referenced by conservation arithmetic and the pool commitment's meaning, so an un-upgraded node cannot validate ops using a new tag. | P5.1 (denomination-tag section) |
| 4 | Freshness-window boundary semantics: is `[0, 36000]` inclusive on both ends? (minor) | **Accepted, fixed.** Pinned: inclusive both ends — 0 and 36,000 valid, 36,001 invalid, future anchors invalid. | P5.3 (validation step 3) |
| 5 | `known_serials` loss with keys retained: recovery via pk-enumeration is possible but unstated (minor) | **Accepted, fixed.** Stated as an explicit recovery path, and reconciled with the "ownership tracked by serial, never pk" rule: restore-time enumeration is the one sanctioned pk-enumeration use, its output *becomes* the new explicit serial list, and Hot provenance already forces immediate rotation of everything recovered. | P5.6 (restore flow) |
| 6 | POS sweep is a recognizable fingerprint; batch-size variation / timing jitter worth mentioning as wallet-layer mitigation (observation) | **Accepted, added** — with the honesty constraint kept: noted as optional, best-effort, wallet-layer behavior that never softens the protocol-level claim (the protocol makes no attempt to hide this structure). | P5.7 (graph-structure passage) |
| 7 | T = 10⁶ × genesis difficulty: relative multiplier means absolute security depends on what genesis difficulty actually is; external review should sanity-check against realistic values (flag) | **Accepted, added** as an explicit calibration caveat alongside the existing one: before the parameter freezes for mainnet (P9.5 regenerates genesis), sanity-check that 10⁶ × the real genesis difficulty is plausibly-organic yet expensive-to-fake; the multiplier moves if not, the mechanism doesn't. | P5.8 (T definition) |
| — | Everything else: P5.1 structures, unified `TransferOp`, byte/mass estimates, freshness-anchor design, zero-input consensus-gap catch, P5.3 validation order and conflict resolution, P5.4 sync flow, P5.5 modes, P5.6 backup/POS/hazard handling, P5.7 honesty, P5.8 parameters and fail-open — reviewed with no changes requested | **No action** (confirmations). | — |

## Notes

- Finding 1 is the reason the review gate exists. It was invisible from inside the
  design's own framing ("Redeem is Mint read backwards" made the payload feel
  symmetric; the asymmetry was that `Transfer`'s destinations live *inside* the signed
  payload and `Redeem`'s live *outside* it) and obvious once stated from the outside.
- Finding 2's fix deliberately went one level deeper than requested. The review asked
  for a precise overlap criterion; the precise criterion only becomes *cleanly* precise
  once the cadence is DAA-keyed, so the cadence definition was changed rather than
  layering a fuzzy-tolerant rule on top of a wall-clock cadence.
- Nothing in the review was rejected or deferred. All seven substantive findings were
  accepted; the second pending review evaluates v1.1-draft including these changes.
