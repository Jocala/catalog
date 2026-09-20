# Jocala Catalog (`com.jocala.catalog`)

SwiftUI app. This dir IS the Swift package (flattened 2026-09-20, ex-`catalog-swift/`).
Rust workspace restored 2026-09-20 from `~/Desktop/zzz/` (`crates/`, `Cargo.toml`,
`Cargo.lock`) — lives alongside, not wired into the Swift build. Second copy in
nightly source tarballs (`/Users/jeff/source/backups/`, e.g. `e34e6e0d.tar` 2026-09-19).

## Product identity
- **Name:** Jocala Catalog · **ID:** `com.jocala.catalog`
- Settings/data live under `~/Library/Application Support/com.jocala.Catalog/` on macOS.
  Never hardcode hosts, credentials, or paths.
- Credentials come from Settings at runtime only; passwords are masked everywhere.

## Layout
- `Sources/CatalogCore/` — fork of `reader/macos` core (Services + Models + Calibre + vendored SMBClient/ZIPFoundation); data root + Keychain service re-pointed at catalog
- `Sources/CatalogSwiftApp/` — SwiftUI app (`CatalogStore`, `SmbCatalogDB` single-DB `:memory:`, `ThumbnailService`, search, Settings, detail, Kobo fail-closed)
- `build/` — SwiftPM scratch-path + `Jocala Catalog.app` (sole product; never copy to `~/Desktop`)
- `Package.swift` (`JocalaCatalogSwift`, macOS 14+, product `CatalogSwift`), `Info.plist`, `CatalogSwift.entitlements` (`network.client` only), `Resources/AppIcon.icns`
- `crates/catalog-core/` — GUI-free Rust business logic (the only thing UI crates may depend on); `crates/catalog-core/tests/fixtures/` holds the 4-book `metadata.db` + `make-fixture.sql`
- `crates/catalog-ffi/` — C ABI over core (`catalog_ffi` DLL/dylib, JSON in/out) for native shells; 6 smoke tests vs fixture
- `crates/catalog-cli/` — `catalog-cli` acceptance harness: `library open|list|detail`, `import-zip` (zip-slip safe), `smb ls|get`, `settings get|set|path` (CLI prints `(set)`/`(empty)`, never values)
- `crates/catalog-egui/` — **FROZEN** egui/eframe UI (perf verdict → SwiftUI; kept on disk, delisted from workspace). Mitigations archived as reference: per-visible-row resolution, in-flight fetch guard, visibility-gated fetches, worker decode, 4-uploads/frame cap, 256-texture LRU, TableBuilder virtualization. `[reload]`/`[covers]` stderr lines live here only — never carry into Swift UI
- `crates/catalog-macos/` — **FROZEN** AppKit/objc2 attempt (reference only, do not build on)
- Not restored: `target/` (cargo artifacts, regenerable), pre-flatten `build/` (old egui bundle staging — still in `~/Desktop/zzz/build/`), zzz-era `AGENTS.md` (superseded by this file)

## Signing (macOS)
- **Always sign + notarize unless user says otherwise** (profile `notary-jeff`, shared with Reader).
  Canonical flags + verify: `~/.config/opencode/AGENTS.md` § Apple Developer ID signing + notarization.
  Assemble + sign + notarize steps: `README.md` § Build.
- Release binary: `build/arm64-apple-macosx/release/CatalogSwift` →
  `build/Jocala Catalog.app/Contents/MacOS/CatalogSwift`.
- Proven 2026-09-20 release: Dev-ID signed + stapled `accepted, source=Notarized Developer ID`.

## Conventions
- No hardcoded hosts/ports/credentials; keep diffs small.
- Re-sign + notarize `.app` after ANY bundle change (binary, plist, icns).
- `Info.plist` must carry `NSLocalNetworkUsageDescription` (else `NWConnection` fails posix 50).

## Rust (alongside, not wired into the Swift build)
- What's proven (do not regress): core 26 tests (20 unit + 6 golden), FFI 6 smoke tests, `cargo clippy --workspace --all-targets -- -D warnings` clean. Live SMB vs debian: NTLMv2, 6755 books in ~5s, `author:austen` works. Windows native (2026-09-19): Rust 1.98.1 MSVC on win10, `catalog_ffi.dll` 12MB release, `CatalogWin.sln` 0/0 + 17/17 tests, P/Invoke probe `books=4` vs fixture — live parity (`books=6742`) still user-driven, Linux build unverified.
- Rules: `catalog-core` takes **no GUI dependencies**, ever; `catalog-ffi` fns are all **blocking** (tokio runtime inside — shells call off UI thread); no `unwrap()` on new library paths.
- Verify: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Frozen / rejected (do not revive without asking): `catalog-macos` (bridging friction — `msg_send!` for init, ivars start uninitialized, no early `return` in `method_id` fns, `*const` defeats `Send` wrappers); Slint/iced/Tauri/Dioxus/Qt (license). Sibling `~/source/reader/rust/` also frozen reference.

## Known issue (2026-09-19, resolved 2026-09-20 via reboot)
- SMB fetch failed with `NWError` posix 50 ENETDOWN at TCP connect when launched
  as a bundle, yet the same binary run bare (and Reader, same stack) connected
  fine. Packaging/entitlements/plist verified; `tccutil` reset refused by OS.
  Resolved by macOS reboot 2026-09-20 — all well currently.
  Full case file: `~/Desktop/catalog-smb-issue-synopsis.txt`.
