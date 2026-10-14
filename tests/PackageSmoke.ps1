#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-13-2026. Validates the actual downloaded package and its one launcher.
[CmdletBinding()]
param([string]$ArchivePath)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
if (-not $ArchivePath) { $ArchivePath=Join-Path $root 'downloads/MRI-Drive-Maker-v0.3.0.zip' }
$temp=Join-Path ([IO.Path]::GetTempPath()) ('mri-package-'+[guid]::NewGuid().ToString('N'))
Add-Type -AssemblyName System.IO.Compression.FileSystem
try {
    [IO.Compression.ZipFile]::ExtractToDirectory($ArchivePath,$temp)
    $folders=@(Get-ChildItem -LiteralPath $temp -Directory)
    if ($folders.Count -ne 1) { throw 'The ZIP must have one top-level folder.' }
    $package=$folders[0].FullName
    $launchers=@(Get-ChildItem -LiteralPath $package -File | Where-Object { $_.Extension -in @('.cmd','.ps1','.exe') })
    if ($launchers.Count -ne 1 -or $launchers[0].Name -ne 'START-MRI-DRIVE-MAKER.cmd') { throw 'Package has competing top-level launchers.' }
    $manifest=Read-MRIJson (Join-Path $package 'RELEASE.json')
    foreach ($property in $manifest.sha256.PSObject.Properties) {
        $path=Join-MRIPath $package $property.Name
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $property.Value) { throw "Package hash mismatch: $($property.Name)" }
    }
    Write-Host 'PASS Packaged source hashes and the single obvious launcher are correct.'
    if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
        $module=Join-Path $package 'App/MRI.Batch.psm1'
        [IO.File]::AppendAllText($module, "`nfunction Read-Host { param([string]`$Prompt); return 'Q' }`nExport-ModuleMember -Function '*-MRI*','Read-Host'`n")
        $cmd=Join-Path $env:SystemRoot 'System32/cmd.exe'
        $start=[Diagnostics.ProcessStartInfo]::new()
        $start.FileName=$cmd; $start.Arguments='/d /s /c ""'+(Join-Path $package 'START-MRI-DRIVE-MAKER.cmd')+'""'
        $start.WorkingDirectory=$temp; $start.UseShellExecute=$false; $start.CreateNoWindow=$true
        $start.RedirectStandardInput=$true; $start.RedirectStandardOutput=$true; $start.RedirectStandardError=$true
        $process=[Diagnostics.Process]::new(); $process.StartInfo=$start
        try {
            [void]$process.Start()
            $stdout=$process.StandardOutput.ReadToEndAsync(); $stderr=$process.StandardError.ReadToEndAsync()
            $process.StandardInput.Close()
            if (-not $process.WaitForExit(30000)) { & (Join-Path $env:SystemRoot 'System32/taskkill.exe') /PID $process.Id /T /F | Out-Null; throw 'Packaged launcher timed out.' }
            $output=$stdout.GetAwaiter().GetResult()+$stderr.GetAwaiter().GetResult()
            if ($process.ExitCode -ne 0 -or -not $output.Contains('Choose MRI ISO') -or -not $output.Contains('Set simultaneous builds')) { throw "Packaged launcher failed: $output" }
            Write-Host 'PASS The actual packaged top-level launcher opens the ISO menu on Windows PowerShell 5.1.'
        } finally { $process.Dispose() }
    }
} finally { Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue }
exit 0
