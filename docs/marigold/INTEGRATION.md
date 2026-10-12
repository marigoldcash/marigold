# Integrating Marigold: what an exchange, a wallet builder or a tool inherits from Kaspa, and what differs

Marigold is a fork of rusty-kaspa. Anything that speaks Kaspa's gRPC or wRPC, parses its addresses, or follows its headers mostly works unchanged, and the differences are few and deliberate. This page lists them, so that nobody files a design choice as a bug. The normative text is [POOL-SPEC.md](POOL-SPEC.md); the reasons behind each choice are in [DECISIONS.md](DECISIONS.md).

## The ledger side: one address per account, and change returns to it

Anyone who inherits Kaspa tooling expects a wallet to rotate change addresses and will read their absence as a wallet bug. It is not one. A Marigold account has one ledger address, every payment it makes returns its change to that address, and the wallet never derives a second one on its own (DECISIONS, "Single ledger address per account", 2026-09-05).

The reason is where privacy lives. The ledger is the observable perimeter on purpose: it is where exchanges, auditors and the conservation check look, and nothing on it is meant to be hidden. Rotating addresses on a fully public side buys little, because change heuristics relink them anyway, and it costs gap-limit scans, address-index bookkeeping and probabilistic recovery. Privacy is the note's job: value that wants it leaves the ledger and becomes a note in the pool, where a fresh key per payment is the default and the chain records no sender, receiver or account. Restoring an account from its 24 words therefore means deriving one key and asking the node about one address, with nothing to miss.

Two consequences for integrators. A deposit address handed to a customer is stable for the life of the account, so tooling that expects to see a fresh address per deposit should not alarm on reuse. And a Marigold address encodes the public key itself, as Kaspa's does, so the classic objection that reuse reveals the key does not apply; the key was always visible.

Older addresses a wallet derived before this policy, or imported, stay tracked so their funds remain spendable; `address` lists any that still hold coins and `sweep` gathers them.

## Addresses and networks

The address format is Kaspa's bech32 variant with Marigold prefixes: `marigold:` on mainnet and `marigoldtest:` on the testnet. Testnet-10 is the public test network; its ticker is TMAGLD, its smallest unit the petal, 100,000,000 to a coin.

## UTXO queries page by cursor

`GetUtxosByAddresses` answers in pages of at most 100,000 entries, continued by a cursor of `(address, outpoint)`; a request that does not page is served whole up to 500,000 entries and refused above that, with a message saying to page (DECISIONS, 2026-09-14). A mining address accumulates many coinbase outputs, so integrators who watch such an address should page.

## Mining software

Marigold's block header carries one field Kaspa's does not, `pool_commitment`, a commitment to the note pool's state. Software that round-trips a full block template must know the field; ASICs and stratum miners behind a bridge are unaffected. [MINING-COMPAT.md](MINING-COMPAT.md) has the details and the list of what is affected.

## Finality

Five trustees co-sign a recent block every 300 DAA scores (about thirty seconds), and consensus refuses any chain that conflicts with the latest valid anchor. An anchored block is therefore final in the strict sense, not the probabilistic one; the anchored block sits about 600 DAA scores, one minute, behind the tip. `GetFinalityAnchor` returns the latest anchor with its header; `GetFinalityAnchorStatus` says whether anchors are being enforced and whether the latest one is stale, in which case the chain is on plain proof-of-work finality until the next arrives. The anchors are a permanent part of the design (DECISIONS, "The anchors stay", 2026-10-11), and a service that wants no trustee in the loop runs a full node and uses the finality depth as on Kaspa.

## The note pool, for those who touch it

Exchanges and auditors never have to: deposits and withdrawals are ordinary ledger transactions, and `exchange <address> <amount>` in the wallet pays to a plain address. The pool is a consensus-maintained map of live notes, each a denomination, a current public key and a serial; the operations on it are mint, transfer, redeem, locked transfer and the fee stamp, all plaintext, all conservation-checked by every node. A wallet builder who wants to hold notes finds the data model in POOL-SPEC P5.1 and the signing rules in P5.2, and the node's RPC for a wallet without a node of its own, the pool state at a block and the per-block diffs, in [SYNC-LIGHT.md](SYNC-LIGHT.md).
