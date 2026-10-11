#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-10-2026 for MRI Drive Maker.
# GPL-3.0-only. No USB access, formatting, copying, or MRI files required.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'This smoke test requires Windows.' }
Import-Module (Join-Path (Split-Path $PSScriptRoot -Parent) 'MRI.Batch.psm1') -Force -DisableNameChecking
Initialize-MRINative
$dir=Join-Path ([IO.Path]::GetTempPath()) ('mri-process-smoke-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($dir)
$exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$runner=$null
try {
    # Parent exits promptly; detached child must still be tracked until its writes finish.
    $child=Join-Path $dir 'child.ps1'; $parent=Join-Path $dir 'parent.ps1'; $marker=Join-Path $dir 'child-finished'
    [IO.File]::WriteAllText($child, "Start-Sleep -Seconds 2`n[IO.File]::WriteAllText((Join-Path `$PSScriptRoot 'child-finished'),'done')")
    [IO.File]::WriteAllText($parent, 'Start-Process -FilePath (Join-Path $PSHOME ''powershell.exe'') -ArgumentList (''-NoProfile -NonInteractive -File "''+(Join-Path $PSScriptRoot ''child.ps1'')+''"'') -WindowStyle Hidden | Out-Null')
    $clock=[Diagnostics.Stopwatch]::StartNew()
    $argsLine='-NoProfile -NonInteractive -File '+[MRIBatch.NativeProcess]::Quote($parent)
    $runner=[MRIBatch.NativeProcess]::new($exe,$argsLine,$dir,(Join-Path $dir 'tree.log'),$false)
    while ($runner.ActiveProcesses -gt 0) { if ($clock.Elapsed.TotalSeconds -gt 20) { throw 'Process-tree smoke test timed out.' }; Start-Sleep -Milliseconds 50 }
    if ($runner.ExitCode -ne 0 -or -not [IO.File]::Exists($marker) -or $clock.Elapsed.TotalSeconds -lt 2) { throw 'Parent exited before child verification failed.' }
    $runner.Dispose(); $runner=$null
    Write-Host 'PASS: parent exit did not cause premature completion; its child finished first.'
    # Closing a job must kill a still-running descendant, including after its parent exits.
    $childPid=Join-Path $dir 'child-pid'
    [IO.File]::WriteAllText($child, "[IO.File]::WriteAllText((Join-Path `$PSScriptRoot 'child-pid'),[string]`$PID)`nStart-Sleep -Seconds 30")
    $runner=[MRIBatch.NativeProcess]::new($exe,$argsLine,$dir,(Join-Path $dir 'kill.log'),$false)
    $clock.Restart()
    while (-not [IO.File]::Exists($childPid)) { if ($clock.Elapsed.TotalSeconds -gt 10) { throw 'Child did not start.' }; Start-Sleep -Milliseconds 50 }
    $pidToCheck=[int][IO.File]::ReadAllText($childPid)
    $runner.Dispose(); $runner=$null
    $clock.Restart()
    while ($null -ne (Get-Process -Id $pidToCheck -ErrorAction SilentlyContinue)) {
        if ($clock.Elapsed.TotalSeconds -gt 5) { throw 'Job close did not terminate its child process.' }; Start-Sleep -Milliseconds 50
    }
    Write-Host 'PASS: cancellation terminated the remaining child process.'
} finally {
    if ($runner) { $runner.Dispose() }
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
