// SPDX-License-Identifier: GPL-3.0-only
//! Rust mirror of scripts/Build-Download.py: reproducible committed-source ZIPs.
//! Does not touch USB devices or execute any bundled scripts.
use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    version: String,
    source_commit: String,
    launcher: String,
    source_files: IndexMap<String, String>,
    sha256: IndexMap<String, String>,
}
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git").args(args).current_dir(repo).output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("Git source lookup failed: {}",
            String::from_utf8_lossy(&output.stderr)));
    }
    Ok(output.stdout)
}
fn committed(repo: &Path, revision: &str, path: &str) -> Result<Vec<u8>, String> {
    git(repo, &["show", &format!("{revision}:{path}")])
}
fn manifest_name(source: &str, prefix: &str) -> String {
    format!("{prefix}{source}")
}
fn verify(repo: &Path, archive_path: &Path) -> Result<Manifest, String> {
    let file = File::open(archive_path).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut names = BTreeSet::new();
    let mut prefixes = BTreeSet::new();
    let mut files = HashMap::new();
    for index in 0..zip.len() {
        let mut member = zip.by_index(index).map_err(|e| e.to_string())?;
        let name = member.name().map_err(|e| e.to_string())?.into_owned();
        if !names.insert(name.clone()) {
            return Err("Duplicate package entries".into());
        }
        prefixes.insert(name.split('/').next().unwrap_or("").to_owned());
        let mut contents = Vec::new();
        member.read_to_end(&mut contents).map_err(|e|format!("ZIP CRC failure: {e}"))?;
        files.insert(name, contents);
    }
    if prefixes.len() != 1 {
        return Err("Expected one top-level directory".into());
    }
    let prefix = prefixes.iter().next().unwrap().to_owned() + "/";
    let manifest_bytes = files.get(&(prefix.clone()+"RELEASE.json"))
        .ok_or("Missing RELEASE.json")?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| format!("Bad RELEASE.json: {e}"))?;
    if !Regex::new(r"^[a-f0-9]{40}$").unwrap().is_match(&manifest.source_commit) {
        return Err("Invalid source commit".into());
    }
    let expected: BTreeSet<String> = manifest.sha256.keys()
        .map(|name| manifest_name(name, &prefix))
        .chain(std::iter::once(prefix.clone()+"RELEASE.json")).collect();
    if names != expected {
        return Err("Unexpected package entries".into());
    }
    if manifest.sha256.keys().collect::<BTreeSet<_>>()
        != manifest.source_files.keys().collect::<BTreeSet<_>>() {
        return Err("Every packaged file must have a source origin".into());
    }
    for (name, source) in &manifest.source_files {
        let contents = files.get(&manifest_name(name, &prefix))
            .ok_or("Missing declared package file")?;
        if digest(contents) != manifest.sha256[name] {
            return Err(format!("Package file SHA-256 mismatch: {name}"));
        }
        if committed(repo, &manifest.source_commit, source)? != *contents {
            return Err(format!("Source differs from declared Git commit: {name}"));
        }
    }
    let launchers:Vec<&str> = manifest.sha256.keys().map(String::as_str)
        .filter(|n| !n.contains('/') &&
            [".cmd", ".ps1", ".exe"].iter().any(|extension|
                n.to_ascii_lowercase().ends_with(extension))).collect();
    if launchers != ["START-MRI-DRIVE-MAKER.cmd"] {
        return Err("Competing launchers".into());
    }
    if manifest.sha256.keys().any(|n|n.starts_with("Developer/")||n.starts_with("legacy/")){
        return Err("Historical files cannot be included in user download".into());
    }
    Ok(manifest)
}
// zip-rs emits "version made by" 0x032E (46, Unix); Python zipfile
// uses 0x0314 (20, Unix). For a byte-identical reproducible source ZIP,
// normalize ONLY that central-directory metadata. Do not modify file payloads.
fn normalize_zip_creator_version(path:&Path)->Result<(),String>{
    let mut bytes=fs::read(path).map_err(|e|e.to_string())?;
    if bytes.len()<22 {return Err("ZIP lacks end-of-central-directory record".into());}
    let end=bytes.len()-22;
    if bytes.get(end..end+4)!=Some(&b"PK\x05\x06"[..]){
        return Err("Expected no ZIP comment or ZIP64 extensions".into());
    }
    let count=u16::from_le_bytes([bytes[end+10],bytes[end+11]]) as usize;
    let expected_len=u32::from_le_bytes([
        bytes[end+12],bytes[end+13],bytes[end+14],bytes[end+15]
    ]) as usize;
    let start=u32::from_le_bytes([
        bytes[end+16],bytes[end+17],bytes[end+18],bytes[end+19]
    ]) as usize;
    if start> end || start.checked_add(expected_len)!=Some(end) {
        return Err("Invalid ZIP central-directory bounds".into());
    }
    let mut cursor=start;
    for _ in 0..count {
        if cursor.checked_add(46).is_none_or(|v|v>end)
            || bytes.get(cursor..cursor+4)!=Some(&b"PK\x01\x02"[..]) {
            return Err("Invalid ZIP central-directory header".into());
        }
        if bytes[cursor+4]!=46 || bytes[cursor+5]!=3 {
            return Err(format!("Unexpected native ZIP creator platform/version: {}/{}", bytes[cursor+4], bytes[cursor+5]));
        }
        bytes[cursor+4]=20; // Python zipfile: version made by 20
        let filename_len=u16::from_le_bytes([bytes[cursor+28],bytes[cursor+29]]) as usize;
        let extra_len=u16::from_le_bytes([bytes[cursor+30],bytes[cursor+31]]) as usize;
        let comment_len=u16::from_le_bytes([bytes[cursor+32],bytes[cursor+33]]) as usize;
        cursor=cursor.checked_add(46+filename_len+extra_len+comment_len)
            .ok_or("ZIP central-directory length overflow")?;
    }
    if cursor!=end {return Err("ZIP central-directory entries do not match declared size".into());}
    let mut output=OpenOptions::new().write(true).open(path).map_err(|e|e.to_string())?;
    output.write_all(&bytes).map_err(|e|e.to_string())?;
    output.flush().map_err(|e|e.to_string())
}

