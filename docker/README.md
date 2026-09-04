# Marigold Docker images

Five images live here: `Dockerfile.marigoldd` (the node), `Dockerfile.stratum-bridge`
(the ASIC-facing stratum bridge), `Dockerfile.kaspa-wallet`, `Dockerfile.rothschild`,
and `Dockerfile.simpa` (dev/test tools inherited from upstream). All follow the same
pattern: a `cargo-chef` multi-stage build for fast rebuilds, an Alpine runtime image,
a non-root user, `tini` as PID 1.

## Local multi-container testing

`docker-compose.yml` in this directory is a **local test harness, not a production
deployment recipe** — see its own header comment for the full rationale. It brings up:

- Two testnet `marigoldd` nodes, peered over P2P (proves cross-container block
  propagation, the same property `docs/marigold/NOTES.md`'s P2.9 smoke test proves on
  bare metal).
- One mainnet `marigoldd` node running alongside them (proves the two networks coexist
  cleanly — mainnet hasn't launched yet, so this node has no real peers, it's purely a
  "does this port/appdir combination collide with anything" check).
- One `stratum-bridge` instance, external mode, pointed at the first testnet node.
- One `kaspa-miner` CPU miner (see below), pointed at the same node.

```bash
cd docker
docker compose up --build
```

Everything published to the host is bound to `127.0.0.1` only — this file is for
testing on your own machine, not for exposing anything to a network. Don't lift the
`ports:` bindings from here into a real deployment; see `docs/marigold/TESTNET.md` for
the actual production security posture (gRPC/wRPC never reachable from the internet,
period), which the Ansible playbook under `deploy/ansible/` enforces for real hosts.

## The CPU miner: two real compatibility bugs, both patched and verified

The compose file builds `kaspa-miner` from **our own fork**,
[marigoldcash/marigold-miner](https://github.com/marigoldcash/marigold-miner) (private
for now), not the stock `kaspanet/cpuminer` image or a plain clone of
[elichai/kaspa-miner](https://github.com/elichai/kaspa-miner). That fork exists because
an unmodified `kaspa-miner` **cannot mine Marigold at all** — not a config issue, a real
protocol incompatibility, found and fixed here on 2026-08-20 by running it against a
live node and reading both codebases side by side rather than assuming compatibility:

1. **Submissions were rejected outright** with `Hex parsing error: Invalid input length
   64`. Cause: this fork added a new block-header field, `pool_commitment` (P6.5, the
   note pool — see `rpc/grpc/core/proto/rpc.proto`, field 16), and
   `elichai/kaspa-miner`'s vendored `proto/rpc.proto` has no such field. `kaspa-miner`
   clones the whole template it receives and resubmits it (not rebuilt field by field —
   see its `src/miner.rs`), so the value would carry through fine *if its struct had a
   slot for it*; since it doesn't, the field serializes empty, and
   `rpc/grpc/core/src/convert/header.rs`'s `RpcHash::from_str(&item.pool_commitment)?`
   hard-fails on that empty string before anything else is even checked.
   **Fix**: add the one missing field to `proto/rpc.proto` on marigold-miner's
   `marigold` branch. Sufficient on its own for the wire format — prost carries it
   through the existing clone-and-resubmit path automatically.
2. **Once (1) was fixed, every submission instead failed with `block has invalid
   proof-of-work`** — a genuinely different, deeper bug. `pool_commitment` isn't only a
   wire-format field, it's also part of the kHeavyHash pre-image
   (`consensus/core/src/hashing/header.rs`, between `utxo_commitment` and `timestamp`).
   `kaspa-miner`'s own `src/pow.rs::serialize_header` never touches it, so it was
   computing proof-of-work over the wrong byte sequence entirely — its own `check_pow()`
   accepted nonces the server correctly rejected, because the two sides were hashing
   different pre-images. **Fix**: insert `pool_commitment` into the hash chain at the
   same position marigold-node's own header-hashing code uses.

Both fixes are single, small, well-commented commits on marigold-miner's `marigold`
branch (`main` continues to track upstream unmodified — same split as this repo's own
`master`/`main`) — confirmed working end to end in this exact compose stack: the
patched miner finds blocks, submits them successfully, `node-testnet-1` accepts them,
`node-testnet-2` receives them over real P2P relay, and — once real blocks were
flowing — `bridge-testnet`'s own `is_synced` gate cleared and its internal CPU miner
started getting its own blocks accepted too.

This isn't just a Docker convenience fix: it means **no unmodified third-party
mining tool can currently mine Marigold**, on the public testnet or anywhere else,
without hitting the exact same wall. Worth keeping in mind for anything that talks
directly to gRPC without going through this project's own (already `pool_commitment`-
aware) bridge — a real ASIC's firmware talking through the bridge is fine; a bare
solo-mining tool talking gRPC directly is not, unless it's this patched fork.

