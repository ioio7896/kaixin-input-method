#ifndef KXAppVersion
#error KXAppVersion must be passed by build.py with /DKXAppVersion=<version>
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
#ifndef KXComponentPythonRuntime
#define KXComponentPythonRuntime ""
#endif
#ifndef KXComponentRapidOcrPackages
#define KXComponentRapidOcrPackages ""
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
Compression=lzma2/fast
SolidCompression=yes
ArchitecturesAllowed=x64compatible
PrivilegesRequired=lowest
WizardStyle=modern
AlwaysRestart=no
CloseApplications=no
RestartApplications=no
RestartIfNeededByRun=no

[Languages]
Name: "chinesesimp"; MessagesFile: "ChineseSimplified.isl"

[Files]
Source: "{#KXPackageDir}\*"; DestDir: "{app}"; Excludes: "runtime\*,.python-runtime\*,.python-packages\*,.venv-rapidocr\*,.venv-translate\*,RapidOCR-3.9.0\*,models\*,component_manifest.ini"; Flags: ignoreversion recursesubdirs createallsubdirs restartreplace uninsrestartdelete
Source: "{#KXPackageDir}\runtime\*"; DestDir: "{app}\runtime"; Flags: ignoreversion onlyifdoesntexist recursesubdirs createallsubdirs uninsrestartdelete solidbreak
Source: "{#KXPackageDir}\.python-runtime\*"; DestDir: "{app}\.python-runtime"; Flags: ignoreversion recursesubdirs createallsubdirs restartreplace uninsrestartdelete skipifsourcedoesntexist solidbreak; Check: ShouldInstallPythonRuntime
Source: "{#KXPackageDir}\.python-packages\*"; DestDir: "{app}\.python-packages"; Flags: ignoreversion recursesubdirs createallsubdirs restartreplace uninsrestartdelete skipifsourcedoesntexist solidbreak; Check: ShouldInstallRapidOcrPackages
Source: "{#KXPackageDir}\RapidOCR-3.9.0\*"; DestDir: "{app}\RapidOCR-3.9.0"; Flags: ignoreversion recursesubdirs createallsubdirs restartreplace uninsrestartdelete skipifsourcedoesntexist solidbreak; Check: ShouldInstallRapidOcrPayload
Source: "{#KXPackageDir}\component_manifest.ini"; DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete solidbreak

[InstallDelete]
Type: filesandordirs; Name: "{app}\ShareX"
Type: filesandordirs; Name: "{app}\.python-runtime"; Check: ShouldInstallPythonRuntime
Type: filesandordirs; Name: "{app}\.python-packages"; Check: ShouldInstallRapidOcrPackages
Type: filesandordirs; Name: "{app}\.venv-rapidocr"
Type: filesandordirs; Name: "{app}\RapidOCR-3.9.0"; Check: ShouldInstallRapidOcrPayload
Type: filesandordirs; Name: "{app}\.venv-translate"
Type: filesandordirs; Name: "{app}\models\translate"
Type: filesandordirs; Name: "{app}\ShareX-Source"
Type: files; Name: "{group}\开心输入法 OCR.lnk"; Check: not OcrIncluded
Type: files; Name: "{app}\srf_ime_ocr.exe"; Check: not OcrIncluded
Type: files; Name: "{app}\tools\kaixin_ocr_engine.py"; Check: not OcrIncluded
Type: files; Name: "{app}\tools\kaixin_ocr_engine.cmd"; Check: not OcrIncluded
Type: files; Name: "{app}\tools\kaixin_cv_crop.py"; Check: not OcrIncluded
Type: filesandordirs; Name: "{app}\.python-runtime"; Check: not OcrIncluded
Type: filesandordirs; Name: "{app}\.python-packages"; Check: not OcrIncluded
Type: filesandordirs; Name: "{app}\RapidOCR-3.9.0"; Check: not OcrIncluded
Type: files; Name: "{group}\开心输入法中英翻译.lnk"
Type: files; Name: "{app}\srf_ime_translate.exe"
Type: files; Name: "{app}\tools\kaixin_translate_engine.py"
Type: files; Name: "{app}\tools\kaixin_translate_engine.cmd"
Type: dirifempty; Name: "{app}\tools"
Type: files; Name: "{app}\lexicon\translate\cedict_ts.u8"
Type: files; Name: "{app}\lexicon\translate\stardict_slim.db"
Type: dirifempty; Name: "{app}\lexicon\translate"
Type: dirifempty; Name: "{app}\models"

