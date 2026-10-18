#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Focused source-to-partition resolution tests. All Storage cmdlets are mocked.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
$module=Get-Module MRI.Batch
& $module {
    $script:ProtectedPathPartitions=@()
    $script:ProtectedPathDisks=@()
    $script:ProtectedPathShares=@()
    function script:Get-Partition {
        [CmdletBinding()] param()
        $script:ProtectedPathPartitions
    }
    function script:Get-Disk {
        [CmdletBinding()] param([int]$Number)
        @($script:ProtectedPathDisks | Where-Object { $_.Number -eq $Number })
    }
    function script:Get-CimInstance {
        [CmdletBinding()] param([string]$ClassName,[string]$Filter)
        if ($ClassName -eq 'Win32_Share') { return $script:ProtectedPathShares }
        if ($ClassName -eq 'Win32_ComputerSystem') { return [pscustomobject]@{Name='MRI-HOST';DNSHostName='MRI-HOST';Domain='TEST'} }
        if ($ClassName -eq 'Win32_NetworkAdapterConfiguration') { return [pscustomobject]@{IPAddress=@('192.0.2.10')} }
        throw "Unexpected CIM query in protected-path fixture: $ClassName"
    }
}
function Set-ProtectedFixtures {
    param([object[]]$Partitions,[object[]]$Disks=@())
    $fixture=[pscustomobject]@{Partitions=$Partitions;Disks=$Disks}
    & $module {
        param($fixtures)
        $script:ProtectedPathPartitions=[object[]]$fixtures.Partitions
        $script:ProtectedPathDisks=[object[]]$fixtures.Disks
    } $fixture
}
function Set-ProtectedShares {
    param([object[]]$Shares=@())
    $fixture=[pscustomobject]@{Shares=$Shares}
    & $module { param($value); $script:ProtectedPathShares=[object[]]$value.Shares } $fixture
}
function Check {
    param([bool]$Condition,[string]$Message='Assertion failed.')
    if (-not $Condition) { throw $Message }
}
function Throws {
    param([scriptblock]$Code,[string]$Pattern)
    $caught=$false
    try { & $Code | Out-Null }
    catch {
        $caught=$true
        if ($_.Exception.Message -notmatch $Pattern) { throw "Unexpected rejection: $($_.Exception.Message)" }
    }
    if (-not $caught) { throw "Expected rejection matching '$Pattern'." }
}
$temp=Join-Path ([IO.Path]::GetTempPath()) ('mri-protected-paths-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
try {
    $hostRoot=[IO.Path]::GetPathRoot($temp)
    $mountRoot=Join-Path $temp 'mounted-usb'
    [void][IO.Directory]::CreateDirectory($mountRoot)
    $hostPartition=[pscustomobject]@{DiskNumber=0;AccessPaths=@($hostRoot)}
    $mountPartition=[pscustomobject]@{DiskNumber=8;AccessPaths=@($mountRoot+[IO.Path]::DirectorySeparatorChar)}
    $usb=[pscustomobject]@{Number=8;Key='USB-IDENTITY-8';BusType='USB'}
    $hostDisk=[pscustomobject]@{Number=0;BusType='SATA'}
    $sourcePath=Join-Path $mountRoot 'source.zip'
    Set-ProtectedFixtures @($hostPartition,$mountPartition) @($hostDisk)

    # Reproduce the prior root-only lookup: it identifies the host partition
    # and misses the USB mounted below it. The fixed resolver selects the
    # longest matching AccessPath and returns the USB's identity.
    $legacyRoot=[IO.Path]::GetPathRoot([IO.Path]::GetFullPath($sourcePath))
    $legacyPartition=@($hostPartition,$mountPartition | Where-Object {
        @($_.AccessPaths | Where-Object { $_ -eq $legacyRoot }).Count -gt 0
    })
    $legacyKeys=@($usb | Where-Object { $_.Number -in @($legacyPartition | ForEach-Object { $_.DiskNumber }) } | ForEach-Object { $_.Key })
    Check ($legacyKeys.Count -eq 0) 'The fixture did not reproduce the old drive-root lookup gap.'
    $protected=@(Get-MRIProtectedKeys @($sourcePath) @($usb))
    Check ($protected.Count -eq 1 -and $protected[0] -eq $usb.Key) 'Mounted-folder source identity was not protected.'
    Write-Host 'PASS mounted-folder path resolves to the deepest partition access path'

    $internalPath=Join-Path $temp 'tool.ps1'
    $protected=@(Get-MRIProtectedKeys @($internalPath) @($usb))
    Check ($protected.Count -eq 0) 'An internal source path protected the mounted USB incorrectly.'
    Write-Host 'PASS parent partition does not capture an unrelated mounted USB'

    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        $linkRoot=Join-Path $temp 'unmapped-link'
        [void][IO.Directory]::CreateSymbolicLink($linkRoot,$mountRoot)
        try {
            Throws { Get-MRIProtectedKeys @((Join-Path $linkRoot 'source.zip')) @($usb) } 'unmapped filesystem link'
            Write-Host 'PASS an unmapped filesystem link fails closed instead of resolving to its parent disk'
        } finally { [IO.Directory]::Delete($linkRoot) }
    }

    $letterUsb=[pscustomobject]@{Number=7;Key='USB-IDENTITY-7';BusType='USB'}
    Set-ProtectedFixtures @([pscustomobject]@{DiskNumber=7;AccessPaths=@($hostRoot)}) @()
    $protected=@(Get-MRIProtectedKeys @((Join-Path $temp 'drive-letter-source.iso')) @($letterUsb))
    Check ($protected.Count -eq 1 -and $protected[0] -eq $letterUsb.Key) 'Drive-letter root did not map to its backing disk.'
    Write-Host 'PASS drive-letter access path continues to protect its backing disk'

    $volumeRoot='\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\'
    $volumePartition=[pscustomobject]@{DiskNumber=9;AccessPaths=@($volumeRoot)}
    $volumeUsb=[pscustomobject]@{Number=9;Key='USB-IDENTITY-9';BusType='USB'}
    if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
        Set-ProtectedFixtures @($hostPartition,$volumePartition) @($hostDisk)
        $protected=@(Get-MRIProtectedKeys @($volumeRoot+'source.iso') @($volumeUsb))
        Check ($protected.Count -eq 1 -and $protected[0] -eq $volumeUsb.Key) 'Volume GUID source identity was not protected.'
        Write-Host 'PASS volume GUID path resolves to its backing partition'

        Set-ProtectedFixtures @($hostPartition,$mountPartition) @($hostDisk)
        $extendedPath='\\?\'+$sourcePath
        $protected=@(Get-MRIProtectedKeys @($extendedPath) @($usb))
        Check ($protected.Count -eq 1 -and $protected[0] -eq $usb.Key) 'An extended drive path did not resolve to its backing partition.'
        Write-Host 'PASS extended-length drive paths retain their physical disk identity'

        Set-ProtectedFixtures @($hostPartition,$mountPartition) @($hostDisk)
        Set-ProtectedShares @([pscustomobject]@{Name='MRI_TEST';Path=$mountRoot})
        $sharePath='\\localhost\MRI_TEST\source.zip'
        $protected=@(Get-MRIProtectedKeys @($sharePath) @($usb))
        Check ($protected.Count -eq 1 -and $protected[0] -eq $usb.Key) 'Local SMB share did not resolve to its backing USB partition.'
        Write-Host 'PASS local SMB share maps to its backing filesystem path'

        $remotePath='\\192.0.2.20\MRI_TEST\source.zip'
        $protected=@(Get-MRIProtectedKeys @($remotePath) @())
        Check ($protected.Count -eq 0) 'A verified remote SMB share was treated as a local physical disk.'
        Write-Host 'PASS a verified remote SMB source remains supported without a local disk key'
    } else {
        Write-Host 'SKIP volume GUID and local SMB normalization require Windows; covered by SelfTest and owned-VHD smoke'
    }

    $remoteKind=& $module {
        param($fixture)
        Get-MRIUNCServerKind $fixture.Server $fixture.LocalNames $fixture.LocalAddresses $fixture.ServerAddresses
    } ([pscustomobject]@{Server='192.0.2.20';LocalNames=@('MRI-HOST');LocalAddresses=@('192.0.2.10');ServerAddresses=@('192.0.2.20')})
    Check ($remoteKind -eq 'Remote') 'A verified remote network share was not classified as remote.'
    $localKind=& $module {
        param($fixture)
        Get-MRIUNCServerKind $fixture.Server $fixture.LocalNames $fixture.LocalAddresses $fixture.ServerAddresses
    } ([pscustomobject]@{Server='files-alias';LocalNames=@('MRI-HOST');LocalAddresses=@('192.0.2.10');ServerAddresses=@('192.0.2.10')})
    Check ($localKind -eq 'Local') 'A server name resolving to a local address was not classified as local.'
    Throws { & $module { Resolve-MRIUNCPath '\\localhost\MRI_TEST\..\outside\source.iso' } } 'unsafe path segment'
    Write-Host 'PASS local SMB traversal segments are rejected before share resolution'
    Throws {
        & $module {
            param($fixture)
            Get-MRIUNCServerKind $fixture.Server $fixture.LocalNames $fixture.LocalAddresses $fixture.ServerAddresses
        } ([pscustomobject]@{Server='unresolved-host';LocalNames=@('MRI-HOST');LocalAddresses=@('192.0.2.10');ServerAddresses=@()})
    } 'Could not verify'
    Write-Host 'PASS remote shares remain external; local aliases and unresolved server names are distinguished safely'

    $ambiguous=@(
        [pscustomobject]@{DiskNumber=8;AccessPaths=@($hostRoot)}
        [pscustomobject]@{DiskNumber=9;AccessPaths=@($hostRoot)}
    )
    Set-ProtectedFixtures $ambiguous @()
    Throws { Get-MRIProtectedKeys @($internalPath) @($usb) } 'multiple partitions'
    Write-Host 'PASS duplicate access paths fail closed'

    $usbRoot=[IO.Path]::GetPathRoot($temp)
    Set-ProtectedFixtures @([pscustomobject]@{DiskNumber=10;AccessPaths=@($usbRoot)}) @([pscustomobject]@{Number=10;BusType='USB'})
    Throws { Get-MRIProtectedKeys @($internalPath) @() } 'usable hardware identity'
    Write-Host 'PASS USB identity omitted from inventory fails closed'

    foreach ($busType in @('Unknown','Virtual','File Backed Virtual','Storage Spaces')) {
        Set-ProtectedFixtures @([pscustomobject]@{DiskNumber=14;AccessPaths=@($usbRoot)}) @([pscustomobject]@{Number=14;BusType=$busType})
        Throws { Get-MRIProtectedKeys @($internalPath) @() } 'physical disk identity'
    }
    Write-Host 'PASS unknown and virtual disk mappings fail closed'

    $badUsb=[pscustomobject]@{Number=11;Key='';BusType='USB'}
    Set-ProtectedFixtures @([pscustomobject]@{DiskNumber=11;AccessPaths=@($hostRoot)}) @()
    Throws { Get-MRIProtectedKeys @($internalPath) @($badUsb) } 'usable hardware identity'
    Write-Host 'PASS unusable USB identity fails closed'

    $shortRoot=Join-Path $temp 'mount'
    $neighborPath=Join-Path $temp 'mounted-usb-neighbor/source.zip'
    Set-ProtectedFixtures @([pscustomobject]@{DiskNumber=12;AccessPaths=@($shortRoot+[IO.Path]::DirectorySeparatorChar)}) @()
    Throws { Get-MRIProtectedKeys @($neighborPath) @() } 'Could not map'
    Write-Host 'PASS path-prefix boundary does not confuse sibling directories'
    $unmountedPath=Join-Path $temp 'unmounted-volume/source.iso'
    Set-ProtectedFixtures @([pscustomobject]@{DiskNumber=13;AccessPaths=@()}) @()
    Throws { Get-MRIProtectedKeys @($unmountedPath) @() } 'Could not map'
    Write-Host 'PASS an unmounted volume with no access paths cannot be resolved'
    Write-Host 'Protected-path behavior checks completed; no physical storage was accessed.'
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
