# Changes

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
