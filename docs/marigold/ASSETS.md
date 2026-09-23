# Assets in the note pool: the foundation for issued notes

**Status: plan, 2026-09-23.** Founder-asked: lay the consensus foundations now, while the testnet is still the place to make consensus changes, so that a fiat-backed coin can be built on Marigold later without a hard fork on a community-run mainnet. The coin itself is far out; MAGLD comes first. What this page fixes is the *shape* the pool takes so that a second asset is a rule change gated by an activation, not a redesign. The stablecoin's own questions, the issuer, the reserve, the bank, and whether an issuer can freeze anything, are named at the end and deliberately left open.

The normative home of the pool is [POOL-SPEC.md](POOL-SPEC.md); once decided, this page becomes its P5.10 the way time-locked notes became P5.9.

## The idea in one paragraph

A note today is a serial, a denomination and a key. A note of an issued asset is a serial, an **asset**, a denomination and a key; a MAGLD note stays exactly what it is, on the wire, in the state and in the leaf. The four operations that exist stay MAGLD's and do not change by a byte. Four operations are added beside them for issued assets: a transfer and a locked transfer that carry the asset once per operation and otherwise do what rotate, split, merge and the locked rotation do; and **issue** and **retire**, which create notes from nothing and consume them to nothing, both authorized by the key of the lane whose tag the asset is. Conservation runs per asset, fees stay in MAGLD, and the pool commitment in every block header covers every note of every asset, so every node checks at every block that the notes of an asset in circulation equal what its issuer has issued minus what it has retired. That is the on-chain half of a reserve's proof; the off-chain half, that a bank holds the same number of dollars, is an attestation the issuer anchors in its own lane.

## What is decided now and what waits

**Now, on the testnet (the foundation):**

- the asset on notes of issued assets, in the leaf, the state, the sync and the RPC, and the four asset operations on the wire, all refused until issuing is switched on; MAGLD notes and MAGLD operations untouched;
- per-asset conservation written into validation;
- per-asset outstanding totals as consensus state, so "how much of this asset exists" is a lookup, not a walk;
- lane tags of up to sixteen bytes, so a ticker with its exchange fits;
- the lane registry's claims as consensus state, tag to key, with rotation of the key by the current key, because an issuer is a lane key and consensus has to be able to look it up;
- an activation for the shape, set on testnet-10 and always-on for mainnet; and a second, separate activation for issuing, never on mainnet until decided.

**Later, when there is an issuer (the coin):**

- the second activation, on a founder's decision, after the questions at the end are answered;
- wallets that hold more than one asset and show which is which;
- the issuer's own tooling: issue against a bank receipt, retire against a payout, anchor the monthly attestation;
- the freeze question.

The line between the two is that everything in the first list is a shape that costs nothing while unused, and everything in the second list is a policy that can wait. Mainnet has not launched, which is the whole reason for doing the shape now.

**What this costs MAGLD: nothing.** Founder, 2026-09-23: MAGLD untouched on the wire. A MAGLD transaction after the activation is byte for byte a MAGLD transaction before it, its leaf is the same hash, its fee is the same penny, and no wallet has to upgrade at the activation to keep paying. Anchoring transactions in a lane are untouched too: an anchor never touches the pool, and the subnetwork id it rides in is twenty bytes today and stays twenty bytes. Only a transaction that moves an issued asset uses the wider shape below, and pays for it.

## P5.10 draft: issued assets

### The asset

```
AssetTag = [u8; 16]     // the tag bytes of the issuing lane's subnetwork id
```

An asset is a claimed lane ([LANE-REGISTRY.md](LANE-REGISTRY.md)), named by its tag: the first sixteen bytes of the lane's twenty-byte subnetwork id, raw ASCII, zero-padded, with the last four bytes of the subnetwork id always zero. Padding a tag to twenty bytes gives the lane; nothing else is encoded, and an explorer shows the tag as it is. The tag is the asset and the lane's key is its issuer; a lane whose tag is never used as an asset is an anchoring lane and nothing more. MAGLD has no tag: it is the native asset, the one the ledger emits, and it never appears in an `AssetTag` field, so a tag of all zeros is invalid wherever one is expected.

