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
; mcfd prefers a Scheduled Task; a standard-user installation falls back to a
; per-user Windows Run entry when task registration is not permitted.
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
Source: "{#MyAppSource}\mcfd-agent.jar"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyAppSource}\mcfd-agent-attach.jar"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyAppSource}\README.txt"; DestDir: "{app}"; Flags: ignoreversion

[Code]
procedure InstallAndStartMcfd;
var
  ResultCode: Integer;
begin
  if not Exec(ExpandConstant('{app}\{#MyAppExeName}'), 'service install', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    RaiseException('Could not run mcfd service installation.')
  else if ResultCode <> 0 then
    RaiseException(Format('Could not create the MCFC mcfd logon task (exit code %d).', [ResultCode]));

  if not Exec(ExpandConstant('{sys}\schtasks.exe'), '/Run /TN "MCFC mcfd"', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    RaiseException('The MCFC mcfd logon task was created but could not be started.')
  else if ResultCode <> 0 then
    RaiseException(Format('The MCFC mcfd logon task was created but could not be started (exit code %d).', [ResultCode]));
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    InstallAndStartMcfd;
end;

[UninstallRun]
Filename: "{app}\{#MyAppExeName}"; Parameters: "service uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveMcfdLogonTask"

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\MCFC\mcfd"
