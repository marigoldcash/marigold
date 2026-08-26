# Marigold: Transparent Bearer Notes on a Fully Auditable Chain

**The Marigold Project** · marigold.cash · Draft v0.3, August 2026

> **Status: working draft.** The normative specification of everything described here is [POOL-SPEC.md](../docs/x-fork/POOL-SPEC.md) (v1.1, externally reviewed); where this paper and the specification disagree, the specification is correct. This paper is the readable account: what transparent bearer notes are, why they exist, and what they provide — and what they deliberately do not.

## Abstract

Physical cash possesses two properties absent from mainstream digital currencies: (i) bearer-instrument status, where possession constitutes ownership and transfer effects immediate settlement; and (ii) absence of historical provenance, such that no record exists linking a unit to prior holders.

Transparent blockchains reproduce neither property. All custody history is immutably recorded and subject to clustering analysis. Privacy-enhanced chains recover the second property through cryptographic concealment of on-chain data, but incur two consequential tradeoffs. First, the verification infrastructure required to validate the protocol exceeds the practical auditing capacity of most token holders. Second, regulatory classifications respond to the presence of sophisticated cryptographic mechanisms themselves, regardless of functional outcome.

Marigold implements an alternative architecture. Rather than obscuring linkages between participants, the protocol omits such linkages from its data model entirely. Value is represented by fixed-denomination notes—plaintext on-chain records containing a serial number, denomination, and associated public key. Each note functions as a minimal custodial unit: one key secures one discrete value. A payment transfers control of the note itself, not balance between accounts — hence the designation *notes* rather than *wallets*. Consequently, the protocol records only that a note was retired and subsequently reissued under a distinct key. No identifier for "payer" or "payee" exists within the protocol specification.

All note metadata, denominations, and operations remain publicly visible. Total supply is verified at every block via a consensus-enforced conservation invariant. No data is concealed; accountability derives from structural completeness rather than obfuscation.

The implementation runs on a modified Proof-of-Work Directed Acyclic Graph (DAG) substrate derived from Kaspa, achieving approximately 10 blocks per second with sub-second first confirmation and rapidly deepening finality. The token supply caps at 210,000,000 coins, deployed via fair launch from genesis.

## 1. Motivation

Physical cash derives its utility from three conjoint properties. First, it is **final**: transfer of a banknote constitutes settlement, with no intermediary capable of reversal or withholding. Second, it is **fungible**: all notes of a given denomination are equivalent, as no note retains information regarding prior holders. Third, it is **holder-verifiable**: authenticity can be confirmed without counterparty trust and without specialized expertise.

Existing digital currencies fail to satisfy all three properties simultaneously. Transparent blockchains, such as Bitcoin, achieve settlement finality and holder-verifiability but lack fungibility by design. Every unit preserves its complete custody history indefinitely, enabling address clustering as an active analytical practice, and coin histories are routinely used as a basis for discriminatory treatment.

Privacy-enhanced chains, such as Monero and Zcash, recover fungibility through cryptographic concealment of on-chain data — employing ring signatures, stealth addresses, confidential transaction amounts, and zero-knowledge proofs. The underlying data model, however, retains sender, receiver, and amount fields; cryptography renders these fields opaque rather than eliminating them. This approach incurs two material costs relevant to any system intended as a cash replacement.

First, verification ceases to be personally accessible. The holder must trust that the cryptographic construction — auditable end to end by only a small number of individuals — contains no latent vulnerability and no deliberate trapdoor. Historical precedent confirms that such constructions have exhibited both.

Second, the category itself attracts regulatory attention. Assets classified as privacy coins face delisting from exchanges and, under forthcoming European Union regulation effective 2027, statutory restrictions. This classification attaches to the cryptographic concealment mechanisms themselves, irrespective of operator intent.

