# Jocala Catalog (`com.jocala.catalog`)

Qt direction (2026-09-22): the product is the Qt Widgets app in
`windows/qt/` over the Rust engine (`crates/catalog-core` +
`crates/catalog-ffi`; `crates/catalog-cli` acceptance harness). One
tree builds all three targets: win10 (static Qt 6.11.1), debian
(system Qt 6.8.2), macOS (static Qt 6.11.1, universal app).
Archived 2026-09-22 to `/Users/jeff/source/backups/` (final tarballs;
git history keeps everything too): SwiftUI app
(`catalog-swift-final-*`), WPF (`catalog-wpf-final-*`), WinUI Phase 1
(`catalog-winui-final-*`), frozen Rust shells
(`catalog-frozen-crates-*`: egui/gtk/macos).

## Product identity
- **Name:** Jocala Catalog · **ID:** `com.jocala.catalog` (Qt Mac
  bundle uses `com.jocala.catalogqt` to avoid colliding with the
  retired Swift app at ship time).
- Settings/data: `%APPDATA%/com.jocala.Catalog/` on Windows (one
  shared-schema file, all shells); platform app-data fallback where
  `APPDATA` is absent (macOS `~/Library/...`, Linux XDG — see
  `settings.cpp::settingsPath`). Never hardcode hosts, credentials,
  or paths.
- Credentials come from Settings at runtime only; passwords are masked everywhere.

## Layout
- `windows/qt/` — THE shell: gallery, detail, settings, search,
  about/help, theme, status; demand-coalesced covers (no queues),
  disk + byte-capped memory caches, silent fresh start, single Kobo
  is default, golden default star, no click outline, aspect-fit
  covers, rich list rows (`ListDelegate`). Assets mirrored in-tree
  (`assets/`: `help.html`, `donatel.png`, `appicon.ico`,
  `AppIcon.icns` — canonical now; ex-Swift originals live in the
  swift tarball). Per-platform build scripts beside it
  (`build-catalogqt-windows.ps1`, `build-catalogqt-macos.sh`;
  Linux still uses raw cmake — script TODO).
  `packaging/catalogqt.iss.in` (single-exe Inno installer).
- `crates/catalog-core/` — GUI-free Rust business logic (the only thing UI crates may depend on); `tests/fixtures/` holds the 4-book `metadata.db` + `make-fixture.sql`
- `crates/catalog-ffi/` — C ABI over core (staticlib for the Qt shells, JSON in/out, blocking fns, tokio inside); 6 smoke tests vs fixture
- `crates/catalog-cli/` — acceptance harness: `library open|list|detail`, `import-zip` (zip-slip safe), `smb ls|get`, `cover`, `kobo`, `settings` probes (CLI prints `(set)`/`(empty)`, never values)
- Build trees live outside the repo (win10/debian `~/build-catalogqt`, Mac `source/builds/catalogqt`) + cargo `target/` (regenerable).

## Conventions
- No hardcoded hosts/ports/credentials; keep diffs small.
- Qt is cross-platform by construction: every behavior change must
  keep all three targets building (win10/debian/Mac) — verify the
  other two, or record an explicit todo.
- Cover pipeline rules (proven twice, do not regress): demand-gated
  fetches only (never queues/backlogs), 6 concurrent max, disk cache
  capped (256 MB), worker decode, coalesced repaints, silent fresh
  start, byte-capped (not count-capped) memory.
- Passwords from Settings at runtime; never baked into builds; never logged.

