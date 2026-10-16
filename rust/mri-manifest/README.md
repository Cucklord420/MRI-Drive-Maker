# Rust MRI manifest — offline parity port

This is a **read-only** Rust candidate for the original `Get-MRIManifest` and `Get-MRIFileHash` in `MRI.Batch.psm1`. It recurses source folders, validates Windows-safe relative paths, rejects linked paths, calculates SHA-256 in 1 MiB chunks, detects changed file sizes/timestamps, supports both optional ignore-system and no-hash modes, and reports an ordered manifest with a SHA-256 summary.

```powershell
cargo run --manifest-path rust/mri-manifest/Cargo.toml -- C:\SyntheticMRI
cargo run --manifest-path rust/mri-manifest/Cargo.toml -- C:\SyntheticMRI --no-hash --ignore-system-directories
```

The Windows CI runs the **real original** `Get-MRIManifest` function for **400 synthetic cases** (100 separate folder trees × four combinations of flags), comparing each path/entry/size/SHA and the overall manifest digest. All test data are disposable local files; this port **does not discover, erase, partition, format, or write USB drives**.

The original Windows launcher, copy process, and safety/authorization gates are unchanged. Only after original-vs-Rust parity and real approved hardware testing may the privileged production stages be reconsidered.

## Final source/target manifest comparison

`mri-manifest --compare-json-file INPUT.json` accepts `Expected.Entries`, `Actual.Entries`, `MutableFiles`, `Exact`, and `LayoutOnly` values. It returns JSON `Ok` and `Error` and uses a nonzero exit code for rejected comparisons.

The original PowerShell `Assert-MRIManifestMatch` remains intact. A Windows CI differential test performs **600 fictional comparisons** against the actual function: exact and layout-only rules, missing and extra paths, mutable empty-file rejection, file type/length/SHA mismatches, and default case-insensitive Windows path/hash comparisons. It compares accepted/rejected status and **exact rejection text**.

This check is purely in-memory and cannot enumerate, mount, modify, partition, or erase a USB disk. Production copying and its security decisions are not delegated to the Rust binary until separately verified.
