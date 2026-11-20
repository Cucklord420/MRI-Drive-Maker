#![cfg(windows)]

use sha2::Digest;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

fn build_runtime_archive(directory: &Path) -> std::path::PathBuf {
    let archive = directory.join("runtime-package.zip");
    let built = Command::new(env!("CARGO_BIN_EXE_mri-release"))
        .arg("--output")
        .arg(&archive)
        .output()
        .expect("the native Rust release builder must launch");
    assert!(
        built.status.success(),
        "runtime package build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    archive
}

fn rehash_tampered_copy_worker(source: &Path, destination: &Path) {
    let mut archive = ZipArchive::new(File::open(source).unwrap()).unwrap();
    let mut entries = Vec::with_capacity(archive.len());
    let mut helper_changed = false;
    let mut manifest_found = false;

    for index in 0..archive.len() {
        let mut member = archive.by_index(index).unwrap();
        let name = member.name().to_owned();
        let compression = member.compression();
        let last_modified = member.last_modified();
        let permissions = member.unix_mode().unwrap_or(0o100644);
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();

        if name.ends_with("/App/mri-copy.exe") {
            assert!(bytes.len() > 127);
            bytes[127] ^= 0x5a;
            helper_changed = true;
        } else if name.ends_with("/RELEASE.json") {
            manifest_found = true;
        }

        entries.push((name, bytes, compression, last_modified, permissions));
    }

    assert!(helper_changed, "the runtime package must contain its copy worker");
    assert!(manifest_found, "the runtime package must contain RELEASE.json");

    let mut helper_digest = None;
    for (name, bytes, _, _, _) in &entries {
        if name.ends_with("/App/mri-copy.exe") {
            helper_digest = Some(format!("{:x}", sha2::Sha256::digest(bytes)));
            break;
        }
    }
    let helper_digest = helper_digest.unwrap();
    let mut rehashed_manifest = false;
    for (name, bytes, _, _, _) in &mut entries {
        if name.ends_with("/RELEASE.json") {
            let mut manifest: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            manifest["sha256"]["App/mri-copy.exe"] =
                serde_json::Value::String(helper_digest.clone());
            *bytes = serde_json::to_vec(&manifest).unwrap();
            rehashed_manifest = true;
            break;
        }
    }
    assert!(
        rehashed_manifest,
        "the helper hash must be rewritten in RELEASE.json"
    );

    let mut writer = ZipWriter::new(File::create(destination).unwrap());
    for (name, bytes, compression, last_modified, permissions) in entries {
        let options = SimpleFileOptions::default()
            .compression_method(compression)
            .last_modified_time(last_modified)
            .unix_permissions(permissions);
        writer.start_file(name, options).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn packaged_copy_worker_captures_source_identity_without_an_external_snapshot() {
    let temp = tempfile::tempdir().expect("owned synthetic fixture");
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/payload.bin"), b"synthetic payload").unwrap();

    let archive = build_runtime_archive(temp.path());
    let release_binary = env!("CARGO_BIN_EXE_mri-release");

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

#[test]
fn release_verifier_uses_embedded_helper_against_rehashed_archive_replacement() {
    let temp = tempfile::tempdir().expect("owned synthetic fixture");
    let archive = build_runtime_archive(temp.path());
    let release_binary = env!("CARGO_BIN_EXE_mri-release");

    let verifier_copy_dir = temp.path().join("verifier-copy");
    fs::create_dir(&verifier_copy_dir).unwrap();
    let verifier_copy = verifier_copy_dir.join("mri-release.exe");
    fs::copy(release_binary, &verifier_copy).unwrap();

    let standalone_verification = Command::new(&verifier_copy)
        .arg("--verify")
        .arg(&archive)
        .output()
        .expect("the copied verifier must launch without an adjacent helper");
    assert!(
        standalone_verification.status.success(),
        "release verifier should be self-contained: {}",
        String::from_utf8_lossy(&standalone_verification.stderr)
    );

    let helper_path = Path::new(release_binary)
        .parent()
        .unwrap()
        .join("mri-copy.exe");
    let helper_copy = verifier_copy_dir.join("mri-copy.exe");
    let mut helper_bytes = fs::read(helper_path).unwrap();
    assert!(helper_bytes.len() > 127);
    helper_bytes[127] ^= 0x5a;
    fs::write(&helper_copy, helper_bytes).unwrap();
    let unaffected_by_sibling = Command::new(&verifier_copy)
        .arg("--verify")
        .arg(&archive)
        .output()
        .expect("the copied release verifier must launch");
    assert!(
        unaffected_by_sibling.status.success(),
        "a mutable sibling helper must not affect verification: {}",
        String::from_utf8_lossy(&unaffected_by_sibling.stderr)
    );

    let tampered_archive = temp.path().join("rehashed-helper.zip");
    rehash_tampered_copy_worker(&archive, &tampered_archive);
    fs::remove_file(helper_copy).unwrap();
    let rejected_rehashed_helper = Command::new(&verifier_copy)
        .arg("--verify")
        .arg(&tampered_archive)
        .output()
        .expect("the copied verifier must reject a rehashed replacement helper");
    assert!(!rejected_rehashed_helper.status.success());
    assert!(
        String::from_utf8_lossy(&rejected_rehashed_helper.stderr)
            .contains("embedded trusted copy worker"),
        "rehashed helper rejection did not come from the embedded trust anchor: {}",
        String::from_utf8_lossy(&rejected_rehashed_helper.stderr)
    );
}
