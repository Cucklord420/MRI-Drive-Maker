// SPDX-License-Identifier: GPL-3.0-only
//! Compare the original read-only PowerShell registry policy with the Rust port.
//! Windows-only; all 240 inputs are synthetic and no USB media is enumerated.
#![cfg(windows)]

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command};

const ORIGINAL_ORACLE: &str = r#"
$ErrorActionPreference = 'Stop'
Import-Module -Name $env:MRI_REGISTRY_MODULE -Force -DisableNameChecking
$cases = Get-Content -LiteralPath $env:MRI_REGISTRY_CASES -Raw | ConvertFrom-Json
foreach ($case in $cases) {
    try {
        Assert-MRIRegistry -Records $case.Records
        [Console]::Out.WriteLine('OK')
    }
    catch {
        [Console]::Out.WriteLine('ERR|' + $_.Exception.Message)
    }
}
"#;

fn fixtures() -> Vec<Value> {
    let mut cases = Vec::with_capacity(240);
    for index in 0..20 {
        let serial = format!("ABC-{index:03}");
        let size = 16_000_000_000_i64 + index;
        let identity = format!("{serial}\nUSB\\VID\nTEST\n{size}");
        let key = format!("{:X}", Sha256::digest(identity.as_bytes()));
        for variant in 0..12 {
            let record = json!({
                "Key": key,
                "Serial": serial,
                "PnpId": "USB\\VID",
                "Model": "TEST",
                "Size": size,
                "Label": format!("MRI_{index:02}"),
                "AllowErase": true
            });
            let mut records = vec![record];
            match variant {
                0 => {}
                1 => records[0]["Key"] = json!(key.to_ascii_lowercase()),
                2 => records[0]["Label"] = json!(format!("mri_{index:02}")),
                3 => records[0]["Label"] = json!("MRI_AA"),
                4 => records[0]["Key"] = json!("invalid"),
                5 => records[0]["Serial"] = json!("UNKNOWN"),
                6 => {
                    let mut other = records[0].clone();
                    other["Label"] = json!(format!("mri_{index:02}"));
                    records.push(other);
                }
                7 => records[0]["AllowErase"] = json!("true"),
                8 => {
                    records[0]
                        .as_object_mut()
                        .expect("record is an object")
                        .remove("Key");
                }
                9 => records[0]["PnpId"] = json!("  "),
                10 => records[0]["Serial"] = json!("000"),
                11 => {
                    let mut other = records[0].clone();
                    other["Label"] = json!(format!("MRI_{:02}", index + 40));
                    records.push(other);
                }
                _ => unreachable!("all variants are covered"),
            }
            cases.push(json!({"Records": records}));
        }
    }
    cases
}

#[test]
fn registry_identity_matches_original_powershell_on_240_cases() {
    let cases = fixtures();
    assert_eq!(cases.len(), 240, "the full fixture set must run");
    let temp = tempfile::tempdir().expect("create owned temporary fixture directory");
    let batch = temp.path().join("cases.json");
    fs::write(&batch, serde_json::to_vec(&cases).expect("serialize fixtures"))
        .expect("write synthetic fixture file");

    let module = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../MRI.Batch.psm1");
    assert!(module.is_file(), "original PowerShell module must exist");
    let original = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", ORIGINAL_ORACLE])
        .env("MRI_REGISTRY_MODULE", &module)
        .env("MRI_REGISTRY_CASES", &batch)
        .output()
        .expect("Windows PowerShell 5.1 must be available");
    assert!(
        original.status.success(),
        "PowerShell oracle failed: stdout={} stderr={}",
        String::from_utf8_lossy(&original.stdout),
        String::from_utf8_lossy(&original.stderr)
    );
    let original_stdout = String::from_utf8(original.stdout).expect("ASCII verdicts");
    let expected: Vec<&str> = original_stdout.lines().collect();
    assert_eq!(
        expected.len(),
        cases.len(),
        "original policy must return one decision for every case: {original_stdout}"
    );

    for (index, case) in cases.iter().enumerate() {
        let input = temp.path().join("input.json");
        fs::write(&input, serde_json::to_vec(case).expect("serialize case"))
            .expect("write case");
        let output = Command::new(env!("CARGO_BIN_EXE_registry_identity"))
            .arg(&input)
            .output()
            .expect("run Rust read-only registry validator");
        let rust: Value =
            serde_json::from_slice(&output.stdout).expect("validator returns JSON");
        let oracle = expected[index];
        if oracle == "OK" {
            assert!(
                output.status.success() && rust["Ok"] == true,
                "case {index}: original accepted but Rust rejected: {rust}"
            );
        } else {
            let error = oracle
                .strip_prefix("ERR|")
                .unwrap_or_else(|| panic!("case {index}: malformed oracle verdict {oracle:?}"));
            assert!(
                !output.status.success() && rust["Ok"] == false,
                "case {index}: original rejected {error:?} but Rust accepted: {rust}"
            );
            assert_eq!(
                rust["Error"].as_str(),
                Some(error),
                "case {index}: rejection reason differs"
            );
        }
    }
}
