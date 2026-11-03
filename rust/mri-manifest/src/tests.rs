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

#[test]
fn ignored_system_directories_are_pruned_before_descent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for name in ["System Volume Information", "$RECYCLE.BIN"] {
        let nested = root.join(name).join("deep");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("private.txt"), b"synthetic").unwrap();
    }
    fs::write(root.join("bootmgr"), b"test").unwrap();

    // Inspect the iterator itself: ignored directories must not be opened or
    // yielded, rather than merely filtered from the final JSON manifest.
    let entries = WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|entry| include_entry(entry, true))
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(entries, ["bootmgr"]);
    let manifest = compute(root, true, false).unwrap();
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].relative_path, "bootmgr");
}

#[cfg(unix)]
#[test]
fn refuses_special_files_instead_of_attempting_to_hash_them() {
    use std::os::unix::net::UnixListener;
    let tmp = tempfile::tempdir().unwrap();
    let _socket = UnixListener::bind(tmp.path().join("socket")).unwrap();
    for no_hash in [false, true] {
        let error = compute(tmp.path(), false, no_hash).unwrap_err();
        assert!(error.contains("Unsupported media entry type"), "{error}");
    }
}

#[cfg(windows)]
#[test]
fn windows_directory_junctions_are_refused_at_root_and_inside_source() {
    use std::process::Command;
    let tmp = tempfile::tempdir().unwrap();
    let destination = tmp.path().join("physical with spaces");
    let source = tmp.path().join("source with spaces");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&source).unwrap();
    let untouched = destination.join("private fixture.bin");
    fs::write(&untouched, b"unrelated synthetic bytes").unwrap();
    let junction = source.join("junction with spaces");
    let status = Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&destination)
        .status()
        .unwrap();
    assert!(status.success(), "Windows junction fixture creation failed");
    let metadata = fs::symlink_metadata(&junction).unwrap();
    assert!(has_link_or_reparse(&metadata));
    let root_error = compute(&junction, false, true).unwrap_err();
    assert!(
        root_error.contains("links or mounted-directory junctions"),
        "{root_error}"
    );
    let nested_error = compute(&source, false, true).unwrap_err();
    assert!(
        nested_error.contains("Links are not allowed in media content"),
        "{nested_error}"
    );
    let receipt_error = crate::contents::verify(&source, None).unwrap_err();
    assert!(
        receipt_error.contains("Links are not allowed in media content"),
        "{receipt_error}"
    );
    assert_eq!(fs::read(&untouched).unwrap(), b"unrelated synthetic bytes");
}
