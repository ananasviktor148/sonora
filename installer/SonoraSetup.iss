; Sonora Hi-Res Audio Player — Windows Installer Script (Inno Setup 6)

#define MyAppName "Sonora Hi-Res Audio Player"
#define MyAppShortName "Sonora"
#define MyAppVersion "1.0.0"
#define MyAppPublisher "Sonora Audio"
#define MyAppExeName "Sonora.exe"

[Setup]
AppId={{8F6D3C9A-4B21-4E9F-9A72-5C1E84D03B2A}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppShortName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
OutputDir=..\dist
OutputBaseFilename=Sonora-Setup-{#MyAppVersion}-x64
SetupIconFile=sonora.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "assoc"; Description: "Register Sonora as an Open With handler for Hi-Res audio files (.flac, .dsf, .dff, .wav, .wv, .ape, .m3u8)"; GroupDescription: "File Associations"

[Files]
Source: "..\target\release\hires_player.exe"; DestDir: "{app}"; DestName: "{#MyAppExeName}"; Flags: ignoreversion
Source: "sonora.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\sonora.ico"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppShortName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\sonora.ico"; Tasks: desktopicon

[Registry]
; Clean ProgID registration under HKA (HKCU for per-user, HKLM for all-users)
Root: HKA; Subkey: "Software\Classes\Sonora.HiResAudio"; ValueType: string; ValueName: ""; ValueData: "Sonora Hi-Res Audio File"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Sonora.HiResAudio\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\sonora.ico"",0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Sonora.HiResAudio\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: assoc

Root: HKA; Subkey: "Software\Classes\.flac\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.dsf\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.dff\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.wav\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.aiff\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.wv\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.ape\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.opus\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.m3u8\OpenWithProgids"; ValueType: string; ValueName: "Sonora.HiResAudio"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
