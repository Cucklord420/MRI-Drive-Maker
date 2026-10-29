use super::*;
use std::ffi::OsString;
use std::io::Cursor;
use std::process::Command;

fn test_environment(overrides: &[(&str, String)]) -> Vec<(OsString, OsString)> {
    let mut environment: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| !is_git_environment_key(key))
        .collect();
    environment.extend(
        overrides
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    );
    environment
}

fn raw_git(
    repo: &Path,
    args: &[&str],
    environment: &[(OsString, OsString)],
) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .env_clear()
        .envs(environment.iter().cloned())
        .output()
        .unwrap()
}

fn raw_git_success(repo: &Path, args: &[&str], environment: &[(OsString, OsString)]) -> Vec<u8> {
    let output = raw_git(repo, args, environment);
    assert!(
        output.status.success(),
        "Git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn synthetic_repository(
    path: &Path,
    source: &str,
    launcher: &str,
    environment: &[(OsString, OsString)],
) -> String {
    fs::create_dir_all(path).unwrap();
    raw_git_success(path, &["init", "-q"], environment);
    raw_git_success(path, &["config", "core.autocrlf", "false"], environment);
    raw_git_success(
        path,
        &["config", "user.name", "MRI provenance test"],
        environment,
    );
    raw_git_success(
        path,
        &["config", "user.email", "mri-provenance@example.invalid"],
        environment,
    );
    fs::write(path.join("SOURCE.txt"), source).unwrap();
    fs::write(path.join("START-MRI-DRIVE-MAKER.cmd"), launcher).unwrap();
    raw_git_success(
        path,
        &["add", "SOURCE.txt", "START-MRI-DRIVE-MAKER.cmd"],
        environment,
    );
    raw_git_success(
        path,
        &["commit", "-m", "synthetic provenance fixture"],
        environment,
    );
    String::from_utf8(raw_git_success(path, &["rev-parse", "HEAD"], environment))
        .unwrap()
        .trim()
        .to_owned()
}

fn synthetic_sources() -> Vec<(String, String)> {
    vec![
        (
            "START-MRI-DRIVE-MAKER.cmd".into(),
            "START-MRI-DRIVE-MAKER.cmd".into(),
        ),
        ("SOURCE.txt".into(), "SOURCE.txt".into()),
    ]
}

#[test]
fn inherited_git_context_cannot_redirect_synthetic_release_build_or_verification() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("attacker.gitconfig");
    fs::write(&config, "[mri]\n    injected = attacker\n").unwrap();
    let clean = test_environment(&[]);
    let intended = temp.path().join("intended");
    let alternate = temp.path().join("alternate");
    let intended_commit = synthetic_repository(
        &intended,
        "intended source bytes\n",
        "@echo intended\r\n",
        &clean,
    );
    let alternate_commit = synthetic_repository(
        &alternate,
        "alternate source bytes\n",
        "@echo alternate\r\n",
        &clean,
    );
    let alternate_git_dir = alternate.join(".git");

    // This raw Git control proves the two-repository fixture catches the original defect.
    let raw_environment = test_environment(&[("GIT_DIR", alternate_git_dir.display().to_string())]);
    let raw = raw_git_success(&intended, &["show", "HEAD:SOURCE.txt"], &raw_environment);
    assert_eq!(raw, b"alternate source bytes\n");

    let mut scenarios: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "GIT_DIR",
            vec![("GIT_DIR", alternate_git_dir.display().to_string())],
        ),
        (
            "GIT_WORK_TREE",
            vec![("GIT_WORK_TREE", alternate.display().to_string())],
        ),
        (
            "GIT_COMMON_DIR",
            vec![("GIT_COMMON_DIR", alternate_git_dir.display().to_string())],
        ),
        (
            "GIT_OBJECT_DIRECTORY",
            vec![(
                "GIT_OBJECT_DIRECTORY",
                alternate_git_dir.join("objects").display().to_string(),
            )],
        ),
        (
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            vec![(
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                alternate_git_dir.join("objects").display().to_string(),
            )],
        ),
        (
            "GIT_CONFIG_COUNT",
            vec![
                ("GIT_CONFIG_COUNT", "1".into()),
                ("GIT_CONFIG_KEY_0", "mri.injected".into()),
                ("GIT_CONFIG_VALUE_0", "attacker".into()),
            ],
        ),
        (
            "GIT_CONFIG",
            vec![("GIT_CONFIG", config.display().to_string())],
        ),
        (
            "GIT_CONFIG_GLOBAL",
            vec![("GIT_CONFIG_GLOBAL", config.display().to_string())],
        ),
        (
            "GIT_CONFIG_SYSTEM",
            vec![("GIT_CONFIG_SYSTEM", config.display().to_string())],
        ),
        (
            "GIT_CONFIG_PARAMETERS",
            vec![("GIT_CONFIG_PARAMETERS", "'mri.injected'='attacker'".into())],
        ),
    ];
    for (name, overrides) in scenarios.drain(..) {
        let environment = test_environment(&overrides);
        match name {
            "GIT_WORK_TREE" => {
                let raw_root = raw_git_success(
                    &intended,
                    &["rev-parse", "--path-format=absolute", "--show-toplevel"],
                    &environment,
                );
                let raw_root = String::from_utf8(raw_root).unwrap();
                assert_eq!(
                    Path::new(raw_root.trim()).canonicalize().unwrap(),
                    alternate.canonicalize().unwrap()
                );
            }
            "GIT_COMMON_DIR" | "GIT_OBJECT_DIRECTORY" => {
                assert!(
                    !raw_git(&intended, &["show", "HEAD:SOURCE.txt"], &environment)
                        .status
                        .success(),
                    "{name} fixture did not redirect Git's object lookup"
                );
            }
            "GIT_ALTERNATE_OBJECT_DIRECTORIES" => {
                assert!(
                    raw_git(
                        &intended,
                        &["cat-file", "-e", &alternate_commit],
                        &environment
                    )
                    .status
                    .success(),
                    "alternate object fixture was not visible to raw Git"
                );
                assert!(
                    git_with_environment(
                        &intended,
                        &["cat-file", "-e", &alternate_commit],
                        &environment
                    )
                    .is_err(),
                    "alternate object directory reached the sanitized Git child"
                );
            }
            _ => {}
        }
        if name.starts_with("GIT_CONFIG") {
            let raw_config = raw_git(
                &intended,
                &["config", "--get", "mri.injected"],
                &environment,
            );
            assert!(
                raw_config.status.success(),
                "{name} fixture did not inject Git config"
            );
            assert_eq!(raw_config.stdout, b"attacker\n");
            assert!(
                git_with_environment(
                    &intended,
                    &["config", "--get", "mri.injected"],
                    &environment
                )
                .is_err(),
                "{name} reached the sanitized Git child"
            );
        }
        let archive = temp.path().join(format!("{name}.zip"));
        let manifest = build_with_environment(
            &intended,
            &archive,
            "0.3.0",
            &synthetic_sources(),
            &environment,
        )
        .unwrap_or_else(|error| panic!("{name} redirected or blocked build: {error}"));
        assert_eq!(
            manifest.source_commit, intended_commit,
            "{name} changed the selected commit"
        );
        assert_ne!(manifest.source_commit, alternate_commit);
        let package = members(&archive);
        let prefix = "MRI-Drive-Maker-v0.3.0/";
        assert_eq!(
            package[&(prefix.to_owned() + "SOURCE.txt")].0,
            b"intended source bytes\n",
            "{name} changed SOURCE.txt"
        );
        assert_eq!(
            package[&(prefix.to_owned() + "START-MRI-DRIVE-MAKER.cmd")].0,
            b"@echo intended\r\n",
            "{name} changed the launcher"
        );
        assert_eq!(
            verify_with_environment(&intended, &archive, &environment).unwrap(),
            manifest,
            "{name} redirected or blocked --verify"
        );
    }
}

