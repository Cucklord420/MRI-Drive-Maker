// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data subset of Assert-MRIConfig, checked against the original source.
//! This CLI never queries hardware, opens media, or authorizes writes.
//! Source existence, source extension, and setup file existence remain in PowerShell.
use regex::RegexBuilder;
use serde_json::{json, Value};

fn relative_path(path: &str) -> Result<(), String> {
    // Mirror ConvertTo-MRIRelativePath's Windows-specific lexical policy.
    let normalized = path.replace('/', "\\");
    let bytes = normalized.as_bytes();
    if normalized.trim().is_empty()
        || normalized.starts_with('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || normalized
            .chars()
            .any(|ch| (ch as u32) <= 0x1f || matches!(ch, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(format!("A relative file path is required: {path}"));
    }

    for part in normalized.split('\\') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(' ')
            || part.ends_with('.')
        {
            return Err(format!("Unsafe relative file path: {path}"));
        }
        let device = part.split('.').next().unwrap_or("").to_uppercase();
        let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
            || ["COM", "LPT"].iter().any(|prefix| {
                device.strip_prefix(*prefix).is_some_and(|rest| {
                    matches!(
                        rest,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            });
        if reserved {
            return Err(format!("Windows device names are not allowed: {path}"));
        }
    }
    Ok(())
}

fn strings<'a>(input: &'a Value, name: &str) -> Result<Vec<&'a str>, String> {
    let values = input[name]
        .as_array()
        .ok_or_else(|| format!("{name} must be an array."))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("{name} must contain strings."))
        })
        .collect()
}

