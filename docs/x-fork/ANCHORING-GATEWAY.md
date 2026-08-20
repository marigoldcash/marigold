# Marigold anchoring gateway — API contract (DRAFT v0)

Status: **draft for integrator review** (2026-08-16; rev 2 same day — namespace pinned and token-rotation semantics resolved per integrator feedback). The gateway itself ships with the P9-era launch tooling; this contract is published early so integrators on constrained runtimes (Cloudflare Workers etc.) can build against it now. Field names and semantics here are intended to be stable; anything marked OPEN is explicitly up for discussion.

## What this is

A minimal HTTPS service colocated with a Marigold node. It exists so that an integrator can anchor a 32-byte digest on chain with a single `fetch()` call — without holding MAGLD, without building or signing transactions, and without speaking gRPC/wRPC. The gateway holds a funded fee key (never the integrator's data or keys) and runs behind ordinary TLS on 443, so it works from any runtime that can make an HTTPS request.

The intended usage pattern (e.g. monthly document batches): the integrator builds a Merkle tree over their document hashes, submits only the **root**, and hands each customer their document plus a Merkle inclusion proof. Per-document verification never involves the gateway operator's trust: any archival Marigold node can confirm the root's on-chain presence and timestamp.

## Authentication

- `POST` endpoints: `Authorization: Bearer <token>` — per-integrator tokens issued out of band. 401 without.
- **Zero-downtime rotation**: the gateway holds a *set* of active tokens per integrator (not a single value). Rotation is: (1) a new token is added alongside the old, (2) the integrator switches on their own schedule — both tokens are valid for the whole overlap window, which has no built-in expiry, (3) the old token is revoked once the integrator confirms the switch. No coordinated cutover, ever; a token swap is never a maintenance event on the integrator's side. All active tokens map to the same integrator identity for rate limiting and `label` bookkeeping.
- `GET` endpoints: public, no auth (they serve independent verification).
- All endpoints: HTTPS only, standard ports (Workers-compatible; no custom ports).

## Endpoints

### `POST /v0/anchors`

Anchor a 32-byte digest.

Request body (JSON):

```json
{
  "root": "9f3c…64 hex chars…a1",     // required: 32-byte digest, lowercase hex
  "label": "acme-t&a-2026-08"          // optional: opaque client string, ≤ 64 bytes,
                                       // stored by the gateway for the client's own
                                       // bookkeeping; NOT written on chain
}
```

Response `202 Accepted`:

```json
{
  "root": "9f3c…a1",
  "txid": "77b0…64 hex…c2",            // the anchoring transaction's id
  "status": "submitted"
}
```

Semantics:

- **Idempotent by root**: re-submitting an already-anchored (or in-flight) root returns the existing `txid` with `200 OK` instead of creating a second transaction.
- The gateway builds, signs (its own funded fee key), and submits the transaction; fees are the gateway operator's concern, invisible to the caller.
- Errors: `400` (malformed root — must be exactly 64 hex chars), `401`, `429` (rate limit), `503` (node unreachable / not synced; retry with backoff).

### `GET /v0/anchors/{root}`

Public status/verification lookup by digest.

Response `200 OK`:

```json
{
  "root": "9f3c…a1",
  "txid": "77b0…c2",
  "status": "pending" | "confirmed" | "final",
  "block_hash": "51ee…9d",             // accepting chain block (absent while pending)
  "block_daa_score": 1234567,
  "block_timestamp_millis": 1786700000000,
  "confirmations": 412
}
```

- `pending`: submitted, not yet accepted by the selected chain.
- `confirmed`: accepted by a chain block (the timestamp above is the attestation time a verifier should display).
- `final`: additionally past the chain's finality threshold — for practical purposes irreversible. (Marigold's launch-era trustee finality anchors typically make this minutes, not hours.)
- `404` for unknown roots.

### `GET /v0/health`

Public. `{ "node_synced": bool, "network": "marigold-mainnet", "virtual_daa_score": … }`. Gate submissions on `node_synced` if you want conservative behavior.

## The on-chain encoding (what independent verifiers rely on)

This is the part that must hold regardless of the gateway's existence — a verifier must be able to check an anchor against any archival node with no gateway involved:

- The anchoring transaction lives in a **dedicated user-lane subnetwork**: a 20-byte subnetwork id of the form `[4-byte namespace, 16 zero bytes]`. The namespace for this integration is **pinned: `0x54 0x33 0x36 0x30` (ASCII `"T360"`)** — integrator's choice, 2026-08-16. Filtering a block's transactions by this subnetwork id finds all of this integration's anchors.
- The transaction payload is exactly **33 bytes**: `0x01` (payload version) followed by the 32-byte root, big-endian as submitted.
- Independent verification of a document, end to end:
  1. Hash the document (the integrator's declared hash function).
  2. Walk the supplied Merkle inclusion proof to a root.
  3. Fetch the transaction by `txid` from any archival Marigold node (or block explorer) and check its payload carries that root and its subnetwork id matches.
  4. Read the accepting block's timestamp — that is the attestation time.
- **Archival note**: Marigold, like Kaspa, prunes old transaction data on ordinary nodes. Long-term verification therefore relies on archival nodes (the partner integration includes a commitment to run one; marigold.cash will run one as well). Verifiers should be pointed at an archival endpoint or an explorer backed by one.

## Non-goals

- The gateway never sees documents, only digests. It stores `(root, label, txid)` for idempotency and lookup — nothing else.
- No batching inside the gateway: one `POST` = one root = one transaction. Batching (Merkle-tree construction) is the integrator's side, by design — it keeps the gateway trivial and the proof format under the integrator's control.
- Not a general-purpose RPC proxy. Anything beyond anchor/lookup uses the node's own RPC surfaces.

## Resolved (formerly OPEN)

1. **Subnetwork namespace: `"T360"`** (`0x54 0x33 0x36 0x30`) — pinned above.
2. **Token rotation: zero-downtime by design** — active-token *sets* with an unbounded overlap window, specified under Authentication above. (Issuance channel — how a new token is delivered out of band — remains an ops detail, deliberately outside this contract.)

## OPEN items for v1

3. Rate limits (proposal: 60 submissions/day per integrator — monthly batching needs 1).
4. Whether `GET /v0/anchors/{root}` should also return a merkle-independent chain-inclusion proof blob (KIP-21 lane proofs exist on the node; probably overkill for v1).
