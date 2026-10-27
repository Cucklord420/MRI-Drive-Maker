// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let value = input
        .get("CancelPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !value.is_empty() && std::path::Path::new(value).is_file() {
        Err("Build cancelled.".into())
    } else {
        Ok(json!(true))
    }
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("Usage: policy-binary INPUT.json");
        std::process::exit(2);
    };
    let data = std::fs::read(path).unwrap_or_else(|error| {
        eprintln!("Cannot read local policy fixture: {error}");
        std::process::exit(2);
    });
    let input: Value = serde_json::from_slice(&data).unwrap_or_else(|error| {
        eprintln!("Invalid local JSON fixture: {error}");
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
    fn missing_or_empty_cancel_marker_allows_continue() {
        assert_eq!(policy(&json!({"CancelPath":""})).unwrap(), true);
        assert_eq!(
            policy(&json!({"CancelPath":"__not_a_real_path__"})).unwrap(),
            true
        );
    }
    #[test]
    fn existing_cancel_marker_stops_work() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cancel");
        std::fs::write(&path, b"cancelled").unwrap();
        let input = json!({"CancelPath":path.to_string_lossy()});
        assert_eq!(policy(&input).unwrap_err(), "Build cancelled.");
    }

    // This comparison runs on Windows CI with the original read-only module.
    // It never enumerates or modifies removable drives.
    #[cfg(windows)]
    #[test]
    fn cancellation_guard_matches_original_powershell() {
        use std::process::Command;

        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("cancelled marker.txt");
        let unicode_marker = temp.path().join("annulé.txt");
        std::fs::write(&marker, b"cancel").unwrap();
        std::fs::write(&unicode_marker, b"cancel").unwrap();

        let module = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("MRI.Batch.psm1");
        assert!(module.is_file(), "original PowerShell module is missing");
        let script = r#"
$ErrorActionPreference = 'Stop'
Import-Module -Name $env:MRI_ORIGINAL_MODULE -Force
try {
    Assert-MRINotCancelled -CancelPath $env:MRI_CANCEL_PATH
    'ALLOW'
} catch {
    if ($_.Exception.Message -ne 'Build cancelled.') { throw }
    'CANCEL'
}
"#;

        let cases = [
            ("empty", String::new(), false),
            (
                "absent",
                temp.path().join("absent").to_string_lossy().into_owned(),
                false,
            ),
            ("directory", temp.path().to_string_lossy().into_owned(), false),
            ("existing file", marker.to_string_lossy().into_owned(), true),
            (
                "unicode filename",
                unicode_marker.to_string_lossy().into_owned(),
                true,
            ),
        ];
        for (name, path, should_cancel) in cases {
            let rust_cancelled = policy(&json!({"CancelPath": path.clone()})).is_err();
            assert_eq!(rust_cancelled, should_cancel, "Rust case: {name}");

            let output = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .env("MRI_ORIGINAL_MODULE", &module)
                .env("MRI_CANCEL_PATH", &path)
                .output()
                .expect("run original PowerShell guard");
            assert!(
                output.status.success(),
                "PowerShell case {name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = if should_cancel { "CANCEL" } else { "ALLOW" };
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                expected,
                "PowerShell case {name}"
            );
        }
    }
}
