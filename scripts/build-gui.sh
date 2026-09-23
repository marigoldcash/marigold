#!/usr/bin/env bash
#
# Build the desktop wallet (gui/) on the big rig, like scripts/remote-build.sh
# does for the node and the terminal wallet, and copy the binary back to
# gui/target/release/marigold-wallet. The GUI is its own workspace (gui/Cargo.toml
# says why), so the build runs from that directory on the rig; the working
# tree is synced first through remote-build.sh's own rsync (CHECK mode, which
# compiles nothing new once the tree is checked).
#
#   scripts/build-gui.sh            # sync and build
#   NO_BUMP=1 scripts/build-gui.sh  # keep the version (a rebuild of the same source)
set -euo pipefail
HOST="${MARIGOLD_BUILD_HOST:-pve3}"
KEY="${MARIGOLD_BUILD_KEY:-$HOME/.ssh/marigold_deploy}"
REMOTE_DIR="${MARIGOLD_BUILD_DIR:-/opt/marigold-build/remote-build}"
SSH="ssh -i $KEY -o BatchMode=yes"
cd "$(git rev-parse --show-toplevel)"
if [ -z "${NO_BUMP:-}" ]; then
  echo "→ $(scripts/bump-version.sh)"
fi
NO_BUMP=1 CHECK=1 scripts/remote-build.sh marigold-cli | grep -E "^→ syncing|Finished" || true
echo "→ cargo build --release in gui/ on $HOST"
# shellcheck disable=SC2029
$SSH "$HOST" "set -o pipefail; cd $REMOTE_DIR/gui && cargo build --release --jobs \$(nproc) 2>&1 | grep -vE '^\s+(Compiling|Downloaded|Downloading|Updating|Locking|Adding)' | tail -30"
mkdir -p gui/target/release
rsync -a --info=name -e "$SSH" "$HOST:$REMOTE_DIR/gui/target/release/marigold-wallet" gui/target/release/marigold-wallet
echo "✓ done — gui/target/release/marigold-wallet ($(grep -m1 '^version' gui/Cargo.toml))"
