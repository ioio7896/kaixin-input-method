@echo off
setlocal
set "ROOT=%~dp0"
set "DLL=%ROOT%build\Release\srf_tsf_tip.dll"
if not exist "%DLL%" (
  echo DLL not found: %DLL%
  exit /b 1
)
echo Unregistering machine scope ^(run as Administrator^): %DLL%
PowerShell -ExecutionPolicy Bypass -File "%ROOT%invoke_registration.ps1" -DllPath "%DLL%" -Machine -Unregister
exit /b %ERRORLEVEL%
