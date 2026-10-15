#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-14-2026. Only newly created, explicitly bound disposable VHDs.
# USB inventory/classification is simulated. Preparation, copy workers,
# GUID destinations, process overlap, hashes and flushing use the real backend.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Windows is required.' }
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
Initialize-MRINative
$temp=Join-Path ([IO.Path]::GetTempPath()) ('mri-parallel-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$images=[Collections.Generic.List[string]]::new(); $jobs=@{}; $fixtures=@(); $checks=0
function Check { param([bool]$Condition,[string]$Message='Assertion failed'); if (-not $Condition) { throw $Message } }
function Pass { param([string]$Name); $script:checks++; Write-Host "PASS $Name" }
try {
    $source=Join-Path $temp 'Source files & spaces'
    [void][IO.Directory]::CreateDirectory((Join-Path $source 'efi/boot'))
    [IO.File]::WriteAllText((Join-Path $source 'efi/boot/bootx64.efi'),'not a bootable binary; owned fixture')
    $stream=[IO.File]::Create((Join-Path $source 'payload.bin'))
    try { $stream.SetLength(32MB) } finally { $stream.Dispose() }
    $cfg=Read-MRIJson (Join-Path $root 'mri-config.example.json')
    $cfg.SourcePath=$source; $cfg.SourceKind='Folder'; $cfg.PrepareDisk=$true; $cfg.MinimumDiskBytes=1
    $manifest=Get-MRIManifest $source
    foreach ($i in 1..4) {
        $image=Join-Path $temp ('owned-'+$i+'.vhd')
        $diskpart=Join-Path $temp ('create-'+$i+'.txt')
        [IO.File]::WriteAllText($diskpart,('create vdisk file="'+$image+'" maximum=512 type=expandable'+[Environment]::NewLine))
        $output=& (Join-Path $env:SystemRoot 'System32/diskpart.exe') /s $diskpart
        if ($LASTEXITCODE -ne 0 -or -not [IO.File]::Exists($image)) { throw "Owned VHD creation failed: $output" }
        $mounted=Mount-DiskImage -ImagePath $image -StorageType VHD -Access ReadWrite -PassThru -ErrorAction Stop
        $images.Add($image)
        $disks=@($mounted | Get-Disk -ErrorAction Stop)
        Check ($disks.Count -eq 1) 'An owned fixture must resolve to one disk.'
        $disk=$disks[0]
        Check (-not $disk.IsBoot -and -not $disk.IsSystem -and [string]$disk.BusType -eq 'File Backed Virtual') 'Only a newly created file-backed fixture may be used.'
        $serial='OWNED-VHD-'+$i
        $drive=[pscustomobject]@{Key=(Get-MRIDriveKey $serial $image 'OWNED VHD' $disk.Size);Number=[int]$disk.Number
            Serial=$serial;PnpId=$image;Model='OWNED VHD';Size=[long]$disk.Size;IsBoot=$false;IsSystem=$false
            IsReadOnly=$false;IsOffline=$false;BusType='USB';IdentityError=''}
        $record=[pscustomobject]@{Key=$drive.Key;Serial=$serial;PnpId=$image;Model=$drive.Model;Size=$drive.Size;Label=('MRI_{0:D2}' -f $i);AllowErase=$true}
        $fixtures+=([pscustomobject]@{Image=$image;Drive=$drive;Record=$record})
    }
    $app=Join-Path $temp 'Fixture App'
    [void][IO.Directory]::CreateDirectory($app)
    Copy-Item -LiteralPath (Join-Path $root 'NativeProcess.cs') -Destination $app
    # Inject classification solely into a temporary copy, not production code.
    $moduleText=[IO.File]::ReadAllText((Join-Path $root 'MRI.Batch.psm1'))+@'

$script:OwnedFixtures=@(Read-MRIJson (Join-Path $PSScriptRoot 'fixtures.json'))
function Get-MRIInventory { foreach ($fixture in $script:OwnedFixtures) { $fixture.Drive } }
function Get-MRIWritableDisk {
    param($Record,$Config,[string[]]$ProtectedKeys,[int]$ExpectedNumber,[string]$CancelPath)
    Assert-MRINotCancelled $CancelPath
    [void](Assert-MRIDrive $Record $Config $ProtectedKeys $ExpectedNumber)
    $matched=@($script:OwnedFixtures | Where-Object { $_.Record.Key -eq $Record.Key })
    if ($matched.Count -ne 1) { throw 'Not an owned fixture.' }
    $disks=@(Get-DiskImage -ImagePath $matched[0].Image -ErrorAction Stop | Get-Disk -ErrorAction Stop)
    if ($disks.Count -ne 1 -or $disks[0].Number -ne $ExpectedNumber -or $disks[0].IsBoot -or $disks[0].IsSystem -or
        [string]$disks[0].BusType -ne 'File Backed Virtual' -or $disks[0].IsReadOnly -or $disks[0].IsOffline) { throw 'The owned VHD changed or is protected.' }
    Assert-MRINotCancelled $CancelPath
    return $disks[0]
}
Export-ModuleMember -Function *-MRI*
'@
    $modulePath=Join-Path $app 'MRI.Batch.psm1'
    [IO.File]::WriteAllText($modulePath,$moduleText)
    Write-MRIJson (Join-Path $app 'fixtures.json') $fixtures
    # A test-only barrier proves all four actual copy processes are alive at
    # once before their native copies begin. It never enters the download.
    $workerText=[IO.File]::ReadAllText((Join-Path $root 'Copy-MRIFiles.ps1'))
    $barrierText=@'
    $barrier=Join-Path (Split-Path (Split-Path $PlanPath -Parent) -Parent) 'copy-barrier'
    $label=Split-Path (Split-Path $PlanPath -Parent) -Leaf
    Write-MRIJson (Join-Path $barrier ($label+'-started.json')) ([pscustomobject]@{Pid=$PID;StartedTicks=[datetime]::UtcNow.Ticks})
    $clock=[Diagnostics.Stopwatch]::StartNew()
    while (@(Get-ChildItem -LiteralPath $barrier -Filter '*-started.json').Count -lt 4) {
        if ($clock.Elapsed.TotalSeconds -gt 45) { throw 'Four copy workers did not overlap.' }
        Start-Sleep -Milliseconds 50
    }
    $files=0
'@
    $workerText=$workerText.Replace('    $files=0',$barrierText)
    $workerText=$workerText.Replace('    exit 0',"    Write-MRIJson (Join-Path `$barrier (`$label+'-finished.json')) ([pscustomobject]@{FinishedTicks=[datetime]::UtcNow.Ticks})`n    exit 0")
    [IO.File]::WriteAllText((Join-Path $app 'Copy-MRIFiles.ps1'),$workerText)
    $configPath=Join-Path $temp 'config.json'; $manifestPath=Join-Path $temp 'manifest.json'
    $configHash=Write-MRIJson $configPath $cfg -PassThruHash
    $manifestHash=Write-MRIJson $manifestPath $manifest -PassThruHash
    $workRoot=Join-Path $temp 'Work'; $barrier=Join-Path $workRoot 'copy-barrier'
    [void][IO.Directory]::CreateDirectory($barrier)
    foreach ($record in @(Select-MRIBuilds @($fixtures | ForEach-Object { $_.Record }) $jobs 4)) {
        $work=Join-Path $workRoot $record.Label
        $job=Start-MRIBuildJob $modulePath $record $configPath $configHash $manifestPath $manifestHash @() (Join-Path $work 'state.json') (Join-Path $work 'receipt.json') $work (Join-Path $temp 'cancel')
        $jobs[$record.Key]=$job
    }
    Check ($jobs.Count -eq 4) 'The scheduler did not fill four slots.'
    $jobs.Values | Wait-Job -Timeout 100 | Out-Null
    foreach ($fixture in $fixtures) {
        $job=$jobs[$fixture.Record.Key]
        Check ($job.State -eq 'Completed') 'Parallel preparation/copy did not finish.'
        $outcomes=@(Receive-Job -Job $job -ErrorAction Stop | Where-Object { $null -ne $_.PSObject.Properties['Success'] })
        if ($outcomes.Count -ne 1 -or -not $outcomes[0].Success) {
            $copyLog=Join-Path (Join-Path $workRoot $fixture.Record.Label) 'copy.log'
            if ([IO.File]::Exists($copyLog)) { Write-Host ([IO.File]::ReadAllText($copyLog)) }
            throw ('A parallel build failed: '+($outcomes | ConvertTo-Json -Depth 6))
        }
    }
    Pass 'Four real background builds prepare FAT32 and copy verified files to separate owned VHD roots'
    $started=@(Get-ChildItem -LiteralPath $barrier -Filter '*-started.json' | ForEach-Object { Read-MRIJson $_.FullName })
    $finished=@(Get-ChildItem -LiteralPath $barrier -Filter '*-finished.json' | ForEach-Object { Read-MRIJson $_.FullName })
    Check ($started.Count -eq 4 -and $finished.Count -eq 4)
    Check (@($started.Pid | Select-Object -Unique).Count -eq 4)
    Check (($started.StartedTicks | Measure-Object -Maximum).Maximum -lt ($finished.FinishedTicks | Measure-Object -Minimum).Minimum)
    Pass 'All four actual native copy-worker process lifetimes overlap'
    Import-Module $modulePath -Force -DisableNameChecking
    foreach ($fixture in $fixtures) {
        $target=Get-MRITargetVolume $fixture.Drive -RequireDriveLetter $false
        Check (-not $target.Letter) 'A direct-copy fixture unexpectedly consumed a drive letter.'
        Check ($target.FileSystem -eq 'FAT32')
        Check ([IO.File]::Exists((Join-MRIPath $target.Path 'payload.bin'))) 'Payload was not copied to the drive root.'
        Assert-MRIManifest $manifest $target.Path
    }
    Pass 'FAT32 copies and hashes work without assigning drive letters'
    $cfg.MaxParallel=30
    foreach ($fixture in $fixtures) {
        $work=Join-Path $workRoot $fixture.Record.Label
        $result=Invoke-MRIBuild $fixture.Record $cfg $manifest @() (Join-Path $work 'state.json') (Join-Path $work 'receipt.json') $work (Join-Path $temp 'cancel')
        Check ($result.Success -and $result.AlreadyReady)
    }
    Pass 'Changing the real build concurrency does not format or copy verified media again'
    $first=Join-Path $workRoot $fixtures[0].Record.Label
    $planPath=Join-Path $first 'copy-plan.json'; $plan=Read-MRIJson $planPath
    $hash=Get-MRIHash ([IO.File]::ReadAllText($planPath))
    $plan.TargetRoot='\\?\Volume{11111111-1111-1111-1111-111111111111}\'; Write-MRIJson $planPath $plan
    $argsLine=(@('-NoLogo','-NoProfile','-ExecutionPolicy','RemoteSigned','-File',(Join-Path $app 'Copy-MRIFiles.ps1'),'-PlanPath',$planPath,'-ExpectedHash',$hash) | ForEach-Object { [MRIBatch.NativeProcess]::Quote($_) }) -join ' '
    $log=Join-Path $temp 'tampered-plan.log'
    $code=Invoke-MRICommand (Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe') $argsLine $temp $log 20 { } -CancelPath (Join-Path $temp 'cancel')
    Check ($code -ne 0 -and [IO.File]::ReadAllText($log).Contains('changed or is corrupt'))
    Pass 'The actual copy worker rejects a redirected plan before touching its target'
    # Exercise the actual finite menu-2 path as well as the lower-level jobs.
    # Give the first owned VHD two existing NTFS partitions with sentinel files.
    $fixture=$fixtures[0]
    $owned=Get-MRIWritableDisk $fixture.Record $cfg @() $fixture.Drive.Number ''
    Clear-Disk -InputObject $owned -RemoveData -RemoveOEM -Confirm:$false -ErrorAction Stop
    $owned=Get-MRIWritableDisk $fixture.Record $cfg @() $fixture.Drive.Number ''
    if ([string]$owned.PartitionStyle -eq 'RAW') { $owned=Initialize-Disk -InputObject $owned -PartitionStyle MBR -PassThru -ErrorAction Stop }
    foreach ($index in 1..2) {
        $part=New-Partition -InputObject $owned -Size 64MB -ErrorAction Stop
        $volume=$part | Format-Volume -FileSystem NTFS -Force -Confirm:$false -ErrorAction Stop
        [IO.File]::WriteAllText((Join-MRIPath $volume.Path ('old-'+$index+'.txt')),'owned disposable contents')
    }
    Copy-Item -LiteralPath (Join-Path $root 'MRI-Batch.ps1') -Destination $app
    Copy-Item -LiteralPath (Join-Path $root 'mri-config.example.json') -Destination $app
    Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination $app
    [IO.File]::AppendAllText($modulePath,@'

function Read-Host {
    param([string]$Prompt)
    $numbers=($script:OwnedFixtures.Drive.Number | Sort-Object) -join ','
    if ($Prompt -like 'Disk numbers to clean*') { return $numbers }
    if ($Prompt -like 'Type*') { return 'ERASE '+$numbers }
    throw ('Unexpected operator prompt: '+$Prompt)
}
Export-ModuleMember -Function '*-MRI*','Read-Host'
'@)
    $beforeConfig=(Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash
    $batchData=Join-Path $temp 'Selected batch data'
    $arguments=(@('-NoLogo','-NoProfile','-ExecutionPolicy','RemoteSigned','-File',(Join-Path $app 'MRI-Batch.ps1'),
        '-Action','Build','-ConfigPath',$configPath,'-DataPath',$batchData) | ForEach-Object { [MRIBatch.NativeProcess]::Quote($_) }) -join ' '
    $batchLog=Join-Path $temp 'selected-batch.log'
    $code=Invoke-MRICommand (Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe') $arguments $temp $batchLog 150 { } -CancelPath (Join-Path $temp 'cancel')
    $output=[IO.File]::ReadAllText($batchLog)
    if ($code -ne 0 -or -not $output.Contains('Batch finished: 4/4 verified; 0 failed or cancelled.')) { throw "Selected batch failed or did not finish: $output" }
    Check (-not [IO.File]::Exists((Join-Path $batchData 'drives.json'))) 'Selected builds must not create permanent erase registrations.'
    Check ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -eq $beforeConfig)
    foreach ($fixture in $fixtures) {
        $target=Get-MRITargetVolume $fixture.Drive -RequireDriveLetter $false
        Check ($target.FileSystem -eq 'FAT32' -and -not $target.Letter)
        $actual=Get-MRIManifest $target.Path -IgnoreSystemDirectories
        Assert-MRIManifestMatch $manifest $actual -Exact
    }
    Pass 'The real selected batch cleans existing multi-partition contents, builds four USB fixtures and exits without permanent approvals'
    Write-Host "`n$checks/$checks parallel Windows storage checks passed. Only owned VHD fixtures were modified."
} finally {
    try {
        foreach ($job in $jobs.Values) { Stop-Job -Job $job -ErrorAction SilentlyContinue; Remove-Job -Job $job -Force -ErrorAction SilentlyContinue }
    } finally {
        foreach ($image in $images) { Dismount-DiskImage -ImagePath $image -ErrorAction SilentlyContinue | Out-Null }
        Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
exit 0
