var
  ComponentChecksReady: Boolean;
  InstallPythonRuntimeComponent: Boolean;
  InstallRapidOcrPackagesComponent: Boolean;
  InstallRapidOcrPayloadComponent: Boolean;

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
  InstallPythonRuntimeComponent := InstalledComponentDiffers('python_runtime', '{#KXComponentPythonRuntime}');
  InstallRapidOcrPackagesComponent := InstalledComponentDiffers('rapidocr_packages', '{#KXComponentRapidOcrPackages}');
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

function QueryMachineInstall(var Version: string; var InstallLocation: string): Boolean;
begin
  Result := QueryInstallDisplayVersion(HKLM64, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{2B91F956-7D55-4D85-B3A6-456A4A5DBB84}_is1', Version, InstallLocation);
  if not Result then
    Result := QueryInstallDisplayVersion(HKLM, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{2B91F956-7D55-4D85-B3A6-456A4A5DBB84}_is1', Version, InstallLocation);
end;

function QueryUserInstall(var Version: string; var InstallLocation: string): Boolean;
begin
  Result := QueryInstallDisplayVersion(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{2B91F956-7D55-4D85-B3A6-456A4A5DBB84}-User_is1', Version, InstallLocation);
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

function UninstallParamEnabled(const ParamName: string): Boolean;
var
  Value: string;
begin
  Value := Lowercase(ExpandConstant('{param:' + ParamName + '|}'));
  Result := (Value = '1') or (Value = 'yes') or (Value = 'true') or (Value = 'on');
end;

function CommandLineRequestsRemoveUserData(): Boolean;
begin
  Result :=
    UninstallParamEnabled('RemoveUserData') or
    UninstallParamEnabled('DeleteUserData') or
    UninstallParamEnabled('KaixinRemoveUserData');
end;

function CommandLineRequestsRemoveTransientUserData(): Boolean;
begin
  Result :=
    UninstallParamEnabled('RemoveTransientUserData') or
    UninstallParamEnabled('CleanupTransientUserData') or
    UninstallParamEnabled('KaixinRemoveTransientUserData');
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
