// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Rust policy port; the existing PowerShell production code stays intact.
use serde_json::{json, Value};
use std::path::Path;

fn policy(input: &Value) -> Result<Value, String> {
    let source = input["SourcePath"].as_str().ok_or("Missing source")?;
    let app = input["ApplicationPath"]
        .as_str()
        .ok_or("Missing automation root")?;
    let cache = input["CacheDirectory"].as_str();
    let normalized = |path: &str| -> Result<String, String> {
        let resolved = std::fs::canonicalize(Path::new(path)).map_err(|e| e.to_string())?;
        let mut text = resolved.to_string_lossy().replace('/', "\\").to_lowercase();
        if !text.ends_with('\\') {
            text.push('\\');
        }
        Ok(text)
    };
    let source = normalized(source)?;
    let app = normalized(app)?;
    let owned = if let Some(cache) = cache {
        source.starts_with(&normalized(cache)?)
    } else {
        false
    };
    if app.starts_with(&source) || (source.starts_with(&app) && !owned) {
        return Err(
            "Keep the official MRI source folder separate from the automation folder.".into(),
        );
    }
    Ok(json!(true))
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
    fn blocks_nested_app_and_nonowned_source() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("app");
        let source = tmp.path().join("source");
        std::fs::create_dir_all(app.join("cache").join("extract")).unwrap();
        std::fs::create_dir(&source).unwrap();
        assert!(policy(&json!({"SourcePath":source,"ApplicationPath":app})).is_ok());
        assert!(policy(&json!({"SourcePath":app,"ApplicationPath":app.join("cache")})).is_err());
        assert!(
            policy(&json!({"SourcePath":app.join("cache").join("extract"),
            "ApplicationPath":app}))
            .is_err()
        );
        assert!(
            policy(&json!({"SourcePath":app.join("cache").join("extract"),
            "ApplicationPath":app,"CacheDirectory":app.join("cache")}))
            .is_ok()
        );
    }
}
