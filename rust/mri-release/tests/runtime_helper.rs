#![cfg(windows)]

use sha2::Digest;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use zip::ZipArchive;

#[test]
fn packaged_copy_worker_captures_source_identity_without_an_external_snapshot() {
    let temp = tempfile::tempdir().expect("owned synthetic fixture");
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/payload.bin"), b"synthetic payload").unwrap();

    let archive = temp.path().join("runtime-package.zip");
    let release_binary = env!("CARGO_BIN_EXE_mri-release");
    let built = Command::new(release_binary)
        .arg("--output")
        .arg(&archive)
        .output()
        .expect("the native Rust release builder must launch");
    assert!(
        built.status.success(),
        "runtime package build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let verified = Command::new(release_binary)
        .arg("--verify")
        .arg(&archive)
        .output()
        .expect("the Rust release verifier must launch");
    assert!(
        verified.status.success(),
        "runtime package verification failed: {}",
        String::from_utf8_lossy(&verified.stderr)
    );

    let verifier_copy_dir = temp.path().join("verifier-copy");
    fs::create_dir(&verifier_copy_dir).unwrap();
    let verifier_copy = verifier_copy_dir.join("mri-release.exe");
    fs::copy(release_binary, &verifier_copy).unwrap();
    let helper_path = Path::new(release_binary)
        .parent()
        .unwrap()
        .join("mri-copy.exe");
    let helper_copy = verifier_copy_dir.join("mri-copy.exe");
    let mut helper_bytes = fs::read(helper_path).unwrap();
    assert!(helper_bytes.len() > 127);
    helper_bytes[127] ^= 0x5a;
    fs::write(helper_copy, helper_bytes).unwrap();
    let rejected_helper = Command::new(verifier_copy)
        .arg("--verify")
        .arg(&archive)
        .output()
        .expect("the copied release verifier must launch");
    assert!(!rejected_helper.status.success());
    assert!(String::from_utf8_lossy(&rejected_helper.stderr).contains("build-time SHA-256"));

    let mut package = ZipArchive::new(File::open(&archive).unwrap()).unwrap();
    let mut release_manifest = String::new();
    package
        .by_name("MRI-Drive-Maker-v0.3.0/RELEASE.json")
        .unwrap()
        .read_to_string(&mut release_manifest)
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&release_manifest).unwrap();
    assert!(manifest["generated_files"]["App/mri-copy.exe"].is_array());

    package.extract(temp.path()).unwrap();
    let helper = temp.path().join("MRI-Drive-Maker-v0.3.0/App/mri-copy.exe");
    let self_verification = Command::new(&helper)
        .arg("--verify")
        .arg(&archive)
        .output()
        .expect("the packaged copy worker must launch");
    assert_eq!(self_verification.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&self_verification.stderr).contains("Usage: mri-copy"));

    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    let plan = serde_json::json!({
        "SourceRoot": source,
        "TargetRoot": target,
        "Manifest": {
            "Entries": [
                {
                    "RelativePath": "nested",
                    "IsDirectory": true,
                    "Length": 0,
                    "Sha256": ""
                },
                {
                    "RelativePath": "nested\\payload.bin",
                    "IsDirectory": false,
                    "Length": 17,
                    "Sha256": format!("{:x}", sha2::Sha256::digest(b"synthetic payload"))
                }
            ]
        }
    });
    let plan_bytes = serde_json::to_vec(&plan).unwrap();
    let expected_hash = format!("{:x}", sha2::Sha256::digest(&plan_bytes));
    let plan_path = temp.path().join("copy-plan.json");
    fs::write(&plan_path, plan_bytes).unwrap();
    let rejected = Command::new(&helper)
        .arg("--copy")
        .arg(&plan_path)
        .arg(expected_hash)
        .output()
        .expect("the packaged Rust copy entry point must launch");
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("volume GUID root"));
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}
