// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data MRI policy migration; requires independent PowerShell acceptance.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let names = [
        "SchemaVersion",
        "SourcePath",
        "SetupBatch",
        "SetupArguments",
        "SetupInteractive",
        "SetupTimeoutSeconds",
        "CopyTimeoutSeconds",
        "PrepareDisk",
        "FileSystem",
        "RequiredFiles",
        "MutableFiles",
        "MaxParallel",
        "PollSeconds",
        "StablePolls",
        "MinimumDiskBytes",
        "MaximumDiskBytes",
        "SerializeSetup",
    ];
    for field in names {
        if input.get(field).is_none() {
            return Err(format!("Missing setting: {field}"));
        }
    }
    if input["SchemaVersion"] != 1 {
        return Err("Unsupported configuration version.".into());
    }
    if input["SourcePath"]
        .as_str()
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err("Select the official MRI ISO, ZIP, or extracted folder first.".into());
    }
    if input["FileSystem"] != "FAT32" {
        return Err("This media recipe supports FAT32 only.".into());
    }
    for field in ["SetupInteractive", "PrepareDisk", "SerializeSetup"] {
        if !input[field].is_boolean() {
            return Err(format!("{field} must be true or false."));
        }
    }
    for field in [
        "MaxParallel",
        "PollSeconds",
        "StablePolls",
        "MinimumDiskBytes",
        "MaximumDiskBytes",
        "SetupTimeoutSeconds",
        "CopyTimeoutSeconds",
    ] {
        if input[field].as_i64().is_none() {
            return Err(format!("{field} must be an integer."));
        }
    }
    let parallel = input["MaxParallel"].as_i64().unwrap();
    if !(1..=30).contains(&parallel) {
        return Err("MaxParallel must be 1 through 30.".into());
    }
    let poll = input["PollSeconds"].as_i64().unwrap();
    let stable = input["StablePolls"].as_i64().unwrap();
    if !(1..=30).contains(&poll) || !(2..=30).contains(&stable) {
        return Err("Use PollSeconds 1-30 and StablePolls 2-30.".into());
    }
    if input["MinimumDiskBytes"].as_i64().unwrap() < 1
        || input["MaximumDiskBytes"].as_i64().unwrap() < input["MinimumDiskBytes"].as_i64().unwrap()
    {
        return Err("Invalid capacity limits.".into());
    }
    if input["SetupTimeoutSeconds"].as_i64().unwrap() < 1
        || input["CopyTimeoutSeconds"].as_i64().unwrap() < 1
    {
        return Err("Timeouts must be positive.".into());
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
    fn base() -> Value {
        json!({"SchemaVersion":1,"SourcePath":"C:\\MRI",
        "SetupBatch":"","SetupArguments":[],"SetupInteractive":false,
        "SetupTimeoutSeconds":600,"CopyTimeoutSeconds":1200,"PrepareDisk":true,
        "FileSystem":"FAT32","RequiredFiles":["bootmgr"],"MutableFiles":[],
        "MaxParallel":8,"PollSeconds":2,"StablePolls":3,
        "MinimumDiskBytes":8000000000_i64,"MaximumDiskBytes":64000000000_i64,
        "SerializeSetup":false})
    }
    #[test]
    fn type_limits_are_enforced() {
        assert!(policy(&base()).is_ok());
        for (field, value) in [
            ("MaxParallel", json!(31)),
            ("PollSeconds", json!(0)),
            ("StablePolls", json!(1)),
            ("FileSystem", json!("exFAT")),
            ("SerializeSetup", json!("yes")),
            ("MinimumDiskBytes", json!(0)),
            ("CopyTimeoutSeconds", json!(0)),
        ] {
            let mut v = base();
            v[field] = value;
            assert!(policy(&v).is_err(), "should reject {field}");
        }
    }
}
