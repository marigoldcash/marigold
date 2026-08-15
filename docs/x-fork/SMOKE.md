# SMOKE.md — Full user-journey manual test

Documented manual smoke test for P4.2. Walks a full "user journey" end to end on the
[P4.1 local testnet](../../scripts/x-testnet-local.sh): create a wallet, mine to it, wait for
maturity, send to a second wallet, restart a node, confirm balances persist.

`kaspa-cli` is REPL-only and cannot be scripted (see NOTES.md's P0.3 entry) — every step below
uses RPC-direct commands (`rothschild` for keypair generation and sending, `kaspa-miner` for
mining, a small throwaway gRPC client for balance/DAG queries) instead. This mirrors every prior
live check in this project (P0.5, P2.9, P3.3, P4.1) and is the supported way to exercise the
chain without a working interactive wallet.

**Before you start**, rebuild every binary you'll use — a stale binary can silently pass with
wrong behavior (bit the project once at P2.3, and again at P4.2 itself — see "Gotchas" below):

```bash
cargo build --release --bin kaspad --bin rothschild
```

`kaspa-miner` (the community `elichai/kaspa-miner` tool) must be installed separately at
`~/.cargo/bin/kaspa-miner` — see NOTES.md's Environment section.

## Steps

### 1. Launch the local testnet

```bash
./scripts/x-testnet-local.sh
```

Confirms 3 peered devnet nodes are up (node1 on default ports: gRPC `26610`, P2P `26611`).

### 2. Create wallet A

```bash
target/release/rothschild --network devnet --rpcserver 127.0.0.1:26610
```

No `--private-key` given, so rothschild generates and prints a keypair + `marigolddev:...`
address, then exits (nothing to spend yet). Save the private key and address — this is "wallet
A."

### 3. Mine to wallet A

```bash
kaspa-miner --mining-address <wallet-A-address> --kaspad-address 127.0.0.1 --port 26610 \
  --threads 2 --mine-when-not-synced
```

### 4. Wait for maturity, check balance

Coinbase outputs need `coinbase_maturity * 2` confirmations to be spendable (a rothschild-side
check, not a consensus rule — see "Gotchas"). At 10 BPS, `coinbase_maturity = 1000`, so wait
until the virtual DAA score is at least ~2000 past when you started mining, then stop the miner
and check:

```bash
# any RPC client works; this project's throwaway gRPC example is the established pattern —
# see NOTES.md's P0.3/P0.4 entries for how to wire get_balance_by_address into it temporarily
```

Balance should be `(blocks mined) × 15,228,085` petals (the P3.2 table's month-0 per-block
value) — see P3.3's entry in NOTES.md for the exact reasoning.

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

Leave the miner running (or restart it, still mining to wallet A) so the mempool transactions
actually confirm — rothschild alone only submits to the mempool, it doesn't mine. Let it send a
few transactions (a few seconds at `--tps 1`), then stop it (`pkill -x rothschild`).

### 7. Confirm wallet B's balance

Query wallet B's balance over RPC — should be nonzero, matching what rothschild's own log
reported sending (`Tx rate: ... avg UTXO amount: ... avg outs per tx: 2` — half the value per
tx typically returns to the sender as change, half goes to the recipient).

### 8. Restart a node, confirm balances persist

```bash
kill <node1-pid>          # or Ctrl-C if run in foreground
target/release/kaspad --devnet --enable-unsynced-mining --utxoindex --appdir=<node1's appdir>
```

Same `--appdir` as before — the node reloads its existing database rather than starting fresh
(no "Resyncing the utxoindex..." log line on restart, unlike a first-ever launch). Query both
wallet A and wallet B's balances again — both must exactly match their pre-restart values.

## Gotchas found while writing this (2026-08-15)

- **`rothschild` needs rebuilding just like `kaspad`** — the exact same stale-binary trap
  documented for `kaspad` at P2.3 bit this step too. A `rothschild` binary built before the
  P2.1 address-prefix rebrand printed `kaspadev:...` addresses instead of `marigolddev:...`
  (silently wrong, not an error) until rebuilt. Always rebuild every binary you're about to use
  for a live check, not just the one you most recently edited.

- **A real, non-obvious bug: `rothschild`'s hardcoded `DEFAULT_SEND_AMOUNT` (originally
  `10 * SOMPI_PER_KASPA`, i.e. "10 KAS") could never be satisfied from Marigold's own coinbase
  UTXOs.** `select_utxos()` caps input-combining at `MAX_UTXOS = 8`; Marigold's genesis-era
  per-block reward is only 15,228,085 petals (~0.152 MAGLD), so 8 combined coinbase UTXOs sum to
  ~1.2 MAGLD — nowhere near a 10 MAGLD send target. Every send attempt failed with
  `"Has not enough funds"` regardless of how long you'd mined (this is a hard cap, not a timing
  issue — more mining just creates more same-sized UTXOs, never bigger ones, while the schedule
  sits in one flat month). Fixed by lowering `DEFAULT_SEND_AMOUNT` to `1 * SOMPI_PER_KASPA`
  ("1 MAGLD," ~7 blocks' worth at genesis — the same proportional margin Kaspa's original
  constant had relative to its own genesis-era reward). Own commit; see `rothschild/src/main.rs`.

## Verification (2026-08-15)

Walked the full script above once, end to end, on the P4.1 local testnet:

- Wallet A funded via ~200 mined blocks; balance `65,847,603,983` petals confirmed mature.
- Wallet B created, funded via 1 send (`3,485,867,022` petals, 66 UTXOs).
- Stopped node1, restarted it with the same `--appdir` — **both balances matched exactly**
  (`65,847,603,983` and `3,485,867,022` petals, unchanged), and the virtual DAA score also
  matched exactly (`4554`, since no blocks were mined during the brief stop/restart window).

Every step passed. Full narrative log (including the two bugs found and fixed along the way)
in [NOTES.md](NOTES.md)'s P4.2 entry.
