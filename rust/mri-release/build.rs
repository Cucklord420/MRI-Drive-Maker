// SPDX-License-Identifier: GPL-3.0-only
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_HELPER_BYTES: u64 = 16 * 1024 * 1024;

fn helper_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > MAX_HELPER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "copy worker must be a nonempty bounded regular file",
        ));
    }

    let capacity = usize::try_from(metadata.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "copy worker size overflow"))?;
    let mut bytes = Vec::with_capacity(capacity);
    File::open(path)?
        .take(MAX_HELPER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let total = u64::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "copy worker size overflow"))?;
    if total == 0 || total > MAX_HELPER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "copy worker exceeds the build-time size limit",
        ));
    }
    if total != metadata.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "copy worker changed while it was read",
        ));
    }
    Ok(bytes)
}

fn main() {
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let embedded_helper = output_dir.join("trusted_copy_helper.bin");
    if target_os == "windows" {
        let profile_dir = output_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("Cargo OUT_DIR must be inside the target profile directory");
        let helper_path = profile_dir.join("mri-copy.exe");
        println!("cargo:rerun-if-changed={}", helper_path.display());
        match helper_bytes(&helper_path) {
            Ok(bytes) => {
                let digest = format!("{:x}", Sha256::digest(&bytes));
                fs::write(&embedded_helper, bytes)
                    .expect("embed the trusted Rust copy worker bytes");
                println!("cargo:rustc-env=MRI_COPY_SHA256={digest}");
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::write(&embedded_helper, Vec::<u8>::new())
                    .expect("write an empty copy-worker placeholder");
            }
            Err(error) => panic!("Cannot pin the Rust copy worker digest: {error}"),
        }
    } else {
        fs::write(&embedded_helper, Vec::<u8>::new())
            .expect("write an empty copy-worker placeholder");
    }
}
