; Disk Flashlight installer (Inno Setup 6).
; Built by tools\make_release.py, which passes the version and the icon path:
;   ISCC /DMyAppVersion=0.1.0 /DVersionInfoVersion=0.1.0.0 /DAppIcon=..\target\app.ico tools\setup.iss

#define MyAppName "Disk Flashlight"
#define MyAppExeName "disk_flashlight.exe"
#define MyAppPublisher "Fan4_Metal"
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef VersionInfoVersion
  #define VersionInfoVersion "0.0.0.0"
#endif
#ifndef AppIcon
  #define AppIcon "..\target\app.ico"
#endif

[Setup]
AppId={{3E54EDD2-D754-4D66-BC36-0A020E427D6F}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
VersionInfoVersion={#VersionInfoVersion}
AppCopyright=Copyright (C) 2026 {#MyAppPublisher}
AppPublisher={#MyAppPublisher}
; Per-user installation without administrator rights:
; {autopf} points to %LOCALAPPDATA%\Programs.
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=Disk_Flashlight_{#MyAppVersion}_Setup
SetupIconFile={#AppIcon}
UninstallDisplayIcon={app}\{#MyAppExeName}
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
english.ContextMenuGroup=Explorer integration:
russian.ContextMenuGroup=Интеграция с Проводником:
english.ContextMenuTask=Add the "Analyze with Disk Flashlight" item to the context menu of folders and drives
russian.ContextMenuTask=Добавить пункт «Анализировать в Disk Flashlight» в контекстное меню папок и дисков
english.ContextMenuVerb=Analyze with Disk Flashlight
russian.ContextMenuVerb=Анализировать в Disk Flashlight

[Tasks]
Name: "contextmenu"; Description: "{cm:ContextMenuTask}"; GroupDescription: "{cm:ContextMenuGroup}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.ru.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Explorer context menu item for folders (Directory) and drives (Drive).
; The keys are under HKCU, as the installation needs no administrator rights;
; they are removed on uninstall. For a drive root Explorer passes "C:\", which
; the application receives as C:" and turns back into C:\ (see normalize in
; src\main.rs).
Root: HKCU; Subkey: "Software\Classes\Directory\shell\DiskFlashlight"; ValueType: string; ValueName: "MUIVerb"; ValueData: "{cm:ContextMenuVerb}"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\DiskFlashlight"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName}"; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\DiskFlashlight\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

Root: HKCU; Subkey: "Software\Classes\Drive\shell\DiskFlashlight"; ValueType: string; ValueName: "MUIVerb"; ValueData: "{cm:ContextMenuVerb}"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Drive\shell\DiskFlashlight"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName}"; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\Drive\shell\DiskFlashlight\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent
