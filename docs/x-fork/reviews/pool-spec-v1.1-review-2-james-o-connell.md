# Technical Review: POOL-SPEC P5.1–P5.8

**Scope.** Technical review of the requested `POOL-SPEC.md` sections P5.1–P5.8 and the corresponding P5.1–P5.8 rationale in `docs/x-fork/DECISIONS.md`.

**Status.** Pre-implementation review.

## Evidence and notation

I went through the full POOL-SPEC.md (all P5.1–P5.8 material) and the corresponding decisions in DECISIONS.md, with the most scrutiny on P5.2 and P5.8 as you requested. I also checked the v1.1 changes called out at the top of the spec, because those materially affect the cryptographic review.

- **[Open]** identifies an item that needs source confirmation or further review before a design decision is treated as closed.

## Executive summary

The design is mature in its reuse of established chain mechanisms: serials resemble UTXO outpoints, conflict resolution is composed into GHOSTDAG state, and pruning/synchronization use a committed state plus sparse-Merkle proofs. The specification’s privacy limitations are also unusually candid. These are material strengths.

The two novel decision areas are not ready to be described as cryptographically or game-theoretically complete. P5.2 makes the freshness anchor central to the lifetime of an offline authorization, while P5.8 introduces a temporary trustee-based finality root and a parameterized sunset. Neither choice presents an obvious catastrophic flaw in the reviewed material, but both need explicit specialist analysis before implementation is frozen.

### Overall disposition

| Area | Disposition | Priority |
|---|---|---|
| P5.1 data model, serials, SMT | Sound architectural direction; tighten security language | P2 |
| P5.2 signature coverage and replay model | Plausibly sound but needs a written authorization/replay theorem and field-by-field coverage audit | P0 |
| P5.3 conflict handling | Strong use of existing composed-state ordering; prove interaction with P5.2 invariants | P1 |
| P5.4 pruning and sync | Strong architecture; implementation/proof review still required | P1 |
| P5.5 transfer modes | Clear separation of consensus and wallet semantics | P1 |
| P5.6 wallet/recovery | Cryptographically coherent; operationally high-risk | P1 |
| P5.7 privacy | Claims are responsibly bounded; quantify real-world anonymity separately | P2 |
| P5.8 finality anchors and sunset | Coherent construction; governance, bootstrap, and economic assumptions require specialist sign-off | P0 |

**Priority definitions:** **P0** must be resolved or explicitly accepted before implementation lock. **P1** should be resolved before launch or treated as an explicit design constraint. **P2** improves precision, safety, or operability but is not a demonstrated exploit.

## Design-level strengths

- [Source: P5.1/P5.3] The pool state is integrated with existing chain-state composition rather than introducing a parallel double-spend ordering system.
- [Source: P5.2] v1.1 binds Redeem transparent outputs into the signing hash, closing the obvious destination-substitution gap.
- [Source: P5.4] The state-commitment plus sparse-Merkle-proof sync model has a clear integrity story across pruning.
- [Source: P5.5] The specification keeps recipient key-delivery modes at the wallet layer instead of inventing consensus-visible transfer types.
- [Source: P5.7] The privacy section does not overclaim shielded-pool properties.
- [Source: P5.8/DECISIONS] DAA-score parameters, deterministic equivocation semantics, fail-open behavior, and an absolute trustee sunset are all better-defined than informal timing or governance rules would be.

## P5.1 — Data model, serials, and commitments

### Source-derived observations

- [Source: P5.1] A note serial is described as `H_serial(creating_tx_id || output_index)`.
- [Source: P5.1] Pool state is committed using an SMT-style commitment.
- [Source: P5.1] The serial construction is intended to provide globally distinguishable note identities without additional consensus uniqueness state.

### Review findings

**P2 — qualify “uniqueness.”** A hash-derived serial is not mathematically unique. The defensible statement is computational uniqueness under the collision resistance of the transaction-ID and serial-hash constructions. This is a documentation precision issue, not an identified practical collision attack.

**P1 — define canonical encodings.** [Open] The reviewed excerpt does not establish exact byte encodings, field widths, endianness, hash function identifiers, or rejection rules for malformed encodings. These must be consensus-defined. A correct high-level serial formula is insufficient when different implementations can serialize it differently.

**P1 — establish state invariants.** The specification should enumerate invariants: serial membership/non-membership, one current key per live serial, denomination/value constraints, and the exact consequences of rotation and redemption. Those invariants become the premises for P5.2 and P5.3 replay reasoning.

