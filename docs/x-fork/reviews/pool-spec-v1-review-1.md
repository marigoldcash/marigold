
This is a genuinely well-structured spec — the discipline of "every decision cites the real codebase precedent it extends, not an analogy" is unusual and valuable. Most of what follows is confirmation that the design holds up under scrutiny, with a few issues of varying severity flagged.

# P5.1 — Data structures
Solid. The sn = H(creating_tx_id || output_index) derivation is the right call — it mirrors the existing TransactionOutpoint pattern, inherits collision resistance from tx-id uniqueness, and requires zero consensus state. The SMT leaf as H_leaf(d || pk) (33-byte preimage) is clean, and the proof model (present (d, pk), recompute hash, check against proven leaf) is standard.

One minor observation: the spec says tags 8–255 are reserved, but doesn't specify whether a future denomination-set change is a soft fork or hard fork. Given that the pool commitment and validation rules reference the tag→value table, adding tags would require every node to know the new mapping — inherently a hard fork. Probably obvious, but worth stating explicitly so nobody later assumes tag extensibility is somehow backward-compatible.

No issues with the pool_commitment as a new header field. The decision not to overload accepted_id_merkle_root is correct — one field meaning two different things depending on activation is exactly the kind of ambiguity that causes chain-split bugs.

# P5.2 — Transaction format and signature scheme
## The unified TransferOp design
Collapsing rotate/split/merge into one wire shape is elegant and well-justified — the plan's own framing already pointed at this. The worked byte-size estimates are thorough and the mass calculations are comfortably within block budgets. No concerns here.

## NotePoolTransferSigningHash — a significant gap in Redeem coverage
This is my most substantive finding. The signed message is:

´´´
H("NotePoolTransferSig" || sorted(group.serials) || op.produced || freshness.anchor_daa_score)
´´´

