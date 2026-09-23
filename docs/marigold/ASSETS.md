# Assets in the note pool: the foundation for issued notes

**Status: plan, 2026-09-23.** Founder-asked: lay the consensus foundations now, while the testnet is still the place to make consensus changes, so that a fiat-backed coin can be built on Marigold later without a hard fork on a community-run mainnet. The coin itself is far out; MAGLD comes first. What this page fixes is the *shape* the pool takes so that a second asset is a rule change gated by an activation, not a redesign. The stablecoin's own questions, the issuer, the reserve, the bank, and whether an issuer can freeze anything, are named at the end and deliberately left open.

The normative home of the pool is [POOL-SPEC.md](POOL-SPEC.md); once decided, this page becomes its P5.10 the way time-locked notes became P5.9.

## The idea in one paragraph

A note today is a serial, a denomination and a key. A note becomes a serial, an **asset**, a denomination and a key. MAGLD is the asset written as all zeros. Rotate, split, merge and the locked rotation stay exactly what they are and gain one check: the notes an operation produces carry the asset of the notes it consumed. Mint and redeem bridge MAGLD notes to and from the ledger and stay MAGLD-only, because no other asset has a ledger side. Two new operations, **issue** and **retire**, are the bridge for every other asset: they create notes from nothing and consume them to nothing, and both are authorized by the key of the lane whose tag the asset is. Conservation runs per asset, fees stay in MAGLD, and the pool commitment in every block header covers every note of every asset, so every node checks at every block that the notes of an asset in circulation equal what its issuer has issued minus what it has retired. That is the on-chain half of a reserve's proof; the off-chain half, that a bank holds the same number of dollars, is an attestation the issuer anchors in its own lane.

## What is decided now and what waits

**Now, on the testnet (the foundation):**

- the asset field on every note, in the leaf, the wire, the signing preimage, the state, the sync and the RPC, with all zeros the only asset any node accepts;
- per-asset conservation written into validation, which for MAGLD alone is the rule that already runs;
- per-asset outstanding totals as consensus state, so "how much of this asset exists" is a lookup, not a walk;
- the lane registry's claims as consensus state, tag to key, with rotation of the key by the current key, because an issuer is a lane key and consensus has to be able to look it up;
- the operation tags for issue and retire reserved, and their wire shapes written down;
- an activation parameter for the asset wire, set on testnet-10 and always-on for mainnet; and a second, separate activation for non-zero assets, never on mainnet until decided.

**Later, when there is an issuer (the coin):**

- the second activation, on a founder's decision, after the questions at the end are answered;
- wallets that hold more than one asset and show which is which;
- the issuer's own tooling: issue against a bank receipt, retire against a payout, anchor the monthly attestation;
- the freeze question.

The line between the two is that everything in the first list is a shape that costs nothing while unused, and everything in the second list is a policy that can wait. The asset field is the only part that is a hard fork, and mainnet has not launched, which is the whole reason for doing it now.

## P5.10 draft: the asset field

### The note

```
Note {
    asset: AssetId,          // 20 bytes: the issuing lane's subnetwork id; all zeros is MAGLD
    d:     DenominationTag,  // 1 byte, unchanged
    pk:    [u8; 32],         // unchanged
    sn:    Hash,             // unchanged
}
```

**`asset`** is the issuing lane's subnetwork id, twenty bytes, no second encoding. All zeros is the native subnetwork id already, so all zeros is MAGLD: the native asset, the one the ledger emits and the only one mint and redeem know. Any other value is a claimed lane ([LANE-REGISTRY.md](LANE-REGISTRY.md)): its tag, zero-padded to twenty bytes, exactly the id its anchoring transactions already carry. An explorer shows the tag; nothing needs a lookup to display an asset's name. The tag is the asset and the lane's key is its issuer; a lane whose tag is never used as an asset is an anchoring lane and nothing more.