#[test]
fn linked_worktree_git_identity_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(&[]);
    let repo = temp.path().join("main");
    let commit = synthetic_repository(
        &repo,
        "linked worktree source\n",
        "@echo linked\r\n",
        &environment,
    );
    let linked = temp.path().join("linked");
    raw_git_success(
        &repo,
        &[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD",
        ],
        &environment,
    );
    let output = git_with_environment(&linked, &["show", "HEAD:SOURCE.txt"], &environment).unwrap();
    assert_eq!(output, b"linked worktree source\n");
    assert_eq!(
        String::from_utf8(
            git_with_environment(&linked, &["rev-parse", "HEAD"], &environment).unwrap()
        )
        .unwrap()
        .trim(),
        commit
    );
    let archive = temp.path().join("linked-worktree.zip");
    let manifest = build_with_environment(
        &linked,
        &archive,
        "0.3.0",
        &synthetic_sources(),
        &environment,
    )
    .unwrap();
    assert_eq!(manifest.source_commit, commit);
    assert_eq!(
        verify_with_environment(&linked, &archive, &environment).unwrap(),
        manifest
    );
}

#[test]
fn git_identity_accepts_equivalent_paths_with_dot_segments() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(&[]);
    let repo = temp.path().join("repository");
    let commit = synthetic_repository(
        &repo,
        "path alias source bytes\n",
        "@echo path alias\r\n",
        &environment,
    );
    fs::create_dir(repo.join("existing-component")).unwrap();
    let alias = repo.join("existing-component").join("..");

    let source = git_with_environment(&alias, &["show", "HEAD:SOURCE.txt"], &environment).unwrap();
    assert_eq!(source, b"path alias source bytes\n");
    assert_eq!(
        String::from_utf8(
            git_with_environment(&alias, &["rev-parse", "HEAD"], &environment).unwrap()
        )
        .unwrap()
        .trim(),
        commit
    );
}

#[derive(Clone, Copy)]
enum TestEntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone)]
struct TestEntry {
    name: String,
    contents: Vec<u8>,
    kind: TestEntryKind,
}

fn archive_entries(path: &Path) -> Vec<TestEntry> {
    let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut member = archive.by_index(index).unwrap();
            let kind = if member.is_symlink() {
                TestEntryKind::Symlink
            } else if member.is_dir() {
                TestEntryKind::Directory
            } else {
                TestEntryKind::File
            };
            let mut contents = Vec::new();
            member.read_to_end(&mut contents).unwrap();
            TestEntry {
                name: member
                    .name()
                    .expect("generated test archive entry has a valid UTF-8 name")
                    .into_owned(),
                contents,
                kind,
            }
        })
        .collect()
}

fn write_test_archive(path: &Path, entries: &[TestEntry]) {
    let date = DateTime::from_date_and_time(2000, 1, 1, 0, 0, 0).unwrap();
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(date)
        .unix_permissions(0o644);
    let mut archive = ZipWriter::new(File::create(path).unwrap());
    for entry in entries {
        match entry.kind {
            TestEntryKind::File => {
                archive.start_file(entry.name.as_str(), options).unwrap();
                archive.write_all(&entry.contents).unwrap();
            }
            TestEntryKind::Directory => {
                archive
                    .add_directory(entry.name.as_str(), options.unix_permissions(0o755))
                    .unwrap();
            }
            TestEntryKind::Symlink => {
                let target = String::from_utf8_lossy(&entry.contents);
                archive
                    .add_symlink(entry.name.as_str(), target, options)
                    .unwrap();
            }
        }
    }
    archive.finish().unwrap();
}