The architectural premise of Marigold is the following: **fungibility does not require concealment if the protocol does not record participant linkages in the first place**. A banknote is not private because its serial number is encrypted — the serial number is printed in plain view. It is private because the note carries no record of prior custody. This property admits a digital implementation using plaintext records and no cryptography beyond standard digital signatures: the chain tracks discrete notes rather than account balances or transaction flows; ownership is defined as knowledge of a private key rather than association with an identity; and a spend operation constitutes a key rotation rather than a transfer between identified parties. Under this design, no encrypted sender field exists to attack or regulate — no sender field exists at all.

The foundational insight motivating Marigold is as follows. Conventional cryptocurrency architectures employ two object types: wallets, which maintain balances, and transactions, which transfer value between them. The entity designated a "wallet" in cryptocurrency terminology is functionally analogous not to a physical wallet but to a bank account. Marigold inverts this relationship. A Marigold note constitutes, in cryptographic terms, a minimal wallet — a single key securing a single fixed balance. A transaction does not transfer value between accounts; it transfers control of the note itself, analogous to a physical cash payment, by replacing the controlling key. Value does not move; notes change custody. Under this model, a note's private key serves the role that a seed phrase serves in a conventional cryptocurrency wallet, with the critical distinction that it secures exactly one discrete unit of value. Consequently, possession, disclosure, or loss of a key is scoped to a single note — never to an aggregate balance. The remaining system design follows from this principle: because note balances are immutable, notes must be issued in fixed denominations, with split and merge operations replacing arbitrary-value transfers; transactions reduce to key rotations; and the client software installed by the user ceases to function as a cryptocurrency wallet, instead serving as a keychain for held notes (Section 4), more closely resembling a traditional physical wallet.

The resulting architecture is termed a **transparent bearer-note chain**. It is not a privacy coin, and this paper does not claim privacy properties beyond those the design demonstrably provides (Section 7 specifies precisely what an observer can infer). The claims advanced are narrower and universally verifiable: total value is fully accounted for at every block; no on-chain data is encrypted or obfuscated; and the ledger does not contain the payer-to-payee relationship — analogous to the manner in which a cash economy's records do not encode transactional counterparties.

## 2. Transparent Bearer Notes

The Marigold chain maintains two classes of value. The **ledger** constitutes the conventional account layer inherited from the host chain — Unspent Transaction Output (UTXO) entries holding arbitrary amounts, spendable via script signatures, identical in function to those on Bitcoin or Kaspa. Mining rewards accrue to this layer, and exchanges and auditors interact exclusively with it. Parallel to the ledger exists the **note pool**: a consensus-maintained registry of notes constituting the system's primary currency.

Each note is defined as a fully public triple:

```
(serial, denomination, public_key)
```

**Serial**. The serial number uniquely identifies a note for the duration of its lifecycle, which — as with a UTXO — extends from the operation that creates it to the operation that consumes it. Serials are derived rather than assigned: serial = H(creating_txid || output_index), computed as a domain-separated hash. This mirrors the outpoint pattern employed by the host chain for UTXOs, requiring no monotonic counter and no registry of next-available identifiers. Every operation that affects a note permanently retires its serial and issues successor notes with fresh serials derived from the originating transaction of that operation. Consequently, the pool map supports only insertion and removal of entries; no entry is ever mutated in place. This property also ensures that replaying an executed operation is structurally impossible: the serial it would consume no longer exists in the pool.

**Denomination**. Each note carries one of eight fixed denominations: 0.01, 0.1, 1, 10, 100, 1,000, 10,000, or 100,000 MAGLD. No other amounts are representable. Fixed denominations ensure note interchangeability: no attribute of a given 10-MAGLD note distinguishes it from any other 10-MAGLD note, analogous to the equivalence between identical-denomination banknotes. What an observer may infer from a note's operational history is addressed in Section 7.

**Public key**. The public key constitutes the note's current locking condition. Knowledge of the corresponding private key confers ownership of the note — no account structure exists above the note, no identity is associated with it, and no registration mechanism links it to any other note.

Five operations, and only five, modify the note pool:

