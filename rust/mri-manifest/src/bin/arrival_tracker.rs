// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data MRI policy migration; requires independent PowerShell acceptance.
use serde_json::{json, Value};
use std::collections::HashMap;

fn policy(input: &Value) -> Result<Value, String> {
    let drives=input["Inventory"].as_array().ok_or("Missing Inventory")?;
    let registry=input["Records"].as_array().ok_or("Missing Records")?;
    let minimum=input["StablePolls"].as_i64().ok_or("Missing StablePolls")?;
    if minimum<2{return Err("StablePolls must be at least 2".into());}
    let mut tracker=input["Tracker"].as_object().cloned().unwrap_or_default();
    let mut seen=HashMap::new();
    let mut ready=Vec::new();
    for drive in drives{
        let Some(key)=drive["Key"].as_str().filter(|s|!s.is_empty()) else {continue;};
        seen.insert(key.to_ascii_lowercase(),true);
        let number=drive["Number"].as_i64().ok_or("Missing drive Number")?;
        let mut state=tracker.get(key).cloned().unwrap_or(json!({
            "Number":number,"Stable":0,"Attempted":false
        }));
        if state["Number"]!=number {
            state["Number"]=json!(number);
            state["Stable"]=json!(0);
        }
        if drive["Blocked"]==true {
            state["Stable"]=json!(0);
        }else{
            let stable=state["Stable"].as_i64().ok_or("Bad Stable count")?+1;
            state["Stable"]=json!(stable);
            let matches=registry.iter().filter(|r|r["Key"].as_str()
                .is_some_and(|id|id.eq_ignore_ascii_case(key))).collect::<Vec<_>>();
            if matches.len()==1 && stable>=minimum && state["Attempted"]==false {
                ready.push(matches[0].clone());
            }
        }
        tracker.insert(key.to_owned(),state);
    }
    tracker.retain(|key,_|seen.contains_key(&key.to_ascii_lowercase()));
    Ok(json!({"Tracker":tracker,"Arrivals":ready}))
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
    fn only_stable_registered_unblocked_drive_arrives() {
        let mut input=json!({"Inventory":[{"Key":"disk-x","Number":7,"Blocked":false}],
            "Records":[{"Key":"disk-x","Label":"MRI_01"}],
            "StablePolls":3,"Tracker":{}});
        for count in 1..=3{
            let next=policy(&input).unwrap();
            assert_eq!(next["Arrivals"].as_array().unwrap().len(),if count==3{1}else{0});
            input["Tracker"]=next["Tracker"].clone();
        }
        input["Inventory"][0]["Blocked"]=json!(true);
        let blocked=policy(&input).unwrap();
        assert_eq!(blocked["Tracker"]["disk-x"]["Stable"],0);
        input["Inventory"]=json!([]);
        assert!(policy(&input).unwrap()["Tracker"].as_object().unwrap().is_empty());
    }
}
