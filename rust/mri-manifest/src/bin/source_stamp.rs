// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Rust policy port; the existing PowerShell production code stays intact.
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// Convert a UTC file timestamp into .NET DateTime ticks (100ns since year 1).
/// A Windows file may legitimately be stamped before the Unix epoch.
fn dotnet_ticks(time: SystemTime) -> i128 {
    const UNIX_EPOCH_TICKS: i128 = 621_355_968_000_000_000;
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            UNIX_EPOCH_TICKS
                + i128::from(duration.as_secs()) * 10_000_000
                + i128::from(duration.subsec_nanos() / 100)
        }
        Err(error) => {
            let duration = error.duration();
            UNIX_EPOCH_TICKS
                - i128::from(duration.as_secs()) * 10_000_000
                - i128::from(duration.subsec_nanos() / 100)
        }
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<Option<&'a Value>, String> {
    let object = value.as_object().ok_or("Expected a JSON object")?;
    // PowerShell PSCustomObject property lookup is case-insensitive.
    let mut matches = object
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name));
    let result = matches.next().map(|(_, value)| value);
    if matches.next().is_some() {
        return Err(format!("Duplicate case-insensitive {name} property"));
    }
    Ok(result)
}

fn policy(input: &Value) -> Result<Value, String> {
    let Some(stamp) = field(input, "InputStamp")?.filter(|v| !v.is_null()) else {
        return Ok(json!(true));
    };
    let path = field(stamp, "Path")?
        .and_then(Value::as_str)
        .ok_or("Missing stamped source path")?;
    let length = field(stamp, "Length")?
        .and_then(Value::as_u64)
        .ok_or("Missing stamped byte length")?;
    let ticks = field(stamp, "WriteTicks")?
        .and_then(Value::as_i64)
        .ok_or("Missing stamped write ticks")?;
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let updated = meta.modified().map_err(|e| e.to_string())?;
    let actual = dotnet_ticks(updated);
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
    fn supports_dotnet_file_timestamps_before_unix_epoch() {
        use std::time::Duration;

        assert_eq!(dotnet_ticks(UNIX_EPOCH), 621_355_968_000_000_000);
        assert_eq!(
            dotnet_ticks(UNIX_EPOCH - Duration::from_secs(1)),
            621_355_967_990_000_000
        );
        assert_eq!(
            dotnet_ticks(UNIX_EPOCH + Duration::from_millis(1500)),
            621_355_968_015_000_000
        );
        assert_eq!(
            dotnet_ticks(UNIX_EPOCH - Duration::from_millis(1500)),
            621_355_967_985_000_000
        );
    }

    #[test]
    fn stamped_identity_detects_changed_size_and_time() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.zip");
        std::fs::write(&path, b"original").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let ticks = dotnet_ticks(meta.modified().unwrap());
        let fixture = json!({"InputStamp":{"Path":path.to_string_lossy(),
            "Length":meta.len(),"WriteTicks":ticks as i64}});
        assert!(policy(&fixture).is_ok());
        let folded = json!({"inputstamp": {"PATH":path.to_string_lossy(),
            "length":meta.len(),"writeticks":ticks as i64}});
        assert!(policy(&folded).is_ok());
        let ambiguous = json!({"InputStamp": fixture["InputStamp"], "inputstamp": fixture["InputStamp"]});
        assert!(policy(&ambiguous).is_err());
        std::fs::write(&path, b"changed size").unwrap();
        assert!(policy(&fixture).is_err());
        assert!(policy(&json!({})).is_ok());
    }
}
