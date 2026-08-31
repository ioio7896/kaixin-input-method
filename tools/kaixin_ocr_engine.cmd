@echo off
setlocal
set "ROOT=%~dp0.."
set "RUNTIME_PY=%ROOT%\.python-runtime\python.exe"
if exist "%RUNTIME_PY%" (
  set "PYTHON_EXE=%RUNTIME_PY%"
  set "PYTHONPATH=%ROOT%\.python-packages"
) else (
  echo Bundled RapidOCR Python not found: "%RUNTIME_PY%" 1>&2
  exit /b 1
)
set "PYTHONUTF8=1"
set "PYTHONIOENCODING=utf-8"
set "PYTHONDONTWRITEBYTECODE=1"
"%PYTHON_EXE%" "%~dp0kaixin_ocr_engine.py" %* --rapidocr-root "%ROOT%\RapidOCR-3.9.0"