## P5.2 — Transfer signing and freshness-anchor analysis

### Construction under review

[Source: P5.2] The reviewed signing construction is:

```text
NotePoolTransferSigningHash = H(
  "NotePoolTransferSig" || sorted(group.serials) || op.produced ||
  transparent_outputs_hash || freshness.anchor_daa_score
)
```

[Source: P5.2/P5.3] The construction is used for Transfer and apparently also for Redeem; v1.1 adds `transparent_outputs_hash` so a Redeem destination cannot be substituted while leaving the pool payload unchanged.

### What the signature appears to authorize

Subject to canonical-encoding confirmation, this preimage commits to the authorizing serial group, the complete produced-note set, transparent outputs, and the freshness anchor. That is the correct basic shape for an authorization hash. Binding transparent outputs is necessary: without it, an attacker who obtains a valid signature could potentially alter the transparent redemption destination.

### P0 findings and questions

**P0 — write and review the central authorization theorem.** The security claim should be stated precisely:

> Given a valid signature for serial group `G` anchored at `A`, an adversary who obtains the signed operation cannot cause an unintended pool-state transition. Before expiry, the adversary can at most cause the exact transition authorized by the signed message; after acceptance, the same authorization cannot be accepted again.

The claimed result depends on more than freshness. It requires (1) verification against the current key in the composed pool state, (2) key rotation or serial removal after execution, (3) all economically meaningful effects being bound or consensus-determined, and (4) unambiguous encoding.

**P0 — perform an unsigned-field audit.** [Open] List every transaction and pool-operation field excluded from `NotePoolTransferSigningHash`: transaction version, operation discriminant, lock time, fee, transparent inputs, ordering rules, ancillary payloads, and any consensus metadata. For each, establish one of two outcomes: it is committed by the signature, or varying it cannot alter the economic/security meaning of the authorization. This is the most important P5.2 review artifact.

**P1 — bind operation type and protocol version explicitly.** The same domain tag, `"NotePoolTransferSig"`, appears to cover both Transfer and Redeem. No theft path is demonstrated from the reviewed material because transparent outputs are now committed and consensus rules constrain the operation. Still, cheap domain separation is preferred:

```text
H("NotePoolSig" || pool_protocol_version || operation_type || ...)
```

or separate tags such as `NotePoolTransferSig` and `NotePoolRedeemSig`. This protects against cross-operation reinterpretation following future extensions.

**P1 — specify group and collection canonicalization.** [Open] `sorted(group.serials)` must specify sorting key, duplicate treatment, byte encoding, limits, and whether `op.produced` and transparent outputs are ordered canonically. “Sorted” without a complete canonicalization rule is a consensus and interoperability risk.

### Freshness anchor: what it does and does not do

[Source: P5.2/DECISIONS P5.2] The design deliberately does not sign `tx_id` because the signature resides in the transaction payload and would make that construction circular. It instead relies on executed signatures becoming unusable through state change and unexecuted signatures expiring after 36,000 DAA-score units (described as about one hour). The same window also functions as an invoice-expiration interval for sign-to-fresh-pk.

This is an elegant replacement for a transaction-ID commitment, but it should not be described as conventional anti-replay protection in isolation. Before first execution, a captured valid signed operation is a bearer authorization: a party that obtains it can broadcast the exact authorized operation. After execution, it becomes unusable because the current key has changed or the serial no longer exists. The freshness anchor limits the period of pre-execution bearerability; state transition supplies post-execution replay exclusion.

**P0 — define the authorization threat model.** [Open] The design must explicitly say whether early broadcast by a thief, relay, merchant, or malware is acceptable. If it is acceptable, say so as an inherent property of offline signed payment authorization. If it is not acceptable, this construction needs another control (for example, recipient interaction, an external channel binding, or a different transaction model).

**P1 — define anchor acceptance semantics completely.** [Open] Specify inclusive/exclusive expiry comparison; what anchor a validator reads; whether an anchor must be in the selected-parent chain; behavior at reorg boundaries; allowed future anchors; behavior when the operation is delayed; and any DAA-score edge cases. Every node must derive the same validity result.

**P1 — treat 36,000 as a liveness/UX parameter, not a derived security constant.** The cited rationale—payments usually settle quickly, an hour leaves headroom, and abandoned authorizations should expire within a session—is sensible. It does not make 36,000 cryptographically optimal. Model 5-minute, 15-minute, 1-hour, 6-hour, and 24-hour regimes against congestion, partitions, merchant retry flows, and signature theft exposure.