fn prepend_zip_prefix_and_upgrade_to_zip64(zip32: &[u8]) -> Vec<u8> {
    let eocd = zip32.len() - 22;
    let entry_count = u16::from_le_bytes([zip32[eocd + 10], zip32[eocd + 11]]) as u64;
    let directory_size = u32::from_le_bytes(zip32[eocd + 12..eocd + 16].try_into().unwrap()) as u64;
    let directory_offset =
        u32::from_le_bytes(zip32[eocd + 16..eocd + 20].try_into().unwrap()) as u64;
    let mut archive = zip32[..eocd].to_vec();
    let zip64_eocd_offset = eocd as u64;
    archive.extend_from_slice(b"PK\x06\x06");
    archive.extend_from_slice(&44_u64.to_le_bytes());
    archive.extend_from_slice(&45_u16.to_le_bytes());
    archive.extend_from_slice(&45_u16.to_le_bytes());
    archive.extend_from_slice(&0_u32.to_le_bytes());
    archive.extend_from_slice(&0_u32.to_le_bytes());
    archive.extend_from_slice(&entry_count.to_le_bytes());
    archive.extend_from_slice(&entry_count.to_le_bytes());
    archive.extend_from_slice(&directory_size.to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(b"PK\x06\x07");
    archive.extend_from_slice(&0_u32.to_le_bytes());
    archive.extend_from_slice(&zip64_eocd_offset.to_le_bytes());
    archive.extend_from_slice(&1_u32.to_le_bytes());
    archive.extend_from_slice(b"PK\x05\x06");
    archive.extend_from_slice(&0_u16.to_le_bytes());
    archive.extend_from_slice(&0_u16.to_le_bytes());
    archive.extend_from_slice(&u16::MAX.to_le_bytes());
    archive.extend_from_slice(&u16::MAX.to_le_bytes());
    archive.extend_from_slice(&u32::MAX.to_le_bytes());
    archive.extend_from_slice(&u32::MAX.to_le_bytes());
    archive.extend_from_slice(&0_u16.to_le_bytes());

    let mut prefixed = b"synthetic ZIP prefix".to_vec();
    prefixed.extend_from_slice(&archive);
    prefixed
}

fn write_repetitive_deflated_archive(path: &Path, entries: &[(String, u64)]) {
    let date = DateTime::from_date_and_time(2000, 1, 1, 0, 0, 0).unwrap();
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(date)
        .unix_permissions(0o644);
    let mut archive = ZipWriter::new(File::create(path).unwrap());
    let block = [b'A'; 64 * 1024];
    for (name, size) in entries {
        archive.start_file(name, options).unwrap();
        let mut remaining = *size;
        while remaining != 0 {
            let count = remaining.min(block.len() as u64) as usize;
            archive.write_all(&block[..count]).unwrap();
            remaining -= count as u64;
        }
    }
    archive.finish().unwrap();
}

fn archive_prefix(entries: &[TestEntry]) -> String {
    entries
        .iter()
        .find_map(|entry| entry.name.strip_suffix("/RELEASE.json"))
        .expect("release manifest entry")
        .to_owned()
}

fn manifest_from_entries(entries: &[TestEntry]) -> (usize, Manifest) {
    let index = entries
        .iter()
        .position(|entry| entry.name.ends_with("/RELEASE.json"))
        .expect("release manifest entry");
    let manifest = serde_json::from_slice(&entries[index].contents).unwrap();
    (index, manifest)
}

fn update_manifest(entries: &mut [TestEntry], edit: impl FnOnce(&mut Manifest)) {
    let (index, mut manifest) = manifest_from_entries(entries);
    edit(&mut manifest);
    let mut contents = serde_json::to_vec_pretty(&manifest).unwrap();
    contents.push(b'\n');
    entries[index].contents = contents;
}

fn add_declared_source(entries: &mut Vec<TestEntry>, repo: &Path, name: &str, source: &str) {
    let prefix = archive_prefix(entries);
    let (_, manifest) = manifest_from_entries(entries);
    let contents = committed(repo, &manifest.source_commit, source).unwrap();
    update_manifest(entries, |manifest| {
        manifest.sha256.insert(name.to_owned(), digest(&contents));
        manifest
            .source_files
            .insert(name.to_owned(), source.to_owned());
    });
    entries.push(TestEntry {
        name: format!("{prefix}/{name}"),
        contents,
        kind: TestEntryKind::File,
    });
}

fn duplicate_central_directory_entry(path: &Path, suffix: &str) {
    let mut bytes = fs::read(path).unwrap();
    let end = bytes
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .expect("ZIP end record");
    let entry_count = u16::from_le_bytes(bytes[end + 10..end + 12].try_into().unwrap());
    let central_size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap());
    let central_start = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    assert_eq!(central_start + central_size as usize, end);

    let mut cursor = central_start;
    let mut duplicate = None;
    while cursor < end {
        assert_eq!(&bytes[cursor..cursor + 4], b"PK\x01\x02");
        let name_len =
            u16::from_le_bytes(bytes[cursor + 28..cursor + 30].try_into().unwrap()) as usize;
        let extra_len =
            u16::from_le_bytes(bytes[cursor + 30..cursor + 32].try_into().unwrap()) as usize;
        let comment_len =
            u16::from_le_bytes(bytes[cursor + 32..cursor + 34].try_into().unwrap()) as usize;
        let record_len = 46 + name_len + extra_len + comment_len;
        let name = std::str::from_utf8(&bytes[cursor + 46..cursor + 46 + name_len]).unwrap();
        if name.ends_with(suffix) {
            duplicate = Some(bytes[cursor..cursor + record_len].to_vec());
            break;
        }
        cursor += record_len;
    }
    let duplicate = duplicate.expect("matching central directory entry");
    let duplicate_len = duplicate.len();
    bytes.splice(end..end, duplicate);
    let new_end = end + duplicate_len;
    let new_count = entry_count.checked_add(1).expect("ZIP entry count");
    bytes[new_end + 8..new_end + 10].copy_from_slice(&new_count.to_le_bytes());
    bytes[new_end + 10..new_end + 12].copy_from_slice(&new_count.to_le_bytes());
    let new_size = central_size
        .checked_add(duplicate_len as u32)
        .expect("ZIP central directory size");
    bytes[new_end + 12..new_end + 16].copy_from_slice(&new_size.to_le_bytes());
    fs::write(path, bytes).unwrap();
}

