use super::*;
use std::fs;
use std::path::PathBuf;
#[test]
fn sha256_known_vector_is_uppercase_and_correct() {
    assert_eq!(
        sha_upper(b"abc"),
        "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
    );
}
#[test]
fn the_windows_reserved_names_and_unsafe_paths_are_rejected() {
    for value in [
        "CON",
        "CON.txt",
        "prn",
        "LPT1",
        "COM¹",
        "NUL",
        "a\\..\\file",
        "a\\.",
        "a\\",
        "C:\\secret",
        "a|b",
        "a?b",
    ] {
        assert!(check_path(value).is_err(), "Expected failure: {value}");
    }
    for value in [
        "efi\\boot\\bootx64.efi",
        "bootmgr",
        "com10.bin",
        "a file.txt",
    ] {
        assert!(check_path(value).is_ok(), "Expected acceptance: {value}");
    }
}
#[test]
fn source_file_hash_and_no_hash_manifest_is_read_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("fictional");
    let nested = root.join("EFI").join("BOOT");
    fs::create_dir_all(&nested).unwrap();
    let file = root.join("bootmgr");
    let bytes = b"synthetic source bytes";
    fs::write(&file, bytes).unwrap();
    fs::write(nested.join("BOOTX64.EFI"), b"EFI bytes").unwrap();
    let original = fs::read(&file).unwrap();
    let hashed = compute(&root, false, false).unwrap();
    let unhashed = compute(&root, false, true).unwrap();
    assert_eq!(hashed.entries.len(), 4);
    assert_eq!(unhashed.entries.len(), 4);
    assert_eq!(hashed.total_bytes, bytes.len() as i64 + 9);
    assert_eq!(unhashed.total_bytes, hashed.total_bytes);
    assert_ne!(hashed.digest, unhashed.digest);
    assert!(unhashed.entries.iter().all(|entry| entry.sha256.is_empty()));
    assert_eq!(original, fs::read(&file).unwrap());
    let _unused = PathBuf::from("no-files-modified");
}
#[test]
fn system_directories_are_omitted_when_requested() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("fictional");
    fs::create_dir_all(root.join("System Volume Information")).unwrap();
    fs::write(
        root.join("System Volume Information").join("tracked.bin"),
        b"fictional",
    )
    .unwrap();
    fs::write(root.join("bootmgr"), b"data").unwrap();
    let all = compute(&root, false, false).unwrap();
    let without = compute(&root, true, false).unwrap();
    assert!(all.entries.len() > without.entries.len());
    assert_eq!(without.entries.len(), 1);
    assert_eq!(without.entries[0].relative_path, "bootmgr");
}
#[test]
fn refuses_missing_source_and_linked_inputs() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(compute(&tmp.path().join("absent"), false, false).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let folder = tmp.path().join("actual");
        fs::create_dir(&folder).unwrap();
        let link = tmp.path().join("linked");
        symlink(&folder, &link).unwrap();
        assert!(compute(&link, false, false).is_err());
    }
}

#[cfg(windows)]
#[test]
fn windows_directory_junctions_are_rejected_by_manifest_and_receipt_checks() {
    use std::os::windows::fs::MetadataExt;
    use std::process::Command;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("media with spaces");
    let outside = tmp.path().join("outside with spaces");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    let outside_file = outside.join("private fixture.bin");
    fs::write(&outside_file, b"unrelated bytes").unwrap();

    // A real Windows junction is not a Unix symlink. Use an owned, disposable
    // directory and verify that the fixture has the reparse-point attribute.
    let junction = root.join("unexpected junction");
    // Give cmd.exe the command and each path as distinct arguments. A single
    // prequoted /C command string is parsed differently by cmd.exe on Windows.
    let output = Command::new("cmd")
        .arg("/D")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(&junction)
        .arg(&outside)
        .output()
        .expect("Windows command processor must be present");
    assert!(output.status.success(), "mklink /J failed: {output:?}");
    let attributes = fs::symlink_metadata(&junction).unwrap().file_attributes();
    assert_ne!(attributes & 0x0400, 0, "test fixture is not a reparse point");

    let nested_error = compute(&root, false, true).unwrap_err();
    assert!(
        nested_error.contains("Links are not allowed"),
        "{nested_error}"
    );
    let root_error = compute(&junction, false, true).unwrap_err();
    assert!(
        root_error.contains("links or mounted-directory junctions"),
        "{root_error}"
    );
    assert!(crate::contents::verify(&root, None).is_err());
    assert_eq!(fs::read(&outside_file).unwrap(), b"unrelated bytes");
}
