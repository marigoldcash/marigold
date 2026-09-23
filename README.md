<h1>Marigold</h1>

**Marigold (MAGLD)** is a transparent, fixed-denomination bearer-note coin — digital
cash with a fully auditable chain — built as a fork of
[rusty-kaspa](https://github.com/kaspanet/rusty-kaspa), the Rust implementation of the
Kaspa full node.

Marigold keeps everything that makes Kaspa fast and robust — GHOSTDAG parallel-block
proof-of-work consensus at 10 blocks per second, kHeavyHash mining, pruning, and the
full wallet/RPC/P2P stack — and adds a **note pool**: an on-chain registry of
fixed-denomination bearer notes that can be minted from and redeemed to **the ledger**
(Marigold's ordinary account side, holding arbitrary amounts — the transparent tier in
upstream Kaspa terms), split and merged along a ×10 denomination ladder, and
transferred by handing over a private key.

> ⚠️ **Status: pre-mainnet.** Marigold's mainnet has not launched. There is no MAGLD
> token to buy anywhere — anything sold under that name today is a scam. The current
> phase is the public testnet soak; mainnet follows a full external security audit.
> Launch is fair from zero: no premine, no dev fund, no airdrop.

## Nothing is hidden

Marigold is *not* a privacy coin, and it is not marketed as one. Every claim below is
mechanically verifiable from the chain itself:

1. **Every unit of supply is publicly accountable at every block.** Consensus enforces
   `Σ pool notes + ledger supply == emitted supply`.
2. **Nothing on the chain is encrypted or obfuscated.** The note pool is plaintext —
   every note's serial number, denomination, and current public key are visible to
   everyone. There are no ring signatures, stealth addresses, confidential amounts, or
   mixers; the only cryptography on chain is ordinary signatures authorizing spends.
3. **Exchanges and auditors only ever touch the ledger**, and every entry to and exit
   from the note pool is a visible, value-conserving public event.

What the pool *does* provide is what physical cash provides: notes carry no owner
identity, so they are fungible. The chain is complete about value by construction —
it simply never records who holds a note, the same way a banknote doesn't.

## How the note pool works

- Notes exist in fixed denominations: **0.01, 0.1, 1, 10, 100, 1,000, 10,000, and
  100,000 MAGLD** (8 tiers).
- Each note is a `(serial, denomination, public key)` triple in the pool. Whoever
  holds the matching private key owns the note.
- **Spending** a note means giving its private key to the recipient (QR code or any
  other channel). The recipient immediately *rotates* the note — signs a request with
  the old key to install a fresh public key only they control.
- Notes **split** into 10 notes of a tenth the value and **merge** 10-for-1 back up
  the ladder, so any amount can be assembled.
- Pool operations pay fees with **fee stamps** — whole small-denomination notes
  consumed in the operation, credited to the including block's miner. Wallets
  therefore hold nothing but note keys: no addresses, no balances, no seed phrase.

The full design, with every decision and its rationale, lives in
[PLAN.md](PLAN.md) and [docs/marigold/](docs/marigold/) — start with
[POOL-SPEC.md](docs/marigold/POOL-SPEC.md) (the pool specification),
[DECISIONS.md](docs/marigold/DECISIONS.md) (locked parameters and why), and
[WALLET.md](docs/marigold/WALLET.md) (wallet usage).

## Network parameters

| Parameter | Value |
| --- | --- |
| Ticker | MAGLD |
| Supply cap | 210,000,000 MAGLD (hard cap, no tail emission) |
| Base unit | petal (1 MAGLD = 10⁸ petals) |
| Emission | Smooth geometric decay, halving every 3 years from genesis |
| Consensus | GHOSTDAG PoW, 10 blocks/sec (unchanged from Kaspa Crescendo) |
| PoW function | kHeavyHash (Kaspa ASICs mine Marigold natively) |
| Address prefixes | `marigold` (mainnet), `marigoldtest` (testnet) |
| P2P ports | 26111 mainnet / 26211 testnet |
| gRPC ports | 26110 mainnet / 26210 testnet |
| wRPC (borsh / JSON) | 27110/28110 mainnet, 27210/28210 testnet |

Ports deliberately differ from Kaspa's so a Kaspa node and a Marigold node can share a
host.

## Relationship to Kaspa

This repository tracks upstream
[kaspanet/rusty-kaspa](https://github.com/kaspanet/rusty-kaspa) and intends to keep
merging its fixes for years. For that reason the internal crate and module names remain
`kaspa-*` — only user-facing identity (the `marigoldd` binary name, network name,
ticker, prefixes, ports, genesis, emission) is changed. This is a deliberate,
permanent choice, not an unfinished rebrand. Development happens on the default
branch `main`; `master` is a pristine mirror of upstream rusty-kaspa, kept as the
merge base. Marigold is not affiliated with or
endorsed by the Kaspa project; we are grateful to its developers, whose work this chain
builds on.

## Installation

<details>
<summary>Building on Linux</summary>

1. Install general prerequisites

    ```bash
    sudo apt install curl git build-essential libssl-dev pkg-config
    ```

2. Install Protobuf (required for gRPC)

    ```bash
    sudo apt install protobuf-compiler libprotobuf-dev
    ```

3. Install the clang toolchain (required for RocksDB and WASM secp256k1 builds)

    ```bash
    sudo apt-get install clang-format clang-tidy \
    clang-tools clang clangd libc++-dev \
    libc++1 libc++abi-dev libc++abi1 \
    libclang-dev libclang1 liblldb-dev \
    libllvm-ocaml-dev libomp-dev libomp5 \
    lld lldb llvm-dev llvm-runtime \
    llvm python3-clang
    ```

4. Install the [Rust toolchain](https://rustup.rs/) (Rust ≥ 1.91); if already
   installed, `rustup update`

5. (Only for WASM builds) `cargo install wasm-pack` and
   `rustup target add wasm32-unknown-unknown`

6. Clone the repo

    ```bash
    git clone https://github.com/marigoldcash/marigold
    cd marigold
    ```

7. Build the node

    ```bash
    cargo build --release --bin marigoldd
    ```

</details>

<details>
<summary>Building on Windows or macOS</summary>

The toolchain requirements (protoc, LLVM/clang, Rust) are identical to upstream
rusty-kaspa, including the Windows LLVM `AR.exe` copy trick and the macOS
Homebrew-LLVM setup for WASM targets. Follow the platform sections of the
[upstream README](https://github.com/kaspanet/rusty-kaspa#installation), cloning this
repository instead, then `cargo build --release --bin marigoldd`.

</details>

## Running a node

Mainnet has not launched; the network to join today is the **testnet**:

```bash
cargo run --release --bin marigoldd -- --testnet --utxoindex
```

`--utxoindex` is needed if you plan to use a wallet against the node. RPC listens on
loopback only unless explicitly configured — see
[docs/marigold/TESTNET.md](docs/marigold/TESTNET.md) for the full deployment runbook
(including ready-made [Ansible plays](deploy/ansible/) that install the node and the
stratum bridge as systemd services), and pass `--help` for all options. A configuration
file is supported via `-C /path/to/configfile.toml`.

For local experimentation, a private devnet works exactly as in upstream:

```bash
cargo run --release --bin marigoldd -- --devnet --enable-unsynced-mining --rpclisten-borsh=127.0.0.1 --utxoindex
```

## Mining

Marigold uses unmodified kHeavyHash, so existing Kaspa miners and ASICs work. The
in-repo [stratum bridge](bridge/docs/README.md) connects stratum miners to a node; note
that Marigold's bridge mirrors the `pool_commitment` header field, so use the bridge
from this repository, not an external one.

## Wallet and CLI

The `marigold-cli` binary provides the interactive wallet, including the Marigold note
commands (mint, redeem, send/receive bearer notes, split/merge, vault backup):

```bash
cargo run --release --bin marigold-cli
```

Usage is documented in [docs/marigold/WALLET.md](docs/marigold/WALLET.md).

There is also a **desktop wallet** in [gui/](gui/): the same wallet with screens instead of commands, built with Tauri over the very code the terminal wallet runs. It is released beside the terminal wallet under the same version.

**Ready-made binaries** for Linux, macOS and Windows are not attached here: they are published under [marigold-wallet](https://github.com/marigoldcash/marigold-wallet/releases/latest), one release per wallet version, named after the version inside it. A tag `v2.R.B` in this repository and the release `v2.R.B` there are the same code; the binaries are built from this repository's source by its own [Wallet binaries](https://github.com/marigoldcash/marigold/actions/workflows/binaries.yaml) workflow on GitHub's runners. Docker users get the same wallet through the marigold-wallet repository's compose file.

## Development process and review

Marigold is developed by a human founder using AI-assisted engineering, with all
consensus-critical work externally reviewed: the pool specification passed review by
external cryptographers (see [docs/marigold/reviews/](docs/marigold/reviews/)), a full
security audit precedes mainnet, and the network soaks on a public testnet for months
before any real value touches it. The development record — including the step-by-step
[PLAN.md](PLAN.md) this project is built from — is published as-is; trust
should rest on tests, review, audit, and soak, not on who typed the code.

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Consensus changes
require a test in the same PR, and rebranding never mixes with consensus code in one
commit.

## Security

Please report vulnerabilities privately to **security@marigold.cash** (see
[marigold.cash/.well-known/security.txt](https://marigold.cash/.well-known/security.txt)).
Do not open public issues for security-sensitive reports.

## Links

- Website: [marigold.cash](https://marigold.cash)
- Plan & roadmap: [PLAN.md](PLAN.md)
- Design docs: [docs/marigold/](docs/marigold/)
- Upstream: [kaspanet/rusty-kaspa](https://github.com/kaspanet/rusty-kaspa)

## License

[ISC](LICENSE), same as upstream rusty-kaspa.
