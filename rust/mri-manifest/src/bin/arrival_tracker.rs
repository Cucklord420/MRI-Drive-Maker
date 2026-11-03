// SPDX-License-Identifier: GPL-3.0-only
//! Pure-data stable-arrival tracking. Drive-block decisions are caller-validated.
use serde_json::{json, Value};
use std::collections::HashSet;

fn policy(input: &Value) -> Result<Value, String> {
    let drives = input["Inventory"].as_array().ok_or("Missing Inventory")?;
    let registry = input["Records"].as_array().ok_or("Missing Records")?;
    let minimum = input["StablePolls"].as_i64().ok_or("Missing StablePolls")?;
    if minimum < 2 {
        return Err("StablePolls must be at least 2".into());
    }

    // Silently replacing a malformed persisted tracker with an empty one can
    // hide identity/state corruption. Only a real empty object starts fresh.
    let mut tracker = input["Tracker"]
        .as_object()
        .ok_or("Missing or invalid Tracker")?
        .clone();
    let mut present = HashSet::new();
    let mut ready = Vec::new();

    for drive in drives {
        let Some(key) = drive["Key"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        // A physical identity must be observed at most once in a poll. The
        // block decision is supplied externally; reject duplicate identities
        // even if a caller incorrectly marks both entries unblocked.
        if !present.insert(key.to_lowercase()) {
            return Err("Ambiguous duplicate inventory identity".into());
        }
        let number = drive["Number"].as_i64().ok_or("Missing drive Number")?;
        let blocked = drive["Blocked"]
            .as_bool()
            .ok_or("Missing validated drive Blocked decision")?;

        // PowerShell's original $Tracker hashtable uses case-insensitive keys.
        // Preserve the existing entry (including its spelling) between polls.
        let matching_keys = tracker
            .keys()
            .filter(|stored| stored.to_lowercase() == key.to_lowercase())
            .cloned()
            .collect::<Vec<_>>();
        if matching_keys.len() > 1 {
            return Err("Ambiguous duplicate tracker identity".into());
        }
        let storage_key = matching_keys
            .into_iter()
            .next()
            .unwrap_or_else(|| key.to_owned());
        let mut state = tracker
            .get(&storage_key)
            .cloned()
            .unwrap_or_else(|| json!({"Number":number,"Stable":0,"Attempted":false}));
        // Treat tracker state as untrusted persisted input rather than
        // accepting silently coerced counts or attempted flags.
        if state["Number"].as_i64().is_none()
            || !state["Stable"].as_i64().is_some_and(|stable| stable >= 0)
            || state["Attempted"].as_bool().is_none()
        {
            return Err("Malformed tracker state".into());
        }

        if state["Number"] != number {
            state["Number"] = json!(number);
            state["Stable"] = json!(0);
        }

        if blocked {
            state["Stable"] = json!(0);
        } else {
            let stable = state["Stable"]
                .as_i64()
                .ok_or("Bad Stable count")?
                .checked_add(1)
                .ok_or("Stable count overflow")?;
            state["Stable"] = json!(stable);
            let matches = registry
                .iter()
                .filter(|record| {
                    record["Key"]
                        .as_str()
                        .is_some_and(|id| id.eq_ignore_ascii_case(key))
                })
                .collect::<Vec<_>>();
            if matches.len() == 1 && stable >= minimum && state["Attempted"] == false {
                ready.push(matches[0].clone());
            }
        }

        tracker.insert(storage_key, state);
    }

    tracker.retain(|key, _| present.contains(&key.to_lowercase()));
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
        let mut input = json!({
            "Inventory":[{"Key":"disk-x","Number":7,"Blocked":false}],
            "Records":[{"Key":"disk-x","Label":"MRI_01"}],
            "StablePolls":3,
            "Tracker":{}
        });
        for count in 1..=3 {
            let next = policy(&input).unwrap();
            let expected = if count == 3 { 1 } else { 0 };
            assert_eq!(next["Arrivals"].as_array().unwrap().len(), expected);
            input["Tracker"] = next["Tracker"].clone();
        }
        input["Inventory"][0]["Blocked"] = json!(true);
        let blocked = policy(&input).unwrap();
        assert_eq!(blocked["Tracker"]["disk-x"]["Stable"], 0);
        input["Inventory"] = json!([]);
        assert!(policy(&input).unwrap()["Tracker"]
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn changing_identity_case_preserves_stable_polls_without_duplicate_entries() {
        let input = json!({
            "Inventory":[{"Key":"disk-x","Number":7,"Blocked":false}],
            "Records":[{"Key":"DISK-X","Label":"MRI_01"}],
            "StablePolls":3,
            "Tracker":{
                "DISK-X":{"Number":7,"Stable":2,"Attempted":false}
            }
        });
        let result = policy(&input).unwrap();
        assert_eq!(result["Arrivals"].as_array().unwrap().len(), 1);
        assert_eq!(result["Tracker"].as_object().unwrap().len(), 1);
        assert_eq!(result["Tracker"]["DISK-X"]["Stable"], 3);
    }

    #[test]
    fn ambiguous_case_variants_and_missing_block_decision_fail_closed() {
        let mut input = json!({
            "Inventory":[{"Key":"disk-x","Number":7,"Blocked":false}],
            "Records":[{"Key":"disk-x"}],
            "StablePolls":2,
            "Tracker":{
                "disk-x":{"Number":7,"Stable":1,"Attempted":false},
                "DISK-X":{"Number":7,"Stable":1,"Attempted":false}
            }
        });
        assert_eq!(
            policy(&input).unwrap_err(),
            "Ambiguous duplicate tracker identity"
        );
        input["Tracker"] = json!({});
        input["Inventory"][0]["Blocked"] = Value::Null;
        assert_eq!(
            policy(&input).unwrap_err(),
            "Missing validated drive Blocked decision"
        );
    }

    #[test]
    fn duplicate_inventory_identity_cannot_count_as_two_stable_polls() {
        let input = json!({
            "Inventory":[
                {"Key":"disk-x","Number":7,"Blocked":false},
                {"Key":"DISK-X","Number":7,"Blocked":false}
            ],
            "Records":[{"Key":"disk-x","Label":"MRI_01"}],
            "StablePolls":3,
            "Tracker":{"disk-x":{"Number":7,"Stable":1,"Attempted":false}}
        });
        assert_eq!(
            policy(&input).unwrap_err(),
            "Ambiguous duplicate inventory identity"
        );
    }

    #[test]
    fn malformed_persisted_tracker_is_rejected() {
        let mut input = json!({
            "Inventory":[{"Key":"disk-x","Number":7,"Blocked":false}],
            "Records":[{"Key":"disk-x","Label":"MRI_01"}],
            "StablePolls":2,
            "Tracker":null
        });
        assert_eq!(policy(&input).unwrap_err(), "Missing or invalid Tracker");
        input["Tracker"] = json!({
            "disk-x":{"Number":7,"Stable":1,"Attempted":"false"}
        });
        assert_eq!(policy(&input).unwrap_err(), "Malformed tracker state");
        input["Tracker"]["disk-x"]["Attempted"] = json!(false);
        input["Tracker"]["disk-x"]["Stable"] = json!(-1);
        assert_eq!(policy(&input).unwrap_err(), "Malformed tracker state");
        input["Tracker"]["disk-x"]["Stable"] = json!(1);
        input["Tracker"]["disk-x"]["Number"] = Value::Null;
        assert_eq!(policy(&input).unwrap_err(), "Malformed tracker state");
    }

    #[test]
    fn stale_and_overflowed_counters_cannot_yield_false_arrivals() {
        let input = json!({
            "Inventory":[{"Key":"disk-x","Number":7,"Blocked":false}],
            "Records":[{"Key":"disk-x"}],
            "StablePolls":2,
            "Tracker":{
                "disk-x":{"Number":7,"Stable":9223372036854775807_i64,"Attempted":false}
            }
        });
        assert_eq!(policy(&input).unwrap_err(), "Stable count overflow");
    }
}
