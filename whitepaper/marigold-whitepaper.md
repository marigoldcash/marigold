# Marigold: Transparent Bearer Notes on a Fully Auditable Chain

**The Marigold Project** · marigold.cash · Draft v0.1, August 2026

> **Status: working draft.** The normative specification of everything described here is [POOL-SPEC.md](../docs/x-fork/POOL-SPEC.md) (v1.1, externally reviewed); where this paper and the specification disagree, the specification is correct. This paper is the readable account: what transparent bearer notes are, why they exist, and what they honestly do and do not provide.

## Abstract

Physical cash has two properties no mainstream digital money reproduces together: it is a bearer instrument — possession is ownership, and handing it over is settlement — and it carries no memory of who held it. Transparent blockchains provide neither: every unit's full custody history is permanently recorded and clusterable. Privacy chains provide the second property by cryptographically hiding recorded data, at the cost of verification machinery few holders can personally check and a regulatory classification that follows the machinery. Marigold takes a third path: instead of hiding the linkage between payer and payee, it **never records one**. Value is carried by fixed-denomination *notes* — plaintext on-chain entries, each a serial number, a denomination, and a public key. Whoever holds the matching private key holds the note; spending is handing over or re-deriving control of keys, and the chain records only that a note was retired and reissued under a fresh key — never who did it, because "who" is not a concept the protocol contains. Every note, every denomination, and every operation is public; supply is accountable at every block by a consensus-enforced conservation rule. Nothing is hidden: the chain is complete about value by construction, like physical cash. Marigold instantiates this design on a fork of Kaspa's rusty-kaspa node — a 10 blocks-per-second proof-of-work DAG whose sub-second confirmation makes handing over a note feel like handing over cash — with a fixed 210,000,000-coin supply and a fair launch from zero.

## 1. Motivation

Cash works because of three properties working together. It is **final**: handing over a banknote settles the payment, with no intermediary able to reverse or withhold it. It is **fungible**: one note of a denomination is exactly as good as another, because nothing about a note tells you where it has been. And it is **verifiable by its holder**: you can check a banknote is real without trusting the person who gave it to you, and without a PhD.

Digital money keeps failing to combine these. On transparent chains such as Bitcoin, settlement and holder-verifiability are excellent, but fungibility is structurally absent: every coin carries its complete custody graph forever, address clustering is an industry, and coins are routinely discriminated by history. Privacy chains such as Monero and Zcash restore fungibility by *hiding recorded data* — ring signatures, stealth addresses, confidential amounts, zero-knowledge proofs. The data model still contains senders, receivers, and amounts; cryptography obscures them. This works, but at two costs that matter for a cash replacement. First, verification is no longer personal: the holder must trust that a sophisticated cryptographic construction, which very few humans can audit end to end, contains no flaw and no trapdoor — and history shows such constructions have contained both. Second, the category itself has become a regulatory magnet: assets classified as "privacy coins" face delistings and, in the EU from 2027, statutory restrictions — a categorization that attaches to the *hiding machinery*, whatever the intent.

The observation Marigold is built on: **fungibility does not require hiding anything, if the protocol never records the linkage in the first place.** A banknote is not private because its serial number is encrypted — the serial is printed right on it. It is private because the note carries no ledger of hands it passed through. That property can be built digitally, in plaintext, with no cryptography beyond ordinary signatures: let the chain track *notes* rather than *accounts or flows*, let ownership be knowledge of a key rather than an identity, and let a spend be a key rotation rather than a sender-to-receiver transfer. Then there is no encrypted sender field to attack or regulate — there is no sender field.

We call the result a **transparent bearer-note chain**. It is not a privacy coin, and this paper will not claim privacy properties the design does not have (Section 7 states exactly what an observer sees). What it claims is narrower and verifiable by anyone: value is fully accounted for at every block, nothing on the chain is encrypted or obfuscated, and the ledger simply does not contain the payer→payee relation — the same way a cash economy's ledger doesn't.

## 2. Transparent bearer notes

