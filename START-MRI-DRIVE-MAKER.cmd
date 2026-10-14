@echo off
rem SPDX-License-Identifier: GPL-3.0-only
rem Current launcher. Added 10-13-2026. See LICENSE and NOTICE.md.
setlocal
set "MRI_SCRIPT=%~dp0App\MRI-Batch.ps1"
if not exist "%MRI_SCRIPT%" set "MRI_SCRIPT=%~dp0MRI-Batch.ps1"
if not exist "%MRI_SCRIPT%" (
  echo Extract the complete ZIP before opening START-MRI-DRIVE-MAKER.cmd.
  pause
  exit /b 1
)
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy RemoteSigned -File "%MRI_SCRIPT%" -ConfigPath "%~dp0mri-config.json" -DataPath "%~dp0Data" %*
set "MRI_EXIT=%ERRORLEVEL%"
if not "%MRI_EXIT%"=="0" (
  echo.
  echo MRI Drive Maker stopped with an error. Read the message above.
  pause
)
exit /b %MRI_EXIT%
