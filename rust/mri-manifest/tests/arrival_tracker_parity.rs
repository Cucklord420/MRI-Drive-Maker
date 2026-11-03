// SPDX-License-Identifier: GPL-3.0-only
//! Windows-only differential test against the unchanged production PowerShell
//! arrival tracker. All drive records are synthetic; no hardware is queried.
#[cfg(windows)]
mod windows_oracle {
    use serde_json::{json, Map, Value};
    use std::{fs, path::Path, process::Command};

    fn ps_quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "''"))
    }

    fn cases() -> Vec<Value> {
        (0..100)
            .map(|n| {
                let key = if n % 2 == 0 { "DISK-X" } else { "disk-x" };
                let prior_key = if n % 2 == 0 { "disk-x" } else { "DISK-X" };
                let number = if n % 9 == 0 { 8 } else { 7 };
                let mut tracker = Map::new();
                tracker.insert(
                    prior_key.to_owned(),
                    json!({
                        "Number": 7,
                        "Stable": n % 5,
                        "Attempted": n % 17 == 0
                    }),
                );
                let inventory = if n % 13 == 0 {
                    Vec::new()
                } else {
                    vec![json!({
                        "Key": key,
                        "Number": number,
                        "Serial": "SIMULATED-SERIAL",
                        "BusType": "USB",
                        "IsBoot": false,
                        "IsSystem": false,
                        "IsOffline": n % 7 == 0,
                        "IsReadOnly": false,
                        "Size": 32_000_000
                    })]
                };
                let protected_keys: Vec<&str> =
                    if n % 11 == 0 { vec![key] } else { Vec::new() };
                json!({
                    "Inventory": inventory,
                    "Records": [{"Key": "disk-x", "Label": "MRI_01"}],
                    "Config": {
                        "StablePolls": 3,
                        "MinimumDiskBytes": 1000,
                        "MaximumDiskBytes": 100_000_000
                    },
                    "ProtectedKeys": protected_keys,
                    "Tracker": Value::Object(tracker)
                })
            })
            .collect()
    }

    #[test]
    fn one_hundred_synthetic_arrival_decisions_match_original_powershell() {
        let directory = tempfile::tempdir().expect("exclusive temporary test directory");
        let fixtures = cases();
        assert_eq!(fixtures.len(), 100);

        let cases_path = directory.path().join("cases.json");
        fs::write(&cases_path, serde_json::to_vec(&fixtures).unwrap()).unwrap();
        let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../MRI.Batch.psm1");
        assert!(module.is_file(), "the original production module is required");

        // The harness is Rust; PowerShell is invoked only to run the existing
        // original functions as the differential oracle.
        let script = r#"
$ErrorActionPreference = 'Stop'
Import-Module -Name __MODULE__ -Force -DisableNameChecking
$cases = Get-Content -LiteralPath __FIXTURE__ -Raw | ConvertFrom-Json
$results = New-Object 'System.Collections.Generic.List[object]'
foreach ($case in $cases) {
    $tracker = @{}
    foreach ($property in $case.Tracker.PSObject.Properties) {
        $tracker[$property.Name] = $property.Value
    }
    $inventory = @($case.Inventory)
    $records = @($case.Records)
    $config = $case.Config
    $protectedKeys = [string[]]@($case.ProtectedKeys)
    $blocked = @(
        foreach ($drive in $inventory) {
            [bool](Get-MRIDriveBlockReason -Drive $drive -Inventory $inventory -Config $config -ProtectedKeys $protectedKeys)
        }
    )
    $arrivals = @(Update-MRIArrivals -Tracker $tracker -Inventory $inventory -Records $records -Config $config -ProtectedKeys $protectedKeys)
    $results.Add([pscustomobject]@{
        Blocked = $blocked
        Arrivals = $arrivals
        Tracker = $tracker
    })
}
ConvertTo-Json -InputObject ($results.ToArray()) -Compress -Depth 20
"#
        .replace("__MODULE__", &ps_quote(&module))
        .replace("__FIXTURE__", &ps_quote(&cases_path));

        let oracle_output = Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(script)
            .output()
            .expect("Windows PowerShell 5.1 is required for original-function parity");
        assert!(
            oracle_output.status.success(),
            "PowerShell oracle failed: {}",
            String::from_utf8_lossy(&oracle_output.stderr)
        );
        let expected: Value = serde_json::from_slice(&oracle_output.stdout)
            .expect("the original PowerShell oracle must emit valid JSON");
        let expected = expected.as_array().expect("one result for every case");
        assert_eq!(expected.len(), fixtures.len());

        let request_path = directory.path().join("request.json");
        for (case_index, (fixture, oracle)) in fixtures.iter().zip(expected).enumerate() {
            let mut request = json!({
                "Inventory": fixture["Inventory"],
                "Records": fixture["Records"],
                "StablePolls": fixture["Config"]["StablePolls"],
                "Tracker": fixture["Tracker"]
            });
            let inventory = request["Inventory"].as_array_mut().unwrap();
            let blocked = oracle["Blocked"].as_array().expect("one block decision per drive");
            assert_eq!(blocked.len(), inventory.len(), "case {case_index}");
            for (drive, decision) in inventory.iter_mut().zip(blocked) {
                drive["Blocked"] = Value::Bool(decision.as_bool().unwrap());
            }
            fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();

            let output = Command::new(env!("CARGO_BIN_EXE_arrival_tracker"))
                .arg(&request_path)
                .output()
                .expect("the Cargo-built arrival policy CLI must launch");
            assert!(
                output.status.success(),
                "case {case_index}: Rust policy rejected valid oracle fixture: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let actual: Value =
                serde_json::from_slice(&output.stdout).expect("Rust CLI must emit JSON");
            assert_eq!(actual["Ok"], true, "case {case_index}");
            assert_eq!(
                actual["Result"]["Arrivals"], oracle["Arrivals"],
                "case {case_index}: arrival records disagree"
            );

            let rust_tracker = actual["Result"]["Tracker"].as_object().unwrap();
            let ps_tracker = oracle["Tracker"].as_object().unwrap();
            assert_eq!(rust_tracker.len(), ps_tracker.len(), "case {case_index}");
            for (name, expected_state) in ps_tracker {
                let (_, actual_state) = rust_tracker
                    .iter()
                    .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                    .unwrap_or_else(|| panic!("case {case_index}: missing tracker {name}"));
                for field in ["Number", "Stable", "Attempted"] {
                    assert_eq!(
                        actual_state[field], expected_state[field],
                        "case {case_index}: tracker {name} field {field}"
                    );
                }
            }
        }
    }
}
