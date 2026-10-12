# Scope statement for the independent security audit of Marigold

**Parties.** The audit is commissioned by marigold.cash Ltd (a UK company limited by guarantee with open-source clauses, incorporation applied for in October 2026) and paid for by O'Brien & Partners, Neyruz, Switzerland, the founder's consultancy, on the company's behalf. The auditor must be independent: no ownership, employment or other payment from either party beyond the fee for this audit, and in particular the auditor is not O'Brien & Partners. The report is published in full, under the auditor's name, in `docs/marigold/reviews/` of the public repository, beside the project's own [self-assessment](self-assessment-2026-10.md), which is disclosed as the founder's.

**Object.** The Marigold source at a named tag of [marigoldcash/marigold](https://github.com/marigoldcash/marigold), `v2.87.343` or the tag current at signing, as a fork of rusty-kaspa version 2.0.1 (upstream commit `a41a333b`). The audit is of the difference from upstream, `git diff master..<tag>`, not of rusty-kaspa itself.

## In scope

Each item names the code and the question the auditor is asked to answer.

1. **The note pool's consensus rules** (`consensus/src/pipeline/virtual_processor/notepool.rs`, `consensus/core/src/notepool/`, the pool stores). Does validation enforce, for every operation and every block, what [POOL-SPEC.md](../POOL-SPEC.md) v1.3 states: authorization by the note's current key over the signing preimage of P5.2, including the bound transparent outputs; conservation of value; the freshness window of P5.3; the lock rule of P5.9; serial uniqueness and derivation? Can any transaction create, destroy or redirect value, or spend a note without its key? Does the selected-parent replay reproduce the parent's verdict exactly?
2. **The pool commitment and the pool hand-over** (`header.rs` hashing, the pool Merkle tree, the pruning-point pool sync, `get_pool_state_at`, `get_pool_diff_of`). Does the header commit to the state the spec says, can a node be made to accept a wrong pool state at the pruning point or serve a wrong one to a light wallet without the root failing to match?
3. **Finality anchors** (`consensus/core/src/finality_anchor/`, the virtual processor's guard and deny-list bookkeeping, `protocol/flows` gossip and IBD, `trustee-signer/`). Is the 3-of-5 verification correct and free of signer repetition; is the equivocation rule exactly decidable as specified and its evidence lifecycle safe against false disqualification; does fail-open engage and disengage as stated; can an anchor-free or old-anchor chain be made to win during IBD; can the signer be made to equivocate by its own failover?
4. **The light wallet** (`wallet/core/src/light.rs`). Given a hostile node and honest trustees, can the wallet be made to accept a pool state that is not the chain's, miss a note of its own, or spend on a stale view? Is the anchor check against followed roots sound, and the restart on disagreement complete?
5. **Wallet key material and backups** (`wallet/core/src/storage/local/notevault.rs`, `notekeys*`, the backup sealing in `cli/src/backup.rs` and the bundle code, `wallet/keys`). Are the vault key derivation from the 24 words, its wrapping under the password, the per-note file encryption and the sealing of backups to the words sound; can a backup or a wallet file be read without the words; does the password change rewrite everything it should?
6. **Payment codes and requests** (`cli/src/modules/pay.rs`, `receive.rs`, `request.rs`, the code encoders and decoders). Can a code be forged, replayed, altered in flight, or made to pay a different amount or key than shown; are locked notes refused where the spec requires?
7. **Self-update** (`cli/src/selfupdate.rs`, the manifest format of `scripts/release-notice.sh`). Can a wallet be made to install an unsigned, downgraded or substituted binary; is the hash check and the signature check complete across platforms?
8. **The RPC additions** (`rpc/core`, `rpc/service`, `rpc/grpc`): the paged UTXO query, the pool and anchor calls. Any unbounded work reachable by a stranger, any panic reachable from input, any response that a node can shape to mislead a client beyond what the trust model admits.
9. **The mempool fee floor and the own lane** (`mining/`, the mempool changes). Can the floor be bypassed by anyone but the node's own opted-in lane?

## Out of scope

Upstream rusty-kaspa's consensus, P2P and PoW as inherited; the Telegram bot's transport and Telegram itself; the desktop app's UI layer beyond the service calls it makes; the faucet; the Ansible deployment; the website; the stratum bridge beyond its handling of the pool commitment; economic parameters and the emission schedule, which are decisions, not code defects; the lanes' point-of-sale design, which is not implemented.

## What the auditor receives

The public repository at the tag; this scope; the self-assessment; POOL-SPEC v1.3 and the two prior external reviews with their triage; the whitepaper; the test suite and CI; INCIDENTS.md; access to the public testnet and, on request, a dedicated testnet node and a trustee signer configured with throwaway keys for drills; the founder's time for questions, in writing so the answers can be published with the report.

## Deliverables

A written report with: the scope as actually covered, including anything in this statement that was not reached and why; findings with a severity scale defined in the report, each with reproduction and the affected code; recommendations; and, after the project's fixes, a re-review of every finding rated high or critical with the fixing commits named. The report is published whole, with no finding removed; the project's responses are published beside it. Findings rated critical or high must be closed, with linked commits, before mainnet genesis (PLAN P8.8).

## Timeline

Engagement signed by the end of October 2026; report before the mainnet genesis announcement, which is made at least four weeks before genesis. If the audit cannot be delivered on that timeline, the launch date moves; the audit does not shrink.
