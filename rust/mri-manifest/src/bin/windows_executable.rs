// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let root = input
        .get("SystemRoot")
        .and_then(Value::as_str)
        .ok_or("Missing SystemRoot")?;
    let name = input
        .get("Name")
        .and_then(Value::as_str)
        .ok_or("Missing executable name")?;
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == "."
        || name == ".."
        || name.contains(':')
    {
        return Err("Executable must be a single filename".into());
    }
    let resolved = std::path::Path::new(root).join("System32").join(name);
    Ok(json!(resolved.to_string_lossy()))
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
    fn standard_system32_programs_resolve_in_platform_paths() {
        let result = policy(&json!({"SystemRoot":"C:\\Windows","Name":"cmd.exe"})).unwrap();
        assert!(result.as_str().unwrap().contains("System32"));
        assert!(result.as_str().unwrap().ends_with("cmd.exe"));
    }
    #[test]
    fn malicious_relative_names_do_not_escape_system32() {
        for name in ["../bad.exe", "..\\bad.exe", "D:evil", "/bad.exe", ""] {
            assert!(policy(&json!({"SystemRoot":"C:\\Windows","Name":name})).is_err());
        }
    }
}
