#ifndef ReleaseVersion
  #error ReleaseVersion is required
#endif
#ifndef ReleaseArch
  #error ReleaseArch is required
#endif
#define Stage "..\..\target\release-staging\tacterm-" + ReleaseVersion + "-windows-" + ReleaseArch + "-portable"

[Setup]
AppId=io.github.narma.tacterm
AppName=tacterm
AppVersion={#ReleaseVersion}
AppPublisher=tacterm contributors
AppPublisherURL=https://tacterm.dev/
DefaultDirName={autopf}\tacterm
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DisableProgramGroupPage=yes
OutputDir=..\..\target\release-assets
OutputBaseFilename=tacterm-{#ReleaseVersion}-windows-{#ReleaseArch}-setup
SetupIconFile=..\..\misc\icons\tacterm.ico
UninstallDisplayIcon={app}\tacterm.exe
LicenseFile=..\..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=tacterm.exe
RestartApplications=no
Uninstallable=yes
#if ReleaseArch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
MinVersion=10.0.22000
#elif ReleaseArch == "x64"
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
#else
  #error Unsupported ReleaseArch
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Stage}\tacterm.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Stage}\release-info.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\extras\rio.terminfo"; DestDir: "{app}\extras"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\tacterm"; Filename: "{app}\tacterm.exe"
Name: "{autodesktop}\tacterm"; Filename: "{app}\tacterm.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\tacterm.exe"; Description: "{cm:LaunchProgram,tacterm}"; Flags: nowait postinstall skipifsilent unchecked runasoriginaluser
