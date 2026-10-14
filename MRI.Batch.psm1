# SPDX-License-Identifier: GPL-3.0-only
# GPL-3.0-only. Updated 10-14-2026 for concurrency and security checks.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-MRIJson {
    param([string]$Path, $Value, [switch]$PassThruHash)
    $Path=[IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($Path)
    [void][IO.Directory]::CreateDirectory($parent)
    $tmp = $Path + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
    $json=ConvertTo-Json -InputObject $Value -Depth 30
    try {
        [IO.File]::WriteAllText($tmp, $json, [Text.UTF8Encoding]::new($false))
        if ([IO.File]::Exists($Path)) { [IO.File]::Replace($tmp, $Path, [System.Management.Automation.Language.NullString]::Value) }
        else { [IO.File]::Move($tmp, $Path) }
    } finally { if ([IO.File]::Exists($tmp)) { [IO.File]::Delete($tmp) } }
    if ($PassThruHash) { return Get-MRIHash $json }
}
function Read-MRIAuthenticatedJson {
    param([string]$Path,[string]$ExpectedHash)
    if ($ExpectedHash -notmatch '^[A-Fa-f0-9]{64}$') { throw 'Missing or invalid expected data hash.' }
    # Read once: hashing then reopening the path would permit a replacement race.
    $json=[IO.File]::ReadAllText($Path)
    if ((Get-MRIHash $json) -ne $ExpectedHash) { throw 'Runtime data changed or is corrupt. No copy or preparation is allowed.' }
    return ($json | ConvertFrom-Json)
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
    foreach ($part in $parts) {
        if ($part -match '^(CON|PRN|AUX|NUL|CLOCK\$|COM[1-9\u00b9\u00b2\u00b3]|LPT[1-9\u00b9\u00b2\u00b3])(?:\.|$)') { throw "Windows device names are not allowed: $Path" }
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
    if ($Config.MaxParallel -lt 1 -or $Config.MaxParallel -gt 30) { throw 'MaxParallel must be 1 through 30.' }
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
        if ($kind -eq 'Folder') {
            Assert-MRIPlainDirectory $context.InputPath
            # The selected folder's contents belong at the USB root. Do not
            # silently flatten a legitimate single child directory.
            $context.Path=$context.InputPath; return $context
        }
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
        $context.Path=[string]$volumes[0].DriveLetter+':\'
        Assert-MRIPlainDirectory $context.Path
        return $context
    } catch { Close-MRISource $context; throw }
}
function Close-MRISource {
    param($Context)
    if ($null -eq $Context) { return }
    if ($Context.OwnsMount) { Dismount-DiskImage -ImagePath $Context.MountedIso -ErrorAction Stop | Out-Null; $Context.OwnsMount=$false }
    if ($Context.CacheDirectory -and [IO.Directory]::Exists($Context.CacheDirectory)) { Remove-Item -LiteralPath $Context.CacheDirectory -Recurse -Force -ErrorAction Stop }
}
function Assert-MRISourceSeparation {
    param($Context,[string]$ApplicationPath)
    $separator=[IO.Path]::DirectorySeparatorChar
    $source=[IO.Path]::GetFullPath($Context.Path).TrimEnd([char[]]@('\','/'))+$separator
    $application=[IO.Path]::GetFullPath($ApplicationPath).TrimEnd([char[]]@('\','/'))+$separator
    $ownedCache=$false
    if ($Context.CacheDirectory) {
        $cache=[IO.Path]::GetFullPath($Context.CacheDirectory).TrimEnd([char[]]@('\','/'))+$separator
        $ownedCache=$source.StartsWith($cache,[StringComparison]::OrdinalIgnoreCase)
    }
    if ($application.StartsWith($source,[StringComparison]::OrdinalIgnoreCase) -or
        ($source.StartsWith($application,[StringComparison]::OrdinalIgnoreCase) -and -not $ownedCache)) {
        throw 'Keep the official MRI source folder separate from the automation folder.'
    }
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
    param($Drive,[bool]$RequireDriveLetter=$true)
    $parts=@(Get-Partition -DiskNumber $Drive.Number -ErrorAction Stop)
    if (@($parts | Where-Object { $_.IsBoot -or $_.IsSystem }).Count -gt 0) { throw 'A boot/system partition was found on the target.' }
    if ($parts.Count -ne 1) { throw 'Copy mode requires exactly one existing partition. Prepare a fresh drive or use the registered erase mode.' }
    $p=$parts[0]
    $volumes=@(Get-Volume -Partition $p -ErrorAction Stop)
    if ($volumes.Count -ne 1) { throw 'The USB needs one formatted volume.' }
    if ($RequireDriveLetter -and [string]::IsNullOrWhiteSpace([string]$p.DriveLetter)) { throw 'The optional setup batch needs a drive letter.' }
    $v=$volumes[0]
    $letter=[string]$p.DriveLetter
    if ($letter -notmatch '^[A-Za-z]$') { $letter='' }
    if ($RequireDriveLetter -and -not $letter) { throw 'The optional setup batch needs a drive letter.' }
    if ([string]$v.Path -notmatch '^\\\\\?\\Volume\{[0-9A-Fa-f-]+\}\\$') { throw 'A stable volume GUID path could not be obtained.' }
    [pscustomobject]@{ Path=[string]$v.Path; Letter=$letter; Number=$Drive.Number
        PartitionNumber=$p.PartitionNumber; FileSystem=[string]$v.FileSystem; FreeBytes=[long]$v.SizeRemaining }
}
function Get-MRIExistingPartitions {
    param([int]$DiskNumber)
    try { Get-Partition -DiskNumber $DiskNumber -ErrorAction Stop }
    catch {
        $originalError=$_
        # Get-Partition throws for a real, newly attached RAW disk. Do not
        # suppress other inventory failures or assume a formatted disk is empty.
        $disk=Get-Disk -Number $DiskNumber -ErrorAction Stop
        if ($disk.Number -eq $DiskNumber -and [string]$disk.PartitionStyle -eq 'RAW') { return }
        throw $originalError
    }
}
function Assert-MRITarget {
    param($Record, $Config, $Target, [string[]]$ProtectedKeys=@())
    $d=Assert-MRIDrive $Record $Config $ProtectedKeys $Target.Number
    $needsLetter=-not [string]::IsNullOrWhiteSpace($Config.SetupBatch)
    $v=Get-MRITargetVolume $d -RequireDriveLetter $needsLetter
    if ($v.Path -ne $Target.Path -or ($needsLetter -and $v.Letter -ne $Target.Letter)) { throw 'The USB volume or drive-letter mapping changed. Build stopped.' }
    return $d
}
function Get-MRIFileHash {
    param([string]$Path,[scriptblock]$Guard)
    $stream=$null; $sha=[Security.Cryptography.SHA256]::Create()
    try {
        $stream=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
        $buffer=New-Object byte[] (1MB)
        while (($read=$stream.Read($buffer,0,$buffer.Length)) -gt 0) {
            if ($Guard) { & $Guard | Out-Null }
            [void]$sha.TransformBlock($buffer,0,$read,$buffer,0)
        }
        [void]$sha.TransformFinalBlock($buffer,0,0)
        return [BitConverter]::ToString($sha.Hash).Replace('-','')
    } finally { if ($stream) { $stream.Dispose() }; $sha.Dispose() }
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
            [void](ConvertTo-MRIRelativePath $rel)
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Links are not allowed in media content: $rel" }
            if ($names.ContainsKey($rel)) { throw "Case-insensitive path collision: $rel" }; $names[$rel]=$true
            $entry=[pscustomobject]@{ RelativePath=$rel; IsDirectory=[bool]$item.PSIsContainer; Length=[long]0; Sha256='' }
            if ($item.PSIsContainer) { $stack.Push($item.FullName) }
            else {
                $entry.Length=[long]$item.Length
                if (-not $NoHash) {
                    if ($Guard) { & $Guard | Out-Null }
                    $beforeLength=$item.Length; $beforeTime=$item.LastWriteTimeUtc
                    $entry.Sha256=Get-MRIFileHash $item.FullName -Guard $Guard
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
            if ((Get-MRIFileHash $path -Guard $Guard) -ne $e.Sha256) { throw "SHA-256 mismatch: $($e.RelativePath)" }
        } elseif ($info.Length -eq 0 -and $e.Length -ne 0) { throw "Setup left an empty mutable file: $($e.RelativePath)" }
        if ($Guard) { & $Guard | Out-Null }
    }
}
function Assert-MRIManifestMatch {
    param($Expected, $Actual, [string[]]$MutableFiles=@(),[switch]$Exact,[switch]$LayoutOnly)
    if ($Exact -and @($Expected.Entries).Count -ne @($Actual.Entries).Count) { throw 'Media contains unexpected or missing paths.' }
    $index=@{}; foreach ($e in @($Actual.Entries)) { $index[$e.RelativePath]=$e }
    foreach ($e in @($Expected.Entries)) {
        if (-not $index.ContainsKey($e.RelativePath)) { throw "Missing final path: $($e.RelativePath)" }
        $a=$index[$e.RelativePath]
        if ($e.IsDirectory -ne $a.IsDirectory) { throw "Final path type mismatch: $($e.RelativePath)" }
        if ($e.IsDirectory) { continue }
        if ($e.RelativePath -notin $MutableFiles) {
            if ($e.Length -ne $a.Length -or (-not $LayoutOnly -and $e.Sha256 -ne $a.Sha256)) { throw "Final file mismatch: $($e.RelativePath)" }
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
function Invoke-MRICopy {
    param($Config,$Target,$Manifest,[string]$WorkPath,[scriptblock]$Guard,[string]$CancelPath)
    $planPath=Join-Path $WorkPath 'copy-plan.json'
    $planHash=Write-MRIJson $planPath ([pscustomobject]@{SourceRoot=$Config.SourcePath;TargetRoot=$Target.Path;Manifest=$Manifest}) -PassThruHash
    $arguments=@('-NoLogo','-NoProfile','-ExecutionPolicy','RemoteSigned','-File',(Join-Path $PSScriptRoot 'Copy-MRIFiles.ps1'),'-PlanPath',$planPath,'-ExpectedHash',$planHash)
    $command=($arguments | ForEach-Object { [MRIBatch.NativeProcess]::Quote([string]$_) }) -join ' '
    return Invoke-MRICommand (Get-MRIWindowsExecutable 'WindowsPowerShell\v1.0\powershell.exe') $command $Config.SourcePath (Join-Path $WorkPath 'copy.log') $Config.CopyTimeoutSeconds $Guard -CancelPath $CancelPath
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
    $needsLetter=-not [string]::IsNullOrWhiteSpace($Config.SetupBatch)
    $part=New-Partition -InputObject $actual -Size $size -IsActive -AssignDriveLetter:$needsLetter -ErrorAction Stop
    [void](Get-MRIWritableDisk $Record $Config $ProtectedKeys $ExpectedNumber $CancelPath)
    $part | Format-Volume -FileSystem FAT32 -NewFileSystemLabel $Record.Label -Force -Confirm:$false -ErrorAction Stop | Out-Null
    return Get-MRITargetVolume (Assert-MRIDrive $Record $Config $ProtectedKeys $ExpectedNumber) -RequireDriveLetter $needsLetter
}
function Get-MRIRecipe {
    param($Config,$SourceManifest)
    # Operational settings do not change the copied media. Moving an identical
    # source or changing concurrency/timeouts must not force a rebuild.
    $media=[ordered]@{Version=2;PrepareDisk=$Config.PrepareDisk;FileSystem=$Config.FileSystem
        SetupBatch=$Config.SetupBatch;SetupArguments=@($Config.SetupArguments);SetupInteractive=$Config.SetupInteractive
        RequiredFiles=@($Config.RequiredFiles);MutableFiles=@($Config.MutableFiles);SourceDigest=$SourceManifest.Digest}
    return Get-MRIHash (ConvertTo-Json $media -Depth 8 -Compress)
}
function Test-MRILegacyDirectReceipt {
    param($Receipt,$Config,$SourceManifest,$Target)
    if ($null -ne $Receipt.PSObject.Properties['RecipeVersion'] -or $Receipt.SetupExit -ne $null -or
        $Receipt.SourceDigest -ne $SourceManifest.Digest -or $Target.FileSystem -ne 'FAT32') { return $false }
    try { Assert-MRIManifestMatch $SourceManifest $Receipt.TargetManifest -Exact } catch { return $false }
    if ($Config.PrepareDisk) {
        $disk=Get-Disk -Number $Target.Number -ErrorAction Stop
        $partition=@(Get-Partition -DiskNumber $Target.Number -PartitionNumber $Target.PartitionNumber -ErrorAction Stop)
        if ([string]$disk.PartitionStyle -ne 'MBR' -or $partition.Count -ne 1 -or -not $partition[0].IsActive) { return $false }
    }
    return $true
}
function Start-MRIBuildJob {
    param([string]$ModulePath,$Record,[string]$ConfigPath,[string]$ConfigHash,[string]$ManifestPath,[string]$ManifestHash,
        [string[]]$ProtectedKeys,[string]$StatePath,[string]$ReceiptPath,[string]$WorkPath,[string]$CancelPath)
    return Start-Job -ScriptBlock {
        param($module,$record,$snapshot,$configHash,$manifestPath,$manifestHash,$protected,$statePath,$receiptPath,$work,$cancel)
        $ErrorActionPreference='Stop'
        Import-Module $module -Force -DisableNameChecking
        $config=Read-MRIAuthenticatedJson $snapshot $configHash
        $manifest=Read-MRIAuthenticatedJson $manifestPath $manifestHash
        Invoke-MRIBuild $record $config $manifest @($protected) $statePath $receiptPath $work $cancel
    } -ArgumentList $ModulePath,$Record,$ConfigPath,$ConfigHash,$ManifestPath,$ManifestHash,$ProtectedKeys,$StatePath,$ReceiptPath,$WorkPath,$CancelPath
}
function Select-MRIBuilds {
    param([array]$Arrivals,[hashtable]$Jobs,[int]$MaxParallel)
    if ($MaxParallel -lt 1 -or $MaxParallel -gt 30) { throw 'Use 1 through 30 simultaneous builds.' }
    $remaining=[Math]::Max(0,$MaxParallel-$Jobs.Count)
    $selected=@{}
    foreach ($record in $Arrivals) {
        if ($remaining -eq 0) { break }
        if ($Jobs.ContainsKey($record.Key) -or $selected.ContainsKey($record.Key)) { continue }
        $selected[$record.Key]=$true; $remaining--; $record
    }
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
        if (-not @($SourceManifest.Entries | Where-Object { -not $_.IsDirectory }).Count) { throw 'The source contains no files. Preparation was not started.' }
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
        $parts=@(Get-MRIExistingPartitions $number)
        if ($parts.Count -gt 0) {
            $target=Get-MRITargetVolume $drive -RequireDriveLetter $runSetup
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
        $recipe=Get-MRIRecipe $Config $SourceManifest
        $sameRecipe=$false
        if ($receipt -and $target) {
            $sameRecipe=$receipt.Recipe -eq $recipe -or $receipt.Recipe -eq (Get-MRIHash ($identity+$SourceManifest.Digest))
            if (-not $sameRecipe -and -not $runSetup) { $sameRecipe=Test-MRILegacyDirectReceipt $receipt $Config $SourceManifest $target }
        }
        if ($alreadyVerified -and $receipt -and $target -and $sameRecipe -and $receipt.VolumePath -eq $target.Path) {
            # Full previous-output verification above prevents trusting a stale completion flag.
            & $guard $true | Out-Null
            Flush-MRIVolume $target.Path
            [void](& $guard $true)
            if ($receipt.Recipe -ne $recipe) {
                $receipt.Recipe=$recipe; $receipt | Add-Member -NotePropertyName RecipeVersion -NotePropertyValue 2 -Force
                Write-MRIJson $ReceiptPath $receipt
            }
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
        $code=Invoke-MRICopy $Config $target $SourceManifest $WorkPath $guard $CancelPath
        if ($code -ne 0) { throw "The copy worker reported failure (exit $code)." }
        Set-MRIState $StatePath $Record 'VERIFYING_COPY' 'Checking copied paths and file sizes; SHA-256 verification follows the write flush.'
        $copied=Get-MRIManifest $target.Path -IgnoreSystemDirectories -NoHash:(-not $runSetup) -Guard $guard
        if (@($copied.Entries).Count -ne @($SourceManifest.Entries).Count) { throw 'Copied media contains paths absent from the source snapshot. Setup was not started.' }
        Assert-MRIManifestMatch $SourceManifest $copied -Exact -LayoutOnly:(-not $runSetup)
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
        Assert-MRIManifestMatch $SourceManifest $final -MutableFiles @($Config.MutableFiles) -Exact:(-not $runSetup)
        [void](& $guard $true)
        Write-MRIJson $ReceiptPath ([pscustomobject]@{ Key=$Record.Key; Label=$Record.Label; Status='READY'; Recipe=$recipe; RecipeVersion=2
            SourceDigest=$SourceManifest.Digest; SetupExit=$setupExit; CopyExit=$code; VolumePath=$target.Path
            TargetManifest=$final; CompletedUtc=[datetime]::UtcNow.ToString('o') })
        $newReceipt=$true
        [void](& $guard $true)
        Set-MRIState $StatePath $Record 'READY' 'Files verified; writes flushed. Safely eject in Windows.'
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