fn corrupt_member_payload(path: &Path, suffix: &str) {
    let mut bytes = fs::read(path).unwrap();
    let end = bytes
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .expect("ZIP end record");
    let central_size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap()) as usize;
    let central_start = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    let mut cursor = central_start;
    let central_end = central_start + central_size;
    let mut local_header = None;
    while cursor < central_end {
        assert_eq!(&bytes[cursor..cursor + 4], b"PK\x01\x02");
        let name_len =
            u16::from_le_bytes(bytes[cursor + 28..cursor + 30].try_into().unwrap()) as usize;
        let extra_len =
            u16::from_le_bytes(bytes[cursor + 30..cursor + 32].try_into().unwrap()) as usize;
        let comment_len =
            u16::from_le_bytes(bytes[cursor + 32..cursor + 34].try_into().unwrap()) as usize;
        let record_len = 46 + name_len + extra_len + comment_len;
        let name = std::str::from_utf8(&bytes[cursor + 46..cursor + 46 + name_len]).unwrap();
        if name.ends_with(suffix) {
            local_header = Some(u32::from_le_bytes(
                bytes[cursor + 42..cursor + 46].try_into().unwrap(),
            ) as usize);
            break;
        }
        cursor += record_len;
    }
    let local = local_header.expect("matching local file header");
    assert_eq!(&bytes[local..local + 4], b"PK\x03\x04");
    let name_len = u16::from_le_bytes(bytes[local + 26..local + 28].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(bytes[local + 28..local + 30].try_into().unwrap()) as usize;
    let data = local + 30 + name_len + extra_len;
    bytes[data] ^= 1;
    fs::write(path, bytes).unwrap();
}

fn make_adversarial_archive(repo: &Path, source: &Path, destination: &Path, case: &str) {
    let mut entries = archive_entries(source);
    let prefix = archive_prefix(&entries);
    match case {
        "tampered-payload" => {
            let entry = entries
                .iter_mut()
                .find(|entry| entry.name.ends_with("/App/MRI-Batch.ps1"))
                .unwrap();
            entry.contents.push(b'\n');
        }
        "unauthorized-extra" => entries.push(TestEntry {
            name: format!("{prefix}/App/unauthorized.txt"),
            contents: b"unlisted payload".to_vec(),
            kind: TestEntryKind::File,
        }),
        "changed-manifest-hash" => update_manifest(&mut entries, |manifest| {
            manifest
                .sha256
                .insert("App/MRI-Batch.ps1".into(), "0".repeat(64));
        }),
        "missing-launcher" => {
            entries.retain(|entry| entry.name != format!("{prefix}/START-MRI-DRIVE-MAKER.cmd"));
            update_manifest(&mut entries, |manifest| {
                manifest.sha256.shift_remove("START-MRI-DRIVE-MAKER.cmd");
                manifest
                    .source_files
                    .shift_remove("START-MRI-DRIVE-MAKER.cmd");
            });
        }
        "bad-source-commit" => update_manifest(&mut entries, |manifest| {
            manifest.source_commit = "not-a-commit".into();
        }),
        "source-byte-mismatch" => {
            let entry = entries
                .iter_mut()
                .find(|entry| entry.name.ends_with("/App/MRI-Batch.ps1"))
                .unwrap();
            entry
                .contents
                .extend_from_slice(b"\n# altered synthetic payload\n");
            let updated_hash = digest(&entry.contents);
            update_manifest(&mut entries, |manifest| {
                manifest
                    .sha256
                    .insert("App/MRI-Batch.ps1".into(), updated_hash);
            });
        }
        "competing-launcher" => add_declared_source(
            &mut entries,
            repo,
            "START-ALTERNATE.cmd",
            "START-MRI-DRIVE-MAKER.cmd",
        ),
        "excluded-legacy" => add_declared_source(
            &mut entries,
            repo,
            "legacy/old.ps1",
            "legacy/MRI_CreatorV3.ps1.txt",
        ),
        "case-alias" => {
            add_declared_source(&mut entries, repo, "app/MRI-Batch.ps1", "MRI-Batch.ps1")
        }
        "file-directory" => {
            add_declared_source(&mut entries, repo, "App/path-conflict", "README.md");
            add_declared_source(&mut entries, repo, "App/path-conflict/child.txt", "LICENSE");
        }
        "traversal" => add_declared_source(&mut entries, repo, "../../outside.txt", "README.md"),
        "app-traversal" => {
            add_declared_source(&mut entries, repo, "App/../../outside.txt", "README.md")
        }
        "windows-device" => add_declared_source(&mut entries, repo, "App/CON.txt", "README.md"),
        "unicode" => add_declared_source(&mut entries, repo, "App/caf\u{00e9}.txt", "README.md"),
        "backslash" => add_declared_source(&mut entries, repo, "App\\outside.txt", "README.md"),
        "trailing-component" => {
            add_declared_source(&mut entries, repo, "App/trailing.", "README.md")
        }
        "symlink" => {
            let entry = entries
                .iter_mut()
                .find(|entry| entry.name.ends_with("/App/MRI-Batch.ps1"))
                .unwrap();
            entry.kind = TestEntryKind::Symlink;
            entry.contents = b"MRI-Batch.ps1".to_vec();
        }
        "directory" => entries.push(TestEntry {
            name: format!("{prefix}/App/empty-directory/"),
            contents: Vec::new(),
            kind: TestEntryKind::Directory,
        }),
        "root-trailing-dot" => {
            for entry in &mut entries {
                entry.name = entry
                    .name
                    .replacen(&format!("{prefix}/"), &format!("{prefix}./"), 1);
            }
        }
        "version-mismatch" => {
            update_manifest(&mut entries, |manifest| manifest.version = "9.9.9".into())
        }
        "duplicate-payload" | "duplicate-manifest" => {
            write_test_archive(destination, &entries);
            let suffix = if case == "duplicate-payload" {
                "/App/MRI-Batch.ps1"
            } else {
                "/RELEASE.json"
            };
            duplicate_central_directory_entry(destination, suffix);
            return;
        }
        "bad-crc" => {
            write_test_archive(destination, &entries);
            corrupt_member_payload(destination, "/App/MRI-Batch.ps1");
            return;
        }
        _ => panic!("Unknown adversarial archive case: {case}"),
    }
    write_test_archive(destination, &entries);
}