### Recommended P5.2 proof/test matrix

1. Same signed payload broadcast twice in different blocks/blue order.
2. Two conflicting rotations from the same key and serial group.
3. Redeem destination mutation, amount/value mutation, and output-order mutation.
4. Transfer/Redeem cross-interpretation attempt.
5. Duplicate serial, differently encoded serial, and noncanonical sorting attempts.
6. Freshness boundary at `A + 36,000`, one score before, and one score after.
7. Reorg around the selected anchor and delayed transaction admission.
8. Mempool rebroadcast, replacement, and concurrent relay behavior.

## P5.3 — GHOSTDAG conflict handling

### Source-derived observations

[Source: P5.3] Pool validation uses selected-parent pool state plus accumulated `PoolDiff`, and blue-topological ordering determines which conflicting operation is accepted. A signature is checked against the serial’s current public key in the composed pool view.

### Review finding

**P1 — strong architectural reuse.** Reusing composed state and established GHOSTDAG ordering is materially safer than introducing a pool-specific conflict-resolution mechanism. If a first operation rotates a serial’s key, a conflicting later operation verifies against the changed key and fails; two competing rotations cannot both succeed in one composed state.

**P1 — make the dependency explicit.** P5.2 replay safety is only as strong as this current-key/state-transition invariant. The spec should connect the P5.2 theorem directly to the P5.3 ordering and validation algorithm, including block-local ordering and every reorg/rebuild path.

## P5.4 — Pruning, state commitment, and synchronization

### Source-derived observations

[Source: P5.4] Pool state survives pruning analogously to UTXO state; a dedicated `pool_commitment` commits to it; SMT inclusion proofs support incremental streamed-state validation; and a final root comparison provides an additional integrity check. The decision record prefers a dedicated header commitment over overloading `accepted_id_merkle_root`.

### Review finding

**P1 — good structure; verify implementation details.** Separating semantic commitments is a clean choice. The important remaining review questions are proof format, domain separation, leaf/value serialization, empty-tree behavior, atomicity with normal state updates, and resource bounds for streamed sync. These are implementation-critical rather than objections to the architecture.

## P5.5 — Transfer modes and settlement semantics

### Source-derived observations

[Source: P5.5] Bearer mode and sign-to-fresh-pk are wallet/UX concepts rather than consensus-visible transaction classes. The universal settlement rule is that a recipient owns a note only after it rotates to a key known only to that recipient and the rotation confirms.

### Review finding

**P1 — clear trust semantics.** This cleanly distinguishes “who currently knows a key” from consensus authorization. Bearer mode necessarily has a shared-key period. It should remain explicit in UX and APIs that possession of a copy of a bearer key does not provide exclusivity until the recipient’s confirmed rotation.

**P1 — couple P5.5 to P5.2 disclosure risk.** Sign-to-fresh-pk should communicate the authorization-expiry window and the risk that a signed authorization can be broadcast by any holder before expiry. This is a product and protocol-boundary requirement.

## P5.6 — Wallet model, backups, and recovery

### Source-derived observations

[Source: P5.6] Notes are not reconstructed from a seed; losing the note-key database loses the notes. The backup design includes encrypted XChaCha20-Poly1305 backups, explicit serials, recovery against plaintext PoolState, and stale-backup revocation through rotation.

### Review finding

**P1 — operational risk dominates.** The construction can be internally consistent while remaining difficult for ordinary users. The critical question is whether users can reliably identify the keys representing their money, recover partial state safely, and avoid divergent wallet copies creating conflicting transactions.

**P1 — require recovery drills.** Before launch, exercise device loss, partial restore, rollback, stale-backup restore, concurrent wallets, and backup theft. Document the user-visible outcome and the remediation path for each. Cryptographic backup encryption does not solve key-management failure modes.

## P5.7 — Privacy claims and practical anonymity

### Source-derived observations

[Source: P5.7] PoolState, serials, denominations, and operation history are public; pruning does not erase historical observability; mint/redeem link transparent history; timing, denominations, and fee-stamp ancestry are linkable. The specification does not describe the system as shielded or Monero/Zcash-like privacy.

### Review finding

**P2 — the claims are responsibly scoped.** The cited “at best roughly every other currently-live note of the same denomination” is an upper-bound intuition, not a measurement of real-world anonymity. Timing, transaction batching, merchant sweeps, denomination patterns, mint/redeem timing, and fee-stamp ancestry can shrink the effective set substantially.

