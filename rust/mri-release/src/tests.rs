use super::*;
use std::process::Command;

fn python_builder(args:&[&str])->serde_json::Value {
    let script=repo_root().join("scripts/Build-Download.py");
    let output=Command::new("python3").arg(script)
        .args(args).current_dir(repo_root()).output().unwrap();
    assert!(output.status.success(),"Original Python tool failed: {}",
        String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn python_verify(path:&Path)->std::process::Output {
    let script=repo_root().join("scripts/Build-Download.py");
    Command::new("python3").arg(script).args(["--verify",path.to_str().unwrap()])
        .current_dir(repo_root()).output().unwrap()
}

fn make_adversarial_archive(repo:&Path, source:&Path, destination:&Path, case:&str) {
    let script=r#"
import hashlib, json, stat, sys, zipfile
source, destination, case, repo = sys.argv[1:]
with zipfile.ZipFile(source) as package:
    records = []
    for info in package.infolist():
        records.append([info.filename, package.read(info.filename), info.create_system, info.external_attr])
manifest_name = next(name for name, _, _, _ in records if name.endswith('/RELEASE.json'))
manifest_record = next(record for record in records if record[0] == manifest_name)
manifest = json.loads(manifest_record[1])
prefix = manifest_name.rsplit('/', 1)[0]
def add_source(name, source_path):
    import subprocess
    data = subprocess.check_output(['git', 'show', manifest['source_commit'] + ':' + source_path], cwd=repo)
    manifest['sha256'][name] = hashlib.sha256(data).hexdigest()
    manifest['source_files'][name] = source_path
    records.append([prefix + '/' + name, data, 3, (stat.S_IFREG | 0o644) << 16])
if case == 'duplicate-payload':
    target = next(record for record in records if record[0].endswith('/App/MRI-Batch.ps1'))
    records.insert(records.index(target), [target[0], b'not approved payload', 3, (stat.S_IFREG | 0o644) << 16])
elif case == 'duplicate-manifest':
    records.insert(records.index(manifest_record), [manifest_name, b'not the approved manifest', 3, (stat.S_IFREG | 0o644) << 16])
elif case == 'case-alias':
    add_source('app/MRI-Batch.ps1', 'MRI-Batch.ps1')
elif case == 'file-directory':
    add_source('App/path-conflict', 'README.md')
    add_source('App/path-conflict/child.txt', 'LICENSE')
elif case == 'traversal':
    add_source('../../outside.txt', 'README.md')
elif case == 'app-traversal':
    add_source('App/../../outside.txt', 'README.md')
elif case == 'windows-device':
    add_source('App/CON.txt', 'README.md')
elif case == 'unicode':
    add_source('App/caf\u00e9.txt', 'README.md')
elif case == 'backslash':
    add_source('App\\\\outside.txt', 'README.md')
elif case == 'symlink':
    target = next(record for record in records if record[0].endswith('/App/MRI-Batch.ps1'))
    target[3] = (stat.S_IFLNK | 0o777) << 16
elif case == 'root-trailing-dot':
    for record in records:
        record[0] = record[0].replace(prefix + '/', prefix + './', 1)
elif case == 'version-mismatch':
    manifest['version'] = '9.9.9'
else:
    raise ValueError(case)
if case not in ('duplicate-payload', 'duplicate-manifest', 'symlink'):
    manifest_record[1] = (json.dumps(manifest, indent=2) + '\n').encode()
with zipfile.ZipFile(destination, 'w', compression=zipfile.ZIP_STORED) as package:
    for name, data, creator, attributes in records:
        info = zipfile.ZipInfo(name, date_time=(2000, 1, 1, 0, 0, 0))
        info.create_system = creator
        info.external_attr = attributes
        package.writestr(info, data)
"#;
    let result=Command::new("python3").arg("-c").arg(script)
        .arg(source).arg(destination).arg(case).arg(repo)
        .current_dir(repo).output().unwrap();
    assert!(result.status.success(),"Could not create synthetic {case} archive: {}",
        String::from_utf8_lossy(&result.stderr));
}

fn assert_both_verifiers_reject(
    repo:&Path,
    archive:&Path,
    python_expected:Option<&str>,
    rust_expected:Option<&str>,
) {
    let python=python_verify(archive);
    assert!(!python.status.success(),"Original Python verifier accepted {}: {}",
        archive.display(),String::from_utf8_lossy(&python.stdout));
    if let Some(expected)=python_expected {
        assert!(String::from_utf8_lossy(&python.stderr).contains(expected),
            "Python verifier did not report {expected}: {}",String::from_utf8_lossy(&python.stderr));
    }
    let rust=verify(repo,archive);
    assert!(rust.is_err(),"Rust verifier accepted {}",archive.display());
    if let Some(expected)=rust_expected {
        assert!(rust.unwrap_err().contains(expected),"Rust verifier reported an unexpected reason");
    }
}

type ZipTimestamp=(u16,u8,u8,u8,u8,u8);
type ZipMembers=IndexMap<String,(Vec<u8>,ZipTimestamp)>;

fn members(path:&Path)->ZipMembers{
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

#[test]
fn python_and_rust_reject_duplicate_release_members() {
    let temp=tempfile::tempdir().unwrap();
    let valid=temp.path().join("valid.zip");
    build(&repo_root(),&valid,"0.3.0").unwrap();
    for (case,expected) in [
        ("duplicate-payload","Duplicate package entries"),
        ("duplicate-manifest","Duplicate package entries"),
    ] {
        let archive=temp.path().join(format!("{case}.zip"));
        make_adversarial_archive(&repo_root(),&valid,&archive,case);
        assert_both_verifiers_reject(&repo_root(),&archive,Some(expected),Some(expected));
    }
}

#[test]
fn python_and_rust_reject_windows_archive_aliases_and_unsafe_types() {
    let temp=tempfile::tempdir().unwrap();
    let valid=temp.path().join("valid.zip");
    build(&repo_root(),&valid,"0.3.0").unwrap();
    for (case,expected) in [
        ("case-alias",Some("Duplicate normalized package entries")),
        ("file-directory",Some("File/directory collision")),
        ("traversal",Some("Unsafe ZIP member path")),
        ("app-traversal",Some("Unsafe ZIP member path")),
        ("windows-device",Some("Windows device alias")),
        ("unicode",Some("Non-ASCII")),
        ("backslash",None),
        ("symlink",Some("Unexpected ZIP member type")),
        ("root-trailing-dot",Some("Windows-trimmed package root")),
        ("version-mismatch",Some("Package version does not match its root")),
    ] {
        let archive=temp.path().join(format!("{case}.zip"));
        make_adversarial_archive(&repo_root(),&valid,&archive,case);
        assert_both_verifiers_reject(&repo_root(),&archive,expected,expected);
    }
}
