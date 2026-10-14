# Exchanges and integrators: the interface, and what differs from Kaspa

Marigold is a fork of rusty-kaspa. Anything that speaks Kaspa's gRPC or wRPC, parses its addresses, or follows its headers mostly works unchanged, and the differences are few and deliberate. This page is the integrator's contract: which calls to use, what counts as final, the address format, and from which tag the interface is frozen. An exchange that converts at a shop's till, the processor model, has its own page, [MERCHANT-PROCESSOR.md](MERCHANT-PROCESSOR.md): the bill as a request code, the payment landing under the processor's key, and what paid and final mean in seconds. The normative text of the pool is [POOL-SPEC.md](POOL-SPEC.md); the reasons behind each choice are in [DECISIONS.md](DECISIONS.md).

## Where the code is, and what is stable

Every release is a tag `vMAJOR.MINOR.BUILD` on `main` of [marigoldcash/marigold](https://github.com/marigoldcash/marigold), with a source release under the same tag and the binaries for every system under the same tag on [marigoldcash/marigold-wallet](https://github.com/marigoldcash/marigold-wallet/releases). There is no separate stable branch: `main` is release-tagged, and an integrator builds against a tag, never against the tip. The node binary is `marigoldd`; the public testnet is testnet-10, seeded by `tn-seed1`–`tn-seed3.marigold.cash`, with a public RPC at `wss://rpc1.marigold.cash` meant as a stopgap, not a dependency: an exchange runs its own node.

## The RPC: rusty-kaspa's, plus these

The gRPC service (`protowire`) and the wRPC server (Borsh and JSON encodings) are rusty-kaspa's, with the same method names and the same semantics for everything on the ledger side: `GetBlock`, `GetBlocks`, `GetVirtualChainFromBlock`, `GetBlockDagInfo`, `GetSink`, `GetSinkBlueScore`, `GetUtxosByAddresses`, `GetBalanceByAddress`, `GetBalancesByAddresses`, `GetMempoolEntriesByAddresses`, `SubmitTransaction`, `GetFeeEstimate`, `GetDaaScoreTimestampEstimate`, `GetCoinSupply`, `GetServerInfo`, `GetSyncStatus`, and the notifications `UtxosChanged`, `VirtualChainChanged`, `BlockAdded`, `VirtualDaaScoreChanged`, `SinkBlueScoreChanged`. An exchange's deposit and withdrawal code written for Kaspa uses exactly these.

Three things differ on calls an integrator already uses:

- **`GetUtxosByAddresses` pages.** The request takes an optional `cursor` (`address`, `outpoint` of the last entry served) and an optional `limit`; the response returns the entries and the `cursor` to continue with, or none at the end. Pages are capped at 100,000 entries; a request without a cursor is served whole up to 500,000 entries and refused above that with a message saying to page (DECISIONS, 2026-09-14). A mining address accumulates many coinbase outputs; page it.
- **Block headers carry one more field,** `pool_commitment`, a 32-byte commitment to the note pool's state. It appears in every header the RPC returns and in the block template; software that round-trips a template must carry it ([MINING-COMPAT.md](MINING-COMPAT.md)).
- **Wire compatibility follows from that field.** The gRPC proto and the Borsh encoding of every message that carries a header differ from upstream's, so an upstream rusty-kaspa client library cannot be pointed at a Marigold node for those messages. Use this repository's client crates (`kaspa-grpc-client`, `kaspa-wrpc-client`), its `protowire` proto, or the JSON encoding of wRPC, whose field names are self-describing.

Calls that exist only here, none of which a ledger-only integration needs:

| Call | What it answers |
|---|---|
| `GetFinalityAnchorStatus` | The latest anchor (block and DAA score), whether anchors are being enforced, whether the latest is stale, the cadence interval, and disqualified trustee indices. The field `expired` is always false; it stayed on the wire when the anchors' sunset was withdrawn (POOL-SPEC v1.3). |
| `GetFinalityAnchor` | The latest anchor with the header of its anchored block, for a wallet that verifies the trustees' signatures itself. |
| `GetPoolStateAtBlock`, `GetPoolDiff` | The note pool as of a chain block, and the per-block pool changes over a range: what a wallet without a node of its own uses ([SYNC-LIGHT.md](SYNC-LIGHT.md)). |
| `GetNotesBySerial`, `GetPoolStats`, `NotifyNotesChanged` | Note lookups by serial, the pool's composition per denomination, and a subscription to note changes, for wallets that hold notes. |
| `GetSeqCommitLaneProof` | The lane sequencing proofs of the point-of-sale lanes ([LANE-REGISTRY.md](LANE-REGISTRY.md)). |
| `GetMinerStatus`, `ControlMiner` | The node's built-in miner; of no use to an integrator. |

**The interface is frozen from tag v2.87.343 until genesis** (DECISIONS, "The RPC interface is frozen", 2026-10-11). Until mainnet's genesis, every call above keeps its name, its request and response fields and their meanings; changes are additive only, a new call or a new optional field, and a call is never removed or renamed. If a breaking change ever became unavoidable it would be a new freeze tag, announced in the release channel and recorded in DECISIONS before it shipped. The consensus parameters are locked separately, in DECISIONS, and the mainnet set freezes at PLAN P9.5.

## Confirmations: what the anchors change

On Kaspa an exchange picks a confirmation depth and lives with probabilistic finality. On Marigold five trustees co-sign a block every 300 DAA scores (about thirty seconds at ten blocks a second), and consensus refuses any chain that conflicts with the latest valid anchor, so an anchored block is final in the strict sense. The anchored block sits about 600 DAA scores, one minute, behind the tip.

The recommended policy, in two lines. **Credit a deposit when its block is at or below the latest anchor** (`GetFinalityAnchorStatus`, `latest_anchor`, or the anchor's header from `GetFinalityAnchor`): about a minute, and not a probability. **If the status reports the anchors stale** (`stale: true`, which the node also logs loudly), fall back to a depth, and the conservative one is the merge depth, 36,000 blocks or about one hour, below which consensus accepts no merging block; the hard reorg limit, the finality depth, is 432,000 blocks or about twelve hours. Anchors stale is the fail-open case: the chain keeps running on plain proof of work until the next anchor arrives, and it has only ever happened on the testnet when the signers were restarted. The anchors are a permanent part of the design (DECISIONS, "The anchors stay"), and an exchange that wants no trustee in its deposit policy uses the depth alone, as on Kaspa.

## Address format

Kaspa's address scheme with Marigold prefixes: `marigold:` on mainnet, `marigoldtest:` on testnet-10 (`marigoldsim:` and `marigolddev:` on the simulation and development networks). The same bech32 variant and checksum as Kaspa's, the same version bytes: 0 for a Schnorr public key (the usual address; the 32-byte key itself is in the address), 1 for an ECDSA public key, 8 for a script hash. Any Kaspa address parser that lets the prefix be configured parses Marigold addresses. The ticker on the testnet is TMAGLD; the smallest unit is the petal, 100,000,000 to a coin, and amounts over the RPC are in petals.

## The ledger side: one address per account, and change returns to it

Anyone who inherits Kaspa tooling expects a wallet to rotate change addresses and will read their absence as a wallet bug. It is not one. A Marigold account has one ledger address, every payment it makes returns its change to that address, and the wallet never derives a second one on its own (DECISIONS, "Single ledger address per account", 2026-09-05).

The reason is where privacy lives. The ledger is the observable perimeter on purpose: it is where exchanges, auditors and the conservation check look, and nothing on it is meant to be hidden. Rotating addresses on a fully public side buys little, because change heuristics relink them anyway, and it costs gap-limit scans, address-index bookkeeping and probabilistic recovery. Privacy is the note's job: value that wants it leaves the ledger and becomes a note in the pool, where a fresh key per payment is the default and the chain records no sender, receiver or account. Restoring an account from its 24 words therefore means deriving one key and asking the node about one address, with nothing to miss.

Two consequences for integrators. A deposit address handed to a customer is stable for the life of the account, so tooling that expects to see a fresh address per deposit should not alarm on reuse. And a Marigold address encodes the public key itself, as Kaspa's does, so the classic objection that reuse reveals the key does not apply; the key was always visible.

Older addresses a wallet derived before this policy, or imported, stay tracked so their funds remain spendable; `address` lists any that still hold coins and `sweep` gathers them.

## Operational notes for an exchange

Everything here follows from the one-address rule, and most of it is simpler than on Kaspa. One point, the last, has to be got right.

**Deposit attribution.** One ledger account per customer, one address per account, attribution by address, as on any UTXO chain. The address is never rotated, so the mapping from address to customer is permanent: there is no address-reuse detection, no gap limit, and no case of a customer paying to an address the wallet stopped watching. A deposit can be credited at the anchor rather than after a count of confirmations: an anchored block is final in the strict sense, and the anchor sits about a minute behind the tip, so "one anchor" is the natural deposit policy, with the finality depth as the fallback whenever `GetFinalityAnchorStatus` reports the anchors stale.

**Dust and spam.** Anyone can send dust to a published address; nothing on any UTXO chain prevents it, and nothing here needs to. Marigold inherits Kaspa's storage mass, which makes creating small outputs expensive for the sender, so spam against a hot address costs the spammer more than the exchange. When the entry count grows, consolidate with an ordinary transaction to the same address, and read a busy address by cursor, as the RPC section above says.

**Sweeping to cold storage.** A ledger transaction from the hot address to the cold one, and nothing else: no note, no pool operation, no special case. Cold storage is a plain address whose key is kept offline, and the partially signed transaction flows inherited from Kaspa (`pskb`, `sign`) cover signing there.

**Accounts, and what the 24 words bring back.** Per-customer addresses means per-customer accounts, and the one-address design's recovery promise, derive one key and ask about one address, holds per account. An exchange with many customer accounts has moved the discovery question from the address index to the account index, and must treat it as such. The facts, from the wallet as it stands: every ledger account is derived from the wallet's 24 words, with account indices 0, 1, 2 … in the order the accounts were created, so the same words recreate the same addresses; the wallet's own files remember which indices exist, and a restore from the words alone brings back the first account, after which `account create bip32` recreates the rest in order, each with the address it had. The wallet library carries an account scan (`accounts_discovery`) that the terminal does not yet expose as a command. So an exchange keeps two things beside the words: a backup of the wallet files, and its own ledger of account index to customer. With those, nothing is probabilistic; without them, the first restore from words alone looks like missing funds until the accounts are recreated in order.

## Mining software

Marigold's block header carries one field Kaspa's does not, `pool_commitment`, a commitment to the note pool's state. Software that round-trips a full block template must know the field; ASICs and stratum miners behind a bridge are unaffected. [MINING-COMPAT.md](MINING-COMPAT.md) has the details and the list of what is affected.

## The note pool, for those who touch it

Exchanges and auditors never have to: deposits and withdrawals are ordinary ledger transactions, and `exchange <address> <amount>` in the wallet pays to a plain address. The pool is a consensus-maintained map of live notes, each a denomination, a current public key and a serial; the operations on it are mint, transfer, redeem, locked transfer and the fee stamp, all plaintext, all conservation-checked by every node. A wallet builder who wants to hold notes finds the data model in POOL-SPEC P5.1 and the signing rules in P5.2, and the node's RPC for a wallet without a node of its own, the pool state at a block and the per-block diffs, in [SYNC-LIGHT.md](SYNC-LIGHT.md).