| Operation | Consumes | Produces | Bridges to ledger? |
| --- | --- | --- | --- |
| **Mint** | ledger funds | new notes | yes (in) |
| **Rotate** (transfer) | a note (authorized by its current key) | a successor note: fresh serial, recipient's key | no |
| **Split** | one note | ten notes of 1/10 the value | no |
| **Merge** | ten equal notes | one note of 10× the value | no |
| **Redeem** | notes | ledger funds | yes (out) |

Spending a note entails transferring control of its associated key. In the simplest case, the payer conveys the private key directly to the recipient — via QR code, near-field communication (NFC), or oral transmission — after which the recipient immediately performs a **rotation**: signing, with the received key, a transaction that installs a fresh public key known only to the recipient. The settlement rule is uniform and succinct: a note is irrevocably owned by a party when a rotation to a key known only to that party is confirmed on-chain.

Two handover protocols are supported with equivalent standing:

**Bearer handover**. The private key itself is transferred to the recipient, who subsequently rotates to a fresh key. This protocol accommodates passive receipt scenarios — instances in which the recipient cannot or does not issue a payment request in advance. During the interval between key disclosure and confirmation of the recipient's rotation, both payer and payee possess knowledge of the key. Whichever party rotates first establishes exclusive control. This interval constitutes the digital analog of the physical moment during which cash is in transit between hands: the exposure is equivalent, and the mitigation is identical — complete the transfer promptly. Sub-second block confirmation renders this practical rather than theoretical. No protocol-imposed timeout can eliminate this window, as the payer's residual capability derives from knowledge of a key, and knowledge is not subject to expiration.

**Sign-to-fresh-key**. The recipient supplies a fresh public key via a payment request, and the payer's client signs a rotation directly to that key. The private key never traverses any channel. This protocol eliminates the shared-knowledge interval entirely and is the natural choice whenever the transacting parties can exchange a payment request. Bearer handover exists for circumstances where such exchange is not feasible.

Both protocols reduce to the identical on-chain operation; the selection between them occurs at the wallet layer and is transparent to consensus. They differ solely in the shared-knowledge property described above.

Split and merge operations traverse the ×10 denomination ladder. Any payable amount is thus representable using a small number of notes, analogous to the manner in which any cash amount is payable with a small number of banknotes. As with physical currency, change-making is an intrinsic component of the payment process.

## 3. Value Conservation: The Whole System in One Equation

Every transaction affecting the note pool must satisfy a single rule, enforced by consensus:

```
Σ(notes consumed) + Σ(ledger inputs) = Σ(notes produced) + Σ(ledger outputs) + fee
```

All subsequent properties derive from this invariant. A mint operation consumes ledger funds and produces notes of equal total value. A redeem operation consumes notes and produces ledger funds of equal total value. A rotate operation consumes a note and produces a successor note of identical denomination. A split operation consumes one note and produces ten notes whose total value equals the input note's denomination. Transaction fees are defined residually: any deficit of produced value relative to consumed value constitutes the fee, credited to the miner of the including block through the host chain's native value-in-minus-value-out accounting. No additional fee-handling mechanism is required.

For pool-only operations — which, by design, involve no ledger funds — fees are remitted via a mechanism termed the fee stamp. The payer includes one small-denomination note among the consumed inputs and produces no corresponding output for it. The stamp's note is thereby removed from the pool; its value, however, is conserved. It transfers to the miner through the block reward disbursement on the ledger, and the conservation equation remains satisfied across the complete event.

No value is ever destroyed in this system. Intensive pool usage does not reduce total supply; rather, value circulates from note holders to miners, who may subsequently mint it back into notes at their discretion. No denomination can become scarce, as notes are not a finite pre-printed stock but are created on demand through mint and split operations. No user wallet is required to hold ledger funds or maintain a ledger address to participate in the pool; the wallet holds only note keys. Congestion pricing operates natively: including a larger or higher-denomination fee stamp increases the transaction's fee density within the existing mempool ordering, forming a denomination-quantized fee market. Mint and redeem operations, which interact with the ledger by definition, remit fees directly from ledger funds at fine granularity and require no fee stamp.

