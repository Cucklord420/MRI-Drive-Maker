#![cfg(windows)]

// Synthetic Mark-of-the-Web regression for the shipped PowerShell entry point.
// All work happens inside an owned temporary extracted ZIP. No USB is accessed.
use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Output};
use zip::ZipArchive;

fn run_original_script(script: &Path) -> Output {
    Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "RemoteSigned",
            "-File",
        ])
        .arg(script)
        .args(["-Action", "SelfTest"])
        .output()
        .expect("Windows PowerShell 5.1 must be available")
}

#[test]
fn downloaded_script_is_blocked_until_trusted_fixture_is_explicitly_unblocked() {
    let temp = tempfile::tempdir().expect("owned temporary release fixture");
    let archive = temp.path().join("candidate.zip");
    let release_binary = env!("CARGO_BIN_EXE_mri-release");

    let built = Command::new(release_binary)
        .args(["--version", "0.3.2", "--output"])
        .arg(&archive)
        .output()
        .expect("the Rust release builder must launch");
    assert!(
        built.status.success(),
        "candidate build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let verified = Command::new(release_binary)
        .arg("--verify")
        .arg(&archive)
        .output()
        .expect("the Rust release verifier must launch");
    assert!(
        verified.status.success(),
        "candidate verification failed: {}",
        String::from_utf8_lossy(&verified.stderr)
    );

    let mut zip = ZipArchive::new(File::open(&archive).expect("built archive"))
        .expect("valid release archive");
    zip.extract(temp.path()).expect("extract owned release ZIP");
    let script = temp
        .path()
        .join("MRI-Drive-Maker-v0.3.2")
        .join("App")
        .join("MRI-Batch.ps1");
    assert!(
        script.is_file(),
        "the packaged PowerShell entry point is missing"
    );

    // Confirm this test does not silently fall back to a weaker effective policy.
    let policy = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "RemoteSigned",
            "-Command",
            "Get-ExecutionPolicy",
        ])
        .output()
        .expect("query effective execution policy");
    assert!(
        policy.status.success(),
        "could not inspect execution policy"
    );
    assert_eq!(
        String::from_utf8_lossy(&policy.stdout).trim(),
        "RemoteSigned",
        "the runner cannot exercise the requested RemoteSigned policy"
    );

    // NTFS alternate data streams model the Internet-zone label that Windows
    // Explorer may propagate from a browser-downloaded ZIP.
    let zone = format!("{}:Zone.Identifier", script.display());
    let label = "[ZoneTransfer]\r\nZoneId=3\r\n";
    fs::write(&zone, label).expect("create synthetic Internet-zone stream");
    assert_eq!(
        fs::read_to_string(&zone).expect("read synthetic Internet-zone stream"),
        label
    );

    let blocked = run_original_script(&script);
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&blocked.stdout),
        String::from_utf8_lossy(&blocked.stderr)
    );
    assert!(
        !blocked.status.success()
            && diagnostic.to_ascii_lowercase().contains("digitally signed"),
        "Internet-zone-marked unsigned script did not fail for the expected signing reason: {diagnostic}"
    );

    // Unblock only the owned verified fixture, not the runner's global policy.
    // Production instructions require the operator to verify trust first.
    let unblock = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Unblock-File -LiteralPath $env:MRI_TEST_SCRIPT -ErrorAction Stop",
        ])
        .env("MRI_TEST_SCRIPT", &script)
        .output()
        .expect("invoke the built-in per-file unblock cmdlet");
    assert!(
        unblock.status.success(),
        "could not unblock trusted synthetic fixture: {}",
        String::from_utf8_lossy(&unblock.stderr)
    );

    let restored = run_original_script(&script);
    assert!(
        restored.status.success(),
        "unblocked packaged PowerShell self-test failed: stdout={} stderr={}",
        String::from_utf8_lossy(&restored.stdout),
        String::from_utf8_lossy(&restored.stderr)
    );
}
