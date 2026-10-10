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

## 2b. Keeping the wallet current: `update` (2026-10-08)

Once a day the wallet reads `https://marigold.cash/release.json`. When it names a newer release the wallet says so once, and since v2.79 the line ends with "Type `update` to install it". `update` fetches the document again, checks that the document's **release manifest** — the newest release's file names and SHA-256 digests — carries a quorum of the trustees' signatures (the same pinned keys that make finality anchors and release notices count; `consensus/core/src/finality_anchor/release_manifest.rs`), downloads this platform's file from the compiled-in release address (`github.com/marigoldcash/marigold-wallet/releases/download/v<version>/<asset>`), checks its digest against the signed one, and asks "Install <version> and restart now? [Y/n]". Yes closes an open wallet the way `close` does (its backup goes out), stops the sync, moves the running program aside as `marigold-cli.old`, puts the new one in its place and starts it again with the same arguments. No fetched document can send the wallet anywhere, and nothing is installed whose digest the trustees did not sign; a file that does not match is deleted and said so. Docker says `docker pull` instead; a build for a platform without a release file says so. `MARIGOLD_UPDATE_FORCE=1` installs the signed release even when it is not newer, for testing the path against a real release.

The manifest is written by the release procedure: the binaries workflow attaches `SHA256SUMS-cli.txt` to the release, and `scripts/release-notice.sh --latest <v> --manifest SHA256SUMS-cli.txt` has each testnet trustee host sign it (`marigold-trustee-signer --sign-release-manifest`) and writes `release` into the document. The desktop wallet keeps the notice with a download link until the installers are signed for their platforms (Apple notarization, a Windows certificate — DECISIONS.md 2026-10-08).

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
5b. The password policy (founder, 2026-10-10, after a day of "no password on the testnet"): a password, or at least a PIN, of four characters or more is required of a regular user; an empty answer at `Enter wallet encryption password:` is refused with the line that says where the choice is. A wallet without a password is a deliberate advanced choice, whatever the reason: `advanced on` before `create`, the "Advanced: no password" box on the desktop's Create screen, or `MARIGOLD_ALLOW_EMPTY_PASSWORD=1` in the environment for a script. The policy is one function, `password_verdict` in `cli/src/lib.rs`, with tests that hold it (GitHub #20); nothing about it changes at mainnet. The password prompts of an existing wallet take Enter (a second Enter, at open) since such wallets exist.
6. The wallet shows its **24 recovery words** in a numbered panel, explains that they are the key to everything — the ledger balance on their own, the notes together with a backup, and every backup opens with these words and never with the password — and, after `Press <enter> once you have written them down:`, asks for two of the words by number (`Word 7 of 24:`) as a check that the paper is right. An empty answer shows the words again; the check does not end until both are right. The panel has rules above and below and nothing at the sides, so a selection across it copies the numbers and words clean into a password manager; the advice is paper or a password manager, never a screenshot or a photo (founder, 2026-10-06). With `advanced on` the wizard first offers to take your own 24 words instead, in which case nothing is shown back or checked. `words` prints them again at any time.

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

