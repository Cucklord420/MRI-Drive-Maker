#![cfg(windows)]

use sha2::Digest;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::process::Command;
use zip::ZipArchive;

#[test]
fn packaged_runtime_helper_captures_a_synthetic_source_snapshot() {
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
    let snapshot = temp.path().join("source-identity.json");
    let captured = Command::new(helper)
        .arg("--snapshot")
        .arg(&source)
        .arg(&snapshot)
        .output()
        .expect("the packaged Rust helper must launch");
    assert!(
        captured.status.success(),
        "packaged source snapshot failed: {}",
        String::from_utf8_lossy(&captured.stderr)
    );

    let identity: serde_json::Value = serde_json::from_slice(&fs::read(snapshot).unwrap()).unwrap();
    let entry = &identity["Entries"]["nested\\payload.bin"];
    assert_eq!(entry["IsDirectory"], false);
    assert_eq!(entry["Length"], 17);
    let file_id = entry["Identity"]["FileId"].as_str().unwrap();
    assert_eq!(file_id.len(), 32);
    assert_ne!(file_id, "00000000000000000000000000000000");

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
            ],
            "SourceIdentity": identity
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