[Icons]
Name: "{group}\开心输入法设置"; Filename: "{app}\srf_ime_settings.exe"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法剪贴板"; Filename: "{app}\srf_ime_clipboard.exe"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法手写查字"; Filename: "{app}\srf_ime_handwrite.exe"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法 OCR"; Filename: "{app}\srf_ime_ocr.exe"; IconFilename: "{app}\assets\kaixin-input.ico"; Check: OcrIncluded
Name: "{group}\修复开心输入法"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\repair_install.ps1"" -InstallationRoot ""{app}"""; WorkingDir: "{app}"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\开心输入法日志"; Filename: "{win}\explorer.exe"; Parameters: """{localappdata}\kaixin"""; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\导出开心输入法诊断包"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\export_diagnostics.ps1"" -InstallationRoot ""{app}"""; WorkingDir: "{app}"; IconFilename: "{app}\assets\kaixin-input.ico"
Name: "{group}\卸载开心输入法"; Filename: "{uninstallexe}"; IconFilename: "{app}\assets\kaixin-input.ico"

[Run]
Filename: "{sys}\shutdown.exe"; Parameters: "/r /t 0"; Description: "立即重启电脑"; Flags: postinstall skipifsilent unchecked runhidden

[Code]
const
  KaixinStateRegKey = 'Software\kaixin\State';
  KaixinInstallMaintenanceValue = 'InstallMaintenance';
  KaixinInstallMaintenanceTickValue = 'InstallMaintenanceTick';

var
  DeleteUserData: Boolean;
  CleanupTransientUserData: Boolean;

#include "kaixin-common-code.iss"

function WindowsGetTickCount(): LongWord;
  external 'GetTickCount@kernel32.dll stdcall';

function TranslationIncluded(): Boolean;
begin
  Result := '{#KXIncludeTranslation}' = '1';
end;

function OcrIncluded(): Boolean;
begin
  Result := '{#KXIncludeOcr}' = '1';
end;

function ShouldInstallPythonRuntime(): Boolean;
begin
  InitializeComponentChecks();
  Result := OcrIncluded() and InstallPythonRuntimeComponent;
end;

function ShouldInstallRapidOcrPackages(): Boolean;
begin
  InitializeComponentChecks();
  Result := OcrIncluded() and InstallRapidOcrPackagesComponent;
end;

function ShouldInstallRapidOcrPayload(): Boolean;
begin
  InitializeComponentChecks();
  Result := OcrIncluded() and InstallRapidOcrPayloadComponent;
end;

function ProcessRunning(const ImageName: string): Boolean;
var
  ResultCode: Integer;
  TempFile: string;
  Text: AnsiString;
