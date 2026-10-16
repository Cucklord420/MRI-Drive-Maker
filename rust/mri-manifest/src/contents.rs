// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of Assert-MRIContentsSafe. Enumerate names and sizes before
//! hashing anything; never hash an unexpected file or authorize an erase.
use crate::{check_path, compute, file_sha};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const NO_RECEIPT: &str =
    "Drive is not empty and has no successful local MRI receipt. It will not be erased or overwritten.";
const COUNT_MISMATCH: &str =
    "Drive contains unexpected or missing content. It will not be erased or overwritten.";
const ENTRY_MISMATCH: &str =
    "Drive contains unexpected content. It will not be erased or overwritten.";

fn receipt_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    // Receipt JSON is untrusted. Never allow an absolute path, traversal, or
    // separator smuggling to redirect validation outside the selected root.
    check_path(relative)?;
    if relative.contains('/') {
        return Err(format!("Unsafe relative file path: {relative}"));
    }
    let mut result = root.to_path_buf();
    for segment in relative.split('\\') {
        result.push(segment);
    }
    Ok(result)
}

fn assert_not_linked(root: &Path, path: &Path) -> Result<(), String> {
    // Do not follow parent junctions/symlinks introduced after enumeration.
    let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component.as_os_str());
        if fs::symlink_metadata(&cursor).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("A target file became a link.".into());
        }
    }
    Ok(())
}

/// Returns false for empty media, true only for an unchanged, fully verified
/// receipt, and an error for all other nonempty media. Does not write media.
pub(crate) fn verify(root: &Path, receipt: Option<&Value>) -> Result<bool, String> {
    let current = compute(root, true, true)?;
    if current.entries.is_empty() {
        return Ok(false);
    }
    let receipt = receipt.ok_or(NO_RECEIPT)?;
    let previous = receipt
        .get("TargetManifest")
        .and_then(|v| v.get("Entries"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    // Match the original PowerShell case-insensitive hashtable behavior:
    // repeated names replace earlier entries rather than increasing count.
    let mut old = HashMap::<String, &Value>::new();
    for entry in previous {
        if let Some(name) = entry.get("RelativePath").and_then(Value::as_str) {
            old.insert(name.to_lowercase(), entry);
        }
    }
    if old.len() != current.entries.len() {
        return Err(COUNT_MISMATCH.into());
    }
    for entry in &current.entries {
        let Some(previous) = old.get(&entry.relative_path.to_lowercase()) else {
            return Err(ENTRY_MISMATCH.into());
        };
        if previous.get("IsDirectory").and_then(Value::as_bool) != Some(entry.is_directory)
            || previous.get("Length").and_then(Value::as_i64) != Some(entry.length)
        {
            return Err(ENTRY_MISMATCH.into());
        }
    }

    // The cheap no-hash guard is complete. Only now inspect file contents
    // already named and size-matched in the authenticated receipt.
    let mut paths = Vec::with_capacity(previous.len());
    for item in previous {
        let relative = item
            .get("RelativePath")
            .and_then(Value::as_str)
            .ok_or("Invalid receipt relative path")?;
        paths.push((item, relative, receipt_path(root, relative)?));
    }
    for (item, relative, path) in paths {
        assert_not_linked(root, &path)?;
        let directory = item
            .get("IsDirectory")
            .and_then(Value::as_bool)
            .ok_or("Invalid receipt path type")?;
        if directory {
            if !path.is_dir() {
                return Err(format!("Missing directory: {relative}"));
            }
            continue;
        }
        if !path.is_file() {
            return Err(format!("Missing file: {relative}"));
        }
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        if i64::try_from(metadata.len()).ok() != item.get("Length").and_then(Value::as_i64) {
            return Err(format!("File size mismatch: {relative}"));
        }
        let expected = item
            .get("Sha256")
            .and_then(Value::as_str)
            .ok_or("Invalid receipt SHA-256")?;
        if file_sha(&path)? != expected {
            return Err(format!("SHA-256 mismatch: {relative}"));
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_and_receipt_decisions_are_fail_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert_eq!(verify(root, None).unwrap(), false);
        fs::write(root.join("bootmgr"), b"known").unwrap();
        assert_eq!(verify(root, None).unwrap_err(), NO_RECEIPT);
        let receipt = json!({"TargetManifest": compute(root, true, false).unwrap()});
        assert_eq!(verify(root, Some(&receipt)).unwrap(), true);
        fs::write(root.join("bootmgr"), b"tampr").unwrap();
        assert_eq!(
            verify(root, Some(&receipt)).unwrap_err(),
            "SHA-256 mismatch: bootmgr"
        );
        fs::write(root.join("unexpected.bin"), b"private").unwrap();
        assert_eq!(verify(root, Some(&receipt)).unwrap_err(), COUNT_MISMATCH);
    }

    #[test]
    fn system_directories_are_excluded_and_traversal_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("System Volume Information")).unwrap();
        fs::write(root.join("System Volume Information").join("ignored"), b"data").unwrap();
        assert_eq!(verify(root, None).unwrap(), false);
        fs::write(root.join("bootmgr"), b"known").unwrap();
        let receipt = json!({"TargetManifest": compute(root, true, false).unwrap()});
        assert!(verify(root, Some(&receipt)).unwrap());
        assert!(receipt_path(root, "..\\outside").is_err());
        assert!(receipt_path(root, "/outside").is_err());
        assert!(receipt_path(root, "C:\\outside").is_err());
    }
}
