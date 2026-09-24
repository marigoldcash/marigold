# WALLET.md — Note wallet walkthrough

A step-by-step manual walkthrough of every note-pool wallet flow (P7.1-P7.6) on a local testnet, using the real interactive `marigold-cli`. Unlike [SMOKE.md](SMOKE.md) (which predates the wallet and works around `marigold-cli`'s REPL-only nature with RPC-direct tools — see NOTES.md's P0.3 entry), every step below genuinely uses the wallet as a user would. Written for P7.7; verified line-by-line against a running local node (see "Verification" at the end).

## Before you start

Rebuild every binary you'll use — a stale one can silently misbehave (bit this project at P2.3 and again at P4.2):

```bash
cargo build --release --bin marigoldd --bin marigold-cli
```

`kaspa-miner` (the community `elichai/kaspa-miner` tool) must be installed separately — see NOTES.md's Environment section.

**Use `simnet`, not `devnet`, for anything in this document.** This is not a style preference: `DEVNET_PARAMS` sets `pool_activation: ForkActivation::never()` (and `toccata_activation: never()` too) — the note-pool subnetwork is consensus-gated off on devnet specifically, so every `note` command in this walkthrough would be rejected at the consensus level on a devnet node. This isn't a wallet bug: devnet is the one network shape deliberately kept pool-inactive (useful for isolating pre-pool behavior; see PLAN P6.5's activation notes). Mainnet, testnet, and simnet all run with `pool_activation: ForkActivation::always()` — all upgrades active from block 0, per P2.6's new-chain rule. Simnet is the right choice *locally* because it's also the only network with `skip_proof_of_work: true`, letting mined blocks confirm instantly without a real miner grinding.

## 1. Launch a local simnet node

The wRPC Borsh listener `marigold-cli` connects over is **not started by default** — `--rpclisten-borsh` must be given explicitly (unlike gRPC/P2P, which are):

```bash
mkdir -p x-simnet-local-data
target/release/marigoldd --simnet --enable-unsynced-mining --unsaferpc --disable-upnp \
  --utxoindex --rpclisten-borsh=127.0.0.1:27510 --appdir=x-simnet-local-data \
  > x-simnet-local-data/node.log 2>&1 &
```

Confirms up once the log shows all three listeners:

```
GRPC Server starting on: 127.0.0.1:26510
P2P Server starting on: 0.0.0.0:26511
WRPC Server starting on: 127.0.0.1:27510
```

(Simnet's default ports: gRPC `26510`, P2P `26511`, wRPC Borsh `27510` — different from devnet's `266*0`/devnet's wRPC `27610` used in [SMOKE.md](SMOKE.md)/P4.1's script.)

## 2. Start `marigold-cli` and connect

```bash
target/release/marigold-cli
```

At the `$` prompt:

```
network simnet
server 127.0.0.1:27510
connect
```

The prompt changes from `N/C $` (not connected) to showing sync state, then a live balance once synced — starting `SYNC $`/`SYNC ... $` briefly against a fresh node.

## 3. Create a wallet

```
wallet create
```

Walks an interactive wizard, in this exact order:

1. A short explainer, then `Keep a ledger account too? [Y/n]:` — enter (or `y`) keeps one; `n` makes a **notes-only wallet** (section 12). Most people never need the ledger; it is for mining and for exchanges that only pay to an address, and `account create bip32` can add it later.
2. `Default account title:` — only asked when keeping a ledger; press enter to skip.
3. A phishing-hint explainer paragraph, then `Create phishing hint (optional, press <enter> to skip):` — press enter to skip.
4. `Enter wallet encryption password:` (masked) and `Re-enter wallet encryption password:` (masked) — must match.
5. `Enter bip39 mnemonic passphrase (optional):` (masked) — only asked when keeping a ledger; press enter to skip (a *second*, optional secret on top of the wallet password; skip it unless you specifically want one).
6. That is all. The wizard says that your password and a backup are what bring the wallet back, and the wallet is open. The vault's 24 recovery words exist (the vault key is their entropy) but are not put in front of everyone: `note vault words` prints them at any time for whoever wants paper, and with `advanced on` the wizard runs the ceremony as before — your own 24 words or generated ones, shown once in a numbered panel.

The wizard then prints the wallet's storage path (and, with `advanced on`, the ledger address when keeping a ledger). The account's own 12-word phrase is deliberately not shown (it is derived from the vault key; `export mnemonic` produces it if another program ever needs it). The wallet is opened and activated in the same session — no separate `wallet open` needed.

## 4. Fund the wallet

Every note-pool operation needs a ledger balance to mint from. Mine to the receive address printed in step 3, from a second terminal:

