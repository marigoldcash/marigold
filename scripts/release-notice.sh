#!/usr/bin/env bash
#
# Write the wallet's release document, https://marigold.cash/release.json
# (cli/src/release_check.rs), and print it. It carries the newest wallet
# release and the trustee-signed notices of the oldest release each network
# still allows.
#
#   scripts/release-notice.sh --latest 2.59.260
#       keep every notice as it is, only move the "latest" line
#   scripts/release-notice.sh --latest 2.59.260 --min 2.58 --network testnet-10
#       also collect a fresh testnet notice: one signature per trustee host
#       (the testnet keys sit on seed2 and seed3, see TESTNET_TRUSTEES in
#       consensus/core/src/config/params.rs), replacing that network's notice
#
# Reads the current document from the site clone given by SITE (default
# ../marigoldcash.github.io) and writes it back there; committing and pushing
# the site is left to the caller, so the change can be looked at first.
# Mainnet's trustees sign on their own machines; for them this script is a
# template, not a tool.
set -euo pipefail
LATEST=""; MIN=""; NETWORK=""
SITE="${SITE:-$(git rev-parse --show-toplevel)/../marigoldcash.github.io}"
SIGNER="${SIGNER:-/opt/marigold/bin/marigold-trustee-signer}"
while [ $# -gt 0 ]; do
  case "$1" in
    --latest) LATEST="$2"; shift 2 ;;
    --min) MIN="$2"; shift 2 ;;
    --network) NETWORK="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$LATEST" ] || { echo "--latest MAJOR.RELEASE.BUILD is required" >&2; exit 2; }
[ -d "$SITE" ] || { echo "site clone not found at $SITE (set SITE=)" >&2; exit 2; }
DOC="$SITE/release.json"
[ -f "$DOC" ] || echo '{"latest":"0.0.0","notices":[]}' > "$DOC"

# Which trustee index lives on which host, and under which key file. Testnet
# only: indices 0, 1 (and the cold spare 4) on seed2, 2 and 3 on seed3.
declare -A HOST_OF=([0]=root@78.46.16.56 [1]=root@78.46.16.56 [2]=root@49.12.37.83 [3]=root@49.12.37.83)

SIGNED=""
if [ -n "$MIN" ]; then
  [ -n "$NETWORK" ] || { echo "--min needs --network" >&2; exit 2; }
  ISSUED=$(date +%s)
  for index in 0 1 2 3; do
    host="${HOST_OF[$index]}"
    echo "→ trustee $index signs on $host" >&2
    line=$(ssh -o BatchMode=yes "$host" "sudo -u marigold $SIGNER --trustee-index $index --key-file /etc/marigold/trustee-$index.key --sign-release-notice $MIN --network $NETWORK --issued-at $ISSUED 2>/dev/null" | tail -1)
    SIGNED="$SIGNED$line"$'\n'
  done
fi

python3 - "$DOC" "$LATEST" "$NETWORK" "$SIGNED" <<'PY'
import json, sys
path, latest, network, signed = sys.argv[1:5]
doc = json.load(open(path))
doc["latest"] = latest
if signed.strip():
    lines = [json.loads(l) for l in signed.strip().splitlines()]
    first = lines[0]
    notice = {
        "network": first["network"],
        "min_version": first["min_version"],
        "issued_at": first["issued_at"],
        "signatures": [{"trustee": l["trustee"], "signature": l["signature"]} for l in lines],
    }
    doc["notices"] = [n for n in doc.get("notices", []) if n["network"] != network] + [notice]
json.dump(doc, open(path, "w"), indent=2)
open(path, "a").write("\n")
print(json.dumps(doc, indent=2))
PY