> *To preclude a common misreading: a fee stamp destroys the note, not the value. The note is removed from the pool; its value is credited to the miner within the block reward and may be minted into new notes at any subsequent time.*

The conservation invariant also underpins the system's principal audit property. Every block header contains a **pool commitment**: a 32-byte cryptographic commitment (a sparse Merkle tree root) to the complete state of the note pool, analogous to the host chain's UTXO set commitment. Every validating node verifies, for every block processed, that:

```
Σ(all notes in the pool) + Σ(ledger supply) = total emitted supply
```

Total supply is thus publicly accountable at every block by consensus — not contingent upon trust in a third-party auditor, nor dependent on the soundness of any external proof system. Any node whose state diverges from this invariant forks visibly from the network. This constitutes the precise and verifiable content of the claim that the chain is fully auditable.

## 4. Wallet Architecture: Independent Key Management Without Seed Phrases

A Marigold wallet is not an account. It is a **key ring**: client software managing a collection of independent note keys, none derived from any master secret. This realizes the architectural inversion introduced in Section 1 — the notes constitute the wallets; the application functions solely as a keychain.

Independence is structurally enforced for keys received from external parties. Acquiring a bearer note entails receiving a private key generated by another party's wallet, and no seed phrase maintained by the recipient can reconstruct a key the recipient did not create. Immediate rotation does not eliminate this category. At any given time, a wallet may hold a note received but not yet rotated, a payment accepted offline pending rotation, or a bearer key intentionally retained in its original state — for example, a note held in a gift envelope.

For keys generated internally by the wallet — those produced during rotation and mint operations — independence is a design choice. It would be theoretically possible to derive such keys from a master seed alongside the ledger's keys, consolidating all key material under a single derivation hierarchy. However, this approach compromises the conceptual simplicity that motivates the bearer model. Externally received note keys — the defining case for a bearer instrument — cannot be derived from any seed held by the recipient and would necessarily fall outside the derivation hierarchy, creating a hybrid system in which some notes are seed-derived and others are not. The resulting recovery model would need to accommodate two key paradigms simultaneously, increasing complexity without eliminating the need for the encrypted vault described below. The design instead treats all note keys uniformly as independent secrets, whether internally generated or externally received, and unifies recovery through encryption rather than derivation.

Additionally, the wallet necessarily interacts with the host chain's ledger (Section 2) — for exchange deposits, withdrawals, and mining reward collection — and that interaction requires a conventional seed phrase. The wallet's recovery model must account for both key categories: independent note keys and the ledger seed phrase. The solution is to store all key material — note keys and the ledger seed phrase alike — within an encrypted **vault**, implemented as one encrypted file per note plus an encrypted record of the ledger seed. The vault is protected by a single 24-word recovery secret. The 24-word format is selected for user familiarity; functionally, it serves as an encryption passphrase, not as a BIP-39 derivation seed. It generates no keys.

Because note keys are independent secrets rather than derived artifacts, users may maintain multiple wallets, each with its own recovery secret and optionally its own ledger account. As a consequence of this design, notes can be transferred freely between wallet applications under the same holder's control — exported from one and imported into another — with no on-chain interaction required. This property is a direct consequence of the bearer model: since ownership of a note is defined solely as knowledge of its private key, moving a key between one's own wallet applications changes nothing the protocol observes. (Conveying a key to a *different* party is, by contrast, simply bearer handover, and the settlement rule of Section 2 applies in full.)

Recovery invariably requires two components: the encrypted vault files and the 24-word recovery secret. Neither component alone is sufficient. The word list is an encryption key — it reconstructs nothing in isolation. The vault files are unintelligible without the decryption key. This two-factor requirement is what distinguishes the recovery secret from the seed phrase model rejected above. A conventional seed phrase is the keys: sole possession grants full spending authority over all derived addresses. The recovery secret grants nothing without the vault files, and the vault files grant nothing without the recovery secret. The aggregate vulnerability of either component alone is zero.

