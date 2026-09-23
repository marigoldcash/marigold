#!/usr/bin/env bash
# Raise the workspace version by one build, cut a release, or set it outright.
#
#   ./bump-version.sh            2.45.208 -> 2.45.209   (every build)
#   ./bump-version.sh release    2.45.209 -> 2.46.210   (a release is a build too)
#   ./bump-version.sh 2.45.208   straight to 2.45.208
#
# The number reads major.release.build (founder, 2026-09-20): the first
# figure is the era — 2 for testnet, 3 from mainnet — the second counts
# releases, the third counts builds and never resets, so any binary still
# traces to its "build: version" commit. A release is tagged with the
# binary's own version, v2.46.210, in both repositories, so the front note,
# `marigold-cli --version`, the release page and the Dockerfile all read the
# same thing.
#
# The version lives in sixty-four places: once under [workspace.package], and
# again in every internal `{ version = "...", path = "..." }` entry, which cargo
# requires to match. Editing them by hand is how a workspace ends up
# unbuildable, so this rewrites exactly those two shapes and nothing else — an
# external crate that happens to share our version number is left alone.
set -euo pipefail

cd "$(dirname "$0")/.."

current=$(awk '/^\[workspace\.package\]/{f=1} f && /^version = "/{gsub(/[":]/,"",$3); print $3; exit}' Cargo.toml)
if [[ -z "$current" ]]; then
    echo "cannot find the workspace version in Cargo.toml" >&2
    exit 1
fi

IFS=. read -r major minor patch <<<"$current"
if [[ -z "${patch:-}" ]]; then
    echo "workspace version '$current' is not major.release.build" >&2
    exit 1
fi
if [[ $# -ge 1 && "$1" == "release" ]]; then
    next="$major.$((minor + 1)).$((patch + 1))"
elif [[ $# -ge 1 ]]; then
    next="$1"
else
    next="$major.$minor.$((patch + 1))"
fi

if [[ "$next" == "$current" ]]; then
    echo "already at $current"
    exit 0
fi

python3 - "$current" "$next" <<'PY'
import re, sys
current, next_ = sys.argv[1], sys.argv[2]
path = "Cargo.toml"
out, in_workspace_package, changed = [], False, 0
for line in open(path):
    if line.startswith("["):
        in_workspace_package = line.strip() == "[workspace.package]"
    # The workspace's own version, and the internal path dependencies that
    # must agree with it. Nothing else.
    if (in_workspace_package and line.startswith(f'version = "{current}"')) or (
        f'version = "{current}"' in line and "path = " in line
    ):
        line = line.replace(f'version = "{current}"', f'version = "{next_}"')
        changed += 1
    out.append(line)
open(path, "w").write("".join(out))
# The desktop wallet is its own workspace and cannot inherit the version;
# its manifest and Tauri configuration carry the same number by hand.
for extra in ("gui/Cargo.toml", "gui/tauri.conf.json"):
    try:
        text = open(extra).read()
    except FileNotFoundError:
        continue
    replaced = text.replace(f'version = "{current}"', f'version = "{next_}"', 1).replace(f'"version": "{current}"', f'"version": "{next_}"', 1)
    if replaced != text:
        open(extra, "w").write(replaced)
        changed += 1
print(f"rewrote {changed} version entries")
PY

# Refresh Cargo.lock so the tree is consistent — without it the next build
# rewrites the lock file anyway, and it lands in whatever commit comes next.
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null 2>&1 || true

echo "$current -> $next"
