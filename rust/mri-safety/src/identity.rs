// SPDX-License-Identifier: GPL-3.0-only
// Hardware identity policy only: cannot enumerate, write, or erase real disks.

use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct HardwareIdentity {
    serial: String,
    pnp_id: String,
    model: String,
    size: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct IdentityResponse {
    key: Option<String>,
    pub error: Option<String>,
}

pub fn key(serial:&str,pnp_id:&str,model:&str,size:i64)->Result<String,String>{
    let serial=serial.trim();
    if serial.is_empty() || serial.chars().all(|c|c=='0') ||
        ["UNKNOWN","N/A","NONE"].iter().any(|blocked|serial.eq_ignore_ascii_case(blocked)){
        return Err("This drive does not report a usable hardware serial number.".into());
    }
    if pnp_id.trim().is_empty() || model.trim().is_empty(){
        return Err("Incomplete USB hardware identity.".into());
    }
    // Windows Get-Disk reports positive capacities. Reject malformed Rust
    // input rather than allowing a nonphysical capacity to define an identity.
    // This is intentionally stricter than the original PowerShell hash helper.
    if size <= 0 {
        return Err("Disk size must be positive.".into());
    }
    let payload=format!("{}\n{}\n{}\n{}",serial.to_uppercase(),
        pnp_id.trim().to_uppercase(),model.trim().to_uppercase(),size);
    Ok(format!("{:X}",Sha256::digest(payload.as_bytes())))
}
pub fn evaluate(record:&HardwareIdentity)->IdentityResponse{
    match key(&record.serial,&record.pnp_id,&record.model,record.size){
        Ok(key)=>IdentityResponse{key:Some(key),error:None},
        Err(error)=>IdentityResponse{key:None,error:Some(error)}
    }
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn rejects_unreliable_serial_identifiers(){
        for serial in [""," ","0","0000","UNKNOWN","Unknown","none","N/A"]{
            assert!(key(serial,"USB\\VID","Some Model",32_000_000_000).is_err());
        }
    }
    #[test]
    fn identity_fails_closed_when_pnp_or_model_missing(){
        assert!(key("A"," ","MODEL",42).is_err());
        assert!(key("A","PNP","",42).is_err());
    }
    #[test]
    fn rejects_nonpositive_disk_sizes_before_hashing(){
        for size in [0,-1,i64::MIN]{
            assert_eq!(key("SERIAL01","USB\\VID_1234","Model",size).unwrap_err(),
                "Disk size must be positive.");
        }
    }
    #[test]
    fn equivalent_case_and_whitespace_are_hash_stable(){
        let expected=key("serial01","pnp01","media",32).unwrap();
        assert_eq!(expected,key("  SERIAL01  "," PNP01 "," MEDIA ",32).unwrap());
        assert_eq!(expected.len(),64);
        assert!(expected.chars().all(|c|c.is_ascii_hexdigit()&&!c.is_ascii_lowercase()));
    }
    #[test]
    fn hundred_different_size_and_serial_fixtures(){
        let mut keys=std::collections::HashSet::new();
        for id in 0..100 {
            let candidate=key(&format!("ID{id:03}"),"USB\\VID_1234","Model",100_000_000+id)
                .unwrap();
            assert!(keys.insert(candidate));
        }
    }
}
