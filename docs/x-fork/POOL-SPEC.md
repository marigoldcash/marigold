# Marigold Note Pool — Specification

Phase 5 of [FORK-PLAN.md](../../FORK-PLAN.md). A complete written spec of the note pool,
frozen and externally reviewed (P5.9) before any implementation begins (Phase 6+). Each
`## P5.N` section below is that plan step's deliverable — do not implement against this
document until P5.9 has closed.

**The design in one paragraph** (context for every section below): a **note** is
`(d, pk, sn)` — denomination `d`, current owner's public key `pk`, and a stable serial
number `sn` that never changes across a note's life. The **pool** is a plaintext,
consensus-maintained map `sn → (d, pk)` that every full node holds, analogous to the UTXO
set. Ownership is purely "whoever can sign with the private key matching the note's current
`pk`" — no encryption, no zero-knowledge proofs anywhere in this design. Five operations
mutate the pool: **mint**, **rotate**, **split**, **merge**, **redeem** (defined precisely in
P5.2-P5.3).

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

Total: **65 bytes** when a note is serialized as a self-contained unit (e.g. in a wallet's
key database, P5.6). Inside the pool state map itself, `sn` is the map *key*, so only 33
bytes (`d` + `pk`) are the stored *value* per entry — see "Pool state map" below.

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

Tags 8-255 are reserved (unassigned) — a future denomination-set extension (e.g. via
hard fork, matching how the ladder itself could grow) fits in the same 1-byte field without
widening the Note struct. A tag is a **lookup index into a consensus-defined constant
table**, not a value nodes compute — this makes the table itself the single source of
truth for "what denominations exist," directly mirroring how `SUBSIDY_BY_MONTH_TABLE`
(P3.2) is one canonical const array rather than a formula recomputed ad hoc.

#### `pk` — owner public key (32 bytes)

Reuses Kaspa's existing x-only BIP340-style Schnorr public key format **exactly** —
the same 32-byte payload as address `Version::PubKey`
([crypto/addresses/src/lib.rs](../../crypto/addresses/src/lib.rs)) and the same key type
(`secp256k1::XOnlyPublicKey`) used for signature verification in
[crypto/txscript](../../crypto/txscript/src/lib.rs). This is a deliberate, not incidental,
reuse: it means a note's `pk` is a real Kaspa/Marigold-format public key, signatures over
pool ops reuse the exact same `secp256k1` Schnorr signing/verification call path already
proven in `consensus/core/src/sign.rs`, and no new curve or signature scheme enters the
codebase for the pool feature. Signatures over pool ops are the standard 64-byte BIP340
Schnorr signature (see P5.2 for the exact signed message).

Rotating a note replaces `pk` with a new one; `pk` alone can therefore never identify a
note across its lifetime — which is exactly why `sn` exists.

#### `sn` — serial number (32 bytes, `kaspa_hashes::Hash`)

**Answering the plan's explicit question — is `sn` needed, and if so what is it?** Yes:
since `pk` changes on every rotation, the pool map must be keyed by something stable, and
no other field is a candidate. `sn` is that key. Its value is defined uniformly for every
note regardless of which operation created it (mint, split, or merge all create new notes):

```
sn = H_serial(creating_tx_id || output_index_within_op)
```

where `creating_tx_id` is the 32-byte transaction ID of the pool-op transaction that
created this note, `output_index_within_op` is a `u32` (little-endian) index of this note
among the notes that specific op created, and `H_serial` is a new domain-separated hash
function (see "Hashing" below). This directly mirrors how Kaspa already treats
`(transaction_id, output_index)` — a `TransactionOutpoint`
([consensus/core/src/tx.rs](../../consensus/core/src/tx.rs)) — as a globally unique handle
for UTXOs; `sn` is that same idea, collapsed into one fixed-size opaque hash instead of a
raw pair, because the pool map (P5.1 below) needs a single `Hash`-typed key to match the
existing `crypto/smt` sparse Merkle tree's key type. Two properties fall out for free:

- **Global uniqueness with zero extra consensus state.** No incrementing counter, no
  registry of "next available serial" — collision resistance comes from `creating_tx_id`
  already being unique (transaction IDs are themselves domain-separated hashes over the
  whole transaction) and the index disambiguating multiple notes from one op.
- **No rotation-of-`sn`.** `sn` is fixed at note creation and never appears as mutable
  state anywhere in this spec — every op that changes a note's `pk` (rotate; split and
  merge produce brand-new notes with brand-new `sn`s, they don't relabel old ones) leaves
  existing, untouched notes' `sn`s alone.

### The pool state map

```
PoolState : sn → (d, pk)
```

Implemented as a **sparse Merkle tree** using the existing
[`crypto/smt`](../../crypto/smt/src/lib.rs) crate — the same 256-bit-depth, `Hash`-keyed,
`Hash`-valued SMT already used in production for the seq-commit feature (KIP-21,
`consensus/seq-commit/`, `consensus/smt-store/`). This is a direct architectural
precedent, not a new pattern: seq-commit already proves this exact crate can back a
plaintext, node-verified, consensus-committed key→value map inside this codebase.