This design also eliminates the two-recovery-procedure problem. Whether restoring note keys or the ledger seed, the procedure is identical: restore the vault files, enter the 24-word secret, decrypt. Upon restoration, every note key is by default rotated to a fresh key, predicated on the assumption that any backup copy may have been accessed by an unauthorized party. The ledger seed, being a conventional derivation mechanism, is not rotatable in this sense; its exposure risk is mitigated by the encryption layer and by operational guidance to limit the balance held on the ledger to amounts actively in transit to or from an exchange.

Backups therefore consist of ordinary file copies — hosted on the user's own cloud storage, a USB device, or a printable paper-QR export for cold storage. A backup is a copy of encrypted key material. Any party holding both the vault files and the recovery secret possesses the notes. Recovery is a custody convenience constructed upon this fact; it does not constitute an exception to bearer ownership.

Two additional operational flows extend the physical currency analogy. A **point-of-sale landing pad** — an implemented wallet mode, not a proposal — enables a merchant terminal to receive payments addressed to fresh single-use keys and subsequently sweep them, such that the terminal itself never holds long-term value; the designation reflects precisely this role, keys on which value only ever lands and departs. Further, because notes are represented solely as keys, **gifting**, **inheritance**, and **escrow** are directly representable in physical form: a printed QR code enclosed in an envelope constitutes the monetary value itself, bearing the same properties — and the same custodial responsibilities — as an envelope containing physical banknotes.

## 5. Consensus Integration: Native Operations, Not Contracts

The note pool is implemented as a first-class consensus feature rather than as a smart contract. Five operation types with fixed wire formats are validated by every node. This architectural decision carries three consequences.

First, the action space is enumerable. Every operation that can affect a note is one of five defined types, each with a complete validation specification. The total set of admissible state transitions is finite and fully described, rendering the system's behavior specifiable without recourse to a general-purpose execution model.

Second, no execution environment is instantiated. There is no virtual machine to audit for correctness, no gas metering to calibrate, and no instruction set whose pricing surface may admit exploitation. The absence of a programmable execution layer eliminates an entire category of attack vectors associated with smart-contract platforms.

Third, pool operations are commutative with respect to the host DAG's parallel-block merge rules. Parallel blocks containing pool operations resolve under the same deterministic ordering applied to parallel blocks containing ordinary transactions, requiring no special-case handling within the DAG consensus algorithm.

Pool-operation authorization is defined as follows. The note's current controlling key produces a BIP340 Schnorr signature over a domain-separated hash binding the following elements: the operation type, the consumed serials, the enclosing transaction's ledger outputs (preventing redirect malleability), and a freshness anchor consisting of a recent DAA score. The signature expires approximately one hour after the anchor, establishing a bounded validity interval for any signed but unbroadcast operation. This mechanism addresses the replay vulnerability inherent to bearer instruments: a signed payment authorization — whether encoded in a QR code, transmitted over a communication channel, or retained from a prior transaction attempt — ceases to constitute a valid authorization once its bounded temporal window elapses, without requiring protocol-level revocation.

Replay of executed operations is prevented by construction rather than by temporal expiry. Every operation permanently retires the serials it consumes; successor notes carry fresh serials derived from the originating transaction, even when the controlling key is intentionally reused. A replayed authorization therefore references notes that no longer exist in the pool and is rejected by consensus validation.

## 6. Host-Chain Requirements and Selection

The design described in Sections 2–5 is not specific to any particular blockchain. The transparent bearer-note model is portable to any chain satisfying the following requirements: a proof-of-work or otherwise neutrality-guaranteeing base layer, a UTXO-style data model compatible with parallel block processing, standard signature verification, and extensibility to accommodate a small set of additional operation types. The system's value proposition resides in the note pool, not in the host chain. This portability is stated explicitly to define the scope of Marigold's contribution: it is an instantiation of a portable architectural concept, configured for a single purpose — functioning as digital cash.

