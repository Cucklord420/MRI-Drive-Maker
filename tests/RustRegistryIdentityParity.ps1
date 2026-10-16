# SPDX-License-Identifier: GPL-3.0-only
# 240 read-only registry identity decisions against the actual PowerShell module.
# Fixtures are synthetic JSON. No physical USB enumeration, writes, or erasure.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary = Join-Path $repo 'rust/mri-manifest/target/debug/registry_identity.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Rust registry binary was not built' }
$temp = Join-Path $env:TEMP ('mri-registry-parity-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($temp) | Out-Null
$cases = 0
try {
    for ($i = 0; $i -lt 20; $i++) {
        for ($variant = 0; $variant -lt 12; $variant++) {
            $serial = ('ABC-{0:D3}' -f $i)
            $size = [long](16000000000 + $i)
            $pnp = 'USB\VID'
            $model = 'TEST'
            $key = Get-MRIDriveKey $serial $pnp $model $size
            $record = [pscustomobject]@{
                Key = $key; Serial = $serial; PnpId = $pnp
                Model = $model; Size = $size
                Label = ('MRI_{0:D2}' -f $i); AllowErase = $true
            }
            $records = @($record)
            switch ($variant) {
                0 {}
                1 { $record.Key = $key.ToLowerInvariant() }
                2 { $record.Label = $record.Label.ToLowerInvariant() }
                3 { $record.Label = 'MRI_AA' }
                4 { $record.Key = 'invalid' }
                5 { $record.Serial = 'UNKNOWN' }
                6 {
                    $other = [pscustomobject]@{
                        Key = $key; Serial = $serial; PnpId = $pnp
                        Model = $model; Size = $size
                        Label = $record.Label.ToLowerInvariant(); AllowErase = $true
                    }
                    $records = @($record, $other)
                }
                7 { $record.AllowErase = 'true' }
                8 { $record.PSObject.Properties.Remove('Key') }
                9 { $record.PnpId = '  ' }
                10 { $record.Serial = '000' }
                11 {
                    $other = [pscustomobject]@{
                        Key = $key; Serial = $serial; PnpId = $pnp
                        Model = $model; Size = $size
                        Label = ('MRI_{0:D2}' -f ($i + 40)); AllowErase = $true
                    }
                    $records = @($record, $other)
                }
            }
            $fixture = Join-Path $temp 'case.json'
            [IO.File]::WriteAllText($fixture,
                (ConvertTo-Json -InputObject ([pscustomobject]@{Records=$records}) -Depth 10 -Compress),
                [Text.UTF8Encoding]::new($false))
            $output = & $binary $fixture
            $status = $LASTEXITCODE
            if (-not $output) { throw "Missing Rust output for case $i/$variant" }
            $rust = $output | ConvertFrom-Json
            $powershellError = $null
            try { Assert-MRIRegistry -Records $records }
            catch { $powershellError = $_.Exception.Message }
            if ($null -eq $powershellError) {
                if ($status -ne 0 -or -not $rust.Ok -or $null -ne $rust.Error) {
                    throw "Mismatch accepted case $i/$variant Rust=$($rust.Error)"
                }
            } else {
                if ($status -eq 0 -or $rust.Ok -or $rust.Error -cne $powershellError) {
                    throw "Mismatch rejected case $i/$variant PS=$powershellError Rust=$($rust.Error)"
                }
            }
            $cases++
        }
    }
    if ($cases -ne 240) { throw "Incomplete registry parity run ($cases/240)" }
    Write-Host "PASS: $cases actual PowerShell/Rust registry identity decisions."
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
