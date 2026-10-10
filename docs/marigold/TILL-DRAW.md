# Verified tills and the draw — a design note (proposal, 2026-10-08, revised 2026-10-10)

Status: a proposal for the group, written at the founder's request after the mining discussion of 2026-10-07/08 and revised after the founder put it to the tester group on 2026-10-10 (the till lane format, the activation rule, the fee threshold and the journal come from that discussion). Nothing here is decided or implemented. It belongs in the mainnet design (PLAN P9.5), not in a testnet tweak.

## In one paragraph

The question behind the mining debate is not "how do we keep ASICs out" but "how does a person who keeps a computer on still get something". No proof-of-work answers that: whoever brings better hardware takes the reward, and on one algorithm the small miner ends with dust whichever hardware that is. That holds on a dual-algo chain too, once enough people mine the second lane. And ASICs give far more security per watt than any other hardware, so leaving the chain's security to them is as close to an answer to the energy question as proof-of-work has. This proposal pays a share of that reward to different participants: to the shop that takes Marigold at the till, and to the customer who pays at such a till. A till is a Raspberry Pi running the wallet, sold to a verified business, verified because a till has to account for VAT anyway, so the shop is public already. Every verified till holds one ticket in a draw on the chain, seeded by the trustees' anchors so no miner can steer it, paid from a fixed slice of the block reward. The shop earns a little for being a place where the coin is spent, which is the one thing hashrate cannot buy: adoption. The pot, won by customers' payments at those tills, is the receipt lottery governments run for VAT, and it is the customer's side of the same design.

## The problem, stated plainly

Mining pays in proportion to hashrate. The "keep my computer on" miner loses the moment anyone shows up with better gear on the same algorithm; ASICs are only the last step, GPUs already outhash a laptop a hundredfold on kHeavyHash. The only thing a design can choose is who the small miner competes against, and in every case the answer to "what does a Raspberry Pi earn" is: a Raspberry Pi's share of the total, which rounds to nothing once there are many. The reward has to be given for something a chip cannot do more of. Being a verified shop is such a thing.

## The proposal: verified tills

### The till

A Raspberry Pi (or any small machine) running the Marigold wallet as a point of sale: request codes on a screen, payments taken as bearer notes, the day's takings and the VAT split kept on the device, a Z report at closing. The customer is anonymous, as with cash; the shop is public, as it is already. The shop's lane (LANE-REGISTRY.md) becomes its fiscal log: the hash of each day's Z report goes into the lane every evening, and the chain is the tamper-proof record that fiscal-device laws (Germany's KassenSichV, Austria's RKSV, Italy's and France's rules) ask of a cash register. "Prove your records were there" turned into a product. Each country's certification is its own project; start with one.

**A till is a wallet with a journal, not a node.** It never needs the chain's history, not even its own lane's, because everything it reports it saw first-hand. During the day it is the wallet it already is and writes each sale to a journal on the device: receipt number, time, amount, VAT split, note serials, transaction id. At closing it sums the journal into the Z report, hashes it, and sends one small transaction into its lane with the hash; that is the only time the lane is written. For a node it needs what any wallet needs, the public node or the embedded light node a Pi can run. Proof is the auditor's job, not the till's: hash the journal the shop hands over, fetch the day's anchor from any archival node, compare. The journal is the valuable thing on the device, kept for the legal retention period (ten years in Germany), so the folder backup (WALLET.md §11b) with a cloud-mirrored folder is its off-site copy from the first version.

**The journal is signed and chained, and the payments pin it.** The fiscal laws ask for this shape explicitly: Austria's rule is that every receipt carries the shop's signature over the previous receipt's hash, with the chain value printed in the receipt's QR code; Germany asks the same with a stricter twist, the signing must come from a certified security module the shop cannot control, with a monotonic counter, because a shop that holds the key can rewrite the whole day and re-sign it. The till signs and chains every receipt with its till key from the first version; on its own that proves order and integrity, not the honesty of the key holder. What closes that gap, and no cash register has it, is that every sale is already a transaction on the chain: a payment is a note transfer to the till, timestamped by its block, and once the transfer names the till's lane id the chain carries the count of payments each till took that day. A journal entry commits to its transaction id. Delete a sale and there is a payment to the till on the chain with no receipt behind it, which an auditor finds by counting; add a fake sale and there is a receipt with no payment. The daily anchor seals the journal's head, the payments pin the receipts in between, and the till has nothing it can quietly rewrite. Whether that passes a country's certification without a hardware module is the per-country project above: Austria is the natural first, Germany the hard one.

