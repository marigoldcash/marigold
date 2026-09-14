#!/usr/bin/env bash
#
# Build the browser wallet on the big rig, and fetch it back to be served.
#
#   scripts/remote-build-wasm.sh            # build, fetch into wallet/wasm/web
#   SERVE=8770 scripts/remote-build-wasm.sh # ...and serve it on that port
#
# The browser wallet is the same `kaspa-cli` crate as the terminal one,
# compiled to wasm32 and driven through xterm.js instead of a pty. It is the
# reason the CLI draws in lines rather than repainting a screen, so it is also
# the only honest way to check that anything new still works in both places.
#
# NO_BUMP is set on purpose: this is meant to be run right after
# remote-build.sh, and the two should report the same version rather than the
# browser silently being one ahead of the binary it is supposed to match.
set -euo pipefail

HOST="${MARIGOLD_BUILD_HOST:-root@157.90.15.33}"
KEY="${MARIGOLD_BUILD_KEY:-$HOME/.ssh/marigold_deploy}"
REMOTE_DIR="${MARIGOLD_BUILD_DIR:-/opt/marigold-build/remote-build}"
SSH="ssh -i $KEY -o BatchMode=yes"

cd "$(git rev-parse --show-toplevel)"

echo "→ syncing working tree to $HOST:$REMOTE_DIR"
$SSH "$HOST" "mkdir -p $REMOTE_DIR"
rsync -a --delete --info=stats1 \
  --exclude '/target' --exclude '/.git' --exclude '/.claude' \
  --exclude '/deploy/ansible/.fetched' --exclude '*.pdf' --exclude '/docker/*.tar' \
  --exclude '/whitepaper' --exclude '/upstream' \
  -e "$SSH" ./ "$HOST:$REMOTE_DIR/"

# No embedded node: there is no kaspad inside a browser tab, and the feature
# is not offered for wasm32 in the first place.
echo "→ wasm-pack build --target web"
# shellcheck disable=SC2029
$SSH "$HOST" "source \$HOME/.cargo/env; cd $REMOTE_DIR/wallet/wasm && ./build-web --release 2>&1 | tail -20"

echo "→ fetching the bundle"
mkdir -p wallet/wasm/web/kaspa-wallet
rsync -a --info=stats1 -e "$SSH" \
  "$HOST:$REMOTE_DIR/wallet/wasm/web/kaspa-wallet/" wallet/wasm/web/kaspa-wallet/

VERSION=$(awk '/^\[workspace\.package\]/{f=1} f && /^version = "/{print $3; exit}' Cargo.toml)
echo "✓ done — wallet/wasm/web  ($VERSION)"

if [ -n "${SERVE:-}" ]; then
  # Served rather than opened as a file:// URL — a wasm module loaded from
  # the filesystem is blocked by the same-origin rules in every browser.
  echo "→ serving wallet/wasm/web on http://127.0.0.1:$SERVE/"
  cd wallet/wasm/web
  exec python3 -m http.server "$SERVE" --bind 127.0.0.1
fi
