#!/usr/bin/env bash
#
# Cut a wallet release from a clean main, end to end. docs/marigold/RELEASE.md
# says what it needs and what to do when it stops half way.
#
#   scripts/release.sh v2.87.343 notes.md
#
#   $1  the previous release tag: it is replaced by the new one in the wallet
#       repository's Dockerfile and README
#   $2  a file with the release notes, in Markdown
#
# What it does, in order: bumps the release number and tags main; pushes the
# wallet repository (the Docker image's files); creates the release on
# marigold-wallet with the notes and the source release on marigold; starts
# the three build workflows (wallet, desktop, the core for the phones); moves the site's "latest" line; waits for the
# wallet build, attaches the bare `marigold-cli` for the self-update; waits
# for the desktop build, writes the desktop update document if the
# installers were signed; has the trustees sign the release's digests and
# pushes the site. Announcing, the STATE.md row and the local rebuild are the
# releaser's, afterwards.
#
# Clones it needs beside this tree, or named by environment:
#   WALLET_REPO  the marigold-wallet clone   (default ../marigold-wallet)
#   SITE         the marigoldcash.github.io clone (default ../marigoldcash.github.io)
set -euo pipefail
PREV="${1:?previous tag, like v2.87.343}"; NOTES_FILE="${2:?release notes file}"
[ -f "$NOTES_FILE" ] || { echo "no such notes file: $NOTES_FILE" >&2; exit 2; }
NOTES=$(cat "$NOTES_FILE")
ROOT=$(git rev-parse --show-toplevel); cd "$ROOT"
WALLET_REPO="${WALLET_REPO:-$ROOT/../marigold-wallet}"
SITE="${SITE:-$ROOT/../marigoldcash.github.io}"
[ -d "$WALLET_REPO/.git" ] || { echo "wallet repository clone not found at $WALLET_REPO (set WALLET_REPO=)" >&2; exit 2; }
[ -d "$SITE/.git" ] || { echo "site clone not found at $SITE (set SITE=)" >&2; exit 2; }
[ "$(git branch --show-current)" = "main" ] || { echo "release from main, not from $(git branch --show-current)" >&2; exit 2; }
[ -z "$(git status --short --untracked-files=no)" ] || { echo "the tree has uncommitted changes; commit or stash them first" >&2; exit 2; }
R=marigoldcash/marigold; W=marigoldcash/marigold-wallet

scripts/bump-version.sh release >/dev/null
V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
git add Cargo.toml Cargo.lock gui/Cargo.toml gui/tauri.conf.json
git commit -q -m "build: version $V" && git tag "v$V" && git push -q origin main && git push -q origin "v$V" && echo "tagged v$V"

(cd "$WALLET_REPO" && git pull -q && sed -i "s/${PREV}/v$V/g" Dockerfile README.md && git add Dockerfile README.md && git commit -q -m "v$V" && git push -q && echo "wallet repository pushed")

gh release create "v$V" --repo $W --title "v$V" --notes "$NOTES" >/dev/null && echo "wallet release created"
gh release create "v$V" --repo $R --title "v$V" --notes "Source release. Binaries and installers are under marigold-wallet: https://github.com/$W/releases/tag/v$V. The files attached here (marigold-core-*, SHA256SUMS-core.txt) are the wallet core for the phone app, for developers." >/dev/null && echo "source release created"
gh workflow run binaries.yaml --repo $R -f release_tag="v$V" && gh workflow run gui-binaries.yaml --repo $R -f release_tag="v$V" && gh workflow run mobile-core.yaml --repo $R -f release_tag="v$V" && echo "workflows dispatched"

(cd "$SITE" && git pull -q)
SITE="$SITE" scripts/release-notice.sh --latest "$V" 2>&1 | grep -m1 latest
(cd "$SITE" && git add -A && git commit -q -m "release $V" && git push -q && echo "site pushed: latest is $V")
echo "V=$V"

# The builds take a while; the runs just dispatched are the newest of each workflow.
sleep 30
WALLET_RUN=$(gh run list -R $R --workflow binaries.yaml --limit 1 --json databaseId -q ".[0].databaseId")
DESKTOP_RUN=$(gh run list -R $R --workflow gui-binaries.yaml --limit 1 --json databaseId -q ".[0].databaseId")
MOBILE_RUN=$(gh run list -R $R --workflow mobile-core.yaml --limit 1 --json databaseId -q ".[0].databaseId")
w=""; for i in $(seq 1 40); do w=$(gh run view "$WALLET_RUN" -R $R --json status,conclusion -q '.status+"/"+.conclusion'); case "$w" in completed/*) break;; esac; sleep 120; done
echo "wallet run: $w"
if [ "$w" = "completed/success" ]; then
  # The bare `marigold-cli` is what the self-update downloads on Linux.
  d=$(mktemp -d); (cd "$d" && gh release download "v$V" -R $W -p marigold-cli-linux-x86_64 && chmod +x marigold-cli-linux-x86_64 && echo "asset version: $(./marigold-cli-linux-x86_64 --version | head -1)" && cp marigold-cli-linux-x86_64 marigold-cli && gh release upload "v$V" marigold-cli -R $W --clobber && echo "bare marigold-cli attached")
fi
d2=""; for i in $(seq 1 30); do d2=$(gh run view "$DESKTOP_RUN" -R $R --json status,conclusion -q '.status+"/"+.conclusion'); case "$d2" in completed/*) break;; esac; sleep 120; done
echo "desktop run: $d2"; echo "assets on the release: $(gh release view "v$V" -R $W --json assets -q '.assets | length')"
# The wallet core for the phones attaches itself to this repository's release (two zips and their checksums);
# the wallet release does not wait on it, and the phone app takes it from there.
m=""; for i in $(seq 1 30); do m=$(gh run view "$MOBILE_RUN" -R $R --json status,conclusion -q '.status+"/"+.conclusion'); case "$m" in completed/*) break;; esac; sleep 120; done
echo "phone core run: $m; assets on the marigold release: $(gh release view "v$V" -R $R --json assets -q '[.assets[].name] | join(", ")')"
# The desktop's update document, only when the installers carry the updater's signature.
if [ "$d2" = "completed/success" ]; then SITE="$SITE" scripts/desktop-manifest.sh "$V" 2>&1 | tail -1; fi
# The trustees sign the release's digests, and the site carries them: what 'update' installs.
if [ "$w" = "completed/success" ]; then
  d=$(mktemp -d); (cd "$d" && gh release download "v$V" -R $W -p SHA256SUMS-cli.txt)
  SITE="$SITE" scripts/release-notice.sh --latest "$V" --manifest "$d/SHA256SUMS-cli.txt" 2>&1 | grep -m1 '"version"'
  (cd "$SITE" && git add -A && git commit -q -m "release $V: signed manifest" && git push -q && echo "manifest signed and site pushed")
fi
echo "done: v$V. Now: announce (scripts/announce.sh), the STATE.md row, and NO_BUMP=1 scripts/remote-build.sh marigold-cli so the local binary matches the tag."