### The registry

A till is registered by a lane claim that carries a till key, made by a business the association has verified off-chain: a VAT number, a registration, a person. The chain sees the key, the tag and the association's countersignature, never the business data. The association is therefore a gatekeeper, a central point that is acceptable for public merchants but needs written rules from the first day: how a till is revoked (a shop closes, a Pi is stolen, a business turns out not to exist), who decides, how a shop appeals, and how many tills one business may register (one per premises, say, with a cap). The claim fee (100 MAGLD today) and the rule that an idle till holds no ticket make it pointless to register tills nobody uses; whether the business behind a till is real is a question only the verification answers.

**Till lanes are left-padded; company lanes are right-padded.** A company lane is a tag of up to five characters in the first bytes of the twenty-byte subnetwork id and a zero tail (LANE-REGISTRY.md). A till lane is the mirror image: a zero head and the tag in the last bytes. The two shapes are disjoint by construction, a company id has a zero tail and a till id has non-zero bytes at the end, so no id reads both ways, no prefix has to be reserved, and the all-zero native id and the coinbase id collide with neither. It costs no storage: the subnetwork id is a fixed twenty-byte field in every transaction already, padding included. And nobody can squat the namespace before the fork: today's validator rejects a left-padded id because its tail is not zero, so till ids come into existence only when the draw's hard fork adds the new arm to the shape check.

**The till tag is a country code and a running number.** The two letters of ISO 3166 (`DE`, `CH`, `US`) and then the number the association hands out at verification, without fixed width and without leading zeros: `DE1`, `DE7832738`, never `DE0000008`, so that each number has one id and the width grows when the count does (nineteen bytes leave room for seventeen digits). The country code is for the wallet: it picks the VAT rules the till applies and the Z report format from the first two letters, which is why the split is by country and not by region; where a country has sub-national rates, the United States with its state sales tax, the till holds that locally and the chain never knows. Verification is per country as well, since a VAT number is bound to one.

**The countersignature is what makes a lane a till.** Anyone can pay the claim fee and claim a tag, so if the draw counted every till-shaped lane a ticket would cost 100 MAGLD and the draw would be farmed. A till claim is therefore a new claim version carrying the full id, the till key and the association's signature over both, with the association's key known to the chain the way the trustees' keys are. The shape tells a node the lane is till-shaped; the signature tells it the till is verified; only the signed ones hold tickets, and because the signature covers the tag, a number cannot be squatted ahead of the shop it was given to.

## The shop draw

Every `D` blocks (a number to decide; every anchor interval is natural) one registered till wins a payout, written into the block like a coinbase output. Every node computes the winner the same way, so it is a consensus rule and not a service:

- **Tickets.** One per registered till that is *active*: the payments it took in the last `W` days carried at least `F` MAGLD in fees. The customers pay those fees, so this is fees the till has processed, not fees it has paid, and the chain can count them because every payment to a till names the till's lane id. Not weighted by turnover. A reward that grows with turnover is a reward for a shop paying itself in a loop; one ticket each cannot be gamed by volume, and it keeps a shop's turnover off the public chain, where competitors would read it. Fees are the activity measure because a loop pays them to miners and gets back less, and that is the rule for setting `F`: a till that fakes its activity with payments to itself must spend more in fees over the window than a ticket can expect to win in it, by a safe margin. A ticket's expectation is the slice paid out over the window divided by the number of active tills, so `F` is that figure times a safety factor, three say, computed from the previous window's count, which makes it a formula every node evaluates rather than a constant anyone has to revisit.
- **Seed.** The latest finality anchor's signatures, not a block hash. A block hash is the miner's to grind: the miner of the seed block could try hashes until a friend wins. The anchors are signed by a trustee quorum and no miner can touch them, which makes them a randomness beacon we already have.
- **Winner.** `H(seed ‖ draw index) mod (number of active tills)` over the registry sorted by key. Every node holds the registry already (it is the lane registry); the draw costs a hash and a lookup, which is nothing against verifying a block.
- **Funding.** A fixed slice `S` of the block reward, withheld from every block's coinbase and paid out at the draw. The miners pay for it, and what they get in return is adoption: places to spend the coin they are paid in, which is what gives it any worth.
- **Activation.** There is nothing to withhold until there are tills, so the slice is withheld only in blocks where the registry holds at least `N` active tills, otherwise the miners keep it, and if the count later falls under `N` the draw pauses. Every node has the registry in memory, so this costs a count. One count for the whole network, not per country or region: a ticket is a till wherever it stands, and the customers' pot pays wherever people actually spend, so a town that adopts early wins more often by holding more tickets, without any notion of place in consensus. A regional launch is a marketing act, not a consensus rule.

