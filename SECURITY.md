# Reporting a security problem

If you have found a way to make Marigold spend a note twice, take a note that is not yours, learn who holds a note, stop the network, or anything else that should not be possible, please tell us privately first.

## How

Open a private report on GitHub: **[Report a vulnerability](https://github.com/marigoldcash/marigold/security/advisories/new)** on this repository. It is not a public issue: only the maintainers see it, it reaches them the moment it is filed, and the form asks for what we need, so there is no back and forth over sensitive details. Say what you found, how to reproduce it, which version you were running (`marigold-cli --version`, or the version on the wallet's front note), and whether you have told anyone else. Attach a proof of concept if you have one; it stays in the report.

If you cannot use GitHub, write to security@marigold.cash instead, and then send a short message to **@marigoldcash on Telegram** saying only that a security mail is waiting, with no details in the message. The report on GitHub is read sooner than the mail.

You will get a reply from a person, normally within two days, and a fix or an explanation as soon as there is one. The report stays private until the fix has shipped; then it is published as an advisory with you credited, or not, as you prefer.

## What counts

Anything in this repository, the wallet image, the bridge, the pool, the faucet, and the website. The testnet is there to be broken: try what you like on it, and coins there have no value. Please keep proofs of concept off other people's wallets and machines, and do not run anything against mainnet once it exists.

There is no bounty programme. There is gratitude, credit, and a fast fix.

## What does not need this

Crashes, wrong balances, confusing prompts, and everything else that is a bug but not a hole go in a normal issue. If you are not sure which it is, use the private report.
