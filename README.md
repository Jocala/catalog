# Jocala Catalog

Qt Widgets library manager over the Rust catalog engine — one tree,
three targets (Windows, Linux, macOS). Ebook library browser for a
Calibre library (local folder or SMB share): gallery + detail,
browse/search, settings, Kobo sync/open over WiFi.

## Layout

- `qt/` — the Qt shell (gallery, detail, settings, search,
  about/help, theme, status). Per-platform build scripts beside it:
  `build-catalogqt-windows.ps1`, `build-catalogqt-macos.sh`
  (Linux uses raw cmake for now). `packaging/catalogqt.iss.in` is
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
  `Cargo.toml`/`Cargo.lock`; `cargo build --release -p catalog-ffi`
  with `RUSTFLAGS=-C target-feature=+crt-static`; run
  `build-catalogqt-windows.ps1` from `C:\source\catalog\qt`
  (cmake + build + `ctest`).
- **debian** (system Qt via `qt6-base-dev`): mirror same; plain
  `cargo build --release -p catalog-ffi`; cmake + build + `ctest`
  (`QT_QPA_PLATFORM=offscreen` — headless box).
- **macOS** (static Qt 6.11.1): `qt/build-catalogqt-macos.sh`
  (universal Rust lib + universal app + `ctest`).

Verify: `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`.

Full procedure (release, staging, AppId): `AGENTS.md`.
Prior shells (SwiftUI/WPF/WinUI) + frozen Rust UIs are archived in
`/Users/jeff/source/backups/` (final tarballs) and git history.