fn build(repo:&Path, path:&Path, version:&str)->Result<Manifest,String>{
    if !Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,48}$").unwrap().is_match(version) {
        return Err("Version contains unsupported path characters".into());
    }
    if path.exists() {
        return Err(format!("Refusing to overwrite existing download: {}",path.display()));
    }
    let revision=String::from_utf8(git(repo,&["rev-parse","HEAD"])?)
        .map_err(|e|e.to_string())?.trim().to_owned();
    let mut entries=IndexMap::<String,Vec<u8>>::new();
    let mut origins=IndexMap::<String,String>::new();
    let mut insert=|source:&str,dest:&str|->Result<(),String>{
        let content=committed(repo,&revision,source)?;
        entries.insert(dest.to_owned(),content);
        origins.insert(dest.to_owned(),source.to_owned());
        Ok(())
    };
    for name in ["START-MRI-DRIVE-MAKER.cmd","READ-ME-FIRST.txt","LICENSE","NOTICE.md"] {
        insert(name,name)?;
    }
    for name in ["MRI-Batch.ps1","MRI.Batch.psm1","Copy-MRIFiles.ps1",
        "NativeProcess.cs","mri-config.example.json","LICENSE"] {
        insert(name,&format!("App/{name}"))?;
    }
    let mut tests=Vec::new();
    for item in fs::read_dir(repo.join("tests")).map_err(|e|e.to_string())?{
        let entry=item.map_err(|e|e.to_string())?;
        let path=entry.path();
        if path.extension().is_some_and(|x|x=="ps1"){
            tests.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    tests.sort();
    for name in tests {
        insert(&format!("tests/{name}"),&format!("App/tests/{name}"))?;
    }
    drop(insert);
    let sha256:IndexMap<String,String> = entries.iter()
        .map(|(name,bytes)|(name.clone(),digest(bytes))).collect();
    let manifest=Manifest {
        version:version.to_owned(),
        source_commit:revision,
        launcher:"START-MRI-DRIVE-MAKER.cmd".to_owned(),
        source_files:origins,
        sha256,
    };
    let mut manifest_bytes=serde_json::to_string_pretty(&manifest)
        .map_err(|e|e.to_string())?.into_bytes();
    manifest_bytes.push(b'\n');
    entries.insert("RELEASE.json".to_owned(),manifest_bytes);

    fs::create_dir_all(path.parent().ok_or("Missing destination directory")?)
        .map_err(|e|e.to_string())?;
    let output=OpenOptions::new().write(true).create_new(true).open(path)
        .map_err(|e|e.to_string())?;
    let mut zip=ZipWriter::new(output);
    let date=DateTime::from_date_and_time(2000,1,1,0,0,0)
        .map_err(|_|"Invalid fixed ZIP timestamp")?;
    let options=SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(date)
        .unix_permissions(0o644);
    let prefix=format!("MRI-Drive-Maker-v{version}/");
    let mut sorted:Vec<_>=entries.iter().collect();
    sorted.sort_by(|a,b|a.0.cmp(b.0));
    for (name,contents) in sorted{
        zip.start_file(format!("{prefix}{name}"),options).map_err(|e|e.to_string())?;
        zip.write_all(contents).map_err(|e|e.to_string())?;
    }
    zip.finish().map_err(|e|e.to_string())?;
    normalize_zip_creator_version(path)?;
    let checked=verify(repo,path)?;
    if checked!=manifest {
        return Err("Release verification differs from original manifest".into());
    }
    Ok(manifest)
}
fn main(){
    let mut version="0.3.0".to_owned();
    let mut output:Option<PathBuf>=None;
    let mut verify_path:Option<PathBuf>=None;
    let mut args=std::env::args().skip(1);
    while let Some(key)=args.next(){
        let Some(value)=args.next() else {
            eprintln!("Option is missing a value");std::process::exit(2);
        };
        match key.as_str(){
            "--version"=>version=value,
            "--output"=>output=Some(PathBuf::from(value)),
            "--verify"=>verify_path=Some(PathBuf::from(value)),
            _=>{eprintln!("Unrecognized option");std::process::exit(2)}
        }
    }
    let repo=repo_root();
    if let Some(path)=verify_path {
        match verify(&repo,&path){
            Ok(manifest)=>println!("{}",serde_json::json!({
                "verified":path.display().to_string(),
                "source_commit":manifest.source_commit,
                "files":manifest.sha256.len()
            })),
            Err(reason)=>{eprintln!("{reason}");std::process::exit(1)}
        }
    }else{
        let path=output.unwrap_or_else(||repo.join(format!("downloads/MRI-Drive-Maker-v{version}.zip")));
        match build(&repo,&path,&version){
            Ok(manifest)=>{
                let bytes=fs::read(&path).expect("archive generated");
                println!("{}",serde_json::json!({
                    "path":path.display().to_string(),
                    "sha256":digest(&bytes),
                    "bytes":bytes.len(),
                    "files":manifest.sha256.len()+1,
                    "source_commit":manifest.source_commit
                }));
            }
            Err(reason)=>{eprintln!("{reason}");std::process::exit(1)}
        }
    }
}
#[cfg(test)]
mod tests;
