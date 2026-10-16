// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let Some(kind) = input.get("SourceKind") else {
        return Ok(json!("Folder"));
    };
    let kind = kind
        .as_str()
        .ok_or("SourceKind must be Folder, ISO, or ZIP.")?;
    if ["folder", "iso", "zip"].contains(&kind.to_ascii_lowercase().as_str()) {
        Ok(json!(kind))
    } else {
        Err("SourceKind must be Folder, ISO, or ZIP.".into())
    }
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
    fn source_kind_defaults_and_acceptance() {
        assert_eq!(policy(&json!({})).unwrap(), "Folder");
        for value in ["Folder", "ISO", "ZIP", "zip", "folder", "isO"] {
            assert_eq!(policy(&json!({"SourceKind":value})).unwrap(), value);
        }
    }
    #[test]
    fn source_kind_fails_closed() {
        for value in [json!(null), json!(1), json!("BAD"), json!(""), json!(false)] {
            assert!(policy(&json!({"SourceKind":value})).is_err());
        }
    }
}
