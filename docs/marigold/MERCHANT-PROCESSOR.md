# Taking Marigold at the till: the processor's integration

**Status: integration contract, draft v0, 2026-10-13.** Founder's direction: in each market an exchange converts at the instant of sale, so a shop takes Marigold without ever holding it. This page is for that exchange's engineer. It says what the chain gives you, the flow step by step, what "paid" and "final" mean in numbers, what to run, and what is still on the list. The ledger-side integration for deposits and withdrawals is a separate page, [EXCHANGE.md](EXCHANGE.md); the normative text of the pool is [POOL-SPEC.md](POOL-SPEC.md). Everything here is on the public testnet today unless marked as on the list.

## The model in one paragraph

The shop's till asks you for a bill. You quote the Marigold for the euro amount at your own rate and spread, make a payment request for that amount with a key you hold, and hand the till a code to show. The customer scans it, their wallet checks it and pays it, and the notes land under your key within a second or two. You see them land, you credit the shop in euros at the rate you quoted, and about a minute later the block is anchored and the payment is final in the strict sense. The price risk between your quote and your conversion is yours and is priced into your fee; the shop sees euros and nothing else; you are the licensed party, as a card acquirer is. The chain records a payment to a key, not a shop and not a customer.

## What the chain gives you

**A bill is a request code.** `marigoldreq:` followed by 112 bytes: a 32-byte request key, the amount in petals (8 bytes, zero for an open request), an expiry as unix seconds (8 bytes, zero for none) and a 64-byte BIP340 signature by the request key over the first 48 (DECISIONS, 2026-09-20). One fresh key per bill, held by you, never reused. The customer's wallet verifies the signature and the expiry before it shows anything and refuses an altered or expired code with the reason, so a code on a screen cannot be swapped for another amount or another key. The petal is the smallest unit, 100,000,000 to a coin.

**Payment lands under your key, and the customer pays the fee.** Paying a request is one pool transfer: the customer's wallet consumes its notes and produces notes of the exact amount under the request key, with change back to itself, and attaches the fee stamp. The notes are yours the moment the transfer is in a block, under a key only you hold; no rotation, no sweep and no fee on your side (DECISIONS, "Paying, as a person would put it", 2026-09-15). A usual payment costs the customer one penny, 0.01 MAGLD.

**You see it land by subscription.** `NotifyNotesChanged` on the node's wRPC, with the request key in `pks`, delivers every note produced under that key as it enters the pool; summing their denominations against the amount is the whole check. The wallet does exactly this (`await_payment_request` in `wallet/core/src/account/notepool.rs`); a light wallet does the same against its own copy of the pool with nothing subscribed. `GetNotesBySerial` answers a lookup by serial for a receipt.

**Final means anchored.** Five trustees co-sign a block every 300 DAA scores, about thirty seconds, and consensus refuses any chain that conflicts with the latest valid anchor. A payment whose block is at or below the latest anchor (`GetFinalityAnchorStatus`) cannot be reversed; the anchored block sits about a minute behind the tip. If the status reports the anchors stale, the fail-open case that has only ever happened on the testnet when the signers were restarted, fall back to the merge depth, 36,000 blocks or about an hour. The full policy is in [EXCHANGE.md](EXCHANGE.md), "Confirmations".

**A receipt is a pointer, not a proof.** After paying, the customer's wallet prints `marigoldreceipt:` followed by 72 bytes: the transaction id, the request key and the amount in petals. Nothing in it is trusted: you fetch the transaction, check that the notes it produced sit under the request key and that they sum to the amount. It exists so a web cart can find a payment the instant the customer pastes it instead of polling; a till that is subscribed does not need it.

**The money is then yours to move.** Notes under a bill's key are ordinary notes. Consolidate them into your treasury keys with a transfer, or send them to a ledger address of yours with `exchange <address> <amount>`, which redeems notes to the ledger in one transaction priced by mass; the ledger is where your exchange books already live.

## The flow, step by step

