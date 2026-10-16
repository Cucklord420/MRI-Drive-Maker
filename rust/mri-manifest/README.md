# Rust MRI manifest — offline parity port

This is a **read-only** Rust candidate for the original `Get-MRIManifest` and `Get-MRIFileHash` in `MRI.Batch.psm1`. It recurses source folders, validates Windows-safe relative paths, rejects linked paths, calculates SHA-256 in 1 MiB chunks, detects changed file sizes/timestamps, supports both optional ignore-system and no-hash modes, and reports an ordered manifest with a SHA-256 summary.

```powershell
cargo run --manifest-path rust/mri-manifest/Cargo.toml -- C:\SyntheticMRI
cargo run --manifest-path rust/mri-manifest/Cargo.toml -- C:\SyntheticMRI --no-hash --ignore-system-directories
```

The Windows CI runs the **real original** `Get-MRIManifest` function for **400 synthetic cases** (100 separate folder trees × four combinations of flags), comparing each path/entry/size/SHA and the overall manifest digest. All test data are disposable local files; this port **does not discover, erase, partition, format, or write USB drives**.

The original Windows launcher, copy process, and safety/authorization gates are unchanged. Only after original-vs-Rust parity and real approved hardware testing may the privileged production stages be reconsidered.