**A newly-submitted note-pool transaction needs a confirming block before it shows up in on-chain queries** (`wallet verify`, another wallet's `receive`, etc.) — on a real network this happens automatically as blocks keep arriving; on this local testnet, mine at least one more block after any note operation before checking its on-chain effects from elsewhere.

## 6. Your notes: the words, a check, a paper copy

A wallet keeps its notes as one encrypted file each, under a key that is the entropy of the wallet's 24 words (PLAN P7.6; DECISIONS.md "Note vault, backup, and restore-rotation policy" has the design, under the old name — since 2026-10-06 nothing a person reads says "vault": the bucket of money is the wallet, the money in it is notes, and the software is the Marigold wallet, `marigold-cli` in the terminal and `marigold-wallet` on the desktop).

```
words                 # the wallet's 24 words, for paper (asks the password); 'wallet words' is the same
wallet verify         # every note you hold checked against the live pool, no secret needed
wallet verify deep    # also opens every note file and re-derives its key — the one check that catches a corrupted file
wallet paper export <dir>    # a paper QR copy: encrypted pages written to <dir>, their own 12-word password printed once
wallet paper import <page-file> ...   # reads the pages back (asks for that password), rotating every note as it lands
```

The words are shown at creation and checked (step 3); `words` shows them again. They are the key to everything: the ledger balance on their own, the notes together with a backup (`backup`, section 11b) — and every backup opens with the words and never with the wallet password.

The paper export is deliberately self-contained: its pages are encrypted under a short password generated and printed once at export time, to be written on the printed page itself (its threat model is safe physical storage, not a secret kept apart — DECISIONS.md). Recovering from paper needs the pages and that password, never the wallet's files or words.

The old `note vault …` spellings say their new name and run (`words`, `verify`, `export`, `import`); `note vault create`, `backup <dir>` and `restore <dir>` are gone — a wallet has its notes and words from the moment it is made, and `backup`/`wallet restore <file>` replaced the loose-file copy.

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

## 11. Restore from a backup file — "24 words + the file"

```
wallet restore <file> [<name>]
```

Rebuilds every wallet in a backup file whose 24 words are given (section 11b for the file's shape). Never overwrites: if anything it would write is already there it writes nothing and says which file stopped it; `<name>` restores a single wallet under another name, so it can sit beside one you already have.

A restored wallet is a copy of the keys, and any other copy of that backup can spend the same notes. So the first time the restored wallet opens with a synced node it offers to **rotate every note to fresh keys** — recommended whenever another copy could exist, optional, never done without a yes (founder, 2026-10-06: someone testing a backup should not pay for a full rotation of a large holding). The question is put once; `n` leaves the notes as they are and `note rotate all` rotates them whenever wanted. Without a node the wallet opens offline and says the offer is waiting.

**Caveat found while validating this**: if the *source* wallet the backup came from is still active and mid-spend, a rotation batch that needs a fee-stamp from a note the source wallet is simultaneously spending fails for that batch; the other batches still run and the failed one is reported. Pause the original first.

## 11b. Automatic backups to Telegram, and the restore (2026-09-23/24)

The wallet keeps an encrypted copy of every wallet on the computer in Telegram, and keeps it current by itself — the way a phone backs itself up to its maker's cloud, except that the only servers involved are Telegram's and they hold nothing they can read. It needs the wallet's bot (`telegram link <token>`, the token from @BotFather) paired with its owner; the backups go to the bot's own chat with them, the same chat the payment codes arrive in — one chat, so a restore is "forward everything from the last dashed line to the end" to the very bot that posted it.

Once the bot is paired and nothing has been backed up yet, the wallet asks — once, at the next `open` in the terminal, as a banner in the desktop wallet — "Back up your wallets automatically to your Telegram bot chat? (highly recommended)". Yes posts the first full copy and the automation runs from then on; "not now" is remembered ("Run 'telegram backup' later if you change your mind") and the commands below remain.

```
telegram                        # where things stand: paired with whom, the daily limit, the backup state
telegram backup                 # the first time: posts a full copy and starts the automatic backups; later: a full copy now
telegram cover                  # a wallet made before 2026-10-06 and not opened since: type its 24 words once, so the backups cover it
telegram autobackup off / on    # pause and resume the automatic posts
telegram link <token> · unlink · limit <amount> · pin · unlock · code
telegram home <group id>        # one bot, several wallets: a group where the bot answers this wallet too (see below)
```

**One bot, several wallets (2026-10-06).** Telegram allows twenty bots per account, so a wallet per bot does not scale; one bot can serve several wallets of the same computer, opened one at a time (a bot token has one poller — two wallets open at once with the same bot are told so and take turns). Since every backup covers every wallet of the folder (below), the backups need nothing more: whichever wallet is open keeps the one chat current for all of them. Answering is another matter — a payment code in a chat should come from one wallet — so the bot's private chat answers for the wallet that paired first (recorded in `telegram-homes.json` beside the wallets), and a second wallet linked to the same bot is told, when it pairs, that the bot answers it in a group of its own if wanted: make a group, add the bot, then `telegram home <group id>`. From then on the bot answers that wallet there and that wallet's backups go there too, with every other wallet's; the restore instruction stays "forward everything from the last dashed line in this chat". A bot that is an admin of the group may tidy at any age, not just within two days. `telegram home none` drops the group again.

`telegram` is the one command for the bot since 2.70 (2026-09-27); it replaced `mobile telegram …`, and the note mirror that the bot superseded is gone. The old spellings `backup telegram` and `wallet restore telegram` still work.

The first run posts a **checkpoint** — every covered wallet's keys file and note files — straight away. From then on, while the wallet is open, it posts by itself: a **delta** holding only the files that changed since the last post (and the names of any removed), once a vault has been quiet for two minutes and at most every ten; a fresh checkpoint once a day, or sooner when the deltas since the last one outweigh half of it; and any change still unposted goes out at `close`. Nothing is asked: each wallet's part is sealed to a key its own 24 words name, which the owner keeps anyway and which bring that wallet back on any machine. What was last posted to a chat — paths and digests of every wallet's files — is kept in `telegram-backup-<bot>-<chat>.json` beside the wallets, which is how a delta knows what changed and how two wallets sharing one bot keep one chat current between them; whether a wallet's automatic backups are on is its own setting, in `telegram-backup.json` inside its directory.

**Every wallet, each under its own words (2026-10-06).** A backup covers every wallet in the folder, not only the open one: a reserve wallet that is never opened is exactly the one a lost disk would take for good (founder: "backups should back up all the wallets, not just the one being automated"). Each wallet's 24 words also name a key pair — the private half is a hash of the words, derived when needed and never stored; the public half, 33 bytes, sits in the clear in `<name>.wallet/backup.pub`, written when the wallet is made (terminal wizard and desktop app alike) and at every open. A backup seals each wallet's files to its public half with a fresh ephemeral key (ECDH on secp256k1, XChaCha20-Poly1305 under a domain-separated hash of the shared secret), so one bundle holds every wallet and each part opens with its own words alone — the open wallet's words never stand in for another's (founder: "I find it weird that the whole bundle sits behind one 24 set, when each one has its own"). The bundle names its wallets in the clear, which a restore needs in order to ask for the right words; balances and keys stay inside the encryption as before. A wallet from before this change has no key until it is opened once with its password — the wallet says so at that open — or its words are typed once: at `open`, when the bot has somewhere to post and a wallet of the folder is still without a key, the wallet explains that the backups now cover every wallet and asks for that wallet's 24 words (Enter skips it until it is opened; `never` leaves it out, undone by `telegram cover`); the words are checked against one of the wallet's note files, since the vault key is the words' entropy, and a wallet without a note yet cannot be checked and is covered when opened instead. `view` at that prompt takes the wallet's password instead (checked against its keys file first), shows its words and uses them. A wallet from before wallets had 24 words at all — made before August 2026 — gets them made at that moment, at `view` or at its own open, and they are shown once with the usual advice (founder, 2026-10-07: "there is no 24 words to see, just create them"). `telegram` and `telegram backup` say which wallets are covered and which are still waiting. The file backup, `wallet backup [<file>]`, writes the same bundle (the open wallet's part is read back with its words before the file is trusted), and `wallet backup verify <file>` and `wallet restore <file> [<name>]` ask for each wallet's words in turn, Enter skipping one; a backup from before 2026-10-06 — one wallet behind one key — still opens with its words (or, before 2026-09-24, its passphrase).

Every backup message is delivered silently — no sound, no badge — so the chat stays quiet unless it is opened. Only the last two full copies, with the deltas after each, stay in the chat: when a new full copy is posted the wallet takes the third-newest down. Telegram lets a bot delete its own messages for two days after posting and refuses afterwards (measured 2026-10-06), which is why the full copy is daily and not weekly; a copy left over from a wallet closed for days stays, and the wallet says so. The bot also takes down the codes it posts — a request once paid or lapsed, a hand-over once the notes are taken or came back from a lock — so bearer value does not sit in the chat history; an untaken hand-over keeps its code, which may be the only copy. A dashed line is posted before every full copy, so a restore is one instruction: forward everything from the last dashed line to the end. What lands there, per backup: one plain line saying what it is ("Marigold backup of the wallets 'marigold' and 'reserve' on this computer: a full copy, 2026-10-06 11:17", or "change 3 after the full copy of …") and the file, named so that anyone can read it — `Marigold backup - marigold, reserve - 2026-10-06 11.17.46 +0200 - full.mgb`, `… - change 3.mgb` (the wallet names while they fit in forty characters, "4 wallets" otherwise) (the time is the machine's own, with its UTC offset written in; the wallet keeps the identity in UTC underneath so the newest copy sorts newest on any machine), with " (part 1 of 3)" before the extension only when an archive had to be split. Parts stay under 20 MB because that is the most a bot may fetch back. A wallet of a few hundred notes is a few hundred kilobytes, so a checkpoint is one part and a delta far less; a week never needs more than the newest checkpoint and the deltas after it forwarded back.

To bring a wallet back on any machine:

```
telegram restore [<name>]
```

**A folder as well (2026-10-09).** `backup folder <path>` keeps the same backups current in a folder on this computer — the idea being one a cloud service mirrors (Dropbox, iCloud Drive, OneDrive, Nextcloud), so the copy is off the machine the moment it is written. The folder gets the same files the chat gets, named the same (`Marigold backup - <wallets> - <stamp> - full.mgb`, `… - change N.mgb`), written beside and renamed into place so a mirroring service never sees a half-written file, with the same two-full-copies retention: the older copy's files are deleted when a new full copy is written. The founder's first thought was one growing archive with the changes appended; a folder of files is better for a mirror (a change is one small new file to carry, not a rewrite of the whole), safer (a damaged file costs one change, not the lot) and tidier to prune. Setting the folder writes the first full copy at once; `backup folder off` stops it and leaves the files; `backup folder` shows the state. Telegram and the folder are two *destinations* of one run (`tgbackup::Destination`), each with its own index beside the wallets (`folder-backup-<hash of path>.json`), and the housekeeping tick and `close` serve both. `wallet restore <folder>` restores from the newest full copy and the changes after it, exactly as the forwarded chat does, asking for each wallet's words. The desktop wallet has the folder on its Backup tab and "Restore one from a backup folder" on its opening screen.

The desktop wallet has the same on its **Backup** tab: where the backups go, on or off, the last full copy and the change sets since, the wallets covered; "Back up now"; a "Wallets waiting for their words" box when a wallet of the folder has no key yet (pick it, type its words, Cover it); the bot's setup (token and PIN, then the pairing code to send it) when there is none yet; and "Save a backup file", which writes the same bundle into Downloads. Its opening screen has "Restore one from Telegram": token and the words of the wallet wanted first, then the forward; the words open that wallet's part, and the answer names the others in the backup, each restored by running it again with its own words. Both wallets run the bot and the automatic backup while the wallet is open.

It asks for the bot's token hidden (a token typed on the command line would sit in the terminal and its history); then forward the bot everything from the last dashed line in the backup chat to the end: select those messages, forward, pick the bot. Order does not matter, and extra files do no harm — the wallet takes the newest checkpoint and the deltas after it and ignores the rest. It goes on a few seconds after the last part, fetches each, says which wallets the backup holds and asks for **the 24 words of each** in turn (Enter skips one; words that belong to another wallet of the backup are used for that one; a backup from before 2026-10-06 held one wallet and asks once, and one from before September 2026 opens with its passphrase instead) — merges the checkpoint and the deltas in order per wallet (it refuses if a delta in the middle is missing and says which), and runs the ordinary restore for every wallet opened, each under its own name (the `<name>` given applies when one wallet is restored). A pre-checkpoint archive sealed with a passphrase (from before 2026-09-24) restores the same way with its passphrase. A bot cannot read a chat's history, which is why the parts have to be forwarded to it. The Telegram bot API is the whole dependency; nothing is stored on any server of ours.

## 12. Notes-only wallets

Answer `n` to `Keep a ledger account too?` and the wallet has a vault and nothing else: no account key, no ledger address ever derived, `list` shows no account and the prompt carries no account name. Everything about notes works exactly as above — `request`, `pay`, `receive`/`export`, `note pos`, `words`, `wallet verify`, `wallet paper export`/`import`, `note history` — because none of it ever needed the ledger; it only used the account as a handle. `balance` shows notes alone. The ledger commands (`mint`, `redeem`, `transfer`, `sweep`, `estimate`, `address`, `utxos`, `message sign`) refuse with one line: *This wallet keeps notes only — there is no ledger account. 'account create bip32' adds one.* That command attaches the ledger at any later time, derived from the same 24 words, so there is no new secret and backups need nothing extra.

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
