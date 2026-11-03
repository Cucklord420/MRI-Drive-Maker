// SPDX-License-Identifier: GPL-3.0-only
//! Windows-only, original-PowerShell oracle for the read-only Rust source-stamp policy.
//! All test files are owned disposable fixtures; no USB devices are enumerated.
#![cfg(windows)]

use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn powershell(script: &str, source: &Path, fixture: Option<&Path>) -> Output {
    let module = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../MRI.Batch.psm1");
    let mut command = Command::new("powershell");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("MRI_PARITY_SOURCE", source)
        .env("MRI_PARITY_MODULE", module);
    if let Some(path) = fixture {
        command.env("MRI_PARITY_INPUT", path);
    }
    let output = command.output().expect("Windows PowerShell must be available");
    assert!(
        output.status.success(),
        "PowerShell harness failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn last_write_ticks(path: &Path) -> i64 {
    let output = powershell(
        "(Get-Item -LiteralPath $env:MRI_PARITY_SOURCE -Force).LastWriteTimeUtc.Ticks",
        path,
        None,
    );
    String::from_utf8(output.stdout)
        .expect("PowerShell tick output must be UTF-8")
        .trim()
        .parse()
        .expect("PowerShell must return a .NET UTC tick number")
}

fn original_accepts(source: &Path, fixture: &Path) -> bool {
    let output = powershell(
        r#"
$ErrorActionPreference = 'Stop'
Import-Module -Name $env:MRI_PARITY_MODULE -Force -DisableNameChecking
$config = Get-Content -LiteralPath $env:MRI_PARITY_INPUT -Raw | ConvertFrom-Json
try {
    Assert-MRIInputSource -Config $config
    Write-Output '__MRI_ACCEPT__'
} catch {
    Write-Output '__MRI_REJECT__'
}
"#,
        source,
        Some(fixture),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let decision = stdout
        .lines()
        .map(str::trim)
        .find(|line| *line == "__MRI_ACCEPT__" || *line == "__MRI_REJECT__")
        .unwrap_or_else(|| panic!("Original PowerShell returned no verdict: {stdout}"));
    decision == "__MRI_ACCEPT__"
}

fn rust_accepts(fixture: &Path) -> bool {
    let output = Command::new(env!("CARGO_BIN_EXE_source_stamp"))
        .arg(fixture)
        .output()
        .expect("the Rust source-stamp binary must execute");
    let decision: Value = serde_json::from_slice(&output.stdout)
        .expect("the Rust source-stamp binary must return JSON");
    output.status.success() && decision["Ok"] == true
}

#[test]
fn source_stamp_matches_original_powershell_across_twenty_windows_cases() {
    let temp = tempfile::tempdir().expect("owned temporary test directory");
    for case in 0_u8..20 {
        let source = temp.path().join(format!("source-{case:02}.iso"));
        fs::write(&source, [0, 1, 2, case]).expect("create synthetic source");
        if (10..15).contains(&case) {
            powershell(
                "[IO.File]::SetLastWriteTimeUtc($env:MRI_PARITY_SOURCE, [DateTime]::SpecifyKind([DateTime]'1965-06-15 12:00:00', [DateTimeKind]::Utc))",
                &source,
                None,
            );
        }
        let length = fs::metadata(&source).expect("read synthetic file").len();
        let ticks = last_write_ticks(&source);
        let path = source.to_string_lossy().into_owned();
        let stamp = json!({"Path":path, "Length":length, "WriteTicks":ticks});
        let mut input = json!({"InputStamp":stamp});
        match case {
            0 => input = json!({}),
            1 => input = json!({"InputStamp":null}),
            2 => input = json!({"inputstamp":stamp}),
            3 => {
                input = json!({"INPUTSTAMP":{
                    "path":path, "length":length, "writeticks":ticks
                }});
            }
            4 | 11 => input["InputStamp"]["Length"] = json!(length + 1),
            5 | 12 => input["InputStamp"]["WriteTicks"] = json!(ticks + 1),
            6 => fs::write(&source, [1, 2, 3, 4, 5]).expect("mutate synthetic file"),
            7 | 14 => {
                powershell(
                    "$f = Get-Item -LiteralPath $env:MRI_PARITY_SOURCE -Force; [IO.File]::SetLastWriteTimeUtc($env:MRI_PARITY_SOURCE, $f.LastWriteTimeUtc.AddSeconds(3))",
                    &source,
                    None,
                );
            }
            8 | 17 => {
                input["InputStamp"]["Path"] =
                    json!(temp.path().join("missing.iso").to_string_lossy());
            }
            9 => {
                input["InputStamp"]
                    .as_object_mut()
                    .expect("stamp object")
                    .remove("WriteTicks");
            }
            13 => {
                input = json!({"inputstamp":{
                    "PATH":path, "LENGTH":length, "WRITETICKS":ticks
                }});
            }
            15 => input = json!({"InputStamp":false}),
            16 => input = json!({"InputStamp":{"Path":path}}),
            18 => input["InputStamp"]["Length"] = json!(0),
            10 | 19 => {}
            _ => unreachable!("covered synthetic case"),
        }
        let fixture = temp.path().join(format!("input-{case:02}.json"));
        fs::write(&fixture, serde_json::to_vec(&input).unwrap()).expect("write synthetic fixture");
        let expected = matches!(case, 0 | 1 | 2 | 3 | 10 | 13 | 19);
        let original = original_accepts(&source, &fixture);
        let rust = rust_accepts(&fixture);
        assert_eq!(original, expected, "original PowerShell case {case}");
        assert_eq!(rust, original, "Rust versus original PowerShell case {case}");
    }
}
