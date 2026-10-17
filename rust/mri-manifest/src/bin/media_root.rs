// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Rust policy port; the existing PowerShell production code stays intact.
use serde_json::{json, Value};
use std::path::Path;

// Match Assert-MRIPlainDirectory: reject links and Windows junctions in the
// selected directory and every ancestor before probing or descending through it.
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
    // The original Get-MRIMediaRoot starts with [IO.Path]::GetFullPath.
    // On Windows, std::path::absolute uses the same GetFullPathNameW behavior
    // without canonicalize's extended-length \\?\ prefix.
    let mut root = std::path::absolute(value).map_err(|e| e.to_string())?;
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
    use std::path::PathBuf;
    #[test]
    fn detects_boot_files_under_nested_synthetic_wrappers() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("wrapper").join("second");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("bootmgr"), b"synthetic").unwrap();
        assert_eq!(
            policy(&json!({"Path":temp.path()})).unwrap(),
            json!(std::path::absolute(&nested).unwrap().to_string_lossy())
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

    #[cfg(windows)]
    fn assert_original_parity(input: &Path, expected: Option<&Path>, module: &Path) {
        let rust = policy(&json!({"Path":input}));
        let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding -ArgumentList $false
Import-Module -LiteralPath $env:MRI_BATCH_MODULE -Force -DisableNameChecking | Out-Null
try {
    $mediaRoot = Get-MRIMediaRoot -Path $env:MRI_MEDIA_ROOT
    [Console]::Out.Write($mediaRoot)
    exit 0
} catch {
    [Console]::Error.Write($_.Exception.Message)
    exit 1
}
"#;
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .env("MRI_BATCH_MODULE", module)
            .env("MRI_MEDIA_ROOT", input)
            .output()
            .expect("Windows PowerShell 5.1 must be available for the direct parity test");

        assert_eq!(rust.is_ok(), expected.is_some());
        assert_eq!(output.status.success(), expected.is_some());
        if let Some(expected) = expected {
            let expected = expected.to_string_lossy();
            let rust = rust.unwrap();
            let rust = rust.as_str().unwrap();
            let powershell = String::from_utf8(output.stdout).unwrap();
            assert_eq!(rust, expected);
            assert_eq!(powershell, expected);
        }
    }

    #[cfg(windows)]
    #[test]
    fn matches_original_powershell_media_root_for_synthetic_cases() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("media root parity with spaces");
        std::fs::create_dir(&base).unwrap();
        let module =
            std::path::absolute(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../MRI.Batch.psm1"))
                .unwrap();

        let bootmgr_root = base.join("bootmgr root");
        std::fs::create_dir(&bootmgr_root).unwrap();
        std::fs::write(bootmgr_root.join("BOOTMGR"), b"synthetic").unwrap();
        assert_original_parity(&bootmgr_root, Some(&bootmgr_root), &module);

        let efi_root = base.join("efi root");
        let efi_boot = efi_root.join("eFi").join("bOoT");
        std::fs::create_dir_all(&efi_boot).unwrap();
        std::fs::write(efi_boot.join("bOoTx64.EfI"), b"synthetic").unwrap();
        assert_original_parity(&efi_root, Some(&efi_root), &module);

        let wrapper_root = base.join("single wrapper");
        let wrapper_leaf = wrapper_root.join("MRI files").join("nested");
        std::fs::create_dir_all(&wrapper_leaf).unwrap();
        std::fs::write(wrapper_leaf.join("bootmgr"), b"synthetic").unwrap();
        assert_original_parity(&wrapper_root, Some(&wrapper_leaf), &module);

        let one_file_root = base.join("one file");
        std::fs::create_dir(&one_file_root).unwrap();
        std::fs::write(one_file_root.join("payload.bin"), b"synthetic").unwrap();
        assert_original_parity(&one_file_root, Some(&one_file_root), &module);

        let multiple_children = base.join("multiple children");
        std::fs::create_dir_all(multiple_children.join("first")).unwrap();
        std::fs::create_dir_all(multiple_children.join("second")).unwrap();
        assert_original_parity(&multiple_children, Some(&multiple_children), &module);

        let dot_root = base.join("dot component root");
        std::fs::create_dir(&dot_root).unwrap();
        std::fs::write(dot_root.join("bootmgr"), b"synthetic").unwrap();
        let dot_input = base
            .join("not-created")
            .join("..")
            .join("dot component root");
        assert_original_parity(&dot_input, Some(&dot_root), &module);

        let mut depth_ok = base.join("depth nine");
        for index in 0..9 {
            depth_ok.push(format!("wrapper-{index}"));
            std::fs::create_dir(&depth_ok).unwrap();
        }
        std::fs::write(depth_ok.join("bootmgr"), b"synthetic").unwrap();
        assert_original_parity(&base.join("depth nine"), Some(&depth_ok), &module);

        let mut too_deep = base.join("depth ten");
        for index in 0..10 {
            too_deep.push(format!("wrapper-{index}"));
            std::fs::create_dir(&too_deep).unwrap();
        }
        assert_original_parity(&base.join("depth ten"), None, &module);

        assert_original_parity(&base.join("missing root"), None, &module);
    }
}
