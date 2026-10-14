# SPDX-License-Identifier: GPL-3.0-only
# GPL-3.0-only. Updated 10-13-2026 for ISO input and audited drive guards.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-MRIJson {
    param([string]$Path, $Value)
    $Path=[IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($Path)
    [void][IO.Directory]::CreateDirectory($parent)
    $tmp = $Path + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
    try {
        [IO.File]::WriteAllText($tmp, (ConvertTo-Json -InputObject $Value -Depth 30), [Text.UTF8Encoding]::new($false))
        if ([IO.File]::Exists($Path)) { [IO.File]::Replace($tmp, $Path, [System.Management.Automation.Language.NullString]::Value) }
        else { [IO.File]::Move($tmp, $Path) }
    } finally { if ([IO.File]::Exists($tmp)) { [IO.File]::Delete($tmp) } }
}
function Read-MRIJson {
    param([string]$Path)
    if (-not [IO.File]::Exists($Path)) { throw "File not found: $Path" }
    return ([IO.File]::ReadAllText($Path) | ConvertFrom-Json)
}
function Get-MRIHash {
    param([string]$Text)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text))).Replace('-', '') }
    finally { $sha.Dispose() }
}
function ConvertTo-MRIRelativePath {
    param([string]$Path)
    $p = $Path.Replace('/', '\')
    if ([string]::IsNullOrWhiteSpace($p) -or $p -match '^[\\/]|^[A-Za-z]:|[\x00-\x1f:*?"<>|]') {
        throw "A relative file path is required: $Path"
    }
    $parts = $p.Split('\')
    if (@($parts | Where-Object { $_ -in @('', '.', '..') -or $_.EndsWith(' ') -or $_.EndsWith('.') }).Count -gt 0) {
        throw "Unsafe relative file path: $Path"
    }
    return $p
}
function Join-MRIPath {
    param([string]$Root, [string]$RelativePath)
    # IO.Path.Combine, rather than PowerShell drive resolution, also accepts volume GUID paths.
    $rel = ConvertTo-MRIRelativePath $RelativePath
    return [IO.Path]::Combine($Root, $rel.Replace('\', [IO.Path]::DirectorySeparatorChar))
}
function Assert-MRIConfig {
    param($Config)
    $required = @('SchemaVersion','SourcePath','SetupBatch','SetupArguments','SetupInteractive','SetupTimeoutSeconds',
        'CopyTimeoutSeconds','PrepareDisk','FileSystem','RequiredFiles','MutableFiles','MaxParallel','PollSeconds',
        'StablePolls','MinimumDiskBytes','MaximumDiskBytes','SerializeSetup')
    foreach ($name in $required) {
        if ($null -eq $Config.PSObject.Properties[$name]) { throw "Missing setting: $name" }
    }
    if ($Config.SchemaVersion -ne 1) { throw 'Unsupported configuration version.' }
    $kind=Get-MRISourceKind $Config
    if ([string]::IsNullOrWhiteSpace($Config.SourcePath)) { throw 'Select the official MRI ISO, ZIP, or extracted folder first.' }
    if ($kind -eq 'Folder') {
        if (-not [IO.Directory]::Exists($Config.SourcePath)) { throw 'The selected MRI source folder was not found.' }
    } else {
        if (-not [IO.File]::Exists($Config.SourcePath) -or [IO.Path]::GetExtension($Config.SourcePath) -ine ('.'+$kind)) {
            throw "The selected $kind source file was not found or has the wrong extension."
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($Config.SetupBatch)) {
        $batch=ConvertTo-MRIRelativePath $Config.SetupBatch
        if ($batch -notmatch '\.(bat|cmd)$' -or $batch -match '[%!&^()]') { throw 'Choose a .bat/.cmd path without CMD metacharacters.' }
        if ($kind -eq 'Folder') {
            if (-not [IO.File]::Exists((Join-MRIPath $Config.SourcePath $batch))) { throw "Setup batch not found: $batch" }
            if ((Get-Item -LiteralPath (Join-MRIPath $Config.SourcePath $batch)).Length -eq 0) { throw 'The configured setup batch is empty.' }
        }
    } elseif (@($Config.SetupArguments).Count -or @($Config.MutableFiles).Count -or $Config.SetupInteractive) {
        throw 'Direct-copy mode must have no setup arguments, mutable-file exceptions, or interactive setup.'
    }
    foreach ($item in @($Config.RequiredFiles) + @($Config.MutableFiles)) { [void](ConvertTo-MRIRelativePath $item) }
    foreach ($arg in @($Config.SetupArguments)) {
        $rest = ([string]$arg) -replace '\{(DriveLetter|DriveRoot|DiskNumber|Label)\}', ''
        if ($rest -match '[^A-Za-z0-9_ .:/\\=+,-]') { throw "Unsupported batch argument: $arg" }
    }
    foreach ($name in @('SetupInteractive','PrepareDisk','SerializeSetup')) {
        if ($Config.$name -isnot [bool]) { throw "$name must be true or false." }
    }
    if ($Config.FileSystem -ne 'FAT32') { throw 'This media recipe supports FAT32 only.' }
    foreach ($name in @('MaxParallel','PollSeconds','StablePolls','MinimumDiskBytes','MaximumDiskBytes','SetupTimeoutSeconds','CopyTimeoutSeconds')) {
        if ($Config.$name -isnot [int] -and $Config.$name -isnot [long]) { throw "$name must be an integer." }
    }
    if ($Config.MaxParallel -lt 1 -or $Config.MaxParallel -gt 8) { throw 'MaxParallel must be 1 through 8.' }
    if ($Config.PollSeconds -lt 1 -or $Config.PollSeconds -gt 30 -or $Config.StablePolls -lt 2 -or $Config.StablePolls -gt 30) {
        throw 'Use PollSeconds 1-30 and StablePolls 2-30.'
    }
    if ($Config.MinimumDiskBytes -lt 1 -or $Config.MaximumDiskBytes -lt $Config.MinimumDiskBytes) { throw 'Invalid capacity limits.' }
    if ($Config.SetupTimeoutSeconds -lt 1 -or $Config.CopyTimeoutSeconds -lt 1) { throw 'Timeouts must be positive.' }
}
function Get-MRISourceKind {
    param($Config)
    if ($null -eq $Config.PSObject.Properties['SourceKind']) { return 'Folder' }
    if ($Config.SourceKind -notin @('Folder','ISO','ZIP')) { throw 'SourceKind must be Folder, ISO, or ZIP.' }
    return [string]$Config.SourceKind
}
function Assert-MRIPlainDirectory {
    param([string]$Path)
    $item=Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (-not $item.PSIsContainer) { throw 'A source directory is required.' }
    while ($null -ne $item) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Source/target paths cannot pass through links or mounted-directory junctions.' }
        $item=$item.Parent
    }
}
function Get-MRIMediaRoot {
    param([string]$Path)
    $root=[IO.Path]::GetFullPath($Path)
    for ($depth=0; $depth -lt 10; $depth++) {
        Assert-MRIPlainDirectory $root
        if ([IO.File]::Exists((Join-MRIPath $root 'efi\boot\bootx64.efi')) -or [IO.File]::Exists((Join-MRIPath $root 'bootmgr'))) { return $root }
        $items=@(Get-ChildItem -LiteralPath $root -Force -ErrorAction Stop)
        if ($items.Count -eq 1 -and $items[0].PSIsContainer) { $root=$items[0].FullName; continue }
        return $root
    }
    throw 'The source has too many nested wrapper folders. Choose its boot-files folder directly.'
}
function Expand-MRIArchive {
    param([string]$Path,[string]$Destination)
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $base=[IO.Path]::GetFullPath($Destination).TrimEnd([char[]]@('\','/'))+[IO.Path]::DirectorySeparatorChar
    [void][IO.Directory]::CreateDirectory($base)
    $archive=[IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $names=@{}; $total=[long]0
        foreach ($entry in $archive.Entries) {
            $relative=$entry.FullName.Replace('/','\').TrimEnd('\')
            if (-not $relative) { continue }
            [void](ConvertTo-MRIRelativePath $relative)
            if ($names.ContainsKey($relative)) { throw "Duplicate archive path: $relative" }; $names[$relative]=$true
            $total+=$entry.Length
            if ($total -gt 64GB -or $names.Count -gt 100000) { throw 'The ZIP exceeds the supported source size or entry count.' }
            if (($entry.ExternalAttributes -shr 16 -band 0xF000) -eq 0xA000) { throw 'ZIP symbolic links are not supported.' }
        }
        foreach ($entry in $archive.Entries) {
            $relative=$entry.FullName.Replace('/','\').TrimEnd('\')
            if (-not $relative) { continue }
            $output=Join-MRIPath $base $relative
            if ($entry.FullName.EndsWith('/') -or $entry.FullName.EndsWith('\')) { [void][IO.Directory]::CreateDirectory($output); continue }
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($output))
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,$output,$false)
        }
    } finally { $archive.Dispose() }
}
function Open-MRISource {
    param($Config,[string]$CachePath)
    Assert-MRIConfig $Config
    $kind=Get-MRISourceKind $Config
    $context=[pscustomobject]@{ Path=''; InputPath=[IO.Path]::GetFullPath($Config.SourcePath); MountedIso=''; OwnsMount=$false; CacheDirectory=''; Stamp=$null }
    try {
        if ($kind -eq 'Folder') { $context.Path=Get-MRIMediaRoot $context.InputPath; return $context }
        $inputFile=Get-Item -LiteralPath $context.InputPath -Force -ErrorAction Stop
        if ($inputFile.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'The ISO/ZIP source cannot be a link.' }
        Assert-MRIPlainDirectory $inputFile.Directory.FullName
        $context.Stamp=[pscustomobject]@{ Path=$context.InputPath; Length=[long]$inputFile.Length; WriteTicks=$inputFile.LastWriteTimeUtc.Ticks }
        $iso=$context.InputPath
        if ($kind -eq 'ZIP') {
            $context.CacheDirectory=Join-Path $CachePath ([guid]::NewGuid().ToString('N'))
            Expand-MRIArchive $context.InputPath $context.CacheDirectory
            $root=Get-MRIMediaRoot $context.CacheDirectory
            $items=@(Get-ChildItem -LiteralPath $root -Force)
            if ($items.Count -eq 1 -and -not $items[0].PSIsContainer -and $items[0].Extension -ieq '.iso') { $iso=$items[0].FullName }
            else { $context.Path=$root; return $context }
        }
        if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Opening an ISO image requires Windows.' }
        $image=Get-DiskImage -ImagePath $iso -ErrorAction Stop
        if (-not $image.Attached) {
            $image=Mount-DiskImage -ImagePath $iso -StorageType ISO -Access ReadOnly -PassThru -ErrorAction Stop
            $context.MountedIso=$iso; $context.OwnsMount=$true
        }
        $volumes=@($image | Get-Volume -ErrorAction Stop)
        if ($volumes.Count -ne 1 -or -not $volumes[0].DriveLetter) { throw 'The ISO did not expose one readable mounted volume.' }
        $context.Path=Get-MRIMediaRoot ([string]$volumes[0].DriveLetter+':\')
        return $context
    } catch { Close-MRISource $context; throw }
}
function Close-MRISource {
    param($Context)
    if ($null -eq $Context) { return }
    if ($Context.OwnsMount) { Dismount-DiskImage -ImagePath $Context.MountedIso -ErrorAction Stop | Out-Null; $Context.OwnsMount=$false }
    if ($Context.CacheDirectory -and [IO.Directory]::Exists($Context.CacheDirectory)) { Remove-Item -LiteralPath $Context.CacheDirectory -Recurse -Force -ErrorAction Stop }
}
function Assert-MRIInputSource {
    param($Config)
    if ($null -eq $Config.PSObject.Properties['InputStamp'] -or $null -eq $Config.InputStamp) { return }
    $stamp=$Config.InputStamp
    $file=Get-Item -LiteralPath $stamp.Path -Force -ErrorAction Stop
    if ($file.Length -ne $stamp.Length -or $file.LastWriteTimeUtc.Ticks -ne $stamp.WriteTicks) { throw 'The input ISO/ZIP changed during this session. Restart the watcher.' }
}
function Assert-MRINotCancelled {
    param([string]$CancelPath)
    if ($CancelPath -and [IO.File]::Exists($CancelPath)) { throw 'Build cancelled.' }
}
function Set-MRIOutcome {
    param([hashtable]$Ready,[hashtable]$Failed,[string]$Key,[string]$Label,[bool]$Success)
    if ($Success) { $Ready[$Key]=$Label; $Failed.Remove($Key) }
    else { $Ready.Remove($Key); $Failed[$Key]=$Label }
}
function Get-MRIDriveKey {
    param([string]$Serial, [string]$PnpId, [string]$Model, [long]$Size)
    if ([string]::IsNullOrWhiteSpace($Serial) -or $Serial.Trim() -match '^0+$' -or $Serial.Trim() -in @('UNKNOWN','N/A','NONE')) {
        throw 'This drive does not report a usable hardware serial number.'
    }
    if ([string]::IsNullOrWhiteSpace($PnpId) -or [string]::IsNullOrWhiteSpace($Model)) { throw 'Incomplete USB hardware identity.' }
    return Get-MRIHash (($Serial.Trim().ToUpperInvariant(), $PnpId.Trim().ToUpperInvariant(), $Model.Trim().ToUpperInvariant(), [string]$Size) -join "`n")
}
function Get-MRIInventory {
    $hardware = @(Get-CimInstance -ClassName Win32_DiskDrive -ErrorAction Stop)
    foreach ($disk in @(Get-Disk -ErrorAction Stop | Where-Object { [string]$_.BusType -eq 'USB' })) {
        $rows = @($hardware | Where-Object { $_.Index -eq $disk.Number })
        if ($rows.Count -ne 1) { continue }
        $serial = ([string]$disk.SerialNumber).Trim().ToUpperInvariant()
        if ([string]::IsNullOrWhiteSpace($serial)) { $serial = ([string]$rows[0].SerialNumber).Trim().ToUpperInvariant() }
        $pnp = ([string]$rows[0].PNPDeviceID).Trim().ToUpperInvariant()
        $model = ([string]$disk.FriendlyName).Trim().ToUpperInvariant()
        $key = ''; $identityError = ''
        try { $key = Get-MRIDriveKey $serial $pnp $model ([long]$disk.Size) } catch { $identityError = $_.Exception.Message }
        [pscustomobject]@{
            Key=$key; Number=[int]$disk.Number; Serial=$serial; PnpId=$pnp; Model=$model; Size=[long]$disk.Size
            IsBoot=[bool]$disk.IsBoot; IsSystem=[bool]$disk.IsSystem; IsReadOnly=[bool]$disk.IsReadOnly
            IsOffline=[bool]$disk.IsOffline; BusType=[string]$disk.BusType; IdentityError=$identityError
        }
    }
}
function Get-MRIDriveBlockReason {
    param($Drive, [array]$Inventory, $Config, [string[]]$ProtectedKeys=@())
    if ([string]::IsNullOrWhiteSpace($Drive.Key)) { return "Hardware identity unavailable: $($Drive.IdentityError)" }
    if ($Drive.BusType -ne 'USB' -or $Drive.IsBoot -or $Drive.IsSystem) { return 'Boot, system, and non-USB disks are excluded.' }
    if ($Drive.IsOffline -or $Drive.IsReadOnly) { return 'Drive is offline or write protected.' }
    if ($Drive.Key -in $ProtectedKeys) { return 'This disk contains the source files or the automation itself.' }
    if ($Drive.Size -lt $Config.MinimumDiskBytes -or $Drive.Size -gt $Config.MaximumDiskBytes) { return 'Capacity is outside the configured MRI range.' }
    if (@($Inventory | Where-Object { $_.Serial -eq $Drive.Serial }).Count -ne 1) { return 'Duplicate USB serial number; cannot identify this drive safely.' }
    return $null
}
function Assert-MRIRegistry {
    param([array]$Records)
    $keys=@{}; $serials=@{}; $labels=@{}
    foreach ($r in $Records) {
        foreach ($name in @('Key','Serial','PnpId','Model','Size','Label','AllowErase')) {
            if ($null -eq $r.PSObject.Properties[$name]) { throw "Drive registry is missing $name." }
        }
        if ($r.Key -ne (Get-MRIDriveKey $r.Serial $r.PnpId $r.Model $r.Size)) { throw 'Drive registry identity is invalid.' }
        if ($r.Label -notmatch '^MRI_[0-9]{2,3}$' -or $r.AllowErase -isnot [bool]) { throw 'Invalid registered drive label or erase setting.' }
        if ($keys.ContainsKey($r.Key) -or $serials.ContainsKey($r.Serial) -or $labels.ContainsKey($r.Label)) { throw 'Duplicate identity or label in the drive registry.' }
        $keys[$r.Key]=$true; $serials[$r.Serial]=$true; $labels[$r.Label]=$true
    }
}
function Get-MRIProtectedKeys {
    param([string[]]$Paths, [array]$Inventory)
    foreach ($path in $Paths) {
        $root=[IO.Path]::GetPathRoot([IO.Path]::GetFullPath($path))
        if ($root -match '^([A-Za-z]):\\$') {
            $partition=@(Get-Partition -DriveLetter $Matches[1] -ErrorAction Stop)
            if (-not $partition.Count) { throw 'Could not resolve the source/automation disk. No target is allowed to be written.' }
            foreach ($p in $partition) {
                foreach ($d in @($Inventory | Where-Object { $_.Number -eq $p.DiskNumber })) {
                    if (-not $d.Key) { throw 'A source/automation USB has no usable hardware identity. Use an internal source folder.' }; $d.Key
                }
            }
        }
    }
}
function Assert-MRIDrive {
    param($Record, $Config, [string[]]$ProtectedKeys=@(), [Nullable[int]]$ExpectedNumber=$null)
    Assert-MRIInputSource $Config
    $inventory=@(Get-MRIInventory)
    foreach ($protectedKey in $ProtectedKeys) {
        if (@($inventory | Where-Object { $_.Key -eq $protectedKey }).Count -ne 1) { throw 'Source or automation USB was removed or replaced.' }
    }
    $matches=@($inventory | Where-Object { $_.Key -eq $Record.Key })
    if ($matches.Count -ne 1) { throw 'Registered USB removed, replaced, or ambiguously identified.' }
    $drive=$matches[0]
    $reason=Get-MRIDriveBlockReason $drive $inventory $Config $ProtectedKeys
    if ($reason) { throw $reason }
    if ($null -ne $ExpectedNumber -and $drive.Number -ne $ExpectedNumber) { throw 'Disk number changed during this build.' }
    return $drive
}
function Get-MRITargetVolume {
    param($Drive)
    $parts=@(Get-Partition -DiskNumber $Drive.Number -ErrorAction Stop)
    if (@($parts | Where-Object { $_.IsBoot -or $_.IsSystem }).Count -gt 0) { throw 'A boot/system partition was found on the target.' }
    if ($parts.Count -ne 1) { throw 'Copy mode requires exactly one existing partition. Prepare a fresh drive or use the registered erase mode.' }
    $p=$parts[0]
    $volumes=@(Get-Volume -Partition $p -ErrorAction Stop)
    if ($volumes.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$p.DriveLetter)) { throw 'The USB needs one mounted volume with a drive letter.' }
    $v=$volumes[0]
    if ([string]$v.Path -notmatch '^\\\\\?\\Volume\{[0-9A-Fa-f-]+\}\\$') { throw 'A stable volume GUID path could not be obtained.' }
    [pscustomobject]@{ Path=[string]$v.Path; Letter=[string]$p.DriveLetter; Number=$Drive.Number
        PartitionNumber=$p.PartitionNumber; FileSystem=[string]$v.FileSystem; FreeBytes=[long]$v.SizeRemaining }
}
function Assert-MRITarget {
    param($Record, $Config, $Target, [string[]]$ProtectedKeys=@())
    $d=Assert-MRIDrive $Record $Config $ProtectedKeys $Target.Number
    $v=Get-MRITargetVolume $d
    if ($v.Path -ne $Target.Path -or $v.Letter -ne $Target.Letter) { throw 'The USB volume or drive-letter mapping changed. Build stopped.' }
    return $d
}
function Get-MRIManifest {
    param([string]$Root, [switch]$IgnoreSystemDirectories, [switch]$NoHash, [scriptblock]$Guard)
    $base=[IO.Path]::GetFullPath($Root).TrimEnd([char[]]@('\','/')) + [IO.Path]::DirectorySeparatorChar
    Assert-MRIPlainDirectory $base
    $rootItem=Get-Item -LiteralPath $base -Force -ErrorAction Stop
    if (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Source/target root must be a real directory, not a link.' }
    $stack=[Collections.Generic.Stack[string]]::new(); $stack.Push($base)
    $entries=[Collections.Generic.List[object]]::new(); $names=@{}
    $clock=[Diagnostics.Stopwatch]::StartNew()
    while ($stack.Count -gt 0) {
        foreach ($item in @(Get-ChildItem -LiteralPath $stack.Pop() -Force -ErrorAction Stop)) {
            $rel=$item.FullName.Substring($base.Length).Replace('/', '\')
            if ($IgnoreSystemDirectories -and $rel.Split('\')[0] -in @('System Volume Information','$RECYCLE.BIN')) { continue }
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Links are not allowed in media content: $rel" }
            if ($names.ContainsKey($rel)) { throw "Case-insensitive path collision: $rel" }; $names[$rel]=$true
            $entry=[pscustomobject]@{ RelativePath=$rel; IsDirectory=[bool]$item.PSIsContainer; Length=[long]0; Sha256='' }
            if ($item.PSIsContainer) { $stack.Push($item.FullName) }
            else {
                $entry.Length=[long]$item.Length
                if (-not $NoHash) {
                    if ($Guard) { & $Guard | Out-Null }
                    $beforeLength=$item.Length; $beforeTime=$item.LastWriteTimeUtc
                    $entry.Sha256=(Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256 -ErrorAction Stop).Hash
                    $item.Refresh()
                    if ($item.Length -ne $beforeLength -or $item.LastWriteTimeUtc -ne $beforeTime) { throw "A file changed while being hashed: $rel" }
                    if ($Guard) { & $Guard | Out-Null }
                }
            }
            $entries.Add($entry)
            if ($Guard -and $clock.Elapsed.TotalSeconds -ge 1) { & $Guard | Out-Null; $clock.Restart() }
        }
    }
    $sorted=@($entries | Sort-Object RelativePath)
    $total=[long]0; foreach ($e in $sorted) { $total+=$e.Length }
    [pscustomobject]@{ Entries=$sorted; TotalBytes=$total; Digest=(Get-MRIHash (ConvertTo-Json -InputObject $sorted -Compress -Depth 5)) }
}
function Assert-MRIManifest {
    param($Expected, [string]$Root, [string[]]$MutableFiles=@(), [scriptblock]$Guard)
    foreach ($e in @($Expected.Entries)) {
        if ($Guard) { & $Guard | Out-Null }
        $path=Join-MRIPath $Root $e.RelativePath
        if ($e.IsDirectory) {
            if (-not [IO.Directory]::Exists($path)) { throw "Missing directory: $($e.RelativePath)" }; continue
        }
        if (-not [IO.File]::Exists($path)) { throw "Missing file: $($e.RelativePath)" }
        $info=Get-Item -LiteralPath $path -Force -ErrorAction Stop
        if ($info.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'A target file became a link.' }
        if ($e.RelativePath -notin $MutableFiles) {
            if ($info.Length -ne $e.Length) { throw "File size mismatch: $($e.RelativePath)" }
            if ((Get-FileHash -LiteralPath $path -Algorithm SHA256 -ErrorAction Stop).Hash -ne $e.Sha256) { throw "SHA-256 mismatch: $($e.RelativePath)" }
        } elseif ($info.Length -eq 0 -and $e.Length -ne 0) { throw "Setup left an empty mutable file: $($e.RelativePath)" }
        if ($Guard) { & $Guard | Out-Null }
    }
}
function Assert-MRIManifestMatch {
    param($Expected, $Actual, [string[]]$MutableFiles=@())
    $index=@{}; foreach ($e in @($Actual.Entries)) { $index[$e.RelativePath]=$e }
    foreach ($e in @($Expected.Entries)) {
        if (-not $index.ContainsKey($e.RelativePath)) { throw "Missing final path: $($e.RelativePath)" }
        $a=$index[$e.RelativePath]
        if ($e.IsDirectory -ne $a.IsDirectory) { throw "Final path type mismatch: $($e.RelativePath)" }
        if ($e.IsDirectory) { continue }
        if ($e.RelativePath -notin $MutableFiles) {
            if ($e.Length -ne $a.Length -or $e.Sha256 -ne $a.Sha256) { throw "Final file mismatch: $($e.RelativePath)" }
        } elseif ($a.Length -eq 0 -and $e.Length -ne 0) { throw "Setup left an empty mutable file: $($e.RelativePath)" }
    }
}
function Assert-MRIContentsSafe {
    param([string]$Root, $Receipt, [scriptblock]$Guard)
    # Enumerate names/sizes first: do not hash unexpected client files.
    $current=Get-MRIManifest $Root -IgnoreSystemDirectories -NoHash -Guard $Guard
    if (@($current.Entries).Count -eq 0) { return $false }
    if ($null -eq $Receipt) { throw 'Drive is not empty and has no successful local MRI receipt. It will not be erased or overwritten.' }
    $old=@{}; foreach ($e in @($Receipt.TargetManifest.Entries)) { $old[$e.RelativePath]=$e }
    if ($old.Count -ne @($current.Entries).Count) { throw 'Drive contains unexpected or missing content. It will not be erased or overwritten.' }
    foreach ($e in @($current.Entries)) {
        if (-not $old.ContainsKey($e.RelativePath) -or $old[$e.RelativePath].IsDirectory -ne $e.IsDirectory -or $old[$e.RelativePath].Length -ne $e.Length) {
            throw 'Drive contains unexpected content. It will not be erased or overwritten.'
        }
    }
    Assert-MRIManifest $Receipt.TargetManifest $Root -Guard $Guard
    return $true
}
function Initialize-MRINative {
    if (-not ('MRIBatch.NativeProcess' -as [type])) { Add-Type -Path (Join-Path $PSScriptRoot 'NativeProcess.cs') -ErrorAction Stop }
}
function Flush-MRIVolume {
    param([string]$Path)
    Initialize-MRINative
    [MRIBatch.NativeProcess]::FlushVolume($Path)
}
function Get-MRIWindowsExecutable {
    param([string]$Name)
    return Join-Path $env:SystemRoot ('System32\'+$Name)
}
function Set-MRIState {
    param([string]$Path, $Record, [string]$Stage, [string]$Message='')
    Write-MRIJson $Path ([pscustomobject]@{ Label=$Record.Label; Key=$Record.Key; Stage=$Stage; Message=$Message; UpdatedUtc=[datetime]::UtcNow.ToString('o') })
}
function Invoke-MRICommand {
    param([string]$Executable, [string]$Arguments, [string]$WorkingDirectory, [string]$LogPath,
        [int]$TimeoutSeconds, [scriptblock]$Guard, [bool]$Interactive=$false, [string]$CancelPath='')
    Initialize-MRINative
    & $Guard $true | Out-Null
    $runner=$null
    try {
        $runner=[MRIBatch.NativeProcess]::new($Executable,$Arguments,$WorkingDirectory,$LogPath,$Interactive)
        $timer=[Diagnostics.Stopwatch]::StartNew()
        while ($runner.ActiveProcesses -gt 0) {
            if ($CancelPath -and [IO.File]::Exists($CancelPath)) { throw 'Build cancelled.' }
            & $Guard | Out-Null
            if ($timer.Elapsed.TotalSeconds -ge $TimeoutSeconds) { throw "Command exceeded its ${TimeoutSeconds}s timeout; its process tree was stopped." }
            Start-Sleep -Milliseconds 500
        }
        & $Guard $true | Out-Null
        return [int]$runner.ExitCode
    } finally { if ($null -ne $runner) { $runner.Dispose() } }
}
function Invoke-MRISetupCommand {
    param($Config, [string]$Arguments, [string]$WorkingDirectory, [string]$LogPath,
        [scriptblock]$Guard, [string]$CancelPath)
    $mutex=$null; $owned=$false
    try {
        if ($Config.SerializeSetup) {
            $mutex=[Threading.Mutex]::new($false,'Global\MRI_BATCH_SETUP_V1')
            $wait=[Diagnostics.Stopwatch]::StartNew()
            while (-not $owned) {
                & $Guard $true | Out-Null
                if ([IO.File]::Exists($CancelPath)) { throw 'Build cancelled.' }
                if ($wait.Elapsed.TotalSeconds -ge $Config.SetupTimeoutSeconds) { throw 'Timed out waiting for the other USB setup batch.' }
                try { $owned=$mutex.WaitOne(500) } catch [Threading.AbandonedMutexException] { $owned=$true }
            }
        }
        return Invoke-MRICommand (Get-MRIWindowsExecutable 'cmd.exe') $Arguments $WorkingDirectory $LogPath $Config.SetupTimeoutSeconds $Guard -Interactive $Config.SetupInteractive -CancelPath $CancelPath
    } finally {
        if ($mutex) { if ($owned) { $mutex.ReleaseMutex() }; $mutex.Dispose() }
    }
}
function Get-MRISetupArguments {
    param($Config, $Record, $Target)
    $expanded=@()
    foreach ($a in @($Config.SetupArguments)) {
        $value=([string]$a).Replace('{DriveLetter}',$Target.Letter).Replace('{DriveRoot}',($Target.Letter+':\')).Replace('{DiskNumber}',[string]$Target.Number).Replace('{Label}',$Record.Label)
        if ($value -match '[^A-Za-z0-9_ .:/\\=+,-]') { throw 'Unsafe expanded batch argument.' }
        $expanded+=('"'+$value+'"')
    }
    $batch=$Target.Letter+':\'+(ConvertTo-MRIRelativePath $Config.SetupBatch)
    return '/d /s /c ""'+$batch+'"'+ $(if ($expanded.Count) { ' '+($expanded -join ' ') } else { '' }) +'"'
}
function Get-MRIWritableDisk {
    param($Record,$Config,[string[]]$ProtectedKeys,[int]$ExpectedNumber,[string]$CancelPath)
    Assert-MRINotCancelled $CancelPath
    $drive=Assert-MRIDrive $Record $Config $ProtectedKeys $ExpectedNumber
    $disk=Get-Disk -Number $drive.Number -ErrorAction Stop
    $hardware=@(Get-CimInstance -ClassName Win32_DiskDrive -Filter ("Index = "+$disk.Number) -ErrorAction Stop)
    if ($hardware.Count -ne 1 -or $hardware[0].Index -ne $disk.Number) { throw 'The selected storage object has no unambiguous physical identity.' }
    $serial=([string]$disk.SerialNumber).Trim()
    if (-not $serial) { $serial=[string]$hardware[0].SerialNumber }
    $key=Get-MRIDriveKey $serial ([string]$hardware[0].PNPDeviceID) ([string]$disk.FriendlyName) ([long]$disk.Size)
    if ($key -ne $Record.Key -or $disk.Number -ne $ExpectedNumber -or [string]$disk.BusType -ne 'USB' -or
        $disk.IsBoot -or $disk.IsSystem -or $disk.IsOffline -or $disk.IsReadOnly) { throw 'The storage object changed or is protected. No disk operation was started.' }
    Assert-MRINotCancelled $CancelPath
    return $disk
}
function Initialize-MRITarget {
    param($Record, $Config, [string[]]$ProtectedKeys, [int]$ExpectedNumber,[string]$CancelPath='')
    if (-not $Record.AllowErase) { throw 'This drive was not individually enrolled for automatic erasure.' }
    $actual=Get-MRIWritableDisk $Record $Config $ProtectedKeys $ExpectedNumber $CancelPath
    if ([string]$actual.PartitionStyle -ne 'RAW') {
        Clear-Disk -InputObject $actual -RemoveData -RemoveOEM -Confirm:$false -ErrorAction Stop
    }
    $actual=Get-MRIWritableDisk $Record $Config $ProtectedKeys $ExpectedNumber $CancelPath
    if ([string]$actual.PartitionStyle -eq 'RAW') { $actual=Initialize-Disk -InputObject $actual -PartitionStyle MBR -PassThru -ErrorAction Stop }
    $actual=Get-MRIWritableDisk $Record $Config $ProtectedKeys $ExpectedNumber $CancelPath
    # Stay below the Windows FAT32 format limit and the hardware's usable capacity.
    $size=[long][Math]::Min(32000000000,($actual.Size-2MB))
    $part=New-Partition -InputObject $actual -Size $size -IsActive -AssignDriveLetter -ErrorAction Stop
    [void](Get-MRIWritableDisk $Record $Config $ProtectedKeys $ExpectedNumber $CancelPath)
    $part | Format-Volume -FileSystem FAT32 -NewFileSystemLabel $Record.Label -Force -Confirm:$false -ErrorAction Stop | Out-Null
    return Get-MRITargetVolume (Assert-MRIDrive $Record $Config $ProtectedKeys $ExpectedNumber)
}
function Invoke-MRIBuild {
    param($Record, $Config, $SourceManifest, [string[]]$ProtectedKeys, [string]$StatePath,
        [string]$ReceiptPath, [string]$WorkPath, [string]$CancelPath)
    $target=$null; $number=$null; $newReceipt=$false
    try {
        Assert-MRINotCancelled $CancelPath
        Assert-MRIConfig $Config
        if ((Get-MRISourceKind $Config) -ne 'Folder') { throw 'Open the ISO/ZIP source before starting a drive build.' }
        Initialize-MRINative
        [void][IO.Directory]::CreateDirectory($WorkPath)
        Set-MRIState $StatePath $Record 'CHECKING'
        $drive=Assert-MRIDrive $Record $Config $ProtectedKeys; $number=$drive.Number
        $runSetup=-not [string]::IsNullOrWhiteSpace($Config.SetupBatch)
        if (-not $runSetup) {
            foreach ($required in @($Config.RequiredFiles)) {
                if (-not @($SourceManifest.Entries | Where-Object { $_.RelativePath -eq $required -and -not $_.IsDirectory -and $_.Length -gt 0 }).Count) {
                    throw "Required boot file is missing from the source: $required"
                }
            }
        }
        if (@($SourceManifest.Entries | Where-Object { -not $_.IsDirectory -and $_.Length -gt 4294967295 }).Count) { throw 'A source file is larger than FAT32 permits.' }
        if ($Config.PrepareDisk -and ($SourceManifest.TotalBytes+256MB) -gt [Math]::Min(32000000000,($drive.Size-2MB))) { throw 'Source does not fit this USB; preparation was not started.' }
        $receipt=$null
        if ([IO.File]::Exists($ReceiptPath)) {
            $receipt=Read-MRIJson $ReceiptPath
            if ($receipt.Key -ne $Record.Key -or $receipt.Status -ne 'READY') { throw 'Invalid previous drive receipt.' }
        }
        $alreadyVerified=$false
        $parts=@(Get-Partition -DiskNumber $number -ErrorAction Stop)
        if ($parts.Count -gt 0) {
            $target=Get-MRITargetVolume $drive
            $guardClock=[Diagnostics.Stopwatch]::StartNew()
            $guard={
                param([bool]$ForceCheck=$false)
                if ([IO.File]::Exists($CancelPath)) { throw 'Build cancelled.' }
                if ($ForceCheck -or $guardClock.ElapsedMilliseconds -ge 500) {
                    [void](Assert-MRITarget $Record $Config $target $ProtectedKeys); $guardClock.Restart()
                }
            }.GetNewClosure()
            & $guard $true | Out-Null
            $alreadyVerified=Assert-MRIContentsSafe $target.Path $receipt $guard
        } elseif (-not $Config.PrepareDisk) { throw 'Unpartitioned drive: select registered erase/prepare mode first.' }
        $identity=$(if ($null -ne $Config.PSObject.Properties['RecipeIdentity']) { [string]$Config.RecipeIdentity } else { ConvertTo-Json -InputObject $Config -Depth 6 -Compress })
        $recipe=Get-MRIHash ($identity+$SourceManifest.Digest)
        if ($alreadyVerified -and $receipt -and $target -and $receipt.Recipe -eq $recipe -and $receipt.VolumePath -eq $target.Path) {
            # Full previous-output verification above prevents trusting a stale completion flag.
            & $guard $true | Out-Null
            Flush-MRIVolume $target.Path
            [void](& $guard $true)
            Set-MRIState $StatePath $Record 'ALREADY_READY' 'Already verified for this source and recipe.'
            return [pscustomobject]@{ Success=$true; Label=$Record.Label; AlreadyReady=$true }
        }
        if ($Config.PrepareDisk) {
            Assert-MRINotCancelled $CancelPath
            Set-MRIState $StatePath $Record 'PREPARING' 'Preparing the individually registered USB.'
            if (-not $Record.AllowErase) { throw 'This drive was not individually enrolled for automatic erasure.' }
            if ($receipt) { Write-MRIJson (Join-Path $WorkPath 'previous-receipt.json') $receipt; [IO.File]::Delete($ReceiptPath); $receipt=$null }
            $target=Initialize-MRITarget $Record $Config $ProtectedKeys $number $CancelPath
        } else {
            # Copy-only mode never overwrites nonempty media for a different source/recipe.
            $contents=Get-MRIManifest $target.Path -IgnoreSystemDirectories -NoHash
            if (@($contents.Entries).Count) { throw 'Copy-only mode needs an empty drive for a new recipe. No files were overwritten.' }
            if ($receipt) { Write-MRIJson (Join-Path $WorkPath 'previous-receipt.json') $receipt; [IO.File]::Delete($ReceiptPath); $receipt=$null }
        }
        $guardClock=[Diagnostics.Stopwatch]::StartNew()
        $guard={
            param([bool]$ForceCheck=$false)
            if ([IO.File]::Exists($CancelPath)) { throw 'Build cancelled.' }
            if ($ForceCheck -or $guardClock.ElapsedMilliseconds -ge 500) {
                [void](Assert-MRITarget $Record $Config $target $ProtectedKeys); $guardClock.Restart()
            }
        }.GetNewClosure()
        & $guard $true | Out-Null
        if ($target.FileSystem -ne $Config.FileSystem) { throw 'Target is not FAT32. Select prepare mode for this registered drive.' }
        if ($target.FreeBytes -lt ($SourceManifest.TotalBytes+256MB)) { throw 'Insufficient USB space; source files plus 256 MiB of headroom are required.' }
        if (@($SourceManifest.Entries | Where-Object { -not $_.IsDirectory -and $_.Length -gt 4294967295 }).Count) { throw 'A source file is larger than FAT32 permits.' }
        Set-MRIState $StatePath $Record 'COPYING' 'Copying to the pinned volume GUID, not a reusable drive letter.'
        $copyArgs=@($Config.SourcePath,$target.Path,'/E','/COPY:DAT','/DCOPY:T','/IS','/IT','/XJ','/R:1','/W:1','/FFT','/J','/NP','/NFL','/NDL')
        $command=($copyArgs | ForEach-Object { [MRIBatch.NativeProcess]::Quote([string]$_) }) -join ' '
        $code=Invoke-MRICommand (Get-MRIWindowsExecutable 'robocopy.exe') $command $Config.SourcePath (Join-Path $WorkPath 'copy.log') $Config.CopyTimeoutSeconds $guard -CancelPath $CancelPath
        if ($code -lt 0 -or $code -ge 8) { throw "Robocopy reported failure (exit $code)." }
        Set-MRIState $StatePath $Record 'VERIFYING_COPY' 'Verifying every source file with SHA-256.'
        $copied=Get-MRIManifest $target.Path -IgnoreSystemDirectories -Guard $guard
        if (@($copied.Entries).Count -ne @($SourceManifest.Entries).Count) { throw 'Copied media contains paths absent from the source snapshot. Setup was not started.' }
        Assert-MRIManifestMatch $SourceManifest $copied
        $setupExit=$null
        if ($runSetup) {
            Set-MRIState $StatePath $Record 'SETUP' 'Running the configured media-creation batch and waiting for its process tree.'
            $setupArgs=Get-MRISetupArguments $Config $Record $target
            $setupExit=Invoke-MRISetupCommand $Config $setupArgs ($target.Letter+':\') (Join-Path $WorkPath 'setup.log') $guard $CancelPath
            if ($setupExit -ne 0) { throw "Setup batch reported failure (exit $setupExit)." }
        }
        Set-MRIState $StatePath $Record 'VERIFYING_FINAL' 'Flushing writes and checking final files.'
        & $guard $true | Out-Null
        Flush-MRIVolume $target.Path
        foreach ($required in @($Config.RequiredFiles)) {
            $path=Join-MRIPath $target.Path $required
            if (-not [IO.File]::Exists($path) -or (Get-Item -LiteralPath $path).Length -eq 0) { throw "Required final file missing or empty: $required" }
        }
        $final=Get-MRIManifest $target.Path -IgnoreSystemDirectories -Guard $guard
        Assert-MRIManifestMatch $SourceManifest $final -MutableFiles @($Config.MutableFiles)
        [void](& $guard $true)
        Write-MRIJson $ReceiptPath ([pscustomobject]@{ Key=$Record.Key; Label=$Record.Label; Status='READY'; Recipe=$recipe
            SourceDigest=$SourceManifest.Digest; SetupExit=$setupExit; CopyExit=$code; VolumePath=$target.Path
            TargetManifest=$final; CompletedUtc=[datetime]::UtcNow.ToString('o') })
        $newReceipt=$true
        [void](& $guard $true)
        Set-MRIState $StatePath $Record 'READY' 'Files verified; required boot files checked; writes flushed. Safely eject in Windows.'
        return [pscustomobject]@{ Success=$true; Label=$Record.Label; AlreadyReady=$false }
    } catch {
        if ($newReceipt -and [IO.File]::Exists($ReceiptPath)) { [IO.File]::Delete($ReceiptPath) }
        Set-MRIState $StatePath $Record 'FAILED' $_.Exception.Message
        return [pscustomobject]@{ Success=$false; Label=$Record.Label; Error=$_.Exception.Message }
    }
}
function Update-MRIArrivals {
    param([hashtable]$Tracker, [array]$Inventory, [array]$Records, $Config, [string[]]$ProtectedKeys=@())
    $present=@{}
    foreach ($d in $Inventory) {
        if (-not $d.Key) { continue }; $present[$d.Key]=$true
        if (-not $Tracker.ContainsKey($d.Key)) { $Tracker[$d.Key]=[pscustomobject]@{ Number=$d.Number; Stable=0; Attempted=$false } }
        $t=$Tracker[$d.Key]
        if ($t.Number -ne $d.Number) { $t.Number=$d.Number; $t.Stable=0 }
        if (Get-MRIDriveBlockReason $d $Inventory $Config $ProtectedKeys) { $t.Stable=0; continue }
        $t.Stable++
        $matches=@($Records | Where-Object { $_.Key -eq $d.Key })
        if ($matches.Count -eq 1 -and $t.Stable -ge $Config.StablePolls -and -not $t.Attempted) { $matches[0] }
    }
    foreach ($key in @($Tracker.Keys)) { if (-not $present.ContainsKey($key)) { $Tracker.Remove($key) } }
}
Export-ModuleMember -Function *-MRI*
