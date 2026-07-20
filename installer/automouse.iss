; Inno Setup script for AutoMouse.
; Build with: installer\build.ps1  (or ISCC.exe installer\automouse.iss)

#define AppName "AutoMouse"
#define AppExe "automouse.exe"

; Version comes from Cargo.toml via build.ps1 (-DAppVersion=...). The fallback
; only applies when running ISCC by hand.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Identifies the app to Windows. Generated with Guid.NewGuid() (RFC 4122 v4).
; Must stay stable across releases, or upgrades install side-by-side instead
; of replacing. The leading "{{" is Inno's escape for a literal "{".
AppId={{6E8994D8-6570-45D3-8647-5CBC8EDA951A}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Adrien Boitelle
AppCopyright=Copyright (C) 2026 Adrien Boitelle, GPLv3
LicenseFile=..\LICENSE
DefaultDirName={autopf}\{#AppName}
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
OutputDir=..\dist
OutputBaseFilename=AutoMouse-Setup
SetupIconFile=..\icons\app.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Default to a per-user install (no UAC prompt); the dialog lets the user pick
; all-users instead. "lowest" also keeps {userappdata} pointing at the real user.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; No program-group page: the Start Menu entry is an opt-in task instead.
DisableProgramGroupPage=yes
DisableDirPage=no
; Let the restart manager close a running AutoMouse rather than requiring a reboot.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startmenuicon"; Description: "Create a &Start Menu entry"; GroupDescription: "Shortcuts:"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: startmenuicon
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Settings live outside {app}; only removed if the user opts in (see below).
Type: filesandordirs; Name: "{userappdata}\automouse"; Check: RemoveSettings

[Code]
var
  RemoveSettingsChecked: Boolean;

// Ask at uninstall time whether to also delete saved settings and presets.
function InitializeUninstall(): Boolean;
begin
  RemoveSettingsChecked := False;
  if DirExists(ExpandConstant('{userappdata}\automouse')) then
    RemoveSettingsChecked :=
      SuppressibleMsgBox('Also delete your AutoMouse settings and saved presets?',
        mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES;
  Result := True;
end;

function RemoveSettings(): Boolean;
begin
  Result := RemoveSettingsChecked;
end;