**Tags grow to sixteen bytes** under `assets_activation`, from the five of wide lanes, with the alphabet widened from capitals and digits to capitals, digits, the dot and the hyphen. Founder-asked 2026-09-23: a ticker with its exchange must fit, and the Hong Kong and mainland forms do not fit in eight bytes, let alone five. Sixteen holds every form in use: `0700.HK` (7), `09988.HK` (8), `600519.SS` and `000001.SZ` (9), and the ISO 10383 market codes if a registry policy ever prefers them, `700.XHKG` (8), `600519.XSHG` and `000001.XSHE` (11). Consensus already asks only that the bytes after the tag are zero (`check_transaction_subnetwork`), so the change is the tail length under the activation and the wallet's alphabet check; every lane claimed so far is the same twenty bytes under both rules, as it was when four-letter lanes became five. Tags of one byte stay impossible, since `[x, 0×19]` is a reserved system id. The four bytes past sixteen stay zero, which keeps a lane id visibly a lane id and leaves room for one more widening. Which exchange suffix an issuer writes is not a consensus matter: the registry's listing policy (trustee endorsement for listed tickers, DECISIONS 2026-09-22) is where `0700.HK` and `700.HK` are kept from both being claimed, the chain only keeps them from being the same lane.

The denomination ladder is the same for every asset: tag `d` is worth 10 to the power of `d` minus 2 units of the asset, so tag 0 is one cent of a dollar-denominated asset exactly as it is 0.01 MAGLD. The table in P5.1 is a table of ladder positions; the unit is the asset's. No per-asset table exists and none is needed.

### The leaf

Every existing commitment must stand, so the MAGLD leaf does not change: `H_leaf(d || pk)` unlocked and `H_leaf(d || pk || refund_pk || until_daa)` locked, exactly P5.9. A note of any other asset has the asset in front: `H_leaf(asset || d || pk)` and `H_leaf(asset || d || pk || refund_pk || until_daa)`. The four preimage lengths are 33, 73, 53 and 93 bytes, so no two can collide, the same argument P5.9 made for the lock. The pool state map's value becomes `(asset, d, pk, lock?)`; the store adds the asset as a field that is absent for MAGLD, the way the lock is absent for an unlocked note, so no entry written so far needs rewriting and no migration runs at activation.

### The wire

Pool protocol version 2. `NewNote` gains the asset, encoded compactly so that MAGLD, which is nearly every note, pays one byte:

```rust
struct NewNote {
    asset: Option<AssetId>,  // borsh: 0x00 for MAGLD (1 byte), 0x01 || 20 bytes otherwise
    d:     DenominationTag,
    pk:    [u8; 32],
}                            // 34 bytes for MAGLD, 54 for any other asset
```

`Some` of all zeros is invalid, so every asset has exactly one encoding and a payload cannot spell MAGLD two ways.

`Transfer`, `TransferLocked`, `Mint` and `Redeem` keep their shapes and their tags with the wider `NewNote` inside them. Two new variants:

```rust
struct IssueOp {
    asset:     AssetId,               // never all zeros
    produced:  Vec<NewNote>,          // every note carries `asset`
    freshness: FreshnessAnchor,
    issuer:    [u8; 64],              // BIP340 signature by the lane's current key
}                                     // borsh tag 4

struct RetireOp {
    consumed:  Vec<SignedGroup>,      // the notes' own keys, as in Redeem
    freshness: FreshnessAnchor,
    issuer:    [u8; 64],              // the lane's current key, again
}                                     // borsh tag 5
```

A version-1 payload is a version-2 payload whose every note is MAGLD. Before the asset activation only version 1 is valid; from it only version 2 is. The node keeps decoding version 1 for as long as archival nodes serve blocks that carry it, but accepts none in a block past the activation. Mainnet activates at genesis, so mainnet never sees version 1 at all. A wallet that has not upgraded by the testnet activation stops paying, which is the same thing that happened at the lock activation and is acceptable on a testnet.

### Signing

`pool_protocol_version` in the signing preimage becomes 2, and `op.produced` is serialized per note as its wire encoding, `asset-option || d || pk`, 34 or 54 bytes. `op_type` takes 4 for issue and 5 for retire. The freshness anchor, the transparent outputs hash and the serials are as they are.

The issuer's signature is its own preimage, domain-separated from the holders':

