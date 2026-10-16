# Differential test: the original Get-MRISourceKind is the oracle.
# This script operates only on ephemeral JSON files. No USB, disk or setup calls.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary = Join-Path $repo 'rust/mri-manifest/target/debug/source_kind.exe'
if (-not [IO.File]::Exists($binary)) { throw 'Rust source-kind binary was not built.' }

$cases = @(
    '{}',
    '{"SourceKind":"Folder"}',
    '{"SourceKind":"ISO"}',
    '{"SourceKind":"ZIP"}',
    '{"SourceKind":"folder"}',
    '{"SourceKind":"zip"}',
    '{"SourceKind":"isO"}',
    '{"sourcekind":"ZIP"}',
    '{"SOURCEKIND":"Folder"}',
    '{"Sourcekind":"iso"}',
    '{"SourceKind":""}',
    '{"SourceKind":"Folder "}',
    '{"SourceKind":" ISO"}',
    '{"SourceKind":"Archive"}',
    '{"SourceKind":null}',
    '{"SourceKind":42}',
    '{"SourceKind":false}',
    '{"SourceKind":[]}',
    '{"SourceKind":{}}',
    '{"OtherSetting":"ZIP"}',
    '{"sourcekind":null}',
    '{"SourceKind":"ZIP","sourcekind":"Folder"}',
    '{"SourceKind":"FOLDER"}',
    '{"SourceKind":"zIp"}'
)

$tempRoot = Join-Path $env:TEMP ('mri-source-kind-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($tempRoot)
$tested = 0
try {
    foreach ($json in $cases) {
        $expectedOk = $false
        $expected = $null
        try {
            # Windows PowerShell 5.1's ordinary PSCustomObject conversion is
            # case-insensitive, like the production application configuration.
            $config = ConvertFrom-Json -InputObject $json -ErrorAction Stop
            $expected = Get-MRISourceKind $config
            $expectedOk = $true
        } catch {
            $expectedOk = $false
        }

        $inputPath = Join-Path $tempRoot 'case.json'
        [IO.File]::WriteAllText($inputPath, $json, [Text.UTF8Encoding]::new($false))
        $raw = & $binary $inputPath
        $exitCode = $LASTEXITCODE
        if (-not $raw) { throw "Rust CLI returned no JSON for: $json" }
        $rust = ($raw | Out-String | ConvertFrom-Json)
        if ($expectedOk) {
            if ($exitCode -ne 0 -or -not $rust.Ok -or $rust.Result -cne $expected) {
                throw "SourceKind mismatch for $json : original='$expected' Rust='$($rust.Result)' exit=$exitCode"
            }
        } else {
            if ($exitCode -eq 0 -or $rust.Ok) {
                throw "SourceKind accepted rejected original input: $json"
            }
        }
        $tested++
    }
    Write-Host "PASS: $tested direct original-PowerShell/Rust SourceKind cases (synthetic, no devices)."
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
