# The repositories, and which one a piece of work belongs in

One page for every session, human or model, to read before the first commit. It says where each kind of work lives, how the repositories pin each other, and how a session is expected to behave in a checkout. It exists because on 2026-10-12 three sessions committed into one checkout of this repository and the founder had to untangle it by hand; the rules at the end are the fix.

## The map

| Repository | What lives there | Pins |
| --- | --- | --- |
| `marigoldcash/marigold` (this one; checkouts are usually named `marigold-node`) | The node, consensus, the wallet (`wallet/core`, `cli`), the desktop wallet under `gui/`, the stratum bridge under `bridge/`, the faucet, the trustee signer, the phone wallet's Rust core under `wallet/service`, every plan and decision record under `docs/marigold/` and the top-level `PLAN.md`. Every Rust crate that depends on this workspace lives here, because none of these crates is published anywhere else. | Upstream `kaspanet/rusty-kaspa` through the `upstream` remote, merged on a cadence. |
| `marigoldcash/marigold-wallet` | Releases only: the built wallet binaries, installers and the Dockerfile that fetches them. No source. | A `marigold` release tag, `v<major.release.build>`, the same number in both. |
| `marigoldcash/marigold-pool` | The mining pool: its own Rust workspace, database, API and plan (`PLAN.md` there; the hand-over copy is [MINING-POOL-PLAN.md](MINING-POOL-PLAN.md)). Separate software that runs beside a node. | The `kaspa-*` crates of `marigold` as git dependencies by commit, moved on purpose and recorded in its plan. |
| `marigoldcash/expo-mobile` | The phone wallet's app: one Expo project for Android and iOS, and the native module that wraps the Rust core. Its plan is the part of [MOBILE-PLAN.md](MOBILE-PLAN.md) from PH-M4 on, copied in as its `PLAN.md` when the repository is made. No Rust toolchain is needed there. | A `marigold` release tag, through the prebuilt native core (XCFramework and Android libraries) that release carries as assets, by version and checksum in one file. |
| `marigoldcash/marigoldcash.github.io` | The site: `release.json`, the FAQ, the litepaper and whitepaper pages. | The current release, through `scripts/release-notice.sh` here. |
| `marigoldcash/marigold-miner` (private) | The historical fork of `kaspa-miner` used to bootstrap the testnet. Superseded by `marigold-cli mine-to`; kept for the record. | Nothing. |

`marigold-android` and `marigold-ios` were created empty and are deleted: one Expo app serves both phones.

## The rule that decides where new code goes

- **A Rust crate that depends on this workspace lives in this repository**, beside the crates it depends on, and is built, tested and linted with them. That is why `wallet/service` is here and not with the phone app, and why the desktop wallet is here under `gui/`.
- **Separate software that merely talks to a node pins this repository by commit or tag**: the pool does.
- **An app that only needs the compiled core takes it as a release asset**, never as a source checkout: the phone app does. The build of that asset belongs to this repository's release workflow, on the runners and under the signing that already exist.
- **Plans live with the code they drive.** A step marked `[marigold]` in another repository's plan is done here, on `main`, and that repository's pin is moved afterwards.

## How a session works in a checkout

1. **One checkout per session, and one session per checkout.** The pool session works in its own clone of `marigold-pool`; its `[marigold]` steps use the sibling clone `~/Documents/dev/marigold`. The phone session works on its own `mobile/*` branch in its own worktree for the Rust core, and in a clone of `expo-mobile` for the app. Two sessions in one checkout is how commits land on the wrong branch.
2. **Before the first commit**, `git status -sb` and `git worktree list`. If the checkout is not on `main`, or commits from someone else sit above `origin/main`, stop and say so rather than building on them.
3. **Commits reach `main` by fast-forward and are pushed in the same session.** Nothing waits unpushed in a tree another session might open. A step's commits carry the step ID first (`M8: …` for the mining pool plan, `PH-M8: …` for the phone plan, `P9.4: …` for the main plan), and a build that produces a binary for anyone bumps the version (`scripts/bump-version.sh`, which `scripts/remote-build.sh` runs for you).
4. **Builds and tests run on pve3**, never on the machine the session sits on (`scripts/remote-build.sh`, `CHECK=1` and `TEST=1` for the pool). A step is ticked only when its verify passes.
5. **Another repository's plan is amended by the session that owns it.** A decision taken elsewhere is relayed to that session in words; the file is not edited from two places.
