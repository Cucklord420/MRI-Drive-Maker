# Original PowerShell Update-MRIArrivals versus isolated Rust pure-data policy.
# All identifiers and drive records are fictional; this never queries or writes devices.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary = Join-Path $repo 'rust/mri-manifest/target/debug/arrival_tracker.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Rust arrival tracking binary not built' }
$fixture = Join-Path $env:TEMP ('mri-arrivals-parity-' + [Guid]::NewGuid().ToString('N') + '.json')
$cases = 0
try {
    for ($n = 0; $n -lt 100; $n++) {
        $key = if ($n % 2 -eq 0) { 'DISK-X' } else { 'disk-x' }
        $priorKey = if ($n % 2 -eq 0) { 'disk-x' } else { 'DISK-X' }
        $number = if ($n % 9 -eq 0) { 8 } else { 7 }
        $offline = ($n % 7 -eq 0)
        $protected = if ($n % 11 -eq 0) { @($key) } else { @() }
        $config = [pscustomobject]@{
            StablePolls = 3
            MinimumDiskBytes = [long]1000
            MaximumDiskBytes = [long]100000000
        }
        $drive = [pscustomobject]@{
            Key = $key
            Number = $number
            Serial = 'SIMULATED-SERIAL'
            BusType = 'USB'
            IsBoot = $false
            IsSystem = $false
            IsOffline = $offline
            IsReadOnly = $false
            Size = [long]32000000
        }
        $records = @([pscustomobject]@{ Key = 'disk-x'; Label = 'MRI_01' })
        $inventory = if ($n % 13 -eq 0) { @() } else { @($drive) }
        $tracker = @{}
        $tracker[$priorKey] = [pscustomobject]@{
            Number = 7
            Stable = [long]($n % 5)
            Attempted = ($n % 17 -eq 0)
        }
        $rustDrives = @(
            foreach ($d in $inventory) {
                $reason = Get-MRIDriveBlockReason -Drive $d -Inventory $inventory -Config $config -ProtectedKeys $protected
                [pscustomobject]@{
                    Key = $d.Key
                    Number = $d.Number
                    Blocked = [bool]$reason
                }
            }
        )
        $payload = [pscustomobject]@{
            Inventory = $rustDrives
            Records = $records
            StablePolls = $config.StablePolls
            Tracker = $tracker
        }
        [IO.File]::WriteAllText($fixture, (ConvertTo-Json $payload -Compress -Depth 10), [Text.UTF8Encoding]::new($false))
        $output = & $binary $fixture
        $exit = $LASTEXITCODE
        if ($exit -ne 0 -or -not $output) { throw "Rust rejected valid synthetic arrival case $n" }
        $rust = $output | Out-String | ConvertFrom-Json
        if (-not $rust.Ok) { throw "Rust did not return success for case $n" }

        $actualArrivals = @(Update-MRIArrivals -Tracker $tracker -Inventory $inventory -Records $records -Config $config -ProtectedKeys $protected)
        if ($actualArrivals.Count -ne @($rust.Result.Arrivals).Count) {
            throw "Arrival count mismatch in case $n"
        }
        if ($actualArrivals.Count -gt 0 -and $actualArrivals[0].Label -cne $rust.Result.Arrivals[0].Label) {
            throw "Arrival identity mismatch in case $n"
        }
        $rustKeys = @($rust.Result.Tracker.PSObject.Properties)
        if ($rustKeys.Count -ne $tracker.Count) { throw "Tracker key count mismatch in case $n" }
        foreach ($existing in @($tracker.Keys)) {
            $matches = @($rustKeys | Where-Object { $_.Name -ieq $existing })
            if ($matches.Count -ne 1) { throw "Tracker key mismatch in case $n" }
            $left = $tracker[$existing]
            $right = $matches[0].Value
            if ([long]$left.Number -ne [long]$right.Number -or
                [long]$left.Stable -ne [long]$right.Stable -or
                [bool]$left.Attempted -ne [bool]$right.Attempted) {
                throw "Tracker state mismatch in case $n"
            }
        }
        $cases++
    }
    Write-Host "PASS: $cases original-PowerShell vs Rust stable-arrival tracking fixtures."
} finally {
    Remove-Item -LiteralPath $fixture -Force -ErrorAction SilentlyContinue
}
exit 0
