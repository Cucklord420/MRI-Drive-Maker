// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data MRI policy migration; requires independent PowerShell acceptance.
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

fn powershell_array(value: &Value) -> Value {
    match value {
        Value::Array(_) => value.clone(),
        _ => json!([value]),
    }
}

fn policy(input: &Value) -> Result<Value, String> {
    let config = input.get("Config").ok_or("Missing Config")?;
    let manifest = input
        .get("SourceManifest")
        .ok_or("Missing SourceManifest")?;
    let keys = [
        "PrepareDisk",
        "FileSystem",
        "SetupBatch",
        "SetupArguments",
        "SetupInteractive",
        "RequiredFiles",
        "MutableFiles",
    ];
    let mut fields = Map::new();
    fields.insert("Version".into(), json!(2));
    for key in keys {
        let value = config
            .get(key)
            .ok_or_else(|| format!("Missing recipe setting: {key}"))?;
        let recipe_value = if matches!(key, "SetupArguments" | "RequiredFiles" | "MutableFiles") {
            powershell_array(value)
        } else {
            value.clone()
        };
        fields.insert(key.into(), recipe_value);
    }
    fields.insert(
        "SourceDigest".into(),
        manifest
            .get("Digest")
            .ok_or("Missing source digest")?
            .clone(),
    );
    // The manifest enables serde_json/preserve_order. PowerShell hashes an
    // ordered [ordered] hashtable; a key-sorting Map changes every recipe ID.
    let data = serde_json::to_string(&Value::Object(fields)).map_err(|e| e.to_string())?;
    // Windows PowerShell 5.1 ConvertTo-Json uses JavaScriptSerializer, which
    // escapes these HTML-sensitive characters even in compact JSON strings.
    // Hash the original serialized bytes, not only semantically equal JSON.
    let data = data
        .replace('&', r"\u0026")
        .replace('\u{27}', r"\u0027")
        .replace('<', r"\u003c")
        .replace('>', r"\u003e");
    Ok(json!(format!("{:X}", Sha256::digest(data.as_bytes()))))
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

    fn fixture() -> Value {
        json!({
            "Config": {
                "PrepareDisk": true,
                "FileSystem": "FAT32",
                "SetupBatch": "",
                "SetupArguments": [],
                "SetupInteractive": false,
                "RequiredFiles": ["bootmgr"],
                "MutableFiles": [],
                "MaxParallel": 10
            },
            "SourceManifest": {"Digest": "FAKE_SOURCE_HASH"}
        })
    }

    #[test]
    fn operational_changes_do_not_change_media_recipe() {
        let mut a = fixture();
        let reference = policy(&a).unwrap();
        a["Config"]["MaxParallel"] = json!(1);
        a["Config"]["CopyTimeoutSeconds"] = json!(9000);
        assert_eq!(policy(&a).unwrap(), reference);
        a["SourceManifest"]["Digest"] = json!("CHANGED");
        assert_ne!(policy(&a).unwrap(), reference);
    }

    #[test]
    fn ordered_payload_has_stable_digest() {
        // Hash of the original PowerShell field order and compact JSON bytes.
        assert_eq!(
            policy(&fixture()).unwrap(),
            json!("9596912B27B954DD468A179C3CCA649A3420FCF4D9C8FDB3810B7095D6B29674")
        );
    }

    #[test]
    fn windows_powershell_51_html_escapes_preserve_recipe_hash() {
        let mut input = fixture();
        input["SourceManifest"]["Digest"] = json!("SYNTHETIC-009");
        input["Config"]["SetupArguments"] = json!(["embedded \"quote\"", "a&b"]);
        assert_eq!(
            policy(&input).unwrap(),
            json!("60A4456D854230D5A1DC7B129C153072246043D5E0B4F3CB5B222A831128C170")
        );
    }

    #[cfg(windows)]
    fn powershell_oracle(fixture: &Value) -> String {
        use std::path::PathBuf;
        use std::process::{Command, Stdio};

        // Windows PowerShell 5.1 can decode redirected stdin using a legacy
        // code page. Pass the small synthetic JSON fixture as a Unicode
        // -Command argument to preserve non-ASCII parity cases.
        let module = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../MRI.Batch.psm1");
        let escaped_path = module.to_string_lossy().replace('\'', "''");
        let escaped_json = fixture.to_string().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference = 'Stop'; \
             Import-Module '{escaped_path}' -Force -DisableNameChecking; \
             $fixture = '{escaped_json}' | ConvertFrom-Json; \
             Get-MRIRecipe -Config $fixture.Config -SourceManifest $fixture.SourceManifest"
        );
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("original Windows PowerShell reference is required");
        assert!(
            output.status.success(),
            "PowerShell original failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("PowerShell reference result must be UTF-8")
            .trim()
            .to_owned()
    }

    #[cfg(windows)]
    #[test]
    fn one_hundred_recipes_match_original_powershell() {
        for case in 0..100 {
            let mut input = fixture();
            input["SourceManifest"]["Digest"] = json!(format!("SYNTHETIC-{case:03}"));
            match case % 17 {
                0 => {}
                1 => input["Config"]["PrepareDisk"] = json!(false),
                2 => input["Config"]["SetupBatch"] = json!("prepare.cmd"),
                3 => input["Config"]["SetupArguments"] = json!(["/quiet", "/norestart"]),
                4 => input["Config"]["SetupInteractive"] = json!(true),
                5 => input["Config"]["RequiredFiles"] = json!(["bootmgr", "EFI\\BOOT\\BOOTX64.EFI"]),
                6 => input["Config"]["MutableFiles"] = json!(["BCD"]),
                7 => input["Config"]["MaxParallel"] = json!(1),
                8 => input["Config"]["RequiredFiles"] = json!(["café.txt"]),
                9 => input["Config"]["SetupArguments"] = json!(["embedded \"quote\"", "a&b"]),
                10 => input["Config"]["SetupArguments"] = json!("/quiet"),
                11 => input["Config"]["RequiredFiles"] = json!("bootmgr"),
                12 => input["Config"]["MutableFiles"] = json!("BCD"),
                13 => input["Config"]["SetupArguments"] = json!(["x<y"]),
                14 => input["Config"]["SetupArguments"] = json!(["a>b"]),
                15 => input["Config"]["SetupArguments"] = json!(["it's a test"]),
                16 => input["Config"]["SetupArguments"] = json!(["a&b"]),
                _ => unreachable!(),
            }
            let expected = powershell_oracle(&input);
            let actual = policy(&input).unwrap();
            assert_eq!(actual.as_str(), Some(expected.as_str()), "recipe parity case {case}");
        }
    }

    #[test]
    fn powershell_array_coercion_matches_scalar_and_array_settings() {
        for (name, element) in [
            ("SetupArguments", "/quiet"),
            ("RequiredFiles", "bootmgr"),
            ("MutableFiles", "EFI\\BOOT\\BOOTX64.EFI"),
        ] {
            let mut scalar = fixture();
            scalar["Config"][name] = json!(element);
            let mut array = fixture();
            array["Config"][name] = json!([element]);
            assert_eq!(
                policy(&scalar).unwrap(),
                policy(&array).unwrap(),
                "PowerShell array coercion differs for {name}"
            );
        }
    }

    #[test]
    fn missing_required_fields_fail() {
        assert!(policy(&json!({"Config":{},"SourceManifest":{}})).is_err());
    }
}
