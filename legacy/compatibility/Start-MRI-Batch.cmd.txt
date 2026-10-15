@echo off
rem SPDX-License-Identifier: GPL-3.0-only
rem Added 10-10-2026 for MRI Drive Maker. See LICENSE and NOTICE.md.
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy RemoteSigned -File "%~dp0MRI-Batch.ps1"
if errorlevel 1 (
  echo.
  echo MRI Batch Creator stopped with an error. Read the message above.
  pause
)
endlocal
