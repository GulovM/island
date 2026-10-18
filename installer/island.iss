; Установщик Island (Inno Setup 6). Сборка: powershell -File tools\package.ps1 (или ISCC installer\island.iss).
; Версию можно переопределить: ISCC /DAppVersion=1.2.3 installer\island.iss
#ifndef AppVersion
  #define AppVersion "0.3.0"
#endif

#define AppName "Island"
#define AppExe "island.exe"

[Setup]
; Не менять AppId между версиями: по нему обновление находит предыдущую установку.
AppId={{6B0C2F4E-3D5A-4C1B-9E71-5A2D8F0B7C13}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=GulovM
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; Установка для текущего пользователя, права администратора не нужны.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir=..\dist
OutputBaseFilename=island-{#AppVersion}-setup
SetupIconFile=..\assets\island.ico
UninstallDisplayIcon={app}\{#AppExe}
LicenseFile=..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=force
RestartApplications=no

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "{cm:AutoStart}"; GroupDescription: "{cm:AdditionalOptions}"; Flags: checkedonce
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[CustomMessages]
russian.AutoStart=Запускать Island при входе в Windows
russian.AdditionalOptions=Дополнительно:
russian.RemoveData=Удалить настройки и кэш Island (%APPDATA%\island)?
english.AutoStart=Start Island when I sign in to Windows
english.AdditionalOptions=Options:
english.RemoveData=Also delete Island settings and cache (%APPDATA%\island)?

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion restartreplace
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; То же значение, что пишет переключатель «Автозапуск» в настройках программы (sys.rs).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Island"; ValueData: """{app}\{#AppExe}"""; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/IM {#AppExe} /F"; Flags: runhidden; RunOnceId: "KillIsland"

[Code]
// Запущенный остров держит island.exe открытым: гасим его до копирования файлов, иначе обновление
// упирается в «файл уже существует / занят». Restart Manager (CloseApplications) справляется не всегда.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  // Сначала вежливо (WM_CLOSE), и только если остров не закрылся — принудительно.
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(1500);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExe} /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(300);
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    // Автозапуск мог быть включён из самой программы — ключ чистим в любом случае.
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Island');
    if (not UninstallSilent) and
       (MsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES) then
      DelTree(ExpandConstant('{userappdata}\island'), True, True, True);
  end;
end;