Marigold's chain has two kinds of value. The **ledger** is the ordinary account side inherited from its host chain — UTXO entries holding arbitrary amounts, spendable by script signatures, exactly as on Bitcoin or Kaspa. Mining rewards arrive here; exchanges and auditors interact here. Alongside it lives the **note pool**: a consensus-maintained registry of notes.

A note is a triple, entirely public:

```
(serial, denomination, public_key)
```

- The **serial** uniquely identifies the note for its whole life — which, like a UTXO's, runs from the operation that creates it to the operation that consumes it. It is derived, not assigned: `serial = H(creating_txid || output_index)`, a domain-separated hash — the same outpoint pattern the host chain already uses for UTXOs, requiring no counter and no registry of "next serial." Every operation that touches a note retires its serial permanently and issues successors with fresh serials derived from that operation's own transaction; the pool map only ever inserts and removes entries, never mutates one. (This is also why replaying an executed operation is structurally impossible: the serial it would consume no longer exists.)
- The **denomination** is one of eight fixed values: **0.01, 0.1, 1, 10, 100, 1,000, 10,000, 100,000 MAGLD**. Notes have no other amounts, ever. Fixed denominations are what make notes interchangeable — an observer of a 10-note learns only that it is one of all 10-notes, exactly as a €10 bill is one of all €10 bills.
- The **public key** is the note's current lock. Whoever knows the matching private key owns the note — full stop. There is no account above it, no identity behind it, and no registration linking it to any other note.

Five operations, and only five, change the pool:

| Operation | Consumes | Produces | Bridges to ledger? |
| --- | --- | --- | --- |
| **Mint** | ledger funds | new notes | yes (in) |
| **Rotate** (transfer) | a note (authorized by its current key) | a successor note: fresh serial, recipient's key | no |
| **Split** | one note | ten notes of 1/10 the value | no |
| **Merge** | ten equal notes | one note of 10× the value | no |
| **Redeem** | notes | ledger funds | yes (out) |

