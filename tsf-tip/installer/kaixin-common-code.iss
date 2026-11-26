var
  ComponentChecksReady: Boolean;
  MaintenanceStarted: Boolean;
  InstallRapidOcrPayloadComponent: Boolean;

function CurrentInstallLogContains(const FileName: string; const Needle: string): Boolean;
var
  Text: AnsiString;
  Marker: AnsiString;
  P: Integer;
begin
  Result := False;
  Marker := 'event=install_session_start';
  if not LoadStringFromFile(FileName, Text) then
    exit;
  P := Pos(Marker, Text);
  if P = 0 then
    exit;
  { Discard all previous sessions, including a marker left before log cleanup. }
  while P > 0 do begin
    Delete(Text, 1, P + Length(Marker) - 1);
    P := Pos(Marker, Text);
  end;
  Result := Pos(Needle, Text) > 0;
end;

function InstalledComponentDiffers(const ComponentName: string; const ExpectedId: string): Boolean;
var
  InstalledId: string;
begin
  if ExpectedId = '' then
  begin
    Result := True;
    exit;
  end;
  if ExpectedId = 'absent' then
  begin
    Result := False;
    exit;
  end;
  InstalledId := GetIniString(
    'components', ComponentName, '', ExpandConstant('{app}\component_manifest.ini'));
  Result := CompareText(InstalledId, ExpectedId) <> 0;
end;

procedure InitializeComponentChecks();
begin
  if ComponentChecksReady then
    exit;
  InstallRapidOcrPayloadComponent := InstalledComponentDiffers('rapidocr_payload', '{#KXComponentRapidOcrPayload}');
  ComponentChecksReady := True;
end;

function QueryInstallDisplayVersion(RootKey: Integer; const UninstallKey: string; var Version: string; var InstallLocation: string): Boolean;
begin
  Version := '';
  InstallLocation := '';
  Result := RegQueryStringValue(RootKey, UninstallKey, 'DisplayVersion', Version);
  RegQueryStringValue(RootKey, UninstallKey, 'InstallLocation', InstallLocation);
end;

function QueryInstallByDisplayName(RootKey: Integer; var Version: string; var InstallLocation: string): Boolean;
begin
  Version := '';
  InstallLocation := '';
  Result := QueryInstallDisplayVersion(
    RootKey,
    'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#KXBaseAppName}',
    Version,
    InstallLocation
  );
end;