### Numbers to decide

| Parameter | Meaning | A starting point |
|---|---|---|
| `S` | the slice of the block reward that funds the draw | 5 to 10 % |
| `D` | blocks between draws | one per anchor interval |
| `F`, `W` | fees the payments to a till must have carried in the window for it to hold a ticket | `F` a safe multiple of what a ticket can win in the window, so that faking activity costs more than it pays; `W` long enough that a quiet week does not drop a village shop |
| cap | tills one business may register | one per premises, at most a handful |
| `N` | active tills before the slice is withheld at all | a few dozen |

With ten blocks a second and a modest slice, the pool per draw is a few coins; the point is that a shop in a village with twenty tills in the country wins often, and a shop among twenty thousand wins rarely but the twenty thousand tills are the adoption we wanted.

### Keep it a bonus

A shop should take Marigold because customers pay with it; the draw is the thank-you. If the draw ever looks like the reason to buy a Pi, the Pi looks like an investment product, and that is a conversation with a regulator nobody wants. The slice stays modest and the language stays "a little on top".

## The pot: a receipt lottery for customers

The customer's side of the till draw. A tiny slice of every block reward, one percent say, is withheld into a **pot** that every node tracks as a number in consensus state, and it grows visibly ("the pot stands at 14,000 MAGLD"). Every payment **made to a verified till** whose fee clears a threshold is a ticket. The ticket's number is the hash of the payment and the first finality anchor sealed after it, which nobody knows when they pay; it wins when that hash falls below a target tuned so the pot pays out about once a month across the whole network, or whenever the pot passes a cap, so it never grows without end. The pot goes to a key the payment carries for the purpose, in the block after the anchor. The customer stays anonymous, nobody knows who won but the winner, and the shop is public anyway and gets its own story.

This is not an invention: it is the **receipt lottery**. Taiwan has run one since 1951, Portugal, Slovakia, Malta, Brazil and Italy run theirs, all for the same reason, to make customers ask for the receipt so the shop declares the sale. Governments invented it to collect VAT, and that is the framing: a compliance incentive with precedent, not a casino.

Three design points carry it:

- **Only at verified tills.** Without that condition the pot is spam bait: as it grows, paying oneself in a loop becomes worth the fees and the network fills with wash payments. With it, farming tickets needs a verified shop's cooperation, and a shop that books fake sales pays VAT on them, a real cost with a real auditor. The pot therefore pulls customers to shops that take Marigold and shops into the registry.
- **The match is a hash with a seed from after the payment.** Not the amount and not a number known at payment time; both are grindable, one would keep paying until the amount matched. The anchor sealed after the payment is unknown to payer and miner alike.
- **Flat and anonymous.** One ticket per qualifying payment, no holding, no identity, nothing locked. It costs a node one hash per qualifying payment, a counter, and one output when it pays.

Two details from the group discussion. **The threshold is on the fee, not the amount.** The threshold has two jobs, to keep a node's ticket set small and to stop a coin being paid in a hundred crumbs for a hundred tickets; a fee floor does the second on its own, since a loop of tiny payments costs more in fees than it can win. Set it so a normal purchase, a coffee, clears it and a purchase split into crumbs does not: on the testnet the normal fee of a single-note payment, so every real purchase counts. It is one number the trustees can carry in the anchor and move without a hard fork. **The payment names the till.** For a node to count a ticket the note transfer carries the till's lane id, a field in the transfer and the one place where the till reaches into the payment format; it is the same field that lets an auditor count a till's payments against its journal (the till, above). The pot activates with the draw, at the same `N` tills, and freezes but keeps its balance below it.

