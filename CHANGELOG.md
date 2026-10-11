# Changes — 10-10-2026

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
