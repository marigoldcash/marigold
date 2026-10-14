# Cutting a release

One script does it: `scripts/release.sh <previous tag> <notes file>`, run from a clean `main`. This page says what the machine running it needs, what the script does, what to do when it stops half way, and which three signatures a release carries. Anyone with write access to the two repositories can release; as of 2026-10-12 that is leancode and spacecharly.

## What the machine needs

- **A build host.** Binaries for the release are built by CI; nothing is built locally for the release itself. The local rebuild at the end (`NO_BUMP=1 scripts/remote-build.sh marigold-cli`) uses pve3 through `scripts/remote-build.sh`, which needs SSH to it; it only keeps your own wallet binary in step with the tag and can be skipped.
- **`gh` signed in** with write access to `marigoldcash/marigold` and `marigoldcash/marigold-wallet` (`gh auth status`).
- **Two clones beside this tree**, or named by `WALLET_REPO=` and `SITE=`: `marigoldcash/marigold-wallet` (the Docker image's files, whose Dockerfile and README name the release) and `marigoldcash.github.io` (the site, which carries `release.json`, the document the wallet's `update` reads). Both are pushed by the script, so the clones need push rights too.
- **SSH as root to the trustee hosts** for the manifest signature (hetzmisc, `seed2` in the inventory, and seed3; `scripts/release-notice.sh` names them), with the deploy key `~/.ssh/marigold_deploy`. Without it the release still happens, but the last step, the trustee-signed manifest, fails and `update` keeps offering the previous release; run that step again from a machine that has the key.
- **The announce bot's token** at `~/.config/marigold/announce-bot.token` and the channel id beside it, for the post afterwards (`scripts/announce.sh`); placed by whoever posts, never in the repository.

## What the script does

1. Bumps the release number (`scripts/bump-version.sh release`), commits "build: version X", tags `vX`, pushes main and the tag.
2. Replaces the previous tag with the new one in the wallet repository's Dockerfile and README, commits and pushes.
3. Creates the release on `marigold-wallet` with the notes, and a source release on `marigold` that points at it.
4. Starts the three build workflows (`binaries.yaml`, `gui-binaries.yaml`, `mobile-core.yaml`) for the tag. The last builds the wallet core for the phones and attaches two zips (`marigold-core-<version>-ios.zip`, `marigold-core-<version>-android.zip`), their `.sha256` files and `SHA256SUMS-core.txt` to the `marigold` release, with the workflow's own token; the phone app (`marigoldcash/expo-mobile`) pins one of these by version and checksum.
5. Moves the site's "latest" line in `release.json` and pushes the site.
6. Waits for the wallet build, then attaches the bare `marigold-cli` (the Linux binary under its plain name), which the self-update downloads.
7. Waits for the desktop build; if the installers carry the updater's signature, writes `desktop.json` (they do not yet, see the three signatures).
8. Reports how the phone core's run ended and which files are on the `marigold` release (the wallet release does not wait on it; if it failed, run the workflow again from Actions with the tag).
9. Downloads the release's digest list (`SHA256SUMS-cli.txt`), has the trustees sign it, writes the signatures into `release.json`, pushes the site.

Afterwards, by hand: the announcement (`scripts/announce.sh < post.txt` for the channel, `--to <group id>` for the testers' group, always with the tag's own link, never `/releases/latest`), the row in [STATE.md](STATE.md), and the local rebuild.

## When it stops half way

Every step is idempotent enough to repeat on its own. The two that have failed in practice:

- **A network error talking to GitHub** after the tag is pushed. Re-run the remaining steps by hand: `gh release create` for whichever release is missing, `gh workflow run` for whichever workflow did not start, then the waiting part. The script prints which step it reached.
- **A build fails in CI.** Fix on main, and cut the next release; a tag is never moved. The release with the failed build stays, with whatever assets it got, and the site's "latest" line points at it until the next one, so fix quickly or move "latest" back by hand with `scripts/release-notice.sh --latest <previous>` and push the site.

The attach-the-bare-binary and sign-the-manifest steps can be re-run alone: download `marigold-cli-linux-x86_64` from the release, upload it as `marigold-cli` with `--clobber`; download `SHA256SUMS-cli.txt` and run `scripts/release-notice.sh --latest X --manifest SHA256SUMS-cli.txt`, then push the site.

## The three signatures, and who holds the keys

1. **The trustee-signed manifest** (`release.json`, step 8). The finality-anchor trustees, the same keys that sign the anchors, sign the release's file digests; the terminal wallet's `update` installs only what they signed, verifying against the keys compiled into it. On the testnet the keys are the throwaway set on hetzmisc and seed3, reached over SSH; on mainnet the trustees sign on their own machines and the script is a template for them. This is the signature that matters for safety.
2. **The desktop updater's signature** (`desktop.json`, step 7). Tauri's updater verifies a minisign signature on each installer with a public key compiled into the app; the private key belongs in the build workflow's secrets. Neither exists yet, so the step prints "no signed installers" and the desktop app does not update itself. The key is the founder's to generate and place.
3. **Operating-system code signing** for macOS and Windows: an Apple developer certificate for notarization and a Windows certificate. Not done; they stop the operating systems' warnings on first launch and add no safety beyond the first two.