**P2 — obtain a graph-analysis study before privacy-forward positioning.** This is not necessarily a protocol blocker. It is necessary if the product intends to make meaningful privacy claims beyond the carefully qualified text already in P5.7.

## P5.8 — Finality anchors and game-theoretic review

### Source-derived observations

[Source: P5.8/DECISIONS P5.8] The reviewed design uses 3-of-5 trustees; initial cadence is 300 DAA and anchor depth is 600 DAA. Cadence is DAA-score based. Trustee failure is fail-open: absence of an anchor does not halt the chain, but removes the additional protection. Equivocation is permanently disqualifying under a consensus-defined rule involving the same trustee key, different anchored block, and anchor DAA scores closer than the active cadence interval.

[Source: P5.8/DECISIONS P5.8] Trustee retirement is based on a dual T/M/K condition: difficulty at least `T` for a sustained six-month period and at least five years since genesis. The cited `T` is `10^6 × genesis difficulty`. A hard maximum of 6,311,520,000 DAA-score (20 years) ends trustee relevance even if the earlier condition never triggers.

### Security model and strengths

**P0 — state the trust boundary plainly.** Three compromised or colluding trustees can sign a false anchor. This is not an implementation defect; it is the irreducible security assumption of 3-of-5 finality. The design should state this prominently in both the protocol and bootstrap threat models.

**P1 — 3-of-5 is a defensible availability/security tradeoff.** It tolerates two unavailable trustees while requiring three to fabricate an anchor. Its actual security is not “five names,” however; it is independence across legal jurisdiction, hosting, upstream networks, key-management systems, operators, supply chains, and coercion risks.

**P1 — DAA cadence and deterministic equivocation are good consensus choices.** DAA avoids wall-clock ambiguity. Defining equivocation in terms of consensus-visible interval rules is a substantial improvement over an informal overlapping-time-window test. Confirm all boundary cases and cadence-transition rules are consensus complete.

**P1 — fail-open needs a user-visible risk model.** Fail-open avoids converting trustee outages into chain halts. During an outage the system falls back to PoW-only protection. Node/wallet behavior should make extended missing-anchor intervals observable; otherwise users may believe finality guarantees still apply.

### P0 questions: T/M/K sunset and economics

**P0 — quantitatively justify `T = 10^6 × genesis difficulty`.** A relative multiplier can be numerically large while representing weak absolute security if genesis difficulty is low. The cited material acknowledges that `T` needs economic justification. Before freezing it, model at least `10^4`, `10^5`, `10^6`, `10^7`, and `10^8 × genesis` under plausible launch conditions.

For each value, estimate: sustained hash-rate/hardware cost; energy cost for six months; rentable hash-power availability; expected organic trajectory; ability to manipulate difficulty; attacker capital requirements before and after the threshold; and probability the network reaches the threshold without artificial stimulation. This report does not supply those estimates.

**P0 — analyze difficulty manipulation over the full window.** Requiring every difficulty-window checkpoint during the six-month trailing period to meet `T` is materially stronger than endpoint checking. It does not remove the core question: can a well-funded actor mine enough to sustain the measured difficulty, trigger trustee retirement, and retain sufficient resources to exploit the weakened period? That is the key game-theoretic challenge.

**P0 — define the IBD/bootstrap trust root.** The IBD rule reportedly obtains the latest valid anchor before judging competing chains and rejects a higher-work chain that does not build at or beyond it. This turns trustee signatures and anchor discovery into a bootstrap trust root. Multiple bootstrap peers can improve availability but do not create cryptographic independence. Specify how a fresh node authenticates anchors, detects suppression/rollback, chooses among valid anchors, and handles a network adversary presenting only an old valid anchor.

### Additional P5.8 findings

**P1 — operationalize trustee independence.** Establish admission and rotation criteria: separate operators, jurisdictions, cloud providers, key custody/HSMs, signing software, incident response, contact paths, and disclosure policy. A geographic distribution criterion alone is insufficient.

**P1 — 20-year hard sunset is a governance strength, with a transition obligation.** The hard maximum prevents launch trustees from silently becoming permanent infrastructure and supplies a deterministic endpoint. The protocol and UX still need a plan for the final period: how nodes present the declining protection, how users understand the transition, and whether old anchors retain any bootstrap relevance after retirement.

