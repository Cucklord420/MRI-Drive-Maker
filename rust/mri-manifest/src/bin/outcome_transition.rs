// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let mut ready = input.get("Ready").and_then(Value::as_object).cloned()
        .ok_or("Missing Ready map")?;
    let mut failed = input.get("Failed").and_then(Value::as_object).cloned()
        .ok_or("Missing Failed map")?;
    let key = input.get("Key").and_then(Value::as_str).ok_or("Missing drive key")?;
    let label = input.get("Label").and_then(Value::as_str).ok_or("Missing label")?;
    let success = input.get("Success").and_then(Value::as_bool).ok_or("Missing Success flag")?;
    let drop_key = |map:&mut serde_json::Map<String,Value>| {
        let existing = map.keys().filter(|entry|entry.eq_ignore_ascii_case(key))
            .cloned().collect::<Vec<_>>();
        for name in existing { map.remove(&name); }
    };
    drop_key(&mut ready);
    drop_key(&mut failed);
    if success { ready.insert(key.into(),json!(label)); }
    else { failed.insert(key.into(),json!(label)); }
    Ok(json!({"Ready":ready,"Failed":failed}))
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
    fn ready_and_failed_cannot_coexist() {
        let input=json!({"Ready":{"disk-a":"old"},"Failed":{"DISK-A":"x"},
            "Key":"Disk-A","Label":"MRI_01","Success":true});
        let result=policy(&input).unwrap();
        assert_eq!(result["Ready"]["Disk-A"],"MRI_01");
        assert!(result["Failed"].as_object().unwrap().is_empty());
        let fail=policy(&json!({"Ready":result["Ready"],"Failed":result["Failed"],
            "Key":"disk-a","Label":"MRI_01","Success":false})).unwrap();
        assert!(fail["Ready"].as_object().unwrap().is_empty());
        assert_eq!(fail["Failed"]["disk-a"],"MRI_01");
    }
}
