#!/usr/bin/env bash
# Package CatalogQt for macOS (cpack DragNDrop, after a successful build).
# Sign + notarize stays open (see AGENTS.md) — this produces the unsigned
# local DMG only. Run from anywhere.
set -euo pipefail

CPACK="/opt/homebrew/bin/cpack"
BUILD_DIR="/Users/jeff/source/builds/catalogqt"

$CPACK --config "$BUILD_DIR/CPackConfig.cmake" -B "$BUILD_DIR/packages"
ls -lh "$BUILD_DIR/packages/"
