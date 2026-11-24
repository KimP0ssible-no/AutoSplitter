; Inno Setup 6 script for the Windows installer.
; Build: cargo build --release, fill the stage folder (see .github/workflows/ci.yml), then
;   iscc /DStage=<stage folder> packaging\windows\autosplitter.iss
; The installer is written to packaging\windows\Output\.

#define AppVersion "0.0.0"
#ifndef Stage
  #define Stage "..\..\target\stage"
#endif

[Setup]
AppId={{E64EEC19-3D84-49A6-9458-91BCE0843A91}
AppName=AutoSplitter
AppVersion={#AppVersion}
AppPublisher=AutoSplitter
AppPublisherURL=https://github.com/KimP0ssible-no/AutoSplitter
VersionInfoVersion={#AppVersion}
VersionInfoProductTextVersion={#AppVersion}
DefaultDirName={autopf}\AutoSplitter
DefaultGroupName=AutoSplitter
DisableProgramGroupPage=yes
OutputBaseFilename=autosplitter-{#AppVersion}-x64-setup
SetupIconFile=..\icons\autosplitter.ico
UninstallDisplayIcon={app}\AutoSplitter.exe
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; CI release builds: iscc /DSign "/Scertum=signtool sign ... $f" signs the
; setup and its uninstaller.
#ifdef Sign
SignTool=certum
#endif

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
; Profiles are not here: they live in %APPDATA%\AutoSplitter\profiles and stay on uninstall.
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\AutoSplitter"; Filename: "{app}\AutoSplitter.exe"
Name: "{group}\AutoSplitter Profile Editor"; Filename: "{app}\ProfileEditor.exe"
Name: "{autodesktop}\AutoSplitter"; Filename: "{app}\AutoSplitter.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\AutoSplitter.exe"; Description: "Launch AutoSplitter"; Flags: nowait postinstall skipifsilent
; The in-app update runs setup silently with /RELAUNCH=1 and has quit by then:
; start AutoSplitter again, as the user rather than the administrator. Other
; silent installs (winget, scripts) leave it closed.
Filename: "{app}\AutoSplitter.exe"; Flags: nowait skipifnotsilent runasoriginaluser; Check: Relaunch

[Code]
function Relaunch: Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;
