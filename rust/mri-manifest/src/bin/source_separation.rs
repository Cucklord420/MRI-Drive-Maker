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
    // The original PowerShell GetFullPath does not require the cache path to
    // exist. Missing/unreadable cache cannot authorize an ownership exemption.
    let owned = cache
        .and_then(|directory| normalized(directory).ok())
        .is_some_and(|directory| source.starts_with(&directory));
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
        let missing_cache = tmp.path().join("missing-cache");
        // Missing cache metadata cannot cause a valid, separate source to fail.
        assert!(policy(&json!({"SourcePath":source,
            "ApplicationPath":app,"CacheDirectory":missing_cache}))
        .is_ok());
        // A missing cache also cannot authorize a nested automation source.
        assert!(
            policy(&json!({"SourcePath":app.join("cache").join("extract"),
            "ApplicationPath":app,"CacheDirectory":missing_cache}))
            .is_err()
        );
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

    #[cfg(windows)]
    #[test]
    fn matches_original_powershell_on_synthetic_existing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("automation");
        let cache = app.join("Data").join("Sources");
        let cache_source = cache.join("unpacked");
        let outside = temp.path().join("official MRI source");
        let unowned = app.join("unowned");
        std::fs::create_dir_all(&cache_source).unwrap();
        std::fs::create_dir_all(&unowned).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../MRI.Batch.psm1");
        assert!(module.is_file());
        let script = r#"
$ErrorActionPreference = 'Stop'
try {
    Import-Module -Name $env:MRI_BATCH_MODULE -Force -DisableNameChecking -ErrorAction Stop | Out-Null
    $context = [pscustomobject]@{
        Path = $env:MRI_SOURCE
        CacheDirectory = $env:MRI_CACHE
    }
    Assert-MRISourceSeparation -Context $context -ApplicationPath $env:MRI_APP
    exit 0
} catch {
    [Console]::Error.Write($_.Exception.Message)
    exit 1
}
"#;
        let cases = [
            (&outside, &app, None),
            (&unowned, &app, None),
            (&cache_source, &app, None),
            (&cache_source, &app, Some(cache.as_path())),
            (&cache_source, &app, Some(outside.as_path())),
            (&app, &app, None),
            (&app, &unowned, None),
        ];
        for (source, app_root, cache_root) in cases {
            let cache_text = cache_root.map_or("", |path| path.to_str().unwrap());
            let rust = policy(&json!({
                "SourcePath": source,
                "ApplicationPath": app_root,
                "CacheDirectory": cache_text
            }));
            let output = std::process::Command::new("powershell.exe")
                .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
                .env("MRI_BATCH_MODULE", &module)
                .env("MRI_SOURCE", source)
                .env("MRI_APP", app_root)
                .env("MRI_CACHE", cache_text)
                .output()
                .expect("Windows PowerShell 5.1 is required for source-separation parity");
            assert_eq!(
                rust.is_ok(),
                output.status.success(),
                "source={source:?} app={app_root:?} cache={cache_root:?} Rust={rust:?}, PowerShell exit={:?}, stderr={:?}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
