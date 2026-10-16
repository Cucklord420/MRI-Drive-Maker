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

fn members(path:&Path)->IndexMap<String,(Vec<u8>,(u16,u8,u8,u8,u8,u8))>{
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
