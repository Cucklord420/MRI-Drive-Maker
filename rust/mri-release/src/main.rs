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
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};

#[cfg(test)]
thread_local! {
    static LAST_STAGED_PATH: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn record_staged_path(path: &Path) {
    LAST_STAGED_PATH.with(|staged| *staged.borrow_mut() = Some(path.to_path_buf()));
}

#[cfg(test)]
fn release_test_failure_requested(environment: &[(OsString, OsString)], stage: &str) -> bool {
    environment.iter().any(|(key, value)| {
        key.to_str() == Some("MRI_RELEASE_TEST_FAILURE") && value.to_str() == Some(stage)
    })
}

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
fn committed_size_with_environment(
    repo: &Path,
    revision: &str,
    path: &str,
    environment: &[(OsString, OsString)],
) -> Result<u64, String> {
    let output = git_with_environment(
        repo,
        &["cat-file", "-s", &format!("{revision}:{path}")],
        environment,
    )?;
    let size = std::str::from_utf8(&output)
        .map_err(|_| "Git returned a non-UTF-8 source size")?
        .trim()
        .parse::<u64>()
        .map_err(|_| "Git returned an invalid source size")?;
    Ok(size)
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

#[derive(Clone, Copy)]
struct ResourceLimits {
    max_archive_bytes: u64,
    max_entries: usize,
    max_member_bytes: u64,
    max_total_bytes: u64,
    max_manifest_bytes: u64,
}

const DEFAULT_RESOURCE_LIMITS: ResourceLimits = ResourceLimits {
    max_archive_bytes: 128 * 1024 * 1024,
    max_entries: 4096,
    max_member_bytes: 16 * 1024 * 1024,
    max_total_bytes: 64 * 1024 * 1024,
    max_manifest_bytes: 1024 * 1024,
};

const ZIP_STREAM_BUFFER_BYTES: usize = 64 * 1024;

fn read_exact_at<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    buffer: &mut [u8],
) -> std::io::Result<()> {
    reader.seek(SeekFrom::Start(offset))?;
    reader.read_exact(buffer)
}

fn zip64_entry_count<R: Read + Seek>(reader: &mut R, eocd_offset: u64) -> Option<u64> {
    const LOCATOR_BYTES: u64 = 20;
    const ZIP64_EOCD_FIXED_BYTES: u64 = 56;
    let locator_offset = eocd_offset.checked_sub(LOCATOR_BYTES)?;
    let mut locator = [0_u8; LOCATOR_BYTES as usize];
    read_exact_at(reader, locator_offset, &mut locator).ok()?;
    if locator[..4] != *b"PK\x06\x07" {
        return None;
    }

    let zip64_offset = u64::from_le_bytes(locator[8..16].try_into().ok()?);
    let mut zip64_eocd = [0_u8; ZIP64_EOCD_FIXED_BYTES as usize];
    read_exact_at(reader, zip64_offset, &mut zip64_eocd).ok()?;
    if zip64_eocd[..4] != *b"PK\x06\x06" {
        return None;
    }
    let record_size = u64::from_le_bytes(zip64_eocd[4..12].try_into().ok()?);
    if record_size < 44 || zip64_offset.checked_add(12)?.checked_add(record_size)? > locator_offset
    {
        return None;
    }

    let entries_on_disk = u64::from_le_bytes(zip64_eocd[24..32].try_into().ok()?);
    let entries_total = u64::from_le_bytes(zip64_eocd[32..40].try_into().ok()?);
    Some(entries_on_disk.max(entries_total))
}

fn preflight_zip_entry_count<R: Read + Seek>(
    reader: &mut R,
    archive_bytes: u64,
    max_entries: usize,
) -> Result<(), String> {
    const EOCD_FIXED_BYTES: usize = 22;
    const MAX_COMMENT_BYTES: u64 = u16::MAX as u64;
    const EOCD_SIG: &[u8; 4] = b"PK\x05\x06";
    const EOCD_TAIL_BYTES: u64 = EOCD_FIXED_BYTES as u64 + MAX_COMMENT_BYTES;

    if archive_bytes < EOCD_FIXED_BYTES as u64 {
        return Err("ZIP lacks an end-of-central-directory record".into());
    }
    let tail_len = usize::try_from(archive_bytes.min(EOCD_TAIL_BYTES))
        .map_err(|_| "ZIP end-record window does not fit in memory")?;
    let tail_offset = archive_bytes - tail_len as u64;
    let mut tail = Vec::new();
    tail.try_reserve_exact(tail_len)
        .map_err(|error| format!("Cannot reserve ZIP end-record window: {error}"))?;
    tail.resize(tail_len, 0);
    read_exact_at(reader, tail_offset, &mut tail).map_err(|error| error.to_string())?;

    let mut found_eocd = false;
    for position in (0..=tail_len - EOCD_FIXED_BYTES).rev() {
        if tail.get(position..position + 4) != Some(EOCD_SIG) {
            continue;
        }
        let comment_len = u16::from_le_bytes([tail[position + 20], tail[position + 21]]) as usize;
        if position + EOCD_FIXED_BYTES + comment_len > tail_len {
            continue;
        }

        let entries_on_disk = u16::from_le_bytes([tail[position + 8], tail[position + 9]]) as u64;
        let entries_total = u16::from_le_bytes([tail[position + 10], tail[position + 11]]) as u64;
        let directory_size = u32::from_le_bytes([
            tail[position + 12],
            tail[position + 13],
            tail[position + 14],
            tail[position + 15],
        ]) as u64;
        let directory_offset = u32::from_le_bytes([
            tail[position + 16],
            tail[position + 17],
            tail[position + 18],
            tail[position + 19],
        ]) as u64;
        let zip64 = entries_on_disk == u16::MAX as u64
            || entries_total == u16::MAX as u64
            || directory_size == u32::MAX as u64
            || directory_offset == u32::MAX as u64;
        let declared_entries = if zip64 {
            let eocd_offset = tail_offset + position as u64;
            let Some(zip64_entries) = zip64_entry_count(reader, eocd_offset) else {
                continue;
            };
            zip64_entries
                .max(if entries_on_disk == u16::MAX as u64 {
                    0
                } else {
                    entries_on_disk
                })
                .max(if entries_total == u16::MAX as u64 {
                    0
                } else {
                    entries_total
                })
        } else {
            entries_on_disk.max(entries_total)
        };
        found_eocd = true;
        if declared_entries > max_entries as u64 {
            return Err(format!("ZIP entry count exceeds {max_entries}"));
        }
    }

    if found_eocd {
        Ok(())
    } else {
        Err("ZIP lacks a valid end-of-central-directory record".into())
    }
}

#[derive(Debug)]
struct VerifiedMember {
    expanded_bytes: u64,
    sha256: String,
}

fn stream_member<R: Read>(
    reader: &mut R,
    name: &str,
    declared_size: u64,
    retain_contents: bool,
    total_expanded: &mut u64,
    limits: ResourceLimits,
) -> Result<(VerifiedMember, Option<Vec<u8>>), String> {
    if declared_size > limits.max_member_bytes {
        return Err(format!(
            "ZIP member exceeds the per-member expanded-size limit: {name}"
        ));
    }
    if retain_contents && declared_size > limits.max_manifest_bytes {
        return Err("RELEASE.json exceeds the manifest-size limit".into());
    }

    let mut contents = if retain_contents {
        let capacity = usize::try_from(declared_size)
            .map_err(|_| "RELEASE.json size does not fit in memory")?;
        let mut contents = Vec::new();
        contents
            .try_reserve_exact(capacity)
            .map_err(|error| format!("Cannot reserve bounded manifest memory: {error}"))?;
        Some(contents)
    } else {
        None
    };
    let mut hasher = Sha256::new();
    let mut expanded_bytes = 0_u64;
    let mut buffer = [0_u8; ZIP_STREAM_BUFFER_BYTES];

    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("ZIP CRC failure: {error}"))?;
        if count == 0 {
            break;
        }
        let count = u64::try_from(count).map_err(|_| "ZIP read size overflow")?;
        expanded_bytes = expanded_bytes
            .checked_add(count)
            .ok_or("ZIP member expanded-size overflow")?;
        if expanded_bytes > limits.max_member_bytes {
            return Err(format!(
                "ZIP member exceeds the per-member expanded-size limit: {name}"
            ));
        }
        *total_expanded = total_expanded
            .checked_add(count)
            .ok_or("ZIP aggregate expanded-size overflow")?;
        if *total_expanded > limits.max_total_bytes {
            return Err("ZIP exceeds the aggregate expanded-size limit".into());
        }
        let count = usize::try_from(count).map_err(|_| "ZIP read size does not fit in memory")?;
        hasher.update(&buffer[..count]);
        if let Some(contents) = &mut contents {
            contents.extend_from_slice(&buffer[..count]);
        }
    }

    if expanded_bytes != declared_size {
        return Err(format!(
            "ZIP member expanded size differs from its declaration: {name}"
        ));
    }

    Ok((
        VerifiedMember {
            expanded_bytes,
            sha256: format!("{:x}", hasher.finalize()),
        },
        contents,
    ))
}

