# Changes

## 10-14-2026 — v0.3.0 parallel USB preparation and copy checks

- Add a visible 1–30 simultaneous-build setting, default four for new configurations, and active/waiting totals. Preserve existing settings and keep optional legacy setup serialized by default.
- Support direct copy/preparation with volume GUIDs and no drive letters. Source contents still go directly to each FAT32 root.
- Remove the redundant direct-copy hash pass: flush, then verify every target file once with SHA-256 and exact path matching. Keep pre-batch and final hashing for legacy batch recipes.
- Make hashing cancellable between chunks; authenticate runtime config/manifest/copy-plan data with hashes calculated before writing and a single authenticated read.
- Reject Windows device aliases and unexpected final extra files in direct mode. Retain physical identity, source/system exclusions, unknown-content protection and per-drive erase consent.
- Exclude operational settings from media recipes and safely migrate matching old direct-copy receipts. Required filenames are optional; empty sources remain blocked before preparation.
- Add 30-drive queue/security regressions and four owned-VHD Windows builds covering concurrent preparation/copying, letter-free targets, process overlap and verification. No physical USB/MRI boot or throughput claim is made.
- Fixed a real Windows RAW-disk failure: partition discovery reports no partitions as an error. Only confirmed RAW disks can proceed without partitions; formatted-disk inventory errors still block writes.
- Preserve the selected folder and mounted ISO root exactly rather than flattening legitimate single-child folders.
- Permit only owned ZIP source caches within a source checkout; continue blocking source/program overlap. This fixes ZIP watching when launching the source checkout instead of the packaged App layout.

## 10-14-2026 — v0.2.0 ISO source and audit fixes

- Accept an ISO directly, mount it read-only once per watcher, reuse existing mounts without ejecting them, and clean up owned mounts/caches after jobs stop. ZIPs containing media files or one ISO and extracted folders are supported.
- Make copying and verification independent of a setup batch. The menu no longer asks for `mri.bat`; advanced older batch recipes remain supported.
- Reproduced and fixed cancellation reaching preparation on an unpartitioned drive. Recheck cancellation and actual storage-object identity before destructive operations, and bind partition creation to the checked object.
- Validate required boot files before direct-copy erasure, reject fractional/string timing settings and negative copy exits, protect automation data disks, keep ready/failed totals mutually exclusive, and remove a newly written receipt if final completion checks fail.
- Fixed a real Windows failure: Robocopy rejected the pinned volume GUID destination with error 53/exit 16. Copying now uses a contained Windows file-copy worker while keeping GUID-based writes, timeouts, cancellation, and final hashes.
- Replace the flat source download with one clear top-level `START-MRI-DRIVE-MAKER.cmd` launcher, supporting code under `App/`, and historical files under `Developer/` as non-executable text.
- Add source/archive/physical-object regressions, actual package launch testing, and a Windows smoke test using only owned temporary ISO/VHD fixtures. File verification is not proof of physical MRI booting or BDE-to-USB equivalence.

## 10-13-2026 — v0.1.1 startup fix

- Fixed the default configuration path when the launcher invokes an advanced script with `powershell.exe -File` on Windows PowerShell 5.1. Script-relative defaults now resolve after parameter binding.
- Fixed the same startup defect in `tests/Invoke-ValidationRuns.ps1` and its assignment to the read-only `$PSEdition` variable, while preserving explicit path arguments.
- Added a nondestructive startup smoke test covering the menu, legacy entry points, explicit/empty config paths, the main self-test entry point, the validation runner, and the Windows `.cmd` launcher from another working directory and a path with spaces and an ampersand.
- Added Windows PowerShell 5.1 CI for startup, simulated build safety, and native process completion/cancellation. Physical USBs and the proprietary MRI batch remain outside these checks.
- Verified 8/8 startup checks, 48/48 simulated build cases, and both native process checks on Windows PowerShell 5.1 CI; seven applicable startup checks also passed on PowerShell 7.6.6/Linux. Evidence is in `validation/startup-10-13-2026/`.

## 10-10-2026

Independent update based on JStone21/MRI-Creation; the upstream repository was not changed.

- Replaced blanket USB selection with explicit physical-drive registration, serial/PNP/model/capacity identity, duplicate-serial rejection, and source/system disk exclusions.
- Added continuous plug-in detection, stable attachment checks, bounded simultaneous copies, serialized setup, visible status, local reports, verified reinsertion skipping, and cancellation.
- Added official-batch invocation in the USB working directory, optional explicit arguments, whole normal-process-tree tracking through Windows jobs, per-command timeouts, volume-map monitoring, and volume flushing.
- Added complete source/copy/final SHA-256 checks, exact mutable-file exceptions, required final outputs, FAT32 limits, source stability checks, link rejection, and unknown-content protection.
- Made formatting optional and disabled by default, with individual per-drive erase registration and repeated identity checks.
- Retired the unsafe legacy entry points; retained original files as non-executable provenance and preserved the GPL license.
- Added 48 simulated safety/orchestration tests, a nondestructive Windows process smoke test, and usage/limitations documentation. Windows storage, process execution, actual MRI media creation, and boot behavior remain unverified here.
- Added explicit GPL-3.0-only source headers, attribution and dated modification notices, visible menu license information, and a complete source download.
- Completed the full 48-test simulated suite 100 consecutive times: 4,800 passed, zero failures; added all run logs and tested source hashes.
