#!/usr/bin/env bash
# Mac build for CatalogQt (mirrors adblink: static Qt, universal app,
# ctest). First run builds both Rust arches + lipo (~2 min); incremental
# after that. Run from anywhere.
set -euo pipefail

SRC="/Users/jeff/source/catalog"
QTREE="$SRC/qt"
BLD="/Users/jeff/source/builds/catalogqt"
FFI="$BLD/libcatalog_ffi.a"
CMAKE="/opt/homebrew/bin/cmake"
CTEST="/opt/homebrew/bin/ctest"

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
"$CTEST" --test-dir "$BLD" --output-on-failure