For Transfer, this is sound: every group signs over the complete produced list (all destination notes), which binds the transaction atomically — no group's signature can be lifted into a transaction with different produced notes. Removing or adding groups from consumed doesn't break anything meaningful (removing reduces value-in, potentially failing conservation; adding donates the attacker's own value). The produced list is the thing the signers care about, and it's covered. Good.

__For Redeem, there is a real gap.__ The RedeemOp struct is:
´´´
struct RedeemOp {
    consumed:  Vec<SignedGroup>,
    freshness: FreshnessAnchor,
}
´´´
There is no produced field — the "output" of a Redeem is the transaction's transparent outputs (tx.outputs), which live outside the pool-op payload entirely. The signing hash references op.produced, which for a Redeem is empty or nonexistent. __The signature therefore does not cover the transparent outputs where the redeemed value lands.__

Concretely: if Alice signs a RedeemOp consuming her 100-MAGLD note, intending the transparent output to pay her own address, anyone who obtains the signed payload before broadcast can reconstruct the transaction with a different transparent output address (same consumed serials, same freshness anchor, same signature) and the signature still verifies, the conservation check still passes (Σ(consumed notes) ≥ Σ(new transparent outputs)), and Alice's funds are redirected. This is a transaction-malleability vulnerability — not a signature forgery, but the signature authenticates less than it should.

The asymmetry is telling: for Transfer, the signer authenticates where value goes (the produced notes' pks are in the signed message). For Redeem, the signer authenticates that their notes are consumed but not where the resulting transparent value goes. The fix is straightforward — include a hash of the transaction's transparent outputs (or at least their scripts+amounts) in the signing message for Redeem. Arguably this could be unified by having the signing hash always cover the enclosing transaction's transparent I/O, which would also future-proof against any later op type that interacts with the transparent side.

I'd flag this as the single most important item for the external cryptographic reviewer to examine.

## Freshness anchor as sole anti-replay — generally sound, one edge case
The two-property replay defense (post-execution: serial's pk changes, signature permanently invalid; pre-execution: 36,000-unit window bounds shelf life) is well-reasoned. The choice of DAA-score rather than wall-clock for the window is correct — it's consensus-deterministic, and during a network stall, no transactions can be included anyway, so the extended wall-clock validity is harmless.

One subtle edge: the freshness check rejects anchor_daa_score > pov_daa_score (future anchors) but I didn't see explicit handling of the case where pov_daa_score - anchor_daa_score is exactly 36,000. The spec says "in [0, 36000]" — inclusive on both ends, presumably. This is a minor boundary-condition specification detail, but worth pinning down precisely since consensus rules need exact boundary semantics.

## The consensus-rule gap (zero-input transactions)
Excellent catch on check_transaction_inputs_count rejecting zero-input non-coinbase transactions. This is exactly the kind of thing that would cause a confusing test failure during Phase 6 if not flagged now. The proposed exemption (mirroring the coinbase precedent) is the natural shape.

# P5.3 — Consensus rules
The validation ordering is correct: existence + shared-pk check → signature verification → freshness → denomination validity → conservation → apply diff. You can't verify the signature before knowing the current pk (step 1 must precede step 2), and you can't check conservation before knowing denominations (step 4 must precede step 5). The PoolDiff application as the final step (after all validation passes) is the right pattern — mirrors how UTXO diffs work.

The parallel-blocks double-rotate resolution via GHOSTDAG blue-topological order with a composed pool view is the correct extension of the existing mechanism. "First accepted wins, loser becomes a no-op, not an invalid block" — this is exactly how Kaspa already handles UTXO conflicts, and extending it to pool state is sound.

The mass/fee costing model — one sigop per SignedGroup, not per serial — correctly makes batch sweeps cheap by design. The fact that a 20-serial merchant sweep costs one signature verification (not twenty) is a deliberate economic property, and it's properly justified.

# P5.4 — Pool state sync & pruning interaction
The choice to model the sync flow on the seq-commit SMT import (flow 2) rather than the UTXO MuHash import (flow 1) is architecturally correct and gives a genuine security advantage: per-chunk incremental proof verification catches malicious data as it arrives, not only after the full download. The spec correctly identifies this as an inherited benefit of the SMT design, not something the pool feature had to engineer.

The sync flow description is complete end-to-end and cites real precedents. No concerns.

# P5.5 — Transfer modes
Both modes reducing to the identical TransferOp is clean. The universal settlement rule ("a note is finally yours when a rotation to a key only you know is confirmed on-chain") is correct and reduces to the existing double-spend resolution rather than needing a new mechanism.

The distinction between the two exposure windows is well-articulated:

__Bearer mode:__ shared-key window, receiver-controlled, unbounded in consensus (correct — a paper note in a drawer for a year should still be valid, like cash)
__Sign-to-fresh-pk:__ freshness window, consensus-bounded (~1 hour), doubling as invoice/QR expiry
The observation that "how long do I show this QR" and "how long is a signed-but-unsent payment still valid" are the same parameter is a nice insight that prevents wallet implementers from tuning two independently when they should be one number.

# P5.6 — Wallet protocol
Comprehensive. A few observations:

The paper backup format (chunked, XChaCha20Poly1305, QrPageHeader for missing-page detection without the password) is well-designed. The restore flow — "look up the serial's current owner in PoolState; if pk matches, it's still yours" — is exactly the simplicity the plaintext pool enables, and the observation that "rotation doubles as backup revocation" (a full self-sweep invalidates every prior backup at once) is a real recovery-action primitive worth highlighting.

The POS landing-pad flow (sweep per confirmation, not end-of-day) is the correct security posture. The invariant that bearer handover requires a solo key (revealing a shared sk hands over all notes under that pk) is properly flagged, and the recommendation that personal wallets default to fresh-pk-per-note is good defensive guidance.

The same-key-in-two-wallets hazard is handled sensibly: ownership tracked by serial (never inferred from pk enumeration), and KeyProvenance (Cold vs. Hot) determining rotation urgency. The key-algorithm-deprecation story — ForkActivation-gated, blocks new deprecated-format notes while allowing rotation away — mirrors existing Kaspa address-version patterns and is correctly scoped as "mechanism for future use, not a second format to support today."

One small gap: the spec discusses wallet backup of keys but doesn't address backup of the known_serials list itself. If a wallet loses its known_serials (but retains the private keys), it can recover by enumerating all serials under each pk it holds (the pool is plaintext, so this is possible), but this isn't stated as an explicit recovery path. Worth mentioning for completeness.

# P5.7 — Privacy statement
This section is refreshingly honest — possibly the most important quality for a privacy-adjacent design. The key claims are all correct:

Everything is public (denominations, pks, every operation, permanently — pruning is storage, not privacy)
Mint/redeem edges are the structural deanonymization vector (same as any coin)
Rotate/split/merge graph structure is visible and analyzable (timing, denomination, batching shape)
Anonymity set is an upper bound ("at best, every other live note of the same denomination"), narrowable by correlation
No active correlation resistance (no batching delays, no decoys, no fixed-interval scheduling)
Fee-stamp lineage is linkable (same class as graph visibility, not a new leak category)
The plain-language summary — "transparent, note-based bearer system with unlinkable ownership transfer between mint and redeem, and no protection at all for the mint/redeem edges themselves" — is accurate and not overstated. Comparing the privacy to cash rather than Zcash/Monero is the right calibration.

One observation: the spec mentions that a merchant's POS sweep is "a recognizable shape" (one SignedGroup covering N same-pk serials, moving to N fresh cold keys, at business-hours cadence). This is a real fingerprint, and the spec correctly discloses it. A potential mitigation worth mentioning (even if deferred to wallet-layer) would be for merchants to vary their sweep batch sizes or add timing jitter — but the spec's stance that "the protocol makes no attempt to hide stamp lineage" is the honest position.

# P5.8 — Finality anchors
## Parameters
__3-of-5 trustees:__ Reasonable. Tolerates 2 simultaneous failures, requires majority compromise for active misbehavior, small enough for "independent orgs/geos" to be a real, checkable property. The veto-only constraint (can't originate blocks, can't spend coins) is the crucial property that makes this "strictly less centralized than the threat it defends against" — a compromised quorum can delay finality but never mint or redirect value.

__T = 10⁶ × genesis difficulty:__ The refinement from "fraction of Kaspa's difficulty" to "multiplier of Marigold's own genesis difficulty" is an important correctness fix — the former would have violated the plan's own "deterministic function of on-chain data" requirement. The mechanism (fixed multiplier, fully self-contained, no oracle) is durable; the exact 10⁶ is correctly flagged as a calibration point. The one concern worth raising for the external reviewer: at a cold near-zero-hashrate launch, genesis difficulty is extremely low, and 10⁶ × extremely-low could still be modest in absolute hashrate terms. The security depends on what genesis difficulty actually is (Phase 2's job), so the external review should sanity-check whether 10⁶ × realistic genesis difficulty represents a threshold that's plausibly achievable by organic growth but expensive to sustain artificially for six months.

__M = 6 months sustained median, checking every difficulty-window checkpoint:__ The "every checkpoint, not just endpoints" design is crucial — it defeats an attacker who dips below T between sampled instants. This is the right way to define "sustained."

__K = 5 years:__ Conservative but defensible. The dual condition (T∧M and K) independently defeats the "mine honestly to inflate difficulty, then attack" strategy. A patient attacker sinks years of real resources with no guaranteed payoff.

__20-year hard maximum (6,311,520,000 DAA-score-units):__ Essential. "Trust must end even if network growth disappoints" is the right principle, and the unconditional expiry (regardless of whether the difficulty condition was ever met) is the correct enforcement. The fact that extending beyond this requires an explicit hard fork — meaning the default outcome is always expiry — is the right default bias.

## Fail-open liveness
Sound. Never halting for finality, falling back to plain PoW with loud alerting. The finality_anchor_stale: true RPC/metrics flag is the right operator-facing signal.

## Equivocation — needs a more precise definition
The spec says equivocation is "two validly-signed messages from the same trustee key that conflict — concretely, two FinalityAnchor-domain signatures from one key over two different anchored_block values whose depth/timing windows overlap (both could not honestly have been 'the ~1-minute-deep block' at the time each was signed)."

The concept is correct, but __the definition of "overlap" needs to be pinned down precisely for consensus implementation.__ Several questions arise:

__How is signing time inferred?__ The FinalityAnchor struct contains anchored_block and anchored_daa_score but no signer timestamp. The signing time is presumably inferred as "approximately when anchored_block was 600 DAA-score-units deep," i.e., at DAA score anchored_daa_score + 600. Two anchors overlap if their inferred signing times are close enough that the same block should have been 600-deep at both — but how close is "close enough"?

__Network partitions:__ If a trustee signs anchor X on chain A and later, after partition heals, signs anchor Y on the unified chain, X and Y may have different anchored_block values with DAA-score ranges that appear to overlap. This is honest behavior (the trustee acted on different views), not equivocation — but a naive overlap check might flag it. The external reviewer should examine whether the equivocation definition can distinguish "honest disagreement during partition" from "malicious double-signing."

__Adjacent cadence anchors:__ A trustee signing at 30-second intervals will anchor different blocks each time. Adjacent anchors' depth windows are close in DAA-score space but represent honest sequential signing. The overlap check must not flag normal cadence-following as equivocation.

A concrete proposal for the reviewer to evaluate: define overlap as "two anchors from the same trustee reference different anchored_block hashes but have anchored_daa_score values within one cadence interval (300 blocks at launch)." This captures "you signed two different blocks as 'the 600-deep block' within one anchor period" while allowing normal sequential anchoring. But this needs rigorous definition.

## IBD protection
"Any candidate chain not building at-or-beyond the latest valid anchored block is rejected outright regardless of accumulated work" — this is the critical property that prevents a higher-work attacker chain from overriding anchored history during sync. The reliance on hardcoded trustee keys (shipped in software, cross-checked against multiple independent bootstrap peers) for establishing the initial anchor is the right trust model for IBD — it mirrors how Kaspa already trusts hardcoded DNS seeders and genesis.

## Attack case coverage
The six named attack cases are all addressed honestly. The acknowledgment that "a genuinely compromised 3-of-5 majority can sign a false anchor" — without minimizing it — is exactly the honest framing the external reviewer needs to evaluate.

## Summary
The spec is strong. The overall design philosophy — extend existing Kaspa mechanisms wherever possible, invent new cryptographic constructions only where necessary, cite real codebase precedents rather than analogies — is consistently applied and reduces implementation risk substantially.

__Most important finding:__ The Redeem signature doesn't cover the transaction's transparent outputs, creating a transaction-malleability vector where an interceptor of a signed RedeemOp payload can redirect the redeemed funds. This should be addressed before P5.9 closes.

__Second priority:__ The equivocation definition needs precise overlap criteria, including handling for network partitions and adjacent-cadence anchors. The concept is sound; the exact consensus rule needs tightening.

__Everything else:__ The design holds up well. The privacy statement is honest to a degree that's unusual and valuable. The finality-anchor parameters are defensible. The wallet protocol is complete enough to implement from. The fee-stamp mechanism falling out of the conservation rule "for free" (no new wire format needed) is a genuinely elegant simplification that was discovered during spec-writing, not planned — and it works.