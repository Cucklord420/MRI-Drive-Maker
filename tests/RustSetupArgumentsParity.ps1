# Direct PowerShell/Rust parity for Get-MRISetupArguments.
# Synthetic, read-only: no disks, USB inventory, mounts, or batch execution.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary = Join-Path $repo 'rust/mri-manifest/target/debug/setup_arguments.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Rust setup arguments binary not built' }
$temp = Join-Path $env:TEMP ('mri-setup-parity-' + [guid]::NewGuid().ToString('N') + '.json')
$cases = 0
try {
    for ($n = 0; $n -lt 120; $n++) {
        $letter = [string]([char]([int][char]'E' + ($n % 8)))
        $number = $n + 7
        $label = 'MRI_{0:D2}' -f (($n % 99) + 1)
        $setup = if (($n % 2) -eq 0) { 'setup.cmd' } else { 'tools\boot.bat' }
        $arguments = switch ($n % 5) {
            0 { @() }
            1 { @('--disk={DiskNumber}') }
            2 { @('--root={DriveRoot}', '--label={Label}') }
            3 { @('mode=USB', '--drive={DriveLetter}') }
            4 { @('safe', 'fixed.file', '--number={DiskNumber}') }
        }
        $argsArray = @($arguments)
        $config = [pscustomobject]@{ SetupBatch = $setup; SetupArguments = $argsArray }
        $record = [pscustomobject]@{ Label = $label }
        $target = [pscustomobject]@{ Letter = $letter; Number = $number }
        $expected = [string](Get-MRISetupArguments $config $record $target)

        $fixture = [ordered]@{
            Letter = $letter
            Number = $number
            Label = $label
            SetupBatch = $setup
            SetupArguments = $argsArray
        }
        $json = ConvertTo-Json -InputObject $fixture -Depth 8 -Compress
        [IO.File]::WriteAllText($temp, $json, [Text.UTF8Encoding]::new($false))
        $raw = & $binary $temp
        $exit = $LASTEXITCODE
        if ($exit -ne 0 -or -not $raw) {
            throw "Rust setup argument command failed for case $n (exit=$exit): $raw"
        }
        $actual = $raw | Out-String | ConvertFrom-Json
        if (-not $actual.Ok -or $actual.Result -cne $expected) {
            throw "Setup argument mismatch for case $n; PowerShell=$expected Rust=$($actual.Result)"
        }
        $cases++
    }
    Write-Host "PASS: $cases original PowerShell / Rust CMD argument parity cases (synthetic, read-only)."
} finally {
    Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
}
