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
        if entry["IsDirectory"] != true {
            count += 1;
        }
    }
    // Match Invoke-MRIBuild, not a fictitious unconditional boot-file gate.
    if count == 0 {
        return Err("The source contains no files. Preparation was not started.".into());
    }
    let run_setup = input
        .get("SetupBatch")
        .and_then(Value::as_str)
        .is_some_and(|batch| !batch.trim().is_empty());
    if !run_setup {
        for name in required {
            let wanted = name.as_str().ok_or("Required file name is invalid")?;
            if !files.iter().any(|file| {
                file["IsDirectory"] == false
                    && file["RelativePath"]
                        .as_str()
                        .is_some_and(|path| path.eq_ignore_ascii_case(wanted))
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
}
