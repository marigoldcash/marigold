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

---

## P5.2 — Transaction format

### Subnetwork and payload encoding

Pool ops ride in ordinary Kaspa/Marigold transactions tagged with a dedicated **user-lane**
subnetwork ID — `SubnetworkId::from_namespace([0x50, 0x4f, 0x4f, 0x4c])` ("POOL" in ASCII,
chosen only for memorability; any unclaimed namespace works equally well and Phase 6 may
substitute one if this happens to collide with something reserved by then). This uses the
existing, already-implemented namespace mechanism
([consensus/core/src/subnets.rs](../../consensus/core/src/subnets.rs)) that backs Toccata's
"non-native/non-coinbase subnetworks (user lanes)" feature
([consensus/core/src/constants.rs](../../consensus/core/src/constants.rs)) — **not** the
reserved single-byte `RegistrySubnetwork` path, which (checked directly: `grep` for its only
non-definition usages) exists solely in test fixtures today, with no active
registration/dispatch mechanism to build on. The payload is a single Rust enum,
`PoolOp`, encoded with `borsh::to_vec` — the idiomatic in-repo pattern for "typed struct ⇄
opaque transaction-payload bytes" (e.g. `wallet/core/src/deterministic.rs`,
`wallet/macros/src/wallet/server.rs`), used in preference to the coinbase payload's
hand-rolled little-endian packing (`consensus/src/processes/coinbase.rs`), which is a
fixed, non-extensible legacy format specific to that one use.

```rust
enum PoolOp {
    Mint(MintOp),
    Transfer(TransferOp),
    Redeem(RedeemOp),
}
```

Only **three** wire-format variants, not five — see "Unifying rotate/split/merge" below for
why. Borsh enum tag = 1 byte.

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

`sn` for every note a `PoolOp` creates is *not* stored in the payload — it's derived
per P5.1's rule, `H(this_tx_id || index_among_all_notes_this_op_creates)`, entirely from
data already implicit in the enclosing transaction. This is a deliberate space saving (no
reason to spend 32 bytes writing down a value every node independently computes the same
way) and, more importantly, removes any possibility of a payload lying about a note's `sn`.

### Unifying rotate/split/merge into one `Transfer` op

The plan's own framing already treats split/merge as "a transfer with a different
denomination multiset in vs. out" — taken literally, rotate (same multiset), split (one
note → many smaller), and merge (many notes → one larger) are all just **different
multiset shapes of the same underlying operation**: consume some notes, produce some notes,
under one conservation check. Rather than three near-identical wire formats, `Transfer` is
defined once and "rotate" / "split" / "merge" become purely descriptive labels for what a
given `Transfer`'s multiset happened to do — a UX/documentation distinction, not a
protocol one. This also means one `Transfer` transaction can freely mix these (e.g.
split-and-rotate-part-of-it in one op) without inventing a fourth wire shape.

```rust
struct TransferOp {
    consumed:  Vec<SignedGroup>,    // notes being spent — total petal value = Σ over all groups' serials' denominations
    produced:  Vec<NewNote>,         // notes being created — total petal value = Σ
    freshness: FreshnessAnchor,       // covered by every group's signature (see below)
}
```

**Multiple `SignedGroup`s exist because one transaction may need to spend notes under
different current `pk`s** — e.g. a customer's wallet holding three notes each with its own
key. Each group's serials must currently share one `pk` (a single Schnorr signature can
only verify against one key); a wallet combining differently-keyed notes in one `Transfer`
supplies one group per distinct key. This is also exactly what P5.6's merchant "sweep"
flow needs in the *other* direction: many serials sharing **one** `pk` (the POS landing
pad), authorized by a **single** group with one signature covering all of them.

**Conservation and fee**: valid iff `Σ(consumed note values) ≥ Σ(produced note values)`;
the difference is the transaction's fee — computed exactly the way Kaspa already computes
ordinary transparent fees (`value-in − value-out`), just applied to notes' underlying
petal values instead of UTXO amounts. This single rule is what makes fee stamps require
*no new mechanism at all* — see "Fee-stamp mechanics" below.

### Mint and Redeem

```rust
struct MintOp {
    new_notes: Vec<NewNote>,
}
```
The transaction's ordinary transparent **inputs** (standard signed UTXO spends, verified
by the existing txscript engine exactly as any transparent transaction) must sum to at
least `Σ(new_notes' petal values)`; any excess is an ordinary transparent change output or
the transaction fee, both completely standard Kaspa mechanics — mint needs no note-level
signature at all, since nothing pre-existing in the pool is being touched. This is the
"self-funding" value-touching op the P1.8 flag asks P5.2 to spec: mint pays its fee the
same way any transparent Kaspa transaction always has, no fee stamp required, because it
already holds transparent value to pay from.

