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
        if ($entry.IsDirectory) { [void][IO.Directory]::CreateDirectory($target) }
        else {
            if ($item.PSIsContainer -or $item.Length -ne $entry.Length) { throw 'The source changed before copying.' }
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
            # File.Copy uses the Windows file API, which accepts volume GUID
            # names. No drive letter is used for destination writes.
            [IO.File]::Copy($source,$target,$false)
            $files++
        }
    }
    foreach ($entry in @($plan.Manifest.Entries | Where-Object { $_.IsDirectory } | Sort-Object RelativePath -Descending)) {
        $source=Join-MRIPath $plan.SourceRoot $entry.RelativePath
        $target=Join-MRIPath $plan.TargetRoot $entry.RelativePath
        [IO.Directory]::SetLastWriteTimeUtc($target,[IO.Directory]::GetLastWriteTimeUtc($source))
        [IO.File]::SetAttributes($target,[IO.File]::GetAttributes($source))
    }
    Write-Host "Copied $files files to the pinned volume. Parent verification follows."
    exit 0
} catch { Write-Host $_.Exception.Message; exit 1 }
