# SPDX-License-Identifier: GPL-3.0-only
# Non-destructive Rust vs PowerShell policy parity (no real disks).
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
$executable=Join-Path $root 'rust/mri-safety/target/debug/mri-safety.exe'
if (-not (Test-Path -LiteralPath $executable)) { throw 'Build the Rust selection tool before running parity.' }
function New-FakeDisk {
    param([int]$Number)
    [pscustomobject]@{
        Key="KEY_$Number"; Number=$Number; Serial="SERIAL$Number"; IdentityError=''
        BusType='USB'; Size=[long]32000000000; IsBoot=$false; IsSystem=$false
        IsOffline=$false; IsReadOnly=$false
    }
}
function Test-Parity {
    param([string]$Selection,[array]$Inventory,[string[]]$ProtectedKeys)
    $cfg=[pscustomobject]@{MinimumDiskBytes=[long]1000000000;MaximumDiskBytes=[long]64000000000}
    $inputObject=[pscustomobject]@{
        Selection=$Selection; Inventory=$Inventory; ProtectedKeys=@($ProtectedKeys)
        MinimumDiskBytes=$cfg.MinimumDiskBytes;MaximumDiskBytes=$cfg.MaximumDiskBytes
    }
    $json=ConvertTo-Json -InputObject $inputObject -Depth 10 -Compress
    $fixture=Join-Path $env:TEMP ([guid]::NewGuid().ToString('N')+'.json')
    try {
        [System.IO.File]::WriteAllText($fixture,$json,[System.Text.UTF8Encoding]::new($false))
        $resultJson=& $executable --input-file $fixture
        $nativeExit=$LASTEXITCODE
    } finally {
        Remove-Item -LiteralPath $fixture -Force -ErrorAction SilentlyContinue
    }
    if (-not $resultJson) { throw 'Rust returned no JSON.' }
    $rust=$resultJson | ConvertFrom-Json
    $originalError=$null
    $original=@()
    try {
        $original=@(Select-MRIUsbDrives $Selection $Inventory $cfg $ProtectedKeys)
    } catch {
        $originalError=$_.Exception.Message
    }
    if ($null -ne $originalError) {
        if ($nativeExit -eq 0 -or $rust.Error -cne $originalError -or @($rust.Selected).Count -ne 0) {
            throw "Difference: selection '$Selection': Rust error '$($rust.Error)', PowerShell error '$originalError'"
        }
    } else {
        if ($nativeExit -ne 0 -or $null -ne $rust.Error) {
            throw "Difference: selection '$Selection': Rust rejected PowerShell-approved disks"
        }
        $numbers=@($original | ForEach-Object { [int]$_.Number })
        $actual=@($rust.Selected)
        if (($numbers -join ',') -cne ($actual -join ',')) {
            throw "Selection mismatch: PowerShell $($numbers -join ',') versus Rust $($actual -join ',')"
        }
        if ($numbers.Count) {
            $phrase='ERASE '+(($numbers | Sort-Object) -join ',')
            if ($rust.Confirmation -cne $phrase) { throw 'Erase confirmation phrase mismatch.' }
        } elseif ($null -ne $rust.Confirmation) {
            throw 'Rust emitted an ERASE phrase for an empty selection.'
        }
    }
}
for ($i=0; $i -lt 100; $i++) {
    $disks=@(New-FakeDisk 6; New-FakeDisk 7; New-FakeDisk 8)
    $protected=@()
    if (($i % 2) -eq 1) { $disks[0].IsBoot=$true }
    if (($i % 3) -eq 2) { $disks[2].IsReadOnly=$true }
    if (($i % 5) -eq 4) { $protected=@('KEY_6') }
    Test-Parity -Selection '8,6' -Inventory $disks -ProtectedKeys $protected
}
foreach ($value in @('6,7,8','8,6,7,8','6,','6;7','99','2147483648','6,,7','', '  ')) {
    Test-Parity -Selection $value -Inventory @(New-FakeDisk 6;New-FakeDisk 7;New-FakeDisk 8) -ProtectedKeys @()
}
$serials=@(New-FakeDisk 6;New-FakeDisk 7;New-FakeDisk 8)
$serials[1].Serial='serial6'
Test-Parity -Selection '6' -Inventory $serials -ProtectedKeys @()
$offline=@(New-FakeDisk 6;New-FakeDisk 7;New-FakeDisk 8)
$offline[0].IsOffline=$true
Test-Parity -Selection '6' -Inventory $offline -ProtectedKeys @()
$duplicate=@(New-FakeDisk 6;New-FakeDisk 6;New-FakeDisk 8)
Test-Parity -Selection '6' -Inventory $duplicate -ProtectedKeys @()
Write-Host 'PASS: 112 synthetic Rust/PowerShell selection parity cases; no disks touched.'