That purpose imposes a dominant constraint on host-chain selection: confirmation latency constitutes handover latency. The settlement rule established in Section 2 — a note is irrevocably owned by a party when a rotation to a key known only to that party is confirmed on-chain — means that the time to first confirmation is the duration during which two transacting parties must wait before a payment is considered settled. On a chain with 10-minute block intervals, bearer handover is impractical. At 15-second intervals, it remains cumbersome. Below one second, the latency approaches that of a physical cash transaction.

Marigold is therefore implemented as a fork of the Kaspa network's Rust node implementation (rusty-kaspa). The base layer is a GHOSTDAG proof-of-work blockDAG producing approximately 10 blocks per second, yielding sub-second first-confirmation times and rapid deepening of finality. The implementation provides mature pruning capabilities; the note pool's state-commitment design ensures that pruned nodes retain full verification authority over current state. The codebase includes a production-grade P2P networking and RPC stack and is actively maintained upstream.

The fork is maintained under a deliberate policy: the smallest possible diff against upstream. Network parameters, economic configuration, and the note pool are Marigold-specific additions; all other components are preserved unchanged. This discipline serves two purposes. First, it minimizes the surface area for divergence-related defects. Second, it constitutes an explicit attribution of the base layer's engineering to the Kaspa project. Marigold's contribution is the note pool and its associated economics; the base-layer infrastructure is Kaspa's engineering, retained intact because it meets the system's requirements without modification.

## 7. Observability: Observable Properties and Inference Boundaries

A cash-like system must specify precisely what an observer can and cannot infer from its on-chain data. This section enumerates both without minimization.

All data on the chain is public and unencrypted. Two distinct senses of "public" apply and are separated here for precision.

**Current state**. The pool's current state — every active serial, its denomination, its controlling public key, and the total pool composition by denomination — is stored in plaintext by every full node and queryable at any time.

**Operational history**. A note's operational history — mint timestamp, every rotation, split, merge, and redemption — is public in a consequential but different sense: each operation was broadcast to the entire network within the block that contained it, and any party was free to record it. Ordinary nodes, however, prune historical block data and retain no record of it. Reconstructing a note's past therefore requires an archival observer — a node that retained the history. Archival nodes may withhold historical data but cannot falsify it: per-block cryptographic commitments authenticate any history that is served, and any party may maintain an independent archival copy. Pruning is a storage optimization, not an erasure mechanism; it alters who retains historical data, not whether it was disclosed. The only assumption safe for any party relying on this system is that at least one archival observer persists indefinitely.

**Amounts are not confidential** in either sense. Furthermore, mint and redeem operations visibly associate ledger funds with specific note serials at entry and exit.

What the chain does **not** contain is **any identity attached to any operation**. A rotation consists of a signature by one key over a fresh key; the protocol specification contains no sender field, no receiver field, no address book, and no account model. Unlinkability between payment counterparties is therefore not an engineered feature but a consequence of absent data. The following inference limits apply:

**The operation graph is visible**. Every operation publicly discloses the serials it consumes and the fresh serials it produces. A note's custody timeline is reconstructable as a public chain of transactions: a rotation replaces a serial, but the rotating transaction itself establishes a link between predecessor and successor. One structural qualification applies: operations link sets to sets. A transaction consuming twenty notes and producing twenty successor notes reveals no pairing between individual inputs and outputs; only a one-to-one rotation discloses an exact predecessor-successor correspondence. Timing correlations, denomination distributions, and fee-stamp lineage remain analyzable as graph structure throughout.