```bash
target/release/kaspa-miner --mining-address <your-marigoldsim-address> \
  --kaspad-address 127.0.0.1 --port 26510 --threads 2 --mine-when-not-synced
```

Coinbase outputs need `coinbase_maturity * 2` confirmations before the wallet's own balance tracking treats them as spendable (the same rothschild/wallet-side rule SMOKE.md's P4.2 entry found — not a consensus rule). At simnet's 10 BPS, `coinbase_maturity = 1000`, so leave the miner running until you're comfortably past ~2000 blocks, then stop it (`Ctrl-C`, or `pkill -x kaspa-miner`). Check progress with:

```
list
```

which shows every account's ledger balance (mature and pending) and address. Wait until the mature figure is nonzero before continuing.

## 5. Mint, check balance, list notes

```
note mint 5
```

Prompts `Enter wallet password:` again — **every note-pool command that needs the wallet secret re-prompts for it individually; the CLI never caches a plaintext password across commands.** Mints 5 MAGLD, decomposed into the P1.6 denomination ladder (5×1 MAGLD here). The very first note-storing call also silently runs the vault's 24-word ceremony if one doesn't exist yet (see step 6) — the words are logged as a warning, easy to miss; run `note vault create` explicitly beforehand if you want to see them properly (below).

```
note balance
note list
```

`balance` shows totals by denomination; `note list` shows every held note's serial, denomination, provenance (`Cold`/`Hot`), and status (`Active`/`HandedOver`/ `Superseded`).

