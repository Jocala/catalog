# Jocala Catalog — SwiftUI verdict (`com.jocala.catalog`)

Fork of `reader/macos/` shell (`JReaderMacOS` → `ReaderCatalogGUI`), re-pointed at
the Catalog product. Replaces frozen `crates/catalog-egui` (perf verdict).

## Why
egui required a hand-rolled perf stack (per-visible-row resolution, in-flight
fetch guard, `is_rect_visible` gating, worker decode, 4-uploads/frame cap,
256-texture LRU + `forget_image`, `TableBuilder` virtualization) and still had
uncharacterized LRU eviction + scroll-stress on the 6755-book library.
SwiftUI gets this for free: `LazyVGrid` virtualizes, per-cell `task{}` loads,
`NSCache<NSString, NSImage>` replaces the texture LRU.

## Data path — FORKED, REAL (2026-09-19)
`Sources/CatalogCore/` is a fork of `macos/Sources/CatalogCore`
(Services + Models + Calibre + vendored SMBClient/ZIPFoundation) with two
identity changes: data root `~/Library/Application Support/com.jocala.Catalog/`
(`CatalogPaths`, was `~/.jreader`) and Keychain service
`com.jocala.catalog.smb` (was `com.jocala.reader.smb`).
`Sources/CatalogSwiftApp/` is the `macos` GUI fork (`CatalogStore`,
`SmbCatalogDB` single-DB `:memory:`, `ThumbnailService` + 6-gate throttler,
`SearchFormSheet`, Settings, detail, Kobo fail-closed). Title "Jocala Catalog".
Fixture scaffold removed — the grid reads the live library via Settings.

## Build — .app in build/ (mirrors macos rule, flattened 2026-09-20:
this dir IS the Swift package, ex-`catalog-swift/`)
```sh
swift build -c release --scratch-path /Users/jeff/source/catalog/build \
  --package-path /Users/jeff/source/catalog
# Binary: build/arm64-apple-macosx/release/CatalogSwift
# .app bundle inputs (canonical, in this dir):
#   Info.plist (CFBundleExecutable=CatalogSwift, com.jocala.catalog)
#   CatalogSwift.entitlements (network.client only)
#   Resources/AppIcon.icns
# Assemble + Dev-ID sign (ad-hoc banned for LAN apps — loses grants every rebuild):
#   mkdir -p "build/Jocala Catalog.app/Contents/"{MacOS,Resources}
#   cp build/arm64-apple-macosx/release/CatalogSwift "build/Jocala Catalog.app/Contents/MacOS/CatalogSwift"
#   cp Info.plist "build/Jocala Catalog.app/Contents/Info.plist"
#   cp Resources/AppIcon.icns "build/Jocala Catalog.app/Contents/Resources/AppIcon.icns"
#   printf 'APPL????' > "build/Jocala Catalog.app/Contents/PkgInfo"
#   codesign --force --deep --options runtime --timestamp \
#     --entitlements CatalogSwift.entitlements \
#     --sign "Developer ID Application: jeff elkins" "build/Jocala Catalog.app"
# Notarize (always, unless user says otherwise; profile notary-jeff):
#   ditto -c -k --keepParent "build/Jocala Catalog.app" /tmp/JocalaCatalog-submit.zip
#   xcrun notarytool submit /tmp/JocalaCatalog-submit.zip --keychain-profile "notary-jeff" --wait
#   xcrun stapler staple "build/Jocala Catalog.app"
#   spctl -a -vv "build/Jocala Catalog.app"  # accepted, source=Notarized Developer ID
# Result (2026-09-19, notarized 2026-09-20): build/Jocala Catalog.app,
# Dev-ID signed + notarized (Team 9Q77WK7W3R, network.client=true, profile
# notary-jeff). Always sign + notarize unless user says otherwise.
# Run:
#   open "/Users/jeff/source/catalog/build/Jocala Catalog.app"
```
Do NOT copy to `~/Desktop`. Bundle ID `com.jocala.catalog`.
Entitlements: `network.client=true` only (SMB/Kobo). `Info.plist` must carry
`NSLocalNetworkUsageDescription` or every `NWConnection` fails with posix 50.

## Settings / data
Mirror `macos/` simplified settings (5 fields + Kobo list + Reindex), stored in
`~/Library/Application Support/com.jocala.Catalog/` (never `~/.jreader/`,
never hardcoded hosts). Passwords masked; Kobo fail-closed (0 or >1 → dialog).