fn assert_rust_rejects(repo: &Path, archive: &Path, expected: Option<&str>) {
    let error =
        verify(repo, archive).expect_err("Rust verifier accepted an unsafe release archive");
    if let Some(expected) = expected {
        assert!(
            error.contains(expected),
            "expected {expected:?}, received {error:?}"
        );
    }
}

type ReleaseMembers = IndexMap<String, (Vec<u8>, (u16, u8, u8, u8, u8, u8), Option<u32>)>;

fn members(path: &Path) -> ReleaseMembers {
    let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut result = IndexMap::new();
    for n in 0..zip.len() {
        let mut member = zip.by_index(n).unwrap();
        let date = member
            .last_modified()
            .expect("Missing ZIP member timestamp");
        let meta = (
            date.year(),
            date.month(),
            date.day(),
            date.hour(),
            date.minute(),
            date.second(),
        );
        let mode = member.unix_mode();
        let name = member.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        assert_eq!(member.compression(), CompressionMethod::Stored);
        result.insert(name, (bytes, meta, mode));
    }
    result
}

#[test]
fn package_contains_one_launcher_and_approved_source_files_only() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bundle.zip");
    let manifest = build(&repo_root(), &path, "0.3.0").unwrap();
    let launchers: Vec<_> = manifest
        .sha256
        .keys()
        .filter(|name| name.ends_with(".cmd") && !name.contains('/'))
        .collect();
    assert_eq!(launchers, ["START-MRI-DRIVE-MAKER.cmd"]);
    assert!(manifest.sha256.contains_key("App/MRI.Batch.psm1"));
    assert!(manifest.sha256.contains_key("App/tests/StartupSmoke.ps1"));
    assert!(!manifest
        .sha256
        .keys()
        .any(|name| name.starts_with("legacy/") || name.starts_with("Developer/")));

    let package = members(&path);
    let names: Vec<_> = package.keys().cloned().collect();
    let mut sorted_names = names.clone();
    sorted_names.sort();
    assert_eq!(names, sorted_names, "ZIP entries must remain sorted");
    assert!(
        package.values().all(|(_, _, mode)| *mode == Some(0o100644)),
        "all release ZIP members must retain regular-file mode 0644"
    );
}

#[test]
fn never_overwrites_candidate_or_accepts_untrusted_revision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("do-not-overwrite.zip");
    fs::write(&path, b"ORIGINAL").unwrap();
    assert!(build(&repo_root(), &path, "0.3.0").is_err());
    assert_eq!(fs::read(&path).unwrap(), b"ORIGINAL");
    fs::remove_file(&path).unwrap();
    assert!(build(&repo_root(), &path, "../../exfil").is_err());
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn published_archive_preserves_umask_filtered_create_new_mode() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let output_dir = temp.path().join("output");
    fs::create_dir_all(&output_dir).unwrap();
    let environment = test_environment(&[]);
    synthetic_repository(
        &repo,
        "synthetic source bytes\n",
        "@echo synthetic\r\n",
        &environment,
    );

    let ordinary_file = output_dir.join("ordinary-file");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&ordinary_file)
        .unwrap();
    let expected_mode = fs::metadata(&ordinary_file).unwrap().permissions().mode() & 0o777;

    let archive = output_dir.join("release.zip");
    build_with_environment(&repo, &archive, "0.3.0", &synthetic_sources(), &environment).unwrap();
    let actual_mode = fs::metadata(&archive).unwrap().permissions().mode() & 0o777;
    assert_eq!(actual_mode, expected_mode);
}

