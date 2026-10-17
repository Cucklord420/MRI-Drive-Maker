# Compare safe System32 path resolution with the original PowerShell function.
# The fixture contains only names and paths; no executable is launched.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary = Join-Path $repo 'rust/mri-manifest/target/debug/windows_executable.exe'
if (-not [IO.File]::Exists($binary)) { throw 'Rust System32 policy binary was not built.' }

$tempRoot = Join-Path $env:TEMP ('mri-system32-policy-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($tempRoot)
$fixture = Join-Path $tempRoot 'input.json'
$validCases = 0
$rejectedCases = 0
try {
    foreach ($name in @(
        'cmd.exe',
        'WindowsPowerShell\v1.0\powershell.exe',
        'Nested\custom-tool.exe'
    )) {
        $payload = [ordered]@{ SystemRoot = [string]$env:SystemRoot; Name = $name }
        [IO.File]::WriteAllText(
            $fixture,
            (ConvertTo-Json -InputObject $payload -Compress -Depth 5),
            [Text.UTF8Encoding]::new($false)
        )
        $raw = & $binary $fixture
        $status = $LASTEXITCODE
        if (-not $raw) { throw "Rust produced no path for valid input '$name'." }
        $rust = ($raw | Out-String | ConvertFrom-Json)
        $original = Get-MRIWindowsExecutable -Name $name
        if ($status -ne 0 -or -not $rust.Ok -or [string]$rust.Result -cne [string]$original) {
            throw "Path mismatch for '$name': original='$original' Rust='$($rust.Result)' Error='$($rust.Error)'"
        }
        $validCases++
    }

    foreach ($name in @(
        '',
        '.',
        '..',
        '../outside.exe',
        '..\outside.exe',
        'Nested\..\outside.exe',
        'D:evil.exe',
        '/root.exe',
        '\absolute.exe',
        '\\server\share.exe',
        'bad:name.exe',
        'COM1.exe',
        'folder/NUL.bat'
    )) {
        $payload = [ordered]@{ SystemRoot = [string]$env:SystemRoot; Name = $name }
        [IO.File]::WriteAllText(
            $fixture,
            (ConvertTo-Json -InputObject $payload -Compress -Depth 5),
            [Text.UTF8Encoding]::new($false)
        )
        $raw = & $binary $fixture
        $status = $LASTEXITCODE
        if (-not $raw) { throw "Rust produced no rejection for unsafe input '$name'." }
        $rust = ($raw | Out-String | ConvertFrom-Json)
        if ($status -eq 0 -or $rust.Ok -or [string]::IsNullOrWhiteSpace([string]$rust.Error)) {
            throw "Rust accepted unsafe executable name '$name'."
        }
        $rejectedCases++
    }

    Write-Host "PASS: $validCases safe paths matched Get-MRIWindowsExecutable; $rejectedCases unsafe names rejected. No executable launched."
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