## Rust (the engine)
- What's proven (do not regress): core 33 unit + 6 golden tests, FFI 6 smoke tests, `cargo clippy --workspace --all-targets -- -D warnings` clean. Live SMB vs debian: NTLMv2, 6755 books in ~5s, `author:austen` works. Windows native (2026-09-19): Rust 1.98.1 MSVC on win10; live parity user-driven.
- Linux native (2026-09-22, debian): Rust 1.98.1, `libcatalog_ffi.a` 110MB, Qt 6.8.2 system libs, `JocalaCatalog` 46MB linked, `ctest` 3/3 (offscreen), app runs 20s offscreen clean, `catalog-cli library open /zstore/ebooks/calibre` → 6755 books (exact SMB parity) + cover fetch OK (160x240 JPEG). CMakeLists carries the Linux link set (Threads/DL/m + bz2 + lzma for the engine's zip backend).
- macOS native (2026-09-22, this Mac arm64): Rust 1.98.1, universal `libcatalog_ffi.a` 158MB (lipo of both arches), static Qt 6.11.1 + APPLE CMake branch (bundle `com.jocala.catalogqt`, AppIcon.icns, Cocoa→CoreFoundation link set + bz2 + lzma), universal `JocalaCatalog.app` 85MB, `ctest` 4/4. Build script: `windows/qt/build-catalogqt-macos.sh`. Unsigned local run only so far — SMB proof + sign/notarize still open.
- Rules: `catalog-core` takes **no GUI dependencies**, ever; `catalog-ffi` fns are all **blocking** (tokio runtime inside — shells call off UI thread); no `unwrap()` on new library paths.
- Verify: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Archived (do not revive without asking): egui/gtk/macos shells + SwiftUI/WPF/WinUI apps (tarballs above + git history). Sibling `~/source/reader/rust/` also frozen reference.

## Known issue (2026-09-19, resolved 2026-09-20 via reboot)
- SMB fetch failed with `NWError` posix 50 ENETDOWN at TCP connect when launched
  as a bundle, yet the same binary run bare (and Reader, same stack) connected
  fine. Packaging/entitlements/plist verified; `tccutil` reset refused by OS.
  Resolved by macOS reboot 2026-09-20 — all well currently.
  Full case file: `~/Desktop/catalog-smb-issue-synopsis.txt`.

## Production release — trigger phrase: "build for production" (TEST SITE ONLY)

Single `VERSION` parameter (currently `1.0`): installer filenames
`jocala-catalog-qt.$VERSION.exe` (+ future Mac/Linux artifacts), page
links, and size labels all derive from it. No production
(`jocala.com`) push — staging on debian only; going live is a
separate future step.

Per-platform builds (independent — different machines, no shared
state — run in any order, or parallel where noted):

- **win10** (`192.168.1.170`, primary): mirror `windows/qt` + Rust
  `crates/` + `Cargo.toml`/`Cargo.lock` (qt-subset tar); `cargo
  build --release -p catalog-ffi` with `RUSTFLAGS=-C
  target-feature=+crt-static` (shop Qt is /MT); run
  `build-catalogqt-windows.ps1` from `C:\source\catalog\qt`
  (cmake configure + build + `ctest`, all green); `package-win`
  target or direct ISCC on `packaging/catalogqt.iss.in`
  (`admin` + `{commonpf}` per 2026-09-22 decision, AppId below,
  single `JocalaCatalog.exe`); `scp` the installer back to the Mac.
- **debian** (`192.168.1.39`): mirror same; plain `cargo build
  --release -p catalog-ffi`; cmake against system Qt
  (`qt6-base-dev`) + build + `ctest` (offscreen — headless box).
  No packaging format chosen yet (tarball/AppImage/deb TBD).
- **macOS** (this Mac): `windows/qt/build-catalogqt-macos.sh`
  (universal Rust lib via lipo + universal app + `ctest`); SMB
  proof vs debian; sign (`notary-jeff`) + notarize + DMG still open
  (bundle id `com.jocala.catalogqt`, distinct from the retired Swift
  app — see decision note below).

Preconditions: sources committed (mirror from the fixed commit — a
mid-release source change must never race the builds); target VMs up
(`virsh -c qemu:///system start win10` from debian if needed).

Staging (debian, git-tracked, NO prod push), after builds: per
`~/.config/opencode/jocala-website.md` (Mac→debian via `scp`; MCP
sftp is text-only, never binaries): `scp` installer(s) to
`/zstore/source/www/jocala.com/catalog/`; update links + size
labels; commit the working tree (message carries byte sizes);
verify download URL(s) 200 with exact byte sizes.

- Inno AppId (generated 2026-09-21, keep stable across versions):
  `{E51EB7AD-FEFB-4F3E-BD2C-CA6F49DD4410}` (same AppId means the Qt
  installer replaces a WPF install — intended).

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

## Session status — 2026-09-22 (Qt direction declared + repo consolidated)
- Mac Qt build settled it: all three platforms build green from one
  tree → Qt is the catalog shell. WPF + WinUI frozen immediately;
  test site keeps WPF 1.0 staged, Qt stages beside it when promoted.
- Repo consolidated to Qt + engine: archived to
  `/Users/jeff/source/backups/` (`catalog-swift-final-*`,
  `catalog-wpf-final-*`, `catalog-winui-final-*`,
  `catalog-frozen-crates-*`; art mirrors md5-verified, tarballs
  listed + test-extracted before delete), then `git rm` of
  `Sources/`, `Package.swift`, `Info.plist`, `CatalogSwift.entitlements`,
  `Resources/`, `README.md` (rewritten for Qt), `windows/src|tests|
  CatalogWin.sln|installer|CATALOG.md`, `windows/winui`,
  `crates/{egui,gtk,macos}`. Workspace members now core+cli+ffi
  only (`cargo test --workspace` green: 33 unit + 6 golden + 6 smoke).
- `AGENTS.md` rewritten for Qt (this file); `README.md` rewritten;
  `.gitignore` trimmed. `build/` (338M) + `target/` (9.4G) left on
  disk (untracked, regenerable — separate cleanup call).