fn assert_failed_archive_stage_cleans_output_and_allows_retry(stage: &str) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let output_dir = temp.path().join("output");
    fs::create_dir_all(&output_dir).unwrap();
    let path = output_dir.join("release.zip");
    let clean_environment = test_environment(&[]);
    synthetic_repository(
        &repo,
        "synthetic source bytes\n",
        "@echo synthetic\r\n",
        &clean_environment,
    );

    LAST_STAGED_PATH.with(|staged| *staged.borrow_mut() = None);
    let failed_environment = test_environment(&[("MRI_RELEASE_TEST_FAILURE", stage.to_owned())]);
    let error = build_with_environment(
        &repo,
        &path,
        "0.3.0",
        &synthetic_sources(),
        &failed_environment,
    )
    .expect_err("injected build failure unexpectedly succeeded");
    if stage == "manifest-mismatch" {
        assert!(
            error.contains("differs from original manifest"),
            "unexpected failure: {error}"
        );
    } else {
        assert!(error.contains("injected"), "unexpected failure: {error}");
    }
    assert!(
        !path.exists(),
        "failed build left the requested destination"
    );
    assert_eq!(
        fs::read_dir(&output_dir).unwrap().count(),
        0,
        "failed build left a temporary artifact"
    );
    let staged = LAST_STAGED_PATH
        .with(|staged| staged.borrow_mut().take())
        .expect("builder did not record its staged path");
    assert_ne!(
        staged, path,
        "builder wrote directly to the requested destination"
    );
    assert_eq!(
        staged.parent(),
        path.parent(),
        "staged archive must share the destination directory"
    );

    let retry = build_with_environment(
        &repo,
        &path,
        "0.3.0",
        &synthetic_sources(),
        &clean_environment,
    )
    .unwrap();
    assert_eq!(
        verify_with_environment(&repo, &path, &clean_environment).unwrap(),
        retry
    );
    assert_eq!(fs::read_dir(&output_dir).unwrap().count(), 1);
}

#[test]
fn failed_archive_write_cleans_output_and_allows_retry() {
    assert_failed_archive_stage_cleans_output_and_allows_retry("write");
}

#[test]
fn failed_normalization_cleans_output_and_allows_retry() {
    assert_failed_archive_stage_cleans_output_and_allows_retry("normalize");
}

#[test]
fn failed_verification_cleans_output_and_allows_retry() {
    assert_failed_archive_stage_cleans_output_and_allows_retry("verify");
}

#[test]
fn manifest_mismatch_cleans_output_and_allows_retry() {
    assert_failed_archive_stage_cleans_output_and_allows_retry("manifest-mismatch");
}

#[test]
fn destination_race_preserves_competing_file() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let output_dir = temp.path().join("output");
    fs::create_dir_all(&output_dir).unwrap();
    let path = output_dir.join("release.zip");
    let clean_environment = test_environment(&[]);
    synthetic_repository(
        &repo,
        "synthetic source bytes\n",
        "@echo synthetic\r\n",
        &clean_environment,
    );

    LAST_STAGED_PATH.with(|staged| *staged.borrow_mut() = None);
    let race_environment =
        test_environment(&[("MRI_RELEASE_TEST_FAILURE", "destination-race".to_owned())]);
    let error = build_with_environment(
        &repo,
        &path,
        "0.3.0",
        &synthetic_sources(),
        &race_environment,
    )
    .expect_err("publication replaced a destination created after verification");
    assert!(!error.is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"COMPETING");
    assert_eq!(fs::read_dir(&output_dir).unwrap().count(), 1);
    let staged = LAST_STAGED_PATH
        .with(|staged| staged.borrow_mut().take())
        .expect("builder did not record its staged path");
    assert_ne!(
        staged, path,
        "builder wrote directly to the requested destination"
    );
    assert_eq!(
        staged.parent(),
        path.parent(),
        "staged archive must share the destination directory"
    );

    fs::remove_file(&path).unwrap();
    let retry = build_with_environment(
        &repo,
        &path,
        "0.3.0",
        &synthetic_sources(),
        &clean_environment,
    )
    .unwrap();
    assert_eq!(
        verify_with_environment(&repo, &path, &clean_environment).unwrap(),
        retry
    );
    assert_eq!(fs::read_dir(&output_dir).unwrap().count(), 1);
}
#[test]
fn corrupt_archive_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("valid.zip");
    build(&repo_root(), &path, "0.3.0").unwrap();
    let damaged = temp.path().join("corrupted.zip");
    let mut bytes = fs::read(&path).unwrap();
    // Remove the end-of-central-directory record.
    bytes.truncate(bytes.len() - 22);
    fs::write(&damaged, bytes).unwrap();
    assert!(verify(&repo_root(), &damaged).is_err());
}

#[test]
fn manifest_is_derived_from_pinned_git_commit_not_working_tree() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("versioned.zip");
    let manifest = build(&repo_root(), &path, "0.3.0").unwrap();
    let revision = String::from_utf8(git(&repo_root(), &["rev-parse", "HEAD"]).unwrap()).unwrap();
    assert_eq!(manifest.source_commit, revision.trim());
    for (name, source) in &manifest.source_files {
        let expected = committed(&repo_root(), &manifest.source_commit, source).unwrap();
        assert_eq!(digest(&expected), manifest.sha256[name], "source: {name}");
    }
}

#[test]
fn rust_rejects_duplicate_release_members() {
    let temp = tempfile::tempdir().unwrap();
    let valid = temp.path().join("valid.zip");
    build(&repo_root(), &valid, "0.3.0").unwrap();
    for (case, expected) in [
        ("duplicate-payload", "Duplicate package entries"),
        ("duplicate-manifest", "Duplicate package entries"),
    ] {
        let archive = temp.path().join(format!("{case}.zip"));
        make_adversarial_archive(&repo_root(), &valid, &archive, case);
        assert_rust_rejects(&repo_root(), &archive, Some(expected));
    }
}

