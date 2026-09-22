# Jocala Catalog

Qt Widgets library manager over the Rust catalog engine — one tree,
three targets (Windows, Linux, macOS). Ebook library browser for a
Calibre library (local folder or SMB share): gallery + detail,
browse/search, settings, Kobo sync/open over WiFi.

## Layout

- `qt/` — the Qt shell (gallery, detail, settings, search,
  about/help, theme, status). Per-platform build + package scripts
  beside it: `build-catalogqt-{windows.ps1,macos.sh,linux.sh}` (build +
  `ctest`), `package-catalogqt-{windows.ps1,macos.sh,linux.sh}`
  (Inno EXE / DragNDrop DMG / TGZ). `packaging/catalogqt.iss.in` is
  the Windows installer template.
- `crates/catalog-core/` — GUI-free Rust business logic.
- `crates/catalog-ffi/` — C ABI over core (staticlib the Qt shells
  link; JSON in/out, blocking calls, tokio inside).
- `crates/catalog-cli/` — acceptance harness (`library open|list|detail`,
  `smb`, `cover`, `kobo`, `settings` probes).
- Build trees live outside the repo (`~/build-catalogqt` on
  win10/debian, `source/builds/catalogqt` on Mac) + cargo `target/`.

## Build

Precondition: sources committed (mirrors go from a fixed commit).

- **win10** (static Qt 6.11.1): mirror `qt` + `crates/` +
  `Cargo.toml`/`Cargo.lock`; run `build-catalogqt-windows.ps1` from
  `C:\source\catalog\qt` (cargo with `+crt-static` + cmake + build +
  `ctest`). Package after build: `package-catalogqt-windows.ps1`.
- **debian** (system Qt via `qt6-base-dev`): mirror same; run
  `qt/build-catalogqt-linux.sh` (plain cargo + cmake + build +
  `ctest` offscreen — headless box). Package after build:
  `qt/package-catalogqt-linux.sh` (TGZ).
- **macOS** (static Qt 6.11.1): `qt/build-catalogqt-macos.sh`
  (universal Rust lib + universal app + `ctest`). Package after
  build: `qt/package-catalogqt-macos.sh` (unsigned DMG for now).
- All build scripts take `--clean` (`-Clean` on Windows) to wipe the
  CMake build dir for a fresh build; default is incremental.

Verify: `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`.

Full procedure (release, staging, AppId): `AGENTS.md`.
Prior shells (SwiftUI/WPF/WinUI) + frozen Rust UIs are archived in
`/Users/jeff/source/backups/` (final tarballs) and git history.
