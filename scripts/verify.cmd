@echo off
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0verify.ps1" %*
exit /b %ERRORLEVEL%
