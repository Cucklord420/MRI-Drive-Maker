#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-14-2026. Internal worker; open START-MRI-DRIVE-MAKER.cmd.
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$PlanPath)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'The copy worker requires Windows.' }
    Import-Module (Join-Path $PSScriptRoot 'MRI.Batch.psm1') -Force -DisableNameChecking
    Initialize-MRINative
    $plan=Read-MRIJson $PlanPath
    if ([string]$plan.TargetRoot -notmatch '^\\\\\?\\Volume\{[0-9A-Fa-f-]+\}\\$') { throw 'The copy target must be a volume GUID root.' }
    Assert-MRIPlainDirectory $plan.SourceRoot
    Assert-MRIPlainDirectory $plan.TargetRoot
    $files=0
    foreach ($entry in @($plan.Manifest.Entries)) {
        $source=Join-MRIPath $plan.SourceRoot $entry.RelativePath
        $target=Join-MRIPath $plan.TargetRoot $entry.RelativePath
        $item=Get-Item -LiteralPath $source -Force -ErrorAction Stop
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'The source became a link.' }
        if ($entry.IsDirectory) { [MRIBatch.NativeFiles]::CreateDirectory($target) }
        else {
            if ($item.PSIsContainer -or $item.Length -ne $entry.Length) { throw 'The source changed before copying.' }
            # Native Unicode file APIs accept volume GUID names. The source
            # manifest orders directories before their contained files.
            # Existing destination files are never overwritten.
            [MRIBatch.NativeFiles]::Copy($source,$target)
            $files++
        }
    }
    Write-Host "Copied $files files to the pinned volume. Parent verification follows."
    exit 0
} catch { Write-Host $_.Exception.Message; exit 1 }
