#ifndef KXAppVersion
#error The release build must pass /DKXAppVersion=<version>
#endif
#ifndef KXPackageDir
#define KXPackageDir "..\..\dist\kaixin-package"
#endif
#ifndef KXOutputBaseFilename
#define KXOutputBaseFilename "kaixin-user-setup"
#endif
#ifndef KXIncludeTranslation
#define KXIncludeTranslation "0"
#endif
#ifndef KXIncludeOcr
#define KXIncludeOcr "1"
#endif
#ifndef KXComponentRapidOcrPayload
#define KXComponentRapidOcrPayload ""
#endif
#define KXMachineInstall 0
#define KXBaseAppName "开心输入法"
#if KXIncludeOcr == "1"
#define KXAppName "开心输入法 + OCR"
#else
#define KXAppName "开心输入法"
#endif

[Setup]
AppId={{2B91F956-7D55-4D85-B3A6-456A4A5DBB84}-User
AppName={#KXBaseAppName}
AppVerName={#KXAppName} {#KXAppVersion}
AppVersion={#KXAppVersion}
VersionInfoVersion={#KXAppVersion}.0
VersionInfoTextVersion={#KXAppVersion}
AppPublisher={#KXBaseAppName}
UninstallDisplayName={#KXBaseAppName}
VersionInfoDescription={#KXAppName} 安装程序
SetupIconFile=..\..\assets\kaixin-input.ico
DefaultDirName={localappdata}\Programs\kaixin
DefaultGroupName={#KXBaseAppName}
DisableDirPage=yes
DisableProgramGroupPage=yes
OutputDir=..\..\dist
OutputBaseFilename={#KXOutputBaseFilename}
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64os
PrivilegesRequired=lowest
WizardStyle=modern
AlwaysRestart=no
CloseApplications=no
RestartApplications=no
RestartIfNeededByRun=no

[Languages]
Name: "chinesesimp"; MessagesFile: "ChineseSimplified.isl"

[Files]
Source: "{#KXPackageDir}\srf_ime_setup.exe"; DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "{#KXPackageDir}\*"; DestDir: "{app}"; Excludes: "srf_ime_setup.exe,runtime\*,RapidOCRLite\*,models\*,component_manifest.ini"; Flags: ignoreversion recursesubdirs createallsubdirs restartreplace uninsrestartdelete solidbreak
Source: "{#KXPackageDir}\runtime\*"; DestDir: "{app}\runtime"; Flags: ignoreversion onlyifdoesntexist recursesubdirs createallsubdirs uninsrestartdelete solidbreak
Source: "{#KXPackageDir}\RapidOCRLite\*"; DestDir: "{app}\RapidOCRLite"; Flags: ignoreversion recursesubdirs createallsubdirs restartreplace uninsrestartdelete skipifsourcedoesntexist solidbreak; Check: ShouldInstallRapidOcrPayload
Source: "{#KXPackageDir}\component_manifest.ini"; DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete solidbreak

[InstallDelete]
#include "retired_lexicons.generated.iss"
Type: filesandordirs; Name: "{app}\ShareX"
Type: filesandordirs; Name: "{app}\RapidOCRLite"; Check: ShouldInstallRapidOcrPayload
Type: filesandordirs; Name: "{app}\models\translate"
Type: filesandordirs; Name: "{app}\ShareX-Source"
Type: files; Name: "{group}\开心输入法 OCR.lnk"; Check: not OcrIncluded
Type: files; Name: "{app}\srf_ime_settings.exe"
Type: files; Name: "{app}\srf_ime_tray.exe"
Type: files; Name: "{app}\srf_ime_clipboard.exe"
Type: files; Name: "{app}\srf_ime_handwrite.exe"
Type: files; Name: "{app}\srf_ime_symbols.exe"
Type: files; Name: "{app}\srf_ime_keyboard.exe"
Type: files; Name: "{app}\srf_ime_ocr.exe"
Type: files; Name: "{app}\srf_ime_settings.exe.manifest"
Type: files; Name: "{app}\srf_ime_tray.exe.manifest"
Type: filesandordirs; Name: "{app}\RapidOCRLite"; Check: not OcrIncluded
Type: files; Name: "{group}\开心输入法中英翻译.lnk"
Type: files; Name: "{app}\srf_ime_translate.exe"
; Clean up the retired Python components when upgrading an existing installation.
Type: filesandordirs; Name: "{app}\.python-runtime"
Type: filesandordirs; Name: "{app}\.python-packages"
Type: filesandordirs; Name: "{app}\.venv-rapidocr"
Type: filesandordirs; Name: "{app}\.venv-translate"
Type: files; Name: "{app}\tools\kaixin_ocr_engine.py"
Type: files; Name: "{app}\tools\kaixin_ocr_engine.cmd"
Type: files; Name: "{app}\tools\kaixin_ocr_policy.py"
Type: files; Name: "{app}\tools\kaixin_cv_crop.py"
Type: files; Name: "{app}\tools\kaixin_translate_engine.py"
Type: files; Name: "{app}\tools\kaixin_translate_engine.cmd"
Type: dirifempty; Name: "{app}\tools"
Type: files; Name: "{app}\lexicon\translate\cedict_ts.u8"
Type: files; Name: "{app}\lexicon\translate\stardict_slim.db"
Type: dirifempty; Name: "{app}\lexicon\translate"
Type: dirifempty; Name: "{app}\models"
Type: files; Name: "{app}\install_dev.ps1"
Type: files; Name: "{app}\install_current_user.ps1"
Type: files; Name: "{app}\install_common.ps1"
Type: files; Name: "{app}\config_defaults.generated.ps1"
Type: files; Name: "{app}\invoke_registration.ps1"
Type: files; Name: "{app}\repair_install.ps1"
Type: files; Name: "{app}\uninstall_dev.ps1"
Type: files; Name: "{app}\uninstall_current_user.ps1"

[UninstallDelete]
Type: files; Name: "{app}\install_machine.log"
Type: files; Name: "{app}\tip_registration.log"
Type: files; Name: "{app}\tip_registration_user.log"

[Icons]
Name: "{group}\开心输入法设置"; Filename: "{app}\srf_ime_desktop.exe"; Parameters: "--tool=settings"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法剪贴板"; Filename: "{app}\srf_ime_desktop.exe"; Parameters: "--tool=clipboard"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法手写查字"; Filename: "{app}\srf_ime_desktop.exe"; Parameters: "--tool=handwrite"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法符号大全"; Filename: "{app}\srf_ime_desktop.exe"; Parameters: "--tool=symbols"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法屏幕键盘"; Filename: "{app}\srf_ime_desktop.exe"; Parameters: "--tool=keyboard"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法 OCR"; Filename: "{app}\srf_ime_desktop.exe"; Parameters: "--tool=ocr"; IconFilename: "{app}\assets\kaixin-input.ico"; Check: OcrIncluded
Name: "{group}\修复开心输入法"; Filename: "{app}\srf_ime_setup.exe"; Parameters: "repair --root ""{app}"""; WorkingDir: "{app}"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法日志"; Filename: "{win}\explorer.exe"; Parameters: """{localappdata}\kaixin"""; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\导出开心输入法诊断包"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\export_diagnostics.ps1"" -InstallationRoot ""{app}"""; WorkingDir: "{app}"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\卸载开心输入法"; Filename: "{uninstallexe}"; IconFilename: "{app}\assets\kaixin-input.ico"

[Run]
Filename: "{sys}\shutdown.exe"; Parameters: "/r /t 0"; Description: "安装完成后立即重启电脑"; Flags: postinstall unchecked skipifsilent runhidden nowait

[Code]
const
  KaixinStateRegKey = 'Software\kaixin\State';
  KaixinInstallMaintenanceValue = 'InstallMaintenance';
  KaixinInstallMaintenanceTickValue = 'InstallMaintenanceTick';

#include "kaixin-common-code.iss"

function TranslationIncluded(): Boolean;
begin
  Result := '{#KXIncludeTranslation}' = '1';
end;

function OcrIncluded(): Boolean;
begin
  Result := '{#KXIncludeOcr}' = '1';
end;

function ShouldInstallRapidOcrPayload(): Boolean;
begin
  InitializeComponentChecks();
  Result := OcrIncluded() and InstallRapidOcrPayloadComponent;
end;

function StatusText(const Ok: Boolean): string;
begin
  if Ok then
    Result := '成功'
  else
    Result := '请查看日志';
end;

function BuildFinishedSummary(): string;
var
  UserLog: string;
  UserOk: Boolean;
  TrayOk: Boolean;
begin
  UserLog := ExpandConstant('{localappdata}\kaixin\install_user.log');
  UserOk := CurrentInstallLogContains(UserLog, 'event=install_health_check status=ok');
  TrayOk := CurrentInstallLogContains(UserLog, 'OK: started tray helper');

  Result :=
    '{#KXAppName} 已安装完成。' + #13#10 + #13#10 +
    'TSF 注册：' + StatusText(UserOk) + #13#10 +
    '输入法列表：' + StatusText(UserOk) + #13#10 +
    '托盘启动：' + StatusText(TrayOk) + #13#10;
  if OcrIncluded() then
    Result := Result + 'OCR 扩展：已安装' + #13#10
  else
    Result := Result + 'OCR 扩展：未安装' + #13#10;
  if TranslationIncluded() then
    Result := Result + '外部翻译联动：已启用（需另行安装 HY-MT2 翻译机或 WinTranslator）' + #13#10
  else
    Result := Result + '外部翻译联动：未启用' + #13#10;
  Result := Result + #13#10 +
    '日志目录：' + ExpandConstant('{localappdata}\kaixin') + #13#10 +
    '按 Win + 空格选择开心输入法。升级后，已打开的软件可在重新打开后使用新版。';
end;

function InitializeSetup(): Boolean;
begin
  Result := InitializeSetupShared();
end;

function GetInstallUserParams(): string;
var
  AppDir: string;
begin
  AppDir := ExpandConstant('{app}');
  Result := 'install-user --root "' + AppDir + '" --verify-core-only --report-setup-status --skip-runtime-cleanup';
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpFinished) and not WizardForm.YesRadio.Visible then
    LayoutFinishedPage(BuildFinishedSummary());
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
    SetInstallMaintenance(False);
    RunSetupHelper(GetInstallUserParams(), '正在校验核心文件、注册输入法并刷新语言栏…', False);
  end;
end;

procedure DeinitializeSetup();
begin
  SetInstallMaintenance(False);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveCurrentUserData();
end;

procedure DeinitializeUninstall();
begin
  if (not UninstallSilent) and DirExists(ExpandConstant('{app}\runtime')) then
    MsgBox('部分运行时文件可能仍被系统或应用占用，将在相关程序退出或下次重启后自动清理。', mbInformation, MB_OK);
end;
