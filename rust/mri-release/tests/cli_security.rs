use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn supported_rust_cli_rejects_invalid_manifests_without_legacy_python_entrypoint() {
    assert!(
        !repo_root().join("scripts/Build-Download.py").exists(),
        "the optimization-sensitive Python verifier remains exposed"
    );

    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("invalid-release.zip");
    let file = File::create(&archive).unwrap();
    let mut zip = ZipWriter::new(file);
    zip.start_file(
        "MRI-Drive-Maker-v0.3.0/RELEASE.json",
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
    )
    .unwrap();
    zip.write_all(
        br#"{"version":"0.3.0","source_commit":"invalid","launcher":"START-MRI-DRIVE-MAKER.cmd","source_files":{},"sha256":{}}"#,
    )
    .unwrap();
    zip.finish().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mri-release"))
        .args(["--verify", archive.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success(), "invalid manifest was accepted");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "Invalid source commit"
    );
}
