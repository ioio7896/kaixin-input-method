@echo off
setlocal
set "ROOT=%~dp0"
set "DLL=%ROOT%build\Release\srf_tsf_tip.dll"
if not exist "%DLL%" (
  echo Build Release DLL first: cmake -B build -G "Visual Studio 18 2026" -A x64 ^&^& cmake --build build --config Release
  echo Expected: %DLL%
  exit /b 1
)
echo Registering: %DLL%
PowerShell -ExecutionPolicy Bypass -File "%ROOT%invoke_registration.ps1" -DllPath "%DLL%"
exit /b %ERRORLEVEL%
