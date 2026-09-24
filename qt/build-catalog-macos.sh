#!/usr/bin/env bash
# Mac build for Catalog (mirrors adblink: static Qt, universal app,
# ctest). First run builds both Rust arches + lipo (~2 min); incremental
# after that. Run from anywhere. Pass --clean for a fresh build (wipes
# the CMake build dir + universal FFI lib; cargo target/ stays incremental).
set -euo pipefail

SRC="/Users/jeff/source/catalog"
QTREE="$SRC/qt"
BLD="/Users/jeff/source/builds/catalog"
FFI="$BLD/libcatalog_ffi.a"
CMAKE="/opt/homebrew/bin/cmake"
CTEST="/opt/homebrew/bin/ctest"

if [[ "${1:-}" == "--clean" ]]; then
    rm -rf "$BLD"
fi
mkdir -p "$BLD"

cargo build --release -p catalog-ffi --target aarch64-apple-darwin \
    --manifest-path "$SRC/Cargo.toml"
cargo build --release -p catalog-ffi --target x86_64-apple-darwin \
    --manifest-path "$SRC/Cargo.toml"
lipo -create \
    "$SRC/target/aarch64-apple-darwin/release/libcatalog_ffi.a" \
    "$SRC/target/x86_64-apple-darwin/release/libcatalog_ffi.a" \
    -output "$FFI"

"$CMAKE" -S "$QTREE" -B "$BLD" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="14.0" \
    -DFFI_LIB="$FFI"
"$CMAKE" --build "$BLD"
# Local Network prompt: without NSLocalNetworkUsageDescription macOS never
# asks and LAN SMB fails (EHOSTUNREACH on fresh installs). CMake regenerates
# Info.plist on reconfigure, so (re-)apply it every build (idempotent).
/usr/libexec/PlistBuddy -c "Delete :NSLocalNetworkUsageDescription" \
    "$BLD/JocalaCatalog.app/Contents/Info.plist" >/dev/null 2>&1 || true
/usr/libexec/PlistBuddy -c "Add :NSLocalNetworkUsageDescription string 'Catalog connects to your Calibre library over the local network (SMB) and to your Kobo reader over WiFi.'" \
    "$BLD/JocalaCatalog.app/Contents/Info.plist"
"$CTEST" --test-dir "$BLD" --output-on-failure
