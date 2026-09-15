# P8.0b verify — a wallet that keeps notes only

The pexpect driver that runs FORK-PLAN P8.0b's verify against the live public testnet. Not CI: it moves real testnet notes on rpc1 and needs a payer wallet that already holds some (P8.0's wallet B, restored from the faucet claim, is the one it was written against).

Run from the repo root with a release `marigold-cli` built (`scripts/remote-build.sh`):

```
python3 scripts/p80b-notes-only-verify/verify.py <scratch-dir> <payer-HOME> <fresh-HOME-for-pure> <fresh-HOME-for-again>
```

What it asserts, in order: the wizard with `n` to `Keep a ledger account too?` asks for no account title and no bip39 passphrase and prints no ledger address; `list` shows no account; `note mint`, `address`, `sweep`, `note redeem` each refuse with the `account create bip32` line; `balance` has no ledger row; `note request` is paid by the payer's `note pay`; `exchange` far over the notes' cover is refused, `exchange` covered to within one 0.01 note pays the payer's ledger address straight from notes, and `exchange` beyond what the notes hold is refused as a shortfall; `note vault backup` then `note vault restore` into a second notes-only wallet made from the same 24 words brings the remaining note back, still with no account and no address ever printed; and the payer received the deposit — as a rise in its notes, since a ledger wallet's auto-mint turns a deposit into notes within the minute, or as a ledger row when auto-mint is off. The verdict lands in `<scratch-dir>/verdict.json`; `"pass": true` is a pass.

First passed 2026-09-15 (build 2.0.83): 4 notes paid, payout from 3 notes with fee 0.00945945, 1 note restored (0 stale, 0 corrupted), payer's notes 7.64 → 8.70 after the deposit.