function QueryMachineInstall(var Version: string; var InstallLocation: string): Boolean;
begin
  Result := QueryInstallDisplayVersion(
    HKLM64,
    'Software\Microsoft\Windows\CurrentVersion\Uninstall\{2B91F956-7D55-4D85-B3A6-456A4A5DBB84}_is1',
    Version,
    InstallLocation
  );
  if not Result then
    Result := QueryInstallByDisplayName(HKLM64, Version, InstallLocation);
  if not Result then
    Result := QueryInstallDisplayVersion(HKLM, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{2B91F956-7D55-4D85-B3A6-456A4A5DBB84}_is1', Version, InstallLocation);
  if not Result then
    Result := QueryInstallByDisplayName(HKLM, Version, InstallLocation);
end;

function QueryUserInstall(var Version: string; var InstallLocation: string): Boolean;
begin
  Result := QueryInstallDisplayVersion(
    HKCU,
    'Software\Microsoft\Windows\CurrentVersion\Uninstall\{2B91F956-7D55-4D85-B3A6-456A4A5DBB84}-User_is1',
    Version,
    InstallLocation
  );
  if not Result then
    Result := QueryInstallByDisplayName(HKCU, Version, InstallLocation);
end;

function QueryCurrentScopeInstall(var Version: string; var InstallLocation: string): Boolean;
begin
#if KXMachineInstall
  Result := QueryMachineInstall(Version, InstallLocation);
#else
  Result := QueryUserInstall(Version, InstallLocation);
#endif
end;

function QueryOtherScopeInstall(var Version: string; var InstallLocation: string): Boolean;
begin
#if KXMachineInstall
  Result := QueryUserInstall(Version, InstallLocation);
#else
  Result := QueryMachineInstall(Version, InstallLocation);
#endif
end;

function TakeVersionPart(var Version: string): string;
var
  Dot: Integer;
begin
  Dot := Pos('.', Version);
  if Dot > 0 then
  begin
    Result := Copy(Version, 1, Dot - 1);
    Delete(Version, 1, Dot);
  end
  else
  begin
    Result := Version;
    Version := '';
  end;
end;

function LeadingVersionNumber(const Part: string): Integer;
var
  I: Integer;
  Digits: string;
begin
  Digits := '';
  for I := 1 to Length(Part) do
  begin
    if (Part[I] >= '0') and (Part[I] <= '9') then
      Digits := Digits + Part[I]
    else
      break;
  end;
  Result := StrToIntDef(Digits, 0);
end;

function CompareVersionTexts(LeftVersion: string; RightVersion: string): Integer;
var
  I: Integer;
  LeftPart: Integer;
  RightPart: Integer;
begin
  Result := 0;
  for I := 1 to 4 do
  begin
    LeftPart := LeadingVersionNumber(TakeVersionPart(LeftVersion));
    RightPart := LeadingVersionNumber(TakeVersionPart(RightVersion));
    if LeftPart > RightPart then
    begin
      Result := 1;
      exit;
    end;
    if LeftPart < RightPart then
    begin
      Result := -1;
      exit;
    end;
  end;
end;

procedure RemoveCurrentUserData();
var
  AppDir, SetupExe, RunnerExe, Params: string;
  ResultCode: Integer;
begin
  AppDir := ExpandConstant('{app}');
  SetupExe := AddBackslash(AppDir) + 'srf_ime_setup.exe';
  RunnerExe := ExpandConstant('{tmp}\kaixin-setup-uninstall.exe');
  if not FileCopy(SetupExe, RunnerExe, False) then
    RunnerExe := SetupExe;
  Params := 'uninstall-user --root "' + AppDir + '"';
#if KXMachineInstall
  Params := Params + ' --from-machine-uninstaller --skip-registration';
#endif
  if not ExecAndLogOutput(RunnerExe,
    Params, AppDir, SW_HIDE, ewWaitUntilTerminated, ResultCode, nil) or
    (ResultCode <> 0) then begin
    Log('Current-user uninstall cleanup failed. Code=' + IntToStr(ResultCode));
    if not UninstallSilent then
      MsgBox('未能完成用户数据清理或恢复微软拼音，请重试卸载。', mbError, MB_OK);
    Abort;
  end;
end;

function InstallScopeName(): string;
begin
#if KXMachineInstall
  Result := '管理员版';
#else
  Result := '用户版';
#endif
end;

function OtherInstallScopeName(): string;
begin
#if KXMachineInstall
  Result := '用户版';
#else
  Result := '管理员版';
#endif
end;

function InitializeSetupShared(): Boolean;
var
  CurrentVersion: string;
  CurrentLocation: string;
  OtherVersion: string;
  OtherLocation: string;
  VersionCompare: Integer;
  MessageText: string;
begin
  Result := True;

  if WizardSilent and (ExpandConstant('{param:ACCEPTTERMS|0}') <> '1') then
  begin
    Log('Silent installation requires /ACCEPTTERMS=1 to accept the installation terms.');
    Result := False;
    exit;
  end;

  if QueryOtherScopeInstall(OtherVersion, OtherLocation) then
  begin
    MessageText :=
      '检测到已安装开心输入法' + OtherInstallScopeName() + '。' + #13#10 + #13#10 +
      '版本：' + OtherVersion + #13#10 +
      '位置：' + OtherLocation + #13#10 + #13#10 +
      '为避免 TSF 注册表和语言列表混用，请先卸载另一套安装包，再运行当前' + InstallScopeName() + '安装包。';
    Log('Found conflicting Kaixin IME install in the other scope. Version=' + OtherVersion + ' Location=' + OtherLocation);
    if not WizardSilent then
      MsgBox(MessageText, mbError, MB_OK);
    Result := False;
    exit;
  end;

  if QueryCurrentScopeInstall(CurrentVersion, CurrentLocation) then
  begin
    VersionCompare := CompareVersionTexts('{#KXAppVersion}', CurrentVersion);
    if VersionCompare < 0 then
    begin
      Log('Detected downgrade install; continuing directly. Installed=' + CurrentVersion + ' New={#KXAppVersion}');
    end;
  end;
end;

function InitializeUninstall(): Boolean;
begin
  Result := not UninstallSilent or (ExpandConstant('{param:CONFIRMDATAREMOVAL|0}') = '1');
  if not Result then
    Log('Silent uninstall requires /CONFIRMDATAREMOVAL=1 to confirm deletion of current-user data.');
end;

function WindowsGetTickCount(): LongWord;
  external 'GetTickCount@kernel32.dll stdcall';

#if KXMachineInstall
procedure SetDesktopMaintenanceValue(const Name, Data: string);
var
  Params: string;
  ResultCode: Integer;
begin
  if Data = '' then
    Params := 'delete "HKCU\' + KaixinStateRegKey + '" /v ' + Name + ' /f'
  else
    Params := 'add "HKCU\' + KaixinStateRegKey + '" /v ' + Name + ' /t REG_DWORD /d ' + Data + ' /f';
  if not ExecAsOriginalUser(ExpandConstant('{sys}\reg.exe'), Params, '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
    Log('Could not update the desktop maintenance flag ' + Name + '. Code=' + IntToStr(ResultCode));
end;
#endif

procedure SetInstallMaintenance(Enabled: Boolean);
begin
  if Enabled = MaintenanceStarted then exit;
  MaintenanceStarted := Enabled;
#if KXMachineInstall
  if Enabled then begin
    SetDesktopMaintenanceValue(KaixinInstallMaintenanceTickValue, '0x' + Format('%.8x', [WindowsGetTickCount()]));
    SetDesktopMaintenanceValue(KaixinInstallMaintenanceValue, '1');
  end else begin
    SetDesktopMaintenanceValue(KaixinInstallMaintenanceValue, '');
    SetDesktopMaintenanceValue(KaixinInstallMaintenanceTickValue, '');
  end;
#else
  if Enabled then begin
    RegWriteDWordValue(HKCU, KaixinStateRegKey, KaixinInstallMaintenanceTickValue,
      WindowsGetTickCount());
    RegWriteDWordValue(HKCU, KaixinStateRegKey, KaixinInstallMaintenanceValue, 1);
  end else begin
    RegDeleteValue(HKCU, KaixinStateRegKey, KaixinInstallMaintenanceValue);
    RegDeleteValue(HKCU, KaixinStateRegKey, KaixinInstallMaintenanceTickValue);
  end;
#endif
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  HelperExe: string;
  ResultCode: Integer;
begin
  Result := '';
  if not FileExists(ExpandConstant('{app}\srf_ime_setup.exe')) then exit;
  SetInstallMaintenance(True);
  try
    ExtractTemporaryFile('srf_ime_setup.exe');
    HelperExe := ExpandConstant('{tmp}\srf_ime_setup.exe');
    if not Exec(HelperExe, 'stop-processes', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      Log('Could not start srf_ime_setup stop-processes: ' + SysErrorMessage(ResultCode));
  except
    Log('ExtractTemporaryFile(srf_ime_setup.exe) failed: ' + GetExceptionMessage);
  end;
end;

function RunSetupHelper(const Params, Status: string; AsOriginalUser: Boolean): Integer;
var
  Launched: Boolean;
  SetupExe, AppDir: string;
begin
  WizardForm.StatusLabel.Caption := Status;
  WizardForm.FilenameLabel.Caption := '';
  WizardForm.StatusLabel.Update;
  AppDir := ExpandConstant('{app}');
  SetupExe := AddBackslash(AppDir) + 'srf_ime_setup.exe';
  if AsOriginalUser then
    Launched := ExecAsOriginalUser(SetupExe, Params, AppDir, SW_HIDE, ewWaitUntilTerminated, Result)
  else
    Launched := Exec(SetupExe, Params, AppDir, SW_HIDE, ewWaitUntilTerminated, Result);
  if not Launched then
    RaiseException(Status + #13#10 + SysErrorMessage(Result));
  { Exit 10 is the desktop health report from --report-setup-status. }
  if (Result <> 0) and not ((Result = 10) and (Pos('--report-setup-status', Params) > 0)) then
    RaiseException(Status + #13#10 + '退出码 ' + IntToStr(Result) + #13#10 +
      '请查看安装目录的 install_machine.log 或桌面用户 %LOCALAPPDATA%\kaixin\install_user.log。');
end;

procedure LayoutFinishedPage(const Summary: string);
var
  RunListHeight: Integer;
  RunListTop: Integer;
  AvailableLabelHeight: Integer;
begin
  WizardForm.FinishedLabel.Caption := Summary;
  if WizardForm.RunList.Visible then begin
    RunListHeight := ScaleY(28);
    WizardForm.FinishedLabel.AdjustHeight;
    RunListTop := WizardForm.FinishedLabel.Top + WizardForm.FinishedLabel.Height + ScaleY(8);
    if RunListTop + RunListHeight > WizardForm.FinishedPage.Height - ScaleY(8) then begin
      RunListTop := WizardForm.FinishedPage.Height - RunListHeight - ScaleY(8);
      AvailableLabelHeight := RunListTop - WizardForm.FinishedLabel.Top - ScaleY(8);
      if AvailableLabelHeight > 0 then
        WizardForm.FinishedLabel.Height := AvailableLabelHeight;
    end;
    WizardForm.RunList.Top := RunListTop;
    WizardForm.RunList.Height := RunListHeight;
  end;
end;
