// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let files = input
        .get("Entries")
        .and_then(Value::as_array)
        .ok_or("Missing Entries")?;
    let required = input
        .get("RequiredFiles")
        .and_then(Value::as_array)
        .ok_or("Missing required files list")?;
    let mut count = 0usize;
    for entry in files {
        match entry.get("IsDirectory").and_then(Value::as_bool) {
            Some(false) => count += 1,
            Some(true) => {}
            None => return Err("Invalid IsDirectory flag".into()),
        }
    }
    // Match Invoke-MRIBuild, not a fictitious unconditional boot-file gate.
    if count == 0 {
        return Err("The source contains no files. Preparation was not started.".into());
    }
    let setup_batch = input.get("SetupBatch").ok_or("Missing SetupBatch")?;
    let run_setup = match setup_batch {
        Value::String(batch) => !batch.trim().is_empty(),
        Value::Null => false,
        _ => return Err("Invalid SetupBatch type".into()),
    };
    if !run_setup {
        for name in required {
            let wanted = name.as_str().ok_or("Required file name is invalid")?;
            if !files.iter().any(|file| {
                file["IsDirectory"] == false
                    && file["RelativePath"]
                        .as_str()
                        .is_some_and(|path| path.to_lowercase() == wanted.to_lowercase())
                    && file["Length"].as_u64().is_some_and(|length| length > 0)
            }) {
                return Err(format!(
                    "Required boot file is missing from the source: {wanted}"
                ));
            }
        }
    }
    for file in files {
        if file["IsDirectory"] == true {
            continue;
        }
        let length = file["Length"].as_u64().ok_or("Invalid file length")?;
        if length > 4_294_967_295 {
            return Err("A source file is larger than FAT32 permits.".into());
        }
    }
    Ok(json!({"Files":count}))
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
    fn validates_direct_copy_limits_and_required_boot_files() {
        let base = json!({"Entries":[{"RelativePath":"bootmgr","IsDirectory":false,
            "Length":4294967295_u64}], "RequiredFiles":["BOOTMGR"],"SetupBatch":""});
        assert!(policy(&base).is_ok());
        let mut over = base.clone();
        over["Entries"][0]["Length"] = json!(4294967296_u64);
        assert_eq!(
            policy(&over).unwrap_err(),
            "A source file is larger than FAT32 permits."
        );
        let mut missing = base.clone();
        missing["RequiredFiles"] = json!(["EFI/BOOT/BOOTX64.EFI"]);
        assert_eq!(
            policy(&missing).unwrap_err(),
            "Required boot file is missing from the source: EFI/BOOT/BOOTX64.EFI"
        );
        assert_eq!(
            policy(&json!({"Entries":[],"RequiredFiles":[]})).unwrap_err(),
            "The source contains no files. Preparation was not started."
        );
    }

    #[test]
    fn malformed_directory_flags_cannot_be_counted_as_source_files() {
        for value in [json!("false"), json!(null), json!(1), json!({})] {
            let input = json!({
                "Entries": [{"RelativePath": "bootmgr", "IsDirectory": value, "Length": 10}],
                "RequiredFiles": [],
                "SetupBatch": "setup.cmd"
            });
            assert_eq!(policy(&input).unwrap_err(), "Invalid IsDirectory flag");
        }
        let directories_only = json!({
            "Entries": [{"RelativePath": "EFI", "IsDirectory": true, "Length": 0}],
            "RequiredFiles": [],
            "SetupBatch": "setup.cmd"
        });
        assert_eq!(
            policy(&directories_only).unwrap_err(),
            "The source contains no files. Preparation was not started."
        );
    }

    #[test]
    fn setup_mode_must_not_reject_source_for_missing_final_boot_files() {
        let setup = json!({"Entries":[{"RelativePath":"payload.bin","IsDirectory":false,
            "Length":12}],"RequiredFiles":["bootmgr"],"SetupBatch":"setup.cmd"});
        assert!(policy(&setup).is_ok());
        let mut direct = setup.clone();
        direct["SetupBatch"] = json!("");
        assert!(policy(&direct).is_err());
        let mut oversized = setup;
        oversized["Entries"][0]["Length"] = json!(4294967296_u64);
        assert_eq!(
            policy(&oversized).unwrap_err(),
            "A source file is larger than FAT32 permits."
        );
    }
    #[test]
    fn rejects_non_string_setup_batch_instead_of_treating_it_as_direct_copy() {
        let valid = json!({"Entries":[{"RelativePath":"bootmgr","IsDirectory":false,
            "Length":10}], "RequiredFiles":["BOOTMGR"], "SetupBatch":""});
        assert!(policy(&valid).is_ok());

        let mut invalid = valid.clone();
        invalid["SetupBatch"] = json!(42);
        assert_eq!(policy(&invalid).unwrap_err(), "Invalid SetupBatch type");

        invalid["SetupBatch"] = json!(false);
        assert_eq!(policy(&invalid).unwrap_err(), "Invalid SetupBatch type");

        let mut absent = valid.clone();
        absent.as_object_mut().unwrap().remove("SetupBatch");
        assert_eq!(policy(&absent).unwrap_err(), "Missing SetupBatch");

        let mut nullable = valid;
        nullable["SetupBatch"] = Value::Null;
        assert!(policy(&nullable).is_ok());
    }

    #[test]
    fn unicode_case_insensitive_required_files_match_original_policy() {
        let input = json!({
            "Entries": [{"RelativePath":"Ä.TXT","IsDirectory":false,"Length":12}],
            "RequiredFiles": ["ä.txt"],
            "SetupBatch": ""
        });
        assert!(
            policy(&input).is_ok(),
            "RequiredFiles should use the original PowerShell Unicode case-insensitive comparison"
        );
    }

    #[cfg(windows)]
    #[test]
    fn unicode_required_file_cases_match_windows_powershell_comparisons() {
        for (source, required) in [
            ("Ä.TXT", "ä.txt"),
            ("ÉFI/BOOT/BOOTX64.EFI", "éfi/boot/bootx64.efi"),
            ("BOOTMGR", "bootmgr"),
            ("bootmgr", "BOOTX64.EFI"),
        ] {
            let output = std::process::Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "if ($env:MRI_TEST_SOURCE -eq $env:MRI_TEST_REQUIRED) { 'MATCH' } else { 'NO_MATCH' }",
                ])
                .env("MRI_TEST_SOURCE", source)
                .env("MRI_TEST_REQUIRED", required)
                .output()
                .expect("Windows PowerShell 5.1 must be available for matched-input parity");
            assert!(
                output.status.success(),
                "PowerShell comparison failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let original_match = String::from_utf8_lossy(&output.stdout)
                .trim()
                .eq("MATCH");
            let input = json!({
                "Entries": [{"RelativePath":source,"IsDirectory":false,"Length":12}],
                "RequiredFiles": [required],
                "SetupBatch": ""
            });
            assert_eq!(
                policy(&input).is_ok(),
                original_match,
                "PowerShell/Rust parity differed for {source:?} versus {required:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn paired_original_powershell_preflight_regressions_match_rust_policy() {
        let oversized = json!({
            "Entries": [{"RelativePath":"payload.dat","IsDirectory":false,
                "Length":4_294_967_296_u64}],
            "RequiredFiles": [],
            "SetupBatch": "mri.bat"
        });
        assert_eq!(
            policy(&oversized).unwrap_err(),
            "A source file is larger than FAT32 permits."
        );

        let missing_required = json!({
            "Entries": [{"RelativePath":"payload.dat","IsDirectory":false,"Length":12}],
            "RequiredFiles": ["missing-efi.bin"],
            "SetupBatch": ""
        });
        assert_eq!(
            policy(&missing_required).unwrap_err(),
            "Required boot file is missing from the source: missing-efi.bin"
        );

        let empty_source = json!({
            "Entries": [],
            "RequiredFiles": [],
            "SetupBatch": ""
        });
        assert_eq!(
            policy(&empty_source).unwrap_err(),
            "The source contains no files. Preparation was not started."
        );

        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let repository = manifest_dir
            .parent()
            .and_then(|rust_dir| rust_dir.parent())
            .expect("repository root should be available to the test");
        let self_test = repository.join("tests").join("SelfTest.ps1");
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&self_test)
            .output()
            .expect("Windows PowerShell should be available on the Windows test runner");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "original PowerShell self-tests failed ({}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            stdout,
            stderr
        );

        for test_name in [
            "FAT32 oversized source file is rejected before copy",
            "Missing source boot files block direct-copy preparation before erasure",
            "An empty source cannot reach preparation even with no required filenames",
        ] {
            let expected_line = format!("PASS {test_name}");
            assert!(
                stdout
                    .lines()
                    .any(|line| line.trim() == expected_line.as_str()),
                "original PowerShell self-test did not report {expected_line:?} as passing"
            );
        }
    }
}
