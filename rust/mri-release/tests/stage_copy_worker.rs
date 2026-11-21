#[cfg(windows)]
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

fn run_stage(destination: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mri-release"))
        .arg("--stage-copy-worker")
        .arg("--output")
        .arg(destination)
        .output()
        .expect("the release tool should start")
}

#[cfg(windows)]
#[test]
fn stages_the_embedded_copy_worker_to_the_requested_path() {
    let temp = tempdir().unwrap();
    let destination = temp.path().join("mri-copy.exe");

    let output = run_stage(&destination);

    assert!(
        output.status.success(),
        "stage command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fs::metadata(&destination).unwrap().len() > 0);
    assert!(run_stage(&destination).status.success());
}

#[cfg(windows)]
#[test]
fn refuses_to_replace_a_different_existing_copy_worker() {
    let temp = tempdir().unwrap();
    let destination = temp.path().join("mri-copy.exe");
    let existing = b"keep this existing file";
    fs::write(&destination, existing).unwrap();

    let output = run_stage(&destination);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("Refusing to replace an existing copy worker"));
    assert_eq!(fs::read(destination).unwrap(), existing);
}

#[cfg(not(windows))]
#[test]
fn staging_without_a_windows_copy_worker_fails_without_creating_a_file() {
    let temp = tempdir().unwrap();
    let destination = temp.path().join("mri-copy.exe");

    let output = run_stage(&destination);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("No embedded Rust copy worker is available to stage"));
    assert!(!destination.exists());
}