Two things to settle before proposing it. **Law:** a prize draw tied to a purchase is a promotional game in Swiss and EU law, allowed when the purchase is at the normal price and nobody sells tickets, which is this case; the association must never sell a ticket, or a Pi, as a chance to win, and a lawyer should read it once with the receipt-lottery precedents on the table. **Numbers:** the slice, the threshold, the target and the cap; a month's pot at one percent of the reward is a few thousand MAGLD at today's emission, and whether that is a draw people talk about or one they ignore is the question to decide.

## Privacy and compliance

Customers stay bearer-anonymous; nothing changes for them. Merchants are public, which they are by law. The chain carries the till key, the lane tag and the fiscal-log hashes, never the turnover. A tax office sees a cash system that produces the VAT report by itself and keeps a record it cannot alter; that is the compliance argument, and it is stronger than any argument a coin can make about itself.

## ASIC resistance: the options, and the case against each

The group asked for the alternatives on record.

### A monthly tweak seeded by the last block of the month

The idea: a tiny change to the algorithm every month, chosen by something unpredictable such as the last block's id. It does not work, for three reasons. An ASIC is fixed to a circuit, not to a constant: kHeavyHash already changes its matrix every block and the chips take it in stride; a seed-derived value is just another input the chip reads, and a change that hurts a chip has to alter the structure of the computation, which cannot be tiny. A structural change cannot be generated from a seed either, because every node has to run it and we have to know it is sound, so the variants must exist in the software in advance, a finite menu, and an unpredictable choice among a known menu costs an ASIC maker a little silicon and nothing else (ProgPoW found this: a randomised program from a bounded generator still ended with specialised hardware ahead). And the seed is not unpredictable: whoever mines the last block of the month can grind it and choose the variant that suits their hardware. Monero's scheduled forks worked for a while because people wrote real algorithm changes every six months; it ended in RandomX.

### A memory-bound algorithm (RandomX-style)

The one route that has held: make the work memory- or latency-bound so the best hardware is what everyone already has. Two prices, both heavier for Marigold than for most. **Verification**: we run ten blocks a second with a DAG that also validates the losers; kHeavyHash verifies in microseconds, RandomX in milliseconds with a two-gigabyte dataset or a slow light mode, and a Raspberry Pi node would not keep up. **Botnets**: CPU-mineable coins are mined by stolen computers; Monero's hashrate has a large botnet share, a worse centralisation than a hall of ASICs because nobody can find it.

### Two algorithms, a CPU lane

Most blocks kHeavyHash, a fixed share (one in eight, say) valid only with a memory-bound algorithm, each lane with its own difficulty, as Myriad and DigiByte have run for years. It keeps ASIC security and gives CPUs a lane they compete in among themselves. It costs two difficulty adjustments and the verification budget of the memory-bound share, which has to be measured at our block rate on the Pi and a desktop before committing. And it rewards CPUs collectively: ten thousand laptops share the lane, and each gets a ten-thousandth. It is a real option if the goal is "CPUs keep mining"; it does not answer "the small guy gets something".

### Accept ASICs, borrow their security

Kaspa's stance, and the least work: ASICs are security. For Marigold there is a second reason it is tolerable: the trustees' finality anchors cap how deep a reorganisation can go, so hashrate here is a question of distribution, not of safety. Merge-mining with Kaspa would bring its hashrate for free. With the draw above, distribution is handled elsewhere.

### What is not possible

Telling an ASIC's block from a CPU's on the same algorithm. The hash is the same number whoever found it. Nonce patterns and block rates per address are statistical hints that cost nothing to forge, and any reward for "looks like a CPU" becomes a reward for "pretends to be a CPU". The only distinction the chain can enforce is one the hardware cannot fake: a different lane.

## "Anyone can do a KYC, contribute, and get rewarded"

Taking the idea one step further — a draw for anyone who verifies their identity and runs something — is a step too far, for four reasons.

