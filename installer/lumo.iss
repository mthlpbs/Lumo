; Lumo installer (Inno Setup 6.5.2+, for the PNG wizard images).
;
; Releases ship installer/lumo.nsi (NSIS), which 0.1.x installs came from; CI builds this one too as a check.
;
; Build after `cargo build -p lumo --release`:
;   iscc installer\lumo.iss                      -> dist\Lumo-Setup-2026.1107.2.exe
;   iscc /DAppVersion=2026.1107.3 installer\lumo.iss   (another build)
;
; Per-user install: no admin rights and no UAC prompt. The app goes to
; %LOCALAPPDATA%\Programs\Lumo; settings and data stay in %APPDATA%\Lumo and
; survive uninstalling (only the cache in %LOCALAPPDATA%\Lumo is removed).

#ifndef AppVersion
  #define AppVersion "2026.1109.10"
#endif
#ifndef ExeSource
  #define ExeSource "..\target\release\lumo.exe"
#endif
#define AppName "Lumo"
; Folder, data and Run value names stay "Lumo" (the app and 0.1.x installs use them).
#define AppDir "Lumo"
#define AppPublisher "mthlpbs"
#define AppExe "lumo.exe"
; The island window's class (crates/lumo/src/window.rs), used to ask a running copy to quit.
#define IslandClass "Lumo.Island"

[Setup]
; Never change the AppId: upgrades and the uninstaller find the install by it.
AppId={{6F1D2B7A-3C84-4E59-A1D6-8B2E5F0C9A47}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppCopyright=Copyright (C) 2026 {#AppPublisher}
VersionInfoVersion={#AppVersion}
VersionInfoCompany={#AppPublisher}
VersionInfoDescription={#AppName} Setup
VersionInfoProductName={#AppName}
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\{#AppDir}
DisableDirPage=yes
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=Lumo-Setup-{#AppVersion}
SetupIconFile=..\crates\lumo\res\lumo.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
; Supplied Lumo logo on the welcome page and header (tools/make_brand.py).
; Setup picks the file closest to its image area: 100%, 150% and 200% scaling.
WizardImageFile=art\wizard-202x386.png,art\wizard-336x643.png,art\wizard-430x824.png
WizardSmallImageFile=art\wizard-small-58.png,art\wizard-small-77.png,art\wizard-small-97.png,art\wizard-small-116.png,art\wizard-small-124.png,art\wizard-small-143.png,art\wizard-small-159.png
Compression=lzma2/max
SolidCompression=yes
; A running copy is asked to quit in PrepareToInstall; Restart Manager is the safety net.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start with Windows"; GroupDescription: "Startup:"

[Files]
Source: "{#ExeSource}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\crates\lumo\vendor\webview2\WebView2Loader.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; The shortcut the 0.1.x versions from 0.1.2 on made, before the app was renamed Lumo.
Type: files; Name: "{autoprograms}\Oyen Poni.lnk"
Type: files; Name: "{autoprograms}\Oyenza Studio.lnk"

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "Lumo"

[Registry]
; Same value the app writes from its "Start with Windows" menu item (autostart.rs): the quoted exe path.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppDir}"; ValueData: """{app}\{#AppExe}"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Make sure no copy is still running before its files are removed.
Filename: "{sys}\taskkill.exe"; Parameters: "/IM {#AppExe} /F"; Flags: runhidden waituntilterminated; RunOnceId: "StopLumo"

[UninstallDelete]
; Cache and log only; settings and data in %APPDATA%\Lumo are kept.
Type: filesandordirs; Name: "{localappdata}\Lumo"

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  WM_CLOSE = $0010;

{ Ask a running Lumo to quit (it saves its data on the way out); force it if it doesn't within 3 s. }
procedure StopLumo();
var
  I, ResultCode: Integer;
begin
  if FindWindowByClassName('{#IslandClass}') = 0 then
    Exit;
  PostMessage(FindWindowByClassName('{#IslandClass}'), WM_CLOSE, 0, 0);
  for I := 1 to 30 do
  begin
    if FindWindowByClassName('{#IslandClass}') = 0 then
      Exit;
    Sleep(100);
  end;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExe} /F', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopLumo();
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    RegDeleteValue(HKEY_CURRENT_USER, RunKey, 'Poni');
  { Unticking "Start with Windows" on a reinstall turns it off, even if the app's menu turned it on. }
  if (CurStep = ssPostInstall) and not WizardIsTaskSelected('autostart') then
    RegDeleteValue(HKEY_CURRENT_USER, RunKey, '{#AppDir}');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { Runs before [UninstallRun], so a running copy gets the chance to save and quit cleanly. }
  if CurUninstallStep = usUninstall then
    StopLumo();
  { The app may have written the Run value itself; never leave it pointing at a removed exe. }
  if CurUninstallStep = usPostUninstall then
    RegDeleteValue(HKEY_CURRENT_USER, RunKey, '{#AppDir}');
end;