#[test]
fn rust_rejects_release_manifest_and_source_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let valid = temp.path().join("valid.zip");
    build(&repo_root(), &valid, "0.3.0").unwrap();
    for (case, expected) in [
        ("tampered-payload", Some("Package file SHA-256 mismatch")),
        ("unauthorized-extra", Some("Unexpected package entries")),
        (
            "changed-manifest-hash",
            Some("Package file SHA-256 mismatch"),
        ),
        ("missing-launcher", Some("Competing launchers")),
        ("bad-source-commit", Some("Invalid source commit")),
        (
            "source-byte-mismatch",
            Some("Source differs from declared Git commit"),
        ),
        ("competing-launcher", Some("Competing launchers")),
        (
            "excluded-legacy",
            Some("Historical files cannot be included in user download"),
        ),
        (
            "version-mismatch",
            Some("Package version does not match its root"),
        ),
    ] {
        let archive = temp.path().join(format!("{case}.zip"));
        make_adversarial_archive(&repo_root(), &valid, &archive, case);
        assert_rust_rejects(&repo_root(), &archive, expected);
    }
}

#[test]
fn rust_rejects_windows_archive_aliases_and_unsafe_types() {
    let temp = tempfile::tempdir().unwrap();
    let valid = temp.path().join("valid.zip");
    build(&repo_root(), &valid, "0.3.0").unwrap();
    for (case, expected) in [
        ("case-alias", Some("Duplicate normalized package entries")),
        ("file-directory", Some("File/directory collision")),
        ("traversal", Some("Unsafe ZIP member path")),
        ("app-traversal", Some("Unsafe ZIP member path")),
        ("windows-device", Some("Windows device alias")),
        ("unicode", Some("Non-ASCII")),
        ("backslash", Some("Noncanonical ZIP member path")),
        ("trailing-component", Some("Unsafe ZIP member path")),
        ("symlink", Some("Unexpected ZIP member type")),
        ("directory", Some("Unexpected ZIP member type")),
        ("root-trailing-dot", Some("Windows-trimmed package root")),
        ("bad-crc", Some("ZIP CRC failure")),
    ] {
        let archive = temp.path().join(format!("{case}.zip"));
        make_adversarial_archive(&repo_root(), &valid, &archive, case);
        assert_rust_rejects(&repo_root(), &archive, expected);
    }
}

#[test]
fn rust_rejects_a_high_ratio_member_over_the_expansion_limit() {
    const MEMBER_LIMIT: u64 = 16 * 1024 * 1024;

    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("high-ratio.zip");
    write_repetitive_deflated_archive(
        &archive,
        &[(
            "MRI-Drive-Maker-v0.3.0/App/compressible.bin".into(),
            MEMBER_LIMIT + 1,
        )],
    );

    let mut zip = ZipArchive::new(File::open(&archive).unwrap()).unwrap();
    let member = zip.by_index(0).unwrap();
    assert!(member.compressed_size() * 100 < member.size());

    assert_rust_rejects(
        &repo_root(),
        &archive,
        Some("ZIP member exceeds the per-member expanded-size limit"),
    );
}

#[test]
fn rust_rejects_a_manifest_over_its_retained_memory_limit() {
    const MANIFEST_LIMIT: u64 = 1024 * 1024;

    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("oversized-manifest.zip");
    write_repetitive_deflated_archive(
        &archive,
        &[(
            "MRI-Drive-Maker-v0.3.0/RELEASE.json".into(),
            MANIFEST_LIMIT + 1,
        )],
    );

    assert_rust_rejects(
        &repo_root(),
        &archive,
        Some("RELEASE.json exceeds the manifest-size limit"),
    );
}

#[test]
fn rust_rejects_aggregate_expansion_before_reading_payloads() {
    const MEMBER_LIMIT: u64 = 16 * 1024 * 1024;

    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("aggregate-limit.zip");
    let root = "MRI-Drive-Maker-v0.3.0";
    let mut entries = (0..4)
        .map(|index| (format!("{root}/App/member-{index}.bin"), MEMBER_LIMIT))
        .collect::<Vec<_>>();
    entries.push((format!("{root}/RELEASE.json"), 1));
    write_repetitive_deflated_archive(&archive, &entries);

    assert_rust_rejects(
        &repo_root(),
        &archive,
        Some("ZIP exceeds the aggregate expanded-size limit"),
    );
}

#[test]
fn rust_rejects_archives_over_the_entry_count_limit() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("too-many-entries.zip");
    let entries = (0..=4096)
        .map(|index| (format!("MRI-Drive-Maker-v0.3.0/App/empty-{index}.txt"), 0))
        .collect::<Vec<_>>();
    write_repetitive_deflated_archive(&archive, &entries);

    assert_rust_rejects(&repo_root(), &archive, Some("ZIP entry count exceeds 4096"));
}

#[test]
fn rust_rejects_declared_entry_count_before_parsing_the_central_directory() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("oversized-declared-entry-count.zip");
    let entries = (0..5)
        .map(|index| TestEntry {
            name: format!("MRI-Drive-Maker-v0.3.0/App/empty-{index}.txt"),
            contents: Vec::new(),
            kind: TestEntryKind::File,
        })
        .collect::<Vec<_>>();
    write_test_archive(&archive, &entries);

    let limits = ResourceLimits {
        max_entries: 4,
        ..DEFAULT_RESOURCE_LIMITS
    };
    let error = verify_with_limits(&repo_root(), &archive, &process_environment(), limits)
        .expect_err("declared entry count must be rejected before central-directory parsing");
    assert!(error.contains("ZIP entry count exceeds 4"), "{error}");
}

