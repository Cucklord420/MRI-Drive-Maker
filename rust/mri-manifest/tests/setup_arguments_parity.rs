// SPDX-License-Identifier: GPL-3.0-only
//! Windows-only original PowerShell versus Rust CMD argument-policy regression.
//! The 129 cases are synthetic strings; no batch command is executed.
#[cfg(windows)]
mod windows_oracle {
    use serde_json::{json, Value};
    use std::{fs, path::Path, process::Command};

    fn ps_quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "''"))
    }

    fn fixtures() -> Vec<Value> {
        let mut cases = Vec::new();
        for n in 0..120 {
            let letter = char::from(b'E' + (n % 8) as u8).to_string();
            let number = n + 7;
            let label = format!("MRI_{:02}", (n % 99) + 1);
            let setup = if n % 2 == 0 {
                "setup.cmd"
            } else {
                r"tools\boot.bat"
            };
            let args: Vec<&str> = match n % 5 {
                0 => vec![],
                1 => vec!["--disk={DiskNumber}"],
                2 => vec!["--root={DriveRoot}", "--label={Label}"],
                3 => vec!["mode=USB", "--drive={DriveLetter}"],
                _ => vec!["safe", "fixed.file", "--number={DiskNumber}"],
            };
            cases.push(json!({
                "Letter": letter,
                "Number": number,
                "Label": label,
                "SetupBatch": setup,
                "SetupArguments": args,
                "ShouldReject": false
            }));
        }

        for (batch, args) in [
            ("CON.cmd", vec![]),
            (r"tools\COM9.cmd", vec![]),
            (r"tools\LPT¹.cmd", vec![]),
            (r"tools\..\run.cmd", vec![]),
            (r"tools\\run.cmd", vec![]),
            (r"tools \run.cmd", vec![]),
            ("run.cmd", vec!["&& erase"]),
            ("run.cmd", vec!["%PATH%"]),
            ("run.cmd", vec!["bad|whoami"]),
        ] {
            cases.push(json!({
                "Letter": "E",
                "Number": 1,
                "Label": "MRI_01",
                "SetupBatch": batch,
                "SetupArguments": args,
                "ShouldReject": true
            }));
        }
        cases
    }

    #[test]
    fn one_hundred_twenty_valid_and_nine_rejected_cases_match_original_powershell() {
        let cases = fixtures();
        assert_eq!(cases.len(), 129);
        let directory = tempfile::tempdir().expect("exclusive temporary test directory");
        let cases_path = directory.path().join("cases.json");
        fs::write(&cases_path, serde_json::to_vec(&cases).unwrap()).unwrap();
        let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../MRI.Batch.psm1");
        assert!(
            original.is_file(),
            "the production PowerShell module is required"
        );

        // Rust owns the test and fixtures; the existing original functions are
        // invoked through PowerShell only as the differential oracle.
        let script = r#"
$ErrorActionPreference = 'Stop'
Import-Module -Name __MODULE__ -Force -DisableNameChecking
$cases = Get-Content -LiteralPath __FIXTURE__ -Raw -Encoding UTF8 | ConvertFrom-Json
$results = New-Object 'System.Collections.Generic.List[object]'
foreach ($case in $cases) {
    $config = [pscustomobject]@{
        SetupBatch = [string]$case.SetupBatch
        SetupArguments = [string[]]@($case.SetupArguments)
    }
    $record = [pscustomobject]@{ Label = [string]$case.Label }
    $target = [pscustomobject]@{
        Letter = [string]$case.Letter
        Number = [long]$case.Number
    }
    $outcome = [ordered]@{ Ok = $true; Result = $null; Error = $null }
    try {
        $outcome.Result = [string](Get-MRISetupArguments $config $record $target)
    } catch {
        $outcome.Ok = $false
        $outcome.Error = $_.Exception.Message
    }
    $results.Add([pscustomobject]$outcome)
}
ConvertTo-Json -InputObject ($results.ToArray()) -Compress -Depth 20
"#
        .replace("__MODULE__", &ps_quote(&original))
        .replace("__FIXTURE__", &ps_quote(&cases_path));

        let oracle_output = Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(script)
            .output()
            .expect("Windows PowerShell 5.1 is required for original-function parity");
        assert!(
            oracle_output.status.success(),
            "original PowerShell oracle failed: {}",
            String::from_utf8_lossy(&oracle_output.stderr)
        );
        let expected: Value = serde_json::from_slice(&oracle_output.stdout)
            .expect("original PowerShell must emit valid JSON");
        let expected = expected.as_array().expect("one oracle result per fixture");
        assert_eq!(expected.len(), cases.len());

        let request_path = directory.path().join("request.json");
        for (case_index, (fixture, oracle)) in cases.iter().zip(expected).enumerate() {
            let rejected = fixture["ShouldReject"].as_bool().unwrap();
            let reference_ok = oracle["Ok"].as_bool().unwrap();
            assert_eq!(
                reference_ok, !rejected,
                "case {case_index}: negative fixture was accepted or positive fixture rejected"
            );

            fs::write(&request_path, serde_json::to_vec(fixture).unwrap()).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_setup_arguments"))
                .arg(&request_path)
                .output()
                .expect("Cargo-built setup argument policy CLI must launch");
            let actual: Value =
                serde_json::from_slice(&output.stdout).expect("Rust CLI must emit JSON");
            assert_eq!(
                output.status.success(),
                reference_ok,
                "case {case_index}: Rust CLI exit status differs from original; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(actual["Ok"], oracle["Ok"], "case {case_index}");
            if reference_ok {
                assert_eq!(
                    actual["Result"], oracle["Result"],
                    "case {case_index}: CMD argument string differs"
                );
            } else {
                assert_eq!(
                    actual["Error"], oracle["Error"],
                    "case {case_index}: rejected-input diagnostic differs"
                );
            }
        }
    }
}
