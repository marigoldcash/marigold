# A first sync that takes minutes — a design note (proposal, 2026-10-11)

Status: decided in direction by the founder on 2026-10-11 ("Yes that is the solution. It's not a node, it's a wallet, light is OK"), written after tester Charly found a sync of his own unusable on a 36 GB Mac ("each time I close it, it has to resync and it takes a long time before I can use it for transactions"). The aim: make the private way fast enough for everyone, so that the choice between a public computer and your own copy is a privacy choice and no longer a convenience, and the public-only way can go. Nothing is built yet; the open questions at the end are the work.

## What a first sync is today, measured

A fresh sync of testnet-10 on pve3 (96 cores, the peer on the same machine, so no internet in the way; the wallet's own node settings: pruned, UTXO index on, RAM scale 0.5), started 2026-10-11 13:47 UTC:

| Phase | Amount | Time | Rate |
|---|---|---|---|
| Pruning point proof and headers since the pruning point | 1.28 million headers | 12.7 minutes | about 1,700 a second |
| Block bodies since the pruning point, validated | 1,282,527 blocks | 48.4 minutes | 440 a second |
| The whole first sync | | 61 minutes | |
| Disk after | 5.6 GB | | |
| The pool state at the pruning point | 859 notes, 4 lanes | handed over in a moment | |

The pruning point sits about 36 hours behind the tip, and at ten blocks a second that is 1.3 million blocks. Every one of them is downloaded, validated against the UTXO set and the pool, and written to the database, because that is what a node is. On the fastest machine we have, with the data next door, that is an hour and a 5.6 GB database. A laptop validates a few hundred blocks a second at best and downloads over the internet, so Charly's hours are the same arithmetic, not a fault. Two things made his case worse and are fixed in v2.83: the memory watchdog stopped his sync every few minutes on a false reading of 0.0 GB free, and a first run that is stopped before it finishes is discarded, so he started from nothing each time.

Where the time goes decides what helps: the download is a minority of it; validation and database writes are most of it.

## The options

### A. A shorter pruning window at mainnet

The pruning depth is a network parameter, set at the parameter freeze (PLAN P9.5). Kaspa's 36 hours protect a chain whose finality is probabilistic. Marigold's finality is anchored by the trustees every 300 blocks at launch, so a reorganisation deeper than an anchor cannot happen whatever the window, and a window of twelve hours would cost nothing in safety. It cuts the first sync to a third, which turns an hour into twenty minutes on pve3 and hours into an hour on a laptop. It changes no design and it leaves hours as hours.

### B. Compressed bundles served by nodes

A node could pack the blocks since the pruning point into compressed bundles and a wallet could stream them. Block bodies compress two or three times, so the download shrinks by that much; the validation and the writes do not shrink at all, and they are where the time goes. It is a real saving on a slow line and no answer to the hours.

### C. Only our own coins, from a public computer

A wallet that keeps nothing and asks a node about its notes is what "a public computer" is today. It works at once and it is the privacy problem: the questions a wallet asks name its notes, and whoever runs the node learns which notes are one wallet's. Asking about everyone's notes one by one to hide one's own would be slower than the sync.

### D. Start from the latest anchor: the light sync

The founder's compromise, and it fits what the chain already has. Three facts make it possible:

- Every block header carries the **pool commitment**, a root over the pool state: every live note's serial and current key. The pool state is small, a few dozen bytes a note: 859 notes on testnet-10 today, which a node handed over in a moment during the measurement.
- The trustees' **finality anchors** sign a block every 300 blocks at launch. A wallet holds the trustees' keys, so it can check an anchor without a node's help.
- A node can hand over the pool state as of a block, the way it hands over the UTXO set at the pruning point today.

The light sync is then: fetch the latest anchor and its header from any node; check the anchor's signatures against the trustees' keys; fetch the pool state as of that block and check it against the header's pool commitment; look up one's own notes in it locally, asking nobody; then follow the blocks after the anchor, which are minutes of them, and keep following as the chain grows, the way the wallet follows it today. Nothing older than the anchor is downloaded or stored. On a laptop this is a download of the pool state and a minute of blocks: minutes, not hours, and the same at every start, because the next start takes the latest anchor again.

What it costs. Such a wallet validates nothing before the anchor; it trusts the trustee quorum for the past, which the chain already does for finality, and it keeps no history, so it cannot serve others. It is a light wallet, not a node. The network still needs full nodes, run by whoever wants to be one, and the wallet's own-copy choice stays for them. The pool state hand-over is a new message between nodes and wallets; a node has to be able to produce the pool state as of an anchored block, which means keeping it for the last anchor or two rather than only the tip. The ledger side, for people who mine to an account, is the same shape with the UTXO set, and can come second.

What it gives. The private way takes minutes. The menu's first choice stops being a convenience; it becomes the privacy choice, and the public-only choice can go, which is the founder's aim. A tester who closes the wallet loses nothing, because there is nothing to lose: the next start is a fresh light sync from the latest anchor.

## Recommendation

D, with A beside it at the parameter freeze. D is the one that changes what a laptop experiences; A is a number in the parameters that makes every full node's first sync a third of the work and is worth having regardless. B is not worth building on its own. C is today's public mode and stays only as long as D does not exist.

## What was decided (2026-10-11)

- **The light wallet is the default on every wallet**: low memory, low storage. Its storage is the pool state, the ledger set it needs, and the blocks since the latest anchor; nothing older. It is a wallet, not a node, and it is called a wallet.
- **The connect menu becomes:** 0 Don't connect; 1 Connect to a public computer while downloading your own copy, for privacy, the default, where "your own copy" is the light sync and the public computer covers the minutes it takes; 2 Download your own copy without using the public computer.
- **Mining works from a light start.** A wallet that follows the blocks after the anchor and validates them holds exactly what a block template needs: the tips, the ledger and pool state, and the difficulty window, which the hand-over includes as a few thousand headers before the anchor. It can mine once it has followed the chain for the length of the merge-depth window, minutes at ten blocks a second, not after a full sync.
- **A light wallet can grow into a full node by keeping what it sees.** Keep every block from the anchor forward instead of dropping them, and after one pruning window the wallet holds the same 36 hours a full node holds and can serve peers. The only difference from a node synced the old way is that its earliest history came from a trustee-signed anchor rather than a validated proof. This is an advanced choice, never the default.

## Open questions

1. The pool state hand-over: its message, its size on testnet-10 and at the scale mainnet is planned for, and how a node keeps the state as of an anchored block.
2. Whether the light wallet keeps following blocks after the anchor itself, validating pool ops as it goes, or trusts each new anchor and refetches the delta; the first is more code and more private, the second is simpler and leans on the anchors more.
3. The ledger side for miners: the UTXO set as of the anchor, or the account stays on a full node's copy.
4. What a light wallet does when the anchors stop, the fail-open case of POOL-SPEC P5.8: it should say so and wait, not fall back to a public computer unasked.
5. A, the pruning window, is a line in the mainnet parameters and needs a number: twelve hours is the first proposal.
