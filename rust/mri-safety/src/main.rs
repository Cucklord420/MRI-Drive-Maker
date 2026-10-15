// SPDX-License-Identifier: GPL-3.0-only
//! Pure, read-only USB selection policy port.
//! NO disk enumeration, formatting, erasing, mounting, or write capability.
//! Intended for parity testing against MRI.Batch.psm1 before native integration.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{self, Read};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Disk {
    key: String,
    number: i32,
    serial: String,
    #[serde(default)]
    identity_error: String,
    bus_type: String,
    size: i64,
    is_boot: bool,
    is_system: bool,
    is_offline: bool,
    is_read_only: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Request {
    selection: String,
    inventory: Vec<Disk>,
    protected_keys: Vec<String>,
    minimum_disk_bytes: i64,
    maximum_disk_bytes: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
struct Response {
    selected: Vec<i32>,
    confirmation: Option<String>,
    error: Option<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn block_reason<'a>(disk: &'a Disk, request: &'a Request) -> Option<String> {
    if disk.key.trim().is_empty() {
        return Some(format!("Hardware identity unavailable: {}", disk.identity_error));
    }
    if disk.bus_type != "USB" || disk.is_boot || disk.is_system {
        return Some("Boot, system, and non-USB disks are excluded.".into());
    }
    if disk.is_offline || disk.is_read_only {
        return Some("Drive is offline or write protected.".into());
    }
    if request.protected_keys.iter().any(|key| same(key, &disk.key)) {
        return Some("This disk contains the source files or the automation itself.".into());
    }
    if disk.size < request.minimum_disk_bytes || disk.size > request.maximum_disk_bytes {
        return Some("Capacity is outside the configured MRI range.".into());
    }
    if request.inventory.iter().filter(|other| same(&other.serial, &disk.serial)).count() != 1 {
        return Some("Duplicate USB serial number; cannot identify this drive safely.".into());
    }
    None
}

fn select(request: &Request) -> Result<Vec<i32>, String> {
    if request.selection.trim().is_empty() {
        return Ok(Vec::new());
    }
    let format = Regex::new(r"^\s*[0-9]+(\s*,\s*[0-9]+)*\s*$").expect("constant regex");
    if !format.is_match(&request.selection) {
        return Err("Enter disk numbers separated by commas.".into());
    }
    let mut numbers = Vec::new();
    let mut seen = HashSet::new();
    for part in request.selection.split(',') {
        let number = part.trim().parse::<i32>().map_err(|_| "Invalid disk number.")?;
        if seen.insert(number) {
            numbers.push(number);
        }
    }
    // Validate entire selection before allowing any selected disk to be returned.
    for number in &numbers {
        let matches: Vec<&Disk> = request.inventory.iter().filter(|disk| disk.number == *number).collect();
        if matches.len() != 1 {
            return Err(format!("Disk {number} was not in the displayed USB list."));
        }
        if let Some(reason) = block_reason(matches[0], request) {
            return Err(format!("Disk {number}: {reason}"));
        }
    }
    Ok(numbers)
}

fn respond(request: &Request) -> Response {
    match select(request) {
        Ok(selected) => {
            let confirmation = if selected.is_empty() {
                None
            } else {
                let mut sorted = selected.clone();
                sorted.sort_unstable();
                Some(format!("ERASE {}", sorted.iter().map(i32::to_string).collect::<Vec<_>>().join(",")))
            };
            Response { selected, confirmation, error: None }
        }
        Err(error) => Response { selected: vec![], confirmation: None, error: Some(error) }
    }
}

fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("Could not read selection input.");
        std::process::exit(2);
    }
    let request: Request = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("Invalid inventory JSON. No drive selected.");
            std::process::exit(2);
        }
    };
    let output = respond(&request);
    println!("{}", serde_json::to_string(&output).expect("serializable response"));
    if output.error.is_some() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn disk(n: i32, serial: &str) -> Disk {
        Disk {
            key: format!("KEY_{n}"),
            number: n,
            serial: serial.to_owned(),
            identity_error: String::new(),
            bus_type: "USB".into(),
            size: 32_000_000_000,
            is_boot: false,
            is_system: false,
            is_offline: false,
            is_read_only: false,
        }
    }
    fn request(selection: &str) -> Request {
        Request {
            selection: selection.into(),
            inventory: vec![disk(6, "SERIAL6"), disk(7, "SERIAL7"), disk(8, "SERIAL8")],
            protected_keys: vec![],
            minimum_disk_bytes: 1_000_000_000,
            maximum_disk_bytes: 64_000_000_000,
        }
    }
    #[test]
    fn valid_multiple_and_duplicate_selection() {
        assert_eq!(respond(&request("8,6,7,8")).selected, vec![8,6,7]);
        assert_eq!(respond(&request("8,6,7,8")).confirmation.as_deref(), Some("ERASE 6,7,8"));
        assert_eq!(respond(&request("\n\t ")).selected, Vec::<i32>::new());
    }
    #[test]
    fn invalid_input_never_yields_partial_selection() {
        for value in ["6;", "6,!", "-1", "6,7,", "6,,7", "abc", "2147483648"] {
            let output = respond(&request(value));
            assert!(output.error.is_some(), "{value}");
            assert!(output.selected.is_empty());
        }
    }
    #[test]
    fn blocked_hardware_conditions_are_fail_closed() {
        let conditions: [fn(&mut Request); 9] = [
            |q| q.inventory[0].key.clear(),
            |q| q.inventory[0].bus_type = "SATA".into(),
            |q| q.inventory[0].is_boot = true,
            |q| q.inventory[0].is_system = true,
            |q| q.inventory[0].is_offline = true,
            |q| q.inventory[0].is_read_only = true,
            |q| q.protected_keys.push("key_6".into()),
            |q| q.inventory[0].size = 500_000_000,
            |q| q.inventory[1].serial = "serial6".into(),
        ];
        for mutate in conditions {
            let mut q = request("6,8");
            mutate(&mut q);
            let result = respond(&q);
            assert!(result.selected.is_empty());
            assert!(result.error.is_some(), "{result:?}");
        }
    }
    #[test]
    fn unknown_or_reused_disk_numbers_rejected() {
        let bad = respond(&request("6,5"));
        assert_eq!(bad.error.as_deref(), Some("Disk 5 was not in the displayed USB list."));
        let mut duplicate = request("6");
        duplicate.inventory.push(disk(6, "OTHER"));
        assert!(respond(&duplicate).error.is_some());
    }
    #[test]
    fn hundred_deterministic_variations_have_strict_approval() {
        for i in 0..100 {
            let mut q = request("8,6");
            if i % 2 == 1 { q.inventory[0].is_boot = true; }
            if i % 3 == 2 { q.inventory[2].is_read_only = true; }
            if i % 5 == 4 { q.protected_keys.push("KEY_6".into()); }
            let result = respond(&q);
            let allowed = i % 2 == 0 && i % 3 != 2 && i % 5 != 4;
            assert_eq!(result.error.is_none(), allowed, "variation {i}");
            if allowed {
                assert_eq!(result.selected, [8,6]);
                assert_eq!(result.confirmation.as_deref(), Some("ERASE 6,8"));
            } else {
                assert!(result.selected.is_empty());
            }
        }
    }
    #[test]
    fn serial_and_key_matching_is_case_insensitive() {
        let mut q = request("6");
        q.inventory[1].serial = "serial6".into();
        assert_eq!(respond(&q).error.unwrap(), "Disk 6: Duplicate USB serial number; cannot identify this drive safely.");
        let mut q = request("6");
        q.protected_keys.push("key_6".into());
        assert_eq!(respond(&q).error.unwrap(), "Disk 6: This disk contains the source files or the automation itself.");
    }
}
