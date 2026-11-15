; Wortfluss installer for Windows (Inno Setup 6). Built by CI:
;   iscc /DAppVersion=0.2.0 /DBinDir=..\target\release installer\wortfluss.iss
; Installs for the current user only: no administrator rights, nothing outside the
; user's profile. Settings and API keys are not part of the installer; `vtt setup` asks.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef BinDir
  #define BinDir "..\target\release"
#endif

[Setup]
AppId={{6C3F8A1E-4B2D-4F7A-9E51-57A0D3C2B8F4}
AppName=Wortfluss
AppVersion={#AppVersion}
AppPublisher=MIAK IT (Ammad Khan)
AppPublisherURL=https://miak-it.de
AppSupportURL=https://github.com/1a-khan/wortfluss
AppUpdatesURL=https://github.com/1a-khan/wortfluss/releases
AppComments=Hold a key, speak, and the text is typed into any app. Sessions transcribe lectures and meetings with notes.
DefaultDirName={localappdata}\Programs\Wortfluss
DefaultGroupName=Wortfluss
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=Output
OutputBaseFilename=Wortfluss-Setup-{#AppVersion}
SetupIconFile=wortfluss.ico
UninstallDisplayIcon={app}\wortfluss.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "autostart"; Description: "Start Wortfluss when I log in (recommended)"
Name: "addtopath"; Description: "Add the vtt command to the command line (PATH)"

[Files]
Source: "{#BinDir}\vtt.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\wortfluss.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "wortfluss.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\NOTICE"; DestDir: "{app}"; Flags: ignoreversion

[UninstallDelete]
; left behind by automatic updates (see src/update.rs)
Type: files; Name: "{app}\*.old"
Type: files; Name: "{app}\*.new"

[Icons]
Name: "{group}\Wortfluss"; Filename: "{app}\wortfluss.exe"; IconFilename: "{app}\wortfluss.ico"; Comment: "Start Wortfluss (tray icon)"
Name: "{group}\Wortfluss settings"; Filename: "{app}\vtt.exe"; Parameters: "setup"; IconFilename: "{app}\wortfluss.ico"; Comment: "Service, API key, dictation key, microphone"
Name: "{group}\Wortfluss help"; Filename: "{app}\vtt.exe"; Parameters: "help"; IconFilename: "{app}\wortfluss.ico"
Name: "{group}\Uninstall Wortfluss"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Wortfluss"; ValueData: """{app}\wortfluss.exe"""; Tasks: autostart; Flags: uninsdeletevalue

[Run]
; First start: the setup asks for the service, API key, dictation key and microphone,
; then starts Wortfluss in the background.
Filename: "{app}\vtt.exe"; Parameters: "setup"; Description: "Set up Wortfluss now (service, API key, dictation key)"; Flags: postinstall nowait skipifsilent

[UninstallRun]
Filename: "{app}\vtt.exe"; Parameters: "stop"; Flags: runhidden; RunOnceId: "StopWortfluss"
Filename: "{app}\vtt.exe"; Parameters: "session stop"; Flags: runhidden; RunOnceId: "StopSession"
Filename: "{app}\vtt.exe"; Parameters: "autostart off"; Flags: runhidden; RunOnceId: "AutostartOff"

[Code]
const EnvKey = 'Environment';

procedure AddToPath(Dir: string);
var Path: string;
begin
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Path) then Path := '';
  if Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') > 0 then exit;
  if (Path <> '') and (Copy(Path, Length(Path), 1) <> ';') then Path := Path + ';';
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Path + Dir);
end;

procedure RemoveFromPath(Dir: string);
var Path: string; P: Integer;
begin
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Path) then exit;
  P := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';');
  if P = 0 then exit;
  if P = 1 then
    Delete(Path, 1, Length(Dir) + 1)
  else
    Delete(Path, P - 1, Length(Dir) + 1);
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Path);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddToPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
