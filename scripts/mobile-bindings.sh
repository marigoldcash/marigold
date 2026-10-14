#!/usr/bin/env bash
#
# Write the Swift and Kotlin bindings of the wallet core.
#
#   scripts/mobile-bindings.sh [output-dir]      # default: target/mobile-bindings
#
# The bindings are not checked in: they are produced from the compiled core, so they always match it.
# The core is built once for this computer, and uniffi-bindgen reads the interface out of that library
# (the `ffi` feature of kaspa-wallet-service, wallet/service/src/ffi.rs). Swift and Kotlin do not depend
# on the computer the interface was read on, so any host will do. A second run produces byte-identical
# files, and this script says so by running the generator twice and comparing checksums.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
OUT="${1:-target/mobile-bindings}"
CRATE=kaspa-wallet-service
case "$(uname -s)" in Darwin) EXT=dylib ;; *) EXT=so ;; esac
# The development profile: the release profile strips the symbols the generator reads the interface from.
LIB="target/debug/libkaspa_wallet_service.$EXT"

cargo rustc --locked -p "$CRATE" --lib --features ffi --crate-type cdylib
cargo build --locked -p marigold-uniffi-bindgen

generate() {
  local dir=$1
  rm -rf "$dir"
  mkdir -p "$dir/swift" "$dir/kotlin"
  # --no-format: the generator would otherwise call swiftformat or ktlint when they happen to be installed,
  # and the output would depend on which computer made it.
  target/debug/uniffi-bindgen generate --library "$LIB" --language swift --no-format --out-dir "$dir/swift"
  target/debug/uniffi-bindgen generate --library "$LIB" --language kotlin --no-format --out-dir "$dir/kotlin"
}

sums() { (cd "$1" && find . -type f | LC_ALL=C sort | xargs sha256sum); }

generate "$OUT"
generate "$OUT.again"
if ! diff <(sums "$OUT") <(sums "$OUT.again") >/dev/null; then
  echo "two runs of the generator gave different files:" >&2
  diff <(sums "$OUT") <(sums "$OUT.again") >&2 || true
  exit 1
fi
rm -rf "$OUT.again"
echo "the bindings, byte-identical on a second run:"
sums "$OUT"
