// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data MRI policy migration; requires independent PowerShell acceptance.
use serde_json::{json, Value};
use sha2::{Digest,Sha256};
use std::collections::HashSet;

fn policy(input: &Value) -> Result<Value, String> {
    let records=input["Records"].as_array().ok_or("Missing Records array")?;
    let mut keys=HashSet::new();
    let mut serials=HashSet::new();
    let mut labels=HashSet::new();
    for record in records {
        for field in ["Key","Serial","PnpId","Model","Size","Label","AllowErase"] {
            if record.get(field).is_none(){return Err(format!("Drive registry is missing {field}."));}
        }
        let serial=record["Serial"].as_str().ok_or("Bad serial")?.trim();
        let pnp=record["PnpId"].as_str().ok_or("Bad PnpId")?.trim();
        let model=record["Model"].as_str().ok_or("Bad model")?.trim();
        let size=record["Size"].as_i64().ok_or("Bad size")?;
        if serial.is_empty()||serial.chars().all(|c|c=='0')
            ||["UNKNOWN","N/A","NONE"].contains(&serial.to_ascii_uppercase().as_str()){
            return Err("This drive does not report a usable hardware serial number.".into());
        }
        if pnp.is_empty()||model.is_empty(){
            return Err("Incomplete USB hardware identity.".into());
        }
        let identity=format!("{}\n{}\n{}\n{}",serial.to_uppercase(),
            pnp.to_uppercase(),model.to_uppercase(),size);
        let key=format!("{:X}",Sha256::digest(identity.as_bytes()));
        if record["Key"].as_str()!=Some(key.as_str()){
            return Err("Drive registry identity is invalid.".into());
        }
        let label=record["Label"].as_str().ok_or("Missing label")?;
        let digits=label.strip_prefix("MRI_").unwrap_or("");
        if !(digits.len()==2||digits.len()==3)
            ||!digits.bytes().all(|b|b.is_ascii_digit())
            ||!record["AllowErase"].is_boolean(){
            return Err("Invalid registered drive label or erase setting.".into());
        }
        let reg_key=record["Key"].as_str().unwrap().to_ascii_lowercase();
        if !keys.insert(reg_key)
            ||!serials.insert(serial.to_ascii_lowercase())
            ||!labels.insert(label.to_ascii_lowercase()){
            return Err("Duplicate identity or label in the drive registry.".into());
        }
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
    #[test]
    fn verifies_hardware_identity_and_duplicate_labels() {
        let mut record=json!({"Serial":"ABC","PnpId":"USB\\VID","Model":"TEST",
            "Size":16000000000_i64,"Label":"MRI_01","AllowErase":true});
        let identity="ABC\nUSB\\VID\nTEST\n16000000000";
        record["Key"]=json!(format!("{:X}",Sha256::digest(identity.as_bytes())));
        assert_eq!(policy(&json!({"Records":[record.clone()]})).unwrap(),true);
        assert!(policy(&json!({"Records":[record.clone(),record.clone()]})).is_err());
        record["Label"]=json!("MRI_AA");
        assert!(policy(&json!({"Records":[record]})).is_err());
    }
}
