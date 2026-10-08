#!/usr/bin/env bash
#
# Write the desktop wallet's update document, https://marigold.cash/desktop.json,
# from a release's signed installers: for each platform the installer's URL and
# the minisign signature the build left beside it (gui-binaries.yaml, with the
# updater key in the secrets). The app's updater (gui/tauri.conf.json
# plugins.updater) reads this document, verifies the signature with the
# compiled-in public key, installs and restarts.
#
#   SITE=../marigoldcash.github.io scripts/desktop-manifest.sh 2.80.324
#
# Platforms whose signature is missing from the release are left out, so a
# release built before the key existed writes nothing for them. Committing and
# pushing the site is left to the caller.
set -euo pipefail
V="${1:?version MAJOR.RELEASE.BUILD}"
SITE="${SITE:-$(git rev-parse --show-toplevel)/../marigoldcash.github.io}"
R=marigoldcash/marigold-wallet
BASE="https://github.com/$R/releases/download/v$V"
d=$(mktemp -d); trap 'rm -rf "$d"' EXIT
gh release download "v$V" -R "$R" -p '*.sig' -D "$d" 2>/dev/null || true
python3 - "$d" "$V" "$BASE" "$SITE/desktop.json" <<'PY'
import json, os, sys, datetime
d, v, base, out = sys.argv[1:5]
# Tauri's platform keys → the release's file names.
platforms = {
    "linux-x86_64": f"marigold-wallet-{v}-linux-x86_64.AppImage",
    "linux-aarch64": f"marigold-wallet-{v}-linux-arm64.AppImage",
    "darwin-aarch64": f"marigold-wallet-{v}-macos-arm64.app.tar.gz",
    "darwin-x86_64": f"marigold-wallet-{v}-macos-x86_64.app.tar.gz",
    "windows-x86_64": f"marigold-wallet-{v}-windows-x86_64.msi",
}
found = {}
for key, name in platforms.items():
    sig = os.path.join(d, name + ".sig")
    if os.path.exists(sig):
        found[key] = {"url": f"{base}/{name}", "signature": open(sig).read().strip()}
if not found:
    print(f"no signed installers on v{v}: desktop.json not written", file=sys.stderr)
    sys.exit(0)
doc = {"version": v, "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
       "notes": f"Marigold {v}", "platforms": found}
json.dump(doc, open(out, "w"), indent=2); open(out, "a").write("\n")
print(f"desktop.json: {v}, {', '.join(sorted(found))}")
PY
