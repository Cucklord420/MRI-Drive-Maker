#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-13-2026. Uses only a newly created temporary ISO and VHD.
# USB classification alone is mocked for the VHD copy test; storage, file
# copying, hashing, volume GUID paths, process execution and flushing are real.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Windows is required.' }
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
Initialize-MRINative
$temp=Join-Path ([IO.Path]::GetTempPath()) ('mri-storage-smoke-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$vhd=Join-Path $temp 'owned-fixture.vhd'; $opened=$null; $attached=$false
$mountPath=Join-Path $temp 'owned-vhd-mount'; $mountPathAttached=$false
$failures=[Collections.Generic.List[string]]::new(); $count=0
function Check { param([bool]$Condition,[string]$Message='Assertion failed'); if (-not $Condition) { throw $Message } }
function Test-Storage { param([string]$Name,[scriptblock]$Code)
    $script:count++
    try { & $Code; Write-Host "PASS $Name" } catch { $failures.Add($_.Exception.Message); Write-Host "FAIL ${Name}: $($_.Exception.Message)" }
}
try {
    Test-Storage 'Windows storage cmdlets expose the parameter sets used by the builder' {
        foreach ($entry in @(@('Get-Volume','Partition'),@('New-Partition','InputObject'),@('Set-Partition','NoDefaultDriveLetter'),@('Mount-DiskImage','StorageType'),@('Clear-Disk','InputObject'))) {
            Check ((Get-Command $entry[0]).Parameters.ContainsKey($entry[1])) "$($entry[0]) lacks $($entry[1])."
        }
    }
    $source=Join-Path $temp 'Source files & spaces'
    [void][IO.Directory]::CreateDirectory((Join-Path $source 'efi/boot'))
    [IO.File]::WriteAllText((Join-Path $source 'efi/boot/bootx64.efi'),'test fixture, not a bootable EFI binary')
    [IO.File]::WriteAllText((Join-Path $source 'payload.dat'),'ISO contents copied by the actual Windows backend')
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class MRIStorageSmokeIsoWriter {
    public static void Save(object imageStream, string path) {
        IStream stream=(IStream)imageStream;
        IntPtr count=Marshal.AllocHGlobal(4);
        try {
            byte[] buffer=new byte[65536];
            using(FileStream output=File.Create(path)) {
                while(true) {
                    stream.Read(buffer,buffer.Length,count);
                    int length=Marshal.ReadInt32(count);
                    if(length==0) break;
                    output.Write(buffer,0,length);
                }
            }
        } finally { Marshal.FreeHGlobal(count); }
    }
}
'@
    $fsi=New-Object -ComObject IMAPI2FS.MsftFileSystemImage
    $fsi.FileSystemsToCreate=3; $fsi.VolumeName='MRI_TEST'
    $fsi.Root.AddTree($source,$false)
    $iso=Join-Path $temp 'source image.iso'
    $image=$fsi.CreateResultImage()
    [MRIStorageSmokeIsoWriter]::Save($image.ImageStream,$iso)
    $cfg=Read-MRIJson (Join-Path $root 'mri-config.example.json')
    $cfg.SourceKind='ISO'; $cfg.SourcePath=$iso; $cfg.MinimumDiskBytes=1
    $opened=Open-MRISource $cfg (Join-Path $temp 'cache')
    Test-Storage 'A real ISO opens read-only and exposes its contained boot-file tree' {
        Check $opened.OwnsMount
        Check ([IO.File]::Exists((Join-MRIPath $opened.Path 'efi/boot/bootx64.efi')))
        Check ((Get-DiskImage -ImagePath $iso).Attached)
    }
    Test-Storage 'An existing ISO mount is reused and not ejected by a second source context' {
        $second=Open-MRISource $cfg (Join-Path $temp 'cache')
        Check (-not $second.OwnsMount); Close-MRISource $second
        Check ((Get-DiskImage -ImagePath $iso).Attached)
    }
    $diskpart=Join-Path $temp 'create-owned-vhd.txt'
    [IO.File]::WriteAllText($diskpart,('create vdisk file="'+$vhd+'" maximum=512 type=expandable'+[Environment]::NewLine))
    $output=& (Join-Path $env:SystemRoot 'System32/diskpart.exe') /s $diskpart
    if ($LASTEXITCODE -ne 0 -or -not [IO.File]::Exists($vhd)) { throw "Could not create the owned temporary VHD: $output" }
    $mounted=Mount-DiskImage -ImagePath $vhd -StorageType VHD -Access ReadWrite -PassThru -ErrorAction Stop; $attached=$true
    $disks=@($mounted | Get-Disk -ErrorAction Stop)
    Check ($disks.Count -eq 1) 'VHD must resolve to exactly one disk.'
    $disk=$disks[0]
    Check (-not $disk.IsBoot -and -not $disk.IsSystem -and [string]$disk.BusType -eq 'File Backed Virtual') 'Only the owned file-backed test disk may be initialized.'
    $initialized=Initialize-Disk -InputObject $disk -PartitionStyle GPT -PassThru -ErrorAction Stop
    $part=New-Partition -InputObject $initialized -UseMaximumSize -ErrorAction Stop
    # NoDefaultDriveLetter is honored only on GPT disks. Set it before formatting
    # so the volume-GUID-only fixture never receives an automatically assigned letter.
    Set-Partition -InputObject $part -NoDefaultDriveLetter $true -ErrorAction Stop
    $part=Get-Partition -DiskNumber $disk.Number -PartitionNumber $part.PartitionNumber -ErrorAction Stop
    $part | Format-Volume -FileSystem FAT32 -NewFileSystemLabel 'MRI_TEST' -Confirm:$false -ErrorAction Stop | Out-Null
    $drive=[pscustomobject]@{Key=(Get-MRIDriveKey 'OWNED-VHD' $vhd 'TEST VHD' $disk.Size);Number=[int]$disk.Number
        Serial='OWNED-VHD';PnpId=$vhd;Model='TEST VHD';Size=[long]$disk.Size;IsBoot=$false;IsSystem=$false
        IsReadOnly=$false;IsOffline=$false;BusType='USB';IdentityError=''}
    $record=[pscustomobject]@{Key=$drive.Key;Serial=$drive.Serial;PnpId=$drive.PnpId;Model=$drive.Model;Size=$drive.Size;Label='MRI_01';AllowErase=$false}
    $module=Get-Module MRI.Batch
    & $module { param($fixtureDrive); $script:StorageFixtureDrive=$fixtureDrive; function script:Get-MRIInventory { $script:StorageFixtureDrive } } $drive
    $runtime=($cfg | ConvertTo-Json -Depth 6 | ConvertFrom-Json)
    $runtime.SourcePath=$opened.Path; $runtime.SourceKind='Folder'
    $runtime | Add-Member -NotePropertyName RecipeIdentity -NotePropertyValue (ConvertTo-Json $cfg -Depth 6 -Compress)
    $runtime | Add-Member -NotePropertyName InputStamp -NotePropertyValue $opened.Stamp
    $manifest=Get-MRIManifest $opened.Path
    $state=Join-Path $temp 'state.json'; $receipt=Join-Path $temp 'receipt.json'; $work=Join-Path $temp 'Work'; $cancel=Join-Path $temp 'cancel'
    $target=Get-MRITargetVolume $drive
    Test-Storage 'An archive source protects its original input and data tree without requiring an optical partition' {
        $resolved=@(Get-MRIResolvedProtectedPaths $runtime @($iso,$temp) $work $state $receipt)
        Check ($resolved -contains [IO.Path]::GetFullPath($iso)) 'The original ISO file was not protected.'
        Check ($resolved -contains [IO.Path]::GetFullPath($temp)) 'The archive cache/data root was not protected.'
        Check ($resolved -notcontains [IO.Path]::GetFullPath($opened.Path)) 'A derived optical mount path was mistaken for a writable physical source disk.'
    }
    $part=Get-Partition -DiskNumber $disk.Number -PartitionNumber $part.PartitionNumber -ErrorAction Stop
    Check ([string]::IsNullOrWhiteSpace([string]$part.DriveLetter)) "The owned fixture must have no drive letter for the volume-GUID-only test; current value is '$($part.DriveLetter)'."
    Test-Storage 'A real volume GUID source path protects its backing disk identity' {
        $protected=@(Get-MRIProtectedKeys @($target.Path) @($drive))
        Check ($protected.Count -eq 1 -and $protected[0] -eq $drive.Key) 'The volume GUID path did not protect its backing disk.'
        $selectionConfig=($cfg | ConvertTo-Json -Depth 6 | ConvertFrom-Json)
        $selectionConfig.MinimumDiskBytes=1
        $selectionConfig.MaximumDiskBytes=[long]64000000000
        $blocked=$false
        try { Select-MRIUsbDrives ([string]$drive.Number) @($drive) $selectionConfig $protected | Out-Null }
        catch { $blocked=$_.Exception.Message -match 'source files or the automation' }
        Check $blocked 'The volume-GUID source disk remained eligible for selection.'
    }
    [void][IO.Directory]::CreateDirectory($mountPath)
    Add-PartitionAccessPath -DiskNumber $disk.Number -PartitionNumber $part.PartitionNumber -AccessPath $mountPath -ErrorAction Stop
    $mountPathAttached=$true
    Test-Storage 'A real directory mount protects its backing disk instead of the host drive' {
        $protected=@(Get-MRIProtectedKeys @((Join-Path $mountPath 'source.zip')) @($drive))
        Check ($protected.Count -eq 1 -and $protected[0] -eq $drive.Key) 'The mounted-folder path did not protect its backing disk.'
    }
    Test-Storage 'A junction alias is rejected instead of being attributed to its parent disk' {
        $junctionPath=Join-Path $temp 'owned-vhd-junction'
        New-Item -ItemType Junction -Path $junctionPath -Target $mountPath -ErrorAction Stop | Out-Null
        try {
            $message=''
            try { Get-MRIProtectedKeys @((Join-Path $junctionPath 'source.zip')) @($drive) | Out-Null }
            catch { $message=$_.Exception.Message }
            Check ($message -match 'unmapped filesystem link') 'A junction path was not rejected before disk selection.'
        } finally {
            $command='rmdir "'+$junctionPath+'"'
            & (Join-Path $env:SystemRoot 'System32/cmd.exe') /d /c $command | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'Could not safely remove the owned test junction.' }
        }
    }
    Test-Storage 'A local SMB source path maps back to its physical USB disk' {
        $module=Get-Module MRI.Batch
        & $module {
            param($share)
            $script:StorageSmokeShare=$share
            function script:Get-CimInstance {
                [CmdletBinding()] param([string]$ClassName,[string]$Filter)
                if ($ClassName -eq 'Win32_Share') { return $script:StorageSmokeShare }
                throw "Unexpected CIM query in local-share smoke: $ClassName"
            }
        } ([pscustomobject]@{Name='MRI_TEST';Path=$mountPath})
        try {
            $sharePath='\\localhost\MRI_TEST\source.zip'
            $protected=@(Get-MRIProtectedKeys @($sharePath) @($drive))
            Check ($protected.Count -eq 1 -and $protected[0] -eq $drive.Key) 'The local SMB share did not protect its backing USB.'
        } finally { & $module { Remove-Item Function:\Get-CimInstance -ErrorAction Stop } }
    }
    Test-Storage 'Actual FAT32 volume GUID paths can be enumerated safely' {
        Check ($target.FileSystem -eq 'FAT32')
        $before=Get-MRIManifest $target.Path -IgnoreSystemDirectories
        Check (@($before.Entries).Count -eq 0)
    }
    Test-Storage 'ISO-to-volume copy runs the real copy worker, verification and volume flushing' {
        $result=Invoke-MRIBuild $record $runtime $manifest @() $state $receipt $work $cancel
        if (-not $result.Success) {
            $copyLog=Join-Path $work 'copy.log'
            if ([IO.File]::Exists($copyLog)) { Write-Host ([IO.File]::ReadAllText($copyLog)) }
            throw $result.Error
        }
        Check ((Read-MRIJson $state).Stage -eq 'READY')
        Check ($null -eq (Read-MRIJson $receipt).SetupExit) 'Direct-copy mode must not run setup.'
        Assert-MRIManifest $manifest $target.Path
    }
    Test-Storage 'Actual verified media is skipped on reinsertion' {
        $result=Invoke-MRIBuild $record $runtime $manifest @() $state $receipt $work $cancel
        Check ($result.Success -and $result.AlreadyReady)
    }
    Test-Storage 'Unexpected content on the virtual target prevents a second copy' {
        $client=Join-MRIPath $target.Path 'keep-me.txt'; [IO.File]::WriteAllText($client,'owned test fixture')
        $result=Invoke-MRIBuild $record $runtime $manifest @() $state $receipt $work $cancel
        Check (-not $result.Success); Check ([IO.File]::ReadAllText($client) -eq 'owned test fixture')
    }
    Test-Storage 'Destructive storage guard rejects a file-backed disk even with mocked USB inventory' {
        $blocked=$false
        try { Get-MRIWritableDisk $record $runtime @() $drive.Number $cancel | Out-Null } catch { $blocked=$true }
        Check $blocked
    }
    Test-Storage 'The live source mount is rechecked immediately before disk erasure' {
        $eraseRecord=[pscustomobject]@{Key=$drive.Key;Serial=$drive.Serial;PnpId=$drive.PnpId;Model=$drive.Model
            Size=$drive.Size;Label='MRI_01';AllowErase=$true}
        $guardDisk=[pscustomobject]@{Number=$drive.Number;PartitionStyle='GPT';SerialNumber=$drive.Serial
            FriendlyName=$drive.Model;Size=$drive.Size;BusType='USB';IsBoot=$false;IsSystem=$false;IsOffline=$false;IsReadOnly=$false}
        $guardHardware=[pscustomobject]@{Index=$drive.Number;SerialNumber=$drive.Serial;PNPDeviceID=$drive.PnpId}
        $module=Get-Module MRI.Batch
        & $module {
            param($fixtureDrive,$fixtureDisk,$fixtureHardware)
            $script:PathGuardDrive=$fixtureDrive
            $script:PathGuardDisk=$fixtureDisk
            $script:PathGuardHardware=$fixtureHardware
            $script:PathGuardClearCalls=0
            function script:Get-MRIInventory { $script:PathGuardDrive }
            function script:Get-Disk { [CmdletBinding()] param([int]$Number); $script:PathGuardDisk }
            function script:Get-CimInstance { [CmdletBinding()] param([string]$ClassName,[string]$Filter); $script:PathGuardHardware }
            function script:Clear-Disk {
                [CmdletBinding()] param($InputObject,[switch]$RemoveData,[switch]$RemoveOEM,[switch]$Confirm)
                $script:PathGuardClearCalls++
                throw 'Test intercepted Clear-Disk.'
            }
        } $drive $guardDisk $guardHardware
        $message=''
        try {
            Initialize-MRITarget $eraseRecord $runtime @() $drive.Number $cancel @((Join-Path $mountPath 'source.zip')) | Out-Null
        } catch { $message=$_.Exception.Message }
        $clearCalls=& $module { $script:PathGuardClearCalls }
        Check ($message -match 'now contains a source or automation path') 'The changed source mount was not rejected.'
        Check ($clearCalls -eq 0) 'Clear-Disk was reached for the protected source USB.'
    }
    Write-Host "`n$($count-$failures.Count)/$count Windows storage checks passed. Only owned ISO/VHD fixtures were used."
    if ($failures.Count) { exit 1 }
} finally {
    try { if ($mountPathAttached) { Remove-PartitionAccessPath -DiskNumber $disk.Number -PartitionNumber $part.PartitionNumber -AccessPath $mountPath -ErrorAction Stop } }
    finally {
        try { if ($attached) { Dismount-DiskImage -ImagePath $vhd -ErrorAction Stop | Out-Null } }
        finally { try { Close-MRISource $opened } finally { Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue } }
    }
}
exit 0
