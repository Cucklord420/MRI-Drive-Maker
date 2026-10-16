// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Rust policy port; the existing PowerShell production code stays intact.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// Match Assert-MRIPlainDirectory: reject links and Windows junctions in the
// selected directory and every ancestor BEFORE resolving/canonicalizing it.
fn assert_plain_directory(path: &Path) -> Result<(), String> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    for parent in absolute.ancestors() {
        let metadata = std::fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(
                "Source/target paths cannot pass through links or mounted-directory junctions."
                    .into(),
            );
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(
                    "Source/target paths cannot pass through links or mounted-directory junctions."
                        .into(),
                );
            }
        }
    }
    if !std::fs::metadata(absolute)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("A source directory is required.".into());
    }
    Ok(())
}

fn policy(input: &Value) -> Result<Value, String> {
    let value = input["Path"].as_str().ok_or("Missing media folder")?;
    assert_plain_directory(Path::new(value))?;
    let mut root = std::fs::canonicalize(value).map_err(|e| e.to_string())?;
    for _depth in 0..10 {
        assert_plain_directory(&root)?;
        if root.join("bootmgr").is_file()
            || root.join("efi").join("boot").join("bootx64.efi").is_file()
        {
            return Ok(json!(root.to_string_lossy()));
        }
        let entries = std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if entries.len() != 1 {
            return Ok(json!(root.to_string_lossy()));
        }
        let next = entries[0].path();
        let ty = entries[0].file_type().map_err(|e| e.to_string())?;
        if ty.is_symlink() {
            return Err(
                "Source/target paths cannot pass through links or mounted-directory junctions."
                    .into(),
            );
        }
        if !ty.is_dir() {
            return Ok(json!(root.to_string_lossy()));
        }
        assert_plain_directory(&next)?;
        root = next;
    }
    Err(
        "The source has too many nested wrapper folders. Choose its boot-files folder directly."
            .into(),
    )
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
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("wrapper").join("second");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("bootmgr"), b"synthetic").unwrap();
        assert_eq!(
            policy(&json!({"Path":temp.path()})).unwrap(),
            json!(nested.canonicalize().unwrap().to_string_lossy())
        );
    }
    #[test]
    fn rejects_excessive_wrapper_depth() {
        let tmp = tempfile::tempdir().unwrap();
        let mut inner = PathBuf::from(tmp.path());
        for _ in 0..11 {
            inner.push("outer");
            std::fs::create_dir(&inner).unwrap();
        }
        assert!(policy(&json!({"Path":tmp.path()})).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_selected_symlink_nested_symlink_and_linked_ancestor() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("bootmgr"), b"synthetic").unwrap();
        let linked = temp.path().join("linked");
        symlink(&real, &linked).unwrap();
        assert!(policy(&json!({"Path":linked})).is_err());
        let parent = temp.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        let linked_parent = temp.path().join("linked-parent");
        symlink(&parent, &linked_parent).unwrap();
        assert!(policy(&json!({"Path":linked_parent})).is_err());
        let wrapper = temp.path().join("wrapper");
        std::fs::create_dir(&wrapper).unwrap();
        symlink(&real, wrapper.join("child")).unwrap();
        assert!(policy(&json!({"Path":wrapper})).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_directory_links_if_creation_is_permitted() {
        use std::os::windows::fs::symlink_dir;
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let linked = temp.path().join("linked");
        if symlink_dir(&real, &linked).is_ok() {
            assert!(policy(&json!({"Path":linked})).is_err());
        }
    }

}
