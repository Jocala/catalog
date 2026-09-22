# Jocala Catalog — Windows port (`com.jocala.catalog`)

WPF + Rust core via P/Invoke. Fork of `ReaderWin` (same XAML, same hard
rules in `AGENTS.md`: software-only rendering, raw SMB, `ItemWindow`
windowing, SSH.NET ShellStream heredoc for Kobo) with the backend
swapped: `Core/NativeCatalog.cs` calls `catalog_ffi.dll`
(`source/catalog/crates/catalog-ffi`, JSON in/out) instead of
`Services/CalibreDb+SmbReader+Covers`. The C# Services stay on disk as
the documented fallback transport.

Identity (isolated from the Reader app): assembly `JocalaCatalog`,
namespace `CatalogWin.*`, data `%APPDATA%/com.jocala.Catalog/`
(settings.json, covers/, thumbnails/, kobo_index.txt, app_log.txt).

## Layout on win10 (top-level build env, 2026-09-19)
`CatalogWin` lives at `C:\source\catalog\` (sln + `src/CatalogWin` +
`tests/CatalogWin.Tests` + this file + `probe-ffi.ps1`), alongside the
other top-level projects — NOT under `C:\source\reader\windows\` (that
keeps `ReaderWin` only). Authoring stays in this repo
(`source/catalog/windows`: `src/CatalogWin`, `tests/CatalogWin.Tests`,
`CatalogWin.sln`, `CATALOG.md`); mirror the CatalogWin subset on change:
```sh
tar -cf /tmp/catalogwin.tar -C /Users/jeff/source/catalog/windows \
  CatalogWin.sln CATALOG.md src/CatalogWin tests/CatalogWin.Tests
scp /tmp/catalogwin.tar win10:C:/source/catalogwin.tar
# on win10: tar -xf C:\source\catalogwin.tar -C C:\source\catalog
```
The Rust tree stays at `C:\source\reader\catalog\` (cargo workspace root).

## Qt port (`qt/`, static Qt 6.11.1 + static catalog_ffi)

Full Widgets app (gallery, detail, settings, search, about/help, theme,
bottom status bar) consuming the same C ABI as the WPF bridge — no .NET.
Builds to a single `JocalaCatalog.exe` (~40 MB); installer 12.8 MB.

```powershell
# win10, from C:\source\catalog\qt (mirrored from repo windows/qt):
.\build-catalogqt-windows.ps1   # configure + build + ctest
# Rust staticlib MUST use: set RUSTFLAGS=-C target-feature=+crt-static
# (shop Qt is /MT; dynamic-CRT Rust objects fail LNK4098/1120)
```

Settings schema is shared with WPF (`%APPDATA%/com.jocala.Catalog/settings.json`,
identical keys) — the Qt build migrates seamlessly. Perf verdict
2026-09-22: 6700 SMB borderline-OK once covers cache (FFI db cache +
6-gate throttle + session pool all landed for this); local 300 snappy.

## Build (on win10, from C:\source\catalog, via win10 MCP)

Prereqs on the VM (verify once): Rust toolchain (`rustup`, MSVC host) +
MSVC C toolchain (`cl.exe`, needed by `rusqlite/bundled` + `lzma-sys`
via `zip`) + .NET SDK 10, per-user at `%LOCALAPPDATA%\Microsoft\dotnet`
(installed 2026-09-22 via `dotnet-install.ps1 -Channel 10.0`, no admin;
machine-wide SDK 9.0.317 can NOT target net10 — always invoke the 10 SDK
by full path, e.g. `%LOCALAPPDATA%\Microsoft\dotnet\dotnet.exe build`).

```powershell
# 1. Mirror the Rust tree to the VM (from macOS):
#   scp -r /Users/jeff/source/catalog win10:C:/source/reader/catalog
# 2. Native DLL (Release):
cd C:\source\reader\catalog
cargo build --release -p catalog-ffi
#   -> target\release\catalog_ffi.dll

