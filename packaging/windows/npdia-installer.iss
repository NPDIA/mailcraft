; SPDX-FileCopyrightText: MailCraft contributors
; SPDX-License-Identifier: MIT OR Apache-2.0

#define ProductName "MailCraft_NPDIA_Custom"
#define ProductKey "Software\NPDIA\MailCraft_NPDIA_Custom"

[Setup]
AppId={{EC63F90F-7AB7-4C04-9795-52E56E4D7FA4}
AppName={#ProductName}
AppVersion={#AppVersion}
AppPublisher=NPDIA
AppPublisherURL=https://github.com/NPDIA/mailcraft
DefaultDirName={localappdata}\Programs\{#ProductName}
DefaultGroupName={#ProductName}
DisableDirPage=auto
UsePreviousAppDir=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#if PREPROCVER >= 0x07000000
SetupArchitecture=x64
#endif
MinVersion=10.0
WizardStyle=modern
ShowLanguageDialog=yes
UninstallDisplayIcon={app}\mailcraft.exe
CloseApplications=yes
RestartApplications=no
SetupIconFile=..\..\assets\app-icon\mailcraft.ico
OutputDir={#OutputDir}
OutputBaseFilename={#ProductName}-{#AppVersion}-windows-x64
VersionInfoVersion={#NumericVersion}
VersionInfoTextVersion={#AppVersion}
Compression=lzma2
SolidCompression=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"

[CustomMessages]
english.ForkInstallPath=An existing MailCraft_NPDIA_Custom installation must be updated in its current folder.
korean.ForkInstallPath=기존 MailCraft_NPDIA_Custom 설치는 현재 설치 폴더에서 업데이트해야 합니다.
english.ForeignInstallPath=That folder already contains MailCraft. Choose a separate folder for MailCraft_NPDIA_Custom.
korean.ForeignInstallPath=선택한 폴더에 이미 MailCraft가 있습니다. MailCraft_NPDIA_Custom의 설치 폴더를 따로 선택하세요.

[Files]
Source: "{#BinDir}\mailcraft.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\mailcraft-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\..\NOTICE"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\..\ATTRIBUTION.md"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Registry]
Root: HKCU; Subkey: "{#ProductKey}"; ValueType: string; ValueName: "Executable"; ValueData: "{app}\mailcraft.exe"; Flags: uninsdeletevalue uninsdeletekeyifempty

[Icons]
Name: "{group}\{#ProductName}"; Filename: "{app}\mailcraft.exe"; WorkingDir: "{app}"; AppUserModelID: "MailCraft.NPDIA.Custom"

[Run]
Filename: "{app}\mailcraft.exe"; Description: "{cm:LaunchProgram,{#ProductName}}"; Flags: nowait postinstall skipifsilent unchecked

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  CurrentExecutable: String;
  SelectedExecutable: String;
begin
  Result := '';
  SelectedExecutable := ExpandConstant('{app}\mailcraft.exe');
  if RegQueryStringValue(HKCU, '{#ProductKey}', 'Executable', CurrentExecutable) then begin
    if CompareText(CurrentExecutable, SelectedExecutable) <> 0 then
      Result := CustomMessage('ForkInstallPath');
  end else if FileExists(SelectedExecutable) or FileExists(ExpandConstant('{app}\mailcraft-cli.exe')) then
    Result := CustomMessage('ForeignInstallPath');
end;