- **Key**: `sn` (already a 32-byte `Hash` — no additional hashing needed to use it as an
  SMT key).
- **Leaf value**: `H_leaf(d || pk)` — a domain-separated hash of the 1-byte denomination
  tag concatenated with the 32-byte pubkey (33 bytes input). The SMT stores `Hash → Hash`
  (key → leaf hash), per `crypto/smt`'s design — the tree never stores `(d, pk)` directly,
  only its hash, so proving/verifying an entry means presenting `(d, pk)` alongside an SMT
  proof and recomputing `H_leaf(d || pk)` to check against the proven leaf hash.
- **Removal** (redeem, and the "old" side of split/merge/rotate replacing an entry):
  `crypto/smt` represents removal as inserting the all-zero hash at that key (`remove()`,
  `tree.rs:177`) — the pool follows the same convention. A removed serial's key becomes
  provably absent (a non-inclusion proof, see below), never reused for a different note
  (uniqueness from the `sn` derivation rule above already guarantees no future note ever
  collides with a removed one's key).
- **Production update path**: consensus code must use the pure function
  `compute_root_update::<H, S>(store, current_root, leaf_updates)` (`crypto/smt`
  `tree.rs:231`) against an `SmtStore`-backed persistent store — the same function
  `consensus/smt-store`'s `SmtProcessor::build` already uses for seq-commit — **not** the
  mutable in-memory `SparseMerkleTree::insert`/`remove` API, which is gated
  `#[cfg(any(test, feature = "test-utils"))]` and unavailable in production builds.

### The pool commitment

A new 32-byte `Hash` field in the block header, `pool_commitment`, holding the SMT root
of `PoolState` as of that block. This is a **new header field**, not a reuse of an
existing one (seq-commit reuses `accepted_id_merkle_root` post-toccata; overloading that
same field for a second, unrelated commitment would make one field mean two different
things depending on which of two independent forks activated — confusing and fragile).
Adding a field to `Header` ([consensus/core/src/header.rs](../../consensus/core/src/header.rs))
is a consensus-breaking, hard-fork change: it needs a new block version (the same pattern
`TOCCATA_BLOCK_VERSION` used in `consensus/core/src/constants.rs`) gated by a
`ForkActivation` (Phase 6's job, not this spec's — noted here only so the byte layout is
unambiguous: `pool_commitment` exists in headers from that activation's DAA score onward).

**Sync/verify flow** (mirrors `utxo_commitment`'s MuHash flow exactly, adapted to an SMT —
detailed fully in P5.4; stated here because it's inseparable from what the commitment
*is*): a node computes `pool_commitment` incrementally as it processes each block's pool
ops via `compute_root_update`, and — for a node syncing from a pruning point — downloads
the pool state in chunks (mirroring `PruningPointUtxosetChunkStream`,
`protocol/flows/src/ibd/streams.rs`), folds each chunk into a running SMT build, and
finally checks the resulting root equals the pruning-point header's `pool_commitment`
before trusting the downloaded state — identical in shape to
`Consensus::import_pruning_point_utxo_set`'s `imported_utxo_multiset_hash !=
new_pruning_point_header.utxo_commitment` check
(`consensus/src/pipeline/virtual_processor/processor.rs`).

### Hashing (new domain-separated hash functions required)

Following the existing `crypto/hashes` convention (each hash purpose gets its own
domain-separated tag — see `TransactionSigningHash`, `MuHashFinalizeHash`,
`SeqCommitActiveNode` in [crypto/hashes/src/hashers.rs](../../crypto/hashes/src/hashers.rs)),
the pool feature needs two new ones:

- **`NotePoolSerialHash`** — computes `sn = H(creating_tx_id || output_index_u32_le)`.
- **`NotePoolLeafHash`** — computes the SMT leaf value `H(d_u8 || pk_32bytes)`.

Plus one new concrete `SmtHasher` impl (a `NotePoolSmt` type, structurally mirroring
`consensus/seq-commit/src/hashing.rs`'s existing hasher) supplying the `CollapsedHasher`
and precomputed `EMPTY_HASHES` the SMT needs, built on `NotePoolLeafHash` as its leaf
domain.

### Why not skip `sn` and key the map by `pk` instead?

Addressed explicitly since it's the obvious alternative: `pk` is not stable (rotation
replaces it) and, per P5.6, **is not required to be unique** — multiple notes may
deliberately share one `pk` (the POS landing-pad flow, P5.6). Keying the pool by `pk`
would make "one pk, five notes" inexpressible (a map key can only point to one value) and
would leak nothing extra in exchange, since `pk` is already public in the pool regardless
of which field is the map key. `sn` is the only field satisfying "stable across the note's
life" and "unique per note" simultaneously.

✅ *Verify (P5.1's own condition): every field has a byte size — restated compactly:*
`d`: 1 byte · `pk`: 32 bytes · `sn`: 32 bytes · *pool commitment*: 32 bytes (header field)
· *SMT leaf value*: 32 bytes (`H(d || pk)`, 33-byte preimage) · *serial preimage*: 36 bytes
(`tx_id: 32` + `index: u32 LE = 4`).
