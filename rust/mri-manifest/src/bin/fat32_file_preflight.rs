// SPDX-License-Identifier: GPL-3.0-only
//! Read-only port of a named MRI.Batch.psm1 policy. No physical USB operations.
use serde_json::{json, Value};

fn policy(input: &Value) -> Result<Value, String> {
    let files=input.get("Entries").and_then(Value::as_array).ok_or("Missing Entries")?;
    let mut seen=0usize;
    for file in files {
        if file["IsDirectory"]==true {continue;}
        let len=file["Length"].as_u64().ok_or("Invalid file length")?;
        if len>4_294_967_295 {return Err("A source file exceeds the FAT32 file-size limit.".into());}
        seen+=1;
    }
    if seen==0 {return Err("MRI source contains no files.".into());}
    let required=input.get("RequiredFiles").and_then(Value::as_array)
        .ok_or("Missing required files list")?;
    for name in required {
        let wanted=name.as_str().ok_or("Required file name is invalid")?;
        if !files.iter().any(|f|f["IsDirectory"]==false
            && f["RelativePath"].as_str().is_some_and(|p|p.eq_ignore_ascii_case(wanted))
            && f["Length"].as_u64().is_some_and(|x|x>0)) {
            return Err(format!("The source does not contain the configured required file: {wanted}"));
        }
    }
    Ok(json!({"Files":seen}))
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
    fn boundary_sizes_and_required_paths() {
        let base=json!({"Entries":[{"RelativePath":"bootmgr","IsDirectory":false,"Length":4294967295}],
            "RequiredFiles":["BOOTMGR"]});
        assert!(policy(&base).is_ok());
        let mut over=base.clone();
        over["Entries"][0]["Length"]=json!(4294967296_u64);
        assert!(policy(&over).unwrap_err().contains("FAT32"));
        let mut missing=base.clone();
        missing["RequiredFiles"]=json!(["EFI/BOOT/BOOTX64.EFI"]);
        assert!(policy(&missing).is_err());
        assert!(policy(&json!({"Entries":[],"RequiredFiles":[]})).is_err());
    }
}
