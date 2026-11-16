#![cfg(windows)]

// Synthetic source-type regression for the existing Windows copy worker.
// The original conditional is parsed from the shipped script; no USB is used.
use std::path::PathBuf;
use std::process::Command;

const WINDOWS_FIXTURE: &str = r#"
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    $env:MRI_COPY_WORKER_PATH, [ref]$tokens, [ref]$parseErrors
)
if (@($parseErrors).Count -ne 0) { throw 'Copy worker failed PowerShell parsing.' }

$branches = @($ast.FindAll({
    param($node)
    $node -is [System.Management.Automation.Language.IfStatementAst] -and
    $node.Extent.Text.StartsWith('if ($entry.IsDirectory)')
}, $true))
if ($branches.Count -ne 1) { throw 'Expected exactly one original copy-worker source-type conditional.' }
$originalBranch = [ScriptBlock]::Create($branches[0].Extent.Text)

$linkBranches = @($ast.FindAll({
    param($node)
    $node -is [System.Management.Automation.Language.IfStatementAst] -and
    $node.Extent.Text.StartsWith('if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)')
}, $true))
if ($linkBranches.Count -ne 1) { throw 'Expected the original copy-worker reparse-point guard.' }
$originalLinkGuard = [ScriptBlock]::Create($linkBranches[0].Extent.Text)

$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('mri-dir-type-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($fixtureRoot)
try {
    $source = Join-Path $fixtureRoot 'previously-a-directory'
    [IO.File]::WriteAllText($source, 'ordinary replacement file')
    $item = Get-Item -LiteralPath $source -Force
    if ($item.PSIsContainer) { throw 'Invalid file fixture.' }

    $target = Join-Path $fixtureRoot 'must-not-create'
    $entry = [pscustomobject]@{ IsDirectory = $true; Length = 0 }
    $rejected = $false
    try { & $originalBranch | Out-Null }
    catch { $rejected = $_.Exception.Message -match 'source changed before copying' }
    if (-not $rejected) { throw 'Directory manifest accepted a replacement file.' }
    if ([IO.Directory]::Exists($target)) { throw 'A target directory was created for a replaced source.' }

    $item = Get-Item -LiteralPath $fixtureRoot -Force
    if (-not $item.PSIsContainer) { throw 'Invalid directory fixture.' }
    $entry.IsDirectory = $false
    $rejected = $false
    try { & $originalBranch | Out-Null }
    catch { $rejected = $_.Exception.Message -match 'source changed before copying' }
    if (-not $rejected) { throw 'File manifest accepted a replacement directory.' }

    $outside = Join-Path $fixtureRoot 'owned-junction-target'
    $replacedDirectory = Join-Path $fixtureRoot 'replaced-directory'
    [void][IO.Directory]::CreateDirectory($outside)
    [void][IO.Directory]::CreateDirectory($replacedDirectory)
    [IO.File]::WriteAllText((Join-Path $outside 'sentinel.txt'), 'owned synthetic bytes')
    [IO.Directory]::Delete($replacedDirectory)
    & cmd.exe /D /C mklink /J $replacedDirectory $outside | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not create an owned directory-junction fixture.' }
    $item = Get-Item -LiteralPath $replacedDirectory -Force
    if (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Junction fixture did not have the reparse-point attribute.'
    }
    $entry.IsDirectory = $true
    $rejected = $false
    try { & $originalLinkGuard | Out-Null }
    catch { $rejected = $_.Exception.Message -match 'source became a link' }
    if (-not $rejected) { throw 'Copy worker accepted a directory replaced by a junction.' }
    if ([IO.File]::ReadAllText((Join-Path $outside 'sentinel.txt')) -cne 'owned synthetic bytes') {
        throw 'The outside fixture was altered.'
    }

    Write-Host 'PASS: Copy worker rejects mismatched source types and linked directories.'
}
finally {
    # .NET Framework's recursive delete cannot remove a Windows junction.
    # Unlink only our synthetic reparse point, without traversing its target.
    $ownedJunction = Join-Path $fixtureRoot 'replaced-directory'
    if ([IO.Directory]::Exists($ownedJunction) -and
        ([IO.File]::GetAttributes($ownedJunction) -band [IO.FileAttributes]::ReparsePoint)) {
        [IO.Directory]::Delete($ownedJunction)
    }
    [IO.Directory]::Delete($fixtureRoot, $true)
}
"#;

#[test]
fn copy_worker_rejects_file_replacing_manifest_directory() {
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("Copy-MRIFiles.ps1");
    assert!(worker.is_file(), "Original Windows copy worker is missing");

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_FIXTURE])
        .env("MRI_COPY_WORKER_PATH", &worker)
        .output()
        .expect("Windows PowerShell 5.1 is required for copy-worker parity");

    assert!(
        output.status.success(),
        "Synthetic original-worker source-type fixture failed. stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
