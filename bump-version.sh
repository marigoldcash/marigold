#!/usr/bin/env bash
# Raise the workspace version by one patch, or set it to an argument.
#
#   ./bump-version.sh          2.0.1 -> 2.0.2
#   ./bump-version.sh 2.1.0    straight to 2.1.0
#
# The version lives in sixty-four places: once under [workspace.package], and
# again in every internal `{ version = "...", path = "..." }` entry, which cargo
# requires to match. Editing them by hand is how a workspace ends up
# unbuildable, so this rewrites exactly those two shapes and nothing else — an
# external crate that happens to share our version number is left alone.
set -euo pipefail

cd "$(dirname "$0")"

current=$(awk '/^\[workspace\.package\]/{f=1} f && /^version = "/{gsub(/[":]/,"",$3); print $3; exit}' Cargo.toml)
if [[ -z "$current" ]]; then
    echo "cannot find the workspace version in Cargo.toml" >&2
    exit 1
fi

if [[ $# -ge 1 ]]; then
    next="$1"
else
    IFS=. read -r major minor patch <<<"$current"
    if [[ -z "${patch:-}" ]]; then
        echo "workspace version '$current' is not major.minor.patch" >&2
        exit 1
    fi
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
print(f"rewrote {changed} version entries")
PY

# Refresh Cargo.lock so the tree is consistent — without it the next build
# rewrites the lock file anyway, and it lands in whatever commit comes next.
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null 2>&1 || true

echo "$current -> $next"
