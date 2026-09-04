# SMOKE.md — Full user-journey manual test

Documented manual smoke test for P4.2. Walks a full "user journey" end to end on the [P4.1 local testnet](../../scripts/marigold-testnet-local.sh): create a wallet, mine to it, wait for maturity, send to a second wallet, restart a node, confirm balances persist.

`marigold-cli` is REPL-only and cannot be scripted (see NOTES.md's P0.3 entry) — every step below uses RPC-direct commands (`rothschild` for keypair generation and sending, `kaspa-miner` for mining, a small throwaway gRPC client for balance/DAG queries) instead. This mirrors every prior live check in this project (P0.5, P2.9, P3.3, P4.1) and is the supported way to exercise the chain without a working interactive wallet.

**Before you start**, rebuild every binary you'll use — a stale binary can silently pass with wrong behavior (bit the project once at P2.3, and again at P4.2 itself — see "Gotchas" below):

```bash
cargo build --release --bin marigoldd --bin rothschild
```

`kaspa-miner` (the community `elichai/kaspa-miner` tool) must be installed separately at `~/.cargo/bin/kaspa-miner` — see NOTES.md's Environment section.

## Steps

### 1. Launch the local testnet

```bash
./scripts/marigold-testnet-local.sh
```

Confirms 3 peered devnet nodes are up (node1 on default ports: gRPC `26610`, P2P `26611`).

### 2. Create wallet A

```bash
target/release/rothschild --network devnet --rpcserver 127.0.0.1:26610
```

No `--private-key` given, so rothschild generates and prints a keypair + `marigolddev:...` address, then exits (nothing to spend yet). Save the private key and address — this is "wallet A."

### 3. Mine to wallet A

```bash
kaspa-miner --mining-address <wallet-A-address> --kaspad-address 127.0.0.1 --port 26610 \
  --threads 2 --mine-when-not-synced
```

### 4. Wait for maturity, check balance

Coinbase outputs need `coinbase_maturity * 2` confirmations to be spendable (a rothschild-side check, not a consensus rule — see "Gotchas"). At 10 BPS, `coinbase_maturity = 1000`, so wait until the virtual DAA score is at least ~2000 past when you started mining, then stop the miner and check:

```bash
# any RPC client works; this project's throwaway gRPC example is the established pattern —
# see NOTES.md's P0.3/P0.4 entries for how to wire get_balance_by_address into it temporarily
```

Balance should be `(blocks mined) × 15,228,085` petals (the P3.2 table's month-0 per-block value) — see P3.3's entry in NOTES.md for the exact reasoning.

### 5. Create wallet B

```bash
target/release/rothschild --network devnet --rpcserver 127.0.0.1:26610
```

Same as step 2 — a second generated keypair + address, "wallet B." Don't fund it directly.

### 6. Send from wallet A to wallet B

```bash
target/release/rothschild --network devnet --rpcserver 127.0.0.1:26610 \
  --private-key <wallet-A-private-key> --to-addr <wallet-B-address> --tps 1
```

Leave the miner running (or restart it, still mining to wallet A) so the mempool transactions actually confirm — rothschild alone only submits to the mempool, it doesn't mine. Let it send a few transactions (a few seconds at `--tps 1`), then stop it (`pkill -x rothschild`).

### 7. Confirm wallet B's balance

Query wallet B's balance over RPC — should be nonzero, matching what rothschild's own log reported sending (`Tx rate: ... avg UTXO amount: ... avg outs per tx: 2` — half the value per tx typically returns to the sender as change, half goes to the recipient).

### 8. Restart a node, confirm balances persist

```bash
kill <node1-pid>          # or Ctrl-C if run in foreground
target/release/marigoldd --devnet --enable-unsynced-mining --utxoindex --appdir=<node1's appdir>
```

Same `--appdir` as before — the node reloads its existing database rather than starting fresh (no "Resyncing the utxoindex..." log line on restart, unlike a first-ever launch). Query both wallet A and wallet B's balances again — both must exactly match their pre-restart values.

### 9. Launch the 3-node simnet testnet

Steps 1-8 above (P4.2) predate the note-pool wallet and use devnet + RPC-direct tools. Everything from here on (P7.8) exercises the **full note lifecycle** — minting, cross-node propagation, a mid-flow restart, and vault backup/restore — using the real interactive `marigold-cli` (see [WALLET.md](WALLET.md) for the single-node command-by-command reference; this section builds on it across 3 peered nodes) on **simnet**, not devnet: `DEVNET_PARAMS` sets `pool_activation: ForkActivation::never()`, so no `note` command would even be accepted there — see WALLET.md's "Before you start" note.

```bash
cargo build --release --bin marigoldd --bin marigold-cli
NETWORK=simnet ./scripts/marigold-testnet-local.sh
```

(P7.8 taught the [P4.1 script](../../scripts/marigold-testnet-local.sh) a `NETWORK=simnet` mode; it now also starts `--rpclisten-borsh` and `--unsaferpc` on every node, both required for `marigold-cli` to connect — neither was on by default before P7.7 found the gap.) Confirms 3 peered simnet nodes: node1 on gRPC `26510`/P2P `26511`/borsh-wRPC `27510`, node2 and node3 shifted by `+10`/`+20` on each port.

**`marigold-cli` cannot be driven by piped stdin** (NOTES.md's P0.3 finding) — every step below assumes a real TTY, or `pexpect` (a Python pty-driving library) sending literal `\r` for Enter, matching WALLET.md's own validation method.

### 10. Create and fund a wallet on node 1

In a `marigold-cli` session pointed at node 1 (`server 127.0.0.1:27510`), run `wallet create` (WALLET.md step 3), then mine to its receive address:

```bash
kaspa-miner --mining-address <address> --kaspad-address 127.0.0.1 --port 26510 \
  --threads 2 --mine-when-not-synced
```

Leave it running past `coinbase_maturity * 2 ≈ 2000` blocks (simnet's 10 BPS, same rule as WALLET.md step 4), then `list` to confirm a mature transparent balance, and:

```
note mint 5
```

### 11. Confirm cross-node pool-state agreement

Every node independently derives the same note-pool state from the same blocks — mine one more block so the mint transaction confirms, then compare `get_pool_stats()` (per-denomination live-note counts) and `get_block(sink).header.pool_commitment` (the pool SMT root) across all three nodes' gRPC endpoints (`26510`/`26520`/`26530`). Both must match exactly on every node — this is the same agreement check P6.10's `daemon_notepool_multi_node_agreement_test` makes at the consensus layer; here it's confirmed from the wallet side outward.

### 12. Cross-node payment: request on node 2, pay from node 1

Open a **second** `marigold-cli` session under a separate `$HOME` (so it gets its own wallet storage), pointed at node 2 (`server 127.0.0.1:27520`), and create a wallet there too. From that session:

```
note request 2
```

Copy the printed `marigoldreq:...` payload to the node-1 wallet session and pay it:

```
note pay marigoldreq:<...>
```

Mine a block (on either node — they're peered) so the payment confirms. The node-2 session's `note request` returns on its own, without any command run there — the confirming block propagates over P2P to node 2, node 2's consensus layer processes it, and the wallet's `NotesChanged` subscription (P6.9) fires locally in response to node 2's own view of the chain, not anything pushed from node 1's wallet directly.

### 13. Restart a node mid-flow

Kill node 2 (`kill <node2-pid>`) while its wallet session from step 12 is still open, then restart it with the same `--appdir` (same pattern as step 8, but simnet ports):

```bash
target/release/marigoldd --simnet --enable-unsynced-mining --unsaferpc --utxoindex \
  --listen=127.0.0.1:26521 --rpclisten=127.0.0.1:26520 --rpclisten-borsh=127.0.0.1:27520 \
  --rpclisten-json=127.0.0.1:28520 --addpeer=127.0.0.1:26511 --appdir=<node2's appdir>
```

Confirms clean re-sync (no "Resyncing the utxoindex..." line — same appdir, no fresh start) and peers again with node 1. Re-run step 11's cross-node agreement check — `get_pool_stats()` and `pool_commitment` must still match across all three nodes once node 2 has caught back up. The wallet session connected to node 2 needs a fresh `connect` after the restart (the old wRPC socket is gone), but its local note index is unaffected — `note list` shows the same notes as before the restart once reconnected.

### 14. Vault backup, and restore (with idempotent retry) from a third node

From the node-1 wallet (which now holds several notes from steps 10-12):

```
note vault backup <dir>
```

In a **third**, fresh `marigold-cli` session under its own `$HOME`, pointed at node 3 (`server 127.0.0.1:27530`), create a new wallet and restore from that backup:

```
note vault restore <dir> <word1> ... <word24>
```

**This can legitimately fail partway through and need a retry — that's expected, not a bug.** The default restore-time rotation batches recovered notes into several transactions; if the *source* wallet (node 1's, from step 10-12) is still mid-spend, a batch whose fee-stamp lands on a note the source wallet is simultaneously consuming fails with `does not exist in the pool` (POOL-SPEC.md's same-key-in-two-wallets hazard — see WALLET.md step 11's caveat). Mine a confirming block for the source wallet's pending transactions and re-run the exact same `note vault restore <dir> <word1> ... <word24>` command:

```
a vault from this same restore already exists here (recognized by these words) - resuming...
```

rather than being refused. (**Real bug found and fixed this session**: the vault-exists safety check added in P7.7 couldn't originally distinguish "this is my own partially-completed restore, safe to resume" from "this is a genuinely different existing vault, refuse" — it refused *every* retry, even ones with the exact same words. Fixed by `NoteVault::words_match_existing_key` — unlocks the vault already on disk with the wallet secret at hand and compares the recovered `K` against what these words decode to, so the CLI can tell the two cases apart before deciding whether to proceed. See "Gotchas" below.) The remaining rotation batches complete on the retry; any batch still blocked by an in-flight source-wallet transaction fails again independently and can be picked up the same way once *that* transaction confirms too.

## Gotchas found while writing this (2026-08-15)

- **`rothschild` needs rebuilding just like `kaspad`** — the exact same stale-binary trap documented for `kaspad` at P2.3 bit this step too. A `rothschild` binary built before the P2.1 address-prefix rebrand printed `kaspadev:...` addresses instead of `marigolddev:...` (silently wrong, not an error) until rebuilt. Always rebuild every binary you're about to use for a live check, not just the one you most recently edited.

- **A real, non-obvious bug: `rothschild`'s hardcoded `DEFAULT_SEND_AMOUNT` (originally `10 * SOMPI_PER_KASPA`, i.e. "10 KAS") could never be satisfied from Marigold's own coinbase UTXOs.** `select_utxos()` caps input-combining at `MAX_UTXOS = 8`; Marigold's genesis-era per-block reward is only 15,228,085 petals (~0.152 MAGLD), so 8 combined coinbase UTXOs sum to ~1.2 MAGLD — nowhere near a 10 MAGLD send target. Every send attempt failed with `"Has not enough funds"` regardless of how long you'd mined (this is a hard cap, not a timing issue — more mining just creates more same-sized UTXOs, never bigger ones, while the schedule sits in one flat month). Fixed by lowering `DEFAULT_SEND_AMOUNT` to `1 * SOMPI_PER_KASPA` ("1 MAGLD," ~7 blocks' worth at genesis — the same proportional margin Kaspa's original constant had relative to its own genesis-era reward). Own commit; see `rothschild/src/main.rs`.

## Gotchas found while extending this (2026-08-18)

- **Real bug found and fixed**: `note vault restore`'s idempotent-retry path (documented in WALLET.md step 11 as "mine a confirming block and re-run — idempotent") was actually blocked by P7.7's own safety check. `note vault restore` copies the backup's files in and recovers `K` from the words *before* attempting any rotation, so a rotation-batch failure (the same in-flight-source-wallet hazard step 14 above walks through) leaves a real vault in place — but the P7.7 check that refuses to clobber "a vault that already exists" can't tell that apart from a genuinely different, unrelated vault, and refused *every* retry, even ones using the exact same words against the exact same partially-restored vault:

  ```
  this wallet already has a note vault - restoring here would overwrite its vault.key and
  strand any notes already stored under it. Restore into a fresh wallet instead.
  ```

Fixed by adding `NoteVault::words_match_existing_key` (unlocks the on-disk vault with the wallet secret already at hand, then compares the recovered `K` against what the given words decode to) and a corresponding `NoteKeyStore::vault_words_match` trait method; `cli/src/modules/note.rs`'s `vault_restore` now asks the secret first and, if a vault already exists, checks whether these words match it before deciding whether to refuse or resume. Confirmed live: the same retry that used to print the message above now prints `a vault from this same restore already exists here (recognized by these words) -   resuming...` and completes further rotation batches.

- **The `words_match_existing_key` unit test needed a fresh `NoteVault` handle for its wrong-secret assertion** — `unlock()` caches the successfully-unlocked `K` in memory and returns the cached value on a later call regardless of what secret is passed, so testing "wrong secret" against an instance that had *already* unlocked successfully with the correct one never actually re-attempted decryption. Not a production concern (a real CLI session's wallet secret is constant for its lifetime), but the test needed a second, never-unlocked `NoteVault` pointed at the same directory to genuinely exercise the wrong-secret path — same pattern this file already used elsewhere (`restore_from_words_ recovers_the_same_key_and_notes`'s `restored = make_vault(&dir)`, simulating a fresh process re-opening the same vault).

## Verification (2026-08-15)

Walked the full script above once, end to end, on the P4.1 local testnet:

- Wallet A funded via ~200 mined blocks; balance `65,847,603,983` petals confirmed mature.
- Wallet B created, funded via 1 send (`3,485,867,022` petals, 66 UTXOs).
- Stopped node1, restarted it with the same `--appdir` — **both balances matched exactly** (`65,847,603,983` and `3,485,867,022` petals, unchanged), and the virtual DAA score also matched exactly (`4554`, since no blocks were mined during the brief stop/restart window).

Every step passed. Full narrative log (including the two bugs found and fixed along the way) in [NOTES.md](NOTES.md)'s P4.2 entry.

## Verification (2026-08-18)

Walked steps 9-14 above against a real 3-node simnet testnet (`NETWORK=simnet ./scripts/marigold-testnet-local.sh`, node1/2/3 on gRPC `26510`/`26520`/`26530`, borsh-wRPC `27510`/`27520`/`27530`), driving three separate real `marigold-cli` sessions (each its own `$HOME`, via `pexpect` — see WALLET.md's "Gotchas"):

- **Wallet A** (node1, port `27510`): created, funded past `coinbase_maturity * 2`, ran `note mint 3` successfully.
- **Wallet B** (node2, port `27520`): created independently, ran `note request 1`, and — without any command run on wallet B's own session beyond the request itself — received `payment received: 1 MAGLD in 1 note(s)` once wallet A's `note pay marigoldreq:...` (run against node1) confirmed. Confirms cross-node `NotesChanged` propagation exactly as step 12 describes: node2 picked this up from its own view of the chain over P2P, not anything pushed directly from wallet A's session.
- **Cross-node pool-state agreement**, checked directly over gRPC against all three nodes at once with the throwaway `simple_client` tool's `--pool-stats`/`--sink`/`--pool-commitment` modes (same RPCs as P6.10's `daemon_notepool_multi_node_agreement_test`), after all of the above plus the vault restore/retry below had run: **all three nodes agreed exactly** — `pool stats: [7, 9, 2, 0, 0, 0, 0, 0]` on every node, same sink block (`5521e9bd...9af4d`, virtual DAA score `2315`), and the identical `pool_commitment` (`f38f293e...c8b2cdb`) for that block on every node. This is the strongest form of the step 11/13 check: taken *after* a mid-session node2 restart and a multi-batch vault restore/retry, not just after a clean run.
- **Wallet C** (node3, port `27530`), restore from wallet A's vault backup: first attempt (fresh wallet, no prior vault) recovered 9 live notes across a planned 5-batch rotation; batches 1-2 confirmed, batches 3-5 failed with `does not exist in the pool` (the documented in-flight-source-wallet hazard — wallet A was still active). A same-words retry at that point was **refused** by the pre-fix binary (see "Gotchas" above) — reproducing the bug exactly as described. After the fix and a rebuild, the identical retry command instead printed `a vault from this same restore already exists here (recognized by these words) - resuming...`, then completed 2 more batches (4 more notes rotated). The remaining 2 batches failed again on the same already-consumed-serial condition (the fee-stamp hazard is per-batch, not resolved by the retry itself — mining a confirmation for wallet A's specific in-flight transaction and retrying once more would pick up the rest, as step 14 describes). `note vault verify` afterward reported `light verify: 3 live, 4 stale`, consistent with a partially-completed rotation.

Every new step's documented behavior matched what actually happened, including reproducing the pre-fix bug and confirming the fix live. Full narrative in NOTES.md's P7.8 entry.
