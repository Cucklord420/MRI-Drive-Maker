#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-13-2026. Temporary source/ZIP fixtures only; no drive writes.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
Import-Module (Join-Path (Split-Path $PSScriptRoot -Parent) 'MRI.Batch.psm1') -Force -DisableNameChecking
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$temp=Join-Path ([IO.Path]::GetTempPath()) ('mri-source-tests-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$count=0; $failures=[Collections.Generic.List[string]]::new()
function Check { param([bool]$Condition); if (-not $Condition) { throw 'Assertion failed.' } }
function Test-Source { param([string]$Name,[scriptblock]$Code)
    $script:count++
    try { & $Code; Write-Host "PASS $Name" } catch { $failures.Add($_.Exception.Message); Write-Host "FAIL ${Name}: $($_.Exception.Message)" }
}
function Make-Zip { param([string]$Path,[string[]]$Names)
    $zip=[IO.Compression.ZipFile]::Open($Path,[IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($name in $Names) {
            $entry=$zip.CreateEntry($name); $writer=[IO.StreamWriter]::new($entry.Open())
            try { $writer.Write('source fixture') } finally { $writer.Dispose() }
        }
    } finally { $zip.Dispose() }
}
try {
    $folder=Join-Path $temp 'input with spaces & characters'
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'efi/boot'))
    [IO.File]::WriteAllText((Join-Path $folder 'efi/boot/bootx64.efi'),'fixture efi')
    $cfg=Read-MRIJson (Join-Path (Split-Path $PSScriptRoot -Parent) 'mri-config.example.json')
    $cfg.SourcePath=$folder
    Test-Source 'Extracted source folders open without a setup batch' {
        $opened=Open-MRISource $cfg (Join-Path $temp 'cache'); Check ($opened.Path -eq $folder); Close-MRISource $opened
    }
    Test-Source 'Single wrapper folders resolve to the actual boot-file root' {
        $wrapped=Join-Path $temp 'wrapped'; $inside=Join-Path $wrapped 'MRI files'
        [void][IO.Directory]::CreateDirectory($wrapped); Copy-Item -LiteralPath $folder -Destination $inside -Recurse
        Check ((Get-MRIMediaRoot $wrapped) -eq $inside)
    }
    Test-Source 'ZIP contents open in an isolated cache and are removed on close' {
        $zip=Join-Path $temp 'source.zip'; Make-Zip $zip @('MRI/efi/boot/bootx64.efi','MRI/payload.dat')
        $zipCfg=($cfg | ConvertTo-Json | ConvertFrom-Json); $zipCfg.SourceKind='ZIP'; $zipCfg.SourcePath=$zip
        $opened=Open-MRISource $zipCfg (Join-Path $temp 'cache')
        Check ([IO.File]::Exists((Join-Path $opened.Path 'payload.dat')))
        $cache=$opened.CacheDirectory; Close-MRISource $opened; Check (-not [IO.Directory]::Exists($cache))
        Check ([IO.File]::Exists($zip))
    }
    Test-Source 'Archive traversal paths are rejected before extraction' {
        foreach ($name in @('../escape.txt','/absolute.txt','C:/absolute.txt','EFI/../escape.txt')) {
            $zip=Join-Path $temp ([guid]::NewGuid().ToString('N')+'.zip'); Make-Zip $zip @($name)
            $caught=$false
            try { Expand-MRIArchive $zip (Join-Path $temp 'unsafe') } catch { $caught=$true }
            Check $caught
        }
    }
    Test-Source 'Archive case-insensitive duplicate paths are rejected' {
        $zip=Join-Path $temp 'duplicate.zip'; Make-Zip $zip @('EFI/file.dat','efi/FILE.dat')
        $caught=$false; try { Expand-MRIArchive $zip (Join-Path $temp 'duplicate') } catch { $caught=$true }; Check $caught
    }
    Test-Source 'Unknown source kinds are rejected' {
        $bad=($cfg | ConvertTo-Json | ConvertFrom-Json); $bad.SourceKind='Unknown'
        $caught=$false; try { Assert-MRIConfig $bad } catch { $caught=$true }; Check $caught
    }
    Test-Source 'Windows device aliases in source archives are rejected before files are extracted' {
        foreach ($name in @('NUL','efi/CON.txt','PRN.log','COM1.dat','LPT9','AUX')) {
            $zip=Join-Path $temp ([guid]::NewGuid().ToString('N')+'.zip'); Make-Zip $zip @('normal.txt',$name)
            $destination=Join-Path $temp ([guid]::NewGuid().ToString('N'))
            $caught=$false; try { Expand-MRIArchive $zip $destination } catch { $caught=$true }; Check $caught
            Check (-not [IO.File]::Exists((Join-Path $destination 'normal.txt')))
        }
    }
    Test-Source 'Authenticated runtime data detects tampering before it can be used' {
        $path=Join-Path $temp 'plan.json'
        $hash=Write-MRIJson $path ([pscustomobject]@{TargetRoot='selected-volume';PrepareDisk=$false}) -PassThruHash
        Check ((Read-MRIAuthenticatedJson $path $hash).TargetRoot -eq 'selected-volume')
        [IO.File]::WriteAllText($path,'{"TargetRoot":"system-volume","PrepareDisk":true}')
        $caught=$false; try { Read-MRIAuthenticatedJson $path $hash | Out-Null } catch { $caught=$_.Exception.Message -match 'changed or is corrupt' }; Check $caught
    }
    Test-Source 'Chunked SHA-256 matches the standard result and checks cancellation inside a large file' {
        $path=Join-Path $temp 'large.bin'; $bytes=New-Object byte[] (4MB)
        for ($i=0; $i -lt $bytes.Length; $i+=4096) { $bytes[$i]=[byte](($i/4096)%251) }
        [IO.File]::WriteAllBytes($path,$bytes)
        Check ((Get-MRIFileHash $path) -eq (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash)
        $counter=[pscustomobject]@{Calls=0}; $caught=$false
        try { Get-MRIFileHash $path -Guard { $counter.Calls++; if ($counter.Calls -eq 2) { throw 'fixture cancellation' } } | Out-Null }
        catch { $caught=$_.Exception.Message -match 'fixture cancellation' }
        Check $caught; Check ($counter.Calls -eq 2)
        $handle=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None); $handle.Dispose()
    }
    Test-Source 'A cancelled storage-object query cannot reach disk APIs' {
        $cancel=Join-Path $temp 'cancel'; [IO.File]::WriteAllText($cancel,'cancel')
        $caught=$false; try { Get-MRIWritableDisk $null $null @() 0 $cancel | Out-Null } catch { $caught=$_.Exception.Message -match 'cancelled' }; Check $caught
    }
    $guardDrive=[pscustomobject]@{Key=(Get-MRIDriveKey 'TEST-A' 'USB\\TEST-A' 'TEST USB' 32000000000);Number=6
        Serial='TEST-A';PnpId='USB\\TEST-A';Model='TEST USB';Size=[long]32000000000;IsBoot=$false;IsSystem=$false
        IsReadOnly=$false;IsOffline=$false;BusType='USB';IdentityError=''}
    $record=[pscustomobject]@{Key=$guardDrive.Key;AllowErase=$true;Label='MRI_01'}
    $guardSim=[pscustomobject]@{Drive=$guardDrive;Disk=[pscustomobject]@{Number=6;SerialNumber='TEST-A';FriendlyName='TEST USB';Size=[long]32000000000
        BusType='USB';IsBoot=$false;IsSystem=$false;IsOffline=$false;IsReadOnly=$false;PartitionStyle='MBR'}
        Hardware=[pscustomobject]@{Index=6;PNPDeviceID='USB\\TEST-A';SerialNumber='TEST-A'};Writes=0}
    $module=Get-Module MRI.Batch
    & $module {
        param($simulation)
        $script:GuardSimulation=$simulation
        function script:Get-MRIInventory { $script:GuardSimulation.Drive }
        function script:Get-Disk { param($Number); $script:GuardSimulation.Disk }
        function script:Get-CimInstance { param($ClassName,$Filter); $script:GuardSimulation.Hardware }
        function script:Clear-Disk { param($InputObject,[switch]$RemoveData,[switch]$RemoveOEM,$Confirm); $script:GuardSimulation.Writes++; throw 'A simulated storage write was reached.' }
    } $guardSim
    Test-Source 'A matching fresh storage object is returned for the registered physical drive' {
        $actual=Get-MRIWritableDisk $record $cfg @() 6 ''
        Check ([object]::ReferenceEquals($actual,$guardSim.Disk))
    }
    Test-Source 'A replacement reusing the displayed disk number is blocked before erasure' {
        $guardSim.Disk.SerialNumber='REPLACEMENT'
        $caught=$false
        try { Initialize-MRITarget $record $cfg @() 6 '' | Out-Null } catch { $caught=$_.Exception.Message -match 'changed or is protected' }
        Check $caught; Check ($guardSim.Writes -eq 0)
        $guardSim.Disk.SerialNumber='TEST-A'
    }
    Test-Source 'A fresh non-USB storage object is blocked even when inventory says USB' {
        $guardSim.Disk.BusType='SATA'
        $caught=$false
        try { Initialize-MRITarget $record $cfg @() 6 '' | Out-Null } catch { $caught=$_.Exception.Message -match 'changed or is protected' }
        Check $caught; Check ($guardSim.Writes -eq 0)
        $guardSim.Disk.BusType='USB'
    }
    Write-Host "`n$($count-$failures.Count)/$count source tests passed."
    if ($failures.Count) { exit 1 }
} finally { Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue }
exit 0
