// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Rust policy port; the existing PowerShell production code stays intact.
use serde_json::{json, Value};
use sha2::{Digest,Sha256};

fn policy(input: &Value) -> Result<Value, String> {
    let path=input["Path"].as_str().ok_or("Missing runtime JSON path")?;
    let expected=input["ExpectedHash"].as_str().ok_or("Missing expected data hash")?;
    if expected.len()!=64 || !expected.bytes().all(|b|b.is_ascii_hexdigit()){
        return Err("Missing or invalid expected data hash.".into());
    }
    let bytes=std::fs::read(path).map_err(|e|e.to_string())?;
    // .NET File.ReadAllText consumes only one leading UTF-8 BOM.
    // Removing multiple BOMs would authenticate altered bytes incorrectly.
    let decoded = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let text = decoded.strip_prefix('\u{feff}').unwrap_or(decoded);
    let current=format!("{:X}",Sha256::digest(text.as_bytes()));
    if !current.eq_ignore_ascii_case(expected){
        return Err("Runtime data changed or is corrupt. No copy or preparation is allowed.".into());
    }
    serde_json::from_str(text).map_err(|e|e.to_string())
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
    fn validates_before_parsing_and_rejects_modifications() {
        let temp=tempfile::tempdir().unwrap();
        let path=temp.path().join("run.json");
        let data=r#"{"Stage":"READY","Number":4}"#;
        std::fs::write(&path,data).unwrap();
        let sha=format!("{:X}",Sha256::digest(data.as_bytes()));
        let fixture=json!({"Path":path.to_string_lossy(),"ExpectedHash":sha});
        assert_eq!(policy(&fixture).unwrap()["Stage"],"READY");
        // One UTF-8 BOM is ignored by File.ReadAllText, but a second BOM
        // must never be silently removed before hash verification.
        std::fs::write(&path,format!("\u{feff}{data}")).unwrap();
        assert_eq!(policy(&fixture).unwrap()["Stage"],"READY");
        std::fs::write(&path,format!("\u{feff}\u{feff}{data}")).unwrap();
        assert!(policy(&fixture).is_err());
        std::fs::write(&path,data.replace("READY","FAILED")).unwrap();
        assert!(policy(&fixture).is_err());
        assert!(policy(&json!({"Path":path,"ExpectedHash":"0"})).is_err());
    }
}
