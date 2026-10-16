// SPDX-License-Identifier: GPL-3.0-only
//! Read-only source manifest calculation; never formats or writes a target disk.
//! Ports PowerShell Get-MRIManifest and Get-MRIFileHash for supported files.
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use walkdir::WalkDir;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Entry {
    relative_path: String,
    is_directory: bool,
    length: i64,
    sha256: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Manifest {
    entries: Vec<Entry>,
    total_bytes: i64,
    digest: String,
}
fn sha_upper(bytes: &[u8]) -> String {
    format!("{:X}", Sha256::digest(bytes))
}
fn file_sha(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let count = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        sha.update(&buf[..count]);
    }
    Ok(format!("{:X}", sha.finalize()))
}
fn check_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty()
        || path.starts_with('\\')
        || path
            .chars()
            .any(|x| x.is_control() || matches!(x, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(format!("A relative file path is required: {path}"));
    }
    let reserved =
        regex::Regex::new(r"(?i)^(?:CON|PRN|AUX|NUL|CLOCK\$|COM[1-9¹²³]|LPT[1-9¹²³])(?:\.|$)")
            .map_err(|e| e.to_string())?;
    for part in path.split('\\') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(' ')
            || part.ends_with('.')
        {
            return Err(format!("Unsafe relative file path: {path}"));
        }
        if reserved.is_match(part) {
            return Err(format!("Windows device names are not allowed: {path}"));
        }
    }
    Ok(())
}
fn root_is_physical(path: &Path) -> Result<(), String> {
    let mut parent = Some(path);
    while let Some(next) = parent {
        if fs::symlink_metadata(next).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(
                "Source/target paths cannot pass through links or mounted-directory junctions."
                    .into(),
            );
        }
        parent = next.parent();
    }
    if !path.is_dir() {
        return Err("Source/target root must be a real directory, not a link.".into());
    }
    Ok(())
}
pub fn compute(root: &Path, ignore_system: bool, no_hash: bool) -> Result<Manifest, String> {
    root_is_physical(root)?;
    let mut entries = Vec::<Entry>::new();
    let mut seen = HashSet::new();
    for item in WalkDir::new(root).min_depth(1).follow_links(false) {
        let item = item.map_err(|e| e.to_string())?;
        let path = item.path();
        let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
        let parts = relative
            .components()
            .map(|part| part.as_os_str().to_str().ok_or("Non-UTF8 source path"))
            .collect::<Result<Vec<_>, _>>()?;
        if parts.is_empty() {
            continue;
        }
        if ignore_system
            && ["System Volume Information", "$RECYCLE.BIN"]
                .iter()
                .any(|x| parts[0].eq_ignore_ascii_case(x))
        {
            continue;
        }
        let name = parts.join("\\");
        check_path(&name)?;
        if item.file_type().is_symlink() {
            return Err(format!("Links are not allowed in media content: {name}"));
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(format!("Case-insensitive path collision: {name}"));
        }
        let is_directory = item.file_type().is_dir();
        let mut entry = Entry {
            relative_path: name,
            is_directory,
            length: 0,
            sha256: String::new(),
        };
        if !is_directory {
            let before = fs::metadata(path).map_err(|e| e.to_string())?;
            entry.length = i64::try_from(before.len())
                .map_err(|_| "File is too large for signed 64-bit length")?;
            if !no_hash {
                entry.sha256 = file_sha(path)?;
                let after = fs::metadata(path).map_err(|e| e.to_string())?;
                if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
                    return Err(format!(
                        "A file changed while being hashed: {}",
                        entry.relative_path
                    ));
                }
            }
        }
        entries.push(entry);
    }
    entries.sort_by(|a, b| {
        a.relative_path
            .to_lowercase()
            .cmp(&b.relative_path.to_lowercase())
    });
    let mut serial = Vec::<Value>::new();
    let mut total = 0i64;
    for entry in &entries {
        total = total
            .checked_add(entry.length)
            .ok_or("Total byte count overflow")?;
        let mut properties = Map::new();
        properties.insert(
            "RelativePath".into(),
            Value::String(entry.relative_path.clone()),
        );
        properties.insert("IsDirectory".into(), Value::Bool(entry.is_directory));
        properties.insert("Length".into(), Value::Number(entry.length.into()));
        properties.insert("Sha256".into(), Value::String(entry.sha256.clone()));
        serial.push(Value::Object(properties));
    }
    let json = serde_json::to_vec(&serial).map_err(|e| e.to_string())?;
    Ok(Manifest {
        entries,
        total_bytes: total,
        digest: sha_upper(&json),
    })
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: mri-manifest ROOT [--ignore-system-directories] [--no-hash]");
        std::process::exit(2);
    }
    let mut ignore = false;
    let mut nohash = false;
    for flag in &args[1..] {
        match flag.as_str() {
            "--ignore-system-directories" => ignore = true,
            "--no-hash" => nohash = true,
            _ => {
                eprintln!("Unknown argument");
                std::process::exit(2);
            }
        }
    }
    match compute(Path::new(&args[0]), ignore, nohash) {
        Ok(value) => println!(
            "{}",
            serde_json::to_string(&value).expect("serialize manifest")
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1)
        }
    }
}
#[cfg(test)]
mod tests;
