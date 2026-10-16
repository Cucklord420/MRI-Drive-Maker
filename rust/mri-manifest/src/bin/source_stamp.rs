// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Rust policy port; the existing PowerShell production code stays intact.
use serde_json::{json, Value};
use std::time::UNIX_EPOCH;

fn policy(input: &Value) -> Result<Value, String> {
    let Some(stamp) = input.get("InputStamp").filter(|v| !v.is_null()) else {
        return Ok(json!(true));
    };
    let path = stamp["Path"]
        .as_str()
        .ok_or("Missing stamped source path")?;
    let length = stamp["Length"]
        .as_u64()
        .ok_or("Missing stamped byte length")?;
    let ticks = stamp["WriteTicks"]
        .as_i64()
        .ok_or("Missing stamped write ticks")?;
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let updated = meta.modified().map_err(|e| e.to_string())?;
    let diff = updated
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    const EPOCH_TICKS: i128 = 621355968000000000;
    let actual = EPOCH_TICKS
        + (i128::from(diff.as_secs()) * 10_000_000)
        + i128::from(diff.subsec_nanos() / 100);
    if meta.len() != length || actual != i128::from(ticks) {
        Err("The input ISO/ZIP changed during this session. Restart the watcher.".into())
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
        eprintln!("Cannot read local fixture: {error}");
        std::process::exit(2);
    });
    let input: Value = serde_json::from_slice(&data).unwrap_or_else(|error| {
        eprintln!("Invalid JSON fixture: {error}");
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
    fn stamped_identity_detects_changed_size_and_time() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.zip");
        std::fs::write(&path, b"original").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let diff = meta.modified().unwrap().duration_since(UNIX_EPOCH).unwrap();
        let ticks = 621355968000000000_i128
            + i128::from(diff.as_secs()) * 10000000
            + i128::from(diff.subsec_nanos() / 100);
        let fixture = json!({"InputStamp":{"Path":path.to_string_lossy(),
            "Length":meta.len(),"WriteTicks":ticks as i64}});
        assert!(policy(&fixture).is_ok());
        std::fs::write(&path, b"changed size").unwrap();
        assert!(policy(&fixture).is_err());
        assert!(policy(&json!({})).is_ok());
    }
}