begin
  Result := False;
  TempFile := ExpandConstant('{tmp}\kaixin-tasklist-') + ImageName + '.txt';
  if Exec(ExpandConstant('{sys}\cmd.exe'),
    '/C tasklist /FI "IMAGENAME eq ' + ImageName + '" /NH > "' + TempFile + '"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
  begin
    if LoadStringFromFile(TempFile, Text) then
      Result := Pos(ImageName, Text) > 0;
  end;
  DeleteFile(TempFile);
end;

function FileContains(const FileName: string; const Needle: string): Boolean;
var
  Text: AnsiString;
begin
  Result := False;
  if LoadStringFromFile(FileName, Text) then
    Result := Pos(Needle, Text) > 0;
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
  UserOk := FileContains(UserLog, 'event=install_health_check status=ok');
  TrayOk := FileContains(UserLog, 'OK: started tray helper');

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
    Result := Result + '外部翻译联动：已启用（需另行安装 WinTranslator）' + #13#10
  else
    Result := Result + '外部翻译联动：未启用' + #13#10;
  Result := Result + #13#10 +
    '日志目录：' + ExpandConstant('{localappdata}\kaixin') + #13#10 +
    '开始菜单提供“修复开心输入法”和“开心输入法日志”。';
end;

function InitializeSetup(): Boolean;
begin
  Result := InitializeSetupShared();
end;

procedure SetInstallMaintenance(Enabled: Boolean);
begin
  if Enabled then begin
    RegWriteDWordValue(HKCU, KaixinStateRegKey, KaixinInstallMaintenanceTickValue,
      WindowsGetTickCount());
    RegWriteDWordValue(HKCU, KaixinStateRegKey, KaixinInstallMaintenanceValue, 1);
  end else begin
    RegDeleteValue(HKCU, KaixinStateRegKey, KaixinInstallMaintenanceValue);
    RegDeleteValue(HKCU, KaixinStateRegKey, KaixinInstallMaintenanceTickValue);
  end;
end;

procedure WaitForProcessExit(const ImageName: string; Attempts: Integer);
var
  I: Integer;
begin
  for I := 1 to Attempts do
  begin
    if not ProcessRunning(ImageName) then
      exit;
    Sleep(100);
  end;
end;

procedure StopHelperProcess(const ImageName: string);
var
  ResultCode: Integer;
begin
  if not ProcessRunning(ImageName) then
    exit;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/T /IM "' + ImageName + '"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  WaitForProcessExit(ImageName, 5);
  if not ProcessRunning(ImageName) then
    exit;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /T /IM "' + ImageName + '"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  WaitForProcessExit(ImageName, 10);
end;

procedure RequestCloseHelperProcess(const ImageName: string);
var
  ResultCode: Integer;
begin
  if not ProcessRunning(ImageName) then
    exit;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/T /IM "' + ImageName + '"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

procedure StopInputStackProcess(const ImageName: string);
var
  ResultCode: Integer;
begin
  if not ProcessRunning(ImageName) then
    exit;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/T /IM "' + ImageName + '"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  WaitForProcessExit(ImageName, 5);
  if not ProcessRunning(ImageName) then
    exit;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /T /IM "' + ImageName + '"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  WaitForProcessExit(ImageName, 10);
end;

procedure RepairExistingAppAcl();
var
  ResultCode: Integer;
  AppDir: string;
  ProbeFile: string;
begin
  AppDir := ExpandConstant('{app}');
  if not DirExists(AppDir) then
    exit;
  ProbeFile := AddBackslash(AppDir) + '.kaixin-write-probe.tmp';
  if SaveStringToFile(ProbeFile, 'probe', False) then
  begin
    DeleteFile(ProbeFile);
    exit;
  end;
  Exec(ExpandConstant('{sys}\icacls.exe'), '"' + AppDir + '" /reset /T /C /Q',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  SetInstallMaintenance(True);
  RepairExistingAppAcl();
  StopInputStackProcess('TextInputHost.exe');
  StopInputStackProcess('ctfmon.exe');
  StopHelperProcess('srf_ime_engine.exe');
  StopHelperProcess('srf_ime_tray.exe');
  StopHelperProcess('KaixinShareX.exe');
  RequestCloseHelperProcess('srf_ime_settings.exe');
  RequestCloseHelperProcess('srf_ime_clipboard.exe');
  RequestCloseHelperProcess('srf_ime_handwrite.exe');
  RequestCloseHelperProcess('srf_ime_ocr.exe');
  RequestCloseHelperProcess('srf_ime_translate_result.exe');
  RequestCloseHelperProcess('srf_ime_translate.exe');
  StopHelperProcess('srf_ime_clipboard_svc.exe');
  WaitForProcessExit('srf_ime_settings.exe', 10);
  WaitForProcessExit('srf_ime_clipboard.exe', 10);
  WaitForProcessExit('srf_ime_handwrite.exe', 10);
  WaitForProcessExit('srf_ime_ocr.exe', 10);
  WaitForProcessExit('srf_ime_translate_result.exe', 10);
  WaitForProcessExit('srf_ime_translate.exe', 10);
  WaitForProcessExit('srf_ime_clipboard_svc.exe', 10);
  StopHelperProcess('srf_ime_settings.exe');
  StopHelperProcess('srf_ime_clipboard.exe');
  StopHelperProcess('srf_ime_handwrite.exe');
  StopHelperProcess('srf_ime_ocr.exe');
  StopHelperProcess('srf_ime_translate_result.exe');
  StopHelperProcess('srf_ime_translate.exe');
  StopHelperProcess('srf_ime_clipboard_svc.exe');
  Result := '';
end;

function GetInstallPs1Params(): string;
var
  AppDir: string;
begin
  AppDir := ExpandConstant('{app}');
  Result := '-NoProfile -ExecutionPolicy Bypass -File "' + AppDir + '\install_dev.ps1" ' +
    '-InstallationRoot "' + AppDir + '" -SkipFileCopy -SkipUninstallEntry -SkipStaleHostDiagnostics';
end;

function RunPowerShell(const Params: string; const StepName: string): Boolean;
var
  ResultCode: Integer;
  AppDir: string;
  LogMessage: string;
begin
  AppDir := ExpandConstant('{app}');
  Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    Params, AppDir, SW_HIDE, ewWaitUntilTerminated, ResultCode);
  if (not Result) or (ResultCode <> 0) then begin
    LogMessage :=
      '安装步骤失败：' + StepName + #13#10 #13#10 +
      '退出码：' + IntToStr(ResultCode) + #13#10 +
      '当前用户日志：' + ExpandConstant('{localappdata}\kaixin\install_user.log') + #13#10 +
      '语言列表日志：' + ExpandConstant('{localappdata}\kaixin\install_language_list.log');
    MsgBox(LogMessage, mbError, MB_OK);
    Result := False;
  end else begin
    Result := True;
  end;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpFinished then
    WizardForm.FinishedLabel.Caption := BuildFinishedSummary();
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
    SetInstallMaintenance(False);
    if not RunPowerShell(GetInstallPs1Params(), 'current-user install') then
      RaiseException('Install failed during current-user install.');
  end;
end;

procedure DeinitializeSetup();
begin
  SetInstallMaintenance(False);
end;

function InitializeUninstall(): Boolean;
var
  Choice: Integer;
begin
  DeleteUserData := CommandLineRequestsRemoveUserData();
  CleanupTransientUserData := (not DeleteUserData) and CommandLineRequestsRemoveTransientUserData();
  if (not UninstallSilent) and (not DeleteUserData) and (not CleanupTransientUserData) then begin
    Choice :=
      MsgBox(
        '请选择卸载后如何处理用户数据：' #13#10 #13#10 +
        '是：删除配置、用户词库、缓存和日志；剪贴板历史会保留，可在设置页单独清空。' #13#10 +
        '否：只清理缓存和日志，保留配置、用户词库和剪贴板。' #13#10 +
        '取消：仅卸载程序，保留所有用户数据。',
        mbConfirmation,
        MB_YESNOCANCEL or MB_DEFBUTTON3);
    DeleteUserData := Choice = IDYES;
    CleanupTransientUserData := Choice = IDNO;
  end;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ResultCode: Integer;
  AppDir: string;
  Params: string;
begin
  if CurUninstallStep = usUninstall then begin
    SetInstallMaintenance(True);
    AppDir := ExpandConstant('{app}');
    Params := '-NoProfile -ExecutionPolicy Bypass -File "' + AppDir + '\uninstall_current_user.ps1" ' +
      '-InstallationRoot "' + AppDir + '"';
    if DeleteUserData then
      Params := Params + ' -RemoveUserData'
    else if CleanupTransientUserData then
      Params := Params + ' -RemoveTransientUserData';
    Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
      Params, AppDir, SW_HIDE, ewWaitUntilTerminated, ResultCode);
    StopHelperProcess('srf_ime_engine.exe');
    StopHelperProcess('srf_ime_tray.exe');
    StopHelperProcess('KaixinShareX.exe');
    RequestCloseHelperProcess('srf_ime_settings.exe');
    RequestCloseHelperProcess('srf_ime_clipboard.exe');
    RequestCloseHelperProcess('srf_ime_handwrite.exe');
    RequestCloseHelperProcess('srf_ime_ocr.exe');
    RequestCloseHelperProcess('srf_ime_translate.exe');
    StopHelperProcess('srf_ime_clipboard_svc.exe');
  end;
  if CurUninstallStep = usPostUninstall then
    SetInstallMaintenance(False);
end;

procedure DeinitializeUninstall();
begin
  if (not UninstallSilent) and DirExists(ExpandConstant('{app}\runtime')) then
    MsgBox('部分运行时文件可能仍被系统或应用占用，将在相关程序退出或下次重启后自动清理。', mbInformation, MB_OK);
  SetInstallMaintenance(False);
end;
