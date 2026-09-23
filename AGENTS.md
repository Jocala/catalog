# Jocala Catalog (`com.jocala.catalog`)

Qt direction (2026-09-22): the product is the Qt Widgets app in
`qt/` over the Rust engine (`crates/catalog-core` +
`crates/catalog-ffi`; `crates/catalog-cli` acceptance harness). One
tree builds all three targets: win10 (static Qt 6.11.1), debian
(system Qt 6.8.2), macOS (static Qt 6.11.1, universal app).
Prior shells (SwiftUI, WPF, WinUI Phase 1) + frozen Rust shells
(egui/gtk/macos) are archived in `/Users/jeff/source/backups/`
(final tarballs) and git history — not in this tree, not referenced
below except where a live procedure depends on them.

## Product identity
- **Name:** Jocala Catalog · **ID:** `com.jocala.catalog` (all
  targets; the retired Swift app's ID is unshipped — no conflict).
- Settings/data: `%APPDATA%/com.jocala.Catalog/` on Windows (one
  shared-schema file, all shells); platform app-data fallback where
  `APPDATA` is absent (macOS `~/Library/...`, Linux XDG — see
  `settings.cpp::settingsPath`). Never hardcode hosts, credentials,
  or paths.
- Credentials come from Settings at runtime only; passwords are masked everywhere.
  (No passwords in this repo, ever — including test/hardware ones.)

## Layout
- `qt/` — THE shell: gallery, detail, settings, search,
  about/help, theme, status; demand-coalesced covers (no queues),
  disk + byte-capped memory caches, silent fresh start, single Kobo
  is default, golden default star, no click outline, aspect-fit
  covers, rich list rows (`ListDelegate`). Assets mirrored in-tree
  (`assets/`: `help.html`, `donatel.png`, `appicon.ico`,
  `AppIcon.icns` — canonical). Per-platform build scripts beside it
  (`build-catalog-{windows.ps1,macos.sh,linux.sh}` for build +
  `ctest`, `package-catalog-{windows.ps1,macos.sh,linux.sh}` for
  packaging; all run from anywhere, all take `--clean`/`-Clean` for a
  fresh build).
  `packaging/catalog.iss.in` (single-exe Inno installer).
- `crates/catalog-core/` — GUI-free Rust business logic (the only thing UI crates may depend on); `tests/fixtures/` holds the 4-book `metadata.db` + `make-fixture.sql`
- `crates/catalog-ffi/` — C ABI over core (staticlib for the Qt shells, JSON in/out, blocking fns, tokio inside); 6 smoke tests vs fixture
- `crates/catalog-cli/` — acceptance harness: `library open|list|detail`, `import-zip` (zip-slip safe), `smb ls|get`, `cover`, `kobo`, `settings` probes (CLI prints `(set)`/`(empty)`, never values)
- Build trees live outside the repo (win10/debian `~/build-catalog`, Mac `source/builds/catalog`) + cargo `target/` (regenerable).

## Conventions
- No hardcoded hosts/ports/credentials; keep diffs small.
- macOS is the primary dev platform (Rust engine + Qt GUI);
  Windows and Linux build/test on an ad hoc basis. Flag anything
  that may not port or needs platform-specific attention instead of
  verifying all three targets on every change.
- Cover pipeline rules (proven twice, do not regress): demand-gated
  fetches only (never queues/backlogs), 6 concurrent max, disk cache
  capped (256 MB), worker decode, coalesced repaints, silent fresh
  start, byte-capped (not count-capped) memory.
- Passwords from Settings at runtime; never baked into builds; never logged.

## Rust (the engine)
- What's proven (do not regress): core 33 unit + 6 golden tests, FFI 6 smoke tests, `cargo clippy --workspace --all-targets -- -D warnings` clean. Live SMB vs debian: NTLMv2, 6755 books in ~5s, `author:austen` works.
- FFI metadata cache: 60s TTL process-wide, `"fresh":"1"` bypasses (Reload semantics). Stale-read window is by design.
- russh is pinned at 0.52: 0.63 was refused by the resolver against the smb RC crypto. Do not bump without re-resolving.
- Windows native: Rust 1.98.1 MSVC on win10; Qt build + `ctest` 4/4
  green 2026-09-22 via `build-catalog-windows.ps1`; live parity user-driven.
- Linux native (debian): Rust 1.98.1, `libcatalog_ffi.a` 110MB, Qt 6.8.2 system libs, `JocalaCatalog` 46MB linked, `ctest` 4/4 (offscreen). CMakeLists carries the Linux link set (Threads/DL/m + bz2 + lzma for the engine's zip backend).
- macOS native (this Mac arm64): Rust 1.98.1, universal `libcatalog_ffi.a` 158MB (lipo of both arches), static Qt 6.11.1, universal `JocalaCatalog.app` 85MB, `ctest` 4/4. Build script: `qt/build-catalog-macos.sh`. Unsigned local run only so far — SMB proof + sign/notarize still open.
- Rules: `catalog-core` takes **no GUI dependencies**, ever; `catalog-ffi` fns are all **blocking** (tokio runtime inside — shells call off UI thread); no `unwrap()` on new library paths.
- Verify: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.

## Windows MCP mechanics (win10 `192.168.1.170`)
- `run-command` times out on multi-minute builds (proven 2026-09-22:
  foreground cargo builds died on both win10 and debian) — run long
  builds as background sessions (`open-session type=background` with
  fully path-qualified, single-line commands, no `cd`) and poll with
  `read-session-output`. Sessions vanish when the command exits, so
  confirm via artifacts (fresh binary, ctest logs).
- Stale CMake cache: if configure errors `source ... does not match
  the source ... used to generate cache`, rerun with `-Clean`
  (build dir predates a source-tree move).
- Interactive sessions unsupported (POSIX-shell handshake);
  `open-session type=background` needs fully path-qualified,
  single-line commands (no `cd`).
- The shell is `cmd` (`dir/copy/rmdir/findstr`, `&&` chains) unless
  you wrap `powershell -NoProfile -Command "..."` (then `;`
  separators — PowerShell 5.1 rejects `&&`).
- ISCC lives at `C:\bin\inno\ISCC.exe` (not `C:\bin\bin\`).
- Never relink over a running exe (`LNK1104`) — quit the app first.

## Production release — trigger phrase: "build for production" (TEST SITE ONLY)

Single `VERSION` parameter (currently `1.0`): installer filenames
`jocala-catalog.$VERSION.exe` (+ future Mac/Linux artifacts), page
links, and size labels all derive from it. No production
(`jocala.com`) push — staging on debian only; going live is a
separate future step.

Per-platform builds (independent — different machines, no shared
state — run in any order, or parallel where noted):

- **win10** (primary): mirror `qt` + Rust `crates/` +
  `Cargo.toml`/`Cargo.lock` (qt-subset tar); run
  `build-catalog-windows.ps1` from `C:\source\catalog\qt` (cargo
  with `RUSTFLAGS=-C target-feature=+crt-static` to match shop Qt's
  /MT, then cmake configure + build + `ctest`, all green);
  `package-catalog-windows.ps1` after build (wraps the `package-win`
  target / ISCC on `packaging/catalog.iss.in`: `admin` +
  `{commonpf}`, AppId below, single `JocalaCatalog.exe`); `scp` the
  installer back to the Mac.
- **debian** (`192.168.1.39`): mirror same; run
  `qt/build-catalog-linux.sh` (plain cargo, cmake against system Qt
  `qt6-base-dev`, build + `ctest` offscreen — headless box);
  `qt/package-catalog-linux.sh` after build (CPack TGZ).
- **macOS** (this Mac): `qt/build-catalog-macos.sh` (universal Rust
  lib via lipo + universal app + `ctest`);
  `qt/package-catalog-macos.sh` after build (CPack DragNDrop,
  unsigned — sign (`notary-jeff`) + notarize + stapled DMG still open).

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

## Test hardware (Kobo)
- Kobo Color at `.74` (root, key auth from Mac/win10/linux keypairs;
  password auth also proven). Engine flows proven against it: key,
  password, 255-reject, timeout, user-cancel mid-sync.
- win11 carries the mac keypair for passwordless Kobo SSH (same as
  win10/mac). debian keypair placed 2026-09-22 for the Linux port.
- Passwords live in device Settings only — never in this repo.

## Currently staged (test site only, no prod push)
- WPF 1.0 + Mac DMG 1.0 (Swift), committed `d214530`. Qt stages
  beside them when promoted (`jocala-catalog-1.0.exe`).
- Stable revert point 2026-09-22: Qt + engine consolidated, all three
  platforms green, repo clean — the milestone commit below.
- Full session history lives in git log; retired trees in
  `/Users/jeff/source/backups/` final tarballs. This file carries
  no archaeology by policy — record decisions, not sessions.
