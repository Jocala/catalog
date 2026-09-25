#!/usr/bin/env bash
# Package Catalog for macOS: sign the app, CPack DragNDrop (re-signs the
# staged app), notarize the DMG, staple it. Mirrors adblink. Run from
# anywhere, after a successful build.
set -euo pipefail

SCRIPT_DIR="$(dirname "$0")"
CPACK="/opt/homebrew/bin/cpack"
CODESIGN_IDENTITY="Developer ID Application: jeff elkins (9Q77WK7W3R)"

BUILD_DIR="/Users/jeff/source/builds/catalog"

codesign --force --deep --sign "$CODESIGN_IDENTITY" --timestamp \
  --options=runtime \
  --entitlements "${SCRIPT_DIR}/packaging/catalog.entitlements" \
  "$BUILD_DIR/JocalaCatalog.app"
$CPACK --config "$BUILD_DIR/CPackConfig.cmake" \
  -D CPACK_PRE_BUILD_SCRIPTS="${SCRIPT_DIR}/packaging/sign-after-install.cmake" \
  -D ENTITLEMENTS_PATH="${SCRIPT_DIR}/packaging/catalog.entitlements" \
  -B "$BUILD_DIR/packages"

NOTARY_PROFILE="notary-jeff"
DMG="$BUILD_DIR/packages/catalog-1.03-Darwin.dmg"
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
ls -lh "$BUILD_DIR/packages/"
