#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::tempdir;

fn app_fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempdir().unwrap();
    let app = temp.path().join("App");
    fs::create_dir(&app).unwrap();

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["Copy-MRIFiles.ps1", "MRI.Batch.psm1", "NativeProcess.cs"] {
        fs::copy(repo.join(name), app.join(name)).unwrap();
    }

    (temp, app)
}

fn run_copy_script(app: &Path) -> Output {
    let plan = app.join("copy-plan.json");
    fs::write(&plan, b"{}\n").unwrap();
    Command::new(
        PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .args([
        "-NoLogo",
        "-NoProfile",
        "-ExecutionPolicy",
        "RemoteSigned",
        "-File",
    ])
    .arg(app.join("Copy-MRIFiles.ps1"))
    .arg("-PlanPath")
    .arg(plan)
    .arg("-ExpectedHash")
    .arg("0000000000000000000000000000000000000000000000000000000000000000")
    .current_dir(app)
    .output()
    .expect("Windows PowerShell 5.1 should start")
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn preflight_mode_starts_without_filesystem_side_effects() {
    let temp = tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mri-copy"))
        .arg("--preflight")
        .current_dir(temp.path())
        .output()
        .expect("the Rust copy worker must start its preflight mode");

    assert!(
        output.status.success(),
        "copy-worker preflight failed: {}",
        combined_output(&output)
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn missing_copy_worker_stops_before_any_disk_preparation() {
    let (_temp, app) = app_fixture();
    let sentinel = app.join("disk-preparation-reached.txt");
    let module_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("MRI.Batch.psm1");
    let module_source = fs::read_to_string(module_path).unwrap();
    let marker = "Export-ModuleMember -Function *-MRI*";
    let test_dependencies = r#"
function Assert-MRINotCancelled { param($CancelPath) }
function Assert-MRIConfig { param($Config) }
function Get-MRISourceKind { param($Config); 'Folder' }
function Get-MRIResolvedProtectedPaths { param($Config, $ProtectedPaths, $WorkPath, $StatePath, $ReceiptPath); @('C:\synthetic') }
function Initialize-MRINative {}
function Set-MRIState { param($StatePath, $Record, $State, $Message) }
function Assert-MRIDrive { param($Record, $Config, $ProtectedKeys, $ApprovedEraseNumber); [pscustomobject]@{ Number = 7; Size = 1000000000; Key = 'synthetic' } }
function Get-MRIExistingPartitions { param($Number); @() }
function Get-MRIRecipe { param($Config, $SourceManifest); 'synthetic-recipe' }
function Initialize-MRITarget {
    param($Record, $Config, $ProtectedKeys, $ExpectedNumber, $CancelPath, $ProtectedPaths)
    [IO.File]::WriteAllText([Environment]::GetEnvironmentVariable('MRI_TEST_SENTINEL'), 'reached')
    throw 'Target preparation reached.'
}
Export-ModuleMember -Function *-MRI*
"#;
    assert!(module_source.contains(marker));
    let fixture_module = module_source.replace(marker, test_dependencies);
    fs::write(app.join("MRI.Batch.psm1"), fixture_module).unwrap();
    let script = r#"
$ErrorActionPreference = 'Stop'
Import-Module -Name $env:MRI_TEST_MODULE -Force -DisableNameChecking
$record = [pscustomobject]@{ AllowErase = $true; Key = 'synthetic'; Label = 'fixture' }
$config = [pscustomobject]@{
    PrepareDisk = $true
    SourcePath = 'C:\synthetic'
    FileSystem = 'FAT32'
    SetupBatch = ''
    SetupArguments = @()
    SetupInteractive = $false
    RequiredFiles = @()
    MutableFiles = @()
}
$manifest = [pscustomobject]@{
    Entries = @([pscustomobject]@{ IsDirectory = $false; RelativePath = 'boot.bin'; Length = 1 })
    TotalBytes = 1
    Digest = 'synthetic-digest'
}
$workPath = Join-Path $env:TEMP ('mri-preflight-' + [guid]::NewGuid().ToString('N'))
$result = Invoke-MRIBuild $record $config $manifest @() (Join-Path $workPath 'state.json') (Join-Path $workPath 'receipt.json') $workPath '' -ProtectedPaths @('C:\synthetic')
if ($result.Success -ne $false -or $result.Error -notlike '*Rust copy worker*') { throw 'Missing helper did not stop the build before preparation.' }
if ([IO.File]::Exists($env:MRI_TEST_SENTINEL)) { throw 'Target preparation ran before copy-worker preflight.' }
Write-Output 'PASS missing Rust copy worker stops before disk preparation.'
"#;
    let output = Command::new(
        PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .args([
        "-NoLogo",
        "-NoProfile",
        "-ExecutionPolicy",
        "RemoteSigned",
        "-Command",
        script,
    ])
    .env("MRI_TEST_MODULE", app.join("MRI.Batch.psm1"))
    .env("MRI_TEST_SENTINEL", &sentinel)
    .output()
    .expect("Windows PowerShell 5.1 should run the missing-worker fixture");

    assert!(
        output.status.success(),
        "missing-worker preflight failed: {}",
        combined_output(&output)
    );
    assert!(!sentinel.exists());
}

#[test]
fn copy_script_runs_the_packaged_sibling_helper() {
    let (_temp, app) = app_fixture();
    fs::copy(env!("CARGO_BIN_EXE_mri-copy"), app.join("mri-copy.exe")).unwrap();

    let output = run_copy_script(&app);

    assert!(!output.status.success());
    assert!(combined_output(&output)
        .contains("Runtime data changed or is corrupt. No copy is allowed."));
}

#[test]
fn copy_script_does_not_search_debug_or_release_build_artifacts() {
    for profile in ["debug", "release"] {
        let (_temp, app) = app_fixture();
        let profile_worker = app.join(format!("rust/mri-release/target/{profile}"));
        fs::create_dir_all(&profile_worker).unwrap();
        fs::write(
            app.join("rust/mri-release/Cargo.toml"),
            b"[package]\nname='fixture'\n",
        )
        .unwrap();
        fs::copy(
            env!("CARGO_BIN_EXE_mri-copy"),
            profile_worker.join("mri-copy.exe"),
        )
        .unwrap();

        let output = run_copy_script(&app);

        assert!(!output.status.success());
        assert!(
            !combined_output(&output)
                .contains("Runtime data changed or is corrupt. No copy is allowed."),
            "the script ran the {profile} worker despite the missing packaged sibling"
        );
    }
}