**Tags grow to sixteen bytes** under `assets_activation`, from the five of wide lanes, with the alphabet widened from capitals and digits to capitals, digits, the dot and the hyphen. Founder-asked 2026-09-23: a ticker with its exchange must fit, and the Hong Kong and mainland forms do not fit in eight bytes, let alone five. Sixteen holds every form in use: `0700.HK` (7), `09988.HK` (8), `600519.SS` and `000001.SZ` (9), and the ISO 10383 market codes if a registry policy ever prefers them, `700.XHKG` (8), `600519.XSHG` and `000001.XSHE` (11). Consensus already asks only that the bytes after the tag are zero (`check_transaction_subnetwork`), so the change is the tail length under the activation and the wallet's alphabet check; every lane claimed so far is the same twenty bytes under both rules, as it was when four-letter lanes became five. Tags of one byte stay impossible, since `[x, 0×19]` is a reserved system id. The four bytes past sixteen stay zero, which keeps a lane id visibly a lane id and leaves room for one more widening. Which exchange suffix an issuer writes is not a consensus matter: the registry's listing policy (trustee endorsement for listed tickers, DECISIONS 2026-09-22) is where `0700.HK` and `700.HK` are kept from both being claimed; the chain only keeps them from being the same lane.

**Why raw bytes and not a packed alphabet.** The founder asked whether a thirty-seven-symbol alphabet, six bits a symbol, should pack sixteen characters into twelve bytes. Decided against, for now: the asset appears once per operation, not once per note (below), so the saving is four bytes per asset transaction, which never reaches a fee that is charged in whole pennies per five thousand bytes; and it would be a second encoding of an identity the chain already carries raw in every subnetwork id, with its own canonicalization rule and its own bugs. Case is a wallet rule already (tags are uppercased before they are claimed), not a consensus one. The saving that is free is taken: the note carries the sixteen tag bytes, not the twenty of the subnetwork id.

### The note and the leaf

A MAGLD note is unchanged: `(d, pk, sn)`, leaf `H_leaf(d || pk)` unlocked and `H_leaf(d || pk || refund_pk || until_daa)` locked, exactly P5.1 and P5.9, so every commitment stands. A note of an issued asset is `(asset, d, pk, sn)` with the asset in front of the leaf: `H_leaf(asset || d || pk)` and `H_leaf(asset || d || pk || refund_pk || until_daa)`. The four preimage lengths are 33, 73, 49 and 89 bytes, so no two can collide, the argument P5.9 made for the lock. The pool state map's value becomes `(asset?, d, pk, lock?)`; the store adds the asset as a field that is absent for MAGLD, the way the lock is absent for an unlocked note, so no entry written so far is rewritten and no migration runs at activation. The pruning-point pool sync carries the asset as an optional field the way it carries the lock; a peer that omits it for an asset note produces the wrong root and is rejected.

The denomination ladder is the same for every asset: tag `d` is worth 10 to the power of `d` minus 2 units of the asset, so tag 0 is one cent of a dollar-denominated asset exactly as it is 0.01 MAGLD. The table in P5.1 is a table of ladder positions; the unit is the asset's. No per-asset table exists and none is needed.

### The wire

The pool protocol version stays 1. `Mint`, `Transfer`, `Redeem` and `TransferLocked` (tags 0 to 3) are unchanged and remain MAGLD's: every note they produce is MAGLD, and a consumed serial that names an asset note makes them invalid. Four variants are added, each carrying its asset once, at the operation, because an operation moves notes of one asset:

```rust
struct TransferAssetOp {
    asset:     AssetTag,               // 16 bytes, never all zeros
    consumed:  Vec<SignedGroup>,       // notes of `asset`, plus MAGLD notes as stamps
    produced:  Vec<NewNote>,           // 33 bytes each, all of `asset`
    freshness: FreshnessAnchor,
}                                      // borsh tag 4

struct TransferAssetLockedOp {
    asset:     AssetTag,
    consumed:  Vec<SignedGroup>,
    produced:  Vec<NewNote>,
    locks:     Vec<ProducedLock>,      // as P5.9
    freshness: FreshnessAnchor,
}                                      // borsh tag 5

struct IssueOp {
    asset:     AssetTag,
    consumed:  Vec<SignedGroup>,       // MAGLD stamps only, may be empty if transparent inputs pay
    produced:  Vec<NewNote>,           // all of `asset`
    freshness: FreshnessAnchor,
    issuer:    [u8; 64],               // BIP340 signature by the lane's current key
}                                      // borsh tag 6

struct RetireOp {
    asset:     AssetTag,
    consumed:  Vec<SignedGroup>,       // notes of `asset`, plus MAGLD stamps
    freshness: FreshnessAnchor,
    issuer:    [u8; 64],               // the lane's current key, again
}                                      // borsh tag 7
```