- **It changes what Marigold is.** The coin's promise is cash: bearer notes, anonymous customers. A network where the way to earn is to show your passport puts an identity register at the centre of a cash system. Shops are public by law; people are not.
- **The association becomes an identity verifier for the public.** Thousands of passports, data-protection duties, fraud, support, appeals, in every country at once. For merchants that is a few hundred businesses with VAT numbers to check against a public register; for the public it is a company's worth of work and a liability.
- **Nothing is at stake, so it is a sybil market.** A verified identity with no business behind it is cheap to obtain and cheaper to rent; "one ticket per person" becomes "one ticket per bought identity". A shop has a lane, a fee paid, premises, a VAT return, and a reason to keep its registration clean.
- **What would they contribute?** Running a node costs little and the network needs only so many. Paying for presence without work or stake is a basic income paid from inflation, and it would be the thing regulators and miners would both object to first.

If a wider "presence" reward is ever wanted, the honest form is a stake: a note locked for a period buys a ticket (Decred's model), with no identity involved. That is a bigger design and not one for the testnet this year.

## "Mine with your CPU now, then lock it in to buy tickets"

The idea raised after the first draft: let the CPU and GPU miners of today earn while a CPU still finds blocks, and give them a future once ASICs arrive by letting them lock notes for a period to buy tickets in a draw, Decred's model, with no identity involved. It works mechanically: a ticket is a note whose serial is committed on the chain with a timelock, the draw is seeded by the anchors as above, the winner is paid to the note's key. The pieces exist. What it changes is what Marigold rewards, and that is the case against it. Cash that is locked for yield is not cash: the whole design is notes that change hands, the till draw pays for use, a stake draw pays for not using, and once holding still earns a return, holding still is what a rational owner does; every coin that added staking became a savings product with a payment feature. It compounds: tickets scale with holdings and winnings become tickets, which Decred fights with a moving ticket price and a cap and still concentrates, where the till draw is one ticket per shop by construction. "Lock it in to get more" is the sentence a regulator quotes: a reward for staking money with the expectation of profit from the network's growth is the test they apply, and the association would be the issuer, where a shop's thank-you for being a place to spend is not. Two draws from one slice dilute each other, and the tills are the draw the miners can be sold on because adoption pays them back. And "mine now while you can" is honest for a short while only, on the testnet and the first months of mainnet, and needs no staking promise behind it. The early-adopter story that costs nothing to unwind later is: mine with your CPU now while a CPU still finds blocks, spend what you mine, and if you run a shop, take it at the till and the chain pays you a little for that.

## The use-side alternative: a ticket per payment

If the group wants a reward for ordinary holders anyway, the version that rewards spending rather than hoarding is worth a look before any stake draw. Every payment whose fee is above a threshold is a ticket for the payer, drawn with the same anchor seed, paid to a key the payment carries for the purpose; cashback by lottery rather than yield. The sybil cost is the fee, so a loop of payments to oneself costs more than it expects to win; the customer stays anonymous because the ticket is the payment, not a person; and nothing is locked. It is more work than the till draw (a payout key in the payment format, a ticket set per draw window that every node keeps) and gameable, a little, by whoever can pay fees cheapest, which is why it is listed here as the alternative to the stake idea and not as a second proposal.

## Open questions

1. The slice `S`, the draw interval `D`, the activity threshold `F`/`W`, the activation count `N`, the cap per business; for the pot, its slice, the fee threshold, the target and the cap.
2. The registry rules: who verifies, how a till is revoked, how a shop appeals, what a stolen Pi means; and the association key the chain checks the countersignature against, how it is pinned and how it rotates.
3. Which country's fiscal-device rules to meet first (Austria's signed receipt chain is the natural first, Germany's certified module the hard one), what the Z-report-into-the-lane format is, and what the receipt's QR code carries.
4. Whether the draw starts on the testnet with the first Pi tills (a consensus change at a testnet reset) or waits for the mainnet parameters.
5. Whether a CPU lane is wanted at all once the draw exists, and if so, the verification numbers for it.

## What it costs a node

Per draw: a registry lookup (the lane registry is already in memory), one hash, one output in a coinbase. Per block otherwise: nothing. Compared with the alternatives it is the cheapest change on the table, and it is the only one whose beneficiary is a shop.
