# Marigold Note Pool — Specification

Phase 5 of [FORK-PLAN.md](../../FORK-PLAN.md). A complete written spec of the note pool, frozen and externally reviewed (P5.9) before any implementation begins (Phase 6+). Each `## P5.N` section below is that plan step's deliverable — do not implement against this document until P5.9 has closed.

**Version: v1.1** (2026-08-15, tag `pool-spec-v1.1`). `pool-spec-v1` (tag, commit `9d93ab32`) was the frozen review baseline. This version folds in the accepted findings of two external reviews plus a cross-review concurrence and a confirmation pass — the full trail, with finding-by-finding triages, is under [reviews/](reviews/). P5.9's review gate closed on reviewer 1's confirmation verdict ("none of these should block tagging v1.1"); three non-blocking items are carried forward as standing obligations (the `[Open]`-flagged consumed-group-set malleability → Phase 6 review item; gas-semantics confirmation → Phase 6; the T/M/K quantitative model → P9.5 hard gate). Changes from v1:

1. **P5.2/P5.3 (critical fix)**: the pool-op signing hash now covers the enclosing transaction's transparent outputs, closing a Redeem transaction-malleability vector (an interceptor of a signed `RedeemOp` could previously redirect the redeemed value to their own transparent output; the signature didn't cover the outputs).
2. **P5.8 (tightened)**: anchor cadence is now defined in DAA-score units rather than wall-clock, which makes the equivocation rule exactly decidable (a precise overlap criterion replaces the previous informal "depth/timing windows overlap") and keeps trustee behavior well-defined when block production stalls; anchor staleness (fail-open trigger) is likewise DAA-score-defined.
3. **P5.1/P5.2/P5.6/P5.7/P5.8 (minor)**: denomination-tag extension explicitly stated to be a hard fork; freshness-window boundary semantics pinned (inclusive both ends); `known_serials`-loss recovery path stated explicitly; optional merchant sweep-jitter mitigation noted; T-multiplier absolute-value sanity check flagged for the external reviewer alongside the existing calibration note.

Second batch, from [review 2](reviews/pool-spec-v1.1-review-2-james-o-connell.md) (triage: [pool-spec-v1.1-review-2-TRIAGE.md](reviews/pool-spec-v1.1-review-2-TRIAGE.md)):

