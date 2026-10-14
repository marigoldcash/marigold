#!/usr/bin/env bash
#
# Pack what scripts/mobile-core.sh and scripts/mobile-bindings.sh made into the file an app downloads.
#
#   scripts/mobile-pack.sh android   # marigold-core-<version>-android.zip: jniLibs/ and kotlin/
#   scripts/mobile-pack.sh ios       # marigold-core-<version>-ios.zip: MarigoldCore.xcframework and swift/
#
# <version> is the workspace version, which is the release tag without its v. The zip lands in
# target/mobile-core, and its listing is printed so a log shows what is in it.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=$PWD/target/mobile-core
BIND=$PWD/target/mobile-bindings
case "${1:-}" in
  android)
    [ -d "$OUT/jniLibs" ] && [ -d "$BIND/kotlin" ] || { echo "run scripts/mobile-bindings.sh and scripts/mobile-core.sh android first" >&2; exit 1; }
    ZIP=$OUT/marigold-core-$VERSION-android.zip
    STAGE=$(mktemp -d)
    cp -R "$OUT/jniLibs" "$STAGE/jniLibs"
    cp -R "$BIND/kotlin" "$STAGE/kotlin"
    ;;
  ios)
    [ -d "$OUT/MarigoldCore.xcframework" ] && [ -d "$BIND/swift" ] || { echo "run scripts/mobile-bindings.sh and scripts/mobile-core.sh ios first" >&2; exit 1; }
    ZIP=$OUT/marigold-core-$VERSION-ios.zip
    STAGE=$(mktemp -d)
    cp -R "$OUT/MarigoldCore.xcframework" "$STAGE/MarigoldCore.xcframework"
    mkdir "$STAGE/swift" && cp "$BIND/swift/MarigoldCore.swift" "$STAGE/swift/"
    ;;
  *) echo "usage: scripts/mobile-pack.sh android|ios" >&2; exit 2 ;;
esac
echo "$VERSION" > "$STAGE/VERSION"
rm -f "$ZIP"
# Python's zipfile rather than zip(1), which a build computer may not have; sorted, with fixed times, so two
# packs of the same files are the same bytes.
python3 - "$STAGE" "$ZIP" <<'PY'
import os, sys, zipfile
stage, out = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for root, dirs, files in os.walk(stage):
        dirs.sort()
        for name in sorted(files):
            path = os.path.join(root, name)
            info = zipfile.ZipInfo(os.path.relpath(path, stage), date_time=(2026, 1, 1, 0, 0, 0))
            info.external_attr = (os.stat(path).st_mode & 0o777) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(path, "rb") as f:
                z.writestr(info, f.read())
    for info in z.infolist():
        print(f"{info.file_size:>10}  {info.filename}")
PY
