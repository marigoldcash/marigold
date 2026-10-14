#!/usr/bin/env bash
#
# Link the iOS library the way an app does and run it: a few lines of Swift, compiled against the
# simulator slice of the XCFramework and the generated Swift binding, print the core's version.
# Needs macOS with Xcode, scripts/mobile-bindings.sh and scripts/mobile-core.sh ios run first.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
SLICE=target/mobile-core/MarigoldCore.xcframework/ios-arm64-simulator
BIND=target/mobile-bindings/swift
WORK=target/mobile-core/swift-check
SDK=$(xcrun --sdk iphonesimulator --show-sdk-path)
EXPECT=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

rm -rf "$WORK"; mkdir -p "$WORK"
cat > "$WORK/main.swift" <<'S'
print("marigold core \(coreVersion())")
S
swiftc -target arm64-apple-ios15.1-simulator -sdk "$SDK" -module-name Check \
  -I "$SLICE/Headers" "$WORK/main.swift" "$BIND/MarigoldCore.swift" \
  -L "$SLICE" -lkaspa_wallet_service -lresolv -framework Security -framework SystemConfiguration -framework CoreFoundation \
  -o "$WORK/check"
echo "linked: $(file "$WORK/check" | cut -d: -f2)"

# Run it inside an iOS simulator, as an app would be.
UDID=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
for runtime, devices in json.load(sys.stdin)["devices"].items():
    if "iOS" in runtime:
        for d in devices:
            if d["name"].startswith("iPhone"):
                print(d["udid"]); sys.exit(0)
sys.exit(1)')
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b >/dev/null
OUT=$(xcrun simctl spawn "$UDID" "$PWD/$WORK/check")
echo "$OUT"
xcrun simctl shutdown "$UDID" || true
[ "$OUT" = "marigold core $EXPECT" ] || { echo "expected 'marigold core $EXPECT'" >&2; exit 1; }
