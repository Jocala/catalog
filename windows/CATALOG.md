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

## Build (on win10, from C:\source\catalog, via win10 MCP)

Prereqs on the VM (verify once): Rust toolchain (`rustup`, MSVC host) +
MSVC C toolchain (`cl.exe`, needed by `rusqlite/bundled` + `lzma-sys`
via `zip`) + .NET SDK 9 (present: 9.0.317).

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
dotnet build CatalogWin.sln -c Release
dotnet test tests\CatalogWin.Tests -c Release
copy C:\source\reader\catalog\target\release\catalog_ffi.dll src\CatalogWin\bin\x64\Release\net8.0-windows\
```

Single output tree: both projects pin `<Platforms>x64</Platforms>`
(native `catalog_ffi.dll` is x64-only), so every command above lands in
`bin\x64\Release\` — there is no `bin\Release` tree. Always launch from
`bin\x64\Release\net8.0-windows\JocalaCatalog.exe`; delete a stray
`bin\Release` if one predates this rule (build artifact, regenerable).

Mac has no .NET SDK and no MSVC C toolchain — `catalog-ffi` host
builds/tests (`cargo test -p catalog-ffi`, 6 FFI smoke tests) run on Mac,
but the DLL + `dotnet` verification run on `win10` only.

## Verify before reporting

```powershell
dotnet build CatalogWin.sln -c Release  # 0 errors
dotnet test tests\CatalogWin.Tests -c Release  # all pass (KoboPath + Window gates)
```

Parity gate: `books=6742` against the live library
(`NativeCatalog.GetCountAsync` vs `catalog-cli library open`);
0-or->1 Kobo matches fail-closed. Missing `catalog_ffi.dll` surfaces as
`DllNotFoundException` from `ProbeAsync` — that means the DLL step was
skipped, not a code bug.

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
