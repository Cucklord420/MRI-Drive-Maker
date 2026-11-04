#![cfg(windows)]

// Synthetic original-PowerShell ZIP-cache regression. Runs only on a temporary
// fixture root; volume readings are mocked after validating the Windows API
// lookup independently. No USB or actual low-space disk manipulation.
use std::path::PathBuf;
use std::process::Command;

const WINDOWS_FIXTURE: &str = r#"
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    $env:MRI_ORIGINAL_MODULE, [ref]$tokens, [ref]$parseErrors
)
if (@($parseErrors).Count -ne 0) { throw 'Original MRI module has PowerShell parse errors.' }

foreach ($name in @('Expand-MRIArchive', 'Open-MRISource', 'Close-MRISource')) {
    $defs = @($ast.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq $name
    }, $true))
    if ($defs.Count -ne 1) { throw "Expected exactly one original $name definition." }
    . ([ScriptBlock]::Create($defs[0].Extent.Text))
}

# Other module functions are not the subject of this test; supply safe,
# fixture-only equivalents so the actual ZIP extraction functions run.
function ConvertTo-MRIRelativePath {
    param([string]$Relative)
    if (-not $Relative -or $Relative.Contains('..')) { throw 'Invalid synthetic ZIP path.' }
    return $Relative
}
function Join-MRIPath { param([string]$Parent, [string]$Child) Join-Path $Parent $Child }
function Assert-MRIPlainDirectory {
    param([string]$Path)
    if (-not [IO.Directory]::Exists($Path)) { throw 'Fixture directory does not exist.' }
}
function Assert-MRIConfig { param($Config) }
function Get-MRISourceKind { param($Config) 'ZIP' }
function Get-MRIMediaRoot { param([string]$Root) throw 'Unexpected complete extraction.' }

$script:AvailableBytes = [long]1
$script:FailVolumeQuery = $false
$script:VolumeQueries = 0
function Get-Volume {
    [CmdletBinding()]
    param([string]$FilePath)
    $script:VolumeQueries++
    if ($script:FailVolumeQuery) { throw 'Synthetic volume query unavailable.' }
    if (-not [IO.Directory]::Exists($FilePath)) {
        throw 'Volume lookup did not use an existing cache-parent directory.'
    }
    return [pscustomobject]@{ SizeRemaining = $script:AvailableBytes }
}

function New-SyntheticZip {
    param([string]$Path, [string[]]$Entries)
    $zip = [IO.Compression.ZipFile]::Open($Path, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($name in $Entries) {
            $entry = $zip.CreateEntry($name)
            $writer = [IO.StreamWriter]::new($entry.Open())
            try { $writer.Write('fixture bytes') } finally { $writer.Dispose() }
        }
    } finally { $zip.Dispose() }
}

