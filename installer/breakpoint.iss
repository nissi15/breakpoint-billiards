; Inno Setup script for the Breakpoint Billiards Windows installer.
; Built by .github/workflows/release.yml:
;   iscc /DAppVersion=1.0.0 installer\breakpoint.iss

#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif

[Setup]
AppId={{6E2B9B1C-6F6E-4C2D-9B8B-0B6D2C1A8E11}
AppName=Breakpoint Billiards
AppVersion={#AppVersion}
AppPublisher=Breakpoint Billiards
AppPublisherURL=https://github.com/nissi15/breakpoint-billiards
DefaultDirName={localappdata}\Programs\Breakpoint Billiards
DefaultGroupName=Breakpoint Billiards
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=BreakpointBilliards-{#AppVersion}-Setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\breakpoint-billiards.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "..\target\release\breakpoint-billiards.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Breakpoint Billiards"; Filename: "{app}\breakpoint-billiards.exe"
Name: "{group}\Uninstall Breakpoint Billiards"; Filename: "{uninstallexe}"
Name: "{userdesktop}\Breakpoint Billiards"; Filename: "{app}\breakpoint-billiards.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\breakpoint-billiards.exe"; Description: "Play Breakpoint Billiards now"; Flags: nowait postinstall skipifsilent