# 3. Mirror this tree + copy the DLL next to the exe output:
#   (see Layout above for the selective tar; full-tree scp also works)
cd C:\source\catalog
%LOCALAPPDATA%\Microsoft\dotnet\dotnet.exe build CatalogWin.sln -c Release
%LOCALAPPDATA%\Microsoft\dotnet\dotnet.exe test tests\CatalogWin.Tests -c Release
copy C:\source\reader\catalog\target\release\catalog_ffi.dll src\CatalogWin\bin\x64\Release\net10.0-windows\
```

Self-contained publish (no .NET runtime needed on the target machine;
win-x64 only — the FFI DLL is x64-native; no SingleFile, never trim WPF):
```powershell
%LOCALAPPDATA%\Microsoft\dotnet\dotnet.exe publish src\CatalogWin\CatalogWin.csproj -c Release -r win-x64 --self-contained true -o src\CatalogWin\bin\x64\Release\net10.0-windows\publish
copy C:\source\reader\catalog\target\release\catalog_ffi.dll src\CatalogWin\bin\x64\Release\net10.0-windows\publish\
# Launch to test: ...\net10.0-windows\publish\JocalaCatalog.exe
```

Single output tree: both projects pin `<Platforms>x64</Platforms>`
(native `catalog_ffi.dll` is x64-only), so every command above lands in
`bin\x64\Release\` — there is no `bin\Release` tree. Always launch from
`bin\x64\Release\net10.0-windows\JocalaCatalog.exe`; delete a stray
`bin\Release` if one predates this rule (build artifact, regenerable).

Mac has no .NET SDK and no MSVC C toolchain — `catalog-ffi` host
builds/tests (`cargo test -p catalog-ffi`, 6 FFI smoke tests) run on Mac,
but the DLL + `dotnet` verification run on `win10` only.

## Verify before reporting

```powershell
%LOCALAPPDATA%\Microsoft\dotnet\dotnet.exe build CatalogWin.sln -c Release  # 0 errors
%LOCALAPPDATA%\Microsoft\dotnet\dotnet.exe test tests\CatalogWin.Tests -c Release  # all pass (KoboPath + Window gates)
```

Parity gate: `books=6742` against the live library
(`NativeCatalog.GetCountAsync` vs `catalog-cli library open`);
0-or->1 Kobo matches fail-closed. Missing `catalog_ffi.dll` surfaces as
`DllNotFoundException` from `ProbeAsync` — that means the DLL step was
skipped, not a code bug.

## Verified 2026-09-21 (win10, Kobo password parity)
- `Settings`: `KoboPasswords` ip→password dict (`KoboPasswordFor`/`SetKoboPassword`,
  empty = key-only) + `RenameKobo` (in-place IP rename carries password +
  default); Settings → Kobo is per-row cards mirroring macOS `KoboDevice`
  rows: star (gold = default) + editable mono IP + Password + Show/Hide
  (in-memory reveal) + per-row ping Test with Pass/Fail pill + red 🗑;
  Add row + caption; Remove drops the password.
- `KoboLauncher.SshSync`: configured password → SSH.NET
  `PasswordAuthenticationMethod`, else legacy `PrivateKey` (unchanged);
  `SshAuthenticationException` → `SSH password rejected for <ip> — check
  Settings → Kobo` (code 255, propagated through the probe).
- `dotnet build CatalogWin.sln -c Release`: 0 warnings, 0 errors.
- `dotnet test`: 25/25 pass (incl. 8 `KoboPasswordTests`: store + rename-carry).
- Live password-auth vs Color `.74` still user-driven: enter the password in
  Settings → Kobo on the VM (passwords are never in source).

## Verified 2026-09-19 (win10, native)
- Rust 1.98.1 stable MSVC installed per-user (`%USERPROFILE%\.cargo`);
  MSVC 14.51 + Win10 SDK 10.0.26100 present (VS Community 2022 17.14).
- `cargo build --release -p catalog-ffi` → `catalog_ffi.dll` 12MB.
- `dotnet build CatalogWin.sln -c Release`: 0 warnings, 0 errors.
- `dotnet test`: 14/14 pass (KoboPath + Window gates).
- PowerShell P/Invoke probe vs fixture: `version {"ok":{"version":"0.1.0"}}`,
  `open {"ok":{"books":4}}` → PROBE-OK.
- Live-library parity (`books=6742`) still user-driven: enter SMB settings
  in the app (or pass creds to a probe) — passwords are never in source.
