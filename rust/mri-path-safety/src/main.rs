// SPDX-License-Identifier: GPL-3.0-only
//! Pure read-only Rust port of ConvertTo-MRIRelativePath in MRI.Batch.psm1.
//! No physical storage/device APIs, no file or directory creation.
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{env, fs};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Input { path: String }
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Output { value: Option<String>, error: Option<String> }

fn normalize(path: &str) -> Result<String, String> {
    let converted = path.replace('/', "\\");
    let invalid = Regex::new(r#"^[A-Za-z]:|[\x00-\x1f:*?"<>|]"#).unwrap();
    if converted.trim().is_empty() || converted.starts_with('\\') || invalid.is_match(&converted) {
        return Err(format!("A relative file path is required: {path}"));
    }
    for part in converted.split('\\') {
        if part.is_empty() || part == "." || part == ".."
            || part.ends_with(' ') || part.ends_with('.') {
            return Err(format!("Unsafe relative file path: {path}"));
        }
    }
    let device = Regex::new(r"(?i)^(?:CON|PRN|AUX|NUL|CLOCK\$|COM[1-9¹²³]|LPT[1-9¹²³])(?:\.|$)").unwrap();
    for part in converted.split('\\') {
        if device.is_match(part) {
            return Err(format!("Windows device names are not allowed: {path}"));
        }
    }
    Ok(converted)
}

fn main() {
    let args:Vec<String>=env::args().collect();
    if args.len()!=3 || args[1]!="--input-file" {
        eprintln!("Usage: mri-path-safety --input-file UTF8_JSON");
        std::process::exit(2);
    }
    let raw=fs::read_to_string(&args[2]).unwrap_or_else(|_|{
        eprintln!("Unable to read UTF-8 path fixture"); std::process::exit(2);
    });
    let input:Input=serde_json::from_str(&raw).unwrap_or_else(|_|{
        eprintln!("Invalid JSON path fixture"); std::process::exit(2);
    });
    let response=match normalize(&input.path) {
        Ok(value)=>Output{value:Some(value),error:None},
        Err(error)=>Output{value:None,error:Some(error)}
    };
    let code=if response.error.is_some(){1}else{0};
    println!("{}",serde_json::to_string(&response).expect("output JSON"));
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separator_normalization_is_idempotent() {
        for (input,want) in [
            ("Setup.cmd","Setup.cmd"),
            ("nested/folder/file.txt","nested\\folder\\file.txt"),
            ("mixed\\folder/file.dat","mixed\\folder\\file.dat"),
            ("file with spaces.txt","file with spaces.txt"),
        ] {
            assert_eq!(normalize(input).unwrap(),want);
            assert_eq!(normalize(want).unwrap(),want);
        }
    }

    #[test]
    fn prohibited_input_and_windows_device_names_are_rejected() {
        for input in [
            "", "   ", "/root", "\\absolute", "C:\\Windows\\system32",
            "C:relative", "bad:name", "a|b", "a*b", "a?b", "x\"y",
            "folder/../file", "folder/./file", "folder//file",
            "folder\\", ".","..","file.", "name ", "nested/file.",
            "CON","con.txt","NUL.log","PRN","Aux","CLOCK$","COM1",
            "LPT9","COM²","LPT³.txt","file\nnewline",
        ] {
            assert!(normalize(input).is_err(),"unexpectedly accepted {input:?}");
        }
    }

    #[test]
    fn permitted_non_device_names_remain_valid() {
        for input in [
            "CONTEXT.txt","NULLED","COM10","LPT10.txt","nested/CONTEXT/file",
            "normal.data","IMG_000001","boot/efi/bootx64.efi"
        ] {
            assert!(normalize(input).is_ok(),"rejected benign filename {input}");
        }
    }

    #[test]
    fn two_hundred_mutated_paths_require_consistent_validation() {
        for n in 0..200 {
            let prefix=format!("folder{n}/sub{n}");
            let good=format!("{prefix}/file{n}.bin");
            assert!(normalize(&good).is_ok());
            let bad=format!("{prefix}/../file{n}.bin");
            assert!(normalize(&bad).is_err());
            let device=format!("{prefix}/com1.txt");
            assert!(normalize(&device).is_err());
        }
    }
}
