// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let root = input
        .get("SystemRoot")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .ok_or("Missing SystemRoot")?;
    let name = input
        .get("Name")
        .and_then(Value::as_str)
        .ok_or("Missing executable name")?;
    // The production script resolves both cmd.exe and the nested Windows PowerShell binary.
    // Validate components rather than rejecting legitimate relative subdirectories.
    let relative = name.replace('/', "\\");
    if relative.is_empty() || relative.starts_with('\\') || relative.contains(':') {
        return Err("Executable must be a safe relative path below System32".into());
    }
    let mut resolved = std::path::PathBuf::from(root).join("System32");
    for part in relative.split('\\') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(' ')
            || part.ends_with('.')
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        {
            return Err("Executable must be a safe relative path below System32".into());
        }
        // Match the Windows reserved-device protections used elsewhere in the
        // original MRI path validator; never resolve a device alias as a tool.
        let stem = part.split('.').next().unwrap_or("").to_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
                })
            });
        if reserved {
            return Err("Executable must be a safe relative path below System32".into());
        }
        resolved.push(part);
    }
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
    fn resolves_both_real_production_executables() {
        for name in ["cmd.exe", "WindowsPowerShell\\v1.0\\powershell.exe"] {
            let path = policy(&json!({"SystemRoot":"C:\\Windows","Name":name}))
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            assert!(path.contains("System32"), "{path}");
            assert!(
                path.ends_with(name.replace('\\', std::path::MAIN_SEPARATOR_STR).as_str()),
                "{path}"
            );
        }
    }

    #[test]
    fn rejects_root_escapes_and_invalid_names() {
        for name in [
            "../bad.exe",
            "..\\bad.exe",
            "nested\\..\\bad.exe",
            "D:evil",
            "/bad.exe",
            "\\\\server\\share\\bad.exe",
            "",
            ".",
            "..",
            "dir\\\\bad.exe",
            "dir\\trailing. ",
            "dir\\pipe|bad.exe",
            "dir\\bad?.exe",
            "CON.exe", "AUX.cmd", "LPT1.exe", "COM¹.exe", "folder/NUL.bat",
        ] {
            assert!(
                policy(&json!({"SystemRoot":"C:\\Windows","Name":name})).is_err(),
                "accepted unsafe name: {name}"
            );
        }
        assert!(policy(&json!({"SystemRoot":"","Name":"cmd.exe"})).is_err());
    }
}
