@echo off
setlocal
cd /d "%~dp0"

set "VENV_PYTHON=%~dp0.venv\Scripts\python.exe"
if exist "%VENV_PYTHON%" (
  "%VENV_PYTHON%" "%~dp0build.py" --package-variants ime,ocr --no-verify --no-smoke
) else (
  python "%~dp0build.py" --package-variants ime,ocr --no-verify --no-smoke
)

set "BUILD_EXIT_CODE=%ERRORLEVEL%"
if not "%BUILD_EXIT_CODE%"=="0" (
  echo.
  echo Packaging failed with exit code %BUILD_EXIT_CODE%.
  pause
  exit /b %BUILD_EXIT_CODE%
)

echo.
echo Both installers were created in the dist directory.
pause
exit /b 0
