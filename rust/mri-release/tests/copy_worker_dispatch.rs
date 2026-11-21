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
