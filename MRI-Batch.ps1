#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# GPL-3.0-only. Selected USB media builder, updated 10-15-2026.
[CmdletBinding()]
param(
    [ValidateSet('Menu','Configure','Build','Enroll','List','Watch','SelfTest')][string]$Action='Menu',
    [string]$ConfigPath,
    [string]$DataPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
# Windows PowerShell 5.1 initializes $PSScriptRoot after parameter binding
# when an advanced script is launched with -File. Resolve defaults here.
if ([string]::IsNullOrWhiteSpace($ConfigPath)) {
    $ConfigPath=Join-Path $PSScriptRoot 'mri-config.json'
}
$ConfigPath=[IO.Path]::GetFullPath($ConfigPath)
if ([string]::IsNullOrWhiteSpace($DataPath)) { $DataPath=Join-Path $PSScriptRoot 'Data' }
$DataPath=[IO.Path]::GetFullPath($DataPath)
Import-Module (Join-Path $PSScriptRoot 'MRI.Batch.psm1') -Force -DisableNameChecking
$RegistryPath=Join-Path $DataPath 'drives.json'
function Assert-WindowsAdmin {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Drive operations require Windows 10/11 and Windows PowerShell 5.1.' }
    $principal=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Close this window, then right-click START-MRI-DRIVE-MAKER.cmd and choose Run as administrator.'
    }
}
function Get-Config {
    $cfg=Read-MRIJson $ConfigPath; Assert-MRIConfig $cfg; return $cfg
}
function Get-Records {
    if (-not [IO.File]::Exists($RegistryPath)) { return }
    $records=@(Read-MRIJson $RegistryPath); Assert-MRIRegistry $records; return $records
}
function Read-ParallelLimit {
    param([int]$Default)
    Write-Host 'Several USBs can copy and verify at once. Shared USB ports/hubs still share bandwidth.'
    $inputText=Read-Host "Simultaneous builds, 1-30 [$Default]"
    if ([string]::IsNullOrWhiteSpace($inputText)) { return $Default }
    $limit=0
    if (-not [int]::TryParse($inputText,[ref]$limit) -or $limit -lt 1 -or $limit -gt 30) { throw 'Choose a whole number from 1 through 30.' }
    return $limit
}
function Set-ParallelLimit {
    Assert-WindowsAdmin
    $cfg=Get-Config
    $cfg.MaxParallel=Read-ParallelLimit $cfg.MaxParallel
    Write-MRIJson $ConfigPath $cfg
    Write-Host "Saved: up to $($cfg.MaxParallel) simultaneous builds in the next session."
}
function Configure-Media {
    Assert-WindowsAdmin
    Write-Host '1 = select the extracted MRI folder (default).'
    Write-Host '2 = select an MRI ISO or ZIP file.'
    $choice=Read-Host 'Source [1]'
    if ($choice -and $choice -notin @('1','2')) { throw 'Choose 1 or 2.' }
    Add-Type -AssemblyName System.Windows.Forms
    if ($choice -ne '2') { $dialog=[Windows.Forms.FolderBrowserDialog]::new(); $dialog.Description='Select the folder whose contents belong at the USB root' }
    else { $dialog=[Windows.Forms.OpenFileDialog]::new(); $dialog.Title='Select the MRI ISO or ZIP'; $dialog.Filter='MRI ISO or ZIP|*.iso;*.zip'; $dialog.CheckFileExists=$true }
    try {
        if ($dialog.ShowDialog() -ne [Windows.Forms.DialogResult]::OK) { return }
        $source=$(if ($choice -ne '2') { $dialog.SelectedPath } else { $dialog.FileName })
    } finally { $dialog.Dispose() }
    $cfg=Read-MRIJson (Join-Path $PSScriptRoot 'mri-config.example.json')
    $cfg.SourcePath=$source
    $cfg.SourceKind=$(if ($choice -ne '2') { 'Folder' } else { [IO.Path]::GetExtension($source).TrimStart('.').ToUpperInvariant() })
    $cfg.PrepareDisk=$true
    Write-Host 'The selected contents will be copied directly to each USB root after FAT32 preparation.'
    $cfg.MaxParallel=Read-ParallelLimit $cfg.MaxParallel
    $opened=$null
    try {
        Write-Host 'Checking the source contents and FAT32 limits.'
        $opened=Open-MRISource $cfg (Join-Path $DataPath 'Sources')
        $probe=Get-MRIManifest $opened.Path -NoHash
        if (-not @($probe.Entries | Where-Object { -not $_.IsDirectory }).Count) { throw 'The source contains no files.' }
        foreach ($required in @($cfg.RequiredFiles)) {
            if (-not @($probe.Entries | Where-Object { $_.RelativePath -eq $required -and -not $_.IsDirectory -and $_.Length -gt 0 }).Count) { throw "The source does not contain the configured required file: $required" }
        }
        if (@($probe.Entries | Where-Object { -not $_.IsDirectory -and $_.Length -gt 4294967295 }).Count) { throw 'A source file exceeds the FAT32 file-size limit. The USB was not touched.' }
        Assert-MRIConfig $cfg
        Write-MRIJson $ConfigPath $cfg
    } finally { Close-MRISource $opened }
    Write-Host 'Source saved. Choose 2 to select USBs and build.'
}
function Show-Drives {
    param([array]$Inventory)
    Assert-WindowsAdmin
    if ($null -eq $Inventory) { $Inventory=@(Get-MRIInventory) }
    if (-not $inventory.Count) { Write-Host 'No USB disks detected.'; return }
    $inventory | ForEach-Object {
        $d=$_
        [pscustomobject]@{ Disk=$d.Number; Model=$d.Model; GB=[Math]::Round($d.Size/1e9,1)
            Serial=$d.Serial
            Status=$(if ($d.IdentityError) { $d.IdentityError } elseif ($d.IsBoot -or $d.IsSystem) { 'PROTECTED' } else { 'USB' }) }
    } | Format-Table -AutoSize | Out-Host
}
function Build-SelectedMedia {
    Assert-WindowsAdmin
    $cfg=Get-Config
    $inventory=@(Get-MRIInventory)
    $protected=@(Get-MRIProtectedKeys @($cfg.SourcePath,$PSScriptRoot,$DataPath) $inventory)
    Show-Drives -Inventory $inventory
    $selected=@(Select-MRIUsbDrives (Read-Host 'Disk numbers to clean and build, separated by commas (blank cancels)') $inventory $cfg $protected)
    if (-not $selected.Count) { return }
    $selected | Select-Object Number,Model,Serial,@{Name='GB';Expression={[Math]::Round($_.Size/1e9,1)}} | Format-Table -AutoSize | Out-Host
    $phrase='ERASE '+(($selected.Number | Sort-Object) -join ',')
    Write-Host 'ALL files and partitions on these selected USBs will be erased.'
    if ((Read-Host "Type '$phrase' to start this batch") -cne $phrase) { Write-Host 'Cancelled. No USB was changed.'; return }
    $records=@(); $index=0
    foreach ($drive in $selected) {
        # Bind the approval to both physical identity and the displayed number.
        [void](Assert-MRIDrive $drive $cfg $protected $drive.Number)
        $index++
        $records+=[pscustomobject]@{Key=$drive.Key;Serial=$drive.Serial;PnpId=$drive.PnpId;Model=$drive.Model
            Size=$drive.Size;Label=('MRI_{0:D2}' -f $index);AllowErase=$true;ExpectedNumber=[int]$drive.Number}
    }
    Watch-Media -SessionRecords $records
}
function Enroll-Drives {
    Assert-WindowsAdmin
    $cfg=Get-Config
    $records=@(Get-Records); $inventory=@(Get-MRIInventory)
    $protected=@(Get-MRIProtectedKeys @($cfg.SourcePath,$PSScriptRoot,$DataPath) $inventory)
    Show-Drives -Inventory $inventory
    Write-Host 'Select only the company MRI USB drives shown above. Unknown drives are never automatically enrolled.'
    $selection=Read-Host 'Disk numbers to register, separated by commas (blank cancels)'
    if (-not $selection) { return }
    if ($selection -notmatch '^\s*\d+(\s*,\s*\d+)*\s*$') { throw 'Enter comma-separated disk numbers.' }
    $selected=@($selection.Split(',') | ForEach-Object { [int]$_.Trim() } | Select-Object -Unique)
    foreach ($number in $selected) {
        $matches=@($inventory | Where-Object { $_.Number -eq $number })
        if ($matches.Count -ne 1) { throw "Disk $number was not in the displayed USB inventory." }
        $d=$matches[0]
        $reason=Get-MRIDriveBlockReason $d $inventory $cfg $protected
        if ($reason) { Write-Host "Disk ${number}: $reason" -ForegroundColor Red; continue }
        $existing=@($records | Where-Object { $_.Key -eq $d.Key })
        $label=$(if ($existing.Count) { $existing[0].Label } else {
            $i=1; while (@($records | Where-Object { $_.Label -eq ('MRI_{0:D2}' -f $i) }).Count) { $i++ }; 'MRI_{0:D2}' -f $i
        })
        if ($label -notmatch '^MRI_[0-9]{2,3}$') { throw 'The drive registry supports labels MRI_01 through MRI_999.' }
        Write-Host "Disk $number | $($d.Model) | $([Math]::Round($d.Size/1e9,1)) GB | serial $($d.Serial) | $label"
        $phrase=$(if ($cfg.PrepareDisk) { "ERASE $label" } else { "REGISTER $label" })
        if ($cfg.PrepareDisk) { Write-Host 'This grants this drive automatic erasure for media preparation. Unexpected files still block it.' }
        if ((Read-Host "Type '$phrase' to register this specific physical drive") -cne $phrase) { Write-Host 'Skipped.'; continue }
        # Re-query after operator input: a changed physical disk is never registered from stale display data.
        $fresh=@(Get-MRIInventory)
        $live=@($fresh | Where-Object { $_.Number -eq $number -and $_.Key -eq $d.Key })
        if ($live.Count -ne 1) { throw 'USB identity changed while registering. No registration was saved for this selection.' }
        $reason=Get-MRIDriveBlockReason $live[0] $fresh $cfg $protected
        if ($reason) { throw $reason }
        $other=@($records | Where-Object { $_.Serial -eq $d.Serial -and $_.Key -ne $d.Key })
        if ($other.Count) { throw 'This serial is already registered with a different physical identity.' }
        $record=[pscustomobject]@{ Key=$d.Key; Serial=$d.Serial; PnpId=$d.PnpId; Model=$d.Model; Size=$d.Size; Label=$label; AllowErase=[bool]$cfg.PrepareDisk }
        $records=@($records | Where-Object { $_.Key -ne $d.Key })+@($record)
        Assert-MRIRegistry $records; Write-MRIJson $RegistryPath $records
        Write-Host "$label registered. No drive content was changed."
    }
}
function Watch-Media {
    param([array]$SessionRecords=@())
    Assert-WindowsAdmin
    $once=$SessionRecords.Count -gt 0
    $cfg=Get-Config
    $records=$(if ($once) { $SessionRecords } else { @(Get-Records) })
    $records=@($records)
    if ($once) { $cfg.PrepareDisk=$true; $cfg.SetupBatch=''; $cfg.SetupArguments=@(); $cfg.MutableFiles=@(); $cfg.SetupInteractive=$false }
    if (-not $records.Count) { throw 'Register the intended MRI USB drives first, or use menu 2 for a selected batch.' }
    # A single machine-wide watcher avoids two independent copies racing the same USB.
    $mutex=[Threading.Mutex]::new($false,'Global\MRI_BATCH_CREATOR_V1')
    $locked=$false; $jobs=@{}; $tracker=@{}; $seen=@{}; $ready=@{}; $failed=@{}; $opened=$null
    $run=Join-Path $DataPath ('Runs\'+[guid]::NewGuid().ToString('N'))
    $cancel=Join-Path $run 'cancel'
    try {
        try { $locked=$mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $locked=$true }
        if (-not $locked) { throw 'Another MRI session is already running on this PC.' }
        $inputPath=$cfg.SourcePath
        $recipeIdentity=ConvertTo-Json -InputObject $cfg -Depth 6 -Compress
        $opened=Open-MRISource $cfg (Join-Path $DataPath 'Sources')
        $cfg.SourcePath=$opened.Path
        $cfg | Add-Member -NotePropertyName SourceKind -NotePropertyValue Folder -Force
        $cfg | Add-Member -NotePropertyName RecipeIdentity -NotePropertyValue $recipeIdentity -Force
        $cfg | Add-Member -NotePropertyName InputStamp -NotePropertyValue $opened.Stamp -Force
        Assert-MRIPlainDirectory $PSScriptRoot
        Assert-MRISourceSeparation $opened $PSScriptRoot
        [void][IO.Directory]::CreateDirectory($run)
        Write-Host 'Reading the source once and hashing every file. This can take a few minutes.'
        $manifest=Get-MRIManifest $cfg.SourcePath
        if (-not @($manifest.Entries | Where-Object { -not $_.IsDirectory }).Count) { throw 'MRI source contains no files.' }
        if (@($manifest.Entries | Where-Object { $_.Length -gt 4294967295 }).Count) { throw 'A source file exceeds the FAT32 file-size limit.' }
        foreach ($mutable in @($cfg.MutableFiles)) {
            if (-not @($manifest.Entries | Where-Object { $_.RelativePath -eq $mutable -and -not $_.IsDirectory }).Count) { throw "MutableFiles entry is not a source file: $mutable" }
        }
        $snapshot=Join-Path $run 'config.json'; $manifestPath=Join-Path $run 'source-manifest.json'
        $configHash=Write-MRIJson $snapshot $cfg -PassThruHash
        $manifestHash=Write-MRIJson $manifestPath $manifest -PassThruHash
        $protected=@(Get-MRIProtectedKeys @($inputPath,$PSScriptRoot,$DataPath) @(Get-MRIInventory))
        $module=Join-Path $PSScriptRoot 'MRI.Batch.psm1'
        Write-Host "$($records.Count) selected USBs; maximum $($cfg.MaxParallel) builds at once."
        if ($once) { Write-Host 'This batch ends after each selected USB succeeds or fails. Keep them connected; Q cancels.' }
        else { Write-Host 'Plug in registered drives. Q stops the watcher; unexpected content blocks automatic erasure.' }
        Write-Host 'READY means files verified. Use Windows Safely Remove before unplugging.'
        Write-Host "Session reports: $run"
        $statusClock=[Diagnostics.Stopwatch]::StartNew()
        while ($true) {
            try { if ([Console]::KeyAvailable -and [Console]::ReadKey($true).Key -eq [ConsoleKey]::Q) { break } } catch { }
            $inventory=@(Get-MRIInventory)
            $arrivals=@(Update-MRIArrivals $tracker $inventory $records $cfg $protected)
            if ($once) {
                foreach ($record in $records) {
                    if ($jobs.ContainsKey($record.Key) -or $ready.ContainsKey($record.Key) -or $failed.ContainsKey($record.Key)) { continue }
                    $live=@($inventory | Where-Object { $_.Key -eq $record.Key -and $_.Number -eq $record.ExpectedNumber })
                    $reason=$(if ($live.Count -ne 1) { 'Selected USB removed, replaced, or renumbered before its build.' } else { Get-MRIDriveBlockReason $live[0] $inventory $cfg $protected })
                    if ($reason) { Set-MRIOutcome $ready $failed $record.Key $record.Label $false; Write-Host "$($record.Label): FAILED - $reason" -ForegroundColor Red }
                }
                $arrivals=@($arrivals | Where-Object { -not $failed.ContainsKey($_.Key) })
            }
            foreach ($d in $inventory) {
                $reason=Get-MRIDriveBlockReason $d $inventory $cfg $protected
                if (-not $reason -and -not @($records | Where-Object { $_.Key -eq $d.Key }).Count) { $reason='Unregistered USB ignored.' }
                $notice=([string]$d.Number)+'|'+$d.Key+'|'+$reason
                if ($reason -and -not $seen.ContainsKey($notice)) { Write-Host "Disk $($d.Number): $reason" -ForegroundColor Yellow; $seen[$notice]=$true }
            }
            foreach ($key in @($jobs.Keys)) {
                $info=$jobs[$key]
                if ([IO.File]::Exists($info.StatePath)) {
                    $state=Read-MRIJson $info.StatePath
                    if ($info.LastStage -ne $state.Stage) {
                        $color=$(if ($state.Stage -in @('READY','ALREADY_READY')) { 'Green' } elseif ($state.Stage -eq 'FAILED') { 'Red' } else { 'Cyan' })
                        Write-Host "$($state.Label): $($state.Stage) - $($state.Message)" -ForegroundColor $color
                        $info.LastStage=$state.Stage
                    }
                }
                if ($info.Job.State -in @('Completed','Failed','Stopped')) {
                    try {
                        $results=@(Receive-Job -Job $info.Job -ErrorAction Stop)
                        $outcomes=@($results | Where-Object { $null -ne $_.PSObject.Properties['Success'] })
                        if ($outcomes.Count -ne 1 -or -not $outcomes[0].Success) { throw 'Build did not return verified success. See the per-drive state/logs.' }
                        Set-MRIOutcome $ready $failed $key $info.Record.Label $true
                    } catch {
                        Set-MRIOutcome $ready $failed $key $info.Record.Label $false
                        if ($info.LastStage -ne 'FAILED') { Set-MRIState $info.StatePath $info.Record 'FAILED' $_.Exception.Message; Write-Host "$($info.Record.Label): FAILED - $($_.Exception.Message)" -ForegroundColor Red }
                    } finally { Remove-Job -Job $info.Job -Force; $jobs.Remove($key) }
                    Write-Host "Verified: $($ready.Count)/$($records.Count) | Failed: $($failed.Count) | Active: $($jobs.Count)"
                }
            }
            foreach ($record in @(Select-MRIBuilds $arrivals $jobs $cfg.MaxParallel)) {
                $ready.Remove($record.Key); $failed.Remove($record.Key)
                $tracker[$record.Key].Attempted=$true
                $work=Join-Path $run $record.Label; [void][IO.Directory]::CreateDirectory($work)
                $statePath=Join-Path $work 'state.json'
                $receiptPath=Join-Path $DataPath ('Receipts\'+$record.Key+'.json')
                $approvedNumber=$(if ($once) { [int]$record.ExpectedNumber } else { $null })
                $job=Start-MRIBuildJob $module $record $snapshot $configHash $manifestPath $manifestHash $protected $statePath $receiptPath $work $cancel -ApprovedEraseNumber $approvedNumber
                $jobs[$record.Key]=[pscustomobject]@{ Job=$job; Record=$record; StatePath=$statePath; LastStage='' }
                Write-Host "$($record.Label): detected; build queued."
            }
            if ($statusClock.Elapsed.TotalSeconds -ge 10) {
                $waiting=@($arrivals | Where-Object { -not $jobs.ContainsKey($_.Key) -and -not $tracker[$_.Key].Attempted }).Count
                Write-Host "Active: $($jobs.Count)/$($cfg.MaxParallel) | Waiting: $waiting | Verified: $($ready.Count)/$($records.Count) | Failed: $($failed.Count)"
                $statusClock.Restart()
            }
            if ($once -and $jobs.Count -eq 0 -and ($ready.Count+$failed.Count) -eq $records.Count) { break }
            Start-Sleep -Seconds $cfg.PollSeconds
        }
    } finally {
        try {
            try {
                if ([IO.Directory]::Exists($run)) { [IO.File]::WriteAllText($cancel,'cancel') }
                if ($jobs.Count) { $jobs.Values | ForEach-Object { $_.Job } | Wait-Job -Timeout 10 | Out-Null }
            } finally {
                foreach ($info in @($jobs.Values)) {
                    Stop-Job -Job $info.Job -ErrorAction SilentlyContinue
                    try {
                        $success=$false
                        if ($info.Job.State -eq 'Completed') {
                            $results=@(Receive-Job -Job $info.Job -ErrorAction Stop)
                            $outcomes=@($results | Where-Object { $null -ne $_.PSObject.Properties['Success'] })
                            $success=$outcomes.Count -eq 1 -and $outcomes[0].Success
                        }
                        Set-MRIOutcome $ready $failed $info.Record.Key $info.Record.Label $success
                        if (-not $success) {
                            $stage=$(if ([IO.File]::Exists($info.StatePath)) { (Read-MRIJson $info.StatePath).Stage } else { '' })
                            if ($stage -ne 'FAILED') { Set-MRIState $info.StatePath $info.Record 'INTERRUPTED' 'Watcher stopped. This USB is not marked ready.' }
                        }
                    } catch {
                        Set-MRIOutcome $ready $failed $info.Record.Key $info.Record.Label $false
                        Write-Host "Could not finalize $($info.Record.Label)'s state: $($_.Exception.Message)" -ForegroundColor Red
                    } finally {
                        Remove-Job -Job $info.Job -Force -ErrorAction SilentlyContinue
                    }
                }
            }
        }
        finally {
            try { Close-MRISource $opened }
            finally {
                if ($locked) { $mutex.ReleaseMutex() }; $mutex.Dispose()
                if ($once) {
                    foreach ($record in $records) {
                        if (-not $ready.ContainsKey($record.Key) -and -not $failed.ContainsKey($record.Key)) { Set-MRIOutcome $ready $failed $record.Key $record.Label $false }
                    }
                    Write-Host "Batch finished: $($ready.Count)/$($records.Count) verified; $($failed.Count) failed or cancelled."
                }
                if ([IO.Directory]::Exists($run)) { Write-MRIJson (Join-Path $run 'summary.json') ([pscustomobject]@{ Verified=@($ready.Values); Failed=@($failed.Values); Registered=$records.Count }) }
            }
        }
    }
    if ($once -and $failed.Count) { throw "Batch failed or cancelled for $($failed.Count) selected USB(s). See the per-drive results above." }
}
function Show-AdvancedMenu {
    Write-Host '1 Register USBs for automatic watching'
    Write-Host '2 Watch registered USBs'
    Write-Host 'Q Back'
    switch ((Read-Host 'Select')) {
        '1' { Enroll-Drives }
        '2' { Watch-Media }
        { $_ -match '^[Qq]$' } { return }
        default { Write-Host 'Choose 1, 2, or Q.' }
    }
}
try {
    if ($Action -eq 'SelfTest') { & (Join-Path $PSScriptRoot 'tests\SelfTest.ps1'); exit $LASTEXITCODE }
    if ($Action -eq 'Menu') {
        while ($true) {
            Write-Host "`nMRI Drive Maker"
            Write-Host 'Based on JStone21/MRI-Creation. Modifications (C) 2026 Cucklord420.'
            Write-Host 'GPL v3: you may copy, modify, and redistribute under LICENSE. No warranty.'
            Write-Host ("License: " + (Join-Path $PSScriptRoot 'LICENSE'))
            Write-Host '1 Choose MRI folder / ISO / ZIP'
            Write-Host '2 Clean, format FAT32 and copy to selected USBs'
            Write-Host '3 List connected USB drives'
            Write-Host '4 Set simultaneous builds (1-30)'
            Write-Host 'A Advanced: registered-drive watcher'
            Write-Host 'Q Exit'
            try { switch ((Read-Host 'Select')) {
                '1' { Configure-Media }
                '2' { Build-SelectedMedia }
                '3' { Show-Drives }
                '4' { Set-ParallelLimit }
                '5' { Set-ParallelLimit } # Compatibility with the previous menu.
                { $_ -match '^[Aa]$' } { Show-AdvancedMenu }
                { $_ -match '^[Qq]$' } { exit 0 }
                default { Write-Host 'Choose 1, 2, 3, 4, A, or Q.' }
            } } catch { Write-Host $_.Exception.Message -ForegroundColor Red }
        }
    } else {
        switch ($Action) { Configure { Configure-Media }; Build { Build-SelectedMedia }; Enroll { Enroll-Drives }; List { Show-Drives }; Watch { Watch-Media } }
    }
} catch { Write-Host $_.Exception.Message -ForegroundColor Red; exit 1 }
