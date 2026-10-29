// SPDX-License-Identifier: GPL-3.0-only
//! Reproducible committed-source ZIP generator and verifier.
//! Does not touch USB devices or execute any bundled scripts.
use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
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
fn is_git_environment_key(key: &OsStr) -> bool {
    key.to_string_lossy()
        .to_ascii_uppercase()
        .starts_with("GIT_")
}
fn process_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os().collect()
}
fn git_command(
    repo: &Path,
    args: &[&str],
    environment: &[(OsString, OsString)],
) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(repo).env_clear();
    for (key, value) in environment {
        if !is_git_environment_key(key) {
            command.env(key, value);
        }
    }
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Git source lookup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}
fn repository_layout(repo: &Path) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let root = repo
        .canonicalize()
        .map_err(|e| format!("Cannot resolve intended worktree: {e}"))?;
    if !root.is_dir() {
        return Err("The intended Git worktree is not a directory".into());
    }
    let metadata = root.join(".git");
    let (git_dir, common_dir) = if metadata.is_dir() {
        let git_dir = metadata
            .canonicalize()
            .map_err(|e| format!("Cannot resolve intended Git directory: {e}"))?;
        (git_dir.clone(), git_dir)
    } else if metadata.is_file() {
        let contents = fs::read_to_string(&metadata)
            .map_err(|e| format!("Cannot read intended .git file: {e}"))?;
        let line = contents.strip_suffix('\n').unwrap_or(&contents);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some(git_dir_ref) = line.strip_prefix("gitdir: ") else {
            return Err("The intended Git worktree has an invalid .git file".into());
        };
        if git_dir_ref.contains(['\n', '\r']) {
            return Err("The intended Git worktree has an ambiguous .git file".into());
        }
        let git_dir_ref = PathBuf::from(git_dir_ref);
        let git_dir = if git_dir_ref.is_absolute() {
            git_dir_ref
        } else {
            root.join(git_dir_ref)
        }
        .canonicalize()
        .map_err(|e| format!("Cannot resolve linked worktree Git directory: {e}"))?;
        let common_file = git_dir.join("commondir");
        if !common_file.is_file() {
            return Err("Only linked Git worktrees with a commondir file are supported".into());
        }
        let common_ref = fs::read_to_string(&common_file)
            .map_err(|e| format!("Cannot read linked worktree commondir: {e}"))?;
        let common_ref = common_ref.trim_end_matches(['\r', '\n']);
        if common_ref.is_empty() || common_ref.contains(['\r', '\n']) {
            return Err("The linked worktree has an invalid commondir file".into());
        }
        let common_ref = PathBuf::from(common_ref);
        let common_dir = if common_ref.is_absolute() {
            common_ref
        } else {
            git_dir.join(common_ref)
        }
        .canonicalize()
        .map_err(|e| format!("Cannot resolve common Git directory: {e}"))?;
        let worktrees = common_dir
            .join("worktrees")
            .canonicalize()
            .map_err(|e| format!("Cannot resolve common worktree metadata directory: {e}"))?;
        if git_dir.parent() != Some(worktrees.as_path()) {
            return Err("Linked worktree metadata is outside its common repository".into());
        }
        (git_dir, common_dir)
    } else {
        return Err("The intended source directory is not a Git worktree".into());
    };
    Ok((root, git_dir, common_dir))
}
fn git_path(bytes: Vec<u8>, label: &str) -> Result<PathBuf, String> {
    let value =
        String::from_utf8(bytes).map_err(|_| format!("Git returned a non-UTF-8 {label}"))?;
    let value = value
        .strip_suffix('\n')
        .ok_or_else(|| format!("Git returned an incomplete {label}"))?;
    let value = value.strip_suffix('\r').unwrap_or(value);
    if value.contains(['\n', '\r']) {
        return Err(format!("Git returned an ambiguous {label}"));
    }
    Path::new(value)
        .canonicalize()
        .map_err(|e| format!("Cannot resolve Git {label}: {e}"))
}
fn assert_git_identity(repo: &Path, environment: &[(OsString, OsString)]) -> Result<(), String> {
    let (root, expected_git_dir, expected_common_dir) = repository_layout(repo)?;
    let actual_root = git_path(
        git_command(
            &root,
            &["rev-parse", "--path-format=absolute", "--show-toplevel"],
            environment,
        )?,
        "worktree root",
    )?;
    let actual_git_dir = git_path(
        git_command(&root, &["rev-parse", "--absolute-git-dir"], environment)?,
        "Git directory",
    )?;
    let actual_common_dir = git_path(
        git_command(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            environment,
        )?,
        "common Git directory",
    )?;
    if actual_root != root
        || actual_git_dir != expected_git_dir
        || actual_common_dir != expected_common_dir
    {
        return Err(
            "Git resolved a different worktree or repository than the intended source".into(),
        );
    }
    Ok(())
}
fn git_with_environment(
    repo: &Path,
    args: &[&str],
    environment: &[(OsString, OsString)],
) -> Result<Vec<u8>, String> {
    assert_git_identity(repo, environment)?;
    let root = repo
        .canonicalize()
        .map_err(|e| format!("Cannot resolve intended worktree: {e}"))?;
    git_command(&root, args, environment)
}
#[cfg(test)]
fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    git_with_environment(repo, args, &process_environment())
}
#[cfg(test)]
fn committed(repo: &Path, revision: &str, path: &str) -> Result<Vec<u8>, String> {
    git(repo, &["show", &format!("{revision}:{path}")])
}
fn committed_with_environment(
    repo: &Path,
    revision: &str,
    path: &str,
    environment: &[(OsString, OsString)],
) -> Result<Vec<u8>, String> {
    git_with_environment(repo, &["show", &format!("{revision}:{path}")], environment)
}
fn manifest_name(source: &str, prefix: &str) -> String {
    format!("{prefix}{source}")
}
fn canonical_member_name(name: &str) -> Result<(String, String), String> {
    if !name.is_ascii() {
        return Err("Non-ASCII ZIP member names are not allowed".into());
    }
    if name.as_bytes().contains(&0) || name.contains('\\') {
        return Err("Noncanonical ZIP member path".into());
    }
    if name.len() > 200 {
        return Err("ZIP member path is too long".into());
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
        return Err("Noncanonical ZIP member path".into());
    }
    let root_re = Regex::new(r"^MRI-Drive-Maker-v([A-Za-z0-9][A-Za-z0-9._-]{0,48})$").unwrap();
    let root = parts[0];
    let captures = root_re.captures(root).ok_or("Invalid package root")?;
    if root.ends_with('.') {
        return Err("Windows-trimmed package root".into());
    }
    let component_re = Regex::new(r"^[A-Za-z0-9._-]+$").unwrap();
    for component in &parts[1..] {
        if *component == "."
            || *component == ".."
            || component.ends_with('.')
            || !component_re.is_match(component)
        {
            return Err("Unsafe ZIP member path".into());
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        ) {
            return Err("Windows device alias in ZIP member path".into());
        }
    }
    Ok((captures[1].to_owned(), name.to_ascii_lowercase()))
}
fn verify(repo: &Path, archive_path: &Path) -> Result<Manifest, String> {
    verify_with_environment(repo, archive_path, &process_environment())
}
fn verify_with_environment(
    repo: &Path,
    archive_path: &Path,
    environment: &[(OsString, OsString)],
) -> Result<Manifest, String> {
    assert_git_identity(repo, environment)?;
    let file = File::open(archive_path).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|e| e.to_string())?;
    if let Err(error) = zip.validate_header_consistency() {
        let message = error.to_string();
        if message.contains("duplicate file names") {
            return Err("Duplicate package entries".into());
        }
        return Err(format!("Invalid ZIP member headers: {message}"));
    }
    let mut names = BTreeSet::new();
    let mut canonical_names = BTreeSet::new();
    let mut canonical_paths = Vec::new();
    let mut prefixes = BTreeSet::new();
    let mut files = HashMap::new();
    for index in 0..zip.len() {
        let mut member = zip.by_index(index).map_err(|e| e.to_string())?;
        let name = member.name().map_err(|e| e.to_string())?.into_owned();
        if member.name_raw() != name.as_bytes() {
            return Err("Noncanonical ZIP member name encoding".into());
        }
        if !names.insert(name.clone()) {
            return Err("Duplicate package entries".into());
        }
        if member.is_dir() || member.is_symlink() {
            return Err("Unexpected ZIP member type".into());
        }
        let mode = member.unix_mode().ok_or("Unexpected ZIP member type")?;
        if mode & 0o170000 != 0o100000 {
            return Err("Unexpected ZIP member type".into());
        }
        let (version, canonical) = canonical_member_name(&name)?;
        if !canonical_names.insert(canonical.clone()) {
            return Err("Duplicate normalized package entries".into());
        }
        canonical_paths.push(canonical);
        prefixes.insert(format!("MRI-Drive-Maker-v{version}"));
        let mut contents = Vec::new();
        member
            .read_to_end(&mut contents)
            .map_err(|e| format!("ZIP CRC failure: {e}"))?;
        files.insert(name, contents);
    }
    for path in &canonical_paths {
        let parts: Vec<&str> = path.split('/').collect();
        for index in 1..parts.len() {
            let parent = parts[..index].join("/");
            if canonical_names.contains(&parent) {
                return Err("File/directory collision in ZIP member paths".into());
            }
        }
    }
    if prefixes.len() != 1 {
        return Err("Expected one top-level directory".into());
    }
    let root = prefixes.iter().next().unwrap();
    let version = root
        .strip_prefix("MRI-Drive-Maker-v")
        .ok_or("Invalid package root")?;
    let prefix = root.to_owned() + "/";
    let manifest_bytes = files
        .get(&(prefix.clone() + "RELEASE.json"))
        .ok_or("Missing RELEASE.json")?;
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|e| format!("Bad RELEASE.json: {e}"))?;
    if manifest.version != version {
        return Err("Package version does not match its root".into());
    }
    if !Regex::new(r"^[a-f0-9]{40}$")
        .unwrap()
        .is_match(&manifest.source_commit)
    {
        return Err("Invalid source commit".into());
    }
    let expected: BTreeSet<String> = manifest
        .sha256
        .keys()
        .map(|name| manifest_name(name, &prefix))
        .chain(std::iter::once(prefix.clone() + "RELEASE.json"))
        .collect();
    if names != expected {
        return Err("Unexpected package entries".into());
    }
    if manifest.sha256.keys().collect::<BTreeSet<_>>()
        != manifest.source_files.keys().collect::<BTreeSet<_>>()
    {
        return Err("Every packaged file must have a source origin".into());
    }
    for (name, source) in &manifest.source_files {
        let contents = files
            .get(&manifest_name(name, &prefix))
            .ok_or("Missing declared package file")?;
        if digest(contents) != manifest.sha256[name] {
            return Err(format!("Package file SHA-256 mismatch: {name}"));
        }
        if committed_with_environment(repo, &manifest.source_commit, source, environment)?
            != *contents
        {
            return Err(format!("Source differs from declared Git commit: {name}"));
        }
    }
    let launchers: Vec<&str> = manifest
        .sha256
        .keys()
        .map(String::as_str)
        .filter(|n| {
            !n.contains('/')
                && [".cmd", ".ps1", ".exe"]
                    .iter()
                    .any(|extension| n.to_ascii_lowercase().ends_with(extension))
        })
        .collect();
    if launchers != ["START-MRI-DRIVE-MAKER.cmd"] {
        return Err("Competing launchers".into());
    }
    if manifest
        .sha256
        .keys()
        .any(|n| n.starts_with("Developer/") || n.starts_with("legacy/"))
    {
        return Err("Historical files cannot be included in user download".into());
    }
    Ok(manifest)
}
// zip-rs 9 emits 0x030A for stored files (historical zip-rs: 0x032E); Python zipfile
// uses 0x0314 (20, Unix). For a byte-identical reproducible source ZIP,
// normalize ONLY that central-directory metadata. Do not modify file payloads.
fn normalize_zip_creator_version(path: &Path) -> Result<(), String> {
    let mut bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 22 {
        return Err("ZIP lacks end-of-central-directory record".into());
    }
    let end = bytes.len() - 22;
    if bytes.get(end..end + 4) != Some(&b"PK\x05\x06"[..]) {
        return Err("Expected no ZIP comment or ZIP64 extensions".into());
    }
    let count = u16::from_le_bytes([bytes[end + 10], bytes[end + 11]]) as usize;
    let expected_len = u32::from_le_bytes([
        bytes[end + 12],
        bytes[end + 13],
        bytes[end + 14],
        bytes[end + 15],
    ]) as usize;
    let start = u32::from_le_bytes([
        bytes[end + 16],
        bytes[end + 17],
        bytes[end + 18],
        bytes[end + 19],
    ]) as usize;
    if start > end || start.checked_add(expected_len) != Some(end) {
        return Err("Invalid ZIP central-directory bounds".into());
    }
    let mut cursor = start;
    for _ in 0..count {
        if cursor.checked_add(46).is_none_or(|v| v > end)
            || bytes.get(cursor..cursor + 4) != Some(&b"PK\x01\x02"[..])
        {
            return Err("Invalid ZIP central-directory header".into());
        }
        if ![10u8, 46u8].contains(&bytes[cursor + 4]) || ![0u8, 3u8].contains(&bytes[cursor + 5]) {
            return Err(format!(
                "Unexpected native ZIP creator platform/version: {}/{}",
                bytes[cursor + 4],
                bytes[cursor + 5]
            ));
        }
        // ZIP 9 marks even STORED files as version 10 (ZIP 1.0),
        // while Python's ZipInfo constructor sets version needed to 20.
        // Change only explicitly validated version-needed words and do
        // not modify ZIP sizes, filenames, payload, CRC or flags.
        if bytes[cursor + 6] != 10 || bytes[cursor + 7] != 0 {
            return Err("Unexpected native ZIP central version-needed".into());
        }
        let local = u32::from_le_bytes([
            bytes[cursor + 42],
            bytes[cursor + 43],
            bytes[cursor + 44],
            bytes[cursor + 45],
        ]) as usize;
        if local.checked_add(6).is_none_or(|v| v > start)
            || bytes.get(local..local + 4) != Some(&b"PK\x03\x04"[..])
            || bytes[local + 4] != 10
            || bytes[local + 5] != 0
        {
            return Err("Unexpected native ZIP local header or version-needed".into());
        }
        bytes[local + 4] = 20;
        bytes[cursor + 6] = 20;
        bytes[cursor + 4] = 20; // Python zipfile: version made by 20
        bytes[cursor + 5] = 3; // Python ZipInfo sets Unix creator, including on Windows
        let filename_len = u16::from_le_bytes([bytes[cursor + 28], bytes[cursor + 29]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[cursor + 30], bytes[cursor + 31]]) as usize;
        let comment_len = u16::from_le_bytes([bytes[cursor + 32], bytes[cursor + 33]]) as usize;
        cursor = cursor
            .checked_add(46 + filename_len + extra_len + comment_len)
            .ok_or("ZIP central-directory length overflow")?;
    }
    if cursor != end {
        return Err("ZIP central-directory entries do not match declared size".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    output.write_all(&bytes).map_err(|e| e.to_string())?;
    output.flush().map_err(|e| e.to_string())
}

fn package_sources(repo: &Path) -> Result<Vec<(String, String)>, String> {
    let mut sources = Vec::new();
    for name in [
        "START-MRI-DRIVE-MAKER.cmd",
        "READ-ME-FIRST.txt",
        "LICENSE",
        "NOTICE.md",
    ] {
        sources.push((name.to_owned(), name.to_owned()));
    }
    for name in [
        "MRI-Batch.ps1",
        "MRI.Batch.psm1",
        "Copy-MRIFiles.ps1",
        "NativeProcess.cs",
        "mri-config.example.json",
        "LICENSE",
    ] {
        sources.push((name.to_owned(), format!("App/{name}")));
    }
    let mut tests = Vec::new();
    for item in fs::read_dir(repo.join("tests")).map_err(|e| e.to_string())? {
        let entry = item.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "ps1") {
            tests.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    tests.sort();
    for name in tests {
        sources.push((format!("tests/{name}"), format!("App/tests/{name}")));
    }
    Ok(sources)
}
fn build_with_environment(
    repo: &Path,
    path: &Path,
    version: &str,
    sources: &[(String, String)],
    environment: &[(OsString, OsString)],
) -> Result<Manifest, String> {
    assert_git_identity(repo, environment)?;
    if !Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,48}$")
        .unwrap()
        .is_match(version)
        || version.ends_with('.')
    {
        return Err("Version contains unsupported path characters".into());
    }
    if path.exists() {
        return Err(format!(
            "Refusing to overwrite existing download: {}",
            path.display()
        ));
    }
    let revision = String::from_utf8(git_with_environment(
        repo,
        &["rev-parse", "HEAD"],
        environment,
    )?)
    .map_err(|e| e.to_string())?
    .trim()
    .to_owned();
    let mut entries = IndexMap::<String, Vec<u8>>::new();
    let mut origins = IndexMap::<String, String>::new();
    let mut insert = |source: &str, dest: &str| -> Result<(), String> {
        let content = committed_with_environment(repo, &revision, source, environment)?;
        entries.insert(dest.to_owned(), content);
        origins.insert(dest.to_owned(), source.to_owned());
        Ok(())
    };
    for (source, destination) in sources {
        insert(source, destination)?;
    }
    drop(insert);
    let sha256: IndexMap<String, String> = entries
        .iter()
        .map(|(name, bytes)| (name.clone(), digest(bytes)))
        .collect();
    let manifest = Manifest {
        version: version.to_owned(),
        source_commit: revision,
        launcher: "START-MRI-DRIVE-MAKER.cmd".to_owned(),
        source_files: origins,
        sha256,
    };
    let mut manifest_bytes = serde_json::to_string_pretty(&manifest)
        .map_err(|e| e.to_string())?
        .into_bytes();
    manifest_bytes.push(b'\n');
    entries.insert("RELEASE.json".to_owned(), manifest_bytes);

    fs::create_dir_all(path.parent().ok_or("Missing destination directory")?)
        .map_err(|e| e.to_string())?;
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(output);
    let date = DateTime::from_date_and_time(2000, 1, 1, 0, 0, 0)
        .map_err(|_| "Invalid fixed ZIP timestamp")?;
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(date)
        .unix_permissions(0o644);
    let prefix = format!("MRI-Drive-Maker-v{version}/");
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (name, contents) in sorted {
        zip.start_file(format!("{prefix}{name}"), options)
            .map_err(|e| e.to_string())?;
        zip.write_all(contents).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    normalize_zip_creator_version(path)?;
    let checked = verify_with_environment(repo, path, environment)?;
    if checked != manifest {
        return Err("Release verification differs from original manifest".into());
    }
    Ok(manifest)
}
fn build(repo: &Path, path: &Path, version: &str) -> Result<Manifest, String> {
    let sources = package_sources(repo)?;
    build_with_environment(repo, path, version, &sources, &process_environment())
}
fn main() {
    let mut version = "0.3.0".to_owned();
    let mut output: Option<PathBuf> = None;
    let mut verify_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(key) = args.next() {
        let Some(value) = args.next() else {
            eprintln!("Option is missing a value");
            std::process::exit(2);
        };
        match key.as_str() {
            "--version" => version = value,
            "--output" => output = Some(PathBuf::from(value)),
            "--verify" => verify_path = Some(PathBuf::from(value)),
            _ => {
                eprintln!("Unrecognized option");
                std::process::exit(2)
            }
        }
    }
    let repo = repo_root();
    if let Some(path) = verify_path {
        match verify(&repo, &path) {
            Ok(manifest) => println!(
                "{}",
                serde_json::json!({
                    "verified":path.display().to_string(),
                    "source_commit":manifest.source_commit,
                    "files":manifest.sha256.len()
                })
            ),
            Err(reason) => {
                eprintln!("{reason}");
                std::process::exit(1)
            }
        }
    } else {
        let path = output
            .unwrap_or_else(|| repo.join(format!("downloads/MRI-Drive-Maker-v{version}.zip")));
        match build(&repo, &path, &version) {
            Ok(manifest) => {
                let bytes = fs::read(&path).expect("archive generated");
                println!(
                    "{}",
                    serde_json::json!({
                        "path":path.display().to_string(),
                        "sha256":digest(&bytes),
                        "bytes":bytes.len(),
                        "files":manifest.sha256.len()+1,
                        "source_commit":manifest.source_commit
                    })
                );
            }
            Err(reason) => {
                eprintln!("{reason}");
                std::process::exit(1)
            }
        }
    }
}
#[cfg(test)]
mod tests;
