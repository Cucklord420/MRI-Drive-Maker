#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-10-2026 for MRI Drive Maker.
<#
.SYNOPSIS
    Runs tests/SelfTest.ps1 N times with distinguishable per-run logs.
.DESCRIPTION
    Each run executes SelfTest.ps1 in a child PowerShell process with a
    transcript, then writes validation/<utc-timestamp>/run-NNN.txt with a
    header holding the run number, UTC start/finish timestamps, elapsed time,
    exit code, PowerShell version, platform, and source-tree hash. A
    results.json in the same folder summarizes every run with its log
    SHA-256, so repeated runs are independently verifiable instead of
    resting on a single stored report.
    Run output folders are gitignored by default; use git add -f to keep one.
.PARAMETER Runs
    How many consecutive SelfTest executions to perform. Default 10.
.PARAMETER OutputRoot
    Parent folder for the timestamped run folder. Defaults to validation/.
.PARAMETER SelfTest
    Path to the SelfTest.ps1 script. Defaults to tests/SelfTest.ps1.
.EXAMPLE
    .\Invoke-ValidationRuns.ps1 -Runs 100
#>
[CmdletBinding()]
param(
    [int]$Runs = 10,
    [string]$OutputRoot = (Join-Path (Split-Path $PSScriptRoot -Parent) 'validation'),
    [string]$SelfTest = (Join-Path $PSScriptRoot 'SelfTest.ps1')
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Runs -lt 1) { throw 'Runs must be at least 1.' }
if (-not (Test-Path -LiteralPath $SelfTest -PathType Leaf)) { throw "SelfTest not found: $SelfTest" }
$repoRoot = Split-Path $PSScriptRoot -Parent

function Get-SourceTreeHash {
    param([string]$Root, [string]$ExcludeDir)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $files = Get-ChildItem -Path $Root -Include *.ps1, *.psm1, *.cs -Recurse -File |
            Where-Object { $_.FullName -notlike "$ExcludeDir*" } |
            Sort-Object FullName
        $all = [Collections.Generic.List[byte]]::new()
        foreach ($f in $files) { $all.AddRange([IO.File]::ReadAllBytes($f.FullName)) }
        $hash = $sha.ComputeHash($all.ToArray())
        return ([BitConverter]::ToString($hash)).Replace('-', '').ToLowerInvariant()
    } finally { $sha.Dispose() }
}

$stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss\Z')
$runDir = Join-Path $OutputRoot $stamp
[void][IO.Directory]::CreateDirectory($runDir)
$sourceHash = Get-SourceTreeHash -Root $repoRoot -ExcludeDir $OutputRoot
$psVersion = $PSVersionTable.PSVersion.ToString()
$psEdition = $PSVersionTable.PSEdition
$platform = [Environment]::OSVersion.Platform.ToString()
$exe = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName

# Wrapper avoids nested quoting: the child runs the wrapper via -File, and the
# wrapper starts a transcript (captures Write-Host on PS 5.1 too) around SelfTest.
$wrapper = Join-Path $runDir '_wrapper.ps1'
[IO.File]::WriteAllText($wrapper, @'
param([string]$TranscriptPath, [string]$TestScriptPath)
Start-Transcript -Path $TranscriptPath -Force | Out-Null
& $TestScriptPath
'@)

$results = [Collections.Generic.List[object]]::new()
$suiteStartedUtc = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
$suiteClock = [Diagnostics.Stopwatch]::StartNew()
$failedRuns = 0
for ($i = 1; $i -le $Runs; $i++) {
    $tag = $i.ToString('D3')
    $transcript = Join-Path $runDir "run-$tag.transcript.txt"
    $stdoutLog = Join-Path $runDir "run-$tag.stdout.txt"
    $startedUtc = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    $clock = [Diagnostics.Stopwatch]::StartNew()
    & $exe -NoProfile -NonInteractive -File $wrapper -TranscriptPath $transcript -TestScriptPath $SelfTest > $stdoutLog 2>&1
    $exitCode = $LASTEXITCODE
    $clock.Stop()
    $finishedUtc = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    $body = ''
    if ((Test-Path -LiteralPath $transcript) -and ((Get-Item -LiteralPath $transcript).Length -gt 0)) {
        $body = [IO.File]::ReadAllText($transcript)
    } elseif (Test-Path -LiteralPath $stdoutLog) {
        $body = [IO.File]::ReadAllText($stdoutLog)
    }
    $passCount = ([regex]::Matches($body, '(?m)^PASS ')).Count
    $failCount = ([regex]::Matches($body, '(?m)^FAIL ')).Count
    $header = @(
        '# MRI Drive Maker validation run',
        "# Run: $i of $Runs",
        "# Started (UTC): $startedUtc",
        "# Finished (UTC): $finishedUtc",
        ('# Elapsed seconds: {0:N3}' -f $clock.Elapsed.TotalSeconds),
        "# Exit code: $exitCode",
        "# PowerShell: $psVersion ($psEdition)",
        "# Platform: $platform",
        "# Source tree SHA-256: $sourceHash",
        '# Runner: tests/Invoke-ValidationRuns.ps1',
        ''
    ) -join [Environment]::NewLine
    $logPath = Join-Path $runDir "run-$tag.txt"
    [IO.File]::WriteAllText($logPath, $header + [Environment]::NewLine + $body)
    Remove-Item -LiteralPath $transcript -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stdoutLog -Force -ErrorAction SilentlyContinue
    $logHash = (Get-FileHash -LiteralPath $logPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $ok = ($exitCode -eq 0) -and ($failCount -eq 0) -and ($passCount -gt 0)
    if (-not $ok) { $failedRuns++ }
    $results.Add([pscustomobject]@{
        run            = $i
        passed         = $ok
        exit_code      = $exitCode
        passed_cases   = $passCount
        failed_cases   = $failCount
        elapsed_seconds = [math]::Round($clock.Elapsed.TotalSeconds, 3)
        log_sha256     = $logHash
        started_utc    = $startedUtc
        finished_utc   = $finishedUtc
    })
    $verdict = 'PASS'; if (-not $ok) { $verdict = 'FAIL' }
    Write-Host ("Run {0}/{1}: {2} ({3:N1}s, {4} passed, {5} failed)" -f $i, $Runs, $verdict, $clock.Elapsed.TotalSeconds, $passCount, $failCount)
}
$suiteClock.Stop()
Remove-Item -LiteralPath $wrapper -Force -ErrorAction SilentlyContinue

$passedRuns = (@($results | Where-Object { $_.passed })).Count
$passedCases = ($results | Measure-Object -Property passed_cases -Sum).Sum
$failedCases = ($results | Measure-Object -Property failed_cases -Sum).Sum
$summary = [ordered]@{
    date            = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd')
    time_zone       = 'UTC'
    started_time    = $suiteStartedUtc
    finished_time   = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    source_sha256   = $sourceHash
    power_shell_version = "$psVersion ($psEdition)"
    platform        = $platform
    completed_runs  = $Runs
    passed_runs     = $passedRuns
    passed_cases    = $passedCases
    failed_cases    = $failedCases
    elapsed_seconds = [math]::Round($suiteClock.Elapsed.TotalSeconds, 3)
    runs            = $results.ToArray()
    limitations     = @(
        'Storage, process, and volume APIs are mocked inside SelfTest by design; see tests/SelfTest.ps1 header.',
        'Windows native execution is not covered here; run tests/WindowsProcessSmoke.ps1 on Windows 10/11.'
    )
    runner          = 'tests/Invoke-ValidationRuns.ps1'
}
$jsonPath = Join-Path $runDir 'results.json'
$summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $jsonPath -Encoding utf8

Write-Host ''
Write-Host ("{0}/{1} runs passed, {2}/{3} cases passed in {4:N1}s. Logs: {5}" -f $passedRuns, $Runs, $passedCases, ($passedCases + $failedCases), $suiteClock.Elapsed.TotalSeconds, $runDir)
if ($failedRuns -gt 0) { exit 1 }