**The anonymity set of a note is bounded by its denomination cohort**. A 100-MAGLD note is indistinguishable from other 100-MAGLD notes and from nothing beyond that set. Behavioral patterns constitute the primary inference vector: multiple same-denomination rotations executed in close temporal proximity, a split immediately followed by spending of its constituent notes, or a merchant's periodic accept-then-merge cycle each constitute identifiable activity patterns within the public graph. Operational discipline — rotating on receipt — preserves the baseline anonymity set. Wallet implementations may further attenuate patterns through jittered sweep timing, batched operations, varied payment denominations, and fee-stamp selection without systematic bias; however, these measures constitute mitigation, not cryptographic protection. This paper claims nothing stronger.

**Wallet implementation practice**. Conforming wallet implementations generate a cryptographically independent key for every rotation. Key reuse across distinct notes is protocol-permitted but collapses the anonymity set for all affected notes; wallet software therefore prevents it by default. Fee-stamp denomination selection should likewise avoid systematic bias, which would otherwise introduce traceable patterns into the operation graph. (The specification records these as mitigations, not consensus requirements.)

**Pool entry and exit constitute transparent boundaries**. Any party capable of associating a ledger address with an identity learns which note serials that identity minted or redeemed. Between these boundaries, the chain records no participant linkage. At the boundaries themselves, it visibly does.

The regulatory posture of the system follows from its mechanism, not from characterization. Exchanges and auditors interact exclusively with the transparent ledger layer; every pool entry and exit is a visible, value-conserving public event; no data on the chain is concealed from any party. Marigold does not employ cryptographic concealment of recorded data — the defining characteristic of systems classified as "privacy coins." The distinction is architectural, not euphemistic. What Marigold shares with physical cash is specific and bounded: the chain is complete with respect to value by construction, and silent with respect to people by construction.

## 8. Economics

**Supply**. Total supply is fixed at 210,000,000 MAGLD. No tail emission, premine, development fund, or allocation of any kind is incorporated. All coins enter circulation exclusively through proof-of-work mining from genesis block zero, initiated via a publicly announced fair launch with binaries made available to all prospective participants in advance. The base unit of account is the petal, where 1 MAGLD = 10⁸ petals.

**Emission schedule**. The block reward follows a smooth geometric decay from genesis. Rewards decline in monthly steps of factor 2^(−1/36), producing a halving every three years without discontinuous cliff events. The initial emission rate is approximately 1.5228 MAGLD per second. Approximately 20.6% of total supply is emitted within the first year and approximately 90% within ten years. The per-block reward reaches its 1-petal floor resolution around year 72. The emission table's cumulative total is verified against the supply cap by a permanent consensus-level test, ensuring the schedule can never exceed it; discrete monthly rounding leaves the realized total a negligible remainder — under 0.004 MAGLD — beneath the cap.

**Transaction fees and security budget**. All transaction fees — both ledger-layer fees and consumed fee stamps (Section 3) — are credited to miners. No fees are destroyed or redirected to any fund. The long-term security model is designed around the system's intended function as a medium of circulation. Every payment constitutes an on-chain rotation incurring a transaction fee; a cash economy operating at scale therefore generates a persistent fee revenue base, in contrast to store-of-value settlement patterns where transaction frequency is inherently lower. The security posture across the system's lifetime proceeds in three phases: launch-phase protections secure the network during initial deployment (Section 9), block-subsidy emissions sustain security through intermediate decades, and circulation-derived transaction fees sustain the network at maturity. This constitutes the system's explicit economic assumption, stated without qualification.

## 9. Launch Security: Finality Anchors with Scheduled Sunset

A nascent proof-of-work chain exhibits low aggregate hashrate, rendering it susceptible to deep chain reorganization attacks. Marigold deploys an explicit, temporary, and fully disclosed protective mechanism termed **finality anchors**. A set of five publicly identified trustee keys, each held by an operationally independent party, periodically co-sign an attestation — at a threshold of 3-of-5 — pinning a recent block. Consensus treats any validly anchored block as irreversible: no reorganization may extend past an anchored block.

