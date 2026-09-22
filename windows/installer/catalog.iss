; Jocala Catalog — Inno Setup installer (full installer: wizard,
; Start Menu entries, uninstaller). Compiled on win10 with:
;   C:\bin\inno\ISCC.exe /DVERSION=1.0 catalog.iss
; VERSION is passed on the command line so releases bump in one place.
; AppId generated 2026-09-21 — keep stable across versions (Windows uses
; it to identify the product for upgrades/uninstall).
#ifndef VERSION
  #define VERSION "1.0"
#endif

[Setup]
AppId={{E51EB7AD-FEFB-4F3E-BD2C-CA6F49DD4410}
AppName=Jocala Catalog
AppVersion={#VERSION}
AppPublisher=Jocala Software
AppPublisherURL=https://www.jocala.com/catalog/
DefaultDirName={autopf}\Jocala Catalog
DefaultGroupName=Jocala Catalog
OutputDir=..\install
OutputBaseFilename=jocala-catalog.{#VERSION}
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
UninstallDisplayName=Jocala Catalog
WizardStyle=modern

[Files]
; Single-file self-contained publish: exactly one JocalaCatalog.exe
; (managed + .NET runtime + catalog_ffi.dll + WebView2 loader, compressed).
; help.html is enclosed in the exe (WPF Resource) — no loose Assets dir.
; No Excludes needed (no PDBs/diagnostics ship in the bundle).
; Runtime note: WebView2 still creates a <exe>.WebView2\EBWebView user-data
; dir next to the exe on first run — same as 1.0, never in the installer.
Source: "..\src\CatalogWin\bin\x64\Release\net10.0-windows\publish\JocalaCatalog.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Jocala Catalog"; Filename: "{app}\JocalaCatalog.exe"
Name: "{group}\Uninstall Jocala Catalog"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Jocala Catalog"; Filename: "{app}\JocalaCatalog.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Run]
Filename: "{app}\JocalaCatalog.exe"; Description: "Launch Jocala Catalog"; Flags: nowait postinstall skipifsilent