```
NotePoolIssuerHash = H(
    "NotePoolIssuer"
    || pool_protocol_version              // u8 = 2
    || op_type                             // 4 or 5
    || asset                               // 20 bytes
    || produced (issue) or sorted serials (retire)
    || transparent_outputs_hash
    || freshness.anchor_daa_score
)
```

It binds the operation, the asset, the notes and the enclosing transaction's outputs for the same reasons P5.2 gives, and it expires with the freshness window, so a signed but unbroadcast issue is not a standing liability on the issuer either.

### Conservation, per asset

For every accepted operation and every asset `A` that appears in it:

- `A` is MAGLD: `Σ consumed + Σ ledger in = Σ produced + Σ ledger out + fee`, with `fee ≥ 0`. This is I4, unchanged.
- `A` is anything else: `Σ consumed = Σ produced` in a transfer, `Σ produced = the issue amount` in an issue with nothing consumed, and `Σ consumed = the retire amount` in a retire with nothing produced.

An operation may touch several assets: a transfer of dollar notes attaches a MAGLD fee stamp as the note it consumes and does not produce, and both sums hold. Fees are MAGLD only. Miners receive MAGLD only, the block reward carries MAGLD only, and the whitepaper's line that nothing is burned stays true for MAGLD; an issued asset is created and destroyed by its issuer and by nobody else.

The consequence for a person holding only dollars is that they need a MAGLD stamp to move them, or a payer who supplies one. That is a wallet and issuer matter (an issuer can hand out stamps with the notes it issues), not a consensus one, and it is left for the coin phase.

### The issuer

Consensus keeps the lane registry: a map from tag to `(key, claimed_at)`, applied from the registry lane in chain order exactly as the wallet's listing walk does today, first valid claim per tag wins. One addition: a **re-claim**, a claim for an already-claimed tag signed by the tag's current key, replaces the key. That gives an issuer key rotation, which a company holding other people's money cannot be without, and it costs anchoring lanes nothing. The map is consensus state with its own store and lives beside the pool state; it is small, one entry per claimed tag, and it is not committed in the header for now, because a syncing node can rebuild it from the registry lane's transactions, which the pruning-point block set carries. Whether to commit it is an open question below.

An issue or retire is valid iff the asset's tag is claimed and the issuer signature verifies under the claim's current key at the point-of-view DAA score. An issue for an unclaimed tag is invalid; there is no way to create an asset except by owning its lane. Whether the claim needs the trustee endorsement the registry already has for listed tickers is a policy question for the coin phase.

### Outstanding totals

Consensus keeps, per asset, `issued` and `retired` as two `u64`s in petals-of-the-asset, updated by issue and retire, alongside the existing pool total for MAGLD. `Σ notes of A in the pool = issued(A) − retired(A)` is an invariant every node checks whenever it checks I5, and an RPC returns the pair. This is the number an attestation is written against.

### Activation

Two parameters, both `ForkActivation`s:

- `assets_activation`: the version-2 wire, the wider leaf and the registry as consensus state. Testnet-10: a DAA score chosen when the build is ready, about a day out, the fleet upgraded ahead of it on the usual recipe. Mainnet: always.
- `asset_issue_activation`: issue and retire accepted, non-zero assets allowed in notes. Testnet-10: set only when the founder wants the rehearsal. Mainnet: never, until decided.

Between the two, every note on the chain is MAGLD, an issue or retire is invalid everywhere, and the only visible change is one byte more per produced note on the wire and tags up to sixteen bytes in the registry. That is the price of the foundation.

### Invariants added

- **I6 (asset immutability)**: a live note's `asset` never changes; every operation that consumes notes produces notes of the same asset, and only issue and retire change an asset's total.
- **I7 (MAGLD has no issuer)**: no operation authorized by a lane key can create, destroy or touch a MAGLD note. Whatever an asset's issuer may one day be allowed to do to its own notes, the all-zeros asset has no issuer and no such operation, and this is written down now so that a later issuer power is structurally unable to reach the native coin.
- **I8 (issuer accounting)**: for every asset, the pool's notes of that asset sum to `issued − retired`.

## What changes where