Spending a note means giving its private key to the recipient — by QR code, by NFC tap, by reading it aloud — after which the recipient immediately **rotates**: signs, with the received key, a request installing a fresh public key only they control. The settlement rule is universal and simple enough to print on a card: *a note is finally yours when a rotation to a key only you know is confirmed on-chain.* Two handover styles are supported as equals: **bearer handover** (the key itself changes hands, then the recipient rotates — passive receipt, the "granny at a market" flow) and **sign-to-fresh-key** (the recipient supplies a fresh public key in a payment request and the payer's wallet signs the rotation directly to it — the private key never travels, the point-of-sale flow). Both reduce to the identical on-chain operation; the choice is wallet-level and invisible to consensus.

Split and merge move along the ×10 ladder, so any amount is payable in a handful of notes, the way any cash amount is payable in a handful of bills — and, as with bills, making change is part of paying.

## 3. Value conservation: the whole system in one equation

Every transaction touching the pool must satisfy a single rule, enforced by consensus:

```
Σ(notes consumed) + Σ(ledger inputs) = Σ(notes produced) + Σ(ledger outputs) + fee
```

Everything else follows from it. A mint consumes ledger funds and produces notes of equal value. A redeem consumes notes and produces ledger funds. A rotate consumes a note and produces its equal-denomination successor. A split's ten outputs sum exactly to its input. And **fees are the gap**: any shortfall of produced value against consumed value is the transaction fee, credited to the including block's miner through the host chain's native value-in-minus-value-out accounting — no new payment machinery at all.

This yields Marigold's characteristically boring fee mechanism, the **fee stamp**: to pay the fee on a pure pool operation (which touches no ledger funds by design), the payer simply attaches one small note to the consumed set and produces nothing for it. The stamp is destroyed; its value becomes the miner's fee. No wallet ever needs to hold ledger funds or an address to use the pool — the wallet holds nothing but note keys. Congestion pricing works natively: attaching more or larger stamps raises the transaction's fee density in the existing mempool ordering, a denomination-quantized fee market. Mint and redeem, which touch the ledger anyway, pay ordinary fees from it with petal-fine granularity and need no stamp.

The conservation rule also powers the system's headline audit property. Every block header carries a **pool commitment** — a 32-byte cryptographic commitment (a sparse-Merkle-tree root) to the entire note pool's state, the pool-side twin of the host chain's UTXO commitment. Any node, at any block, can verify that:

```
Σ(all notes in the pool) + Σ(ledger supply) = total emitted supply
```

Supply is publicly accountable at every block, by consensus — not by trusting an auditor, and not modulo the soundness of a proof system. A divergent node visibly forks off. This is the precise, checkable content behind the phrase *fully auditable chain*.

## 4. Wallets without seed phrases

A Marigold wallet is not an account. It is a **key ring**: software managing a collection of independent note keys, none derived from any master secret — necessarily so, because keys arrive from other people. Receiving a bearer note means receiving a private key that someone else's wallet generated; no seed phrase of yours can ever regenerate it. The wallet's jobs are correspondingly cash-like: receive keys (QR/text), rotate immediately, plan payments (choosing and splitting notes to cover an amount, like making change), and keep keys safe.

Key custody follows a deliberately uniform rule. Notes live in an encrypted **vault** — one encrypted file per note — protected by a single 24-word recovery secret. Recovery always requires **your files and your key, period**: the word list alone recovers nothing (it is an encryption key, not a seed), and the files alone recover nothing. This uniformity is chosen over the familiar "seed phrase recovers everything" model because that model is structurally impossible for bearer instruments — and a recovery story with exceptions is how users lose money. Backups are consequently ordinary file backups (your own cloud storage, a USB stick), plus a printable paper-QR export for cold storage. Restoring a vault automatically rotates every restored note to fresh keys, on the assumption that any backup copy may have been read.

Two further flows round out the cash analogy. A **point-of-sale landing pad** lets a merchant device receive payments to fresh single-use keys and sweep them, without the terminal ever holding long-term value. And because notes are just keys, **gifting, inheritance, and escrow** are physically representable: a printed QR in an envelope *is* the money, with the same properties — and the same responsibilities — as an envelope of cash.

## 5. Consensus integration: native operations, not contracts

The note pool is implemented as a first-class consensus feature — five operation types with fixed wire formats, validated by every node — rather than as a smart contract. This is a deliberate simplicity choice with three consequences. The action space is **enumerable**: everything that can ever happen to a note is one of five operations whose complete validation rules fit in a short specification, which is what made external review of the entire design tractable. There is **no execution environment** to escape, meter, or misprice. And operations commute the way the host DAG needs them to — parallel blocks containing pool operations merge under the same deterministic rules as parallel blocks containing ordinary transactions.

Pool-operation authorization is a BIP340 Schnorr signature by the note's current key over a domain-separated hash that binds the operation, its consumed serials, the enclosing transaction's ledger outputs (preventing redirect malleability), and a **freshness anchor** — a recent DAA score after which the signature expires (about one hour). Expiry bounds the shelf life of any signed-but-unbroadcast operation: a photographed payment QR or a stale invoice stops being a live liability within the session that created it, closing the class of replay problems bearer instruments are historically prone to. Replay of *executed* operations is impossible by construction: every operation permanently retires the serials it consumed (successors carry fresh serials, even when a key is reused), so a replayed authorization refers to notes that no longer exist.

## 6. What the concept needs from a chain — and the chain chosen

Nothing in Sections 2–5 is specific to any particular blockchain. Transparent bearer notes could be grafted onto most UTXO chains: the design needs a proof-of-work or otherwise neutral base layer, a UTXO-style parallel-friendly data model, ordinary signature verification, and room for a small set of new operation types. The value proposition of this system is the note pool — not its host. We state this plainly because it defines what Marigold is *for*: it is an instantiation of a portable idea, chosen and tuned for one job, digital cash.

That job dictates the host-chain requirements, and they are unforgiving in one dimension above all: **confirmation latency is handover latency.** The settlement rule — yours when your rotation confirms — means the time-to-first-confirmation is the time two people stand at a market stall waiting. On a 10-minute-block chain, bearer handover is unusable; on a 15-second chain, awkward; below one second, it feels like handing over a bill.

Marigold is therefore built as a fork of **rusty-kaspa**, the Rust node of the Kaspa network: a GHOSTDAG proof-of-work blockDAG producing **10 blocks per second**, giving sub-second first confirmation and rapid deepening finality, with mature pruning (the pool's state-commitment design means pruned nodes retain full verification power over current state), a production-quality P2P/RPC stack, and an actively maintained upstream whose fixes Marigold deliberately keeps mergeable. We credit the Kaspa project plainly: Marigold's base layer is their engineering, kept intact precisely because it is excellent. Marigold changes network identity and economics, adds the note pool, and changes nothing else — the fork discipline is "smallest possible diff against upstream," both for security and for honesty about where the base layer's credit belongs.

## 7. What an observer sees — the honest section

This section makes the claims exact, because a cash-like system must be honest about its envelope. **Everything on the chain is public and unencrypted — and "public" means two distinct things here, worth separating.** The pool's *current state* — every live serial, its denomination, its current public key, the pool's total composition per denomination — is plaintext held by every full node and queryable at any moment. A note's *operational history* — when it was minted, every rotation, split, and merge, when it was redeemed — is public in a different sense: each operation was broadcast to the entire network in the block that carried it, and anyone was free to record it, but ordinary nodes prune old block data and retain none of it, so reconstructing a note's past requires an observer who kept the history — an archival node. Pruning is a storage optimization, not an eraser: it changes *who holds* the history, never whether it was disclosed — and the only safe assumption for anyone relying on this system is that **at least one archival observer always exists.** Amounts are never confidential in either sense, and mint and redeem visibly connect ledger funds to specific serials at entry and exit.

What the chain does **not** contain is any identity attached to any of it. A rotation is a signature by a key over a new key; the protocol has no sender, no receiver, no address book, no account. Unlinkability between the parties of a payment is therefore not an added feature but an absence of recorded data — and the following limits apply, stated without minimization:

- **The operation graph is visible.** Every operation publicly lists the serials it retires and the fresh serials it creates, so a note's custody timeline is a public chain of transactions — rotation replaces the serial, but the rotating transaction itself links predecessor to successor. One precise qualification: operations link *sets* to *sets* — a transaction consuming twenty notes and producing twenty records no pairing of which old note "became" which new one; only a plain one-to-one rotation links exactly. Timing correlations, denomination patterns, and fee-stamp lineage remain analyzable graph structure throughout.
- **The anonymity set of a note is, at best, its denomination cohort** — a 100-note hides among 100-notes, nothing more. Wallet hygiene (rotating on receipt, not reusing keys across notes, sensible stamp selection) preserves this baseline; careless use degrades it.
- **Pool entry and exit are the transparent seams.** Whoever links a ledger address to an identity learns which serials that identity minted or redeemed. Between those seams, the chain records no linkage; at them, it visibly does.

The regulatory posture follows from the mechanism, not from marketing: exchanges and auditors interact exclusively with the transparent ledger; every pool entry and exit is a visible, value-conserving public event; and nothing on the chain is hidden from anyone. Marigold declines the "privacy coin" label not as euphemism but as accuracy — coins in that category cryptographically conceal recorded data, and this system conceals nothing. What it shares with cash is narrower: the ledger is complete about *value* by construction, and silent about *people* by construction.

## 8. Economics

**Supply.** Hard cap of **210,000,000 MAGLD**, no tail emission, no premine, no development fund, no allocation of any kind: every coin enters circulation through mining from block zero (a fair launch, announced publicly in advance with binaries available to all). The base unit is the **petal**, 10⁻⁸ MAGLD.

**Emission.** Smooth geometric decay from genesis: the block reward declines in monthly steps of factor 2^(−1/36) — a halving every three years with no cliff moments — from an initial ≈1.5228 MAGLD per second. Roughly 20.6% of supply is emitted in year one and ~90% by year ten; the per-block reward quantizes to its 1-petal floor around year 72, and the emission table's total is verified against the cap by a permanent consensus test.

**Fees and the long game.** All fees — ledger fees and destroyed fee stamps alike — go to miners, never burned and never to any fund. The security budget is designed around the system's nature as a *circulation* coin: every payment is an on-chain rotation paying a fee, so a successful cash economy is a durable fee base in a way that store-of-value settlement is not. The stated security posture across time: launch guards protect youth (Section 9), emission carries the middle decades, circulation fees carry maturity. That is the bet, made openly.

## 9. Launch security: finality anchors with a sunset

A young proof-of-work chain's hashrate is small, and small hashrate invites deep-reorg attacks. Marigold launches with an explicit, temporary, and fully disclosed guard: **finality anchors**. A set of five publicly identified trustee keys, held by operationally independent parties, periodically co-sign (threshold 3-of-5) an attestation pinning a recent block. Consensus treats a validly anchored block as irreversible: no reorg may cross it. Anchors make deep reorganizations impossible while hashrate grows, at the cost of a disclosed trust assumption — and the design constrains that assumption tightly. Trustees produce no blocks, receive no rewards, and cannot censor, mint, or move anyone's funds; their only power is to *veto reorgs*, and the mechanism **fails open**: if trustees fall silent, the chain continues as ordinary proof of work, degraded not halted. Equivocation by a trustee key (anchoring conflicting branches) is a consensus-visible offense that permanently disqualifies the key.

The guard is built to die. Anchor authority sunsets on a difficulty schedule: once network difficulty demonstrates that attack costs exceed a threshold (a parameter frozen only after quantitative modeling against real network data, and stated in the specification as such), anchors step down from mandatory to advisory to expired. The intended endpoint of governance is the same direction (Section 10): founder-selected trustees are a bootstrap, not an institution.

## 10. Related work and future directions

**Chaumian e-cash** (Chaum, 1982) is the intellectual ancestor: digital bearer tokens with genuine unlinkability. Its structural limit was the mint — a trusted issuer who could inflate invisibly or vanish. Marigold keeps the bearer-instrument model and replaces the mint with a public consensus: issuance is the disclosed emission schedule, and the "mint's books" are the conservation rule every node checks. **Monero and Zcash** solve fungibility by encrypting or obscuring a ledger that still records the payer→payee relation; Marigold removes the relation and encrypts nothing — weaker concealment, radically simpler verification, different regulatory shape. **Transparent chains** offer the same verifiability with no fungibility; Marigold is what a transparent chain looks like when the data model itself is redesigned around value instead of flows. **Physical cash** remains the benchmark: Marigold's design goal throughout is to be describable to a cash user in one sitting, mechanism included.

Future work, deliberately outside launch scope, includes **note-weighted governance**: proposals and votes as native pool operations, where a vote is a signature by a note's current key, weighted by denomination, one vote per serial — sybil resistance from coin weight and double-vote prevention from serial uniqueness, using only machinery already in the system. Its first mandate would be electing trustee successors, so that even the launch guard's remnant passes from founders to holders before it expires entirely.

## 11. Conclusion

Marigold is a small idea taken seriously: that digital cash fails not for lack of cryptographic sophistication but from an excess of it — and that the fungibility cash users actually rely on comes from a ledger that never wrote the compromising fact down, not from one that hides it well. The system presented here is deliberately legible end to end: five operations, one conservation equation, plaintext state, ordinary signatures, a supply anyone can audit at any block, hosted on a base layer fast enough that handing over a note feels like handing over money. Everything it does can be checked by the people it is for. Nothing is hidden — the chain is complete about value by construction, like physical cash.

---

*Specification: [POOL-SPEC.md](../docs/x-fork/POOL-SPEC.md) (v1.1, externally reviewed). Source: [github.com/marigoldcash/marigold-node](https://github.com/marigoldcash/marigold-node). Contact: security@marigold.cash (security), marigold.cash (general).*