function Set-ZipAdvertisedSizesNearFourGiB {
    param([string]$Path, [int]$ExpectedCount)
    # Only modify central-directory uncompressed-size metadata. ZIP entries
    # remain a few synthetic bytes; never allocate the advertised gigabytes.
    [byte[]]$bytes = [IO.File]::ReadAllBytes($Path)
    $patched = 0
    for ($i = 0; $i -le ($bytes.Length - 46); $i++) {
        if ($bytes[$i] -eq 0x50 -and $bytes[$i + 1] -eq 0x4b -and
            $bytes[$i + 2] -eq 0x01 -and $bytes[$i + 3] -eq 0x02) {
            $bytes[$i + 24] = 0xFE
            $bytes[$i + 25] = 0xFF
            $bytes[$i + 26] = 0xFF
            $bytes[$i + 27] = 0xFF
            $patched++
        }
    }
    if ($patched -ne $ExpectedCount) { throw 'Synthetic ZIP metadata patch count was incorrect.' }
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function Expect-NoExtract {
    param([string]$Zip, [string]$Target, [string]$ExpectedMessage)
    $before = $script:VolumeQueries
    $rejected = $false
    $observed = 'no exception'
    try { Expand-MRIArchive $Zip $Target }
    catch {
        $observed = $_.Exception.ToString()
        $rejected = $_.Exception.Message.Contains($ExpectedMessage)
    }
    if (-not $rejected) {
        $actualCommand = (Get-Command Expand-MRIArchive).ScriptBlock.ToString()
        $hasGuard = $actualCommand.Contains('Insufficient free space')
        throw "ZIP preflight mismatch ($ExpectedMessage); observed: $observed; available=$script:AvailableBytes; volume-queries=$script:VolumeQueries; guard-present=$hasGuard; output-exists=$([IO.Directory]::Exists($Target))"
    }
    if ([IO.Directory]::Exists($Target)) {
        throw 'Rejected ZIP created a cache directory before preflight.'
    }
    if ($script:VolumeQueries -le $before -and $ExpectedMessage -ne 'supported source size') {
        throw 'The original extractor never queried the cache volume.'
    }
}

$root = Join-Path ([IO.Path]::GetTempPath()) ('mri-cache-space-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
try {
    # A real Windows Storage query confirms this production lookup works on
    # an existing directory before it is mocked for deterministic low-space tests.
    $realVolume = @(Storage\Get-Volume -FilePath $root -ErrorAction Stop)
    if ($realVolume.Count -ne 1 -or [long]$realVolume[0].SizeRemaining -lt 0) {
        throw 'Windows Storage lookup did not identify one usable cache volume.'
    }

    $cache = Join-Path $root 'cache'
    [void][IO.Directory]::CreateDirectory($cache)
    $sentinel = Join-Path $cache 'unrelated.txt'
    [IO.File]::WriteAllText($sentinel, 'preserve')
    $zipPath = Join-Path $root 'small.zip'
    New-SyntheticZip $zipPath @('payload.txt')

    $script:AvailableBytes = [long]1
    Expect-NoExtract $zipPath (Join-Path $cache 'low') 'Insufficient free space'
    $script:AvailableBytes = [long](1GB)
    Expect-NoExtract $zipPath (Join-Path $cache 'headroom') 'Insufficient free space'
    $script:AvailableBytes = [long]-1
    Expect-NoExtract $zipPath (Join-Path $cache 'negative') 'available cache space'
    $script:AvailableBytes = $null
    Expect-NoExtract $zipPath (Join-Path $cache 'unknown') 'available cache space'
    $script:FailVolumeQuery = $true
    Expect-NoExtract $zipPath (Join-Path $cache 'unavailable') 'Synthetic volume query unavailable'
    $script:FailVolumeQuery = $false

    # Sixteen tiny entries can advertise 64 GiB minus 32 bytes, whereas
    # seventeen exceed the 64 GiB ceiling. Both tests write only small ZIPs.
    $near = Join-Path $root 'near-limit.zip'
    New-SyntheticZip $near @((1..16) | ForEach-Object { "part$_.bin" })
    Set-ZipAdvertisedSizesNearFourGiB $near 16
    $script:AvailableBytes = [long]1
    Expect-NoExtract $near (Join-Path $cache 'near-limit') 'Insufficient free space'

    $above = Join-Path $root 'above-limit.zip'
    New-SyntheticZip $above @((1..17) | ForEach-Object { "part$_.bin" })
    Set-ZipAdvertisedSizesNearFourGiB $above 17
    $before = $script:VolumeQueries
    $sizeRejected = $false
    try { Expand-MRIArchive $above (Join-Path $cache 'above-limit') }
    catch { $sizeRejected = $_.Exception.Message.Contains('supported source size') }
    if (-not $sizeRejected -or [IO.Directory]::Exists((Join-Path $cache 'above-limit'))) {
        throw 'An over-limit ZIP was not rejected before extraction.'
    }
    if ($script:VolumeQueries -ne $before) {
        throw 'Over-limit ZIP metadata did not fail before the volume query.'
    }

    $script:AvailableBytes = [long](3GB)
    $good = Join-Path $cache 'good'
    Expand-MRIArchive $zipPath $good
    if ([IO.File]::ReadAllText((Join-Path $good 'payload.txt')) -cne 'fixture bytes') {
        throw 'Safe ZIP extraction did not preserve the synthetic payload.'
    }
    [IO.Directory]::Delete($good, $true)

    # Valid ZIP with conflicting file/child paths: first entry extracts, the
    # second cannot create a directory over that file. Open-MRISource must
    # remove only its own temporary cache directory after the extraction error.
    $conflict = Join-Path $root 'partial.zip'
    New-SyntheticZip $conflict @('occupied', 'occupied/child.txt')
    $config = [pscustomobject]@{ SourcePath = $conflict }
    $threw = $false
    try { $unused = Open-MRISource $config $cache }
    catch { $threw = $true }
    if (-not $threw) { throw 'Conflict ZIP failed to trigger extraction cleanup.' }
    if ([IO.Directory]::GetDirectories($cache).Count -ne 0) {
        throw 'Failed extraction left a partial owned cache directory.'
    }
    if ([IO.File]::ReadAllText($sentinel) -cne 'preserve') {
        throw 'Extraction cleanup removed unrelated cache data.'
    }
    Write-Host 'PASS: original ZIP-cache extraction preflight, headroom and owned cleanup.'
}
finally {
    [IO.Directory]::Delete($root, $true)
}
"#;

#[test]
fn original_zip_cache_fails_closed_on_low_space_and_cleans_partial_extracts() {
    let module = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("MRI.Batch.psm1");
    assert!(module.is_file(), "Original MRI PowerShell module is missing");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_FIXTURE])
        .env("MRI_ORIGINAL_MODULE", &module)
        .output()
        .expect("Windows PowerShell 5.1 is required for ZIP-cache parity");
    assert!(
        output.status.success(),
        "Original ZIP-cache safety regression failed. stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
