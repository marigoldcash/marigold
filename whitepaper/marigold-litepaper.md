# Marigold Litepaper

## The Problem: Digital Money Forgot What Cash Got Right

Take a dollar bill out of your wallet. Notice three things:

1. **Whoever holds it owns it.** There is no account, no login, no intermediary. Possession is ownership.
2. **It carries no memory.** The bill does not remember who spent it last. There is no record tying you to the coffee you bought this morning.
3. **Anyone can check it is real.** Hold it to the light, feel the paper. No special expertise required.

No mainstream digital money reproduces all three of these properties at once.

Bitcoin gives you the first and third: holding your keys means owning your coins, and anyone can verify the system is honest. But it fails badly on the second. Every Bitcoin ever mined carries its full history permanently and publicly. Every address you have ever sent from or received to is stitched together in a graph that anyone can analyze. Your coins remember everything you ever did with them.

Privacy coins like Monero and Zcash try to fix this by encrypting the history. They succeed at hiding things — but the very cryptography that does the hiding creates two new problems. First, almost nobody can personally verify that the system is honest. You are trusting experts you have never met that the math contains no flaw, no backdoor. Second, regulators take one look at the complex cryptography and classify the coin accordingly — delistings, restrictions, and adversarial treatment follow not from what the coin does but from how it does it.

The result is a landscape where you choose between transparency without privacy, or privacy without trust. Physical cash offered both, effortlessly. Digital money somehow forgot how.

---

## The Insight: Don't Hide the Trail — Don't Make One

Imagine you are at a market stall. You hand a five-dollar bill to the vendor. The vendor puts it in the register. Later that day, the vendor spends that same bill at the bakery. The bakery gives it as change to the next customer.

Now ask: who can trace that bill back to you? Nobody. Not because the bill's journey was encrypted or hidden — the bill was in plain view every step of the way — but because the bill itself never recorded who handed it to whom. The connection between you and the vendor was never written down. It existed only in the moment of handover and then was gone.

This is the difference between hiding information and never collecting it. Privacy coins hide. Marigold doesn't collect.

---

## How It Works: Notes Instead of Accounts

Most cryptocurrencies work like bank accounts. You have an address (like an account number), and transactions move value between addresses. The addresses are public, and every movement between them is recorded forever. It is an accountant's dream and a privacy nightmare.

Marigold works like cash. There are no accounts. Instead, the system maintains a pool of **notes** — think of them as digital banknotes.

Each note is a simple public record:

- A **serial number** (like the serial number on a dollar bill)
- A **value** (0.01, 0.1, 1, 10, 100, 1,000, 10,000, or 100,000 MAGLD)
- A **lock** (a public key — whoever holds the matching key can spend it)

That's it. No owner name, no address, no identity. The note does not know who holds it. It knows only that a key exists, and that whoever can unlock it can spend it.

### Making a Payment

Paying someone is like handing over a bill, with one extra step:

