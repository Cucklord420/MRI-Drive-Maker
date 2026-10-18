use super::*;
use std::ffi::OsString;
use std::process::Command;

fn test_environment(overrides: &[(&str, String)]) -> Vec<(OsString, OsString)> {
    let mut environment: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| !is_git_environment_key(key))
        .collect();
    environment.extend(overrides.iter().map(|(key, value)|
        (OsString::from(key), OsString::from(value))));
    environment
}

fn raw_git(repo: &Path, args: &[&str], environment: &[(OsString, OsString)]) -> std::process::Output {
    Command::new("git").args(args).current_dir(repo).env_clear()
        .envs(environment.iter().cloned()).output().unwrap()
}

fn raw_git_success(repo: &Path, args: &[&str], environment: &[(OsString, OsString)]) -> Vec<u8> {
    let output = raw_git(repo, args, environment);
    assert!(output.status.success(), "Git command failed: {}",
        String::from_utf8_lossy(&output.stderr));
    output.stdout
}

fn synthetic_repository(path: &Path, source: &str, launcher: &str,
    environment: &[(OsString, OsString)]) -> String {
    fs::create_dir_all(path).unwrap();
    raw_git_success(path, &["init", "-q"], environment);
    raw_git_success(path, &["config", "core.autocrlf", "false"], environment);
    raw_git_success(path, &["config", "user.name", "MRI provenance test"], environment);
    raw_git_success(path, &["config", "user.email", "mri-provenance@example.invalid"], environment);
    fs::write(path.join("SOURCE.txt"), source).unwrap();
    fs::write(path.join("START-MRI-DRIVE-MAKER.cmd"), launcher).unwrap();
    raw_git_success(path, &["add", "SOURCE.txt", "START-MRI-DRIVE-MAKER.cmd"], environment);
    raw_git_success(path, &["commit", "-m", "synthetic provenance fixture"], environment);
    String::from_utf8(raw_git_success(path, &["rev-parse", "HEAD"], environment)).unwrap()
        .trim().to_owned()
}

fn synthetic_sources() -> Vec<(String, String)> {
    vec![
        ("START-MRI-DRIVE-MAKER.cmd".into(), "START-MRI-DRIVE-MAKER.cmd".into()),
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
    let intended_commit = synthetic_repository(&intended, "intended source bytes\n",
        "@echo intended\r\n", &clean);
    let alternate_commit = synthetic_repository(&alternate, "alternate source bytes\n",
        "@echo alternate\r\n", &clean);
    let alternate_git_dir = alternate.join(".git");

    // This raw Git control proves the two-repository fixture catches the original defect.
    let raw_environment = test_environment(&[("GIT_DIR", alternate_git_dir.display().to_string())]);
    let raw = raw_git_success(&intended, &["show", "HEAD:SOURCE.txt"], &raw_environment);
    assert_eq!(raw, b"alternate source bytes\n");

    let mut scenarios: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("GIT_DIR", vec![("GIT_DIR", alternate_git_dir.display().to_string())]),
        ("GIT_WORK_TREE", vec![("GIT_WORK_TREE", alternate.display().to_string())]),
        ("GIT_COMMON_DIR", vec![("GIT_COMMON_DIR", alternate_git_dir.display().to_string())]),
        ("GIT_OBJECT_DIRECTORY", vec![("GIT_OBJECT_DIRECTORY", alternate_git_dir.join("objects").display().to_string())]),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES", vec![("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternate_git_dir.join("objects").display().to_string())]),
        ("GIT_CONFIG_COUNT", vec![
            ("GIT_CONFIG_COUNT", "1".into()),
            ("GIT_CONFIG_KEY_0", "mri.injected".into()),
            ("GIT_CONFIG_VALUE_0", "attacker".into()),
        ]),
        ("GIT_CONFIG", vec![("GIT_CONFIG", config.display().to_string())]),
        ("GIT_CONFIG_GLOBAL", vec![("GIT_CONFIG_GLOBAL", config.display().to_string())]),
        ("GIT_CONFIG_SYSTEM", vec![("GIT_CONFIG_SYSTEM", config.display().to_string())]),
        ("GIT_CONFIG_PARAMETERS", vec![("GIT_CONFIG_PARAMETERS", "'mri.injected'='attacker'".into())]),
    ];
    for (name, overrides) in scenarios.drain(..) {
        let environment = test_environment(&overrides);
        match name {
            "GIT_WORK_TREE" => {
                let raw_root = raw_git_success(&intended,
                    &["rev-parse", "--path-format=absolute", "--show-toplevel"], &environment);
                assert_eq!(String::from_utf8(raw_root).unwrap().trim(), alternate.canonicalize().unwrap().display().to_string());
            }
            "GIT_COMMON_DIR" | "GIT_OBJECT_DIRECTORY" => {
                assert!(!raw_git(&intended, &["show", "HEAD:SOURCE.txt"], &environment).status.success(),
                    "{name} fixture did not redirect Git's object lookup");
            }
            "GIT_ALTERNATE_OBJECT_DIRECTORIES" => {
                assert!(raw_git(&intended, &["cat-file", "-e", &alternate_commit], &environment).status.success(),
                    "alternate object fixture was not visible to raw Git");
                assert!(git_with_environment(&intended, &["cat-file", "-e", &alternate_commit], &environment).is_err(),
                    "alternate object directory reached the sanitized Git child");
            }
            _ => {}
        }
        if name.starts_with("GIT_CONFIG") {
            let raw_config = raw_git(&intended, &["config", "--get", "mri.injected"], &environment);
            assert!(raw_config.status.success(), "{name} fixture did not inject Git config");
            assert_eq!(raw_config.stdout, b"attacker\n");
            assert!(git_with_environment(&intended, &["config", "--get", "mri.injected"], &environment).is_err(),
                "{name} reached the sanitized Git child");
        }
        let archive = temp.path().join(format!("{name}.zip"));
        let manifest = build_with_environment(&intended, &archive, "0.3.0",
            &synthetic_sources(), &environment)
            .unwrap_or_else(|error| panic!("{name} redirected or blocked build: {error}"));
        assert_eq!(manifest.source_commit, intended_commit, "{name} changed the selected commit");
        assert_ne!(manifest.source_commit, alternate_commit);
        let package = members(&archive);
        let prefix = "MRI-Drive-Maker-v0.3.0/";
        assert_eq!(package[&(prefix.to_owned()+"SOURCE.txt")].0, b"intended source bytes\n", "{name} changed SOURCE.txt");
        assert_eq!(package[&(prefix.to_owned()+"START-MRI-DRIVE-MAKER.cmd")].0, b"@echo intended\r\n", "{name} changed the launcher");
        assert_eq!(verify_with_environment(&intended, &archive, &environment).unwrap(), manifest,
            "{name} redirected or blocked --verify");
    }
}