**P1 — specify equivocation evidence and disqualification lifecycle.** [Open] Define signature format, evidence relay/storage, acceptance ordering, false-evidence rejection, the exact activation point of disqualification, and whether a disqualified key can ever be replaced. Permanent disqualification is only safe when evidence is objectively and deterministically verifiable.

## Consolidated questions that need to be checked/verified:

### Cryptography / consensus

1. Can the P5.2 authorization theorem be proved under the actual validation rules?
2. Which unsigned fields can alter economics, recipient, value, ordering, or validity?
3. Is the domain separation sufficient across Transfer, Redeem, versions, and future operations?
4. Are all serialization, ordering, duplicate, and boundary rules canonical and consensus-specified?
5. Does composed-state validation make repeated or conflicting signatures fail in every reorg and block-order path?
6. Is pre-execution bearerability acceptable for all target payment flows?
7. Does P5.4 commitment/sync serialization provide unambiguous, bounded, atomic state validation?

### Game theory / governance / bootstrap

1. What exact attacker capabilities does 3-of-5 protect against, and what three-party compromise scenarios remain plausible?
2. What measurable trustee-independence requirements are needed?
3. Can `T` be tied to credible absolute attack cost rather than only a genesis-relative multiplier?
4. Can an attacker economically sustain a qualifying difficulty window and then benefit from retirement?
5. How are long missing-anchor intervals surfaced and reasoned about?
6. What does a brand-new node authenticate before accepting an anchor chain, and how does it resist rollback/suppression?
7. Are cadence changes, equivocation boundaries, evidence propagation, and disqualification deterministic?

## Recommended actions before implementation lock

1. **P0:** Produce a short formal security note for P5.2: state machine, adversary model, signing preimage, validation algorithm, theorem, and proof sketch.
2. **P0:** Produce an explicit signed/unsigned field matrix for every pool operation and enclosing transaction field.
3. **P0:** Commission or perform a quantitative T/M/K sensitivity model. Do not label `10^6 × genesis difficulty` final without its results.
4. **P0:** Write a fresh-node/IBD anchor-authentication and rollback-resistance design, including adversarial peer scenarios.
5. **P1:** Add operation type and protocol version to the signing domain, or document a specialist-approved reason not to.
6. **P1:** Publish canonical encoding and boundary semantics for signatures, serial groups, outputs, freshness, SMTs, anchors, cadence, and equivocation.
7. **P1:** Specify trustee independence, key-management, operational response, and disqualification procedures.
8. **P1:** Build conformance/property tests from the P5.2 and P5.8 matrices in this report.
9. **P1:** Run wallet recovery and stale-backup drills before release.
10. **P2:** Conduct practical graph-analysis/privacy measurement before expanding public privacy claims.

## Source reference map

The following references identify the intended source locations; they are section-level because the attached originals were not available for fresh line verification in this workspace.

- `POOL-SPEC.md`, P5.1 — note data model, serial construction, SMT commitment.
- `POOL-SPEC.md`, P5.2 — `NotePoolTransferSigningHash`, signing coverage, freshness anchor, 36,000 DAA window, v1.1 transparent-output binding.
- `POOL-SPEC.md`, P5.3 — composed pool state, `PoolDiff`, blue-topological conflict handling.
- `POOL-SPEC.md`, P5.4 — pruning, `pool_commitment`, SMT proofs, streamed sync.
- `POOL-SPEC.md`, P5.5 — bearer mode, sign-to-fresh-pk, settlement rule.
- `POOL-SPEC.md`, P5.6 — wallet state, encrypted backups, recovery and stale-backup rotation.
- `POOL-SPEC.md`, P5.7 — public observability, anonymity limits, fee-stamp linkage, non-shielded positioning.
- `POOL-SPEC.md`, P5.8 — trustee anchors, cadence/depth, failure/equivocation, IBD, T/M/K, hard maximum DAA score.
- `docs/x-fork/DECISIONS.md`, P5.1–P5.8 entries — rationale for the corresponding non-obvious design decisions, particularly P5.2 freshness and P5.8 parameterization.

## Closing assessment

The construction appears to have a strong architectural core and avoids several familiar specification mistakes: it binds Redeem outputs in v1.1, reuses consensus ordering, distinguishes state commitment from transaction commitment, and avoids overstating privacy. The remaining work is disciplined rather than speculative: prove the P5.2 authorization boundary, quantify the P5.8 retirement economics, and make the trustee/IBD trust roots explicit. These are the items that should determine whether the design moves from promising specification to implementation-ready protocol.