#[test]
fn zip64_entry_count_preflight_supports_prepended_archives() {
    let temp = tempfile::tempdir().unwrap();
    let zip32 = temp.path().join("ordinary.zip");
    let entries = (0..5)
        .map(|index| TestEntry {
            name: format!("MRI-Drive-Maker-v0.3.0/App/empty-{index}.txt"),
            contents: Vec::new(),
            kind: TestEntryKind::File,
        })
        .collect::<Vec<_>>();
    write_test_archive(&zip32, &entries);
    let bytes = prepend_zip_prefix_and_upgrade_to_zip64(&fs::read(zip32).unwrap());
    let mut parsed = ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
    assert_eq!(
        parsed.len(),
        5,
        "fixture must be a valid prefixed ZIP64 archive"
    );

    let archive_len = bytes.len() as u64;
    let mut archive = Cursor::new(bytes);
    let error = preflight_zip_entry_count(&mut archive, archive_len, 4)
        .expect_err("ZIP64 entry count must be rejected before archive construction");
    assert!(error.contains("ZIP entry count exceeds 4"), "{error}");
}

#[test]
fn zip_comment_eocd_decoy_does_not_override_the_real_entry_count() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("comment-decoy.zip");
    write_test_archive(
        &archive,
        &[TestEntry {
            name: "MRI-Drive-Maker-v0.3.0/App/one.txt".into(),
            contents: b"one".to_vec(),
            kind: TestEntryKind::File,
        }],
    );
    let mut bytes = fs::read(&archive).unwrap();
    let eocd = bytes.len() - 22;
    let directory_size = bytes[eocd + 12..eocd + 16].to_vec();
    let directory_offset = bytes[eocd + 16..eocd + 20].to_vec();
    bytes[eocd + 20..eocd + 22].copy_from_slice(&22_u16.to_le_bytes());

    let mut decoy = [0_u8; 22];
    decoy[..4].copy_from_slice(b"PK\x05\x06");
    decoy[8..10].copy_from_slice(&5_u16.to_le_bytes());
    decoy[10..12].copy_from_slice(&5_u16.to_le_bytes());
    decoy[12..16].copy_from_slice(&directory_size);
    decoy[16..20].copy_from_slice(&directory_offset);
    bytes.extend_from_slice(&decoy);

    let archive_len = bytes.len() as u64;
    let mut archive = Cursor::new(bytes.clone());
    preflight_zip_entry_count(&mut archive, archive_len, 4)
        .expect("an EOCD signature in a ZIP comment must not reject a valid archive");
    let mut parsed = ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(parsed.len(), 1);
}

#[test]
fn rust_rejects_oversized_compressed_input_before_zip_parsing() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("oversized-input.zip");
    fs::write(&archive, b"oversized input fixture").unwrap();
    let limits = ResourceLimits {
        max_archive_bytes: 8,
        ..DEFAULT_RESOURCE_LIMITS
    };

    let error = verify_with_limits(&repo_root(), &archive, &process_environment(), limits)
        .expect_err("oversized compressed input should be rejected before ZIP parsing");
    assert!(error.contains("ZIP archive exceeds the compressed-input-size limit"));
}

#[test]
fn streaming_enforces_actual_member_and_aggregate_expansion_limits() {
    let limits = ResourceLimits {
        max_archive_bytes: 1024,
        max_entries: 4,
        max_member_bytes: 4,
        max_total_bytes: 6,
        max_manifest_bytes: 4,
    };
    let mut actual_total = 0;

    let mut too_large_member = Cursor::new(b"12345".to_vec());
    let error = stream_member(
        &mut too_large_member,
        "member.bin",
        3,
        false,
        &mut actual_total,
        limits,
    )
    .expect_err("actual member output should be bounded even when its declaration is smaller");
    assert!(error.contains("ZIP member exceeds the per-member expanded-size limit"));

    let mut first_member = Cursor::new(b"123".to_vec());
    stream_member(
        &mut first_member,
        "first.bin",
        3,
        false,
        &mut actual_total,
        limits,
    )
    .unwrap();
    let mut second_member = Cursor::new(b"4567".to_vec());
    let error = stream_member(
        &mut second_member,
        "second.bin",
        4,
        false,
        &mut actual_total,
        limits,
    )
    .expect_err("actual aggregate output should stop at the configured total");
    assert!(error.contains("ZIP exceeds the aggregate expanded-size limit"));
}

#[test]
fn oversized_release_member_cleans_the_staged_archive() {
    const MEMBER_LIMIT: usize = 16 * 1024 * 1024;

    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let output_dir = temp.path().join("output");
    fs::create_dir_all(&output_dir).unwrap();
    let archive = output_dir.join("release.zip");
    let environment = test_environment(&[]);
    let oversized_source = "A".repeat(MEMBER_LIMIT + 1);
    synthetic_repository(
        &repo,
        &oversized_source,
        "@echo synthetic\r\n",
        &environment,
    );

    let error =
        build_with_environment(&repo, &archive, "0.3.0", &synthetic_sources(), &environment)
            .expect_err("a release with an oversized committed source should fail verification");
    assert!(error.contains("ZIP member exceeds the per-member expanded-size limit"));
    assert!(
        !archive.exists(),
        "failed verification left the final archive"
    );
    assert_eq!(
        fs::read_dir(&output_dir).unwrap().count(),
        0,
        "failed verification left a staged archive"
    );
}

#[test]
fn published_source_archive_is_accepted_by_the_rust_verifier() {
    let repo = repo_root();
    let archive = repo.join("downloads/MRI-Drive-Maker-v0.3.0.zip");
    verify(&repo, &archive).expect("published source ZIP should remain within verifier limits");
}
