// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Rust policy port; the existing PowerShell production code stays intact.
use serde_json::{json, Value};
use std::path::{Path,PathBuf};

fn policy(input: &Value) -> Result<Value, String> {
    let value=input["Path"].as_str().ok_or("Missing media folder")?;
    let mut root=std::fs::canonicalize(value).map_err(|e|e.to_string())?;
    for _depth in 0..10 {
        if !root.is_dir(){return Err("Media root must be a folder".into());}
        if std::fs::symlink_metadata(&root).is_ok_and(|m|m.file_type().is_symlink()){
            return Err("Media wrapper links are prohibited".into());
        }
        if root.join("bootmgr").is_file()
            || root.join("efi").join("boot").join("bootx64.efi").is_file() {
            return Ok(json!(root.to_string_lossy()));
        }
        let entries=std::fs::read_dir(&root).map_err(|e|e.to_string())?
            .collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        if entries.len()!=1 {return Ok(json!(root.to_string_lossy()));}
        let next=entries[0].path();
        let ty=entries[0].file_type().map_err(|e|e.to_string())?;
        if !ty.is_dir() || ty.is_symlink(){return Ok(json!(root.to_string_lossy()));}
        root=next;
    }
    Err("The source has too many nested wrapper folders. Choose its boot-files folder directly.".into())
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("Usage: policy-binary INPUT.json");
        std::process::exit(2);
    };
    let data = std::fs::read(path).unwrap_or_else(|error| {
        eprintln!("Cannot read local fixture: {error}");
        std::process::exit(2);
    });
    let input: Value = serde_json::from_slice(&data).unwrap_or_else(|error| {
        eprintln!("Invalid JSON fixture: {error}");
        std::process::exit(2);
    });
    match policy(&input) {
        Ok(result) => println!("{}", json!({"Ok":true,"Result":result})),
        Err(error) => {
            println!("{}", json!({"Ok":false,"Error":error}));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_boot_files_under_nested_synthetic_wrappers() {
        let temp=tempfile::tempdir().unwrap();
        let nested=temp.path().join("wrapper").join("second");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("bootmgr"),b"synthetic").unwrap();
        assert_eq!(policy(&json!({"Path":temp.path()})).unwrap(),
            json!(nested.to_string_lossy()));
    }
    #[test]
    fn rejects_excessive_wrapper_depth() {
        let tmp=tempfile::tempdir().unwrap();
        let mut inner=PathBuf::from(tmp.path());
        for _ in 0..11 {inner.push("outer");std::fs::create_dir(&inner).unwrap();}
        assert!(policy(&json!({"Path":tmp.path()})).is_err());
    }
}
