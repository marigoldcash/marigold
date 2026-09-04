# Mining-software compatibility: the `pool_commitment` header field

Marigold adds one field to the block header that vanilla Kaspa does not have: `pool_commitment`, a 32-byte commitment to the note-pool state, the pool-side twin of Kaspa's own `utxo_commitment` (design: [POOL-SPEC.md](POOL-SPEC.md) P5.1; decision record: the "Pool state commitment" row in [DECISIONS.md](DECISIONS.md)). It is protobuf field **16** on `RpcBlockHeader` ([rpc/grpc/core/proto/rpc.proto](../../rpc/grpc/core/proto/rpc.proto)), and it is part of the **proof-of-work pre-image** ([consensus/core/src/hashing/header.rs](../../consensus/core/src/hashing/header.rs), between `utxo_commitment` and `timestamp`).

The consequence, stated plainly: **mining software built for vanilla Kaspa cannot mine Marigold unmodified.** This is deliberate and permanent — the recorded decision (see the "pool_commitment stays in the PoW pre-image" row in [DECISIONS.md](DECISIONS.md)) is that consensus does not bend for tool compatibility; tools carry a small patch instead. This page is that patch.

## Who is affected

Affected — anything that **round-trips a full block template over gRPC/wRPC**: solo miners with built-in node connectivity (e.g. `kaspa-miner`), external stratum bridges (the Go `onemorebsmith`/`rdugan` family), pool software with its own Kaspa gRPC module (e.g. Miningcore), and anything else that deserializes a template into its own header struct and submits it back.

Not affected: ASICs and stratum miners behind a bridge (they hash an opaque pre-computed job and never see header fields — any pool-commitment-aware bridge shields them completely); this repo's own in-repo `stratum-bridge` (first-party, always schema-current); and anything built on this repo's own WASM SDK or Rust crates.

## The symptoms

Both failure modes were hit for real on 2026-08-20 and are why this page exists:

1. **Submission rejected with `missing pool_commitment header field ...`** (older builds: `Hex parsing error: Invalid input length 64`). Your tool's protobuf schema predates the field, so proto3 silently dropped it during the template round-trip and submitted it back empty. Apply patch 1.
2. **Submission rejected with `block has invalid proof-of-work`, even though your tool's own target check passed.** Your wire format is fine but your hasher builds the PoW pre-image without the field, so you are literally solving a different puzzle than the node verifies. Apply patch 2. (Patch 1 alone is not enough — this is the deeper of the two.)

## The two patches

Reference implementation: [marigoldcash/marigold-miner](https://github.com/marigoldcash/marigold-miner), our fork of `elichai/kaspa-miner` — the `marigold` branch carries exactly these two commits and nothing else, each with a full explanation in its commit message. Total change: ~12 lines.

**Patch 1 — wire format.** Add the field to your `RpcBlockHeader` protobuf message, field number 16:

```proto
string poolCommitment = 16;
```

If your tool clones the received template and resubmits it wholesale (most do — `kaspa-miner` does), this alone fixes the wire side: your protobuf library carries the value through untouched. If your tool rebuilds the header field by field for submission, also copy the value across explicitly.

**Patch 2 — PoW pre-image.** In your header-serialization/hashing routine, hash the 32 `pool_commitment` bytes into the pre-image **between `utxo_commitment` and `timestamp`** — the exact position [consensus/core/src/hashing/header.rs](../../consensus/core/src/hashing/header.rs) uses. In `kaspa-miner`'s `src/pow.rs::serialize_header`, that looks like:

```rust
decode_to_slice(&header.pool_commitment, &mut hash).unwrap();
hasher.update(hash);
```

placed directly after the `utxo_commitment` update and before the `timestamp` update. Everything else about the algorithm (kHeavyHash itself, the matrix generation from the pre-pow hash, the target check) is unchanged from Kaspa.

## Verifying your port

Point your patched tool at a local isolated node (`marigoldd --testnet --enable-unsynced-mining ...`, or the [docker-compose harness](../../docker/README.md), which runs exactly this scenario) and confirm you see the node log `Accepted N blocks ... via submit block` — not just your own tool's "found a block" message, which only proves you solved *your* puzzle, not the network's.
