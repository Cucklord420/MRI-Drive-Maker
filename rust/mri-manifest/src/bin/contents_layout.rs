// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data MRI policy migration; requires independent PowerShell acceptance.
use serde_json::{json, Value};
use std::collections::HashMap;

fn policy(input: &Value) -> Result<Value, String> {
    let current=input["CurrentEntries"].as_array().ok_or("Missing current entries")?;
    if current.is_empty(){return Ok(json!(false));}
    let old=input["Receipt"]["TargetManifest"]["Entries"].as_array()
        .ok_or("Drive is not empty and has no successful local MRI receipt. It will not be erased or overwritten.")?;
    if old.len()!=current.len(){
        return Err("Drive contains unexpected or missing content. It will not be erased or overwritten.".into());
    }
    let mut expected=HashMap::new();
    for e in old {
        let path=e["RelativePath"].as_str().ok_or("Malformed receipt path")?;
        expected.insert(path.to_ascii_lowercase(),e);
    }
    for entry in current {
        let path=entry["RelativePath"].as_str().ok_or("Malformed target path")?;
        let valid=expected.get(&path.to_ascii_lowercase()).is_some_and(|old| {
            old["IsDirectory"]==entry["IsDirectory"]&&old["Length"]==entry["Length"]
        });
        if !valid{
            return Err("Drive contains unexpected content. It will not be erased or overwritten.".into());
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
    #[test]
    fn unexpected_client_content_blocks_existing_media_reuse(){
        let expected=json!({"RelativePath":"EFI\\BOOT\\BOOTX64.EFI",
            "IsDirectory":false,"Length":300});
        let base=json!({"CurrentEntries":[expected.clone()],
            "Receipt":{"TargetManifest":{"Entries":[expected.clone()]}}});
        assert_eq!(policy(&base).unwrap(),true);
        let mut unknown=base.clone();
        unknown["CurrentEntries"][0]["RelativePath"]=json!("CLIENT_DATA.DOCX");
        assert!(policy(&unknown).is_err());
        let mut length=base;
        length["CurrentEntries"][0]["Length"]=json!(12);
        assert!(policy(&length).is_err());
        assert_eq!(policy(&json!({"CurrentEntries":[]})).unwrap(),false);
    }
}
