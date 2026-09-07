#!/usr/bin/env bash
#
# Build on the big rig instead of here.
#
#   scripts/remote-build.sh                    # release build of marigold-cli
#   scripts/remote-build.sh marigoldd          # a different binary
#   scripts/remote-build.sh marigold-cli marigoldd
#   CHECK=1 scripts/remote-build.sh            # cargo check only (no binaries fetched)
#
# Mirrors the working tree to a build directory on the rig, compiles there,
# and copies the binaries back into ./target/release. The remote target/
# directory persists between runs, so after the first build these are
# incremental — the point of the exercise.
#
# Deliberately NOT the same checkout the Ansible build role uses
# (/opt/marigold-build/marigold-node): deploys must stay reproducible from
# git, never from whatever happens to be in someone's editor.
set -euo pipefail

HOST="${MARIGOLD_BUILD_HOST:-root@157.90.15.33}"
KEY="${MARIGOLD_BUILD_KEY:-$HOME/.ssh/marigold_deploy}"
REMOTE_DIR="${MARIGOLD_BUILD_DIR:-/opt/marigold-build/remote-build}"
SSH="ssh -i $KEY -o BatchMode=yes"

cd "$(git rev-parse --show-toplevel)"
BINS=("$@")
[ ${#BINS[@]} -eq 0 ] && BINS=(marigold-cli)

echo "→ syncing working tree to $HOST:$REMOTE_DIR"
$SSH "$HOST" "mkdir -p $REMOTE_DIR"
rsync -a --delete --info=stats1 \
  --exclude '/target' --exclude '/.git' --exclude '/.claude' \
  --exclude '/deploy/ansible/.fetched' --exclude '*.pdf' --exclude '/docker/*.tar' \
  --exclude '/whitepaper' --exclude '/upstream' \
  -e "$SSH" ./ "$HOST:$REMOTE_DIR/"

BIN_ARGS=""
for bin in "${BINS[@]}"; do BIN_ARGS="$BIN_ARGS --bin $bin"; done

if [ -n "${CHECK:-}" ]; then
  echo "→ cargo check on $(basename "$HOST") (96 cores)"
  # shellcheck disable=SC2029
  $SSH "$HOST" "cd $REMOTE_DIR && cargo check --release $BIN_ARGS 2>&1 | tail -30"
  exit 0
fi

# The wallet ships with a node compiled in. It costs build time (the whole
# consensus tree) and ~30 MB, and it is what lets a user hold their own notes
# without telling anyone which they are. Pass NO_EMBEDDED_NODE=1 to skip it
# while iterating on wallet code.
FEATURES=""
if [ -z "${NO_EMBEDDED_NODE:-}" ]; then
  FEATURES="--features embedded-node"
fi
echo "→ cargo build --release$BIN_ARGS $FEATURES"
# shellcheck disable=SC2029
$SSH "$HOST" "cd $REMOTE_DIR && cargo build --release $BIN_ARGS $FEATURES 2>&1 | tail -30"

echo "→ fetching binaries"
mkdir -p target/release
for bin in "${BINS[@]}"; do
  rsync -a --info=name -e "$SSH" "$HOST:$REMOTE_DIR/target/release/$bin" "target/release/$bin"
done
echo "✓ done — target/release/${BINS[*]}"
