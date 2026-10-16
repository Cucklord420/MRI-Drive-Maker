// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data MRI policy migration; requires independent PowerShell acceptance.
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

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
        fields.insert(key.into(), value.clone());
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
    fn missing_required_fields_fail() {
        assert!(policy(&json!({"Config":{},"SourceManifest":{}})).is_err());
    }
}
