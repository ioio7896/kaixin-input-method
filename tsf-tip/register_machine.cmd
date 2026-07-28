@echo off
setlocal
set "ROOT=%~dp0"
set "DLL=%ROOT%build\Release\srf_tsf_tip.dll"
if not exist "%DLL%" (
  echo Build Release DLL first.
  exit /b 1
)
echo Registering machine HKLM + TSF ^(run as Administrator^): %DLL%
PowerShell -ExecutionPolicy Bypass -File "%ROOT%invoke_registration.ps1" -DllPath "%DLL%" -Machine
exit /b %ERRORLEVEL%
