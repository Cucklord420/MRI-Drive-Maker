#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-14-2026. Internal worker; open START-MRI-DRIVE-MAKER.cmd.
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$PlanPath,
      [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedHash)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'The copy worker requires Windows.' }
    $files=0
    $worker=Join-Path $PSScriptRoot 'mri-copy.exe'
    & $worker --copy $PlanPath $ExpectedHash
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host 'Copied files after handle-anchored source identity preflight. Parent verification follows.'
    exit 0
} catch { Write-Host $_.Exception.Message; exit 1 }
