@echo off
setlocal
cd /d "%~dp0" || exit /b 1

set "VENV_PYTHON=%~dp0.venv\Scripts\python.exe"
if exist "%VENV_PYTHON%" (
  "%VENV_PYTHON%" "%~dp0build.py" %*
) else (
  python "%~dp0build.py" %*
)

set "BUILD_EXIT_CODE=%ERRORLEVEL%"
if "%BUILD_EXIT_CODE%"=="130" (
  echo.
  echo Build cancelled.
  exit /b 130
)
if not "%BUILD_EXIT_CODE%"=="0" (
  echo.
  echo Packaging failed with exit code %BUILD_EXIT_CODE%.
  if not "%KX_BUILD_NO_PAUSE%"=="1" pause
  exit /b %BUILD_EXIT_CODE%
)

echo.
echo Build command completed successfully. See the output above for artifacts.
if not "%KX_BUILD_NO_PAUSE%"=="1" pause
exit /b 0
