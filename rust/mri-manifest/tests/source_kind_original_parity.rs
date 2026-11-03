// SPDX-License-Identifier: GPL-3.0-only
//! Original-function differential checks; no USB or disk operations.
#![cfg(windows)]

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

const CASES: &[&str] = &[
    r#"{}"#,
    r#"{"SourceKind":"Folder"}"#,
    r#"{"SourceKind":"ISO"}"#,
    r#"{"SourceKind":"ZIP"}"#,
    r#"{"SourceKind":"folder"}"#,
    r#"{"SourceKind":"zip"}"#,
    r#"{"SourceKind":"isO"}"#,
    r#"{"sourcekind":"ZIP"}"#,
    r#"{"SOURCEKIND":"Folder"}"#,
    r#"{"Sourcekind":"iso"}"#,
    r#"{"SourceKind":""}"#,
    r#"{"SourceKind":"Folder "}"#,
    r#"{"SourceKind":" ISO"}"#,
    r#"{"SourceKind":"Archive"}"#,
    r#"{"SourceKind":null}"#,
    r#"{"SourceKind":42}"#,
    r#"{"SourceKind":false}"#,
    r#"{"SourceKind":[]}"#,
    r#"{"SourceKind":{}}"#,
    r#"{"OtherSetting":"ZIP"}"#,
    r#"{"sourcekind":null}"#,
    r#"{"SourceKind":"ZIP","sourcekind":"Folder"}"#,
    r#"{"SourceKind":"FOLDER"}"#,
    r#"{"SourceKind":"zIp"}"#,
];

// The production PowerShell module is the oracle; Rust owns the test harness,
// fixtures, assertions, process lifecycle, and temporary files.
const ORACLE: &str = r#"
$ErrorActionPreference = 'Stop'
Import-Module -Name $env:MRI_SOURCE_MODULE -Force -DisableNameChecking -ErrorAction Stop | Out-Null
try {
    $config = ConvertFrom-Json -InputObject $env:MRI_SOURCE_JSON -ErrorAction Stop
    $kind = Get-MRISourceKind $config
    [Console]::Out.Write('OK:' + [string]$kind)
} catch {
    [Console]::Out.Write('ERR')
}
"#;

#[test]
fn source_kind_matches_original_powershell_on_synthetic_cases() {
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../MRI.Batch.psm1");
    assert!(module.is_file(), "original PowerShell source is required");
    let rust_binary = env!("CARGO_BIN_EXE_source_kind");
    let temp = tempfile::tempdir().expect("create isolated fixture directory");
    let input = temp.path().join("source kind case.json");

    for (index, case) in CASES.iter().enumerate() {
        fs::write(&input, case).expect("write disposable JSON input");

        let original = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", ORACLE])
            .env("MRI_SOURCE_MODULE", &module)
            .env("MRI_SOURCE_JSON", case)
            .output()
            .expect("launch original Windows PowerShell");
        assert!(
            original.status.success(),
            "original PowerShell oracle failed for case {index}: {}",
            String::from_utf8_lossy(&original.stderr)
        );
        let original_result = String::from_utf8(original.stdout)
            .expect("original decision is UTF-8");
        let original_result = original_result.trim();

        let rust = Command::new(rust_binary)
            .arg(&input)
            .output()
            .expect("launch built Rust source-kind CLI");
        let decision: Value = serde_json::from_slice(&rust.stdout)
            .unwrap_or_else(|error| panic!("invalid Rust JSON for case {index}: {error}"));

        if let Some(expected) = original_result.strip_prefix("OK:") {
            assert!(
                rust.status.success(),
                "Rust rejected original-accepted case {index}: {case}"
            );
            assert_eq!(decision.get("Ok").and_then(Value::as_bool), Some(true));
            assert_eq!(
                decision.get("Result").and_then(Value::as_str),
                Some(expected),
                "case {index}: {case}"
            );
        } else {
            assert_eq!(
                original_result, "ERR",
                "unexpected oracle output for case {index}"
            );
            assert!(
                !rust.status.success(),
                "Rust accepted original-rejected case {index}: {case}"
            );
            assert_eq!(decision.get("Ok").and_then(Value::as_bool), Some(false));
        }
    }
}
