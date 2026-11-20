// SPDX-License-Identifier: GPL-3.0-only
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_HELPER_BYTES: u64 = 16 * 1024 * 1024;

fn helper_digest(path: &Path) -> io::Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > MAX_HELPER_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "copy worker must be a nonempty bounded regular file",
        ));
    }

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("copy worker size overflow"))?;
        if total > MAX_HELPER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "copy worker exceeds the build-time size limit",
            ));
        }
        hasher.update(&buffer[..read]);
    }
    if total != metadata.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "copy worker changed while its digest was computed",
        ));
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn main() {
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let profile_dir = output_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("Cargo OUT_DIR must be inside the target profile directory");
        let helper_path = profile_dir.join("mri-copy.exe");
        println!("cargo:rerun-if-changed={}", helper_path.display());
        match helper_digest(&helper_path) {
            Ok(digest) => println!("cargo:rustc-env=MRI_COPY_SHA256={digest}"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => panic!("Cannot pin the Rust copy worker digest: {error}"),
        }
    }
}
