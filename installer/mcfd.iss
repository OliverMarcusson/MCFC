; Per-user x64 installer for the MCFC host-bridge service.
; The packaging script supplies MyAppVersion and MyAppSource.

#ifndef MyAppVersion
  #error MyAppVersion must be supplied by scripts/package-mcfd.ps1
#endif
#ifndef MyAppSource
  #error MyAppSource must be supplied by scripts/package-mcfd.ps1
#endif

#define MyAppName "MCFC mcfd"
#define MyAppPublisher "MCFC"
#define MyAppExeName "mcfd.exe"

[Setup]
AppId={{6B6A6A5C-FF16-4EED-849C-1D44520A369B}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\MCFC\mcfd
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=commandline
UsedUserAreasWarning=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=mcfd-{#MyAppVersion}-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName={#MyAppName}
UninstallDisplayIcon={app}\{#MyAppExeName}

[Files]
Source: "{#MyAppSource}\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyAppSource}\README.txt"; DestDir: "{app}"; Flags: ignoreversion

[Run]
Filename: "{app}\{#MyAppExeName}"; Parameters: "service install"; Flags: runhidden waituntilterminated
Filename: "{sys}\schtasks.exe"; Parameters: "/Run /TN ""MCFC mcfd"""; Flags: runhidden waituntilterminated

[UninstallRun]
Filename: "{app}\{#MyAppExeName}"; Parameters: "service uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveMcfdLogonTask"

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\MCFC\mcfd"