**A newly-submitted note-pool transaction needs a confirming block before it shows up in on-chain queries** (`note vault verify`, another wallet's `receive`, etc.) — on a real network this happens automatically as blocks keep arriving; on this local testnet, mine at least one more block after any note operation before checking its on-chain effects from elsewhere.

## 6. The note vault: create, backup, verify, export

The vault (PLAN P7.6) is the wallet's note key database — one encrypted file per note, backed by its own 24-word recovery key `K` (unrelated to the wallet's own BIP32 mnemonic from step 3; see DECISIONS.md's "Note vault, backup, and restore-rotation policy" for the full design).

```
note vault create
```

Run this **before** your first mint if you want to see the 24 words with the proper one-time warning (an already-auto-provisioned vault, from having minted first, just says "a note vault already exists").

```
note vault backup <dir>
```

Copies the vault's files to `<dir>` — pair this with the 24 words (written down separately, never stored alongside) for a full recovery. Do this after every batch of new notes; a vault copy only protects notes it was taken after receiving.

```
note vault verify
note vault verify deep
note vault verify backup <dir>
```

Three checks: **light** (this wallet's own notes against the live pool, no secret needed), **deep** (decrypts and re-derives every note's key — the mandatory first step of an actual restore, and the only one that catches a corrupted vault file), and **backup `<dir>`** (light-verifies a standalone backup copy directly — no wallet open, no secret — "is this old backup still any good" without ever restoring it).

```
note vault export <dir>
```

Paper QR export: prints an encrypted QR (and writes the same page as hex text to `<dir>`) for every ~40 notes, plus a freshly-generated 12-word password printed once — write it on the printed page itself (its threat model is safe physical storage, not a secret kept apart from the vault — see DECISIONS.md). `note vault import <page-file> ...` reads the pages back (prompts for the password interactively) and imports each note as a bearer key, immediately rotating it.

## 7. Receive a payment (fresh-pk mode)

In a **second** `marigold-cli` session (a separate `wallet create` under a different storage location — pass `wallet create <name>` to keep multiple named wallets, or run from a second `$HOME`), the recipient runs:

```
note request 2
```

Shows a QR + text payload (`marigoldreq:...`), then waits (up to 120s, safely re-runnable/re-checkable via `note list` after a timeout — the request key stays stored either way). The payer, on their own wallet, pays it:

```
note pay marigoldreq:<...text from above...>
```

The moment the payment confirms (mine a block), the requester's `request` returns showing the received notes.

## 8. Hand a note to someone directly (bearer mode)

```
note export <serial>
```

Auto-isolates first if the note's key is shared (a POS landing-pad note, for instance) — waits for that isolation to confirm before showing the handover payload. Shows a QR + text (`marigoldnote:...`); both parties can technically spend the note until the receiver rotates it, so show this only to the intended recipient. They import it with:

```
note import marigoldnote:<...text from above...>
```

which stores it (always `Hot` provenance — POOL-SPEC.md's same-key-in-two-wallets rule) and immediately rotates it to a fresh key.

## 9. Point-of-sale checkout

```
note pos 0.5
```

One landing-pad `pk` for exactly this sale: shows the request QR immediately, waits for the exact payment, and the instant it confirms, sweeps every note that landed on the shared key to its own fresh key — the shared-key exposure window is bounded to this one call.

## 10. Redeem notes back to the ledger

```
note redeem <serial> [<serial> ...]
note redeem amount <amount>
```

Either redeem specific notes by serial, or let the wallet pick enough notes to cover at least `<amount>`. Reports the redeemed value, fee, and net ledger balance gain.

## 11. Restore from a vault backup — "24 words + the files"

Simulates recovering a wallet from nothing but a vault backup and its 24 words. In a **fresh** wallet (no prior vault):

```
note vault restore <backup-dir> <word1> <word2> ... <word24>
```

Copies the backup's files in, recovers `K` from the words, deep-verifies (reports live/stale/corrupted), then — by default — offers the restore-time rotation: 2-5 randomly-composed batches, each its own transaction, rotating every recovered note to a fresh key (invalidating every old copy of this backup, including any that may have leaked). Each batch is attempted independently — one batch's failure doesn't stop the others.

**Caveat found while validating this**: if the *source* wallet the backup came from is still active and mid-spend (e.g. you're testing restore against a backup you just took without pausing the original wallet), a rotation batch that happens to need a fee-stamp from a note the source wallet is simultaneously spending will fail with `already consumed by transaction ... in the mempool` (or, once that transaction confirms, `does not exist in the pool`) — a real instance of POOL-SPEC.md's same-key-in-two-wallets hazard, not a wallet defect. Mine a confirming block for the source wallet's pending transaction and re-run `note vault restore` (idempotent — it re-copies and re-verifies) to pick up wherever it left off.

## 11b. Automatic backups to Telegram, and the restore (2026-09-23/24)

The wallet keeps an encrypted copy of itself in Telegram, and keeps it current by itself — the way a phone backs itself up to its maker's cloud, except that the only servers involved are Telegram's and they hold nothing they can read. It needs the wallet's bot (`mobile telegram <token>`, the token from @BotFather) paired with its owner; the backups then go to the bot's own chat with them, the same chat the payment codes arrive in. A private group the bot is a member of can be given instead.

```
backup telegram                 # the first time: posts a checkpoint and starts the automatic backups; later: a checkpoint now
backup telegram status          # where they go, on/off, last checkpoint, deltas since, last post
backup telegram off / on        # pause and resume the automatic posts
backup telegram 5181777138      # send them to a group instead: its id as Telegram shows it, kept afterwards
```

The first run posts a **checkpoint** — the keys file and every note file — straight away. From then on, while the wallet is open, it posts by itself: a **delta** holding only the files that changed since the last post (and the names of any removed), once the vault has been quiet for two minutes and at most every ten; a fresh checkpoint once a week, or sooner when the deltas since the last one outweigh half of it; and any change still unposted goes out at `close`. Nothing is asked: every archive is sealed under a key made from the wallet's 24 words, which the owner keeps anyway and which bring the whole wallet back on any machine. The wallet remembers what it last posted in `telegram-backup.json` beside the wallet file, which is how a delta knows what changed.

Every backup message is delivered silently — no sound, no badge — so the chat stays quiet unless it is opened. What lands there, per backup: a start message naming it, the parts as documents (`marigold-<wallet>-c20260924T100000.full.mgb.p001of003` for a checkpoint, `….d007.mgb.p001of001` for the seventh delta after it) with "Part 1 of 3" captions and the archive's sha256, and an end message. Parts stay under 20 MB because that is the most a bot may fetch back. A wallet of a few hundred notes is a few hundred kilobytes, so a checkpoint is one part and a delta far less; a week never needs more than the newest checkpoint and the deltas after it forwarded back.

To bring the wallet back on any machine:

```
wallet restore telegram [<name>]
```

It asks for the bot's token hidden (a token typed on the command line would sit in the terminal and its history); then forward the bot everything it posted in the last week: open the chat the backups are in, select the backup files, forward, pick the bot. Order does not matter, and extra files do no harm — the wallet takes the newest checkpoint and the deltas after it and ignores the rest. It goes on a few seconds after the last part, fetches each, asks for **the 24 words** (a backup from before September 2026 opens with its passphrase instead) — merges the checkpoint and the deltas in order (it refuses if a delta in the middle is missing and says which), and runs the ordinary restore. A pre-checkpoint archive sealed with a passphrase (from before 2026-09-24) restores the same way with its passphrase. A bot cannot read a chat's history, which is why the parts have to be forwarded to it. The Telegram bot API is the whole dependency; nothing is stored on any server of ours.

## 12. Notes-only wallets

Answer `n` to `Keep a ledger account too?` and the wallet has a vault and nothing else: no account key, no ledger address ever derived, `list` shows no account and the prompt carries no account name. Everything under `note` works exactly as above — `request`, `pay`, `receive`/`export`, `note pos`, `note verify`, `note vault backup`/`restore`, `note history` — because none of it ever needed the ledger; it only used the account as a handle. `balance` shows notes alone. The ledger commands (`mint`, `redeem`, `transfer`, `sweep`, `estimate`, `address`, `utxos`, `message sign`) refuse with one line: *This wallet keeps notes only — there is no ledger account. 'account create bip32' adds one.* That command attaches the ledger at any later time, derived from the same 24 words, so there is no new secret and backups need nothing extra.

Three things a notes-only wallet meets that a ledger wallet does not:

- **Bootstrap.** It cannot mint or mine, so its first notes must arrive by `pay` from someone else or by importing a bearer note (`receive`). On testnet the faucet hands out bearer notes with fee stamps.
- **Fee stamps.** A pure note transfer pays its fee with a small note, so a wallet holding only large denominations cannot pay for anything — splitting included. A first payment to it should include some 0.01s.
- **Paying out without change.** `exchange <address> <amount>` pays an exchange (or anyone) straight from notes, in one transaction, with no ledger involved. But a redeem cannot make a note, and there is no ledger for change, so the notes chosen must cover the amount to within one 0.01 note; the whole redeemed value less the fee goes to the address, so the deposit arrives a fraction over what was asked, never under. If the closest cover is further over than that the wallet says so and does nothing — pick an amount the notes cover, or `account create bip32`.

`auto on` still works: the password is checked against the vault key instead of an account key, and housekeeping tidies notes (ten of a size into one larger) while the ledger steps stay off.

## Gotchas found while writing this (2026-08-17)

- **`marigold-cli` genuinely cannot be driven by piped stdin** (NOTES.md's P0.3 finding still holds — it needs a real TTY, crossterm raw mode). This walkthrough was validated with `pexpect` (a Python pty-driving library, already available in this environment), which allocates a real pseudo-terminal and sends literal `\r` (not `\n` — crossterm raw mode doesn't do the newline translation a cooked TTY would) for Enter. Useful precedent for any future automated CLI validation.
- **wRPC Borsh needs `--rpclisten-borsh` explicitly** — unlike gRPC and P2P, it isn't started by default. `marigold-cli`'s `connect` fails with "Connection refused" without it, silently continuing to work for anything that doesn't need the network (like `wallet create` itself, which is why the wizard "succeeding" isn't proof the wRPC connection is up).
- **Real bug found and fixed**: `note vault restore`'s rotation loop aborted entirely on the first batch's failure, leaving every batch after it — including ones with no conflict at all — unexecuted. Fixed to report a failed batch and continue with the rest (`cli/src/modules/note.rs`).
- **Real bug found and fixed**: `deep_verify`'s findings weren't reconciled back into local note status. A serial it reported `stale` stayed `Active` in the local index, so `rotate_notes`'s fee-source selection kept proposing the same dead serial as a spare on every subsequent batch, failing repeatedly for the same reason. Fixed by having `note vault restore` mark every `stale` serial `Superseded` locally right after `deep_verify` reports it, before planning any rotation.
- **`note vault import`'s password moved from a CLI argument to an interactive masked prompt** during this pass (it was a known, explicitly-flagged gap from P7.6) — matches how the wallet password itself is always prompted, never passed as text.

## Verification (2026-08-17)

Walked every step above against a real local simnet node (`kaspad --simnet --enable-unsynced-mining --unsaferpc --utxoindex --rpclisten-borsh=127.0.0.1:27510`), driving the real `marigold-cli` binary interactively (via `pexpect`, allocating a genuine pty — see "Gotchas" above):

- Created a wallet through the full interactive wizard exactly as documented; captured its mnemonic and receive address from the real terminal output.
- Funded it (2,350 mined blocks, past `coinbase_maturity * 2`); `list` showed a mature transparent balance.
- `note mint 5` → 5×1 MAGLD, auto-provisioning the vault (24 words logged); `balance`/`note list` matched exactly.
- `note vault create` correctly refused a second ceremony ("already exists"); `note vault backup`/`note vault verify backup <dir>` round-tripped cleanly (5 live, 0 stale); `note vault export` produced a real scannable QR + password.
- `note redeem amount 2` redeemed 2 notes, correct fee and balance-gain math; `note list` showed the right mix of `Superseded`/`Active` rows afterward.
- Created a second, completely independent wallet and ran `note vault restore` against the first wallet's backup + 24 words — recovered exactly the notes still live on chain (correctly excluding ones the first wallet had since redeemed), and — after mining confirmations for the first wallet's in-flight transactions — completed the full batched restore-rotation with zero failed batches on a clean run.

Every documented command matched its documented behavior exactly. Two real bugs (listed above) were found and fixed during this pass; full writeup in NOTES.md's P7.7 entry.
