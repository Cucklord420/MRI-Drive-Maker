// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Rust policy port; the existing PowerShell production code stays intact.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let letter = input["Letter"].as_str().ok_or("Missing target letter")?;
    let number = input["Number"].as_i64().ok_or("Missing disk number")?;
    let label = input["Label"].as_str().ok_or("Missing record label")?;
    let setup = input["SetupBatch"].as_str().ok_or("Missing SetupBatch")?;
    if letter.len() != 1 || !letter.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err("Missing valid target drive letter".into());
    }
    let folder = setup.replace('/', "\\");
    if folder.trim().is_empty() || folder.starts_with('\\') || folder.contains(':')
        || folder.chars().any(|c| c.is_control() || "*?\"<>|".contains(c)) {
        return Err(format!("A relative file path is required: {setup}"));
    }
    for part in folder.split('\\') {
        if part.is_empty() || part == "." || part == ".."
            || part.ends_with(' ') || part.ends_with('.') {
            return Err(format!("Unsafe relative file path: {setup}"));
        }
        let name = part.split('.').next().unwrap_or("").to_uppercase();
        let reserved = matches!(name.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
            || ["COM", "LPT"].iter().any(|prefix| {
                name.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
                })
            });
        if reserved {
            return Err(format!("Windows device names are not allowed: {setup}"));
        }
    }
    let args = input["SetupArguments"]
        .as_array()
        .ok_or("Missing setup arguments")?;
    let mut expanded = Vec::new();
    for arg in args {
        let raw = arg.as_str().ok_or("Batch argument must be a string")?;
        let value = raw
            .replace("{DriveLetter}", letter)
            .replace("{DriveRoot}", &format!("{letter}:\\"))
            .replace("{DiskNumber}", &number.to_string())
            .replace("{Label}", label);
        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_ .:/\\=+,-".contains(c))
        {
            return Err("Unsafe expanded batch argument.".into());
        }
        expanded.push(format!("\"{value}\""));
    }
    let cmd = format!(
        "/d /s /c \"\"{letter}:\\{folder}\"{}\"",
        if expanded.is_empty() {
            String::new()
        } else {
            format!(" {}", expanded.join(" "))
        }
    );
    Ok(json!(cmd))
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
    fn expands_original_placeholders_and_quotes() {
        let result = policy(&json!({"Letter":"E","Number":7,"Label":"MRI_07",
            "SetupBatch":"tools/setup.cmd",
            "SetupArguments":["--disk={DiskNumber}","--root={DriveRoot}","--label={Label}"]}))
        .unwrap();
        let actual = result.as_str().unwrap();
        assert!(actual.contains("E:\\tools\\setup.cmd"));
        assert!(actual.contains("\"--disk=7\""));
        assert!(actual.contains("\"--root=E:\\\""));
        assert!(actual.contains("\"--label=MRI_07\""));
    }
    #[test]
    fn refuses_cmd_injection_after_expansion() {
        for arg in ["&& erase", "{Label}&", "%PATH%", "| whoami"] {
            assert!(policy(&json!({"Letter":"E","Number":1,"Label":"MRI_01",
            "SetupBatch":"setup.cmd","SetupArguments":[arg]}))
            .is_err());
        }
    }
    #[test]
    fn rejects_original_unsafe_windows_setup_components() {
        for setup in [
            "CON.cmd", "AUX.bat", "folder/COM9.cmd", "LPT¹.cmd",
            "folder/setup. ", "folder/bad.", "folder//setup.cmd",
            "folder/./setup.cmd", "folder/../setup.cmd"
        ] {
            let fixture = json!({"Letter":"E","Number":1,"Label":"MRI_01",
                "SetupBatch":setup,"SetupArguments":[]});
            assert!(policy(&fixture).is_err(), "accepted original-unsafe path: {setup}");
        }
        assert!(policy(&json!({"Letter":"E","Number":1,"Label":"MRI_01",
            "SetupBatch":"tools/setup.cmd","SetupArguments":[]})).is_ok());
    }

}