1. **The till asks for a bill.** The shop's till sends you the euro amount, a reference of its own and nothing else.
2. **You quote and make the request.** Convert at your rate, round to the penny of MAGLD (notes come in 0.01, 0.1, 1, 10, 100, 1,000, 10,000 and 100,000; the customer's wallet makes change, you never do), make a request for that amount with a fresh key and a short expiry, ten minutes is plenty, and sign it. Return the code and its QR to the till. Keep the key, the amount, the rate and the shop's reference together.
3. **The customer pays.** Their wallet shows the amount and asks them to confirm; one transfer, their fee. From the moment they confirm, the notes are in a block within a second or two.
4. **You see it land.** The subscription on the request key reports the notes; when their sum reaches the amount, the bill is paid. Tell the till at once: this is the state a shop assistant sees, and it arrives before the customer has put the phone away.
5. **You credit the shop** in euros at the rate you quoted, on your own books, at the instant you choose: at paid, for the amounts you are willing to carry for a minute, or at final.
6. **The block is anchored** about a minute later and the payment is final. Settle the shop as your terms say, daily or at once.
7. **The notes join your treasury.** On your own cadence, consolidate the notes of the day's bills into your own keys or redeem them to your ledger address.

Expiry is a one-way door: an expired code is refused by every wallet, so a bill that was never paid is simply closed. A wallet pays a pinned request for its amount; whatever lands under the key beyond it is nonetheless yours, and a refund is a matter for your terms. A request with no amount is an open request, paid with whatever the customer enters; a till never needs one.

## The numbers

| What | Value |
| --- | --- |
| Request code | 112 bytes: key 32, amount 8, expiry 8, signature 64 |
| Receipt | 72 bytes: transaction id 32, request key 32, amount 8 |
| Default request lifetime | one day; `for 10 minutes` sets it per request |
| Fee on a usual payment | 0.01 MAGLD, paid by the customer |
| Fee on your side at payment | none |
| Paid: notes in a block | a second or two after the customer confirms |
| Final: block at or below the latest anchor | about one minute |
| Anchor cadence | every 300 DAA scores, about thirty seconds |
| Fallback when anchors are stale | merge depth, 36,000 blocks, about one hour |
| Smallest note | 0.01 MAGLD; quote to the penny |
| Testnet ticker and prefix | TMAGLD, `marigoldtest:` |

## What to run

**A node of your own.** `marigoldd`, pruned is fine; the testnet is testnet-10, seeded by `tn-seed1`–`tn-seed3.marigold.cash`. The public `wss://rpc1.marigold.cash` is a stopgap for trying things, not a dependency. Build against a release tag of [marigoldcash/marigold](https://github.com/marigoldcash/marigold), never the tip; the RPC is frozen from v2.87.343 until genesis, changes are additive only, and the calls on this page keep their names and fields ([EXCHANGE.md](EXCHANGE.md), "The RPC").

**The wallet as a library.** The wallet core is a Rust crate in this repository, `wallet/core`, and its `notepool` module is the integration surface today: `create_payment_request` makes and signs a request, `await_payment_request` watches a key until the amount has landed, `pay_payment_request` is what the customer's side runs, and `PaymentReceipt` decodes a receipt. A service that opens a wallet, makes requests and watches them is `cli/src/serve.rs`, the same service the Telegram bot and the desktop wallet run on; its `request` and `await_request` are the two calls a processor needs.

**The wallet as a program, for a trial.** `marigold-cli` does the whole flow by hand: `request 1 for 10 minutes` prints the code and waits, `history` lists what was paid and received, `exchange <address> <amount>` sends to a ledger address. The desktop wallet shows the same request with its QR and marks it paid when it lands.

**A sidecar, on the list.** The shape the project uses for integrators is a small HTTPS service beside your node, the way the anchoring gateway is ([ANCHORING-GATEWAY.md](ANCHORING-GATEWAY.md)): it holds your wallet, and the till speaks JSON to it. The intended contract, so a till can be written against it now:

```
POST /v0/bills        { "amount_petals": 123000000, "reference": "till-7/4411", "expires_in": 600 }
                   -> { "key": "<hex 32>", "code": "marigoldreq:...", "expires_at": 1760360000 }
GET  /v0/bills/{key}  -> { "state": "open" | "paid" | "final" | "expired",
                           "received_petals": 123000000, "txid": "<hex>", "block": "<hex>", "anchored": true }
POST /v0/receipts     { "receipt": "marigoldreceipt:..." } -> the same record as GET
```

plus a webhook per state change. It is a wrapper over the two library calls and the anchor status, not a fourth RPC surface on the node, and it is not built yet. Until it is, the library is the contract.

## What you learn, and what you do not

You hold the key of every bill, so you know which shop was paid how much and when, as any acquirer does; the chain does not, since a request key is a key and nothing more. You do not learn who paid: a note's history is a chain of keys with no account behind them, and the customer's wallet names itself to nobody. Keep request keys and their mapping to shops as you keep card data; they are the one place a shop's turnover is readable.

## Try it on the testnet today

Two wallets on two machines, or two named wallets on one. Fund the paying one from the faucet at marigold.cash/app. On the wallet that plays the processor, `request 2 for 10 minutes` prints a code; on the other, `pay <code>` shows the amount, asks, pays and prints a receipt; the first wallet reports "Paid" within seconds and `history` shows the line. `GetFinalityAnchorStatus` on your node, a minute later, shows an anchor at or past the block. That is the entire flow; the sidecar adds nothing to it but a JSON surface.

## On the list

- **The merchant's check of a receipt and a JavaScript SDK** (PLAN P8.0i): `request check <receipt>` in the wallet, and the same check as a function for carts; the receipt format on this page is the contract for carts written without it.
- **The sidecar** with the contract on this page, and the test flow against testnet-10 that proves a till written against it.
- **Paying a customer out as notes** (PLAN P8.0e): the reverse direction, your ledger funds minting straight to the customer's request key in one transaction, so a customer who sells Marigold to you, or buys it from you, pays nothing to receive.
- **A label on a request.** A request carries no text, so the customer's wallet shows the Marigold amount and the till shows the euros. A reference or a currency amount in the code would let the wallet show "10.00 EUR" beside the Marigold; it is an additive change to the code and is not decided.
- **A rate for display.** For a sale, your quote is the rate. For a wallet showing a balance in euros, a rate published on the chain in a signed lane is the shape that fits; who signs it is a founder decision and nothing on this page depends on it.
