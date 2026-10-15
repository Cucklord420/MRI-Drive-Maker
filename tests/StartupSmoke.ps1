#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-13-2026 for MRI Drive Maker. No USB or MRI payload access.
[CmdletBinding()]
param([string]$ResultsPath)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
Initialize-MRINative
$exe=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$windows=[Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
$temp=Join-Path ([IO.Path]::GetTempPath()) ('mri-startup-'+[guid]::NewGuid().ToString('N'))
$app=Join-Path $temp 'MRI Drive Maker & startup'
$working=Join-Path $temp 'unrelated working folder'
[void][IO.Directory]::CreateDirectory($app)
[void][IO.Directory]::CreateDirectory($working)
$results=[Collections.Generic.List[object]]::new()

function Invoke-StartupProcess {
    param([string]$Executable,[string]$Arguments,[string]$InputText='',[int]$TimeoutSeconds=90)
    $start=[Diagnostics.ProcessStartInfo]::new()
    $start.FileName=$Executable; $start.Arguments=$Arguments; $start.WorkingDirectory=$working
    $start.UseShellExecute=$false; $start.CreateNoWindow=$true
    $start.RedirectStandardInput=$true; $start.RedirectStandardOutput=$true; $start.RedirectStandardError=$true
    $process=[Diagnostics.Process]::new(); $process.StartInfo=$start
    try {
        [void]$process.Start()
        $stdout=$process.StandardOutput.ReadToEndAsync(); $stderr=$process.StandardError.ReadToEndAsync()
        $process.StandardInput.Write($InputText); $process.StandardInput.Close()
        if (-not $process.WaitForExit($TimeoutSeconds*1000)) {
            if ($windows) {
                & (Join-Path $env:SystemRoot 'System32/taskkill.exe') /PID $process.Id /T /F | Out-Null
            } else { $process.Kill() }
            throw 'Startup test timed out.'
        }
        $output=$stdout.GetAwaiter().GetResult()+$stderr.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) { throw "Exit $($process.ExitCode): $output" }
        return $output
    } finally { $process.Dispose() }
}
function Invoke-Script {
    param([string]$Path,[string[]]$Arguments=@(),[string]$InputText='')
    $argsLine=(@('-NoLogo','-NoProfile','-ExecutionPolicy','RemoteSigned','-File',$Path)+$Arguments |
        ForEach-Object { [MRIBatch.NativeProcess]::Quote($_) }) -join ' '
    Invoke-StartupProcess $exe $argsLine $InputText
}
function Check-Menu {
    param([string]$Output)
    if (-not $Output.Contains('MRI Drive Maker') -or -not $Output.Contains('Q Exit') -or
        -not $Output.Contains((Join-Path $app 'LICENSE'))) { throw "Menu or script-relative license path missing: $Output" }
}
function Test-Startup {
    param([string]$Name,[scriptblock]$Code)
    $clock=[Diagnostics.Stopwatch]::StartNew()
    try { & $Code; $passed=$true; $errorText=''; Write-Host "PASS $Name" }
    catch { $passed=$false; $errorText=$_.Exception.Message; Write-Host "FAIL ${Name}: $errorText" }
    $clock.Stop()
    $results.Add([pscustomobject]@{Name=$Name; Passed=$passed; Error=$errorText; ElapsedSeconds=[Math]::Round($clock.Elapsed.TotalSeconds,3)})
}
try {
    foreach ($name in @('MRI-Batch.ps1','MRI.Batch.psm1','Copy-MRIFiles.ps1','NativeProcess.cs','mri-config.example.json','LICENSE')) {
        Copy-Item -LiteralPath (Join-Path $root $name) -Destination $app
    }
    Copy-Item -LiteralPath (Join-Path $root 'START-MRI-DRIVE-MAKER.cmd') -Destination $app
    Copy-Item -LiteralPath (Join-Path $root 'tests') -Destination $app -Recurse
    # Console input is a host capability, not part of the path regression.
    # Only the temporary copy exports a fixed Q response so headless Windows
    # runners can exit the real menu without relying on a console keyboard.
    [IO.File]::AppendAllText((Join-Path $app 'MRI.Batch.psm1'), @'

function Read-Host { param([string]$Prompt); return 'Q' }
Export-ModuleMember -Function '*-MRI*','Initialize-MRINative','Read-Host'
'@)
    $entry=Join-Path $app 'MRI-Batch.ps1'
    Test-Startup 'Direct -File launch with default configuration from another folder' {
        Check-Menu (Invoke-Script $entry -InputText "Q`n")
    }
    Test-Startup 'Direct -File launch accepts an explicit configuration path' {
        Check-Menu (Invoke-Script $entry @('-ConfigPath',(Join-Path $app 'custom config.json')) "Q`n")
    }
    Test-Startup 'Direct -File launch accepts an empty configuration path' {
        Check-Menu (Invoke-Script $entry @('-ConfigPath','') "Q`n")
    }
    Test-Startup 'Main entry point runs the full simulated suite via -File -Action SelfTest' {
        $output=Invoke-Script $entry @('-Action','SelfTest')
        $summary=[regex]::Match($output,'(?m)^(\d+)/(\d+) tests passed\.')
        if (-not $summary.Success -or [int]$summary.Groups[1].Value -lt 1 -or
            $summary.Groups[1].Value -ne $summary.Groups[2].Value) { throw "Simulated suite did not pass: $output" }
        Write-Host "  $($summary.Value)"
    }
    Test-Startup 'Validation runner resolves omitted paths after -File parameter binding' {
        $output=Invoke-Script (Join-Path $app 'tests/Invoke-ValidationRuns.ps1') @('-Runs','1')
        if (-not $output.Contains('1/1 runs passed')) { throw "Validation runner did not pass: $output" }
        $reports=@(Get-ChildItem -LiteralPath (Join-Path $app 'validation') -Filter results.json -Recurse)
        if ($reports.Count -ne 1) { throw 'Validation output was not written beneath the extracted application.' }
        $report=Read-MRIJson $reports[0].FullName
        if ($report.passed_runs -ne 1 -or $report.failed_cases -ne 0 -or $report.passed_cases -lt 1) {
            throw 'Validation report did not record a successful suite.'
        }
    }
    if ($windows) {
        Test-Startup 'The sole START-MRI-DRIVE-MAKER.cmd launcher starts PowerShell 5.1 from a folder with spaces and ampersand' {
            $cmd=Join-Path $env:SystemRoot 'System32/cmd.exe'
            $argsLine='/d /s /c ""'+(Join-Path $app 'START-MRI-DRIVE-MAKER.cmd')+'""'
            Check-Menu (Invoke-StartupProcess $cmd $argsLine "Q`r`n")
        }
    }
    if ([IO.Directory]::Exists((Join-Path $app 'Data')) -or [IO.File]::Exists((Join-Path $app 'mri-config.json'))) {
        throw 'Startup tests unexpectedly created user configuration or drive state.'
    }
    $failed=@($results | Where-Object { -not $_.Passed }).Count
    $summary=[ordered]@{
        Date=(Get-Date).ToString('MM-dd-yyyy'); PowerShellVersion=$PSVersionTable.PSVersion.ToString()
        Platform=[Environment]::OSVersion.Platform.ToString(); Passed=$results.Count-$failed; Failed=$failed
        Tests=$results.ToArray(); Scope='Real entry points with menu input mocked in the temporary copy, mocked build suite, validation runner; no USB or MRI payload access.'
    }
    if (-not [string]::IsNullOrWhiteSpace($ResultsPath)) { Write-MRIJson $ResultsPath $summary }
    Write-Host "`n$($results.Count-$failed)/$($results.Count) startup tests passed."
    $summary | ConvertTo-Json -Depth 5 | Write-Host
    if ($failed) { exit 1 }
} finally { Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue }
exit 0