#[test]
fn linked_worktree_git_identity_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(&[]);
    let repo = temp.path().join("main");
    let commit = synthetic_repository(&repo, "linked worktree source\n", "@echo linked\r\n", &environment);
    let linked = temp.path().join("linked");
    raw_git_success(&repo, &["worktree", "add", "--detach", linked.to_str().unwrap(), "HEAD"], &environment);
    let output = git_with_environment(&linked, &["show", "HEAD:SOURCE.txt"], &environment).unwrap();
    assert_eq!(output, b"linked worktree source\n");
    assert_eq!(String::from_utf8(git_with_environment(&linked, &["rev-parse", "HEAD"], &environment).unwrap()).unwrap().trim(), commit);
    let archive = temp.path().join("linked-worktree.zip");
    let manifest = build_with_environment(&linked, &archive, "0.3.0", &synthetic_sources(), &environment).unwrap();
    assert_eq!(manifest.source_commit, commit);
    assert_eq!(verify_with_environment(&linked, &archive, &environment).unwrap(), manifest);
}

fn python_builder(args:&[&str])->serde_json::Value {
    let script=repo_root().join("scripts/Build-Download.py");
    let output=Command::new("python3").arg(script)
        .args(args).current_dir(repo_root()).output().unwrap();
    assert!(output.status.success(),"Original Python tool failed: {}",
        String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

type ReleaseMembers = IndexMap<String, (Vec<u8>, (u16, u8, u8, u8, u8, u8))>;

fn members(path:&Path)->ReleaseMembers{
    let mut zip=ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut result=IndexMap::new();
    for n in 0..zip.len() {
        let mut member=zip.by_index(n).unwrap();
        let date=member.last_modified().expect("Missing ZIP member timestamp");
        let meta=(date.year(),date.month(),date.day(),
            date.hour(),date.minute(),date.second());
        let name=member.name().unwrap().into_owned();
        let mut bytes=Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        assert_eq!(member.compression(),CompressionMethod::Stored);
        result.insert(name,(bytes,meta));
    }
    result
}

#[test]
fn python_release_zip_has_exact_same_entries_and_manifest_contents(){
    let temp=tempfile::tempdir().unwrap();
    let py=temp.path().join("python-candidate.zip");
    let rust=temp.path().join("rust-candidate.zip");
    let expected=python_builder(&["--output",py.to_str().unwrap()]);
    let new=build(&repo_root(),&rust,"0.3.0").unwrap();
    assert_eq!(expected["files"].as_u64().unwrap(),new.sha256.len() as u64+1);
    let py_entries=members(&py);
    let rust_entries=members(&rust);
    assert_eq!(py_entries.len(),rust_entries.len());
    assert_eq!(py_entries,rust_entries,"ZIP members, timestamps or metadata differ");
    let raw_python=fs::read(&py).unwrap();
    let raw_rust=fs::read(&rust).unwrap();
    if raw_python != raw_rust {
        let offset=raw_python.iter().zip(raw_rust.iter()).position(|(a,b)|a!=b)
            .unwrap_or(raw_python.len().min(raw_rust.len()));
        let start=offset.saturating_sub(16);
        let end=(offset+32).min(raw_python.len()).min(raw_rust.len());
        panic!("ZIP binary parity differs at byte {offset}; Python bytes={}, Rust bytes={}; Python window {:02x?}; Rust window {:02x?}",
            raw_python.len(),raw_rust.len(),&raw_python[start..end],&raw_rust[start..end]);
    }
    assert_eq!(verify(&repo_root(),&rust).unwrap(),new);
    let py_original_verified=python_builder(&["--verify",rust.to_str().unwrap()]);
    assert_eq!(py_original_verified["source_commit"],new.source_commit);
    assert_eq!(py_original_verified["files"],new.sha256.len());
    assert_eq!(verify(&repo_root(),&py).unwrap(),new);
}

#[test]
fn package_contains_one_launcher_and_approved_source_files_only(){
    let temp=tempfile::tempdir().unwrap();
    let path=temp.path().join("bundle.zip");
    let manifest=build(&repo_root(),&path,"0.3.0").unwrap();
    let launchers:Vec<_>=manifest.sha256.keys()
        .filter(|name|name.ends_with(".cmd")&&!name.contains('/')).collect();
    assert_eq!(launchers,["START-MRI-DRIVE-MAKER.cmd"]);
    assert!(manifest.sha256.contains_key("App/MRI.Batch.psm1"));
    assert!(manifest.sha256.contains_key("App/tests/StartupSmoke.ps1"));
    assert!(!manifest.sha256.keys().any(|name|name.starts_with("legacy/")
        ||name.starts_with("Developer/")));
}

#[test]
fn never_overwrites_candidate_or_accepts_untrusted_revision(){
    let temp=tempfile::tempdir().unwrap();
    let path=temp.path().join("do-not-overwrite.zip");
    fs::write(&path,b"ORIGINAL").unwrap();
    assert!(build(&repo_root(),&path,"0.3.0").is_err());
    assert_eq!(fs::read(&path).unwrap(),b"ORIGINAL");
    fs::remove_file(&path).unwrap();
    assert!(build(&repo_root(),&path,"../../exfil").is_err());
    assert!(!path.exists());
}

#[test]
fn corrupt_archive_fails_closed(){
    let temp=tempfile::tempdir().unwrap();
    let path=temp.path().join("valid.zip");
    build(&repo_root(),&path,"0.3.0").unwrap();
    let damaged=temp.path().join("corrupted.zip");
    let mut bytes=fs::read(&path).unwrap();
    // Remove the end-of-central-directory record.
    bytes.truncate(bytes.len()-22);
    fs::write(&damaged,bytes).unwrap();
    assert!(verify(&repo_root(),&damaged).is_err());
}

#[test]
fn manifest_is_derived_from_pinned_git_commit_not_working_tree() {
    let temp=tempfile::tempdir().unwrap();
    let path=temp.path().join("versioned.zip");
    let manifest=build(&repo_root(),&path,"0.3.0").unwrap();
    let revision=String::from_utf8(git(&repo_root(),&["rev-parse","HEAD"]).unwrap()).unwrap();
    assert_eq!(manifest.source_commit,revision.trim());
    for (name,source) in &manifest.source_files {
        let expected=committed(&repo_root(),&manifest.source_commit,source).unwrap();
        assert_eq!(digest(&expected),manifest.sha256[name],"source: {name}");
    }
}
