// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let value = input
        .get("CancelPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !value.is_empty() && std::path::Path::new(value).is_file() {
        Err("Build cancelled.".into())
    } else {
        Ok(json!(true))
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
    fn missing_or_empty_cancel_marker_allows_continue() {
        assert_eq!(policy(&json!({"CancelPath":""})).unwrap(), true);
        assert_eq!(
            policy(&json!({"CancelPath":"__not_a_real_path__"})).unwrap(),
            true
        );
    }
    #[test]
    fn existing_cancel_marker_stops_work() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cancel");
        std::fs::write(&path, b"cancelled").unwrap();
        let input = json!({"CancelPath":path.to_string_lossy()});
        assert_eq!(policy(&input).unwrap_err(), "Build cancelled.");
    }
}