4. **P5.2**: signing preimage now binds a pool protocol version and the operation discriminant (`NotePoolSigningHash`, replacing `NotePoolTransferSigningHash`) — cheap domain separation against cross-operation reinterpretation under future extensions.
5. **P5.2 (new artifacts)**: an explicit authorization threat model (pre-execution bearerability stated as accepted, with reasoning); the authorization theorem with proof sketch, written for specialist verification; a complete signed/unsigned field matrix for every transaction and pool-op field, including the one analyzed deviation (consumed-group-set malleability); full canonicalization rules (sort order, duplicate rejection, encoding, limits); and anchor-acceptance semantics clarified (the anchor is a pure integer compared against the P5.3 validation context's DAA score — no block reference, hence no reorg/selected-chain ambiguity).
6. **P5.1**: "uniqueness" qualified as computational (collision-resistance-based), not mathematical; canonical encoding rules and an explicit state-invariant enumeration added (the invariants are the premises of P5.2's theorem).
7. **P5.8**: the 3-of-5 trust boundary stated prominently up front; the IBD/bootstrap trust root made explicit (software distribution as root, an anchor-ratchet persistence rule, multi-peer anchor query, suppression → staleness alerting, residual eclipse risk named); equivocation evidence lifecycle specified end to end; the T/M/K quantitative sensitivity model made a hard pre-launch gate (flagged into FORK-PLAN's P9.5).

**The design in one paragraph** (context for every section below): a **note** is `(d, pk, sn)` — denomination `d`, current owner's public key `pk`, and a stable serial number `sn` that never changes across a note's life. The **pool** is a plaintext, consensus-maintained map `sn → (d, pk)` that every full node holds, analogous to the UTXO set. Ownership is purely "whoever can sign with the private key matching the note's current `pk`" — no encryption, no zero-knowledge proofs anywhere in this design. Five operations mutate the pool: **mint**, **rotate**, **split**, **merge**, **redeem** (defined precisely in P5.2-P5.3).

---

## P5.1 — Data structures

### The Note

```
Note {
    d:  DenominationTag,   // 1 byte
    pk: [u8; 32],           // 32 bytes — x-only BIP340 Schnorr public key
    sn: Hash,                // 32 bytes — kaspa_hashes::Hash, stable for the note's lifetime
}
```

Total: **65 bytes** when a note is serialized as a self-contained unit (e.g. in a wallet's key database, P5.6). Inside the pool state map itself, `sn` is the map *key*, so only 33 bytes (`d` + `pk`) are the stored *value* per entry — see "Pool state map" below.

#### `d` — denomination tag (1 byte)

A `u8` enum indexing the fixed P1.6 denomination ladder, not the raw petal amount:

| Tag | MAGLD    | Petals (10⁸/MAGLD)     |
|-----|----------|-------------------------|
| 0   | 0.01     | 1,000,000               |
| 1   | 0.1      | 10,000,000              |
| 2   | 1        | 100,000,000             |
| 3   | 10       | 1,000,000,000           |
| 4   | 100      | 10,000,000,000          |
| 5   | 1,000    | 100,000,000,000         |
| 6   | 10,000   | 1,000,000,000,000       |
| 7   | 100,000  | 10,000,000,000,000      |

Tags 8-255 are reserved (unassigned) — a future denomination-set extension fits in the same 1-byte field without widening the Note struct. **Assigning any reserved tag is inherently a hard fork, never a backward-compatible extension** (stated explicitly so nobody later assumes otherwise): the tag→value table is referenced by validation (conservation arithmetic, P5.3) and by the pool commitment's meaning, so a node that doesn't know a new tag's value cannot validate ops using it — every node must upgrade, which is the definition of a hard fork. A tag is a **lookup index into a consensus-defined constant table**, not a value nodes compute — this makes the table itself the single source of truth for "what denominations exist," directly mirroring how `SUBSIDY_BY_MONTH_TABLE` (P3.2) is one canonical const array rather than a formula recomputed ad hoc.

#### `pk` — owner public key (32 bytes)

Reuses Kaspa's existing x-only BIP340-style Schnorr public key format **exactly** — the same 32-byte payload as address `Version::PubKey` ([crypto/addresses/src/lib.rs](../../crypto/addresses/src/lib.rs)) and the same key type (`secp256k1::XOnlyPublicKey`) used for signature verification in [crypto/txscript](../../crypto/txscript/src/lib.rs). This is a deliberate, not incidental, reuse: it means a note's `pk` is a real Kaspa/Marigold-format public key, signatures over pool ops reuse the exact same `secp256k1` Schnorr signing/verification call path already proven in `consensus/core/src/sign.rs`, and no new curve or signature scheme enters the codebase for the pool feature. Signatures over pool ops are the standard 64-byte BIP340 Schnorr signature (see P5.2 for the exact signed message).

Rotating a note replaces `pk` with a new one; `pk` alone can therefore never identify a note across its lifetime — which is exactly why `sn` exists.

#### `sn` — serial number (32 bytes, `kaspa_hashes::Hash`)

**Answering the plan's explicit question — is `sn` needed, and if so what is it?** Yes: since `pk` changes on every rotation, the pool map must be keyed by something stable, and no other field is a candidate. `sn` is that key. Its value is defined uniformly for every note regardless of which operation created it (mint, split, or merge all create new notes):

```
sn = H_serial(creating_tx_id || output_index_within_op)
```

where `creating_tx_id` is the 32-byte transaction ID of the pool-op transaction that created this note, `output_index_within_op` is a `u32` (little-endian) index of this note among the notes that specific op created, and `H_serial` is a new domain-separated hash function (see "Hashing" below). This directly mirrors how Kaspa already treats `(transaction_id, output_index)` — a `TransactionOutpoint` ([consensus/core/src/tx.rs](../../consensus/core/src/tx.rs)) — as a globally unique handle for UTXOs; `sn` is that same idea, collapsed into one fixed-size opaque hash instead of a raw pair, because the pool map (P5.1 below) needs a single `Hash`-typed key to match the existing `crypto/smt` sparse Merkle tree's key type. Two properties fall out for free:

- **Global uniqueness (computational, not mathematical — qualified per review 2) with zero extra consensus state.** No incrementing counter, no registry of "next available serial." The precise claim: serials are unique *under the collision resistance of the transaction-ID and serial-hash constructions* — `creating_tx_id` is itself a domain-separated hash over the whole transaction, and the index disambiguates multiple notes from one op, so producing two identical serials requires a hash collision, not merely protocol misuse. This is the same computational-uniqueness standard every hash-derived identifier in this codebase (transaction IDs, block hashes) already rests on — stated explicitly rather than claimed as absolute.
- **No rotation-of-`sn`.** `sn` is fixed at note creation and never appears as mutable state anywhere in this spec — every op that changes a note's `pk` (rotate; split and merge produce brand-new notes with brand-new `sn`s, they don't relabel old ones) leaves existing, untouched notes' `sn`s alone.

### The pool state map

```
PoolState : sn → (d, pk)
```

Implemented as a **sparse Merkle tree** using the existing [`crypto/smt`](../../crypto/smt/src/lib.rs) crate — the same 256-bit-depth, `Hash`-keyed, `Hash`-valued SMT already used in production for the seq-commit feature (KIP-21, `consensus/seq-commit/`, `consensus/smt-store/`). This is a direct architectural precedent, not a new pattern: seq-commit already proves this exact crate can back a plaintext, node-verified, consensus-committed key→value map inside this codebase.

- **Key**: `sn` (already a 32-byte `Hash` — no additional hashing needed to use it as an SMT key).
- **Leaf value**: `H_leaf(d || pk)` — a domain-separated hash of the 1-byte denomination tag concatenated with the 32-byte pubkey (33 bytes input). The SMT stores `Hash → Hash` (key → leaf hash), per `crypto/smt`'s design — the tree never stores `(d, pk)` directly, only its hash, so proving/verifying an entry means presenting `(d, pk)` alongside an SMT proof and recomputing `H_leaf(d || pk)` to check against the proven leaf hash.
- **Removal** (redeem, and the "old" side of split/merge/rotate replacing an entry): `crypto/smt` represents removal as inserting the all-zero hash at that key (`remove()`, `tree.rs:177`) — the pool follows the same convention. A removed serial's key becomes provably absent (a non-inclusion proof, see below), never reused for a different note (uniqueness from the `sn` derivation rule above already guarantees no future note ever collides with a removed one's key).
- **Production update path**: consensus code must use the pure function `compute_root_update::<H, S>(store, current_root, leaf_updates)` (`crypto/smt` `tree.rs:231`) against an `SmtStore`-backed persistent store — the same function `consensus/smt-store`'s `SmtProcessor::build` already uses for seq-commit — **not** the mutable in-memory `SparseMerkleTree::insert`/`remove` API, which is gated `#[cfg(any(test, feature = "test-utils"))]` and unavailable in production builds.

### The pool commitment

A new 32-byte `Hash` field in the block header, `pool_commitment`, holding the SMT root of `PoolState` as of that block. This is a **new header field**, not a reuse of an existing one (seq-commit reuses `accepted_id_merkle_root` post-toccata; overloading that same field for a second, unrelated commitment would make one field mean two different things depending on which of two independent forks activated — confusing and fragile). Adding a field to `Header` ([consensus/core/src/header.rs](../../consensus/core/src/header.rs)) is a consensus-breaking, hard-fork change: it needs a new block version (the same pattern `TOCCATA_BLOCK_VERSION` used in `consensus/core/src/constants.rs`) gated by a `ForkActivation` (Phase 6's job, not this spec's — noted here only so the byte layout is unambiguous: `pool_commitment` exists in headers from that activation's DAA score onward).

**Sync/verify flow** (mirrors `utxo_commitment`'s MuHash flow exactly, adapted to an SMT — detailed fully in P5.4; stated here because it's inseparable from what the commitment *is*): a node computes `pool_commitment` incrementally as it processes each block's pool ops via `compute_root_update`, and — for a node syncing from a pruning point — downloads the pool state in chunks (mirroring `PruningPointUtxosetChunkStream`, `protocol/flows/src/ibd/streams.rs`), folds each chunk into a running SMT build, and finally checks the resulting root equals the pruning-point header's `pool_commitment` before trusting the downloaded state — identical in shape to `Consensus::import_pruning_point_utxo_set`'s `imported_utxo_multiset_hash != new_pruning_point_header.utxo_commitment` check (`consensus/src/pipeline/virtual_processor/processor.rs`).

### Hashing (new domain-separated hash functions required)

Following the existing `crypto/hashes` convention (each hash purpose gets its own domain-separated tag — see `TransactionSigningHash`, `MuHashFinalizeHash`, `SeqCommitActiveNode` in [crypto/hashes/src/hashers.rs](../../crypto/hashes/src/hashers.rs)), the pool feature needs two new ones:

- **`NotePoolSerialHash`** — computes `sn = H(creating_tx_id || output_index_u32_le)`.
- **`NotePoolLeafHash`** — computes the SMT leaf value `H(d_u8 || pk_32bytes)`.

Plus one new concrete `SmtHasher` impl (a `NotePoolSmt` type, structurally mirroring `consensus/seq-commit/src/hashing.rs`'s existing hasher) supplying the `CollapsedHasher` and precomputed `EMPTY_HASHES` the SMT needs, built on `NotePoolLeafHash` as its leaf domain.

### Why not skip `sn` and key the map by `pk` instead?

Addressed explicitly since it's the obvious alternative: `pk` is not stable (rotation replaces it) and, per P5.6, **is not required to be unique** — multiple notes may deliberately share one `pk` (the POS landing-pad flow, P5.6). Keying the pool by `pk` would make "one pk, five notes" inexpressible (a map key can only point to one value) and would leak nothing extra in exchange, since `pk` is already public in the pool regardless of which field is the map key. `sn` is the only field satisfying "stable across the note's life" and "unique per note" simultaneously.

### Canonical encodings and state invariants (v1.1, per review 2)

**Encodings — one rule, stated once**: every multi-byte integer in this spec is **little-endian**, matching both borsh's integer encoding and the existing coinbase payload convention; every variable-length collection is length-prefixed exactly as borsh encodes it (`u32` LE count followed by elements); every hash is 32-byte Blake2b via a domain-separated `crypto/hashes` hasher (the codebase's single hashing convention — `NotePoolSerialHash`, `NotePoolLeafHash`, and P5.2's hashes are all instances of the same `blake2b_hasher!` macro family). Serial-hash preimage: `creating_tx_id` (32 raw bytes) `||` `output_index` (u32 LE) — no length prefixes inside fixed-width preimages. Leaf-hash preimage: `d` (1 byte) `||` `pk` (32 raw bytes). **Malformed encodings are consensus-invalid, not coerced**: a payload that fails borsh deserialization, carries trailing bytes after the deserialized value, or contains out-of-range enum discriminants is rejected outright (the transaction is invalid, per P5.3), never "interpreted as far as possible."

**State invariants** — enumerated explicitly because they are the premises P5.2's authorization theorem and P5.3's validation rules rest on:

- **I1 (key function)**: at any DAA score, a live serial maps to exactly one `(d, pk)` — `PoolState` is a map, and no operation can create a second entry under an existing serial (mint/split/merge derive fresh serials from a fresh `tx_id`; nothing else inserts).
- **I2 (unconditional serial retirement)**: every operation consuming a serial removes its entry — including a rotation whose produced note lands on the *same* `pk` (produced notes always carry fresh serials, so the consumed serial never survives an operation that touched it).
- **I3 (denomination immutability)**: a live note's `d` never changes — no operation rewrites an entry in place; value changes shape only by consuming notes and producing new ones under conservation (P5.3).
- **I4 (conservation)**: for every accepted operation, `Σ(consumed values) + Σ(transparent in) = Σ(produced values) + Σ(transparent out) + fee`, with fee ≥ 0 — the P1.8 unified rule, enforced per P5.3.
- **I5 (commitment faithfulness)**: after every block, `pool_commitment` is the SMT root of exactly the current `PoolState` — maintained incrementally (P5.3's diff application) and checked at sync (P5.4).

✅ *Verify (P5.1's own condition): every field has a byte size — restated compactly:* `d`: 1 byte · `pk`: 32 bytes · `sn`: 32 bytes · *pool commitment*: 32 bytes (header field) · *SMT leaf value*: 32 bytes (`H(d || pk)`, 33-byte preimage) · *serial preimage*: 36 bytes (`tx_id: 32` + `index: u32 LE = 4`).

---

## P5.2 — Transaction format

### Subnetwork and payload encoding

Pool ops ride in ordinary Kaspa/Marigold transactions tagged with a dedicated **user-lane** subnetwork ID — `SubnetworkId::from_namespace([0x50, 0x4f, 0x4f, 0x4c])` ("POOL" in ASCII, chosen only for memorability; any unclaimed namespace works equally well and Phase 6 may substitute one if this happens to collide with something reserved by then). This uses the existing, already-implemented namespace mechanism ([consensus/core/src/subnets.rs](../../consensus/core/src/subnets.rs)) that backs Toccata's "non-native/non-coinbase subnetworks (user lanes)" feature ([consensus/core/src/constants.rs](../../consensus/core/src/constants.rs)) — **not** the reserved single-byte `RegistrySubnetwork` path, which (checked directly: `grep` for its only non-definition usages) exists solely in test fixtures today, with no active registration/dispatch mechanism to build on. The payload is a single Rust enum, `PoolOp`, encoded with `borsh::to_vec` — the idiomatic in-repo pattern for "typed struct ⇄ opaque transaction-payload bytes" (e.g. `wallet/core/src/deterministic.rs`, `wallet/macros/src/wallet/server.rs`), used in preference to the coinbase payload's hand-rolled little-endian packing (`consensus/src/processes/coinbase.rs`), which is a fixed, non-extensible legacy format specific to that one use.

```rust
enum PoolOp {
    Mint(MintOp),
    Transfer(TransferOp),
    Redeem(RedeemOp),
}
```

Only **three** wire-format variants, not five — see "Unifying rotate/split/merge" below for why. Borsh enum tag = 1 byte.

### Shared building blocks

```rust
struct NewNote {
    d:  DenominationTag,  // 1 byte
    pk: [u8; 32],           // new owner's public key
}                              // 33 bytes

struct SignedGroup {
    serials:   Vec<Hash>,   // sn(s) authorized by `signature`; MUST currently share one pk (P5.3 checks this)
    signature: [u8; 64],     // BIP340 Schnorr signature by that shared current pk
}                                // 4 (borsh Vec length prefix) + 32×serials.len() + 64

struct FreshnessAnchor {
    anchor_daa_score: u64,  // a recent virtual DAA score the signer referenced
}                               // 8 bytes
```

`sn` for every note a `PoolOp` creates is *not* stored in the payload — it's derived per P5.1's rule, `H(this_tx_id || index_among_all_notes_this_op_creates)`, entirely from data already implicit in the enclosing transaction. This is a deliberate space saving (no reason to spend 32 bytes writing down a value every node independently computes the same way) and, more importantly, removes any possibility of a payload lying about a note's `sn`.

### Unifying rotate/split/merge into one `Transfer` op

The plan's own framing already treats split/merge as "a transfer with a different denomination multiset in vs. out" — taken literally, rotate (same multiset), split (one note → many smaller), and merge (many notes → one larger) are all just **different multiset shapes of the same underlying operation**: consume some notes, produce some notes, under one conservation check. Rather than three near-identical wire formats, `Transfer` is defined once and "rotate" / "split" / "merge" become purely descriptive labels for what a given `Transfer`'s multiset happened to do — a UX/documentation distinction, not a protocol one. This also means one `Transfer` transaction can freely mix these (e.g. split-and-rotate-part-of-it in one op) without inventing a fourth wire shape.

```rust
struct TransferOp {
    consumed:  Vec<SignedGroup>,    // notes being spent — total petal value = Σ over all groups' serials' denominations
    produced:  Vec<NewNote>,         // notes being created — total petal value = Σ
    freshness: FreshnessAnchor,       // covered by every group's signature (see below)
}
```

**Multiple `SignedGroup`s exist because one transaction may need to spend notes under different current `pk`s** — e.g. a customer's wallet holding three notes each with its own key. Each group's serials must currently share one `pk` (a single Schnorr signature can only verify against one key); a wallet combining differently-keyed notes in one `Transfer` supplies one group per distinct key. This is also exactly what P5.6's merchant "sweep" flow needs in the *other* direction: many serials sharing **one** `pk` (the POS landing pad), authorized by a **single** group with one signature covering all of them.

**Conservation and fee**: valid iff `Σ(consumed note values) ≥ Σ(produced note values)`; the difference is the transaction's fee — computed exactly the way Kaspa already computes ordinary transparent fees (`value-in − value-out`), just applied to notes' underlying petal values instead of UTXO amounts. This single rule is what makes fee stamps require *no new mechanism at all* — see "Fee-stamp mechanics" below.

### Mint and Redeem

```rust
struct MintOp {
    new_notes: Vec<NewNote>,
}
``` 
The transaction's ordinary transparent **inputs** (standard signed UTXO spends, verified by the existing txscript engine exactly as any transparent transaction) must sum to at least `Σ(new_notes' petal values)`; any excess is an ordinary transparent change output or the transaction fee, both completely standard Kaspa mechanics — mint needs no note-level signature at all, since nothing pre-existing in the pool is being touched. This is the "self-funding" value-touching op the P1.8 flag asks P5.2 to spec: mint pays its fee the same way any transparent Kaspa transaction always has, no fee stamp required, because it already holds transparent value to pay from.

```rust
struct RedeemOp {
    consumed:  Vec<SignedGroup>,
    freshness: FreshnessAnchor,
}
``` 
The transaction's ordinary transparent **outputs** hold what the redeemed notes become; valid iff `Σ(consumed note values) ≥ Σ(transparent outputs) + fee` — the transparent-side mirror of `Transfer`'s conservation rule, and, like mint, self-funding: redeem already produces transparent value, so it pays its fee from that, no stamp required. Redeem is structurally `Mint` read backwards (transparent-in → notes-out vs. notes-in → transparent-out), matching the plan's own five-op description exactly.

### Signature scheme and the freshness anchor

A pool-op signature is **not** a Kaspa/Marigold txscript input signature — notes have no transparent output/script to spend, so the existing per-input sighash machinery (`consensus/core/src/hashing/sighash.rs`) doesn't apply; it signs over transparent inputs, outputs, `gas`, and `subnetwork_id`, none of which describe "authorize this note's ownership to change." This needs its own domain-separated signing hash, following the exact macro convention already used for every other purpose-specific hash in this codebase ([crypto/hashes/src/hashers.rs](../../crypto/hashes/src/hashers.rs) — `TransactionSigningHash`, `MuHashFinalizeHash`, `SeqCommitActiveNode`, …):

```
NotePoolSigningHash = H(
    "NotePoolSig"                                  // domain tag
    || pool_protocol_version                         // u8 = 1 (v1.1, per review 2)
    || op_type                                        // u8 = the PoolOp borsh enum tag: 1=Transfer, 2=Redeem (v1.1)
    || sorted(group.serials)                           // this group's own serials, 32 bytes each
    || op.produced                                      // EVERY note this whole op creates, d||pk, 33 bytes each (empty for Redeem)
    || transparent_outputs_hash                          // 32 bytes — H over the enclosing tx's outputs (below)
    || freshness.anchor_daa_score                         // 8 bytes, LE
)

transparent_outputs_hash = H_outputs(
    for each tx.outputs[i], in order: amount (u64 LE) || script_public_key.version (u16 LE)
                                       || script_public_key.script
)   // over an empty output list (any pure Transfer), this is the domain's empty-input hash
```

The `pool_protocol_version` and `op_type` fields (v1.1, adopted from review 2's domain-separation recommendation) cost two bytes of preimage and buy structural protection against cross-operation reinterpretation: a signature produced for a `Transfer` can never verify in a `Redeem` context or vice versa, and any future protocol revision that changes signing semantics bumps the version byte rather than relying on every other field coincidentally differing. `op_type` reuses the `PoolOp` borsh enum tag values verbatim (one canonical numbering, defined once). Review 2 noted no demonstrated theft path under the previous shared tag — this is cheap insurance against *future* extensions, adopted as such.

**Canonicalization rules** (v1.1, per review 2 — "sorted" alone is not a consensus definition): `sorted(group.serials)` means ascending **lexicographic byte order** over the raw 32-byte serials (serials are opaque hashes; no other order is meaningful). A serial appearing more than once — within one group or across groups of the same op — makes the transaction **invalid** (checked in P5.3 step 1; duplicates are never deduplicated silently). `op.produced` and `tx.outputs` are committed **in their serialized order** — order is signer-chosen, part of the signed message, and not renormalized by validators. Collection bounds: an op's total consumed serials and total produced notes are each capped at **1,000** — the same bound as the existing `max_tx_inputs`/`max_tx_outputs` transaction limits ([consensus/core/src/config/params.rs](../../consensus/core/src/config/params.rs)), chosen to reuse an already-reasoned-about scale rather than invent one.

`H_outputs` is a further domain-separated hash (`NotePoolOutputsHash`, same `crypto/hashes` macro convention as the rest) whose per-output serialization deliberately mirrors the field order the existing transaction sighash already commits outputs with (`consensus/core/src/hashing/sighash.rs`'s `outputs_hash` covers amount, script version, and script — same data, pool-domain-separated here rather than reusing the transparent sighash function directly, which carries `SigHashType` semantics this scheme doesn't want).

**Why the signed message covers the enclosing transaction's transparent outputs** (v1.1 — this closes a real transaction-malleability vector found in external review 1): a `RedeemOp`'s value lands in `tx.outputs`, which live *outside* the pool-op payload — under v1's payload-only signed message, an interceptor of a signed-but-unbroadcast `RedeemOp` could rebuild the transaction with their own address in `tx.outputs` (same consumed serials, same freshness anchor, same still-valid signature) and redirect the redeemed value. The signature authenticated that the notes were consumed, but not where the resulting transparent value went — exactly the asymmetry review 1 identified (`Transfer`'s destination `pk`s were signed; `Redeem`'s destination scripts were not). Covering `tx.outputs` in every pool-op signature closes this uniformly: for `Redeem` it binds the transparent destinations; for a pure `Transfer` (no transparent outputs by design) it binds the output list to *being empty*, which costs nothing and future-proofs any later op shape that touches the transparent side. Note this is **not** circular the way signing the `tx_id` would be: `tx.outputs` does not contain the pool-op payload — only the payload contains the signature — so the signer can compute this hash before signing without any self-reference.

Every `SignedGroup` in a `Transfer`/`Redeem` signs over the **entire** op's `produced` list and the transaction's transparent outputs (not just "its share"), not merely its own serials — this is what makes a `Transfer`/`Redeem` atomic: no signer is vouching for their serials being spent in just *some* context, they're vouching for this *exact* whole-transaction shape — note destinations *and* transparent destinations — so no group's signature can be lifted into a transaction with a different produced-notes list or different transparent outputs. No `tx_id` is signed over (and deliberately can't be — the enclosing transaction's ID is a hash that includes this very payload, so signing it would be circular); replay safety instead comes from two properties working together:

1. **A successfully-executed group's signature can never be reused.** The instant a group executes, every serial it covered now has a different current `pk` (or no longer exists, for redeem), so re-submitting the identical signed message fails P5.3's "signature verifies against the serial's *current* `pk`" check — permanently, not just once.
2. **A signed-but-never-executed op has a bounded shelf life.** Without anything binding the signature to one specific transaction, a valid signed group could otherwise be broadcast at any arbitrary future time by whoever holds it (the "stale invoice" risk P5.5 names explicitly for sign-to-fresh-pk mode). `freshness.anchor_daa_score` bounds this: P5.3 must reject the op once `current_daa_score − anchor_daa_score` exceeds a fixed **freshness window**.

**Recommended freshness window: 36,000 DAA-score units** (≈1 hour at 10 BPS). Reasoning, stated explicitly since the plan calls this a deliberate choice, not a default to leave implicit: long enough that no ordinary in-person or remote payment flow is at risk of the signature expiring mid-transaction (P5.5's bearer and sign-to-fresh-pk flows both settle in seconds at 10 BPS; an hour is generous headroom, not a tight budget), short enough that a leaked or abandoned signed op — an unpaid invoice, a bearer QR photographed but not yet handed over — stops being a live liability within the same session it was created, not days later. Same category of "needs real-world calibration, not a first-principles derivation" as P1.8's stamp-sizing note; recorded here as a concrete recommended default, adjustable at Phase 6/P6.6 calibration, not a placeholder. **Its classification, stated plainly (v1.1, per review 2): 36,000 is a liveness/UX parameter, not a derived security constant.** No property of the design breaks at 35,000 or 40,000; what the number tunes is the trade-off between payment-flow headroom and the shelf life of a leaked authorization. Phase 6/P6.6 calibration should model at minimum the 5-minute, 15-minute, 1-hour, 6-hour, and 24-hour regimes (review 2's list) against congestion, partition recovery, merchant retry flows, and signature-theft exposure before freezing it.

### Anchor acceptance semantics (v1.1, pinned completely per review 2)

The freshness anchor is a **pure integer** — `anchor_daa_score: u64` — not a reference to any block. This resolves, by construction, every ambiguity review 2 asked to have pinned: there is no "anchor block" that must lie on the selected-parent chain, no anchor-side reorg case, and no anchor discovery question — the only DAA score the validator compares against is the **validation context's own POV DAA score** (P5.3's `pov_daa_score`, the same score every other DAA-dependent check in that context already uses). The complete rule, restated from P5.3 step 3: valid iff `0 ≤ pov_daa_score − anchor_daa_score ≤ 36,000`, inclusive both ends. Consequences, spelled out: a transaction delayed in the mempool remains valid until its anchor ages past the window in whatever context finally validates it, then becomes permanently invalid (expiry is monotone in DAA score — it cannot "un-expire"); under a reorg, the same transaction is simply re-evaluated against the new context's POV score by the identical arithmetic — since honest anchors are generated at signing time from the signer's current view, a reorg deep enough to change the verdict is a reorg deeper than the freshness window (≈1 hour), far beyond both the P5.8 anchored-finality depth and the existing finality-depth reorg refusal; and a "future" anchor (`anchor_daa_score > pov_daa_score`) is invalid everywhere, uniformly. Every node computes the identical verdict from `(payload, pov_daa_score)` alone.

### Authorization threat model (v1.1, stated explicitly per review 2)

**Pre-execution bearerability is accepted, deliberately.** Review 2 correctly characterizes a signed-but-unbroadcast pool op: until it executes or expires, anyone holding the payload — recipient, relay, thief, malware — can broadcast it. The design accepts this as an inherent property of offline signed payment authorization, and the reason it is *acceptable* is what the signature binds: the payload authorizes **exactly one state transition** (fixed consumed serials, fixed produced notes with fixed destination keys, fixed transparent outputs, fixed window). Early broadcast by any party executes precisely the transfer the signer already intended — the intended recipients receive the intended value; nothing can be redirected, split differently, or re-denominated (the field matrix below is the systematic argument). The residual adversarial power is **timing within the window** (worst case: the payment lands earlier than the signer would have chosen) plus the bounded group-set malleability analyzed in the matrix. What this implies for wallets (binding on P5.5/P5.6 UX): a signed op must be treated as **already spent from the moment it is signed and leaves the wallet**, not from broadcast — "sign now, hold, maybe don't send" is not a supported pattern, and the sign-to-fresh-pk invoice flow must present the signed payload as payment-in-flight, never as a revocable draft. A signer who wants an unbroadcast authorization dead before expiry has exactly one tool: rotate the consumed serials to fresh keys first (self-spend), which invalidates the outstanding signature via invariant I2.

### The authorization theorem (v1.1 — the written claim review 2 asked for, for specialist verification)

**Theorem.** Let `G` be a `SignedGroup` validly signed by the holder of key `k` for an op with produced list `P`, transparent outputs hash `O`, and anchor `A`, under protocol version `v` and op type `t`. An adversary holding `(G, P, O, A)` (and any number of other parties' signed groups) but not `k` can cause, via any transaction accepted in any block order, **at most** the following state transitions involving `G`'s serials: either (a) no transition, or (b) exactly one execution of the transition `(G.serials consumed) → (P created, outputs matching O)`, in some context whose POV DAA score lies in `[A, A+36,000]`. In particular the adversary can never redirect, re-denominate, or partially execute `G`'s serials, and can never execute the same authorization twice.

**Premises** (each tied to its enforcement point):

- **(1) Current-key verification** — P5.3 steps 1-2: the signature verifies only against the serials' current `pk` in the composed pool view (invariant I1).
- **(2) Unconditional retirement on execution** — invariant I2: any execution removes `G`'s serials from the pool; produced notes carry fresh serials even if `P` reuses the same `pk`, so a second submission of the identical payload fails premise (1)'s existence check in every subsequent context, including every reorg path (the composed-view mechanism re-derives state per context; in any single context the serials exist at most once).
- **(3) Complete binding of economic effect** — the signed message covers `v`, `t`, `G`'s serials, all of `P`, and `O`; the only accepted-transaction degrees of freedom outside the signed message are enumerated in the field matrix below and none alters destination, denomination, or amount of `G`'s serials' disposition.
- **(4) Unambiguous encoding** — P5.1/P5.2 canonicalization: one byte encoding per message, duplicates invalid, malformed payloads rejected outright.

**Proof sketch.** By (4), the signed message determines a unique `(v, t, serials, P, O, A)` tuple; any transaction deviating in any of these fails signature verification (step 2). By (1), acceptance additionally requires the serials live with `pk = k`'s public key in the acceptance context, and by P5.3 step 3, `pov ∈ [A, A+36,000]` — giving exactly transition (b) when accepted. By (2), acceptance in any context destroys premise (1) for every later context evaluating the same payload, and GHOSTDAG's composed-view ordering (P5.3) guarantees every context evaluates each serial's state exactly once in a defined order — so at most one acceptance globally. Absent acceptance, no pool-state change involving `G`'s serials occurs at all (transition (a)). ∎ *(Sketch — a human cryptographer should challenge each premise against the Phase 6 validation code before implementation lock; this is the artifact to review, not its own confirmation.)*

### Signed/unsigned field matrix (v1.1 — review 2's "most important P5.2 artifact")

Every field of the enclosing transaction and pool op, with its binding status. **Committed** = covered by `NotePoolSigningHash`; **harmless** = uncommitted, shown unable to alter the authorization's economic/security meaning; **analyzed** = uncommitted with a real but bounded effect, documented.

| Field | Status | Argument |
|---|---|---|
| `pool_protocol_version`, `op_type` | Committed | In the preimage (v1.1). |
| `group.serials` (own group) | Committed | Sorted set in the preimage; duplicates invalid. |
| `op.produced` (entire list) | Committed | Full list, in order — destinations and denominations fixed. |
| `tx.outputs` (amounts, scripts) | Committed | Via `transparent_outputs_hash` (v1.1). |
| `freshness.anchor_daa_score` | Committed | In the preimage. |
| **Other groups' serials** (consumed set composition) | **Analyzed** | A group's signature does not cover *other* groups. Consequences, exhaustively: an adversary may **add** a group they themselves validly sign for the same `(v,t,P,O,A)` (raises fee at their own expense — a donation); or **strip** another party's group (lowers `Σconsumed`, hence fee — if conservation still holds the op executes with the stripped group's serials left untouched and still owned by their holder; if not, the tx is invalid). Neither redirects nor re-denominates anything; the stripped party loses nothing but their intended fee contribution. Worst case is fee-stripping to zero (mempool relay policy then declines it) — a griefing vector, not theft. Full-consumed-set binding was considered and deliberately not chosen: it would forbid collaborative fee attachment (a second party adding a stamp to an op they didn't author) at the cost of closing only this non-theft vector. **[Open — carried into Phase 6 as a standing review item; reviewer 1's confirmation pass assessed it as "needs specialist sign-off, not a redesign" and non-blocking for v1.1.]** |
| `tx.subnetwork_id` | Harmless | Changing it stops the payload being interpreted as a pool op at all — no pool transition occurs (transition (a)); the mutated tx is then a zero-input non-coinbase transaction, invalid under existing rules. |
| `tx.version` | Harmless | User-lane subnetworks require `TX_VERSION_TOCCATA` (≥1); other values are invalid with a user-lane subnetwork ID under existing rules. |
| `tx.lock_time` | Harmless | Can only delay earliest acceptance; the anchor window bounds total delay — worst case the op expires (transition (a)). |
| `tx.gas` | Harmless* | Lane budgeting only; does not enter pool validation or alter the transition. *[Open — confirm gas semantics for user-lane txs against Phase 6 code, per review 2's audit instruction.]* |
| `tx.inputs` (transparent) | Harmless (Transfer/Redeem) | Both op types forbid transparent inputs (P5.2); a tx carrying any is invalid. For `Mint` there are no note signatures at all — see below. |
| `storage_mass` commitment | Harmless | Independently consensus-checked; cannot alter the pool transition. |
| `Mint` (whole op) | n/a — differently secured | `Mint` has no note signature; its integrity rides entirely on the **existing** transparent-input sighash, which — verified against [consensus/core/src/hashing/sighash.rs](../../consensus/core/src/hashing/sighash.rs) — already commits to the transaction's `payload` (containing `new_notes`) and outputs. A `Mint`'s funder therefore already signs the exact note set being minted, via the existing mechanism, with no new construction needed. |

### Fee-stamp mechanics (P1.8 flag — every bootstrap case, worked through the wire format)

No separate "stamp" field or op type exists in this format — a stamp is just an ordinary consumed serial in a `Transfer`'s `consumed` list with no matching value in `produced`, which the conservation rule (above) already turns into fee automatically. This single mechanism covers every case P1.8 named:

- **Pure rotate** (`Transfer` with `consumed` and `produced` denominations identical) has zero natural conservation slack — `Σconsumed = Σproduced` exactly, so it pays *nothing* unless an extra serial is added to `consumed` with no corresponding `produced` entry: that's the "pre-existing stamp" P1.8 says pure rotate requires. Concretely: rotating one 0.1-note to a new key, with a 0.01-note attached purely as a stamp, is `consumed: [group for the 0.1 note, group for the 0.01 stamp], produced: [one new 0.1-denomination note]` — the 0.01 simply has no matching output, and its value becomes the fee.
- **Self-funding split/merge** (P1.8's worked example: `100 → 9×10 + 9×1 + 9×0.1 + 9×0.01 (= 99.99) + 0.01 fee`) needs no separate stamp at all — `consumed` lists the one 100-note, `produced` lists the 36 smaller notes summing to 99.99, and the 0.01 gap is the fee automatically, computed exactly like the pure-rotate case but arising from the split's own arithmetic rather than an attached extra serial.
- **Handovers include a stamp** (P1.8's option 2): the bearer bundle (P5.6) carries a second note's private key alongside the primary note's; the receiver's eventual rotate op lists both serials in `consumed`, only the primary note's denomination in `produced`. No format difference from the pure-rotate case above — "the stamp came bundled with the note" is a wallet/UX fact, not a wire-format one.
- **Mint produces stamps** (P1.8's option 3): trivially expressible — a `MintOp` whose `new_notes` includes small denominations alongside larger ones; nothing pool-specific to add here since mint already supports minting any combination of denominations in one op.

Congestion pricing (P5.7 will note this as a privacy limitation) falls out for free too: attaching a bigger or additional stamp increases `Σconsumed − Σproduced`, which increases the transaction's fee, which raises its priority in the existing mempool fee-per-mass ordering — no protocol-level fee schedule needed, exactly as P1.8 already concluded.

### A consensus-rule dependency this format creates (flagged for P5.3)

Checked directly against current validation code, not assumed: a pure `Transfer` (or `Redeem`) has **zero transparent inputs** by design ("touches no transparent value"), but `check_transaction_inputs_count` ([consensus/src/processes/transaction_validator/tx_validation_in_isolation.rs:78-80](../../consensus/src/processes/transaction_validator/tx_validation_in_isolation.rs)) currently rejects *any* non-coinbase transaction with `tx.inputs.is_empty()` — `TxRuleError::NoTxInputs` — with no existing exception for other subnetworks. (No equivalent "zero outputs" rule exists, confirmed by its absence — only inputs are currently required to be non-empty for non-coinbase transactions, so `Transfer`'s already- empty `tx.outputs` needs no change.) **P5.3 must define an explicit consensus-rule exception** — the natural shape mirrors the existing `!tx.is_coinbase()` guard, generalizing it to also exempt the pool subnetwork ID from the zero-inputs check — this isn't a new category of problem, just the coinbase precedent extended to a second subnetwork that also legitimately has no transparent inputs.

### Worked byte-size estimates

All figures are **payload-only** (the pool-specific addition); every op also carries standard Kaspa transaction overhead (version, input/output counts, `subnetwork_id` (20 bytes), `gas`, `lock_time`) — small and already well-understood/bounded by existing Kaspa serialization, not re-derived here. `TransactionOutpoint` (32-byte tx ID + 4-byte index = 36 bytes, [consensus/core/src/tx.rs](../../consensus/core/src/tx.rs)) and a P2PK-spend signature script (66 bytes, `wallet/core/src/tx/mass.rs`'s `SIGNATURE_SIZE = 1 + 64 + 1`) are the only transparent-side costs `Mint`/`Redeem` add beyond the payload.

| Op | Shape | Payload bytes |
|---|---|---|
| `Mint` | 1 new note | `1 + 4 + 33×1` = **38** |
| `Mint` | 5 new notes (a mixed-denomination bundle) | `1 + 4 + 33×5` = **170** |
| `Transfer` | plain rotate (1 group/1 serial, 1 produced note) | `1 + [4+(4+32+64)] + [4+33] + 8` = **150** |
| `Transfer` | rotate + attached 1-serial stamp (2 serials in 1 group, 1 produced note) | `1 + [4+(4+64+64)] + [4+33] + 8` = **182** |
| `Transfer` | self-funding split, 1→36 notes (P1.8's worked example) | `1 + [4+(4+32+64)] + [4+33×36] + 8` = **1,305** |
| `Transfer` | merchant sweep, 20 serials/1 group → 20 fresh-key notes | `1 + [4+(4+32×20+64)] + [4+33×20] + 8` = **1,385** |
| `Redeem` | 3 serials/1 group, no new notes | `1 + [4+(4+32×3+64)] + 8` = **177** |

Every realistic shape lands from tens of bytes to ~1.4 KB — comfortably inside "a few KB," and negligible against block mass limits: at `mass_per_tx_byte = 1` ([consensus/core/src/config/params.rs](../../consensus/core/src/config/params.rs)), a 1.4 KB payload costs ~1,400 mass units against a per-block budget of 500,000 (pre-toccata, `prior_block_mass_limits`) to 1,000,000 (`new_transient_mass_limit`) — under 0.3% of a single block's budget even for the largest worked example, with no coinbase-style dedicated payload-length cap (`max_coinbase_payload_len = 204`) applying here at all, since that constant is coinbase-specific.

✅ *Verify (P5.2's own condition): all five ops covered (Mint, and Transfer's three descriptive shapes rotate/split/merge, and Redeem) with worked byte sizes; total transaction sizes land at tens of bytes to ~1.4 KB, far inside the "few KB" target and a small fraction of block mass limits. Fee-stamp mechanics specified for every named bootstrap case (pure rotate, self-funding split/merge, bundled handover stamp, mint-produced stamps) via one unified mechanism, no case left unaddressed.*

---

## P5.3 — Consensus rules

### The pool diff and composed view (mirrors the existing UTXO mechanism exactly)

Kaspa/Marigold already resolves UTXO double-spends between blocks merged in one GHOSTDAG mergeset via a specific, existing mechanism — read directly from [consensus/src/pipeline/virtual_processor/utxo_validation.rs](../../consensus/src/pipeline/virtual_processor/utxo_validation.rs) (`calculate_utxo_state`, lines 109-165) rather than assumed: blocks in the mergeset are visited in GHOSTDAG blue-topological order (`ghostdag_data.consensus_ordered_mergeset_without_selected_parent`, [consensus/src/model/stores/ghostdag.rs:183](../../consensus/src/model/stores/ghostdag.rs)), starting from the selected parent. Each block's transactions are validated against a **composed view** — the selected parent's UTXO state overlaid with a `mergeset_diff` accumulated from every *already-processed* block earlier in that same ordering (`selected_parent_utxo_view.compose(&ctx.mergeset_diff)`, line 131). A transaction whose inputs conflict with what's already in `mergeset_diff` simply fails validation and is excluded from that block's `accepted_transactions` (`MergesetBlockAcceptanceData`/`AcceptedTxEntry`) — the block itself is not rejected, only that one transaction. **This is "first accepted wins," concretely**: not a special rule invoked on conflict, but the ordinary consequence of validating every block against whatever state the blocks before it (in blue order) already committed.

The pool feature needs the exact same shape, one level added: a `PoolDiff` (the pool's analog of `UtxoDiff`) accumulated alongside `mergeset_diff` during the same mergeset walk, and a composed pool view (selected parent's pool state + accumulated `PoolDiff`) that every pool-op transaction validates against, in the same blue-topological order, in the same pass — a pool op and an ordinary UTXO spend can appear in the same transaction (mint, redeem) and must be validated together, atomically, against both composed views at once. **No new conflict-resolution rule is being invented here** — the parallel-blocks case (two rotations of the same serial in two blocks of one mergeset) is answered entirely by this existing mechanism applied to pool state: whichever block's pool op is processed first (blue order) updates the composed pool view; the second block's conflicting op fails step 1 below (the serial's current `pk` in the composed view no longer matches what its signature was checked against) and is excluded from that block's accepted transactions — same "loser becomes a no-op, not an invalid block" outcome real UTXO conflicts already have today.

### Validation order — `Mint`

1. Every `new_notes[i].d` is a valid denomination tag (0-7; P5.1's table).
2. Standard txscript validation of the transaction's transparent inputs against the composed *UTXO* view — completely unchanged, the existing mechanism.
3. Conservation: `Σ(transparent inputs) − Σ(transparent outputs) − Σ(new_notes petal values) ≥ 0`; the result is the transaction's fee (subject to the same minimum-relay-fee mempool policy as any transaction — not a new consensus rule).
4. Apply to `PoolDiff`: insert `sn_i → H(d_i || pk_i)` for each new note, where `sn_i = H_serial(this_tx_id || i)` (P5.1). Uniqueness is guaranteed by construction (this transaction's ID cannot already have been used to derive an existing serial) — not an active check, but an invariant implementers should assert in testing.

### Validation order — `Transfer` (rotate/split/merge)

1. For every `SignedGroup` in `consumed`: every serial in `group.serials` exists in the composed pool view, **and** all of them currently share the exact same `pk` — if any two differ, the op is invalid (one signature cannot authenticate two different keys). No serial may appear more than once across the op's entire consumed set (within or across groups) — a duplicate makes the op invalid (v1.1 canonicalization rule).
2. Recompute `NotePoolSigningHash` (P5.2) over the protocol version, the op's type discriminant, `group.serials`, the op's full `produced` list, the enclosing transaction's `transparent_outputs_hash`, and `freshness.anchor_daa_score`; verify `group.signature` against the shared current `pk` from step 1.
3. Freshness: valid iff `0 ≤ pov_daa_score − freshness.anchor_daa_score ≤ 36,000`, **inclusive on both ends** (boundary semantics pinned explicitly per review 1: a difference of exactly 0 and exactly 36,000 are both valid; 36,001 is not; `anchor_daa_score > pov_daa_score` is not) — rejecting both a stale anchor (too far in the past) and a future one (which could otherwise let a signer pre-date a signature to extend its effective shelf life).
4. Every `produced[i].d` is a valid denomination tag.
5. Conservation: `Σ(consumed notes' current petal values, from the composed view) − Σ(produced notes' petal values) ≥ 0`; the result is the fee (same relay-fee policy note as `Mint`).
6. Apply to `PoolDiff`: remove every consumed serial's entry; insert `sn_i → H(d_i || pk_i)` for each produced note, `sn_i` derived the same way as `Mint`.

### Validation order — `Redeem`

1-3. Identical to `Transfer`'s steps 1-3, applied to `Redeem`'s own `consumed` list. 4. Conservation: `Σ(consumed notes' current petal values) − Σ(transparent outputs) ≥ 0`; the result is the fee — the transparent-side mirror of `Mint`'s rule. 5. Apply: remove every consumed serial's `PoolDiff` entry; the transparent outputs are applied to the ordinary UTXO diff exactly as any transaction's outputs already are — no change to that existing mechanism.

### Mass and fee costing

The plan's own guidance: "a rotate is one sig verify + one map update — cost it like a normal 1-input tx; no special proof costs exist in this design." Concretely, using the existing cost model (`consensus/core/src/mass/mod.rs`, [consensus/core/src/config/params.rs](../../consensus/core/src/config/params.rs)):

- **Payload bytes** already cost `mass_per_tx_byte` (= 1) each, automatically, since the payload counts toward `transaction_estimated_serialized_size` — no new per-byte rate needed (P5.2's worked byte sizes are therefore already mass estimates, 1:1).
- **Each `SignedGroup`'s signature verification** costs one sigop-equivalent — `mass_per_sig_op` (=1000) for v0-style costing, or one `ComputeBudget` unit's worth (100 grams = 10,000 script-units, `consensus/core/src/mass/units.rs`) under the v1 compute-budget model — charged **once per signature, not once per serial**: a 20-serial sweep under one shared `pk` (P5.6) is one signature and therefore one sigop-equivalent, not twenty, which is what makes batch sweeps cheap by design, not an incidental side effect.
- **`Mint`/`Redeem`'s transparent side** costs exactly what it already would as an ordinary transaction — standard txscript sigops for spent inputs (`mass_per_sig_op`/compute-budget as today), `mass_per_script_pub_key_byte` (=10) for any transparent output scripts. Nothing pool-specific changes on that side.
- No zero-knowledge proof verification exists anywhere in this design (stated in P5.7 as a privacy limitation, restated here as a performance fact) — every pool-op cost is either a byte count or a small fixed number of Schnorr signature verifications, the same order of magnitude as costs the mempool already charges for today.

### Explicit dependency: P5.2's theorem stands on this section (v1.1, per review 2)

The authorization theorem's premises (1) and (2) — current-key verification and unconditional serial retirement — are not free-standing cryptographic properties; they are *exactly* steps 1-2 and 6 of the validation orders above, executed inside the composed-view walk. Any Phase 6 change to this section's ordering or diff-application semantics (including any reorg/rebuild path that re-derives composed state) must be re-checked against the theorem, not just against these validation rules in isolation — the two sections are one argument split across two headings.

✅ *Verify (P5.3's own condition): every question in the checklist answered explicitly — validation order stated per op (existence, signature, freshness, denomination validity, conservation, in that order); the parallel-blocks double-rotate case is resolved by citing and extending the exact existing mechanism (composed view over GHOSTDAG-ordered mergeset), not a new rule; signature replay protection restated from P5.2's freshness anchor and tied to the concrete consensus check (step 3 above); mass/fee costing defined per op using existing cost-model constants, with the batch-sweep cost savings made explicit.*

---

## P5.4 — Pool state sync & pruning interaction

### The pool map survives pruning, exactly like the UTXO set

Pruning discards old block bodies/headers beyond the pruning depth; it does **not** discard current state — that's precisely what makes pruning safe: the UTXO set is *current* state, preserved forward indefinitely, and `utxo_commitment` in each header is what lets a node trust a downloaded UTXO set without replaying the discarded history to rebuild it. `PoolState` (P5.1) is current state in the exact same sense — every entry alive today is alive regardless of which now-pruned block created or last touched it — so it survives pruning identically, verified against `pool_commitment` (P5.1) instead of `utxo_commitment`.

### Sync flow for a fresh node — real precedent, cited directly

Two existing sync flows in this codebase are directly relevant, and they're not interchangeable — the pool's `crypto/smt` structure makes the **second** one the closer match, not just an analogy:

1. **UTXO set** (MuHash-based): `protocol/flows/src/ibd/flow.rs`'s `sync_pruning_point_utxoset` (~line 851) requests the set (`RequestPruningPointUtxoSetMessage`), streams it in chunks (`PruningPointUtxosetChunkStream`), and folds each chunk into a running `MuHash` via `Consensus::append_imported_pruning_point_utxos` ([consensus/src/consensus/mod.rs:1115-1128](../../consensus/src/consensus/mod.rs)). Only **after every chunk is received** does `import_pruning_point_utxo_set` compare the final accumulated hash against `utxo_commitment` — MuHash supports no partial/incremental verification, so a corrupt or malicious chunk is only caught at the very end.
2. **Seq-commit SMT** (KIP-21, already in this codebase — not upstream rusty-kaspa): `import_pruning_point_smt` ([consensus/src/consensus/mod.rs:1134](../../consensus/src/consensus/mod.rs)) calls `kaspa_smt_store::streaming_import::streaming_import` ([consensus/smt-store/src/streaming_import/mod.rs:79](../../consensus/smt-store/src/streaming_import/mod.rs)), which — checked directly, not assumed — does something strictly better: **each streamed entry is checked with an SMT inclusion proof against the target root as it arrives** (`proof.verify::<SeqCommitActiveNode>(&lane_key, Some(leaf_hash), lanes_root)`, line 123), *in addition to* a final `result.root != lanes_root` backstop check after the whole import completes (`consensus/src/consensus/mod.rs:1166`).

Since `PoolState` is an SMT (P5.1), it gets flow 2's stronger property for free: a node syncing the pool state can reject a bad or malicious chunk **as it's received**, not only after downloading the entire map. This is a real, structural advantage over the UTXO set's own sync flow, worth stating explicitly since it's not something the pool feature had to design — it inherits it by being built on the same SMT infrastructure seq-commit already proved out.

### The pool's own sync flow (extending flow 2, not re-deriving it)

1. A fresh node learns the current pruning point's header, including its `pool_commitment` (P5.1), the same way it already learns `utxo_commitment` today — part of ordinary header sync, no new mechanism.
2. The node requests the pool state at that pruning point — a new P2P message symmetrical to `RequestPruningPointUtxoSetMessage` (exact wire name is Phase 6's job; spec-level requirement is only that it exists and identifies the same pruning point).
3. The remote peer streams `(sn, d, pk)` triples in chunks, sorted by `sn` (canonical order — matches how the existing `streaming_import` expects pre-sorted batches for its `StreamingSmtBuilder`), each accompanied by what `streaming_import` already needs to verify incrementally: enough of the tree structure to check the entry's leaf (`H_leaf(d || pk)`, P5.1) against `pool_commitment` via an SMT inclusion proof, using the new `NotePoolSmt` hasher (P5.1) in place of seq-commit's `SeqCommitActiveNode`.
4. The receiving node feeds each chunk into `streaming_import` (generalized to the `NotePoolSmt` hasher), rejecting the transfer immediately on any proof-verification failure (flow 2's incremental property) rather than only discovering a mismatch after the full download.
5. After the last chunk, the final computed root is compared against the pruning point header's `pool_commitment` — the same backstop `import_pruning_point_smt` already performs for seq-commit, reused verbatim for the pool's own `SmtStores` instance.
6. Only after both checks pass does the node adopt the downloaded pool state as trusted current state, exactly mirroring `import_pruning_point_utxo_set`'s "verify-then-adopt" ordering — a downloaded-but-unverified pool state is never partially trusted.

### What is downloaded and which committed hash checks it (P5.4's own verify condition, answered directly)

**Downloaded**: the full `PoolState` map as of the pruning point — every live `(sn, d, pk)` triple, streamed in `sn`-sorted chunks, each individually proof-checkable against the target root as described above (not just the final root).

**Checked against**: `pool_commitment` — the new 32-byte `Header` field specified in P5.1 — both incrementally (per-chunk SMT inclusion proofs during streaming) and as a final backstop (recomputed root vs. the pruning-point header's committed value), exactly mirroring how `utxo_commitment` gates trust in a downloaded UTXO set today, with the incremental check as a genuine strengthening the SMT-based design provides over the UTXO set's MuHash-only final check.

✅ *Verify:* the fresh-node sync-from-pruning-point flow is described end to end (learn commitment → request → stream sorted chunks with incremental proof verification → final-root backstop → adopt), citing the two real existing precedents this design extends rather than inventing a new one, and stating explicitly which committed header field (`pool_commitment`) and which verification mechanism (SMT inclusion proofs, incremental *and* final) gate trust in the downloaded state.

---

## P5.5 — Transfer modes

**Decided in the plan (P1.4-era, restated and formalized here against the concrete wire format P5.2-P5.3 now define): both modes are first-class, supported equally.** The on-chain mechanism for both is the identical `TransferOp` (P5.2) — a `SignedGroup` authorizing consumption of some serials, producing new notes. Neither mode is a distinct protocol feature; they're two different ways a **wallet** arrives at "who signs the `TransferOp`, and with which key," described below. Neither mode involves identities: only notes have keys, and "a fresh pk" is nothing more than a newly generated note-keypair no different in kind from any other.

### The universal settlement rule

**A note is finally yours when a rotation to a key only you know is confirmed on-chain.** Stated once here because it governs both modes identically: until that confirmation, the previous holder may still know a key that can authorize spending the note (bearer mode: literally the same key you were just handed; sign-to-fresh-pk mode: the sender could attempt a conflicting `TransferOp` to a different destination before broadcasting the one they showed you). Either way this is the ordinary double-spend case P5.3 already resolves — whichever conflicting `TransferOp` is accepted first (GHOSTDAG blue order) wins, the other is excluded — so "wait for confirmation" is what actually settles a transfer in both modes, and at 10 BPS that wait is seconds, not the minutes-to-hours a slower chain would impose on the same guarantee.

### (a) Bearer key-handover

The sender reveals the note's **existing** private key directly to the receiver — printed on paper, a QR code, any offline channel. The receiver can be completely passive at the moment of handover (the cash-like property this mode exists for: "granny pays with a QR secret key printed on paper," no wallet interaction required to *receive*). The note is not finally theirs yet per the settlement rule above — the sender still knows the same key — so a receiver who needs certainty before releasing goods (point of sale) must actively rotate the note to a key of their own and wait for confirmation before treating it as settled; a receiver who's fine with implicit trust (a personal handover between people who know each other) may simply hold the handed-over key as-is, accepting the shared-key risk until they eventually rotate.

**On the wire**: the receiver constructs a `TransferOp` with one `SignedGroup` (the handed-over note's serial, signed with the handed-over private key) and one `NewNote` in `produced` (their own fresh `pk`) — an ordinary rotate, indistinguishable on-chain from any other. Nothing about bearer mode is visible in the transaction format; it's entirely a fact about *how the signing key reached the signer's wallet*, invisible to consensus.

### (b) Sign-to-fresh-pk

The receiver generates a fresh `pk` and hands it to the sender (a merchant's QR code; a friend's messaged public key) — the **private** key never leaves the receiver's wallet, never existing in two places at once. The sender constructs and signs a `TransferOp` rotating their own note(s) to that `pk`. A handed-out `pk` that's never used is inert — if the sender never broadcasts, the receiver has lost nothing, and the wallet just watches that `pk` for activity ("unpaid-invoice" semantics: showing a payment QR is like writing an invoice, not like handing over cash).

**On the wire**: identical `TransferOp` shape to bearer mode — one or more `SignedGroup`s (the sender's own notes, signed by the sender), `produced` containing the receiver's `pk`. The *only* difference from bearer mode is who generated the destination `pk` and who holds its private key before broadcast — again invisible to consensus, a wallet-level fact only.

### The freshness anchor doubles as invoice expiry

P5.2's `FreshnessAnchor` (36,000-DAA-score / ≈1-hour window) was designed as anti-replay protection for the *signer*, but it has a second, equally important role for sign-to-fresh-pk mode specifically: it bounds how long a shown `pk` remains a *valid target* for the payment it was meant for. A merchant's checkout QR, once its `pk` is generated, only makes sense as "pay this exact amount, now" for as long as the customer's wallet could still construct a `TransferOp` whose freshness anchor will validate — past that window, any signed-but-unbroadcast `TransferOp` targeting that `pk` is rejected by P5.3's freshness check regardless of whether the `pk` itself is technically still "unused." This is precisely why P5.2 called the window's choice deliberate: it is simultaneously anti-replay protection and invoice/QR expiry, and a wallet implementer should treat "how long do I show this QR before regenerating it" and "how long is a signed-but-unsent payment still valid" as the *same* number, not two separately-tuned ones — they're the same protocol parameter.

### The shared-key window (bearer mode) vs. the pk-freshness window (sign-to-fresh-pk)

Both modes have a window of exposure, but to different things, worth stating side by side since P5.9's external review will need to weigh them independently:

- **Bearer mode's shared-key window** runs from the moment a private key is handed over until the receiver rotates it — during which *both* parties can authorize spending the note (not a bug, the defining property of a bearer instrument, same as physical cash). Its natural end is receiver-controlled (rotate whenever they choose); nothing in consensus bounds it, which is correct — a paper bearer note in a drawer for a year is still exactly as valid as one spent immediately, exactly like physical cash.
- **Sign-to-fresh-pk's freshness window** runs from anchor generation until either the `TransferOp` confirms or the anchor expires (≈1 hour, above) — bounded by *consensus*, not by either party's choice, because an unbounded "pay this invoice whenever" QR would mean a merchant's displayed amount could be honored at a wildly different exchange-rate moment than when it was shown, among other staleness problems P5.6's POS flow design (below, in that section) already assumes a short window to avoid.

✅ *Verify:* both modes' decision is recorded (already was, in DECISIONS.md's P1.x-era notes — restated here against the concrete wire format); the settlement/finality rule is stated once, unambiguously, and shown to reduce to P5.3's existing double-spend resolution rather than needing a new one; each mode's on-chain shape is given explicitly (both reduce to the identical `TransferOp`); the shared-key window (bearer) and the pk-freshness window (sign-to-fresh-pk, tied concretely to P5.2's 36,000-DAA-score constant) are both named and distinguished.

---

## P5.6 — Wallet protocol

**The wallet is a key-database manager, not an identity.** No 24-word seed tied to one master key; it holds one private key per note (or, under shared-pk policy below, one key shared by several notes). This section specs the key DB and its backup, the receive and spend flows, QR payload formats, how a wallet tracks its own notes without any scanning/trial-decryption (the pool is plaintext — nothing to decrypt), the shared-pk policy and its one invariant, the same-key-in-two-wallets hazard and its mitigations, and a forward-looking key-algorithm-deprecation story.

### Key database format and the backup story

Minimally, one row per key the wallet holds:

```
KeyDbEntry {
    sk: [u8; 32],              // private key (the corresponding pk is derivable, not stored redundantly)
    provenance: KeyProvenance,  // Cold | Hot — see "same-key-in-two-wallets hazard" below
    known_serials: Vec<Hash>,   // sn(s) this wallet believes are currently under this key
}
```

**State this loudly, as the plan requires**: losing the key database is losing the notes. There is no seed phrase to reconstruct it from, no derivation path, nothing but the raw private keys themselves — this is the direct consequence of "one key per note" instead of one master key deriving everything, and it is the entire reason the note vault (below) is not an optional feature but core to the wallet being usable at all.

### Note vault (primary backup) and paper export

**Decided ahead of P7.6's execution, full rationale in DECISIONS.md's "Note vault, backup, and restore-rotation policy" entry** — summarized here as the spec text this implementation follows.

**Storage format**: one file per `KeyDbEntry`, not one blob. A `notes/` directory with a status subdirectory per `NoteStatus` (`active/`, `handed-over/`, `superseded/` — a status change is an atomic rename); each file is named for its already-public metadata (denomination, serial) and its contents (`sk`, plus enough to reconstruct the row) encrypted under one per-wallet **vault key K**, XChaCha20Poly1305 with a per-file nonce — `wallet/core/src/encryption.rs`'s existing `encrypt_xchacha20poly1305(data, secret)`, the same primitive the wallet already uses elsewhere, not a new one. Balance and coin selection read filenames only; a spend decrypts exactly the notes it selects. This directly replaces the single-encrypted-map approach (P7.1's initial implementation, which decrypts-and-reencrypts the *entire* map on every single-note touch — every operation transiently held every key in memory, not just backup/restore) with a design whose in-memory exposure is bounded to "the notes currently being spent."

**24-word vault key ceremony**: K — the *file* encryption key, explicitly not a BIP32/BIP39-style seed deriving note keys — is shown once at vault creation as 24 words (the familiar wallet-onboarding ceremony, reused for its UX shape only). For daily use K is additionally wallet-password-wrapped, like every other secret this wallet already protects that way. This does not contradict this section's opening line ("no 24-word seed tied to one master key") — note keys stay independently generated, one per note, undiscoverable from K alone. Recovery needs **both** the words and the vault files: an encrypted copy is safe on fully untrusted storage without the words; the words alone recover nothing.

**Manifest**: an optional plaintext companion — `(serial, value, last-rotated-at)` per note — for human/tooling legibility, riding alongside the encrypted copy.

**Paper export**: the paper QR remains available as one printable representation of the same vault entries (not a separate mechanism): serialize a chunk of `(serial, sk, denomination)` entries with `borsh`, encrypt with the same `encrypt_xchacha20poly1305`. ~40 note entries fit per QR code (a `(serial: 32, sk: 32, d: 1)` entry is 65 bytes; borsh-encoded + XChaCha20Poly1305 overhead — 24-byte nonce + 16-byte tag — keeps a 40-entry chunk comfortably under a QR code's practical capacity at a scannable error-correction level). Each QR carries a small plaintext header before the encrypted payload so multi-page restores can detect missing pages without needing the password first:

```
QrPageHeader {
    backup_id:     [u8; 8],   // random, generated once per backup session — groups pages together
    chunk_index:   u16,        // this page's index, 0-based
    chunk_count:   u16,         // total pages in this backup
    format_version: u8,          // 1
}                                    // 15 bytes, plaintext
```

The paper export's password may be written on the printed page itself — unlike the vault (recoverable only with K, kept separate), the paper form's threat model is safe physical storage (a drawer, a safe), not a password kept secret from whoever finds the page; encryption still protects the more likely real-world exposure (a stray phone photo, a printer's spool file, a cloud-synced "Downloads" folder).

### Restore flow — self-reconciling against the plaintext pool

Because the pool is plaintext, a restored key doesn't need the wallet to have tracked anything continuously — it can ask the chain directly, and it can do so at two independent strengths:

- **Light verify** (needs no secrets at all): a serial's `(denomination, pk)` binding is immutable for its life — rotation consumes a serial and mints a new one, never re-pointing an existing one — so "serial still exists in `PoolState`" is exactly equivalent to "note still unspent," and serials already sit in plaintext (filenames, manifest). Checking every manifest serial against live pool state needs zero decryption, zero secrets in memory, and no rotation — lets a backup's health be confirmed without ever restoring.
- **Deep verify** (the mandatory first step of an actual restore): decrypt each entry and re-derive its `pk`, comparing against the current on-chain owner exactly as light verify does, but additionally catching a corrupted ciphertext light verify cannot. For each entry: if the derived `pk` matches the serial's current on-chain owner, the note is still yours; if not, someone's transaction already moved it on (spent since backup, or the backup is stale/superseded) — discard that entry. No merkle-scanning, no trial-decryption, no synchronization protocol — one `PoolState` lookup per restored serial, the same passive read this spec already assumes the wallet can do freely.

**Recovery when `known_serials` is lost but keys survive** (stated explicitly per review 1, rather than left implied): the vault/paper-export formats above store `(serial, sk, d)` per note, so a normal restore never faces this — but a wallet that somehow retains private keys without their serial list (a key-only export, a partially corrupted DB) is still fully recoverable, because the pool is plaintext: enumerate every serial currently under each held key's `pk` and adopt that as the new explicit `known_serials` list. This is the **one sanctioned use of pk-enumeration to establish ownership** — it happens interactively at restore time and its *output* is a rebuilt explicit serial list; it is not an exception to the "ownership is tracked by serial, never inferred by pk" spending rule below, which governs ongoing spend/sweep selection, not one-time recovery. Keys restored this way are Hot by provenance (they crossed a wallet boundary), so the restore-rotation policy immediately below applies to them at its default strength.

**Restore-time rotation: default on, explicitly overridable** — a deliberate exception to this section's general Hot-key rule ("rotate immediately, not lazily"), decided ahead of execution once the vault decoupled "backup leaked" from "password leaked" (full reasoning in DECISIONS.md; bearer *receive*, below, keeps rotating unconditionally — a different threat model, since a bearer handover's shared-key window is deliberately choice-driven by the receiver in the moment, not a recovery-time bulk operation). Flow, after deep verify reports which notes are still live: offer a **batched, randomly-spaced, randomly-composed rotation** (2-5 transactions, mixed denominations per batch — not sorted by value, which would leak structure the mixing exists to hide). The user may accept (default), defer, or decline; deferred notes remain fully spendable (Hot is an urgency flag, not a lock) with the wallet nagging until resolved; the moment rotation completes, prompt for a fresh backup copy, since the whole point was invalidating the old one. State both consequences plainly in the dialog: rotating invalidates every old backup copy including any stolen one; deferring keeps old backups valid including any stolen one. Batching reduces the "entire wealth rotated at one timestamp" fingerprint but does not eliminate linkage (each batch's own consumed-serials list is still an explicit on-chain link) — a genuine improvement over one all-at-once sweep, not a privacy guarantee. Implementation note: this is the existing full self-sweep (below), not new machinery — restore-time rotation is `sweep` with a confirmation dialog in front, which also gives the wallet a standalone "I think my backup leaked" panic button for free.

Two properties worth stating explicitly, since they're easy to get backwards:

- **Rotation doubles as backup revocation.** A leaked backup copy (vault or paper) only endangers notes that haven't been rotated since it was made — the moment any note on it is rotated (by the legitimate owner, for any reason), that specific entry in *every* copy of that backup, leaked or not, becomes worthless (its key no longer matches the note's current `pk`). A full self-sweep (rotate everything) is therefore a deliberate, complete invalidation of every prior backup at once — a real recovery action, not just hygiene, and exactly the mechanism restore-time rotation (above) reuses.
- **Backups go stale.** Notes *received* after a backup was made are, by definition, not on it. The wallet should prompt for a fresh backup copy periodically (e.g. after N new notes received, or on a time interval) — for the vault this is cheap (copy the new files; no re-encryption of anything already backed up) — this is a UX nudge, not a protocol requirement, since nothing about the chain enforces backup freshness.

### Receive flow

1. **Import** — either import a handed-over private key directly (bearer mode, P5.5a), or receive a signed `TransferOp` targeting a `pk` this wallet generated and already holds the private key for (sign-to-fresh-pk mode, P5.5b — the wallet was "watching" that `pk` since generating it, per "unpaid-invoice semantics").
2. **Verify on-chain state** — look up the relevant serial(s) in `PoolState` (a plain lookup, same mechanism as backup restore above) and confirm the expected `TransferOp` has actually confirmed (not merely broadcast — mempool presence alone isn't settlement, per P5.5's universal settlement rule).
3. **Rotate if bearer mode** — per P5.5a, a bearer-received note isn't finally the receiver's until they rotate it to a key only they know; the wallet should do this automatically (not leave it as a manual step) the moment step 2 confirms the note is real and spendable, since delaying only extends the shared-key exposure window for no benefit.
4. **Confirm** — update `known_serials` for the (possibly newly rotated-to) key, mark the entry ready to spend.

### Spend flow

Given a target amount:

1. **Select notes** from `known_serials` whose denominations can combine to at least the target (standard bin-packing over the fixed P1.6 ladder — implementation detail, not specified further here since any correct selection algorithm is protocol-compatible; only the *result* — a valid `TransferOp`/`Redeem` — is consensus-relevant).
2. **Split as needed** to make exact change — a `Transfer` whose `produced` list includes both the payment-sized note(s) (to the recipient's `pk`) and change note(s) (back to a *fresh* key the wallet controls, not the same key being spent from — reusing a key across a split's own inputs and outputs is never necessary and needlessly narrows the note's key-history). This can be the exact same transaction as the transfer itself (P5.2's `Transfer` already allows an arbitrary `produced` list — "split then pay" is one `TransferOp`, not two sequential ones), or a separate prior split if the wallet prefers to hold pre-split change ready in advance.
3. **Transfer** — construct and sign the `TransferOp`/`Redeem`, attaching a fee stamp (P5.2's mechanism — an extra consumed serial with no matching produced entry) only if the op doesn't already self-fund (a pure same-denomination payment does not, per P5.2's "pure rotate has zero natural slack" analysis — the wallet should default to attaching a stamp for `Transfer`s that don't naturally produce a fee, and skip it for ones that do, such as any split).

### QR payload formats

Two distinct QR uses appear in this spec, deliberately different formats since they carry different trust properties:

- **Backup QR** (above): `QrPageHeader` (15 bytes plaintext) `||` XChaCha20Poly1305 ciphertext of a borsh-encoded `Vec<KeyDbEntry-like tuple>` chunk. Multi-page; requires the backup password to read.
- **Payment-request QR** (P5.5b, POS below): plaintext, no encryption — it's a public invitation to pay, not a secret. `PaymentRequest { pk: [u8; 32], amount_petals: u64 }` — 40 bytes; a static day-pk fallback (for printed QRs, P5.5b) omits `amount_petals` (the customer enters it manually) and is simply `{ pk: [u8; 32] }`, 32 bytes.

### How the wallet tracks its notes — no scanning, no trial-decryption

The wallet already knows every serial it holds (they're rows in its own `KeyDbEntry` table) — it does not need to discover them by scanning the chain, because nothing about receiving a note requires the receiver to have been anonymous to the chain first (unlike a shielded-pool design, where a receiver must trial-decrypt every note to find their own). It simply watches `PoolState` for changes touching its own known serials (the pool being plaintext state every full node already holds makes this a plain, cheap map lookup, exactly like a light client watching specific UTXOs today) — confirmed activity on a known serial is how "was this rotation accepted" (receive flow step 2) and "did my spend confirm" are both answered, with the identical mechanism.

### Shared-pk policy (decided)

**Consensus does not require `pk` uniqueness.** Many notes may share one `pk`; each stays an independent `sn → (d, pk)` entry (P5.1), and each remains separately spendable via a signed rotation — `TransferOp`'s `SignedGroup` already allows one signature to cover multiple serials sharing a `pk` (P5.2), so a merchant sweeping many same-`pk` notes needs no re-rotation-per-note first, just one group listing them all.

**The one wallet invariant**: **bearer handover requires a solo key.** Revealing a shared `sk` hands over *every* note under that `pk`, not just the one being paid — so before a note can be bearer-spent (P5.5a), it must first be isolated onto its own fresh key (one ordinary rotation, same-denomination, no fee-stamp-avoiding trick needed since it's a pure rotate that can attach a stamp normally). Personal wallets should default to generating a fresh `pk` per note received via sign-to-fresh-pk mode specifically to avoid ever needing this isolation step later — sharing a `pk` is something a wallet does *deliberately* (the POS landing pad below), not a default state personal notes drift into.

### POS "landing pad" flow (decided)

1. The register encodes `{pk, amount}` (the `PaymentRequest` QR above) — `pk` fresh **per checkout** when dynamically generated (gives free payment matching: the merchant knows exactly which confirmed rotation corresponds to which sale), falling back to one **static day-`pk`** only for printed/static QR codes, with the customer entering the amount manually in that case.
2. The customer wallet displays the amount for confirmation, runs the spend flow above (select, split as needed) targeting the register's `pk`.
3. The merchant wallet watches that `pk` (per "how the wallet tracks its notes," above) and, the instant the payment confirms, **immediately sweeps**: one `TransferOp` with a single `SignedGroup` (all the just-landed notes share the checkout `pk`, so one signature covers all of them) moving every note to its own freshly-generated cold key in `produced`.
4. The checkout `pk` is therefore only ever a **transient landing pad** — steady state is always one-note-one-key, and the shared-key window lasts only as long as it takes the merchant's own sweep transaction to confirm (seconds, at 10 BPS).

**Sweep per confirmation, not end-of-day**: notes left parked on the POS `pk` between sales are exposed to a compromised register device for as long as they sit there — a device that's leaked or logged its signing key (or is simply malicious) can spend anything still parked on it. Sweeping immediately, transaction by transaction, bounds that exposure to the confirmation latency of one transfer, not a business day.

### Same-key-in-two-wallets hazard

Shared-`pk` notes can end up split across wallets that don't know of each other: a partial key export between a user's own devices, a restored old backup that predates a device split, or a bearer handover of a key that (in violation of the invariant above) wasn't actually solo. Since the pool is plaintext, **anyone holding a `pk` can enumerate every serial under it** — so either wallet, in this scenario, technically *could* spend (or accidentally sweep) notes the other wallet also believes it owns. Two rules make this state harmless and self-limiting rather than a live conflict:

1. **Ownership is tracked by serial, never inferred by `pk`.** A wallet's spend/sweep selection (spend flow step 1, POS sweep above) operates *only* on its own explicit `known_serials` list — never derived by scanning what happens to exist under a `pk` it holds. Enumerating a `pk`'s full serial set is useful for audit/debugging, never for deciding what to spend. This alone prevents one wallet from ever accidentally sweeping serials the other wallet added to the same `pk` without this wallet's knowledge.
2. **Key provenance decides laziness** (the `KeyProvenance` field in `KeyDbEntry`, above). A key generated locally and never exported anywhere is **Cold** — lazy isolation is fine, no urgency to rotate away from a shared state that only this wallet could have caused. Any key that ever crossed a wallet boundary — a bearer import, a cross-device export, a backup restore — is **Hot**, and every one of *this* wallet's serials under that key should be rotated to fresh Cold keys immediately on next opportunity, not lazily, since a Hot key's shared-state history could include another wallet the user doesn't fully control or trust in this moment.

### Key-algorithm-deprecation story (forward-looking; no existing "architecture paragraph" to transcribe — checked, none exists elsewhere in this repo)

P5.1 fixes exactly one key format at launch (32-byte x-only BIP340 Schnorr, reusing Kaspa's own). This section specs the *mechanism* a future migration would use, not a second format to support today — inventing a multi-algorithm wire format before it's needed would be exactly the kind of premature complexity this project avoids elsewhere.

**Signaling**: a future deprecation is a consensus-level event, activated the same way every other protocol change in this codebase already is — a `ForkActivation`-gated rule (the identical mechanism `crescendo_activation`/`toccata_activation` already use) that, from some future DAA score, refuses `Mint`/`Transfer`/`Split`/`Merge` outputs using the deprecated key format in `produced`, while continuing to allow existing deprecated-format notes to be rotated *away* from it — mirroring exactly how a `Version` field already lets Kaspa addresses support multiple coexisting key formats (`Version::PubKey` vs. `Version::PubKeyECDSA`, `crypto/addresses`) without breaking anything already using the old one. New note creation moves to the new format; old notes remain fully spendable throughout, their only path forward being an ordinary rotation to a new-format key.

**Wallet-side**: a wallet learns of an upcoming deprecation the same way it learns of any other future consensus rule — shipped in a software update carrying the new `ForkActivation`'s DAA score, or (for earlier, update-independent warning) by querying a node's RPC for upcoming deprecation schedules, analogous to `get_server_info` today. Once aware, the wallet should proactively self-sweep: rotate every note held under the deprecated algorithm to freshly-generated new-algorithm keys well before the enforcement DAA score, using the *exact same* `TransferOp` mechanism already specified for the same-key-hazard self-sweep above — no new wallet operation, just the existing rotate applied preemptively and network-wide rather than reactively to one user's own hazard.

✅ *Verify:* a wallet developer can implement receive → detect → spend without further questions — every flow (backup/restore, receive, spend, POS sweep, cross-wallet hazard mitigation, future key migration) is specified in terms of primitives already fully defined in P5.1-P5.5 (`TransferOp`, `SignedGroup`, `PoolState` lookups, the existing `encrypt_xchacha20poly1305`), with no step left as "figure this out later."

---

## P5.7 — Honest privacy statement

This section is the project's public claim about what the note pool does and does not hide. It must not claim more than the design actually delivers — every item below follows directly from mechanisms already specified in P5.1-P5.6, not from aspiration.

### What is public

- **Every note's denomination and current `pk`.** `PoolState` (P5.1) is plaintext, consensus-maintained state every full node holds in full — there is no encrypted or hidden portion of it. Anyone can enumerate every note currently under a given `pk`.
- **Every operation, ever performed, in full detail.** Every `Mint`, `Transfer` (whichever of rotate/split/merge it happens to be), and `Redeem` is an ordinary, fully visible on-chain transaction (P5.2) — which serials were consumed, which were produced, their denominations, and exact timing (the block's timestamp/DAA score) are all public, for the same reason every Kaspa/Marigold transaction already is.
- **This is not softened by pruning.** Pruning discards old block *bodies* from typical nodes' storage — it is a storage optimization, not a privacy mechanism. An adversary who observes the network in real time, or who runs one archival node, or who simply downloads the chain before old blocks age out, retains the complete operation graph back to genesis regardless of what an ordinary pruned node keeps. No claim in this spec should be read as "old activity becomes unlinkable once pruned" — it does not.

### What is not private — and where the real exposure is

**No sender/receiver "address" exists in the pool the way a transparent UTXO output has one** — a note's only on-chain identity is its `sn` and current `pk`, and `pk` changing on every rotation is precisely what prevents a note from accumulating a named, persistent "account" the way a reused address would. This is real, and it's the core principle that gives the coin the cash like property the design provides.

**But `Mint` and `Redeem` are where a specific note's identity meets a real transparent coin history**, and this is where real-world deanonymization actually happens — exactly the same structural weak point every other non private coin has and these edges are fully public by construction: a `Mint`'s transparent inputs came from *somewhere* — an exchange withdrawal, a previous transparent-tier transaction, anywhere with an existing real-world or on-chain history — and that history is now directly, permanently linked to the specific `sn`(s) that mint created. Symmetrically, a `Redeem`'s transparent outputs go *somewhere* traceable onward. Everything a note does *between* a mint and an eventual redeem is unlinkable to that originating/terminating transparent history only in the narrow sense that `pk` doesn't persist — the mint and redeem edges themselves are not hidden at all.

**Rotate/split/merge graph structure is visible, and it is real structure an adversary can analyze**, not just individually-anonymous events: exact timing, which denominations moved, how many serials a `Transfer` consumed vs. produced, and any batching pattern (a merchant's POS sweep, P5.6, is a recognizable shape — one `SignedGroup` covering N same- `pk` serials, moving to N fresh cold keys, at a predictable business-hours cadence) are all plainly visible graph structure, not encrypted or aggregated away. Timing correlation alone (a mint, followed shortly by a rotate of a newly-created serial of the same denomination) can narrow a note's anonymity set well below "every note of that denomination," even though no cryptographic link between them exists. A wallet *may* blur its own fingerprints at the margin — e.g. a merchant varying sweep batch sizes or adding timing jitter (per review 1's suggestion) — but any such mitigation is wallet-layer behavior, best-effort, and never a protocol guarantee; the protocol makes no attempt to hide this structure, and no claim in this section is softened by the possibility of wallets partially obscuring their own patterns.

### The anonymity set

**A note's anonymity set is, at best, roughly "every other currently-live note of the same denomination."** Nothing in the design distinguishes one 1-MAGLD note from another 1-MAGLD note beyond their (different) `sn` and current `pk` — an observer who has *not* otherwise correlated a specific note via mint/redeem linkage or timing analysis (above) genuinely cannot tell them apart. This bound is real, but it is an upper bound, not a guarantee: any timing or amount correlation an adversary can perform narrows it, exactly as described above, and the design does nothing to actively defeat such correlation (no batching delays, no decoy transactions, no fixed-interval operation scheduling) — naming this absence explicitly rather than leaving it implied.

### Fee-stamp lineage (P1.8 flag, named explicitly per its own instruction)

A fee stamp (P5.2) is an ordinary note with its own history like any other — attaching it to an op does not create a new category of leak, but it does mean **ops sharing one stamp's ancestry become linkable to each other** through that shared lineage, the same way any two operations touching a common serial's history already are. Concretely: a wallet that repeatedly draws stamps from the same original mint batch links every one of those otherwise-unrelated payments together in the note graph. This is the same class of visibility as the rotate/split/merge graph structure above, not a new mechanism — P5.6's wallet hygiene guidance (don't pay for unrelated operations from one linkable stamp source) is the mitigation, at the wallet-implementation layer, not the protocol layer; the protocol makes no attempt to hide stamp lineage.

### What this design is, stated plainly

This is a **transparent, note-based bearer system with unlinkable ownership transfer between mint and redeem, and no protection at all for the mint/redeem edges themselves**. It is not a shielded pool, provides no zero-knowledge unlinkability, and does not resist a well-resourced adversary correlating timing, denomination, and mint/redeem history across the whole (permanently public, pruning notwithstanding) chain. Its privacy is closer to cash than to Zcash's or Monero's — real, useful against casual observation and address-reuse-style tracking, and honestly bounded by exactly the same edges every coin has always been bounded by.

✅ *Verify:* the section exists and states, with no claim stronger than the design delivers: what's fully public (denominations, `pk`s, every operation, unaffected by pruning); the real deanonymization vector (mint/redeem linking notes to transparent history, the same structural weakness as any coin); that rotate/split/merge graph structure (timing, denomination, batching shape) is visible and analyzable; the anonymity-set bound and its explicit lack of any active correlation-resistance; and the fee-stamp lineage leak named explicitly, per the P1.8 flag's own instruction, as the same class of leak as the disclosed graph visibility rather than a new category.

---

## P5.8 — Launch finality anchors ⚠️

**Not a note-pool feature** — a chain-level security mechanism bundled into Phase 5 because, like the pool, it must be fully spec'd before any implementation begins. **Naming**: this section's *finality anchors* are unrelated to P5.2's *freshness anchor* (pool-op replay protection) — same word, different mechanism, never conflate them in any document.

**The trust boundary, stated first and plainly** (v1.1 — promoted to the top of this section per review 2): **any 3 of the 5 trustee keys, compromised or colluding, can sign a false anchor endorsing an attacker's chain — and during IBD, trustee keys shipped in the software release are part of the bootstrap trust root, alongside the genesis block and DNS seeders.** This is not an implementation defect to be engineered away; it is the irreducible security assumption of k-of-n finality, present in every mechanism of this class ever deployed. Everything else in this section — trustee independence requirements, equivocation disqualification, fail-open, the sunset, the 20-year hard expiry — exists to bound, surface, and terminate that assumption, never to eliminate it. Anyone evaluating this design should start from this paragraph.

**Decided in the plan, restated**: the fork launches with a federated finality guard. A young PoW network sized nothing like Kaspa mainnet is trivially 51%-attackable by any sliver of Kaspa's own ASIC fleet redirected for an hour; a veto-only, sunsetting trustee quorum that can never originate a block or spend a coin is strictly less centralized than the alternative of one anonymous mining farm quietly holding 99.9% of Marigold's actual hashrate. Precedent for this class of mechanism: early Bitcoin's checkpoints, Peercoin/Feathercoin checkpointing, Komodo's dPoW, Decred's hybrid PoW/PoS finality.

### Trustee quorum and anchor mechanics

**3-of-5**, independent organizations/geographies (the plan's own recommendation, adopted — a 5-party quorum tolerates 2 simultaneous unavailable/uncooperative trustees while still requiring collusion or compromise of a *majority* for any misbehavior, and stays small enough that "independent orgs/geos" is a real, checkable property, not just a number). Trustees **produce no blocks, hold no mining reward, and can only veto** — they ratify already-mined history, never select transactions or originate new blocks. This is the core of why the mechanism is "strictly less centralized" than the threat it defends against: a compromised trustee quorum can deny/delay finality, never mint value or redirect it.

**Anchors**: a 3-of-5 signature over the hash of a block already **600 DAA-score-units deep** (~1 minute at genesis-era 10 BPS — deep enough that the anchored block has already cleared the chain's own natural early-confirmation noise, not the literal tip), produced once per **cadence interval of 300 DAA-score-units** at launch (~30 seconds at nominal 10 BPS). **The cadence is defined in DAA-score units, not wall-clock time** (v1.1 — tightened per review 1): an honest trustee signs at most one anchor per cadence interval, advancing only as the chain's DAA score advances. This is not cosmetic — it is what makes the equivocation rule below *exactly decidable* (two anchors' cadence relationship is a pure function of on-chain data, no inferred wall-clock signing times needed), and it keeps trustee behavior well-defined when block production stalls: on a slowed chain a DAA-keyed trustee simply signs less often in wall-clock terms, rather than piling up multiple wall-clock-scheduled anchors inside one DAA interval — which under the equivocation rule below would otherwise make *honest* behavior on a stalled chain indistinguishable from double-signing. Published two ways, redundantly: as a tiny transaction in a dedicated subnetwork (the same `SubnetworkId::from_namespace` mechanism P5.2 uses for pool ops — a second, independent namespace, not the pool's) *and* gossiped directly over P2P, so an anchor's availability doesn't depend on it having been mined into a block yet.

**Consensus rule**: a chain conflicting with the latest valid anchor is invalid, regardless of accumulated proof-of-work — a second, faster-triggering enforcement of the same principle the existing finality-depth reorg refusal already encodes, just backed by an explicit trustee signature instead of pure work-accumulation depth.

### Anchor transaction format and quorum verification

```rust
struct FinalityAnchor {
    anchored_block: Hash,        // the block being certified, >= 600 DAA-score-units behind the signer's view of the tip
    anchored_daa_score: u64,      // that block's DAA score, for unambiguous depth verification
    signer_bitmap: u8,             // which of the 5 trustee keys signed (bit i = trustee i)
    signatures: Vec<[u8; 64]>,      // BIP340 Schnorr signatures, one per bit set in signer_bitmap, same order
}
``` 
Trustee public keys are **hardcoded in software** (shipped with each release, the same trust model the genesis block and DNS seeders already use — no on-chain registration mechanism, since the whole point is these keys predate and bootstrap trust in the chain, not the other way around). Verification: recover the ≥3 signing trustee `XOnlyPublicKey`s from `signer_bitmap` against the hardcoded set, verify each signature over `H("FinalityAnchor" || anchored_block || anchored_daa_score)` (a new domain-separated hash, same macro convention as every other purpose-specific hash in this spec), and require `signatures.len() >= 3` with no repeated signer.

### Fail-open liveness

**No anchor arriving is never a halt condition.** Staleness is DAA-score-defined (v1.1, consistent with the cadence redefinition above): the anchor-conflict rule is enforced only while `tip_daa_score − latest_valid_anchor.anchored_daa_score ≤ depth + 3×interval` (at launch: `600 + 3×300 = 1,500` DAA-score-units, ~2.5 minutes at nominal 10 BPS). The moment the latest valid anchor falls further behind than that, the rule simply stops being enforced until a fresh anchor arrives — the chain falls back to ordinary PoW/finality-depth security alone, exactly as if the mechanism didn't exist, for however long the gap lasts. This is automatic and requires no operator action to keep blocks flowing. What *is* required: every node **loudly alerts** the moment this fallback engages (a log line at error severity, an exposed RPC/metrics flag `finality_anchor_stale: true`, node-operator-facing, not silent) — liveness is never sacrificed for finality strictness, but operators must be able to see immediately that the extra protection layer is currently absent. **This visibility requirement extends to wallets** (v1.1, per review 2): the `finality_anchor_stale` flag must be queryable over public RPC so wallet software can surface degraded-finality periods to end users — a user accepting a large payment during an extended anchor outage is operating under plain-PoW guarantees and should be able to know it, not just their node operator.

### Equivocation and permanent key disqualification

**Equivocation proof — exact definition** (v1.1, replacing v1's informal "depth/timing windows overlap" per review 1): two validly-signed `FinalityAnchor`-domain messages from the *same* trustee key where

```
anchored_block_1 ≠ anchored_block_2
AND |anchored_daa_score_1 − anchored_daa_score_2| < interval
```

with `interval` being the cadence interval (in DAA-score units) of the decay stage active at `max(anchored_daa_score_1, anchored_daa_score_2)`. Presenting both signed messages together is self-contained cryptographic proof, verifiable by any node from on-chain data alone — no inferred signing times, no external input.

**Why this is exactly the right boundary, given the DAA-keyed cadence above.** An honest trustee signs at most one anchor per cadence interval, so two *different* blocks certified at DAA scores less than one interval apart is behavior no honest procedure produces — regardless of network conditions:

- **Adjacent honest anchors** certify blocks ≥ one full interval apart in DAA score (the cadence *is* the DAA spacing), so normal sequential anchoring is never flagged.
- **A stalled/slowed chain** doesn't create false positives: a DAA-keyed trustee signs less often in wall-clock terms on a slow chain, never twice within one DAA interval — this is precisely why the cadence was redefined off wall-clock (above).
- **Network partitions**: a partition brief enough that both sides' DAA scores stay within one interval of each other cannot yield two honest signatures from one trustee (one signing event per interval, and a single trustee is on one side of a partition at a time); a longer partition yields anchors whose `anchored_daa_score`s differ by more than an interval — outside the rule, correctly not flagged. Honest disagreement-across-views is therefore structurally excluded from the definition rather than adjudicated case-by-case.
- **The residual honest-risk case is operator error, not protocol ambiguity**: a trustee running two live signer instances against different views (a misconfigured failover, say) *can* trip this rule — and should: from the chain's perspective, one key certifying two different histories inside one interval is exactly the behavior the rule exists to disqualify, whatever its operational cause. Trustee operational guidance (one live signer per key, cold standby only) belongs in the P9.1 ceremony documentation, flagged here.

**Evidence and disqualification lifecycle** (v1.1 — specified end to end per review 2):

- **Evidence format**: the two complete conflicting `FinalityAnchor` messages plus their two signatures from the same trustee key — nothing else; self-contained.
- **Relay and inclusion**: broadcast as a transaction in the same dedicated anchor subnetwork (and gossiped over P2P like anchors themselves); any party may submit it — enforcement never depends on the honest trustees' cooperation.
- **Verification is fully objective**: recompute both signing hashes, verify both signatures against the accused key, check the overlap rule (different `anchored_block`, scores < one interval apart). Anything failing any of these — a forged signature, non-conflicting anchors, a wrong key — is simply an **invalid transaction** (rejected like any malformed transaction; false evidence can never disqualify anyone, and carries no penalty beyond its own rejection since it costs its submitter a normal fee).
- **Activation point, exactly**: disqualification takes effect for all anchor verification performed in validation contexts whose POV chain includes the block containing the accepted evidence — i.e. from that block onward, deterministically. Anchors already accepted in prior contexts are unaffected (no retroactive re-validation of settled history); every node reaches the identical disqualification state at the identical point because the evidence is on-chain data.
- **Storage**: a consensus-tracked deny-list (part of chain state, committed like any other consensus state, surviving pruning the same way pool state does per P5.4's reasoning).
- **Permanence**: no un-disqualify mechanism and no key replacement short of an explicit hard fork. If disqualification ever reduces the count of *live* trustee keys below 3, the quorum can no longer produce valid anchors at all, and the fail-open rule above engages automatically and stays engaged until a hard fork replaces the compromised key(s) — a slow, deliberate recovery path is correct here; an automatic key-replacement mechanism would just relocate the trust assumption, not remove it.

### Trustee DoS

Indistinguishable, at the protocol level, from ordinary unavailability — covered completely by "fail-open liveness" above. No separate mechanism exists (or should exist) to detect *why* anchors stopped arriving (uncooperative trustees vs. network partition vs. coordinated attack are all operationally identical from a syncing node's point of view: no valid anchor within the grace window, fall back to plain PoW, alert loudly).

### The hard-coded sunset

**Retirement trigger** (both required, restated from the plan's own framing): sustained difficulty ≥ threshold **T** for **M = 6 months**, **and** at least **K = 5 years** elapsed since genesis. Both conditions independently defeat the same attack: a patient adversary mining honestly to inflate difficulty, tripping a threshold early, then attacking the newly-unprotected chain. The **M**-month sustained-median requirement defeats a brief spike (six real months of elevated difficulty is a real, expensive, sustained cost — not a cheap momentary rental); the **K**-year floor independently caps *how early* retirement can ever trigger no matter how fast difficulty rises, giving the trustees and community years of runway to observe and react to any anomalous growth pattern before real protection is ever removed. Neither condition alone is sufficient; this is deliberate, per the plan's own reasoning.

**T, defined without needing an external oracle**: the plan's own rationale motivates comparing to Kaspa mainnet's difficulty ("any sliver of Kaspa's ASIC fleet"), but that value isn't on-chain data Marigold's own consensus can deterministically verify — a threshold referencing it would violate the plan's own "exact deterministic function of on-chain data" requirement (a fuzzy/external definition is a chain-split bug, stated explicitly in the plan). **T is instead defined purely against Marigold's own genesis difficulty**: `T = 10⁶ × genesis difficulty target`. A million-fold sustained increase from a cold, near-zero-hashrate launch is a strong, self-contained signal of real organic adoption — order-of-magnitude larger than a transient single-farm rental attack could plausibly sustain for six months, fully computable from data every node already has (no external price feed, no oracle, no off-chain input of any kind). This is a genuine refinement over the plan's literal Kaspa-relative framing, not just a restatement of it — flagged as a calibration point subject to revisiting with real early-network data, the same treatment P1.8's stamp sizing and P2.5's genesis timestamp already received; the *mechanism* (a fixed multiplier of Marigold's own genesis difficulty, deterministically checkable by every node) is the durable part of this decision, the exact `10⁶` less so. **Calibration is a hard pre-launch gate, not a caveat** (raised in review 1, escalated to a P0 requirement by review 2 — both adopted): being a *relative* multiplier, T's absolute meaning depends entirely on what the actual genesis difficulty (P9.5's final regeneration) turns out to be — `10⁶ ×` an extremely low cold-launch difficulty can still be modest in absolute hashrate terms. **Before this parameter freezes for mainnet (gated into FORK-PLAN's P9.5), a quantitative sensitivity model must be produced** covering at minimum multipliers `10⁴, 10⁵, 10⁶, 10⁷, 10⁸ × genesis` under plausible launch conditions, estimating for each: sustained hashrate/hardware cost, six-month energy cost, rentable-hashpower availability, expected organic trajectory, difficulty-manipulation capability, attacker capital before and after the threshold, and the probability of reaching the threshold without artificial stimulation (review 2's list, verbatim). `10⁶` is the working value, **explicitly not final** until that model exists; the multiplier moves if the model says so, the mechanism doesn't. The model must also answer review 2's sharpest framing of the core game-theoretic question: can a well-funded actor afford to sustain a qualifying six-month difficulty window *and* retain enough resources to exploit the weakened chain afterward — i.e., the analysis must compare attack cost against post-retirement extractable value, not just declare the window "expensive."

**"Sustained," defined exactly** (closing the "months, not moments" loophole precisely): reuse the existing difficulty-sampling infrastructure (`difficulty_window_size`/`DIFFICULTY_SAMPLED_WINDOW_SIZE`, [consensus/core/src/config/params.rs](../../consensus/core/src/config/params.rs), [consensus/core/src/config/constants.rs](../../consensus/core/src/config/constants.rs) — the same windowed-median mechanism already driving real-time difficulty adjustment, not a new sampling scheme). The retirement condition is met at the first DAA score where the sampled difficulty median has remained `≥ T` at **every** difficulty-window checkpoint across a trailing span of `M × 30.4375 days` (157,788,000 DAA-score-units at 10 BPS) — checking every checkpoint in the window, not just its two endpoints, is what makes this a real sustained-duration test rather than a start/end comparison an attacker could game by dipping below T only briefly outside the sampled instants.

### Gradual decay, not a cliff edge

The plan's own example shape, with concrete `ForkActivation`-staged thresholds:

Cadence intervals are DAA-score-defined at every stage (consistent with the v1.1 redefinition above); the wall-clock column is the nominal-10-BPS equivalent for readability only:

| Stage | Cadence interval (DAA-score units) | ≈ wall-clock | Trigger |
|---|---|---|---|
| 0 — Launch | 300 | 30s | Genesis |
| 1 — Early easing | 36,000 | 1 hour | Difficulty first sustains ≥ T for M months (the difficulty half of retirement met; the K-year floor not yet required) — a real, if not yet sufficient, signal |
| 2 — Retirement | 864,000 | 1 day | Full trigger: T sustained for M months **and** K = 5 years elapsed |
| 3 — Long-tail advisory | 6,048,000 | 1 week | 2 further years elapsed after Stage 2 (631,152,000 DAA-score-units later) |
| 4 — Expired | *(none — advisory only)* | — | Hard maximum DAA score reached (below), unconditionally |

Fail-open alerting thresholds (the `depth + 3×interval` staleness bound) scale with whatever stage is currently active, so the *relative* tolerance for a missed anchor stays constant in proportion even as the absolute cadence stretches.

**Hard maximum DAA score: 6,311,520,000** (20 years from genesis at 10 BPS, computed as `20 × 12 × 2,629,800 × 10`). At and beyond this score, trustee keys are **consensus- expired unconditionally** — no anchor, however validly signed, has any consensus effect from this point forward, regardless of Stage 3's actual state or whether the difficulty condition was ever even met at all. This is deliberate and non-negotiable per the plan's own instruction: **trust must end even if network growth disappoints.** A chain that never reaches the T/M/K retirement trigger simply runs out its full 20-year anchor lifespan on plain PoW security alone from that hard-coded point on. **Extending trustee life beyond this score requires an explicit hard fork** — the default, unforced outcome is always expiry, never renewal. **Transition obligation** (v1.1, per review 2): the final decay stages need a user-facing story, not just a consensus rule — nodes and wallets should surface the current stage (via the same RPC channel as `finality_anchor_stale`) so the declining protection is visible as it declines, and post-expiry, historical anchors retain **no** bootstrap relevance (the ratchet rule below simply stops applying at the hard maximum — a fresh node syncing after expiry is a plain PoW node, full stop). The wallet/UX detail belongs to the P9.x launch-material steps; the consensus-side facts are fixed here.

### IBD / bootstrap trust root and the anchor ratchet (v1.1, per review 2)

Review 2 correctly observes that anchor-aware IBD makes trustee signatures and anchor discovery a **bootstrap trust root**, and that querying multiple peers improves availability, not cryptographic independence. Made explicit:

- **What a fresh node trusts, and where it comes from**: the 5 trustee public keys (and the deny-list evidence rules) ship in the software release — the same trust root as the genesis block, the network parameters, and the DNS seeder list. A user who cannot trust their software distribution has no security under *any* design; this mechanism adds no new root, it adds new *material* under the existing one. Stated plainly rather than implied.
- **Anchor discovery**: during IBD the node requests the latest known anchors from **every** connected peer (not just the sync peer) and from the anchor subnetwork's on-chain history as it syncs. It accepts the valid anchor with the highest `anchored_daa_score` seen from any source — anchors are just signed data; a single honest peer (or one on-chain copy) suffices to deliver the newest one.
- **The anchor ratchet**: a node **persists the highest-scoring valid anchor it has ever accepted** and never adopts a chain conflicting with it — across restarts, resyncs, and reorgs. This is the rollback-resistance rule: once a node has seen an anchor, no adversary can walk it back to a pre-anchor view by suppressing newer anchors later.
- **Suppression, exactly what it buys an adversary**: an eclipse-level adversary presenting only an *old* valid anchor (or none) to a fresh node cannot forge anchored history — the node still enforces at-or-beyond the newest anchor it *does* hold, and staleness alerting (fail-open rule) fires loudly because that anchor is far behind the presented tips. Suppression degrades a fresh node's protection *toward* plain PoW security (the fail-open floor), never below it, and never silently. The residual risk — a fully eclipsed fresh node fed an attacker chain plus no recent anchors — is the same residual eclipse risk every PoW chain's IBD already carries, now with an alarm attached; naming it honestly rather than claiming the mechanism closes it.

### Attack cases (P5.8's own verify condition, answered explicitly)

- **k-key compromise (< 3 keys)**: no effect — 3-of-5 cannot be satisfied, no valid anchor can be forged.
- **k-key compromise (≥ 3 keys, i.e. quorum-level compromise)**: stated honestly, not minimized — a genuinely compromised majority quorum *can* sign a false anchor endorsing an attacker's chain, exactly the trust this mechanism places in the trustees. This is the same threat model every k-of-n federation carries; the mitigation is trustee selection (independent orgs/geos, per the plan) making simultaneous majority compromise a real practical barrier, and the entire reason a hard-coded sunset exists at all — this trust is bounded in time by design, never indefinite.
- **Equivocation**: any node presenting two signed anchors from one key meeting the exact overlap rule (different `anchored_block`, `anchored_daa_score`s less than one cadence interval apart) triggers permanent disqualification of that key, deterministically, from on-chain data alone, without requiring the honest trustees' cooperation to enforce — and the rule structurally excludes honest partition disagreement and normal sequential anchoring (see the exact definition above).
- **Trustee DoS**: latest valid anchor falls behind the `depth + 3×interval` DAA-score staleness bound → fail-open (plain PoW security resumes automatically) with a loud, node-operator-visible alert — never a chain halt.
- **Difficulty-inflation-then-attack**: defeated by the dual T-and-K condition — a brief inflation fails the M-month sustained-median check; even a genuinely sustained one cannot trigger retirement before the K = 5-year floor regardless of how fast it arrives.
- **Anchor-free chain offered to a syncing node**: IBD requires learning the current latest valid anchor *before* evaluating any candidate chain (from hardcoded trustee keys shipped in software, cross-checked against multiple independent bootstrap peers/seeders) — any candidate chain not building at-or-beyond that anchored block is rejected outright regardless of accumulated work, so a higher-work anchor-free attacker chain loses during sync, exactly as the plan requires.

✅ *Verify:* exact values stated for every named parameter — k=3, n=5, cadence=300 DAA-score-units at launch (~30s at nominal 10 BPS), depth=600 DAA-score-units, T=10⁶×genesis difficulty, M=6 months (157,788,000 DAA-score-units), K=5 years (1,577,880,000 DAA-score-units), the 5-stage cadence-decay schedule (all intervals DAA-score-defined), and the hard maximum DAA score (6,311,520,000, 20 years). Every named attack case answered explicitly, including an honest (not overstated) account of what a genuine majority-quorum compromise can and cannot do, and an exactly-decidable equivocation rule that structurally excludes honest partition disagreement.
