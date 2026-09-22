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

## Windows — `windows/` (`com.jocala.catalog`, `JocalaCatalog`)
- WPF + Rust core via P/Invoke (`windows/CatalogWin.sln` → `src/CatalogWin` + `tests/CatalogWin.Tests`, `CATALOG.md`). Mirrors `CatalogWin` hard rules: software-only rendering (`App.xaml.cs` `SoftwareOnly`), raw SMB, `ItemWindow` windowing (stock `WrapPanel`/`UniformGrid` + spacer window, never custom `VirtualizingWrapPanel`), SSH.NET `ShellStream` heredoc for Kobo, single `bin\x64\Release` tree (`<Platforms>x64</Platforms>` remap). See `windows/CATALOG.md` (authoritative for Windows ops) + `windows/src/CatalogWin/` for the implementation.

## Conventions
- No hardcoded hosts/ports/credentials; keep diffs small.
- Mac/Windows parity: every Mac-side feature or behavior change probably needs
  a Windows mirror in `windows/` (SSH.NET transport, WPF UI) — scope it in the
  same change, or record it as an explicit todo. (Kobo per-IP SSH passwords:
  shipped on Mac + Windows 2026-09-21.)
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

## Production release — trigger phrase: "build for production" (TEST SITE ONLY)

Single `VERSION` parameter (currently `1.0`): installer filenames
`jocala-catalog.$VERSION.dmg` / `jocala-catalog.$VERSION.exe`, page links,
and size labels all derive from it. No production (`jocala.com`) push —
staging on debian only; going live is a separate future step.

Standard form is **parallel A‖B then C** (proven 2026-09-21, full run
2m20s; the long pole is Apple's notarization queue, which no parallelism
can shrink). A and B are independent — different machines, no shared
state — so they run in the same waves; C needs both installers in hand.

Preconditions: sources committed (mirror from the fixed commit — a
mid-release source change must never race the builds); `pgrep -x
CatalogSwift` empty (never rewrite a running bundle); win10 up (`virsh
-c qemu:///system start win10` from debian if needed; primary
`192.168.1.170`).

Prep (fast, sequential): mirror win10 per `windows/CATALOG.md` (windows
subset tar + Rust `crates/` + `Cargo.toml`/`Cargo.lock`).

Wave 1 — Mac `rm -rf build` + `swift build -c release --scratch-path
.../build --package-path ...` ‖ win10 extract tar + swap `crates/`.

Wave 2 — Mac assemble `build/Jocala Catalog.app` (binary +
`Info.plist` + `Resources/AppIcon.icns` + `Resources/help.html` +
`Resources/donatel.png` + **`build/release/JocalaCatalogSwift_CatalogSwiftApp.bundle`**
(REQUIRED — without it About/Help trap on `Bundle.module`) +
`PkgInfo`), Dev-ID sign (`CatalogSwift.entitlements`, `--options runtime
--timestamp`; canonical flags: `~/.config/opencode/AGENTS.md` § Apple
Developer ID signing) ‖ win10 `cargo clean -p catalog-ffi && cargo build
--release -p catalog-ffi`.

