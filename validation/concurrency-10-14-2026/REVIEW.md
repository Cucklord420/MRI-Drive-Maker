# Parallel USB review — 10-14-2026

The requested operation is selected USB discovery, controlled FAT32 preparation, and copying source contents directly to each drive's root. No BDE-to-USB integration or required setup batch was added.

The previous release already defaulted to two background builds, with an eight-build ceiling. This does not prove why an operator saw one copy at a time: an older build, saved `MaxParallel: 1`, an optional serialized setup phase, or shared hardware limits could differ. The update adds a visible 1–30 setting, defaults new configurations to four, preserves saved settings, and displays active/waiting results. Optional batch setup serialization remains for compatibility and safety; direct-copy work has no such global setup lock.

Direct mode no longer needs drive letters. Destructive preparation still resolves and rechecks the exact registered physical drive at each boundary, uses its bound Storage object, and requires its individual erase consent. System, source, unregistered, read-only, ambiguous and unexpected-content drives remain blocked.

Direct copying formerly hashed target files before and after the no-op setup stage. It now checks copied layout, flushes, then verifies every file once with SHA-256 and exact path matching. A late extra file cannot be silently added to a successful direct-copy receipt. Optional batch recipes retain both hash passes. Hashing checks cancellation/identity between chunks rather than waiting for an entire large file.

Config, source-manifest and native-copy plan hashes are computed from serialized in-memory values before they are written. Workers perform one authenticated read before using paths/settings. This catches changed runtime data and avoids a hash-then-reopen replacement race. Windows device aliases are rejected before archive extraction. These checks do not protect against someone able to change the initial trusted configuration/registry, program source, or administrative process itself. Optional batches remain trusted administrative code.

Media recipe identity excludes concurrency, polling, timeouts and setup serialization. Operational tuning does not recopy verified current media. Old direct-copy receipts migrate only after file verification and an exact source match; prepare-mode migration also checks active MBR layout.

The first real parallel Windows run found that `Get-Partition` throws when a newly attached RAW disk has no partitions. The corrected code confirms RAW state before treating that error as no partitions. Errors for formatted disks still stop processing. The following Windows run passed the existing startup, source, single-ISO/VHD and process suites plus four real concurrent preparation/copy builds.

The four-disk check creates only new, explicitly owned file-backed VHDs. USB inventory/classification is simulated; the preparation guard binds each destructive call to its uniquely created VHD. Preparation orchestration, partitioning, FAT32 formatting, native copy workers, root destinations, hashes and flushes are real. A barrier injected only into temporary test copies records four distinct native copy process lifetimes and proves they overlap. No test hook, special permission to treat VHDs as USB, fixture image or company document is included in the program.

Selected folders and mounted ISO roots are preserved; a legitimate single subdirectory is not silently flattened. New recipes have no assumed boot filename and reject empty sources before preparation. ZIP wrapper handling remains supported.

Physical USB throughput, a simultaneous 30-device hardware run, a Windows 10/11 store workstation, the actual MRI payload and firmware bootability are not verified. Parallelism shares source/USB bandwidth and uses more memory/CPU. No measured speedup or new hardware purchase is claimed. Software TCO is $0; existing hardware, electricity and staff costs vary.

Primary documentation reviewed:

Sources: Microsoft — 08-28-2024 — https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file

Sources: Microsoft — 01-07-2021 — https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-volume

Sources reviewed: Microsoft — 10-14-2026 — https://learn.microsoft.com/en-us/powershell/module/storage/new-partition?view=windowsserver2025-ps