```rust
struct RedeemOp {
    consumed:  Vec<SignedGroup>,
    freshness: FreshnessAnchor,
}
```
The transaction's ordinary transparent **outputs** hold what the redeemed notes become;
valid iff `Σ(consumed note values) ≥ Σ(transparent outputs) + fee` — the transparent-side
mirror of `Transfer`'s conservation rule, and, like mint, self-funding: redeem already
produces transparent value, so it pays its fee from that, no stamp required. Redeem is
structurally `Mint` read backwards (transparent-in → notes-out vs. notes-in →
transparent-out), matching the plan's own five-op description exactly.

### Signature scheme and the freshness anchor

A pool-op signature is **not** a Kaspa/Marigold txscript input signature — notes have no
transparent output/script to spend, so the existing per-input sighash machinery
(`consensus/core/src/hashing/sighash.rs`) doesn't apply; it signs over transparent inputs,
outputs, `gas`, and `subnetwork_id`, none of which describe "authorize this note's
ownership to change." This needs its own domain-separated signing hash, following the
exact macro convention already used for every other purpose-specific hash in this codebase
([crypto/hashes/src/hashers.rs](../../crypto/hashes/src/hashers.rs) — `TransactionSigningHash`,
`MuHashFinalizeHash`, `SeqCommitActiveNode`, …):

```
NotePoolTransferSigningHash = H(
    "NotePoolTransferSig"                         // domain tag
    || sorted(group.serials)                        // this group's own serials, 32 bytes each
    || op.produced                                    // EVERY note this whole op creates, d||pk, 33 bytes each
    || freshness.anchor_daa_score                      // 8 bytes, LE
)
```

Every `SignedGroup` in a `Transfer`/`Redeem` signs over the **entire** op's `produced` list
(not just "its share"), not merely its own serials — this is what makes a `Transfer`
atomic: no signer is vouching for their serials being spent in just *some* context, they're
vouching for this *exact* whole-transaction shape, so no group's signature can be lifted
into a transaction with a different produced-notes list. No `tx_id` is signed over (and
deliberately can't be — the enclosing transaction's ID is a hash that includes this very
payload, so signing it would be circular); replay safety instead comes from two properties
working together:

1. **A successfully-executed group's signature can never be reused.** The instant a group
   executes, every serial it covered now has a different current `pk` (or no longer exists,
   for redeem), so re-submitting the identical signed message fails P5.3's "signature
   verifies against the serial's *current* `pk`" check — permanently, not just once.
2. **A signed-but-never-executed op has a bounded shelf life.** Without anything binding
   the signature to one specific transaction, a valid signed group could otherwise be
   broadcast at any arbitrary future time by whoever holds it (the "stale invoice" risk
   P5.5 names explicitly for sign-to-fresh-pk mode). `freshness.anchor_daa_score` bounds
   this: P5.3 must reject the op once `current_daa_score − anchor_daa_score` exceeds a
   fixed **freshness window**.

**Recommended freshness window: 36,000 DAA-score units** (≈1 hour at 10 BPS). Reasoning,
stated explicitly since the plan calls this a deliberate choice, not a default to leave
implicit: long enough that no ordinary in-person or remote payment flow is at risk of the
signature expiring mid-transaction (P5.5's bearer and sign-to-fresh-pk flows both settle
in seconds at 10 BPS; an hour is generous headroom, not a tight budget), short enough that
a leaked or abandoned signed op — an unpaid invoice, a bearer QR photographed but not yet
handed over — stops being a live liability within the same session it was created, not
days later. Same category of "needs real-world calibration, not a first-principles
derivation" as P1.8's stamp-sizing note; recorded here as a concrete recommended default,
adjustable at Phase 6/P6.6 calibration, not a placeholder.

### Fee-stamp mechanics (P1.8 flag — every bootstrap case, worked through the wire format)

No separate "stamp" field or op type exists in this format — a stamp is just an ordinary
consumed serial in a `Transfer`'s `consumed` list with no matching value in `produced`,
which the conservation rule (above) already turns into fee automatically. This single
mechanism covers every case P1.8 named:

