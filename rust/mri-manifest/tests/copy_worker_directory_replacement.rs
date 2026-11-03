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

    Write-Host 'PASS: Original copy-worker conditional rejects both source type replacements.'
}
finally {
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
