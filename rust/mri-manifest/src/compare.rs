// SPDX-License-Identifier: GPL-3.0-only
//! Pure in-memory Rust port of Assert-MRIManifestMatch; never touches a disk.
use serde_json::Value;
use std::collections::HashMap;
fn member<'a>(entry: &'a Value, name: &str) -> Result<&'a Value, String> {
    entry
        .get(name)
        .ok_or_else(|| format!("Missing {name} manifest field"))
}
pub fn compare(
    expected: &[Value],
    actual: &[Value],
    mutable: &[String],
    exact: bool,
    layout_only: bool,
) -> Result<(), String> {
    if exact && expected.len() != actual.len() {
        return Err("Media contains unexpected or missing paths.".into());
    }
    let mut index: HashMap<String, &Value> = HashMap::new();
    for entry in actual {
        let path = member(entry, "RelativePath")?
            .as_str()
            .ok_or("Bad actual RelativePath")?;
        // Windows PowerShell's default hashtable compares paths
        // case-insensitively, and later duplicate keys replace earlier ones.
        index.insert(path.to_lowercase(), entry);
    }
    for entry in expected {
        let relative = member(entry, "RelativePath")?
            .as_str()
            .ok_or("Bad expected RelativePath")?;
        let Some(actual) = index.get(&relative.to_lowercase()) else {
            return Err(format!("Missing final path: {relative}"));
        };
        let was_dir = member(entry, "IsDirectory")?
            .as_bool()
            .ok_or("Bad IsDirectory")?;
        let now_dir = member(actual, "IsDirectory")?
            .as_bool()
            .ok_or("Bad IsDirectory")?;
        if was_dir != now_dir {
            return Err(format!("Final path type mismatch: {relative}"));
        }
        if was_dir {
            continue;
        }
        let size = member(entry, "Length")?
            .as_i64()
            .ok_or("Bad expected Length")?;
        let actual_size = member(actual, "Length")?
            .as_i64()
            .ok_or("Bad actual Length")?;
        if !mutable
            .iter()
            .any(|m| m.to_lowercase() == relative.to_lowercase())
        {
            let sha = member(entry, "Sha256")?
                .as_str()
                .ok_or("Bad expected SHA")?;
            let actual_sha = member(actual, "Sha256")?.as_str().ok_or("Bad actual SHA")?;
            if size != actual_size || (!layout_only && !sha.eq_ignore_ascii_case(actual_sha)) {
                return Err(format!("Final file mismatch: {relative}"));
            }
        } else if actual_size == 0 && size != 0 {
            return Err(format!("Setup left an empty mutable file: {relative}"));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn exact_and_mutable_cases_are_fail_closed() {
        let source = vec![
            json!({"RelativePath":"EFI\\BOOT\\bootx64.efi","IsDirectory":false,
           "Length":20,"Sha256":"AABB"}),
        ];
        assert!(compare(&source, &source, &[], true, false).is_ok());
        assert_eq!(
            compare(&source, &[], &[], false, false).unwrap_err(),
            "Missing final path: EFI\\BOOT\\bootx64.efi"
        );
        let changed = vec![
            json!({"RelativePath":"efi\\boot\\bootx64.efi","IsDirectory":false,
            "Length":20,"Sha256":"FFFF"}),
        ];
        assert!(compare(&source, &changed, &[], true, true).is_ok());
        assert!(compare(&source, &changed, &[], true, false).is_err());
        assert!(compare(
            &source,
            &changed,
            &["EFI\\BOOT\\bootx64.efi".into()],
            true,
            false
        )
        .is_ok());
        let emptied = vec![json!({"RelativePath":"EFI\\BOOT\\bootx64.efi",
            "IsDirectory":false,"Length":0,"Sha256":""})];
        assert_eq!(
            compare(
                &source,
                &emptied,
                &["efi\\boot\\bootx64.efi".into()],
                true,
                false
            )
            .unwrap_err(),
            "Setup left an empty mutable file: EFI\\BOOT\\bootx64.efi"
        );
    }
}