- **Pure rotate** (`Transfer` with `consumed` and `produced` denominations identical) has
  zero natural conservation slack — `Σconsumed = Σproduced` exactly, so it pays *nothing*
  unless an extra serial is added to `consumed` with no corresponding `produced` entry:
  that's the "pre-existing stamp" P1.8 says pure rotate requires. Concretely: rotating one
  0.1-note to a new key, with a 0.01-note attached purely as a stamp, is `consumed: [group
  for the 0.1 note, group for the 0.01 stamp], produced: [one new 0.1-denomination note]`
  — the 0.01 simply has no matching output, and its value becomes the fee.
- **Self-funding split/merge** (P1.8's worked example: `100 → 9×10 + 9×1 + 9×0.1 + 9×0.01
  (= 99.99) + 0.01 fee`) needs no separate stamp at all — `consumed` lists the one 100-note,
  `produced` lists the 36 smaller notes summing to 99.99, and the 0.01 gap is the fee
  automatically, computed exactly like the pure-rotate case but arising from the split's
  own arithmetic rather than an attached extra serial.
- **Handovers include a stamp** (P1.8's option 2): the bearer bundle (P5.6) carries a
  second note's private key alongside the primary note's; the receiver's eventual rotate
  op lists both serials in `consumed`, only the primary note's denomination in `produced`.
  No format difference from the pure-rotate case above — "the stamp came bundled with the
  note" is a wallet/UX fact, not a wire-format one.
- **Mint produces stamps** (P1.8's option 3): trivially expressible — a `MintOp` whose
  `new_notes` includes small denominations alongside larger ones; nothing pool-specific to
  add here since mint already supports minting any combination of denominations in one op.

Congestion pricing (P5.7 will note this as a privacy limitation) falls out for free too:
attaching a bigger or additional stamp increases `Σconsumed − Σproduced`, which increases
the transaction's fee, which raises its priority in the existing mempool fee-per-mass
ordering — no protocol-level fee schedule needed, exactly as P1.8 already concluded.

### A consensus-rule dependency this format creates (flagged for P5.3)

Checked directly against current validation code, not assumed: a pure `Transfer` (or
`Redeem`) has **zero transparent inputs** by design ("touches no transparent value"), but
`check_transaction_inputs_count`
([consensus/src/processes/transaction_validator/tx_validation_in_isolation.rs:78-80](../../consensus/src/processes/transaction_validator/tx_validation_in_isolation.rs))
currently rejects *any* non-coinbase transaction with `tx.inputs.is_empty()` —
`TxRuleError::NoTxInputs` — with no existing exception for other subnetworks. (No
equivalent "zero outputs" rule exists, confirmed by its absence — only inputs are
currently required to be non-empty for non-coinbase transactions, so `Transfer`'s already-
empty `tx.outputs` needs no change.) **P5.3 must define an explicit consensus-rule
exception** — the natural shape mirrors the existing `!tx.is_coinbase()` guard,
generalizing it to also exempt the pool subnetwork ID from the zero-inputs check — this
isn't a new category of problem, just the coinbase precedent extended to a second
subnetwork that also legitimately has no transparent inputs.

### Worked byte-size estimates

All figures are **payload-only** (the pool-specific addition); every op also carries
standard Kaspa transaction overhead (version, input/output counts, `subnetwork_id`
(20 bytes), `gas`, `lock_time`) — small and already well-understood/bounded by existing
Kaspa serialization, not re-derived here. `TransactionOutpoint` (32-byte tx ID + 4-byte
index = 36 bytes, [consensus/core/src/tx.rs](../../consensus/core/src/tx.rs)) and a
P2PK-spend signature script (66 bytes, `wallet/core/src/tx/mass.rs`'s
`SIGNATURE_SIZE = 1 + 64 + 1`) are the only transparent-side costs `Mint`/`Redeem` add
beyond the payload.

| Op | Shape | Payload bytes |
|---|---|---|
| `Mint` | 1 new note | `1 + 4 + 33×1` = **38** |
| `Mint` | 5 new notes (a mixed-denomination bundle) | `1 + 4 + 33×5` = **170** |
| `Transfer` | plain rotate (1 group/1 serial, 1 produced note) | `1 + [4+(4+32+64)] + [4+33] + 8` = **150** |
| `Transfer` | rotate + attached 1-serial stamp (2 serials in 1 group, 1 produced note) | `1 + [4+(4+64+64)] + [4+33] + 8` = **182** |
| `Transfer` | self-funding split, 1→36 notes (P1.8's worked example) | `1 + [4+(4+32+64)] + [4+33×36] + 8` = **1,305** |
| `Transfer` | merchant sweep, 20 serials/1 group → 20 fresh-key notes | `1 + [4+(4+32×20+64)] + [4+33×20] + 8` = **1,385** |
| `Redeem` | 3 serials/1 group, no new notes | `1 + [4+(4+32×3+64)] + 8` = **177** |

Every realistic shape lands from tens of bytes to ~1.4 KB — comfortably inside "a few KB,"
and negligible against block mass limits: at `mass_per_tx_byte = 1`
([consensus/core/src/config/params.rs](../../consensus/core/src/config/params.rs)), a
1.4 KB payload costs ~1,400 mass units against a per-block budget of 500,000 (pre-toccata,
`prior_block_mass_limits`) to 1,000,000 (`new_transient_mass_limit`) — under 0.3% of a
single block's budget even for the largest worked example, with no coinbase-style
dedicated payload-length cap (`max_coinbase_payload_len = 204`) applying here at all, since
that constant is coinbase-specific.

✅ *Verify (P5.2's own condition): all five ops covered (Mint, and Transfer's three
descriptive shapes rotate/split/merge, and Redeem) with worked byte sizes; total
transaction sizes land at tens of bytes to ~1.4 KB, far inside the "few KB" target and a
small fraction of block mass limits. Fee-stamp mechanics specified for every named
bootstrap case (pure rotate, self-funding split/merge, bundled handover stamp, mint-produced
stamps) via one unified mechanism, no case left unaddressed.*

---

## P5.3 — Consensus rules

### The pool diff and composed view (mirrors the existing UTXO mechanism exactly)

Kaspa/Marigold already resolves UTXO double-spends between blocks merged in one GHOSTDAG
mergeset via a specific, existing mechanism — read directly from
[consensus/src/pipeline/virtual_processor/utxo_validation.rs](../../consensus/src/pipeline/virtual_processor/utxo_validation.rs)
(`calculate_utxo_state`, lines 109-165) rather than assumed: blocks in the mergeset are
visited in GHOSTDAG blue-topological order
(`ghostdag_data.consensus_ordered_mergeset_without_selected_parent`,
[consensus/src/model/stores/ghostdag.rs:183](../../consensus/src/model/stores/ghostdag.rs)),
starting from the selected parent. Each block's transactions are validated against a
**composed view** — the selected parent's UTXO state overlaid with a `mergeset_diff`
accumulated from every *already-processed* block earlier in that same ordering
(`selected_parent_utxo_view.compose(&ctx.mergeset_diff)`, line 131). A transaction whose
inputs conflict with what's already in `mergeset_diff` simply fails validation and is
excluded from that block's `accepted_transactions`
(`MergesetBlockAcceptanceData`/`AcceptedTxEntry`) — the block itself is not rejected, only
that one transaction. **This is "first accepted wins," concretely**: not a special rule
invoked on conflict, but the ordinary consequence of validating every block against
whatever state the blocks before it (in blue order) already committed.

The pool feature needs the exact same shape, one level added: a `PoolDiff` (the pool's
analog of `UtxoDiff`) accumulated alongside `mergeset_diff` during the same mergeset walk,
and a composed pool view (selected parent's pool state + accumulated `PoolDiff`) that
every pool-op transaction validates against, in the same blue-topological order, in the
same pass — a pool op and an ordinary UTXO spend can appear in the same transaction (mint,
redeem) and must be validated together, atomically, against both composed views at once.
**No new conflict-resolution rule is being invented here** — the parallel-blocks case
(two rotations of the same serial in two blocks of one mergeset) is answered entirely by
this existing mechanism applied to pool state: whichever block's pool op is processed
first (blue order) updates the composed pool view; the second block's conflicting op fails
step 1 below (the serial's current `pk` in the composed view no longer matches what its
signature was checked against) and is excluded from that block's accepted transactions —
same "loser becomes a no-op, not an invalid block" outcome real UTXO conflicts already
have today.

### Validation order — `Mint`

1. Every `new_notes[i].d` is a valid denomination tag (0-7; P5.1's table).
2. Standard txscript validation of the transaction's transparent inputs against the
   composed *UTXO* view — completely unchanged, the existing mechanism.
3. Conservation: `Σ(transparent inputs) − Σ(transparent outputs) − Σ(new_notes petal
   values) ≥ 0`; the result is the transaction's fee (subject to the same minimum-relay-fee
   mempool policy as any transaction — not a new consensus rule).
4. Apply to `PoolDiff`: insert `sn_i → H(d_i || pk_i)` for each new note, where
   `sn_i = H_serial(this_tx_id || i)` (P5.1). Uniqueness is guaranteed by construction
   (this transaction's ID cannot already have been used to derive an existing serial) —
   not an active check, but an invariant implementers should assert in testing.

### Validation order — `Transfer` (rotate/split/merge)

1. For every `SignedGroup` in `consumed`: every serial in `group.serials` exists in the
   composed pool view, **and** all of them currently share the exact same `pk` — if any
   two differ, the op is invalid (one signature cannot authenticate two different keys).
2. Recompute `NotePoolTransferSigningHash` (P5.2) over `group.serials`, the op's full
   `produced` list, and `freshness.anchor_daa_score`; verify `group.signature` against the
   shared current `pk` from step 1.
3. Freshness: `pov_daa_score − freshness.anchor_daa_score` is in `[0, 36000]` (the P5.2
   window) — reject both a stale anchor (too far in the past) and a future one
   (`anchor_daa_score > pov_daa_score`, which could otherwise let a signer pre-date a
   signature to extend its effective shelf life).
4. Every `produced[i].d` is a valid denomination tag.
5. Conservation: `Σ(consumed notes' current petal values, from the composed view) −
   Σ(produced notes' petal values) ≥ 0`; the result is the fee (same relay-fee policy note
   as `Mint`).
6. Apply to `PoolDiff`: remove every consumed serial's entry; insert
   `sn_i → H(d_i || pk_i)` for each produced note, `sn_i` derived the same way as `Mint`.

### Validation order — `Redeem`

1-3. Identical to `Transfer`'s steps 1-3, applied to `Redeem`'s own `consumed` list.
4. Conservation: `Σ(consumed notes' current petal values) − Σ(transparent outputs) ≥ 0`;
   the result is the fee — the transparent-side mirror of `Mint`'s rule.
5. Apply: remove every consumed serial's `PoolDiff` entry; the transparent outputs are
   applied to the ordinary UTXO diff exactly as any transaction's outputs already are — no
   change to that existing mechanism.

### Mass and fee costing

The plan's own guidance: "a rotate is one sig verify + one map update — cost it like a
normal 1-input tx; no special proof costs exist in this design." Concretely, using the
existing cost model (`consensus/core/src/mass/mod.rs`,
[consensus/core/src/config/params.rs](../../consensus/core/src/config/params.rs)):

- **Payload bytes** already cost `mass_per_tx_byte` (= 1) each, automatically, since the
  payload counts toward `transaction_estimated_serialized_size` — no new per-byte rate
  needed (P5.2's worked byte sizes are therefore already mass estimates, 1:1).
- **Each `SignedGroup`'s signature verification** costs one sigop-equivalent —
  `mass_per_sig_op` (=1000) for v0-style costing, or one `ComputeBudget` unit's worth
  (100 grams = 10,000 script-units, `consensus/core/src/mass/units.rs`) under the v1
  compute-budget model — charged **once per signature, not once per serial**: a
  20-serial sweep under one shared `pk` (P5.6) is one signature and therefore one
  sigop-equivalent, not twenty, which is what makes batch sweeps cheap by design, not an
  incidental side effect.
- **`Mint`/`Redeem`'s transparent side** costs exactly what it already would as an
  ordinary transaction — standard txscript sigops for spent inputs
  (`mass_per_sig_op`/compute-budget as today), `mass_per_script_pub_key_byte` (=10) for
  any transparent output scripts. Nothing pool-specific changes on that side.
- No zero-knowledge proof verification exists anywhere in this design (stated in P5.7 as
  a privacy limitation, restated here as a performance fact) — every pool-op cost is
  either a byte count or a small fixed number of Schnorr signature verifications, the same
  order of magnitude as costs the mempool already charges for today.

✅ *Verify (P5.3's own condition): every question in the checklist answered explicitly —
validation order stated per op (existence, signature, freshness, denomination validity,
conservation, in that order); the parallel-blocks double-rotate case is resolved by
citing and extending the exact existing mechanism (composed view over GHOSTDAG-ordered
mergeset), not a new rule; signature replay protection restated from P5.2's freshness
anchor and tied to the concrete consensus check (step 3 above); mass/fee costing defined
per op using existing cost-model constants, with the batch-sweep cost savings made
explicit.*