Anchors preclude deep reorganizations during the period in which network hashrate is insufficient to resist them organically, at the cost of a disclosed trust assumption. This assumption is constrained to a narrow scope. Trustees produce no blocks, receive no block rewards, and cannot censor transactions, mint notes, or transfer any party's funds. Their sole capability is to veto reorganizations. The mechanism is designed to **fail open**: if trustees cease producing attestations, the chain continues to operate as conventional proof-of-work, with security degraded but not halted. Equivocation by a trustee key — signing attestations for conflicting chain branches — constitutes a consensus-visible offense that permanently disqualifies the offending key from further participation.

The guard mechanism is designed for scheduled obsolescence. Anchor authority terminates on a difficulty-based schedule: once network difficulty demonstrates that the cost of a deep-reorganization attack exceeds a defined threshold, anchor attestations transition from mandatory to advisory and subsequently to expired. The threshold parameter is finalized only after quantitative modeling against empirical network data and is published in the protocol specification as such.

Trustee keys are replaceable prior to sunset. The same documented key ceremony used to instantiate a trustee key governs its rotation or replacement, providing a unified procedure for both planned succession and compromise response. The intended trajectory transfers the selection of successor trustees to note-weighted governance (Section 10) before the sunset process completes. Failure of the succession procedure, like trustee silence, degrades the chain to conventional proof-of-work; no operational halt occurs. The founder-selected trustee set constitutes a bootstrap mechanism, not a permanent institutional structure.

## 10. Related Work and Future Directions

**Chaumian e-cash** (Chaum, 1982) is the intellectual antecedent of this work: digital bearer tokens providing genuine transaction unlinkability. Its structural limitation was reliance on a trusted issuing mint — an entity capable of invisible inflation or unilateral cessation. Marigold retains the bearer-instrument model and replaces the trusted issuer with public consensus. Issuance is governed by a disclosed emission schedule, and the conservation invariant enforced by every node substitutes for the mint's internal ledger.

**Monero and Zcash** address fungibility by encrypting or obscuring a ledger that retains the payer-to-payee relationship within its data model. Marigold removes this relationship from the data model entirely and applies no cryptographic concealment. The tradeoff is explicit: weaker concealment properties, substantially simpler verification, and a categorically different regulatory profile.

**Transparent blockchains** offer the same verifiability properties as Marigold but provide no fungibility guarantees. Marigold can be characterized as a transparent chain whose data model has been redesigned around discrete units of value rather than transaction flows.

**Physical cash** remains the design benchmark. The overarching design objective is that the system's mechanism be comprehensible to a cash user in a single explanatory session.

Future work, explicitly excluded from launch scope, includes **note-weighted governance**: proposals and votes implemented as native pool operations, where a vote consists of a signature by a note's current controlling key, weighted by denomination, with one vote per serial number. Sybil resistance derives from coin-weighted voting, and double-vote prevention derives from serial-number uniqueness — both utilizing only mechanisms already present in the system. The first mandate of this governance system would be the election of trustee successors, ensuring that the launch security mechanism's residual authority transfers from founders to note holders before its scheduled expiration.

## 11. Conclusion

Marigold is founded on a focused thesis: digital cash has failed not due to insufficient cryptographic sophistication, but due to its excess. The fungibility that cash users depend upon derives from a ledger that never records the compromising datum — not from one that conceals it effectively.

The system presented here is deliberately comprehensible end to end. It comprises five operations, one conservation equation, plaintext state, standard digital signatures, and a supply audited by every node at every block. It is hosted on a base layer whose confirmation latency approaches that of a physical transaction. Every computational step the system performs is verifiable by the population it serves. No data is concealed: the chain is complete with respect to value by construction, and silent with respect to people by construction — as with physical cash.

---

*Specification: [POOL-SPEC.md](../docs/x-fork/POOL-SPEC.md) (v1.1, externally reviewed). Source: [github.com/marigoldcash/marigold-node](https://github.com/marigoldcash/marigold-node). Contact: security@marigold.cash (security), marigold.cash (general).*
