#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-10-2026 for MRI Drive Maker.
# GPL-3.0-only. Uses temporary directories and mocked storage/process APIs only.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
Initialize-MRINative
$testRoot=Join-Path ([IO.Path]::GetTempPath()) ('mri-tests-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
$count=0; $failures=[Collections.Generic.List[string]]::new()
function Check { param([bool]$Condition,[string]$Message='Assertion failed'); if (-not $Condition) { throw $Message } }
function Throws {
    param([scriptblock]$Code,[string]$Pattern='.*')
    $caught=$false
    try { & $Code | Out-Null } catch { $caught=$true; if ($_.Exception.Message -notmatch $Pattern) { throw "Unexpected error: $($_.Exception.Message)" } }
    if (-not $caught) { throw 'Expected the operation to be rejected.' }
}
function Test { param([string]$Name,[scriptblock]$Code)
    $script:count++
    try { & $Code; Write-Host "PASS $Name" }
    catch { $script:failures.Add("${Name}: $($_.Exception.Message)"); Write-Host "FAIL ${Name}: $($_.Exception.Message)" -ForegroundColor Red }
}
function Drive {
    param([string]$Serial='STORE-A',[int]$Number=6,[string]$Pnp='USBSTOR\DISK&VEN_STORE\STORE-A&0')
    [pscustomobject]@{ Key=(Get-MRIDriveKey $Serial $Pnp 'STORE USB' 32000000000); Number=$Number; Serial=$Serial; PnpId=$Pnp
        Model='STORE USB'; Size=[long]32000000000; IsBoot=$false; IsSystem=$false; IsReadOnly=$false; IsOffline=$false; BusType='USB'; IdentityError='' }
}
$source=Join-Path $testRoot 'Source'; [void][IO.Directory]::CreateDirectory((Join-Path $source 'efi/boot'))
[IO.File]::WriteAllText((Join-Path $source 'mri.bat'),'@echo off'+[Environment]::NewLine+'exit /b 0')
[IO.File]::WriteAllText((Join-Path $source 'payload.dat'),'sample approved fixture payload')
[IO.File]::WriteAllText((Join-Path $source 'efi/boot/bootx64.efi'),'fixture boot file')
$baseConfig=Read-MRIJson (Join-Path $root 'mri-config.example.json'); $baseConfig.SourcePath=$source; $baseConfig.SetupBatch='mri.bat'
$manifest=Get-MRIManifest $source
$d=Drive
$record=[pscustomobject]@{ Key=$d.Key; Serial=$d.Serial; PnpId=$d.PnpId; Model=$d.Model; Size=$d.Size; Label='MRI_01'; AllowErase=$true }
$sim=[pscustomobject]@{ Inventory=@($d); Source=$source; TargetRoot=''; Letter='R'; CopyCode=0; SetupCode=0; CorruptCopy=$false
    NoPartitions=$false; PartitionError=$false; ActualPartitionStyle='RAW'; CorruptSetup=$false; RemoveDuringCopy=$false; SetupCalls=0; CopyCalls=0; FormatCalls=0; FlushCalls=0; AddOutput=$false; FlushFails=$false; AddCopyExtra=$false; AddFinalExtra=$false; HashCalls=0 }
$sim | Add-Member -NotePropertyName ProtectionPartitions -NotePropertyValue @()
$sim | Add-Member -NotePropertyName ProtectionDisks -NotePropertyValue @()
$module=Get-Module MRI.Batch
& $module {
    param($sim)
    $script:Sim=$sim
    function script:Get-MRIInventory { $script:Sim.Inventory }
    function script:Get-Partition {
        [CmdletBinding()] param([int]$DiskNumber)
        if ($script:Sim.ProtectionPartitions.Count) { return $script:Sim.ProtectionPartitions }
        if ($script:Sim.PartitionError) { throw 'No MSFT_Partition objects found.' }
        if ($script:Sim.NoPartitions) { return }
        [pscustomobject]@{DiskNumber=$DiskNumber; PartitionNumber=1; IsBoot=$false; IsSystem=$false}
    }
    function script:Get-Disk {
        [CmdletBinding()] param([int]$Number)
        $fixture=@($script:Sim.ProtectionDisks | Where-Object { $_.Number -eq $Number })
        if ($fixture.Count) { return $fixture }
        [pscustomobject]@{Number=$Number;PartitionStyle=$script:Sim.ActualPartitionStyle;BusType='SATA'}
    }
    function script:Get-MRITargetVolume {
        param($Drive)
        [pscustomobject]@{Path=$script:Sim.TargetRoot; Letter=$script:Sim.Letter; Number=$Drive.Number; PartitionNumber=1; FileSystem='FAT32'; FreeBytes=[long]30000000000}
    }
    function script:Get-MRIWindowsExecutable { param($Name); return $Name }
    $script:RealFileHash=${function:Get-MRIFileHash}
    function script:Get-MRIFileHash { param($Path,$Guard); $script:Sim.HashCalls++; & $script:RealFileHash $Path $Guard }
    function script:Flush-MRIVolume {
        param($Path)
        $script:Sim.FlushCalls++
        if ($script:Sim.FlushFails) { throw 'Flush failed.' }
        if ($script:Sim.AddFinalExtra) { [IO.File]::WriteAllText((Join-Path $script:Sim.TargetRoot 'unapproved-final.txt'),'unexpected data') }
    }
    function script:Initialize-MRITarget {
        param($Record,$Config,$ProtectedKeys,$ExpectedNumber,$CancelPath='',$ProtectedPaths=@())
        Assert-MRINotCancelled $CancelPath
        if (-not $Record.AllowErase) { throw 'This drive was not individually enrolled for automatic erasure.' }
        [void](Assert-MRIDrive $Record $Config $ProtectedKeys $ExpectedNumber)
        $script:Sim.FormatCalls++
        Get-ChildItem -LiteralPath $script:Sim.TargetRoot -Force | Remove-Item -Recurse -Force
        return Get-MRITargetVolume (Assert-MRIDrive $Record $Config $ProtectedKeys $ExpectedNumber)
    }
    function script:Invoke-MRICopy {
        param($Config,$Target,$Manifest,$WorkPath,$Guard,$CancelPath)
        & $Guard $true | Out-Null
            $script:Sim.CopyCalls++
            Get-ChildItem -LiteralPath $script:Sim.Source -Force | Copy-Item -Destination $script:Sim.TargetRoot -Recurse -Force
            if ($script:Sim.AddCopyExtra) { [IO.File]::WriteAllText((Join-Path $script:Sim.TargetRoot 'unexpected.dat'),'unlisted fixture') }
            if ($script:Sim.CorruptCopy) { [IO.File]::WriteAllText((Join-Path $script:Sim.TargetRoot 'payload.dat'),'corrupt') }
            if ($script:Sim.RemoveDuringCopy) { $script:Sim.Inventory=@() }
            & $Guard $true | Out-Null
            return $script:Sim.CopyCode
    }
    function script:Invoke-MRICommand {
        param($Executable,$Arguments,$WorkingDirectory,$LogPath,$TimeoutSeconds,$Guard,$Interactive=$false,$CancelPath='')
        & $Guard $true | Out-Null
        $script:Sim.SetupCalls++
        if ($script:Sim.CorruptSetup) { [IO.File]::WriteAllText((Join-Path $script:Sim.TargetRoot 'payload.dat'),'changed by fixture setup') }
        if ($script:Sim.AddOutput) { [IO.File]::WriteAllText((Join-Path $script:Sim.TargetRoot 'created-by-setup.dat'),'fixture generated output') }
        & $Guard $true | Out-Null
        return $script:Sim.SetupCode
    }
} $sim
function Reset-Case {
    $script:caseRoot=Join-Path $testRoot ([guid]::NewGuid().ToString('N'))
    $sim.TargetRoot=Join-Path $caseRoot 'Target'; [void][IO.Directory]::CreateDirectory($sim.TargetRoot)
    $sim.Inventory=@(Drive); $sim.NoPartitions=$false; $sim.Letter='R'; $sim.CopyCode=0; $sim.SetupCode=0; $sim.CorruptCopy=$false
    $sim.ProtectionPartitions=@(); $sim.ProtectionDisks=@()
    $sim.CorruptSetup=$false; $sim.RemoveDuringCopy=$false; $sim.SetupCalls=0; $sim.CopyCalls=0; $sim.FormatCalls=0
    $sim.FlushCalls=0; $sim.AddOutput=$false; $sim.FlushFails=$false; $sim.AddCopyExtra=$false
    $sim.AddFinalExtra=$false; $sim.HashCalls=0; $sim.PartitionError=$false; $sim.ActualPartitionStyle='RAW'
    $script:cfg=($baseConfig | ConvertTo-Json -Depth 6 | ConvertFrom-Json)
    $script:state=Join-Path $caseRoot 'state.json'; $script:receipt=Join-Path $caseRoot 'receipt.json'
    $script:cancel=Join-Path $caseRoot 'cancel'
}
function Build { param([string[]]$Protected=@(),$Record=$record)
    Invoke-MRIBuild $Record $cfg $manifest $Protected $state $receipt (Join-Path $caseRoot 'Logs') $cancel
}
try {
    Test 'Selected erase approval permits cleaning existing files only for the confirmed disk number' {
        Reset-Case; $cfg.PrepareDisk=$true; $cfg.SetupBatch=''
        [IO.File]::WriteAllText((Join-Path $sim.TargetRoot 'old.txt'),'owned old content')
        $result=Invoke-MRIBuild $record $cfg $manifest @() $state $receipt (Join-Path $caseRoot 'Logs') $cancel -ApprovedEraseNumber 6
        Check ($result.Success -and $sim.FormatCalls -eq 1 -and -not [IO.File]::Exists((Join-Path $sim.TargetRoot 'old.txt')))
    }
    Test 'Selected erase approval rejects a changed disk number before preparation' {
        Reset-Case; $cfg.PrepareDisk=$true; $cfg.SetupBatch=''
        $result=Invoke-MRIBuild $record $cfg $manifest @() $state $receipt (Join-Path $caseRoot 'Logs') $cancel -ApprovedEraseNumber 7
        Check (-not $result.Success -and $sim.FormatCalls -eq 0 -and $sim.CopyCalls -eq 0)
    }
    Test 'Selected erase approval cannot bypass a protected source disk' {
        Reset-Case; $cfg.PrepareDisk=$true; $cfg.SetupBatch=''
        $result=Invoke-MRIBuild $record $cfg $manifest @($record.Key) $state $receipt (Join-Path $caseRoot 'Logs') $cancel -ApprovedEraseNumber 6
        Check (-not $result.Success -and $sim.FormatCalls -eq 0)
    }
    Test 'PowerShell files parse without errors' {
        foreach ($f in @(Get-ChildItem $root -Include *.ps1,*.psm1 -Recurse)) {
            $errors=$null; $tokens=$null
            [void][Management.Automation.Language.Parser]::ParseFile($f.FullName,[ref]$tokens,[ref]$errors)
            Check ($errors.Count -eq 0) "$($f.Name) has parser errors."
        }
    }
    Test 'A mounted-folder source protects the USB partition instead of the parent drive' {
        $hostRoot=[IO.Path]::GetPathRoot($testRoot)
        $mountRoot=Join-Path $testRoot 'mounted-usb'
        [void][IO.Directory]::CreateDirectory($mountRoot)
        $usb=Drive 'MOUNTED-SOURCE' 8 'USB\MOUNTED-SOURCE'
        $sim.ProtectionPartitions=@(
            [pscustomobject]@{DiskNumber=0;AccessPaths=@($hostRoot)}
            [pscustomobject]@{DiskNumber=8;AccessPaths=@($mountRoot+[IO.Path]::DirectorySeparatorChar)}
        )
        $sim.ProtectionDisks=@([pscustomobject]@{Number=0;BusType='SATA'})
        $keys=@(Get-MRIProtectedKeys @((Join-Path $mountRoot 'source.zip')) @($usb))
        Check ($keys.Count -eq 1 -and $keys[0] -eq $usb.Key) 'The mounted USB identity was not protected.'
        Throws { Select-MRIUsbDrives '8' @($usb) $cfg $keys } 'contains the source files or the automation'
    }
    Test 'A volume GUID source protects its USB partition' {
        if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
            $volumeRoot='\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\'
            $usb=Drive 'VOLUME-GUID-SOURCE' 9 'USB\VOLUME-GUID-SOURCE'
            $sim.ProtectionPartitions=@(
                [pscustomobject]@{DiskNumber=0;AccessPaths=@([IO.Path]::GetPathRoot($testRoot))}
                [pscustomobject]@{DiskNumber=9;AccessPaths=@($volumeRoot)}
            )
            $sim.ProtectionDisks=@([pscustomobject]@{Number=0;BusType='SATA'})
            $keys=@(Get-MRIProtectedKeys @($volumeRoot+'source.iso') @($usb))
            Check ($keys.Count -eq 1 -and $keys[0] -eq $usb.Key) 'The volume GUID USB identity was not protected.'
        } else { Write-Host 'SKIP volume GUID normalization requires Windows.' }
    }
    Test 'An internal path that shares a parent root does not protect a mounted USB' {
        $hostRoot=[IO.Path]::GetPathRoot($testRoot)
        $mountRoot=Join-Path $testRoot 'mounted-usb'
        $usb=Drive 'MOUNTED-SOURCE' 8 'USB\MOUNTED-SOURCE'
        $sim.ProtectionPartitions=@(
            [pscustomobject]@{DiskNumber=0;AccessPaths=@($hostRoot)}
            [pscustomobject]@{DiskNumber=8;AccessPaths=@($mountRoot+[IO.Path]::DirectorySeparatorChar)}
        )
        $sim.ProtectionDisks=@([pscustomobject]@{Number=0;BusType='SATA'})
        $keys=@(Get-MRIProtectedKeys @((Join-Path $testRoot 'tool.ps1')) @($usb))
        Check ($keys.Count -eq 0) 'An unrelated USB partition was marked as the source disk.'
    }
    Test 'An unresolved path, ambiguous access path, or unidentified USB fails closed' {
        $unmappedRoot=Join-Path $testRoot 'unmapped-volume'
        $otherRoot=Join-Path $testRoot 'other-volume'
        $unmapped=Join-Path $unmappedRoot 'source.iso'
        $usb=Drive 'UNRESOLVED-SOURCE' 9 'USB\UNRESOLVED-SOURCE'
        $sim.ProtectionPartitions=@([pscustomobject]@{DiskNumber=0;AccessPaths=@($otherRoot+[IO.Path]::DirectorySeparatorChar)})
        $sim.ProtectionDisks=@([pscustomobject]@{Number=0;BusType='SATA'})
        Throws { Get-MRIProtectedKeys @($unmapped) @($usb) } 'Could not map'
        $sim.ProtectionPartitions=@(
            [pscustomobject]@{DiskNumber=9;AccessPaths=@($unmappedRoot+[IO.Path]::DirectorySeparatorChar)}
            [pscustomobject]@{DiskNumber=10;AccessPaths=@($unmappedRoot+[IO.Path]::DirectorySeparatorChar)}
        )
        Throws { Get-MRIProtectedKeys @($unmapped) @($usb) } 'multiple partitions'
        $sim.ProtectionPartitions=@([pscustomobject]@{DiskNumber=9;AccessPaths=@($unmappedRoot+[IO.Path]::DirectorySeparatorChar)})
        $badUsb=Drive 'UNRESOLVED-SOURCE' 9 'USB\UNRESOLVED-SOURCE'; $badUsb.Key=''
        Throws { Get-MRIProtectedKeys @($unmapped) @($badUsb) } 'usable hardware identity'
        $sim.ProtectionDisks=@([pscustomobject]@{Number=9;BusType='USB'})
        Throws { Get-MRIProtectedKeys @($unmapped) @() } 'usable hardware identity'
        $sim.ProtectionPartitions=@([pscustomobject]@{DiskNumber=0;AccessPaths=@([IO.Path]::GetPathRoot($testRoot))})
        $sim.ProtectionDisks=@([pscustomobject]@{Number=0;BusType='Unknown'})
        Throws { Get-MRIProtectedKeys @((Join-Path $testRoot 'source.iso')) @() } 'physical disk identity'
    }
    Test 'A changed source mount is rechecked against the selected disk before erase' {
        $mountRoot=Join-Path $testRoot 'source-remapped-to-target'
        $target=Drive 'SOURCE-REMAPPED' 12 'USB\SOURCE-REMAPPED'
        $sim.Inventory=@($target)
        $sim.ProtectionPartitions=@(
            [pscustomobject]@{DiskNumber=0;AccessPaths=@([IO.Path]::GetPathRoot($testRoot))}
            [pscustomobject]@{DiskNumber=12;AccessPaths=@($mountRoot+[IO.Path]::DirectorySeparatorChar)}
        )
        $sim.ProtectionDisks=@([pscustomobject]@{Number=0;BusType='SATA'})
        Throws { Assert-MRIPathProtection $target @((Join-Path $mountRoot 'source.iso')) } 'now contains a source'
    }
    Test 'Fresh erase checks retain source, application, input, and working-data paths' {
        Reset-Case
        $inputArchive=Join-Path $testRoot 'source archive.zip'
        $dataPath=Join-Path $caseRoot 'Data'
        $cfg.SourcePath=Join-Path $dataPath 'Sources/cache'
        $cfg | Add-Member -NotePropertyName InputStamp -NotePropertyValue ([pscustomobject]@{Path=$inputArchive}) -Force
        $work=Join-Path $caseRoot 'Work'; $statePath=Join-Path $work 'state.json'
        $receiptPath=Join-Path $caseRoot 'Data/Receipts/receipt.json'
        $extraPath=Join-Path $testRoot 'original-input.iso'
        $paths=@(Get-MRIResolvedProtectedPaths $cfg @($extraPath,$dataPath) $work $statePath $receiptPath)
        foreach ($expected in @($inputArchive,$extraPath,$dataPath,$work,$statePath,$receiptPath,
                ([IO.Path]::GetDirectoryName($receiptPath)),([IO.Path]::GetDirectoryName([IO.Path]::GetDirectoryName($receiptPath))))) {
            Check ($paths -contains [IO.Path]::GetFullPath($expected)) "A live protected path was lost: $expected"
        }
        Check (-not ($paths -contains [IO.Path]::GetFullPath($cfg.SourcePath))) 'An archive cache escaped the protected data directory.'
        Check ($paths -contains [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))) 'The application directory was not protected.'
    }
    Test 'C# interop source compiles' { Check ($null -ne ('MRIBatch.NativeProcess' -as [type])) }
    Test 'Source configuration validates' { Assert-MRIConfig $baseConfig }
    Test 'Missing source batch is rejected' { Reset-Case; $cfg.SetupBatch='missing.bat'; Throws { Assert-MRIConfig $cfg } 'not found' }
    Test 'CMD metacharacters in batch arguments are rejected' { Reset-Case; $cfg.SetupArguments=@('& erase'); Throws { Assert-MRIConfig $cfg } 'argument' }
    Test 'Traversal and absolute relative paths are rejected' { foreach ($p in @('..\file','C:\file','\file','dir\\file','dir\..\file')) { Throws { ConvertTo-MRIRelativePath $p } } }
    Test 'Blank or unusable serial numbers are rejected' { foreach ($s in @('','0000','UNKNOWN')) { Throws { Get-MRIDriveKey $s 'PNP' 'USB' 32 } } }
    Test 'Identity includes hardware serial, PNP, model, and size' {
        $a=Get-MRIDriveKey 'A' 'P' 'M' 32
        foreach ($b in @((Get-MRIDriveKey 'B' 'P' 'M' 32),(Get-MRIDriveKey 'A' 'Q' 'M' 32),(Get-MRIDriveKey 'A' 'P' 'N' 32),(Get-MRIDriveKey 'A' 'P' 'M' 33))) { Check ($a -ne $b) }
    }
    Test 'Boot and system drives are protected' { Reset-Case; $a=Drive; $a.IsBoot=$true; Check ([bool](Get-MRIDriveBlockReason $a @($a) $cfg)); $a.IsBoot=$false; $a.IsSystem=$true; Check ([bool](Get-MRIDriveBlockReason $a @($a) $cfg)) }
    Test 'Read-only and offline drives are protected' { Reset-Case; $a=Drive; $a.IsReadOnly=$true; Check ([bool](Get-MRIDriveBlockReason $a @($a) $cfg)); $a.IsReadOnly=$false; $a.IsOffline=$true; Check ([bool](Get-MRIDriveBlockReason $a @($a) $cfg)) }
    Test 'Source and tool disks are protected' { Reset-Case; Check ([bool](Get-MRIDriveBlockReason $d @($d) $cfg @($d.Key))) }
    Test 'Wrong capacity is rejected' { Reset-Case; $cfg.MaximumDiskBytes=64000000000; $a=Drive; $a.Size=1000000000000; Check ([bool](Get-MRIDriveBlockReason $a @($a) $cfg)) }
    Test 'Duplicate attached serials are rejected' { Reset-Case; $b=Drive 'STORE-A' 7 'OTHER-PNP'; Check ([bool](Get-MRIDriveBlockReason $d @($d,$b) $cfg)) }
    Test 'Duplicate serial in registry is rejected' {
        $r=($record | ConvertTo-Json | ConvertFrom-Json); $r.PnpId='OTHER-PNP'; $r.Key=Get-MRIDriveKey $r.Serial $r.PnpId $r.Model $r.Size; $r.Label='MRI_02'
        Throws { Assert-MRIRegistry @($record,$r) } 'Duplicate'
    }
    Test 'Changed disk number is rejected' { Reset-Case; $sim.Inventory[0].Number=7; Throws { Assert-MRIDrive $record $cfg @() 6 } 'number changed' }
    Test 'Replacement USB is rejected even if it reuses the disk number' { Reset-Case; $sim.Inventory=@(Drive 'CLIENT-B' 6 'CLIENT-PNP'); Throws { Assert-MRIDrive $record $cfg } 'removed|replaced' }
    Test 'Volume or letter remapping is rejected' { Reset-Case; $old=[pscustomobject]@{Number=6;Path=$sim.TargetRoot;Letter='Q'}; Throws { Assert-MRITarget $record $cfg $old } 'mapping changed' }
    Test 'Unknown drives are not queued' { Reset-Case; $t=@{}; $cfg.StablePolls=2; Check (@(Update-MRIArrivals $t @($d) @() $cfg).Count -eq 0); Check (@(Update-MRIArrivals $t @($d) @() $cfg).Count -eq 0) }
    Test 'Registered drive queues only after stable detection' { Reset-Case; $t=@{}; $cfg.StablePolls=2; Check (@(Update-MRIArrivals $t @($d) @($record) $cfg).Count -eq 0); Check (@(Update-MRIArrivals $t @($d) @($record) $cfg).Count -eq 1) }
    Test 'A drive is not retried repeatedly while attached' { Reset-Case; $t=@{}; $cfg.StablePolls=2; Update-MRIArrivals $t @($d) @($record) $cfg | Out-Null; Update-MRIArrivals $t @($d) @($record) $cfg | Out-Null; $t[$d.Key].Attempted=$true; Check (@(Update-MRIArrivals $t @($d) @($record) $cfg).Count -eq 0) }
    Test 'Disconnect clears the attachment tracker' { Reset-Case; $t=@{}; Update-MRIArrivals $t @($d) @($record) $cfg | Out-Null; Update-MRIArrivals $t @() @($record) $cfg | Out-Null; Check ($t.Count -eq 0) }
    Test 'CMD arguments expand placeholders without command injection' { Reset-Case; $cfg.SetupArguments=@('{DriveLetter}:','{Label}'); $v=[pscustomobject]@{Letter='R';Number=6}; Check ((Get-MRISetupArguments $cfg $record $v) -eq '/d /s /c ""R:\mri.bat" "R:" "MRI_01""') }
    Test 'Native argument quoting protects spaces and trailing backslashes' { Check ([MRIBatch.NativeProcess]::Quote('C:\MRI files\') -eq '"C:\MRI files\\"') }
    Test 'Atomic JSON write replaces existing files correctly' { $p=Join-Path $testRoot 'atomic.json'; Write-MRIJson $p @{Value=1}; Write-MRIJson $p @{Value=2}; Check ((Read-MRIJson $p).Value -eq 2) }
    Test 'Successful build copies, runs setup, verifies, flushes, and saves receipt' {
        Reset-Case; $r=Build; Check $r.Success; Check ($sim.CopyCalls -eq 1 -and $sim.SetupCalls -eq 1 -and $sim.FlushCalls -eq 1)
        Check ((Read-MRIJson $state).Stage -eq 'READY'); Check ((Read-MRIJson $receipt).Status -eq 'READY')
        Check (-not [IO.File]::Exists((Join-Path $sim.TargetRoot 'state.json'))) 'Reports must stay off USB.'
    }
    Test 'Nonempty unknown content blocks preparation without reading/erasing it' {
        Reset-Case; $cfg.PrepareDisk=$true; [IO.File]::WriteAllText((Join-Path $sim.TargetRoot 'client.doc'),'private fixture')
        $r=Build; Check (-not $r.Success); Check ($sim.FormatCalls -eq 0 -and $sim.CopyCalls -eq 0); Check ([IO.File]::ReadAllText((Join-Path $sim.TargetRoot 'client.doc')) -eq 'private fixture')
    }
    Test 'Copy failure never launches setup or creates a success receipt' { Reset-Case; $sim.CopyCode=8; $r=Build; Check (-not $r.Success); Check ($sim.SetupCalls -eq 0 -and -not [IO.File]::Exists($receipt)) }
    Test 'Corrupt copied file is detected before setup' { Reset-Case; $sim.CorruptCopy=$true; $r=Build; Check (-not $r.Success); Check ($sim.SetupCalls -eq 0); Check ($r.Error -match 'mismatch') }
    Test 'USB removal during copying stops the build' { Reset-Case; $sim.RemoveDuringCopy=$true; $r=Build; Check (-not $r.Success); Check ($sim.SetupCalls -eq 0) }
    Test 'Batch nonzero exit never marks media ready' { Reset-Case; $sim.SetupCode=2; $r=Build; Check (-not $r.Success); Check (-not [IO.File]::Exists($receipt)) }
    Test 'Unexpected post-setup modifications fail final verification' { Reset-Case; $sim.CorruptSetup=$true; $r=Build; Check (-not $r.Success); Check ($r.Error -match 'mismatch') }
    Test 'Only explicitly mutable source files may change during setup' { Reset-Case; $sim.CorruptSetup=$true; $cfg.MutableFiles=@('payload.dat'); $r=Build; Check $r.Success }
    Test 'Missing required setup output fails completion' { Reset-Case; $cfg.RequiredFiles=@('missing-output.bin'); $r=Build; Check (-not $r.Success); Check (-not [IO.File]::Exists($receipt)) }
    Test 'Flush failure prevents a ready receipt' { Reset-Case; $sim.FlushFails=$true; $r=Build; Check (-not $r.Success); Check (-not [IO.File]::Exists($receipt)) }
    Test 'Cancellation prevents copy and setup' { Reset-Case; [IO.File]::WriteAllText($cancel,'cancel'); $r=Build; Check (-not $r.Success); Check ($sim.CopyCalls -eq 0 -and $sim.SetupCalls -eq 0) }
    Test 'Protected source disk cannot reach formatting or copying' { Reset-Case; $cfg.PrepareDisk=$true; $r=Build @($record.Key); Check (-not $r.Success); Check ($sim.FormatCalls -eq 0 -and $sim.CopyCalls -eq 0) }
    Test 'Automatic preparation needs per-drive erase registration' { Reset-Case; $cfg.PrepareDisk=$true; $r=($record | ConvertTo-Json | ConvertFrom-Json); $r.AllowErase=$false; $result=Build -Record $r; Check (-not $result.Success); Check ($sim.FormatCalls -eq 0) }
    Test 'Preparation runs only for a specifically registered empty drive' { Reset-Case; $cfg.PrepareDisk=$true; $r=Build; Check $r.Success; Check ($sim.FormatCalls -eq 1) }
    Test 'Successful unchanged drive is verified and skipped on reinsertion' { Reset-Case; $sim.AddOutput=$true; Check (Build).Success; $before=$sim.CopyCalls; $r=Build; Check ($r.Success -and $r.AlreadyReady); Check ($sim.CopyCalls -eq $before) }
    Test 'Added client file invalidates a previous ready receipt' { Reset-Case; Check (Build).Success; [IO.File]::WriteAllText((Join-Path $sim.TargetRoot 'new-client.txt'),'private fixture'); $r=Build; Check (-not $r.Success); Check ($sim.CopyCalls -eq 1) }
    Test 'Corrupted previous output cannot be erased or trusted' { Reset-Case; Check (Build).Success; [IO.File]::WriteAllText((Join-Path $sim.TargetRoot 'payload.dat'),'X'*31); $cfg.PrepareDisk=$true; $r=Build; Check (-not $r.Success); Check ($sim.FormatCalls -eq 0) }
    Test 'Recipe changes require fresh media in copy-only mode' { Reset-Case; Check (Build).Success; $cfg.SetupArguments=@('new-mode'); $r=Build; Check (-not $r.Success); Check ($sim.CopyCalls -eq 1) }
    Test 'FAT32 oversized source file is rejected before copy' {
        Reset-Case; $large=($manifest | ConvertTo-Json -Depth 8 | ConvertFrom-Json); $large.Entries[0].IsDirectory=$false; $large.Entries[0].Length=[long]4294967296; $large.TotalBytes=[long]4294967296
        $r=Invoke-MRIBuild $record $cfg $large @() $state $receipt (Join-Path $caseRoot 'Logs') $cancel
        Check (-not $r.Success); Check ($sim.CopyCalls -eq 0)
    }
    Test 'Insufficient free space fails before writing' {
        Reset-Case; $huge=($manifest | ConvertTo-Json -Depth 8 | ConvertFrom-Json); $huge.TotalBytes=[long]40000000000
        $r=Invoke-MRIBuild $record $cfg $huge @() $state $receipt (Join-Path $caseRoot 'Logs') $cancel
        Check (-not $r.Success); Check ($sim.CopyCalls -eq 0)
    }
    Test 'An emptied drive cannot reuse a stale ready receipt' {
        Reset-Case; Check (Build).Success
        Get-ChildItem -LiteralPath $sim.TargetRoot -Force | Remove-Item -Recurse -Force
        $r=Build; Check ($r.Success -and -not $r.AlreadyReady); Check ($sim.CopyCalls -eq 2 -and $sim.SetupCalls -eq 2)
    }
    Test 'Failed refresh invalidates the old ready receipt before changing media' {
        Reset-Case; $cfg.PrepareDisk=$true; Check (Build).Success
        $cfg.SetupArguments=@('changed-recipe'); $sim.SetupCode=1
        $r=Build; Check (-not $r.Success); Check (-not [IO.File]::Exists($receipt))
    }
    Test 'Unexpected copied paths prevent setup' {
        Reset-Case; $sim.AddCopyExtra=$true; $r=Build
        Check (-not $r.Success); Check ($sim.SetupCalls -eq 0); Check ($r.Error -match 'absent|mismatch')
    }
    Test 'Removal of a source USB prevents a build' {
        Reset-Case; $r=Build @('SOURCE-USB-NO-LONGER-PRESENT')
        Check (-not $r.Success); Check ($sim.CopyCalls -eq 0 -and $sim.FormatCalls -eq 0)
    }
    Test 'Direct copy works without a setup batch' {
        Reset-Case; $cfg.SetupBatch=''; $cfg.SetupArguments=@()
        $r=Build; Check $r.Success; Check ($sim.SetupCalls -eq 0 -and $sim.CopyCalls -eq 1)
    }
    Test 'Cancellation before a raw-drive build prevents formatting' {
        Reset-Case; $cfg.PrepareDisk=$true; $sim.NoPartitions=$true
        [IO.File]::WriteAllText($cancel,'cancel')
        $r=Build; Check (-not $r.Success); Check ($sim.FormatCalls -eq 0 -and $sim.CopyCalls -eq 0) 'Cancelled raw drive was formatted.'
    }
    Test 'Missing source boot files block direct-copy preparation before erasure' {
        Reset-Case; $cfg.SetupBatch=''; $cfg.PrepareDisk=$true; $cfg.RequiredFiles=@('missing-efi.bin')
        $r=Build; Check (-not $r.Success); Check ($sim.FormatCalls -eq 0 -and $sim.CopyCalls -eq 0)
    }
    Test 'Direct-copy mode rejects mutable-file and setup-argument exceptions' {
        Reset-Case; $cfg.SetupBatch=''; $cfg.MutableFiles=@('payload.dat'); Throws { Assert-MRIConfig $cfg } 'Direct-copy'
        $cfg.MutableFiles=@(); $cfg.SetupArguments=@('unexpected'); Throws { Assert-MRIConfig $cfg } 'Direct-copy'
    }
    Test 'Fractional and string timing/concurrency values are rejected' {
        foreach ($value in @(1.5,'2',$true)) { Reset-Case; $cfg.MaxParallel=$value; Throws { Assert-MRIConfig $cfg } 'integer' }
    }
    Test 'Negative copy exit cannot create a ready receipt' {
        Reset-Case; $sim.CopyCode=-1; $r=Build; Check (-not $r.Success); Check (-not [IO.File]::Exists($receipt))
    }
    Test 'Ready and failed totals remain mutually exclusive for a drive' {
        $ready=@{}; $failed=@{}
        Set-MRIOutcome $ready $failed 'A' 'MRI_01' $true
        Set-MRIOutcome $ready $failed 'A' 'MRI_01' $false
        Check ($ready.Count -eq 0 -and $failed.Count -eq 1)
        Set-MRIOutcome $ready $failed 'A' 'MRI_01' $true
        Check ($ready.Count -eq 1 -and $failed.Count -eq 0)
    }
    Test 'A source archive changing during a session is rejected' {
        Reset-Case; $p=Join-Path $caseRoot 'source.iso'; [IO.File]::WriteAllText($p,'fixture image')
        $f=Get-Item -LiteralPath $p
        $cfg | Add-Member -NotePropertyName InputStamp -NotePropertyValue ([pscustomobject]@{Path=$p;Length=$f.Length;WriteTicks=$f.LastWriteTimeUtc.Ticks})
        Assert-MRIInputSource $cfg
        [IO.File]::AppendAllText($p,'changed')
        Throws { Assert-MRIInputSource $cfg } 'changed'
    }
    Test 'Concurrency accepts 1 through 30 and rejects invalid limits' {
        foreach ($limit in @(1,4,30)) { Reset-Case; $cfg.MaxParallel=$limit; Assert-MRIConfig $cfg }
        foreach ($limit in @(0,31,-1)) { Reset-Case; $cfg.MaxParallel=$limit; Throws { Assert-MRIConfig $cfg } '1 through 30' }
    }
    Test 'Thirty stable registered drives fill all four slots and drain without duplicates' {
        Reset-Case; $cfg.MaxParallel=4; $cfg.StablePolls=2
        $drives=@(1..30 | ForEach-Object { Drive ('DRIVE-'+$_) $_ ('USB\\'+$_) })
        $records=@($drives | ForEach-Object { [pscustomobject]@{Key=$_.Key;Label=('MRI_{0:D2}' -f $_.Number)} })
        $tracker=@{}; $jobs=@{}; $processed=@{}
        Check (@(Update-MRIArrivals $tracker $drives $records $cfg).Count -eq 0)
        while ($processed.Count -lt 30) {
            $arrivals=@(Update-MRIArrivals $tracker $drives $records $cfg)
            $selected=@(Select-MRIBuilds $arrivals $jobs $cfg.MaxParallel)
            Check ($selected.Count -eq [Math]::Min(4,30-$processed.Count))
            foreach ($r in $selected) {
                Check (-not $processed.ContainsKey($r.Key)); $processed[$r.Key]=$true
                $tracker[$r.Key].Attempted=$true; $jobs[$r.Key]=$true
            }
            Check ($jobs.Count -le 4)
            Check (@(Select-MRIBuilds $arrivals $jobs 4).Count -eq 0 -or $jobs.Count -lt 4)
            $jobs.Clear()
        }
        Check (@(Update-MRIArrivals $tracker $drives $records $cfg).Count -eq 0)
    }
    Test 'Operational setting changes preserve verified receipts without copying again' {
        Reset-Case; Check (Build).Success
        $cfg.MaxParallel=30; $cfg.PollSeconds=5; $cfg.StablePolls=4; $cfg.CopyTimeoutSeconds=7200; $cfg.SerializeSetup=$false
        $r=Build; Check ($r.Success -and $r.AlreadyReady); Check ($sim.CopyCalls -eq 1)
    }
    Test 'Direct-copy media is hashed once after flushing and still detects corruption' {
        Reset-Case; $cfg.SetupBatch=''; $r=Build; Check $r.Success
        Check ($sim.HashCalls -eq @($manifest.Entries | Where-Object { -not $_.IsDirectory }).Count)
        Reset-Case; $cfg.SetupBatch=''; $sim.CorruptCopy=$true; $r=Build
        Check (-not $r.Success); Check (-not [IO.File]::Exists($receipt))
    }
    Test 'An unexpected file appearing before final verification cannot receive a direct-copy receipt' {
        Reset-Case; $cfg.SetupBatch=''; $sim.AddFinalExtra=$true; $r=Build
        Check (-not $r.Success); Check (-not [IO.File]::Exists($receipt))
    }
    Test 'A matching old direct-copy receipt migrates after a concurrency change' {
        Reset-Case; $cfg.SetupBatch=''; Check (Build).Success
        $old=Read-MRIJson $receipt; $old.PSObject.Properties.Remove('RecipeVersion'); $old.Recipe=Get-MRIHash 'old-full-settings-recipe'
        Write-MRIJson $receipt $old; $cfg.MaxParallel=30
        $r=Build; Check ($r.Success -and $r.AlreadyReady); Check ($sim.CopyCalls -eq 1)
        Check ((Read-MRIJson $receipt).RecipeVersion -eq 2)
    }
    Test 'The Windows no-partitions error permits preparation only when the live disk is RAW' {
        Reset-Case; $cfg.PrepareDisk=$true; $sim.PartitionError=$true
        $r=Build; Check $r.Success; Check ($sim.FormatCalls -eq 1)
        Reset-Case; $cfg.PrepareDisk=$true; $sim.PartitionError=$true; $sim.ActualPartitionStyle='MBR'
        $r=Build; Check (-not $r.Success); Check ($sim.FormatCalls -eq 0 -and $sim.CopyCalls -eq 0)
    }
    Test 'An empty source cannot reach preparation even with no required filenames' {
        Reset-Case; $cfg.SetupBatch=''; $cfg.RequiredFiles=@(); $cfg.PrepareDisk=$true
        $empty=[pscustomobject]@{Entries=@();TotalBytes=0;Digest='empty'}
        $r=Invoke-MRIBuild $record $cfg $empty @() $state $receipt (Join-Path $caseRoot 'Logs') $cancel
        Check (-not $r.Success); Check ($sim.FormatCalls -eq 0 -and $sim.CopyCalls -eq 0)
    }
    Write-Host "`n$($count-$failures.Count)/$count tests passed. All storage/process APIs in build tests were simulated."
    if ($failures.Count) { $failures | ForEach-Object { Write-Host $_ }; exit 1 }
} finally { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue }
exit 0