Two other things worth knowing, both confirmed from `kaspa-miner`'s own `src/cli.rs`,
unrelated to the two bugs above:

- `--devfund` is opt-in and off by default (`Option<String>`, `None` unless you pass
  it) — no silent diversion of mined blocks.
- **Stock `kaspa-miner`'s `--port` defaults to real Kaspa's ports** (16110 mainnet /
  16210 testnet) — the marigold-miner fork fixes this to 26110/26210 (its commit
  `0b6d73d`), so only ports of stock builds bite. Independent of that,
  **`-s`/`--kaspad-address` only accepts a bare IP** unless the value already starts
  with `grpc://`, in which case it's used verbatim and `--port` is ignored. The
  compose file uses `-s grpc://node-testnet-1:26210` for exactly this reason — it's
  the only form that resolves Compose's service-name DNS, and it carries the port
  inline since the grpc:// form ignores defaults.

To mine to an address you actually control instead of the compose file's built-in
throwaway one, set `CPU_MINER_ADDRESS` before starting:

```bash
CPU_MINER_ADDRESS=marigoldtest:your_real_address_here GITHUB_TOKEN=$(gh auth token) docker compose up --build
```

`GITHUB_TOKEN` (any PAT with read access to `marigoldcash/marigold-miner`) is required
while that repo is private — see `Dockerfile.cpu-miner`'s header comment for how it's
passed through as a BuildKit secret, never written to an image layer. Drop it once the
fork goes public.

## Other real bugs this exercise found (all fixed, in `docker-compose.yml`)

Three more, unrelated to the CPU miner, all found by actually running the stack rather
than assuming it would work:

- **Non-root containers can't write to a fresh named volume.** Docker creates a named
  volume's mount point root-owned unless the image already has that path chowned
  correctly *before* the volume is ever mounted there. `Dockerfile.marigoldd` now
  pre-creates and chowns `/data` for exactly this reason — worth remembering for any
  future image that mounts a volume and runs as non-root.
- **`marigoldd --addpeer` only accepts a literal IP, not a hostname** — unlike gRPC
  connections (which resolve Compose service names fine), `--addpeer` validates
  strictly as an IP at the CLI level. `node-testnet-2` peers with `node-testnet-1` via
  a static IP on a fixed-subnet `marigold` network (see the `networks:` block) rather
  than the service name.
- **A genuinely isolated node never becomes `is_synced` on its own** — no real peers,
  no DNS seeders reachable from inside Docker, so the sink (genesis) is never "recent"
  and nothing downstream (the bridge, any miner) will proceed by default.
  `node-testnet-1` needs `--enable-unsynced-mining`, and the CPU miner needs its own
  matching `--mine-when-not-synced` — after that, real blocks bootstrap the chain and
  everything downstream unblocks naturally (confirmed: the bridge's own `is_synced`
  wait cleared on its own once real blocks started landing).

## Publishing to a registry (not done yet — open decisions)

The eventual goal is pullable images (`docker pull marigoldcash/marigoldd`, etc.) so
anyone can run a node without building from source. Before that happens, a few things
need deciding — none of them technical blockers, just choices someone needs to make
and record (candidate for a `docs/marigold/DECISIONS.md` row when settled):

- **Registry**: Docker Hub vs. GitHub Container Registry (GHCR) vs. both.
- **Image naming/namespace**: `marigoldcash/marigoldd` seems the obvious pick,
  matching the GitHub org, but worth confirming against whatever's already
  registered/reserved.
- **Tagging scheme**: at minimum a `latest` tracking `main`, plus version tags once
  there's a real release process — needs to land alongside whatever `main` → tagged
  release convention this project ends up using.
- **Multi-arch builds** (`linux/amd64` + `linux/arm64`) — the existing Dockerfiles are
  arch-generic (no arch-specific base images or flags), so this is mostly a CI/build
  pipeline decision (`docker buildx bake`, GitHub Actions matrix, etc.), not a
  Dockerfile rewrite.

None of this blocks local testing via `docker compose up` — it only matters once the
decision is made to actually publish.