fn policy(input: &Value) -> Result<Value, String> {
    let names = [
        "SchemaVersion",
        "SourcePath",
        "SetupBatch",
        "SetupArguments",
        "SetupInteractive",
        "SetupTimeoutSeconds",
        "CopyTimeoutSeconds",
        "PrepareDisk",
        "FileSystem",
        "RequiredFiles",
        "MutableFiles",
        "MaxParallel",
        "PollSeconds",
        "StablePolls",
        "MinimumDiskBytes",
        "MaximumDiskBytes",
        "SerializeSetup",
    ];
    for field in names {
        if input.get(field).is_none() {
            return Err(format!("Missing setting: {field}"));
        }
    }
    if input["SchemaVersion"] != 1 {
        return Err("Unsupported configuration version.".into());
    }
    // PowerShell's -in/-notin and -eq string checks are case-insensitive.
    let kind = match input.get("SourceKind") {
        None => "Folder",
        Some(value) => value
            .as_str()
            .ok_or("SourceKind must be Folder, ISO, or ZIP.")?,
    };
    if !["Folder", "ISO", "ZIP"]
        .iter()
        .any(|v| kind.eq_ignore_ascii_case(v))
    {
        return Err("SourceKind must be Folder, ISO, or ZIP.".into());
    }
    if input["SourcePath"]
        .as_str()
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err("Select the official MRI ISO, ZIP, or extracted folder first.".into());
    }
    // Deliberately NOT a full replacement for Assert-MRIConfig: the production
    // PowerShell must still verify the selected source exists and is the correct
    // kind, and that a nonempty setup script actually exists and is nonempty.
    let setup_batch = input["SetupBatch"]
        .as_str()
        .ok_or("SetupBatch must be a string.")?;
    let setup_args = strings(input, "SetupArguments")?;
    let mutable = strings(input, "MutableFiles")?;
    let required = strings(input, "RequiredFiles")?;
    if !setup_batch.trim().is_empty() {
        relative_path(setup_batch)?;
        let lower = setup_batch.to_ascii_lowercase();
        if !(lower.ends_with(".bat") || lower.ends_with(".cmd"))
            || setup_batch
                .chars()
                .any(|ch| matches!(ch, '%' | '!' | '&' | '^' | '(' | ')'))
        {
            return Err("Choose a .bat/.cmd path without CMD metacharacters.".into());
        }
    } else if !setup_args.is_empty() || !mutable.is_empty() || input["SetupInteractive"] == true {
        return Err("Direct-copy mode must have no setup arguments, mutable-file exceptions, or interactive setup.".into());
    }
    for item in required.iter().chain(mutable.iter()) {
        relative_path(item)?;
    }
    let placeholder = RegexBuilder::new(r"\{(DriveLetter|DriveRoot|DiskNumber|Label)\}")
        .case_insensitive(true)
        .build()
        .expect("fixed placeholder regex");
    for arg in setup_args {
        let remainder = placeholder.replace_all(arg, "");
        if remainder.chars().any(|ch| {
            !ch.is_ascii_alphanumeric()
                && !matches!(
                    ch,
                    '_' | ' ' | '.' | ':' | '/' | '\\' | '=' | '+' | ',' | '-'
                )
        }) {
            return Err(format!("Unsupported batch argument: {arg}"));
        }
    }
    for field in ["SetupInteractive", "PrepareDisk", "SerializeSetup"] {
        if !input[field].is_boolean() {
            return Err(format!("{field} must be true or false."));
        }
    }
    if !input["FileSystem"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("FAT32"))
    {
        return Err("This media recipe supports FAT32 only.".into());
    }
    for field in [
        "MaxParallel",
        "PollSeconds",
        "StablePolls",
        "MinimumDiskBytes",
        "MaximumDiskBytes",
        "SetupTimeoutSeconds",
        "CopyTimeoutSeconds",
    ] {
        if input[field].as_i64().is_none() {
            return Err(format!("{field} must be an integer."));
        }
    }
    let parallel = input["MaxParallel"].as_i64().unwrap();
    if !(1..=30).contains(&parallel) {
        return Err("MaxParallel must be 1 through 30.".into());
    }
    let poll = input["PollSeconds"].as_i64().unwrap();
    let stable = input["StablePolls"].as_i64().unwrap();
    if !(1..=30).contains(&poll) || !(2..=30).contains(&stable) {
        return Err("Use PollSeconds 1-30 and StablePolls 2-30.".into());
    }
    if input["MinimumDiskBytes"].as_i64().unwrap() < 1
        || input["MaximumDiskBytes"].as_i64().unwrap() < input["MinimumDiskBytes"].as_i64().unwrap()
    {
        return Err("Invalid capacity limits.".into());
    }
    if input["SetupTimeoutSeconds"].as_i64().unwrap() < 1
        || input["CopyTimeoutSeconds"].as_i64().unwrap() < 1
    {
        return Err("Timeouts must be positive.".into());
    }
    Ok(json!(true))
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("Usage: policy-binary INPUT.json");
        std::process::exit(2);
    };
    let bytes = std::fs::read(path).unwrap_or_else(|error| {
        eprintln!("Unable to read fixture: {error}");
        std::process::exit(2);
    });
    let input: Value = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        eprintln!("Fixture must be JSON: {error}");
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

    fn base() -> Value {
        json!({
            "SchemaVersion": 1, "SourcePath": "C:\\MRI",
            "SetupBatch": "", "SetupArguments": [], "SetupInteractive": false,
            "SetupTimeoutSeconds": 600, "CopyTimeoutSeconds": 1200,
            "PrepareDisk": true, "FileSystem": "FAT32",
            "RequiredFiles": ["bootmgr"], "MutableFiles": [],
            "MaxParallel": 8, "PollSeconds": 2, "StablePolls": 3,
            "MinimumDiskBytes": 8000000000_i64,
            "MaximumDiskBytes": 64000000000_i64,
            "SerializeSetup": false
        })
    }

    #[test]
    fn original_type_and_range_guards() {
        assert!(policy(&base()).is_ok());
        for (field, value) in [
            ("MaxParallel", json!(31)),
            ("PollSeconds", json!(0)),
            ("StablePolls", json!(1)),
            ("FileSystem", json!("exFAT")),
            ("SerializeSetup", json!("yes")),
            ("MinimumDiskBytes", json!(0)),
            ("CopyTimeoutSeconds", json!(0)),
        ] {
            let mut v = base();
            v[field] = value;
            assert!(policy(&v).is_err(), "should reject {field}");
        }
    }

    #[test]
    fn accepts_powershell_case_insensitive_enums_and_file_system() {
        let mut v = base();
        v["SourceKind"] = json!("zip");
        v["FileSystem"] = json!("fat32");
        v["SetupBatch"] = json!("SETUP\\boot.CMD");
        v["SetupArguments"] = json!(["/drive={driveletter}", "/root={DriveRoot}"]);
        v["MutableFiles"] = json!(["EFI/BOOT/bootx64.efi"]);
        assert!(policy(&v).is_ok());
    }

    #[test]
    fn rejects_unsupported_source_kinds_and_invalid_setup_paths() {
        for bad in ["", "WIM", "XZ", "../ISO"] {
            let mut v = base();
            v["SourceKind"] = json!(bad);
            assert!(policy(&v).is_err(), "kind {bad}");
        }
        for bad in [
            "..\\escape.cmd",
            "C:\\Setup.cmd",
            "\\network.cmd",
            "CON.cmd",
            "LPT¹.bat",
            "foo\\.\\bar.cmd",
            "setup.exe",
            "setup&delete.cmd",
            "foo//setup.cmd",
            "foo \\setup.cmd",
        ] {
            let mut v = base();
            v["SetupBatch"] = json!(bad);
            assert!(policy(&v).is_err(), "setup path {bad}");
        }
    }

    #[test]
    fn rejects_direct_copy_setup_mutations_and_bad_media_paths() {
        for (field, value) in [
            ("SetupArguments", json!(["/unsafe"])),
            ("MutableFiles", json!(["bootmgr"])),
            ("SetupInteractive", json!(true)),
        ] {
            let mut v = base();
            v[field] = value;
            assert!(policy(&v).is_err(), "direct copy {field}");
        }
        for bad in [
            "../outside",
            "C:/bootmgr",
            "AUX",
            "folder//file",
            "EFI/BOOT/.",
            "foo/bar ",
            "COM9.txt",
            "\\root",
            "",
        ] {
            let mut v = base();
            v["RequiredFiles"] = json!([bad]);
            assert!(policy(&v).is_err(), "media path {bad}");
        }
    }

    #[test]
    fn rejects_shell_metacharacters_and_invalid_array_types() {
        for bad in ["hi&bye", "{Label}%PATH%", "abc|more", "a;whoami", "$(calc)"] {
            let mut v = base();
            v["SetupBatch"] = json!("setup.bat");
            v["SetupArguments"] = json!([bad]);
            assert!(policy(&v).is_err(), "argument {bad}");
        }
        for field in ["SetupArguments", "RequiredFiles", "MutableFiles"] {
            let mut v = base();
            v[field] = json!("not-an-array");
            assert!(policy(&v).is_err(), "array {field}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn matches_original_powershell_for_shared_configuration_guards() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("synthetic MRI source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("setup.cmd"), b"@echo off\r\n").unwrap();

        let module = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../MRI.Batch.psm1");
        assert!(module.is_file());

        let script = r#"
$ErrorActionPreference = 'Stop'
try {
    Import-Module -Name $env:MRI_BATCH_MODULE -Force -DisableNameChecking -ErrorAction Stop | Out-Null
    $config = $env:MRI_CONFIG_JSON | ConvertFrom-Json
    Assert-MRIConfig -Config $config
    exit 0
} catch {
    [Console]::Error.Write($_.Exception.Message)
    exit 1
}
"#;

        let mut valid = base();
        valid["SourcePath"] = json!(source.to_str().unwrap());
        let mut cases: Vec<(&str, Value, bool)> =
            vec![("existing source folder", valid.clone(), true)];

        let mut value = valid.clone();
        value["SourceKind"] = json!("folder");
        value["FileSystem"] = json!("fat32");
        cases.push(("case-insensitive enums", value, true));

        let mut value = valid.clone();
        value["SetupBatch"] = json!("setup.cmd");
        value["SetupArguments"] = json!(["/drive={DriveLetter}"]);
        value["MutableFiles"] = json!(["bootmgr"]);
        cases.push(("existing setup batch", value, true));

        for (label, field, invalid) in [
            ("invalid parallel maximum", "MaxParallel", json!(31)),
            ("invalid polling interval", "PollSeconds", json!(0)),
            ("invalid stable count", "StablePolls", json!(1)),
            ("invalid filesystem", "FileSystem", json!("exFAT")),
            ("invalid boolean type", "SerializeSetup", json!("yes")),
            ("invalid capacity", "MinimumDiskBytes", json!(0)),
            ("invalid timeout", "CopyTimeoutSeconds", json!(0)),
            ("invalid source kind", "SourceKind", json!("WIM")),
            ("invalid required path", "RequiredFiles", json!(["../outside"])),
            ("invalid setup path", "SetupBatch", json!("../outside.cmd")),
        ] {
            let mut value = valid.clone();
            value[field] = invalid;
            cases.push((label, value, false));
        }

        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("SchemaVersion");
        cases.push(("missing required schema", missing, false));

        for (label, input, expected) in cases {
            let rust = policy(&input);
            let output = std::process::Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    script,
                ])
                .env("MRI_BATCH_MODULE", &module)
                .env("MRI_CONFIG_JSON", input.to_string())
                .output()
                .expect("Windows PowerShell 5.1 is required for direct differential tests");
            assert_eq!(rust.is_ok(), expected, "Rust policy for {label}: {rust:?}");
            assert_eq!(
                output.status.success(),
                expected,
                "PowerShell policy for {label}: status={:?}, stderr={:?}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