fn verify_with_environment(
    repo: &Path,
    archive_path: &Path,
    environment: &[(OsString, OsString)],
) -> Result<Manifest, String> {
    verify_with_limits(repo, archive_path, environment, DEFAULT_RESOURCE_LIMITS)
}

fn verify_with_limits(
    repo: &Path,
    archive_path: &Path,
    environment: &[(OsString, OsString)],
    limits: ResourceLimits,
) -> Result<Manifest, String> {
    assert_git_identity(repo, environment)?;
    let mut file = File::open(archive_path).map_err(|e| e.to_string())?;
    let archive_bytes = file.metadata().map_err(|e| e.to_string())?.len();
    if archive_bytes > limits.max_archive_bytes {
        return Err("ZIP archive exceeds the compressed-input-size limit".into());
    }
    preflight_zip_entry_count(&mut file, archive_bytes, limits.max_entries)?;
    let mut zip = ZipArchive::new(file).map_err(|e| e.to_string())?;
    if zip.len() > limits.max_entries {
        return Err(format!("ZIP entry count exceeds {}", limits.max_entries));
    }
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
    let mut entries = Vec::with_capacity(zip.len());
    let mut declared_total = 0_u64;
    for index in 0..zip.len() {
        let member = zip.by_index(index).map_err(|e| e.to_string())?;
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
        let declared_size = member.size();
        if declared_size > limits.max_member_bytes {
            return Err(format!(
                "ZIP member exceeds the per-member expanded-size limit: {name}"
            ));
        }
        if name.ends_with("/RELEASE.json") && declared_size > limits.max_manifest_bytes {
            return Err("RELEASE.json exceeds the manifest-size limit".into());
        }
        declared_total = declared_total
            .checked_add(declared_size)
            .ok_or("ZIP aggregate expanded-size overflow")?;
        if declared_total > limits.max_total_bytes {
            return Err("ZIP exceeds the aggregate expanded-size limit".into());
        }
        entries.push((name, declared_size));
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
    let release_manifest_member_name = prefix.clone() + "RELEASE.json";
    if !names.contains(&release_manifest_member_name) {
        return Err("Missing RELEASE.json".into());
    }

    let mut members = HashMap::with_capacity(entries.len());
    let mut manifest_bytes = None;
    let mut actual_total = 0_u64;
    for (index, (expected_name, expected_size)) in entries.iter().enumerate() {
        let mut member = zip.by_index(index).map_err(|e| e.to_string())?;
        let name = member.name().map_err(|e| e.to_string())?.into_owned();
        let declared_size = member.size();
        if name.as_str() != expected_name.as_str() || declared_size != *expected_size {
            return Err("ZIP metadata changed after resource preflight".into());
        }
        let (verified, contents) = stream_member(
            &mut member,
            &name,
            declared_size,
            name == release_manifest_member_name,
            &mut actual_total,
            limits,
        )?;
        if name == release_manifest_member_name {
            manifest_bytes = contents;
        }
        members.insert(name, verified);
    }

    let manifest_bytes = manifest_bytes.ok_or("Missing RELEASE.json")?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|e| format!("Bad RELEASE.json: {e}"))?;
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
        let verified = members
            .get(&manifest_name(name, &prefix))
            .ok_or("Missing declared package file")?;
        if verified.sha256 != manifest.sha256[name] {
            return Err(format!("Package file SHA-256 mismatch: {name}"));
        }
        let source_size =
            committed_size_with_environment(repo, &manifest.source_commit, source, environment)?;
        if source_size > limits.max_member_bytes {
            return Err(format!(
                "Committed source exceeds the per-member size limit: {name}"
            ));
        }
        if source_size != verified.expanded_bytes {
            return Err(format!("Source differs from declared Git commit: {name}"));
        }
        let source_contents =
            committed_with_environment(repo, &manifest.source_commit, source, environment)?;
        let source_bytes =
            u64::try_from(source_contents.len()).map_err(|_| "Committed source size overflow")?;
        if source_bytes != verified.expanded_bytes || digest(&source_contents) != verified.sha256 {
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

#[cfg(unix)]
fn create_staged_archive(parent: &Path) -> Result<tempfile::NamedTempFile, String> {
    use std::os::unix::fs::PermissionsExt;

    let mut builder = tempfile::Builder::new();
    builder.permissions(fs::Permissions::from_mode(0o666));
    builder.tempfile_in(parent).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn create_staged_archive(parent: &Path) -> Result<tempfile::NamedTempFile, String> {
    tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())
}

fn publish_archive<W, N, V>(
    destination: &Path,
    write_archive: W,
    normalize: N,
    verify: V,
) -> Result<Manifest, String>
where
    W: FnOnce(&mut tempfile::NamedTempFile) -> Result<(), String>,
    N: FnOnce(&Path) -> Result<(), String>,
    V: FnOnce(&Path) -> Result<Manifest, String>,
{
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;

    let mut staged = create_staged_archive(parent)?;
    #[cfg(test)]
    record_staged_path(staged.path());

    write_archive(&mut staged)?;
    staged.as_file_mut().flush().map_err(|e| e.to_string())?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;

    normalize(staged.path())?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;

    let manifest = verify(staged.path())?;
    staged
        .persist_noclobber(destination)
        .map_err(|e| format!("Unable to publish release ZIP without overwriting: {e}"))?;
    Ok(manifest)
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

    let prefix = format!("MRI-Drive-Maker-v{version}/");
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    publish_archive(
        path,
        |staged| {
            let mut zip = ZipWriter::new(staged.as_file_mut());
            let date = DateTime::from_date_and_time(2000, 1, 1, 0, 0, 0)
                .map_err(|_| "Invalid fixed ZIP timestamp")?;
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .last_modified_time(date)
                .unix_permissions(0o644);
            #[cfg(test)]
            let mut first_member = true;
            for (name, contents) in sorted {
                zip.start_file(format!("{prefix}{name}"), options)
                    .map_err(|e| e.to_string())?;
                zip.write_all(contents).map_err(|e| e.to_string())?;
                #[cfg(test)]
                if first_member && release_test_failure_requested(environment, "write") {
                    return Err("injected ZIP write interruption".into());
                }
                #[cfg(test)]
                {
                    first_member = false;
                }
            }
            zip.finish().map_err(|e| e.to_string())?;
            Ok(())
        },
        |staged_path| {
            #[cfg(test)]
            if release_test_failure_requested(environment, "normalize") {
                return Err("injected normalization failure".into());
            }
            normalize_zip_creator_version(staged_path)
        },
        |staged_path| {
            #[cfg(test)]
            if release_test_failure_requested(environment, "verify") {
                return Err("injected verification failure".into());
            }
            let checked = verify_with_environment(repo, staged_path, environment)?;
            #[cfg(test)]
            let checked = if release_test_failure_requested(environment, "manifest-mismatch") {
                let mut mismatched = checked;
                mismatched.version.push_str("-injected-mismatch");
                mismatched
            } else {
                checked
            };
            if checked != manifest {
                return Err("Release verification differs from original manifest".into());
            }
            #[cfg(test)]
            if release_test_failure_requested(environment, "destination-race") {
                fs::write(path, b"COMPETING").map_err(|e| e.to_string())?;
            }
            Ok(checked)
        },
    )
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
