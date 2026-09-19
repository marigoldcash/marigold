# Contributing to Marigold

Marigold is digital cash in fixed notes, built as a fork of [rusty-kaspa](https://github.com/kaspanet/rusty-kaspa). This repository holds the node, the wallet, the stratum bridge, the faucet and the trustee signer. The wallet people download lives in [marigold-wallet](https://github.com/marigoldcash/marigold-wallet), which builds from the releases made here.

We are on a public testnet. The most useful thing you can do right now is use it and tell us what broke: a note that would not pay, a prompt that made no sense, a sync that stalled. Open an issue with what you typed, what you saw, and the version from the front note or `marigold-cli --version`.

Found a way to spend a note twice, take a note that is not yours, or stop the network? That goes to security@marigold.cash, not to an issue. [SECURITY.md](SECURITY.md) says how.

## Where things are

- `consensus/`, `consensus/core/src/notepool/`: the chain, and the note pool that makes it Marigold. The specification is [docs/marigold/POOL-SPEC.md](docs/marigold/POOL-SPEC.md).
- `cli/`: the wallet. `wallet/core/`: what it stands on.
- `bridge/`: the stratum bridge for miners. `faucet/`: the testnet faucet. `trustee-signer/`: the finality-anchor signer.
- `docs/marigold/`: the plan ([PLAN.md](PLAN.md) at the root), the decisions ([DECISIONS.md](docs/marigold/DECISIONS.md)), the state of the project ([STATE.md](docs/marigold/STATE.md)) and the working notes. Read DECISIONS.md before proposing to change something it records; it says why things are the way they are.

## Rules that keep the fork mergeable

These are not style. They are what lets us keep taking upstream's security fixes for years.

1. **Do not rename `kaspa-*` crates or Rust module paths.** Rebrand user-facing strings only. Internal names stay identical to upstream so `git merge upstream/master` remains possible.
2. **One concern per commit.** Never mix a consensus change with anything cosmetic. Small commits, each doing one thing, each with a message that says why.
3. **Every consensus change carries its test in the same pull request.** No exceptions.
4. **Money-guarding code soaks on testnet before it goes anywhere near mainnet.** A change to the pool, the wallet's key handling or the finality anchors is not done when it merges; it is done when it has run for weeks with real testers.

## Before you open a pull request

Build on a machine with room; the workspace is large. Then:

```sh
scripts/check
scripts/test
```

`scripts/check` formats the tree and runs clippy (`scripts/check.ps1` on Windows); `scripts/test` runs the suites and needs `cargo-nextest`. CI runs clippy with warnings denied, so a warning here is a failure there. It also builds the wallet for the browser and the node with musl on every push and pull request, so a pull request that fails any of those will not be reviewed until it passes.

Write the pull request for the reviewer: what changed, why, what you considered and rejected, how you tested it, and anything a node operator or wallet user will notice. If it is large, split it.

Commit messages: a subject line under about sixty characters, a blank line, then the reasoning. Write them for the person reading `git log` in two years. No generated attribution lines of any kind.

## Reviewing

If you can read a change, review it, whether or not you have written code here. If you can only run it, run it and say what happened. Approve when you believe it is correct and safe; request changes when you find a real problem, and say what would fix it.

## Discussion

Use issues for anything that touches consensus, the pool specification, the wallet's key handling or the network's parameters; those want a design conversation before code. For everything else, a pull request is a fine place to start talking.

## Conduct

Be direct and be kind. Disagree with the idea, not the person. We are a small project and we remember how people treat each other.

## Licence

By contributing you agree that your contribution is licensed under the same terms as the project, in [LICENSE](LICENSE).
