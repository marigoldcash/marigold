# The lane registry

A company that anchors its records on Marigold does so in a lane of its own: a user-lane subnetwork whose tag, up to five letters like a ticker symbol, is the company's, so that anyone can find its anchoring transactions by filtering a block for that tag and verify them against any archival node, as [ANCHORING-GATEWAY.md](ANCHORING-GATEWAY.md) describes. This page is how a lane is claimed: on the chain, with one transaction, for a fee. No lane is granted in advance to anyone; the first integration partner claims theirs at launch like every other company (founder, 2026-09-22). The chain is the registry. Nobody keeps a list.

## Claiming a tag

A claim is one ordinary transaction with three properties:

- it lives in the registry's own lane, subnetwork id `[0x4c 0x41 0x4e 0x45, 16 zero bytes]` ("LANE");
- it pays at least the registration fee, **100 MAGLD**, to the registry address of the network (testnet: `marigoldtest:qqfc69eulu3v8qamcasxqcx3wxvsfzjkv73fau4exk5kem70nqjsqgm3wm9g8`; mainnet: set at the parameter freeze, PLAN P9.5);
- its payload is a claim: `0x02` (version) ‖ tag (5 bytes, zero-padded) ‖ the company's public key (32 bytes, x-only BIP340) ‖ label length (1 byte) ‖ label (UTF-8, at most 64 bytes). (Version `0x01`, the first testnet claims, carried a four-byte tag.)

A tag is one to five ASCII capitals or digits, like a ticker symbol, and the lane's subnetwork id is the tag zero-padded to twenty bytes. Five-letter tags need wide lanes, a consensus rule with its own activation (testnet-10: DAA score 18,000,000; mainnet: from the first block); until then tags of up to four letters can be claimed, and every four-letter lane keeps its identity under both rules. `POOL`, `ANCR` and `LANE` are the network's own and cannot be claimed. The wallet makes the claim with `lane claim <TAG> <key> [label]`; the key is the company's, the one its anchors will be attributed to, and the wallet never holds its secret.

**The first valid claim of a tag holds it.** Validity is the three properties above; order is the chain's: the claim whose accepting block comes first in the selected chain wins, and a later claim of the same tag is a donation. A claim is never undone and a tag never changes hands on the chain; a company that loses its key makes a new claim under a new tag.

**Listing the claims** is a walk over the registry lane from an archival node: every transaction with the registry subnetwork id and a payload that decodes as a claim, in chain order, keeping the first per tag. marigold.cash's verification service does this walk and publishes the list; anyone with an archival node can do the same and get the same answer.

## What a lane is for

A company builds a Merkle tree over the hashes of a batch of records — a month of signed PDFs, a day of attendance sheets, whatever it attests — and puts the root on the chain in one transaction in its lane, with the payload the gateway contract specifies (`0x01` ‖ the 32-byte root). Each customer gets their record, its inclusion proof, and the transaction id. Every anchor is signed by the lane's registered key (the wallet keeps that key in its vault: `lane key <TAG>`, then `lane anchor <TAG> <root>`), because a lane is not exclusive at the chain level and a stranger's transaction in it must not pass as the company's. Verifying a record needs no company, no gateway and no marigold.cash: hash the record, walk the proof to the root, fetch the transaction from an archival node, check the subnetwork id is the company's lane and the payload's signature verifies under the key in the lane's claim, check the payload holds that root, read the accepting block's time. That is the attestation, and it holds as long as one archival node exists.

## The verification service

Verification is free to anyone with an archival node. marigold.cash runs one, with an HTTPS service in front of it, for companies that would rather not: "is this root anchored, in which block, at what time", and the list of claimed lanes. The service is metered, and **the chain takes nothing**: every fee below is what marigold.cash charges for a service it runs, the way any host charges for hosting, and pays for the archive and the people who build the software. It is not a protocol fee and not a fund; the whitepaper's statement that every on-chain fee goes to miners stands.

- **10 MAGLD a month** per lane, drawn on the first of the month, keeps the archive paid whether or not anyone verifies.
- **0.01 MAGLD per verification call.**
- Both are drawn from a **prepaid balance** the company tops up by paying marigold to the service's share key; the balance and the month's usage show on the company's page. Calls are refused when the balance cannot cover them; the lane and the anchors on the chain are unaffected, since they never depended on the service.

The service ships with the anchoring gateway (PLAN P9.4c), as a sidecar beside the archival node, never as a fourth RPC surface on it. Prices are the founder's to change; on testnet they are nominal and paid in test coins.
