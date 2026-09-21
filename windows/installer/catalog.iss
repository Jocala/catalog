; Jocala Catalog — Inno Setup installer (full installer: wizard,
; Start Menu entries, uninstaller). Compiled on win10 with:
;   C:\bin\bin\ISCC.exe /DVERSION=1.0 catalog.iss
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
; Entire Release output tree: JocalaCatalog.exe + catalog_ffi.dll +
; .NET deps + Assets\help.html (loose file the viewer opens).
Source: "..\src\CatalogWin\bin\x64\Release\net9.0-windows\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\Jocala Catalog"; Filename: "{app}\JocalaCatalog.exe"
Name: "{group}\Uninstall Jocala Catalog"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Jocala Catalog"; Filename: "{app}\JocalaCatalog.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Run]
Filename: "{app}\JocalaCatalog.exe"; Description: "Launch Jocala Catalog"; Flags: nowait postinstall skipifsilent
