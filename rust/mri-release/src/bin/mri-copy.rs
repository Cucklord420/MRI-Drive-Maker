// SPDX-License-Identifier: GPL-3.0-only
//! Handle-anchored source copier. This binary intentionally does not expose
//! release building or verification commands.
#[path = "../copy_worker.rs"]
mod copy_worker;

use std::ffi::OsString;
use std::path::Path;

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0].to_str() == Some("--preflight") {
        return;
    }
    if args.len() != 3 || args[0].to_str() != Some("--copy") {
        eprintln!("Usage: mri-copy --preflight | --copy PLAN_JSON EXPECTED_SHA256");
        std::process::exit(2);
    }
    let Some(expected_hash) = args[2].to_str() else {
        eprintln!("Expected plan hash must be ASCII hexadecimal");
        std::process::exit(2);
    };
    if let Err(reason) = copy_worker::copy_plan(Path::new(&args[1]), expected_hash) {
        eprintln!("{reason}");
        std::process::exit(1);
    }
}
