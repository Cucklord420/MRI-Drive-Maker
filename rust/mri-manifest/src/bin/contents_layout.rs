// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data MRI layout preflight; full content/hash validation remains in PowerShell.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MISSING_OR_EXTRA: &str =
    "Drive contains unexpected or missing content. It will not be erased or overwritten.";
const UNEXPECTED: &str =
    "Drive contains unexpected content. It will not be erased or overwritten.";

fn policy(input: &Value) -> Result<Value, String> {
    let current = input["CurrentEntries"]
        .as_array()
        .ok_or("Missing current entries")?;
    if current.is_empty() {
        return Ok(json!(false));
    }
    let old = input["Receipt"]["TargetManifest"]["Entries"]
        .as_array()
        .ok_or(
            "Drive is not empty and has no successful local MRI receipt. It will not be erased or overwritten.",
        )?;

    // The original PowerShell builds a case-insensitive hashtable first and
    // compares its distinct-key count with the current manifest count.
    let mut expected = HashMap::new();
    for entry in old {
        let path = entry["RelativePath"]
            .as_str()
            .filter(|path| !path.is_empty())
            .ok_or("Malformed receipt path")?;
        if entry["IsDirectory"].as_bool().is_none()
            || !entry["Length"].as_i64().is_some_and(|length| length >= 0)
        {
            return Err(MISSING_OR_EXTRA.into());
        }
        // PowerShell's case-insensitive hashtable accepts repeated receipt
        // paths. Identical repeats preserve its layout decision; inconsistent
        // repeats stay rejected because the full manifest verifier checks
        // every original entry, not just the hashtable's last value.
        if let Some(previous) = expected.insert(path.to_lowercase(), entry) {
            // The PowerShell verifier compares SHA-256 hex case-insensitively.
            let hashes_match = match (
                previous["Sha256"].as_str(),
                entry["Sha256"].as_str(),
            ) {
                (Some(before), Some(after)) => before.eq_ignore_ascii_case(after),
                (None, None) => true,
                _ => false,
            };
            if previous["IsDirectory"] != entry["IsDirectory"]
                || previous["Length"] != entry["Length"]
                || !hashes_match
            {
                return Err(MISSING_OR_EXTRA.into());
            }
        }
    }
    if expected.len() != current.len() {
        return Err(MISSING_OR_EXTRA.into());
    }

    // A valid Get-MRIManifest result already rejects duplicate names. Reject
    // malformed caller-supplied manifests here as a separate fail-closed guard.
    let mut seen_current = HashSet::new();
    for entry in current {
        let path = entry["RelativePath"]
            .as_str()
            .filter(|path| !path.is_empty())
            .ok_or("Malformed target path")?;
        if entry["IsDirectory"].as_bool().is_none()
            || !entry["Length"].as_i64().is_some_and(|length| length >= 0)
        {
            return Err(UNEXPECTED.into());
        }
        let key = path.to_lowercase();
        if !seen_current.insert(key.clone()) {
            return Err(UNEXPECTED.into());
        }
        let valid = expected.get(&key).is_some_and(|old| {
            old["IsDirectory"] == entry["IsDirectory"] && old["Length"] == entry["Length"]
        });
        if !valid {
            return Err(UNEXPECTED.into());
        }
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

    fn entry(path: &str, length: i64) -> Value {
        json!({"RelativePath":path,"IsDirectory":false,"Length":length})
    }

    fn input(current: Vec<Value>, original: Vec<Value>) -> Value {
        json!({
            "CurrentEntries":current,
            "Receipt":{"TargetManifest":{"Entries":original}}
        })
    }

    #[test]
    fn unexpected_client_content_blocks_existing_media_reuse() {
        let original = entry("EFI\\BOOT\\BOOTX64.EFI", 300);
        let baseline = input(vec![original.clone()], vec![original.clone()]);
        assert_eq!(policy(&baseline).unwrap(), true);

        let unknown = input(vec![entry("CLIENT_DATA.DOCX", 300)], vec![original.clone()]);
        assert_eq!(policy(&unknown).unwrap_err(), UNEXPECTED);

        let changed = input(vec![entry("EFI\\BOOT\\BOOTX64.EFI", 12)], vec![original]);
        assert_eq!(policy(&changed).unwrap_err(), UNEXPECTED);
        assert_eq!(policy(&json!({"CurrentEntries":[]})).unwrap(), false);
    }

    #[test]
    fn duplicate_receipt_paths_cannot_fake_a_matching_count() {
        let original = entry("EFI\\BOOT\\BOOTX64.EFI", 300);
        let duplicate = entry("efi\\boot\\bootx64.efi", 300);
        let fixture = input(
            vec![original.clone(), duplicate.clone()],
            vec![original, duplicate],
        );
        assert_eq!(policy(&fixture).unwrap_err(), MISSING_OR_EXTRA);
    }

    #[test]
    fn identical_receipt_duplicates_follow_original_hashtable_parity() {
        let boot = entry("EFI\\BOOT\\BOOTX64.EFI", 300);
        let fixture = input(
            vec![boot.clone()],
            vec![boot.clone(), entry("efi\\boot\\bootx64.efi", 300)],
        );
        assert_eq!(policy(&fixture).unwrap(), true);
    }

    #[test]
    fn duplicate_receipt_hashes_are_case_insensitive_like_powershell() {
        let mut first = entry("EFI\\BOOT\\BOOTX64.EFI", 300);
        first["Sha256"] = json!("A".repeat(64));
        let mut second = entry("efi\\boot\\bootx64.efi", 300);
        second["Sha256"] = json!("a".repeat(64));
        let fixture = input(vec![first.clone()], vec![first, second]);
        assert_eq!(policy(&fixture).unwrap(), true);
    }

    #[test]
    fn conflicting_receipt_duplicates_are_rejected() {
        let boot = entry("EFI\\BOOT\\BOOTX64.EFI", 300);
        let fixture = input(
            vec![boot.clone()],
            vec![entry("efi\\boot\\bootx64.efi", 301), boot],
        );
        assert_eq!(policy(&fixture).unwrap_err(), MISSING_OR_EXTRA);
    }

    #[test]
    fn malformed_entry_schema_does_not_authorize_existing_media_reuse() {
        let boot = entry("EFI\\BOOT\\BOOTX64.EFI", 300);
        let mut fixture = input(vec![boot.clone()], vec![boot.clone()]);
        fixture["Receipt"]["TargetManifest"]["Entries"][0]["Length"] = Value::Null;
        assert_eq!(policy(&fixture).unwrap_err(), MISSING_OR_EXTRA);

        let mut fixture = input(vec![boot.clone()], vec![boot]);
        fixture["CurrentEntries"][0]["IsDirectory"] = json!("false");
        assert_eq!(policy(&fixture).unwrap_err(), UNEXPECTED);
        fixture["CurrentEntries"][0]["IsDirectory"] = json!(false);
        fixture["CurrentEntries"][0]["Length"] = json!(-1);
        assert_eq!(policy(&fixture).unwrap_err(), UNEXPECTED);
    }

    #[test]
    fn duplicate_current_paths_are_rejected_even_with_distinct_receipt_paths() {
        let boot = entry("EFI\\BOOT\\BOOTX64.EFI", 300);
        let alternate = entry("bootmgr", 500);
        let fixture = input(
            vec![boot.clone(), entry("efi\\boot\\bootx64.efi", 300)],
            vec![boot, alternate],
        );
        assert_eq!(policy(&fixture).unwrap_err(), UNEXPECTED);
    }

    #[test]
    fn case_variation_on_one_unique_path_is_accepted() {
        let expected = entry("EFI\\BOOT\\BOOTX64.EFI", 300);
        let current = entry("efi\\boot\\bootx64.efi", 300);
        assert_eq!(policy(&input(vec![current], vec![expected])).unwrap(), true);
    }
}
