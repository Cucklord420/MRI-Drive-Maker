// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let mut ready = input
        .get("Ready")
        .and_then(Value::as_object)
        .cloned()
        .ok_or("Missing Ready map")?;
    let mut failed = input
        .get("Failed")
        .and_then(Value::as_object)
        .cloned()
        .ok_or("Missing Failed map")?;
    let key = input
        .get("Key")
        .and_then(Value::as_str)
        .ok_or("Missing drive key")?;
    let label = input
        .get("Label")
        .and_then(Value::as_str)
        .ok_or("Missing label")?;
    let success = input
        .get("Success")
        .and_then(Value::as_bool)
        .ok_or("Missing Success flag")?;
    let drop_key = |map: &mut serde_json::Map<String, Value>| {
        let existing = map
            .keys()
            .filter(|entry| entry.eq_ignore_ascii_case(key))
            .cloned()
            .collect::<Vec<_>>();
        for name in existing {
            map.remove(&name);
        }
    };
    drop_key(&mut ready);
    drop_key(&mut failed);
    if success {
        ready.insert(key.into(), json!(label));
    } else {
        failed.insert(key.into(), json!(label));
    }
    Ok(json!({"Ready":ready,"Failed":failed}))
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
    fn ready_and_failed_cannot_coexist() {
        let input = json!({"Ready":{"disk-a":"old"},"Failed":{"DISK-A":"x"},
            "Key":"Disk-A","Label":"MRI_01","Success":true});
        let result = policy(&input).unwrap();
        assert_eq!(result["Ready"]["Disk-A"], "MRI_01");
        assert!(result["Failed"].as_object().unwrap().is_empty());
        let fail = policy(&json!({"Ready":result["Ready"],"Failed":result["Failed"],
            "Key":"disk-a","Label":"MRI_01","Success":false}))
        .unwrap();
        assert!(fail["Ready"].as_object().unwrap().is_empty());
        assert_eq!(fail["Failed"]["disk-a"], "MRI_01");
    }

    // Exercise the original exported PowerShell policy on Windows CI.
    // All state lives in process-local hashtables; no media is accessed.
    #[cfg(windows)]
    #[test]
    fn outcomes_match_original_powershell() {
        use std::process::Command;

        let module = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("MRI.Batch.psm1");
        assert!(module.is_file(), "original PowerShell module is missing");
        let script = r#"
$ErrorActionPreference = 'Stop'
Import-Module -Name $env:MRI_ORIGINAL_MODULE -Force -DisableNameChecking
$case = ConvertFrom-Json -InputObject $env:MRI_SCENARIO_JSON
$ready = @{}
$failed = @{}
foreach ($entry in $case.Ready.PSObject.Properties) { $ready[$entry.Name] = $entry.Value }
foreach ($entry in $case.Failed.PSObject.Properties) { $failed[$entry.Name] = $entry.Value }
Set-MRIOutcome -Ready $ready -Failed $failed -Key ([string]$case.Key) -Label ([string]$case.Label) -Success ([bool]$case.Success)
ConvertTo-Json -InputObject @{ Ready = $ready; Failed = $failed } -Depth 10 -Compress
"#;
        let cases = [
            (
                "new ready",
                json!({"Ready":{},"Failed":{},"Key":"disk-a","Label":"MRI_A","Success":true}),
            ),
            (
                "same-state update with case change",
                json!({"Ready":{"disk-a":"old"},"Failed":{},"Key":"DISK-A","Label":"MRI_A","Success":true}),
            ),
            (
                "failed to ready",
                json!({"Ready":{},"Failed":{"disk-a":"bad"},"Key":"DISK-A","Label":"MRI_A","Success":true}),
            ),
            (
                "ready to failed",
                json!({"Ready":{"Disk-A":"old"},"Failed":{},"Key":"disk-a","Label":"MRI_A","Success":false}),
            ),
            (
                "preserve unrelated entries",
                json!({"Ready":{"other":"keep"},"Failed":{"disk-a":"bad"},"Key":"DISK-A","Label":"MRI_A","Success":false}),
            ),
        ];
        for (name, input) in cases {
            let rust = policy(&input).expect("Rust policy accepts fixture");
            let output = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .env("MRI_ORIGINAL_MODULE", &module)
                .env("MRI_SCENARIO_JSON", input.to_string())
                .output()
                .expect("invoke original PowerShell policy");
            assert!(
                output.status.success(),
                "PowerShell case {name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let actual: Value =
                serde_json::from_slice(&output.stdout).expect("PowerShell JSON response");
            assert_eq!(rust, actual, "policy mismatch: {name}");
        }
    }
}