- `consensus/core/src/notepool`: `AssetId`, the field on `NewNote` and `PoolEntry`, `IssueOp`, `RetireOp`, `POOL_PROTOCOL_VERSION = 2`, the two hashers and the per-asset conservation in validation.
- `consensus/src/model/stores`: the asset on the entry store, the outstanding totals store, the lane registry store; the pruning-point pool sync carries the asset as an optional field the way it carries the lock.
- `consensus/core/src/config/params.rs`: the two activations, with the testnet-10 score and the mainnet values above.
- `rpc`: the asset on `RpcNoteEntry`, an `GetAssetSupply` call, and the registry readable over RPC.
- `wallet`: the asset stored with every note in the vault now, zero for all of them, so no vault ever needs migrating; the version-2 payloads at the activation; nothing else until the coin phase.
- `consensus/core/src/subnets.rs` and the transaction validator: the sixteen-byte tag tail under `assets_activation`; `wallet/core/src/account/lane.rs`: the alphabet and length.
- `docs/marigold/POOL-SPEC.md`: this page as P5.10 once decided; `LANE-REGISTRY.md`: sixteen-byte tags with the dot and the hyphen, the re-claim, and the registry as consensus state; the whitepaper: one paragraph in Section 10.

## Verify

- A version-1 payload is refused after `assets_activation` and accepted before; a version-2 payload the other way round; the fleet upgraded on the recipe pays across the activation without a failed payment beyond the wallets that were not upgraded.
- A sixteen-byte tag with a dot, `600519.SS`, is claimable after the activation and refused before; a four- and a five-letter lane keep their ids across it.
- A note minted, rotated, split, merged, locked and redeemed after the activation has asset zero in every RPC view and its leaf hash equals the version-1 leaf, so the commitment of a block containing only MAGLD notes is the same the old code would have computed.
- Before `asset_issue_activation`, an issue op signed by a valid lane key is refused by consensus, not only by the mempool, and a `NewNote` with a non-zero asset is refused in every op.
- On a private testnet with `asset_issue_activation` set: a claim, an issue of ten notes under the claim's key, a rotation of one, a split of one, a locked hand-over of one, a retire of two by the issuer, and at every block `Σ pool(A) = issued − retired`; an issue signed by a key that is not the tag's is refused; a re-claim by the current key changes which key can issue; a transfer that consumes dollar notes and produces MAGLD notes, or mixes assets across consumed and produced, is refused; a dollar transfer without a MAGLD stamp is refused for zero fee exactly as a MAGLD transfer is.
- A node syncing from a pruning point after both activations arrives at the same pool commitment and the same outstanding totals as a node that validated every block.

## Open questions for the coin phase, recorded so they are not decided by accident

- **Freeze.** Regulated issuers are expected to be able to freeze funds on legal order. The founder's position today is that cash cannot be frozen and that this is a discussion for when an issuer exists. Nothing in the foundation decides it: a freeze would be a per-asset operation authorized by the issuer, and I7 keeps it away from MAGLD whatever the answer. If the answer is no, an issuer must be found who accepts that; if yes, it is one more op and one more field, added the way the lock was.
- **Who can retire.** The draft requires both the notes' keys and the issuer's key, so that `issued − retired` only moves when the issuer says dollars left the bank, which makes it the number an attestation can match. The alternative, letting any holder burn their own notes, is simpler and is rejected for now for that reason.
- **Committing the registry.** The lane map is rebuilt from the registry lane at sync rather than committed in the header. If lanes come to matter to consensus in more ways than issuing, it should get a commitment; adding a header field is a hard fork and would be better done before mainnet if it is going to be done at all. Decide before `assets_activation` on mainnet.
- **Listing.** Whether an asset's lane needs the trustee endorsement listed tickers already need, and whether an unendorsed tag can issue at all. This is also where one exchange-suffix convention per market would be chosen, so that a security is not claimable under two spellings.
- **Stamps for asset-only wallets.** Issuer-supplied stamps, wallets keeping a little MAGLD, or a later rule letting a fee be paid in the asset to the miner, which mixes assets into the block reward and is the least attractive.
- **Redemption flow.** A holder rotates notes to the issuer's key and is paid off-chain; the issuer retires. Whether the chain should carry a redemption request, with a receipt like a payment's, is a wallet question.