`NewNote` stays 33 bytes; the asset is sixteen bytes once per operation. An asset transfer is therefore sixteen bytes longer than the same transfer of MAGLD notes, plus the stamp it consumes. Before `assets_activation` tags 4 to 7 are invalid everywhere; before `asset_issue_activation` tag 6 is invalid, and since no asset note can exist without an issue, so in effect are the others.

### Signing

The holders' signing preimage is P5.2's, with `op_type` 4 to 7 and the asset joined after it: `|| asset (16 bytes)`; for tag 5 the locks join after `produced` as in P5.9. Tags 0 to 3 keep their preimage byte for byte, which is how every existing signature keeps verifying and why no wallet has to change at the activation.

The issuer's signature is its own preimage, domain-separated from the holders':

```
NotePoolIssuerHash = H(
    "NotePoolIssuer"
    || pool_protocol_version              // u8 = 1
    || op_type                             // 6 or 7
    || asset                               // 16 bytes
    || produced (issue) or sorted serials (retire)
    || transparent_outputs_hash
    || freshness.anchor_daa_score
)
```

It binds the operation, the asset, the notes and the enclosing transaction's outputs for the same reasons P5.2 gives, and it expires with the freshness window, so a signed but unbroadcast issue is not a standing liability on the issuer either.

### Conservation, per asset

For every accepted operation:

- MAGLD: `Σ consumed + Σ ledger in = Σ produced + Σ ledger out + fee`, with `fee ≥ 0`. This is I4, unchanged. In an asset operation the MAGLD consumed are stamps and the MAGLD produced are none, so the whole of it is fee.
- the operation's asset: `Σ consumed = Σ produced` in a transfer, `Σ produced = the issue amount` in an issue, `Σ consumed = the retire amount` in a retire.
- a consumed note of any other asset makes the operation invalid.

Fees are MAGLD only. Miners receive MAGLD only, the block reward carries MAGLD only, and the whitepaper's line that nothing is burned stays true for MAGLD; an issued asset is created and destroyed by its issuer and by nobody else.

The consequence for a person holding only dollars is that they need a MAGLD stamp to move them, or a payer who supplies one. That is a wallet and issuer matter (an issuer can hand out stamps with the notes it issues), not a consensus one, and it is left for the coin phase.

### The issuer

Consensus keeps the lane registry: a map from tag to `(key, claimed_at)`, applied from the registry lane in chain order exactly as the wallet's listing walk does today, first valid claim per tag wins. One addition: a **re-claim**, a claim for an already-claimed tag signed by the tag's current key, replaces the key. That gives an issuer key rotation, which a company holding other people's money cannot be without, and it costs anchoring lanes nothing. The map is consensus state with its own store and lives beside the pool state; it is small, one entry per claimed tag, and it is not committed in the header for now, because a syncing node can rebuild it from the registry lane's transactions, which the pruning-point block set carries. Whether to commit it is an open question below.

An issue or retire is valid iff the asset's tag is claimed and the issuer signature verifies under the claim's current key at the point-of-view DAA score. An issue for an unclaimed tag is invalid; there is no way to create an asset except by owning its lane. Whether the claim needs the trustee endorsement the registry already has for listed tickers is a policy question for the coin phase.

### Outstanding totals

Consensus keeps, per asset, `issued` and `retired` as two `u64`s in the asset's smallest unit, updated by issue and retire, alongside the existing pool total for MAGLD. `Σ notes of A in the pool = issued(A) − retired(A)` is an invariant every node checks whenever it checks I5, and an RPC returns the pair. This is the number an attestation is written against.

### Activation

Two parameters, both `ForkActivation`s:

- `assets_activation`: the four operation tags decodable and validated, the asset leaf, the registry as consensus state, sixteen-byte tags. Testnet-10: a DAA score chosen when the build is ready, about a day out, the fleet upgraded ahead of it on the usual recipe. Mainnet: always.
- `asset_issue_activation`: issue accepted. Testnet-10: set only when the founder wants the rehearsal. Mainnet: never, until decided.

Between the two, every note on the chain is MAGLD, an issue is invalid everywhere, and the only visible change is that the registry accepts longer tags. Nothing a MAGLD wallet sends or sees is different, before, between or after.

### Invariants added

