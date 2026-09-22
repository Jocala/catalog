#!/usr/bin/env bash
# Linux build for CatalogQt (mirrors adblink: cmake + build + ctest).
# System Qt via qt6-base-dev (no CMAKE_PREFIX_PATH). Headless box, so
# ctest runs offscreen. Run from anywhere. Pass --clean for a fresh
# build (wipes the CMake build dir; cargo target/ stays incremental).
set -euo pipefail

SRC="/zstore/source/catalog"
QTREE="$SRC/qt"
BLD="/home/jeff/build-catalogqt"
FFI="$SRC/target/release/libcatalog_ffi.a"

if [[ "${1:-}" == "--clean" ]]; then
    rm -rf "$BLD"
fi

cargo build --release -p catalog-ffi --manifest-path "$SRC/Cargo.toml"

cmake -S "$QTREE" -B "$BLD" \
    -DCMAKE_BUILD_TYPE=Release \
    -DFFI_LIB="$FFI"
cmake --build "$BLD"
QT_QPA_PLATFORM=offscreen ctest --test-dir "$BLD" --output-on-failure