1. **You give the recipient the key** to the note (via QR code, text message, or even verbally — the same way you'd share a photo).
2. **The recipient immediately swaps the lock** — they record a transaction that replaces your key with a brand-new key that only they know. This is confirmed in under a second.

Once that swap is confirmed, the note is irrevocably theirs. You no longer hold a key that works. Settlement is done.

Alternatively, if the recipient can send you their new key ahead of time (say, via a payment request), you can swap the lock directly to their key. Your key never travels anywhere. Both methods produce the same result: the note moves from one key to another.

### Splitting and Merging

Notes come in fixed values, just like physical currency. If you need to pay 30 MAGLD and you hold a 100-MAGLD note, you **split** it into ten 10-MAGLD notes. You hand over three of them. You keep seven. Making change — same as cash.

Going the other way, ten 10-MAGLD notes can be **merged** into one 100-MAGLD note. The values move in factors of ten, so any amount is payable with a small handful of notes.

### What the System Records

Every operation — creating, swapping locks, splitting, merging, redeeming — is public and in plain view. What no one can see is **who** did it, because the system has no concept of "who." There is no sender field. No receiver field. No address book. No account.

The system records that a note changed hands. It does not record whose hands.

---

## Using Marigold in the Real World

Because a Marigold note is nothing more than a key, using one in the real world is as simple as using physical cash. Here is how:

### Print It, Spend It

Every note in your wallet app can be displayed as a **QR code**. You can print that QR code on a piece of paper, fold it up, and put it in your leather wallet — right next to your credit cards and your driver's license. That piece of paper *is* the money. Hand it to someone, and they scan it, swap the lock, and the note is theirs. You no longer hold a working key. The paper in their pocket is now worthless — the value has moved.

This is not a metaphor. The printed QR code holds the actual key. Losing it is like losing a hundred-dollar bill: whoever finds it can spend it. Keeping it safe is the same responsibility as keeping cash safe. There is no recovery hotline, no "report this stolen" button, no bank to reverse the transaction. That is what bearer means, and it is the entire point.

### Gift It

A birthday card with a printed QR code inside is indistinguishable from a birthday card with a fifty-dollar bill inside. The recipient scans it, swaps the lock immediately, and the note is irrevocably theirs. No account to set up, no waiting period. They open the envelope, they hold the money.

### Leave an Inheritance

A sealed envelope in a safe deposit box, containing printed QR codes for several notes, functions exactly like an envelope of cash. Whoever opens it holds the keys. Whoever holds the keys holds the notes. No executor, no probate court, no third-party permission is needed for the value to transfer — though, as with physical cash, the *legal* arrangements around inheritance are a separate matter the system does not and cannot address. The system guarantees only that possession of the key is possession of the note.

### Pay at a Market Stall

You are buying tomatoes. The vendor displays a QR code — their payment request. You scan it with your wallet app, select the note you want to pay with, and your phone swaps the lock directly to the vendor's key. Under one second later, the note is theirs. You pocket your tomatoes. The vendor never saw your name, your address, or your account. You never saw theirs. The transaction is settled, final, and forgettable — just like cash.

### Store It Cold

Worried about hackers? Print your notes as QR codes, put them in a fireproof box, and delete the app. The notes exist on the blockchain. The keys exist on paper. No internet-connected device holds them. When you want to spend, scan the QR code back into a wallet app, swap the lock immediately (in case someone copied the paper while it was stored), and transact normally.

### Move Between Wallets

Because notes are independent keys and not tied to any seed phrase or account, you can move a note from one wallet app to another at any time. Export the key from one app, import it into another. No on-chain transaction required, no fee paid, no interaction with the network at all. Your note works the same way in every wallet that supports Marigold — pick the app you like, switch whenever you want, your money comes with you.

The common thread: a Marigold note is a key, and a key can be printed, texted, folded into an envelope, stuck on a refrigerator, or committed to memory. The system does not care how the key travels between people, because the system does not know that people exist. It knows only keys and notes — and that is why every way you can move a physical banknote has a direct digital equivalent here.

---

## What an Observer Can and Cannot See

Marigold is not a privacy coin, and this litepaper will not pretend it is. Here is exactly what someone watching the system can figure out:

**They can see:** Every note, every value, every operation, the total supply, and the complete current state of the system. All of it is in plain view. Nothing is encrypted, ever.

**They can see:** The chain of operations — which note was swapped into which new note, when splits and merges happened, timing patterns. A determined analyst studying the public records can identify behavioral patterns: rapid successive payments, split-then-spend sequences, or a merchant's daily accept-and-merge rhythm.

**They can see:** Who created and redeemed notes, if they can link a regular blockchain address to a real-world identity. The points where Marigold connects to the traditional blockchain world are fully visible.

**They cannot see:** Any identity attached to a payment within the system. There is nothing to see, because the data was never recorded.

A note blends in with every other note of the same value — a 100-MAGLD note looks like every other 100-MAGLD note and nothing beyond that group. Wallet software can blur behavioral patterns (slightly randomizing timing, grouping operations together, varying which note sizes are used for fees), but this is good habits, not magic. Marigold makes no stronger claim.

---

## Why It Is Honest Money

Every few seconds, every participant verifies a single rule:

**All notes in existence + all coins on the traditional side = total coins ever mined.**

If this does not hold, something is wrong and the network can see it immediately. Supply is accountable at every moment — not by trusting an auditor, not by trusting a complex proof system, but by the kind of arithmetic anyone can do.

No value is ever created or destroyed within the system. Paying a fee removes a note from the pool, but its value is credited to the miner and can be recreated as a new note at any time. The conservation rule is absolute and universal.

---

## The Chain: Fast Enough to Feel Like Cash

The settlement rule is simple: a note is yours when your lock swap is confirmed on the network. That means **confirmation time is handover time.** On Bitcoin, you would stand at the market stall for ten minutes. On Ethereum, about twelve seconds. On Marigold, under one second.

Marigold runs on a proof-of-work blockchain that produces approximately 10 blocks per second, giving sub-second confirmation. The underlying technology was built by the Kaspa project — a fast, reliable, well-tested blockchain. Marigold adds the note system and its economics on top of that foundation, leaving the base layer intact because it does its job exceptionally well.

**A note on energy.** Marigold's energy story is not that it uses little — a proof-of-work chain attracts as much mining as its rewards are worth, and a successful Marigold will be no exception. The story is that none of it buys waiting. A classical blockchain can accept only one block per round; blocks mined in parallel are thrown away, so the network stays secure only by staying slow. The chain Marigold runs on keeps every block — blocks found at the same moment are woven into the ledger together, all of them counting — which is how the same security budget delivers ten blocks per second and sub-second settlement instead of a ten-minute queue. Per payment, that is an enormous efficiency difference. In total, it is the same honest arithmetic as everything else here: energy spent in proportion to the value being protected.

---

## Economics at a Glance

- **Supply:** 210,000,000 MAGLD, hard cap. No premine, no dev fund, no allocation of any kind.
- **Launch:** Fair launch from day one. Software available to everyone in advance. Everyone starts on equal footing.
- **Emission:** Smooth and gradual — the mining reward halves every three years without sudden drops. About 21% is mined in year one, ~90% by year ten.
- **Base unit:** 1 MAGLD = 100,000,000 petals.
- **Fees:** All fees go to miners. Nothing is burned, nothing is diverted. A cash economy — where every payment is an on-chain transaction — produces steady fee income that store-of-value chains cannot match.

---

## Launch Security: Training Wheels That Come Off

A new proof-of-work chain has low mining power, and low mining power invites attacks. Marigold launches with a temporary, fully disclosed safeguard called **finality anchors**: five publicly identified trustees periodically co-sign (requiring at least 3 of 5) a recent block, making it permanent and irreversible. No one can undo an anchored block.

The trustees cannot censor transactions, create coins, move anyone's funds, or produce blocks. Their only power is to prevent undoing completed transactions. If they go silent, the chain continues normally as a standard proof-of-work network — somewhat less protected, but fully operational.

The safeguard is designed to retire. Once the network's mining power grows strong enough that attacks are prohibitively expensive, anchors step down from mandatory to advisory to fully expired. Before that happens, the selection of successor trustees is intended to pass to note-holder governance — so even the training wheels' remnant transfers from founders to the community before they are removed entirely.

---

## What Marigold Is — and Is Not

**Marigold is** digital cash. Notes are bearer instruments. Possession of the key is ownership. Transfer of the key is settlement. The chain is complete about value and silent about people — exactly like a dollar bill.

**Marigold is not** a privacy coin. Nothing is encrypted. Nothing is hidden. The distinction is not euphemism: privacy coins cryptographically conceal recorded data, and Marigold records no data to conceal. What it shares with cash is specific and honestly bounded — the system is complete about *value* by design, and silent about *people* by design.

**Marigold is** five operations, one conservation rule, plain-view state, ordinary digital signatures, a fixed supply checked by everyone at every moment, on a network fast enough that handing over a note feels like handing over a bill.

Everything it does can be checked by the people it is for.