- **I6 (asset immutability)**: a live note's asset never changes; every operation that consumes asset notes produces notes of the same asset, and only issue and retire change an asset's total.
- **I7 (MAGLD has no issuer)**: no operation authorized by a lane key can create, destroy or touch a MAGLD note. Whatever an asset's issuer may one day be allowed to do to its own notes, MAGLD has no issuer and no such operation, and this is written down now so that a later issuer power is structurally unable to reach the native coin.
- **I8 (issuer accounting)**: for every asset, the pool's notes of that asset sum to `issued − retired`.

## What changes where

- `consensus/core/src/notepool`: `AssetTag`, the optional asset on `PoolEntry`, the four op variants, the asset leaf, the issuer hasher and the per-asset conservation in validation. `NewNote`, the existing four ops and `POOL_PROTOCOL_VERSION` untouched.
- `consensus/src/model/stores`: the asset on the entry store, the outstanding totals store, the lane registry store; the pruning-point pool sync carries the asset as an optional field the way it carries the lock.
- `consensus/core/src/subnets.rs` and the transaction validator: the sixteen-byte tag tail under `assets_activation`; `wallet/core/src/account/lane.rs`: the alphabet and length.
- `consensus/core/src/config/params.rs`: the two activations, with the testnet-10 score and the mainnet values above.
- `rpc`: the asset on `RpcNoteEntry`, an `GetAssetSupply` call, and the registry readable over RPC.
- `wallet`: the asset stored with every note in the vault now, absent for all of them, so no vault ever needs migrating; nothing else until the coin phase.
- `docs/marigold/POOL-SPEC.md`: this page as P5.10 once decided; `LANE-REGISTRY.md`: sixteen-byte tags with the dot and the hyphen, the re-claim, and the registry as consensus state; the whitepaper: one paragraph in Section 10.

## Verify

- A MAGLD transaction built by the old wallet is accepted after `assets_activation`; its payload, its leaf hashes and the resulting pool commitment are byte for byte what the old node computes; the fleet upgraded on the recipe pays across the activation with no failed payment on any wallet, upgraded or not.
- A sixteen-byte tag with a dot, `600519.SS`, is claimable after the activation and refused before; a four- and a five-letter lane keep their ids across it.
- Before `asset_issue_activation`, an issue signed by a valid lane key is refused by consensus, not only by the mempool, and tags 4, 5 and 7 are refused for want of any asset note to consume.
- On a private testnet with `asset_issue_activation` set: a claim, an issue of ten notes under the claim's key with a MAGLD stamp, an asset transfer of one, a split of one, a locked hand-over of one, a retire of two by the issuer, and at every block `Σ pool(A) = issued − retired`; an issue signed by a key that is not the tag's is refused; a re-claim by the current key changes which key can issue; a MAGLD `Transfer` that consumes an asset note is refused, an asset transfer that consumes a note of a second asset is refused, and an asset transfer without a MAGLD stamp is refused for zero fee exactly as a MAGLD transfer is.
- A node syncing from a pruning point after both activations arrives at the same pool commitment and the same outstanding totals as a node that validated every block.

## Open questions for the coin phase, recorded so they are not decided by accident

- **Freeze.** Regulated issuers are expected to be able to freeze funds on legal order. The founder's position today is that cash cannot be frozen and that this is a discussion for when an issuer exists. Nothing in the foundation decides it: a freeze would be a per-asset operation authorized by the issuer, and I7 keeps it away from MAGLD whatever the answer. If the answer is no, an issuer must be found who accepts that; if yes, it is one more op and one more field, added the way the lock was.
- **Who can retire.** The draft requires both the notes' keys and the issuer's key, so that `issued − retired` only moves when the issuer says dollars left the bank, which makes it the number an attestation can match. The alternative, letting any holder burn their own notes, is simpler and is rejected for now for that reason.
- **Committing the registry.** The lane map is rebuilt from the registry lane at sync rather than committed in the header. If lanes come to matter to consensus in more ways than issuing, it should get a commitment; adding a header field is a hard fork and would be better done before mainnet if it is going to be done at all. Decide before `assets_activation` on mainnet.
- **Listing.** Whether an asset's lane needs the trustee endorsement listed tickers already need, and whether an unendorsed tag can issue at all. This is also where one exchange-suffix convention per market would be chosen, so that a security is not claimable under two spellings.
- **Stamps for asset-only wallets.** Issuer-supplied stamps, wallets keeping a little MAGLD, or a later rule letting a fee be paid in the asset to the miner, which mixes assets into the block reward and is the least attractive.
- **Redemption flow.** A holder rotates notes to the issuer's key and is paid off-chain; the issuer retires. Whether the chain should carry a redemption request, with a receipt like a payment's, is a wallet question.
