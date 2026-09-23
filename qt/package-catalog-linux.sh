#!/usr/bin/env bash
# Package Catalog for Linux (cpack TGZ, after a successful build).
# Run from anywhere.
set -euo pipefail

BUILD_DIR="/home/jeff/build-catalog"

cpack --config "$BUILD_DIR/CPackConfig.cmake" -B "$BUILD_DIR/packages"
ls -lh "$BUILD_DIR/packages/"
