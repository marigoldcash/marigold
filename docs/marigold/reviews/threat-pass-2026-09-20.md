# Wallet threat pass, 2026-09-20 (PLAN P8.5)

Three read-only reviews of the wallet as shipped in v2.47.217, each against one attack surface — the codes and payment flows; the vault, backups and key imports; the service, the Telegram bot and the node's RPC — followed by a fix pass the same day. Findings verified in the code before anything was changed. Severities are the reviewer's. Every fix is in the commits that follow this file; the build is 2.47.223.

## Fixed

| Sev | Surface | Finding | Fix |
|---|---|---|---|
| high | node | The below-floor fee exemption for "the node's own wallet" defaulted on for any node whose RPC listens on loopback — including the public node, which nginx proxies onto loopback. Any wRPC client of rpc1 could submit zero-fee transactions into pve3's mempool and blocks. | Opt-in only: `--accept-own-below-floor` must be given. The wallet's built-in node sets it only when it listens on loopback. Deployed to all three hosts the same day. A node that refuses the fee is explained in the wallet. |
| high | payments | Paying a `marigoldreq:` code showed nothing and asked nothing before the transaction; a swapped amount or key in a displayed request QR would have been paid as shown to nobody. A pinned amount silently overrode a typed one. | `pay <request>` prints the amount and the key's tail and asks; a typed amount that differs from the pinned one is refused, not tie-broken. |
| high | storage | `wallet restore` left every restored key as it was: any other copy of the backup could spend the same notes, shown as safely held. | The restore leaves a marker; the first open with a synced node rotates every note to fresh keys (as `note vault restore` already did) and removes it. |
| high | storage | Keys imported from another wallet, or whose own rotation failed, stayed spendable from both wallets indefinitely and counted as ordinary balance. | Housekeeping rotates Hot active notes a batch a tick; `balance` names how much sits on keys another wallet has seen. |
| high | bot | The six-digit pairing code was accepted from any Telegram user, with no limit, no expiry and an oracle reply. | Code drawn uniformly, dead after fifteen minutes or five wrong tries, one reply for every miss; `mobile telegram` makes a fresh one. |
| medium | bot | PIN lockout and the daily total lived in memory: a restart unlocked and reset them. The lockout message said so. | Both persisted in the bot's file; `mobile telegram unlock` clears a lock on purpose. |
| medium | bot | The bot answered the paired user in any chat, so a command typed in a group posted a bearer code there. | The pairing chat is bound; other chats are ignored. |
| medium | bot | The token rode in request URLs and could reach the log through transport errors; the config file had a moment at the umask; the PIN hash was one unstretched SHA-256. | Token redacted from errors; file created 0600 and moved into place; new PINs hashed with Argon2id, old ones still verify until reset; `/receive` fees count against the day. |
| medium | payments | `receive` of a `marigoldpay:` code did not look at locks, so a locked note dressed as a plain handover would be taken and lost to the payer's refund key; the same code could be pasted twice. | Locked notes refused in a plain handover; a code already in the wallet says so. |
| medium | storage | Vault key, manifest, note files, journal, shares, paper pages and restored files were written at the umask, world-readable on most machines; the manifest was rewritten in place. | Owner-only files and directories, existing vaults tightened on load; the manifest is written beside and renamed. |
| medium | RPC | `get_notes_by_serial` took an unbounded list under virtual's read lock; `get_pool_stats` walked the whole pool store per call, both open to strangers on rpc1. | 1,000 serials per call; pool stats served from a five-second cache. |
| low | payments | A lock length could overflow `u64` and panic the wallet. | Checked arithmetic. |
| low | miner | `mine-to --listen` said nothing about who can reach it. | The help says: your own network only; anyone who reaches it can steer the miner and read the payout address. |

## Correct, checked

Every code decoder length-checks before it slices, no decoder trusts a length field, prefixes are unambiguous. The receiver verifies serial, key and denomination against the chain before showing an amount. Share-key derivation is a hashed ECDH tweak with a fresh ephemeral key per payment; the payer cannot recover the share secret and the chain shows unrelated keys. Executed operations cannot be replayed; signed operations expire after about an hour. No path traversal from archive or vault content; no panic reachable from file content. Argon2id for the vault key, paper pages and archives. The journal holds no secrets; the terminal keeps no history file. The inherited Kaspa import paths are gone; `import legacy` refuses. `serve` keeps the password in a guarded, mlocked, split form. The miner verbs are not reachable on rpc1.

## Open, for decisions or later work

- **A node in its first sync accepts transactions it will lose.** The wallet now refuses to use such a node; whether `SubmitTransaction` should refuse during IBD the way block templates do is a node question for P8.4.
- **The wallet trusts its node's word** for balances, sync state, which notes exist, and now which notes come back. Inherent for a light wallet; a second node's opinion is the only check, and out of scope for now.
- **A request code carries no signature.** The confirmation closes the practical attack; a signed request (key signs amount and expiry) would let the wallet tell a tampered code from a real one. Design question.
- **Receipts have no consumer yet** (P8.0i). A forged receipt achieves nothing mechanical today.
- **Codes carry no expiry.** A photographed `marigoldpay:` is money until rotated (warned); a photographed request is live indefinitely.
- **`MARIGOLD_WALLET_PASSWORD`** stays visible in the process environment; the file is the better route and the usage text should say so.
- **A bundle paid to a share key shares one one-time key**, so its notes are linkable to each other on chain. Stated design.
