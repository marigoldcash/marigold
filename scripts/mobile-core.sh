#!/usr/bin/env bash
#
# Build the wallet core (kaspa-wallet-service) for the phones.
#
#   scripts/mobile-core.sh android   # Linux: needs the Android NDK (ANDROID_NDK_HOME)
#   scripts/mobile-core.sh ios       # macOS: needs Xcode
#
# Android writes target/mobile-core/jniLibs/<abi>/libkaspa_wallet_service.so for the phone (arm64-v8a)
# and the emulator (x86_64). iOS writes target/mobile-core/MarigoldCore.xcframework with a slice for
# the iPhone and one for the simulator on Apple silicon. The crate stays an ordinary library for every
# other build: the library kinds the phones need are asked for here, on the command line.
#
# The Rust targets come from rust-toolchain.toml. Phone apps load these files; they never compile Rust
# (docs/marigold/MOBILE-PLAN.md, PH-M3 and PH-M4).
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
OUT=target/mobile-core
CRATE=kaspa-wallet-service
LIB=libkaspa_wallet_service
SYMBOL=marigold_core_version
# The oldest Android and iOS the apps support.
ANDROID_API=24
IOS_MIN=15.1

android() {
  local ndk="${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the Android NDK}"
  local host
  case "$(uname -s)" in Linux) host=linux-x86_64 ;; Darwin) host=darwin-x86_64 ;; *) echo "no NDK host for $(uname -s)" >&2; exit 1 ;; esac
  local bin="$ndk/toolchains/llvm/prebuilt/$host/bin"
  rm -rf "$OUT/jniLibs"
  # Android 15 devices page memory in 16 KB units, and Google Play wants libraries aligned for it.
  local flags="-C link-arg=-Wl,-z,max-page-size=16384"
  for pair in aarch64-linux-android:arm64-v8a x86_64-linux-android:x86_64; do
    local target=${pair%%:*} abi=${pair##*:}
    local up=${target//-/_}; up=${up^^}
    local lo=${target//-/_}
    echo "→ $target ($abi)"
    env "CC_$lo=$bin/${target}${ANDROID_API}-clang" "AR_$lo=$bin/llvm-ar" \
        "CARGO_TARGET_${up}_LINKER=$bin/${target}${ANDROID_API}-clang" \
        "CARGO_TARGET_${up}_RUSTFLAGS=$flags" \
      cargo rustc --release --locked -p "$CRATE" --lib --features ffi --target "$target" --crate-type cdylib
    mkdir -p "$OUT/jniLibs/$abi"
    cp "target/$target/release/$LIB.so" "$OUT/jniLibs/$abi/"
    local symbols
    symbols=$("$bin/llvm-nm" -D "$OUT/jniLibs/$abi/$LIB.so")
    grep -q " T $SYMBOL" <<<"$symbols" || { echo "$SYMBOL missing from $abi" >&2; exit 1; }
  done
  find "$OUT/jniLibs" -name '*.so' -exec ls -l {} +
}

ios() {
  command -v xcodebuild >/dev/null || { echo "xcodebuild not found: this part needs macOS with Xcode" >&2; exit 1; }
  export IPHONEOS_DEPLOYMENT_TARGET="$IOS_MIN"
  rm -rf "$OUT/ios" "$OUT/MarigoldCore.xcframework"
  mkdir -p "$OUT/ios/include"
  cat > "$OUT/ios/include/marigold_core.h" <<H
// The Marigold wallet core. The interface the app calls arrives with PH-M3; this is the one symbol PH-M2 proves links.
const char *marigold_core_version(void);
H
  cat > "$OUT/ios/include/module.modulemap" <<H
module MarigoldCore {
    header "marigold_core.h"
    export *
}
H
  local args=()
  for target in aarch64-apple-ios aarch64-apple-ios-sim; do
    echo "→ $target"
    cargo rustc --release --locked -p "$CRATE" --lib --features ffi --target "$target" --crate-type staticlib
    local symbols
    symbols=$(nm -gU "target/$target/release/$LIB.a" 2>&1 || true)
    grep -q " T _$SYMBOL" <<<"$symbols" || { echo "$SYMBOL missing from $target; nm printed:" >&2; head -20 <<<"$symbols" >&2; exit 1; }
    args+=(-library "target/$target/release/$LIB.a" -headers "$OUT/ios/include")
  done
  xcodebuild -create-xcframework "${args[@]}" -output "$OUT/MarigoldCore.xcframework"
  find "$OUT/MarigoldCore.xcframework" -name '*.a' -exec ls -l {} +
}

case "${1:-}" in
  android) android ;;
  ios) ios ;;
  *) echo "usage: scripts/mobile-core.sh android|ios" >&2; exit 2 ;;
esac