Wave 3 — Mac notarize (`notary-jeff`), staple, `spctl` accept ‖ win10
(full-path per-user SDK 10 — machine SDK 9 cannot target net10):
copy `catalog_ffi.dll` into `src/CatalogWin/` BEFORE build (recipe-owned,
gitignored — csproj flows it into the single-file bundle), then
`dotnet build CatalogWin.sln -c Release` (0/0) + `dotnet test` (all
pass), `dotnet publish … -c Release -r win-x64 --self-contained true`
(single-file flags in csproj; confirm `publish\` holds only
`JocalaCatalog.exe` +pdb, no `Assets\`), launch-smoke the single exe,
confirm no `JocalaCatalog.exe.WebView2/` cache and no stray `bin\Release` tree,
then compile `windows/installer/catalog.iss` with
`C:\bin\inno\ISCC.exe /DVERSION=$VERSION` (single-exe installer, AppId below,
publisher `Jocala Software`, `OutputBaseFilename=jocala-catalog.$VERSION`).
`scp` the exe back to the Mac (`/tmp/jocala-catalog.$VERSION.exe`).

Phase C — test staging (debian, git-tracked, NO prod push), after A‖B:
DMG staging dir (baseline, no background art): `Jocala Catalog.app` +
`Applications` symlink → `hdiutil create -volname "Jocala Catalog"
-srcfolder <stage> -ov -format UDZO jocala-catalog.$VERSION.dmg`; verify
the sig inside the mounted image. Per
`~/.config/opencode/jocala-website.md` (Mac→debian via `scp`; MCP sftp
is text-only, never binaries): `scp` both installers to
`/zstore/source/www/jocala.com/catalog/`; `catalog/index.html` links →
`jocala-catalog.$VERSION.dmg/.exe`, size labels → real sizes (skip if
the rounded MB labels still hold); root `index.html` Catalog `jl-card`
directly after the Adblink card (one-time, already placed). Commit the
working tree on debian (message carries both byte sizes); verify
`http://192.168.1.39/www/jocala.com/catalog/` + both downloads 200 with
exact byte sizes.

- Inno AppId (generated 2026-09-21, keep stable across versions):
  `{E51EB7AD-FEFB-4F3E-BD2C-CA6F49DD4410}`.

## Session status — 2026-09-21 (COMMITTED below as stable revert point)
- Shipped, all in `build/Jocala Catalog.app` (Dev-ID signed + notarized Accepted +
  stapled, `spctl` accepted): Kobo per-IP SSH passwords (in-place edit, star
  first, Show/Hide per row, `KoboDevice` model, passwords in `KeychainHelper`
  store under `kobo-passwords`, legacy `kobo` migrated on Settings open);
  askpass password auth in both SSH primitives (`koboSSHInvocation`, no sshpass,
  `BatchMode` unchanged when empty); wrong-password dialog note
  ("SSH password rejected for <ip> — check Settings → Kobo"); search-grid
  hover cursor (`SearchResultCell` now pushes `pointingHand` like `BookCell`).
- Live-verified 2026-09-21 vs Color `.74`: password `1234` authenticates (`ok`,
  exit 0).
- Themes + About/Help viewers (Mac + Windows): `AboutView`/`HelpView`
  (WKWebView, shared `help.html` + `donatel.png` in `Resources/`), Windows
  `AboutWindow`/`HelpWindow` (WebView2, mirrored `Assets/`), `UI/` light/dark
  themes + `Theme.cs`, `AboutTests`/`ThemeTests`.
- Single Mac Help menu (`CommandGroup(replacing: .help)`, `Cmd+Shift+?` —
  drops the system "<executable> Help" phantom); in-HTML `✕ Close`
  (`catalog:close` intercepted by both viewers).
- Save reloads iff the source changed: Mac snapshots persisted source fields
  at sheet open and posts reload only on host/share/path/user/domain/password/
  source/local-dir difference; Windows mirrors via `SettingsChanged` flag +
  gated `LoadAsync()`. Reindex buttons deleted both platforms; toolbar Reload
  intentionally kept (sole re-read for Calibre-side additions).
- Mac differential verified live via `app_log.txt` (unchanged Save silent,
  source-change Save reloads); Windows `dotnet build`/`test` user-verified
  on win10.
- PRODUCTION RELEASE 1.0 DONE 2026-09-21 (TEST SITE ONLY, no prod push).
  - Phase A DONE: from-scratch release build, signed + notarized Accepted +
    stapled, smoke-tested (6755 books). DMG at `/tmp/jocala-catalog.1.0.dmg`
    (4014112 bytes, md5 `62c4d8c9cbce214bcc995f182e779df9`), sig verified
    inside mounted image. Note: fresh SPM scratch layout is now
    `build/release/` + `build/out/` (old `build/arm64-apple-macosx/` path
    in README is stale).
  - Phase B DONE: resumed `cargo build --release -p catalog-ffi` on win10 →
    `catalog_ffi.dll` 16630784 bytes; `dotnet build` 0 warn/0 err, `dotnet
    test` 30/30 pass; ISCC (`C:\bin\inno\ISCC.exe /DVERSION=1.0`, note:
    actual path is `C:\bin\inno\`, not `C:\bin\bin\`) →
    `C:\source\catalog\install\jocala-catalog.1.0.exe`, scp'd to Mac
    (`/tmp/jocala-catalog.1.0.exe`, 16650227 bytes, md5
    `ffa4957df2fc9dc56406484c75544eb0`). `catalog.iss` now carries
    `Excludes: "*.WebView2"` — first compile shipped the VM's EBWebView
    profile cache (History/Cookies!), caught on review, cache deleted +
    recompiled clean. Always delete `JocalaCatalog.exe.WebView2/` from the
    Release tree before ISCC (it regenerates on every local app run).
  - Phase C DONE: both installers scp'd to debian staging
    (`/zstore/source/www/jocala.com/catalog/jocala-catalog.1.0.*`),
    `catalog/index.html` links + sizes (4 MB dmg / 16 MB exe), root
    Catalog card after Adblink, site tree committed (`d214530`), test URLs
    verified 200 with exact byte sizes. Old `catalog.1.0.*` stubs left in
    place, unlinked. NO prod push.
  - Committed in repo: this AGENTS.md section, `windows/installer/`
    (`catalog.iss`), `.gitignore` (`windows/install/`). Nothing running.
- 2026-09-21 session (Windows emoji clipping, code done + built, visual
  sign-off still user-driven): toolbar 🔍 (`MainWindow.xaml`, 32→36
  wide, Padding 0, 14pt, centered), Kobo 🗑 + ★/☆
  (`SettingsWindow.xaml.cs` `BuildKoboRow`, 30/26→34 wide, Padding 0,
  14pt, centered; trash kept its left margin). Mirrored to win10
  (`C:\source\catalog`), `dotnet build` 0 warn/0 err, `dotnet test`
  30/30. Relaunch `JocalaCatalog.exe` to confirm.
- Env note 2026-09-21: win11 `~/.ssh/id_ed25519[.pub]` is now the mac
  keypair (orig backed up as `.win11vm-orig`; `~/.ssh/config` gained the
  `kobo color` root entry) — win11→kobo `.74` passwordless works, same
  as win10/mac.
- 2026-09-21 session (bottom status bar, both platforms; About/Help crash
  fix): Mac header is controls-only, new always-visible bottom bar
  (`bottomStatusMessage`/`bottomStatusColor`: Kobo > search counts >
  library counts; window 892x818); Windows `StatusText` moved to
  `DockPanel.Dock="Bottom"`, toolbar `ResultsText` deleted; `help.html`
  wording both copies. Rebuilt Mac from scratch, signed + notarized
  Accepted + stapled, `spctl` accepted — but first rebuild crashed on
  About open (`Bundle.module` trap, missing SPM resource bundle):
  assembly must copy `build/release/JocalaCatalogSwift_CatalogSwiftApp.bundle`
  into `Contents/Resources/` (recipe fixed in `README.md`), then re-sign +
  re-notarize. About + Help user-verified. Windows mirrored to win10,
  `dotnet build` 0/0, `dotnet test` 30/30. `.build/` removed — `build/`
  is the only build location (`--scratch-path build/check` for checks).
- 2026-09-21/22 session (Qt port + Rust Kobo flows, committed `265c20f`;
  verdict: borderline acceptable, quits for now): `windows/qt/` full
  Widgets app (gallery, detail, settings, search, about/help, theme,
  status; static Qt 6.11.1 + static `catalog_ffi`, single
  `JocalaCatalog.exe` ~40 MB, installer 12.8 MB via `package-win`).
  Rust: russh 0.52 transport (`kobo/ssh.rs`; 0.63 refused by resolver vs
  smb RC crypto), open/sync flows (`kobo/open.rs`, `kobo/sync.rs`),
  FFI db cache (60s TTL, `"fresh":"1"` bypass), SMB session pool,
  CLI `kobo`/`cover` probes. Hardware-proven vs Color `.74` (key,
  password, 255-reject, timeout). Perf work: single-watcher cover bug,
  6-gate throttle, FFI cache, pixmap+batched paints, session pool —
  6700 SMB borderline-OK once covers cache; local 300 snappy; Read on
  Kobo works through Qt. Deferred: gate tuning/prefetch, satellite
  trim, WPF freeze decision, Qt production release (`.iss` points at
  publish tree). Test site still serves 1.0 (WPF/.NET + Mac).
  win10 trees current (`C:\source\catalog`, `C:\source\reader\catalog`).
  Nothing running.
