# P8.0 recovery verify — "one secret, words alone recover the ledger, words + files recover the notes"

The pexpect pair that runs PLAN P8.0's verify against the live public testnet. Not CI: it claims real bearer notes from faucet.marigold.cash (one claim per IP per hour) and rotates on rpc1.

Run from the repo root with a release `marigold-cli` built (`scripts/remote-build.sh`):

```
python3 scripts/p80-recovery-verify/wallet_a.py <scratch-dir> <fresh-HOME-for-A>
python3 scripts/p80-recovery-verify/wallet_b.py <scratch-dir> <fresh-HOME-for-B>
```

`wallet_a.py` creates a wallet (asserting exactly one recovery ceremony), imports the three faucet notes, lets them confirm, backs the vault up to `<scratch-dir>/backup`, and writes the 24 words and ledger address to `wallet_a.json`. `wallet_b.py` creates a blank wallet **from those words only**, asserts it derived the same ledger address, then runs `note vault restore <backup> <words>` and asserts the notes came back. The verdict lands in `wallet_b.json`; `ledger_rederived: true` with `recovered_live` equal to A's note count is a pass.

First passed 2026-09-14 (build 2.0.67): address identical, 7 live / 0 stale / 0 corrupted, restore-rotation completed.
