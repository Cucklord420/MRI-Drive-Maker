# MRI Drive Maker

[Download the v0.1.1 ZIP](https://github.com/Cucklord420/MRI-Drive-Maker/raw/refs/heads/main/downloads/MRI-Drive-Maker-v0.1.1.zip). This preview includes the scripts, guide, license, tests, and portable Git history. It fixes the Windows PowerShell 5.1 startup error in v0.1.0. Windows hardware and the actual MRI workflow have not yet been validated.

Updated 10-10-2026 from [JStone21/MRI-Creation](https://github.com/JStone21/MRI-Creation), upstream commit `aa7bdc528eb95952be3c95710e277025b3eea5be` dated 12-03-2023. GPL-3.0-only; original license and attribution retained. This is an independent community project. It is not affiliated with or endorsed by Best Buy or Geek Squad.

Register the intended store USB drives once. The watcher detects their arrival, copies the current official MRI files, verifies the copy, runs the specified media-creation batch, waits for its normal descendant processes, verifies final files, flushes writes, and reports READY or FAILED for each drive. Two builds can copy concurrently by default; setup batches run one at a time to avoid shared temporary files or utility conflicts.

**The actual MRI payload and media-creation batch are not included.** Use the store's current official source. The filename `mri.bat` is a configurable default based on the requested workflow; a batch that launches MRI diagnostics is not necessarily the batch that prepares boot media. No claim is made that either operation is equivalent.

## Quick setup

1. Extract the ZIP to a separate folder on the Windows workstation's internal NTFS drive. Right-click `Start-MRI-Batch.cmd` and select **Run as administrator**. The launcher uses process-scoped RemoteSigned; it does not change computer policy, unblock files, or override enforced signing policy.
2. Choose **1 Configure**, select the extracted official MRI folder, and enter its actual media-creation batch path relative to that folder. Enter any real batch arguments one at a time, or press Enter for none. Copy mode is the default. Prepare mode creates one active MBR/FAT32 partition; it needs a separate erase registration for every intended drive.
3. Choose **2 Register**. Check the displayed disk number, model, size, and hardware serial against the intended physical store USB. Select only the intended disk numbers. Type the displayed phrase for each selected USB. Registration changes no drive contents. Number the physical drives to match their displayed labels.
4. Choose **4 Watch**. Plug in registered drives individually or through existing available ports. Swapping to the next group does not require restarting the watcher. Observe the per-drive stages. At READY, use Windows **Safely Remove Hardware** before unplugging. Press **Q** to stop and cancel unfinished builds.

Drives can be registered in several groups; all 30 need not be attached together. New hardware must be registered explicitly. A drive with no usable serial, a duplicate serial, an offline/write-protected drive, or a boot/system disk is rejected. Source and automation disks are protected. The default capacity range is 4-64 decimal GB; capacity alone never authorizes a write.

## What READY establishes

The copy's SHA-256 checks passed; the configured batch returned exit code zero; normal descendant processes in its Windows job ended; required final files exist and are nonempty; source files still match unless individually listed as mutable; final output was hashed; and Windows accepted the volume flush. A receipt is saved on the workstation, and the current physical USB/volume/letter mapping was checked again.

**READY is not a BIOS/UEFI boot test or proof that a batch silently succeeded internally.** A batch can return zero incorrectly, spawn work through services/WMI/task scheduling outside its normal process tree, or act on drives chosen by its own code. Those behaviors require a recipe-specific completion check or a corrected official batch. This wrapper cannot infer the intended target of an arbitrary script. Keep drives connected throughout their build; polling and process cancellation cannot make unplugging during destructive storage operations safe.

A batch that reformats/repartitions or changes drive letters during setup is unsupported: the mapping guard deliberately stops it. Use this tool's preparation stage before copying, followed by a setup batch whose intended target is its USB working directory or explicit target argument. Interactive menus can be shown with `SetupInteractive: true`; the tool never invents responses to them. Fully unattended use requires a batch that needs no prompts.

## File protection and recovery

Unknown USBs are ignored. Copy-only mode does not overwrite nonempty media for a new recipe. Preparation also refuses unexpected files, even on an erase-registered drive. Existing files are accepted only when they exactly match this tool's previous successful local receipt; client files and unrecognized/modified contents block processing before erasure. A verified drive with the same source and settings is checked again and reported ALREADY_READY without recopying it.

A failed or interrupted partial build gets no new successful receipt and is not retried endlessly while attached. If partial files remain, the next build can be refused. Inspect that USB and reset it through the store's normal permitted process after confirming it contains no client data. The wrapper deliberately offers no blanket wipe or automatic recovery erase. Removing the workstation's `Data` folder also removes registrations and receipts; nonempty drives will then need inspection/reset instead of being blindly trusted.

Logs, source manifests, per-drive state, and receipts remain under the workstation's `Data` folder. No wrapper files, logs, or success markers are added to the MRI USB. Only the selected official source and files produced by its configured batch go on it. Content inspection blocks links/junctions and checks source-file stability while hashing.

## Settings

`mri-config.json` is created by the menu; `mri-config.example.json` shows all fields. Restart the watcher after changing the source or settings. Hashing is done once for the source per session, then once before setup and once after setup for each target.

| Setting | Purpose |
| --- | --- |
| `SourcePath` / `SetupBatch` | Extracted current official files / actual relative .bat or .cmd media-creation path. |
| `SetupArguments` | An array of individual arguments; optional `{DriveLetter}`, `{DriveRoot}`, `{DiskNumber}`, `{Label}` placeholders. Shell metacharacters are rejected. |
| `PrepareDisk` | Default false; true requires individual erase registration and creates MBR/FAT32, capped at 32,000,000,000 bytes. Remaining capacity is unused. |
| `RequiredFiles` | Exact relative files that must exist and be nonempty after setup. Configure any real boot/setup outputs that are absent from the source. |
| `MutableFiles` | Exact source files the official setup is known to change; default empty. Directory-wide exceptions are not supported. |
| `MaxParallel` / `SerializeSetup` | Default 2 builds; default true serializes setup only. Raise concurrency only after checking the actual batch's targeting and shared-file behavior. |
| `CopyTimeoutSeconds` / `SetupTimeoutSeconds` | Defaults 3600 / 900 seconds. A setup-slot wait has its own same-duration limit. Timeouts stop the contained command tree and fail the drive. |
| `PollSeconds` / `StablePolls` | Defaults 2 / 3; a registered USB must remain visible across three successful inventories before queuing. Active-command identity checks run at nominal 500 ms intervals, plus forced boundary checks. |
| `SetupInteractive` | Default false: output goes to a log and stdin is EOF. True opens a visible console for real prompts; output then remains in that console. |
| `MinimumDiskBytes` / `MaximumDiskBytes` | Default 4,000,000,000 / 64,000,000,000. These are extra checks, not a substitute for registration. |

A FAT32 file cannot exceed 4,294,967,295 bytes. Oversized files and insufficient space are rejected. Robocopy exit codes 8 and above fail the copy; lower codes still require full content verification. Extra source files, outdated source choices, and a batch's own behavior are the operator's responsibility; this wrapper does not decide which MRI release is currently approved.

## Validation and scope

`tests/StartupSmoke.ps1` launches the real entry points in child PowerShell processes, including the Windows `.cmd` launcher on Windows. It uses an isolated extracted copy, an unrelated working directory, and a folder with spaces and an ampersand. Menu input is mocked to return Q in that temporary copy so these checks do not require a console keyboard. The checks use menu exit and simulated builds only; no USB or proprietary MRI files are accessed. `.github/workflows/powershell.yml` runs these checks and the native process smoke test on Windows PowerShell 5.1. Current CI results are available in [GitHub Actions](https://github.com/Cucklord420/MRI-Drive-Maker/actions).

On 10-10-2026, all 48 simulated PowerShell tests passed in **100 consecutive runs (4,800/4,800 test cases, zero failures)** in PowerShell 7.6.6 on Linux. [The report](validation/100-run/results.json) records per-run results, tested source hashes, timings, and log hashes; the first and last run logs are retained as samples. `tests/Invoke-ValidationRuns.ps1` repeats the suite any number of times and writes timestamped per-run logs plus a fresh `results.json`, so future validation runs are independently verifiable instead of resting on a single stored report. PowerShell scripts parsed and the C# interop compiled. Tests exercised the actual build orchestration, actual temporary-file copies and hashing, atomic reports, identity/volume checks, detection state, erasure restrictions, failed copies/batches, cancellation, unexpected content, receipts, and final-file checks. Storage operations, native Windows process execution, and volume flushing were mocked in the build tests. The code targets Windows PowerShell 5.1 and .NET Framework 4.8 on Windows 10/11; that host has not been exercised here.

`tests/SelfTest.ps1` is nondestructive and can run on Windows as well. `tests/WindowsProcessSmoke.ps1` is a separate nondestructive Windows-only check for waiting on a detached child and killing it on cancellation; it does not access USB drives. It was supplied but not run here. No hardware boot test, actual MRI batch invocation, or physical formatting test was possible in this environment. There is no measured claim about how long 30 drives will take.

The release ZIP also includes `Source-History.git.bundle`, a portable Git copy containing the upstream history and this update. The legacy V3/V4 entry points route to the new menu. Their old all-USB formatting source is retained under `legacy/` as non-executable `.txt` files for provenance.

## License and attribution

This modified project is released under the **GNU General Public License, version 3 only** (`GPL-3.0-only`). The complete, unchanged upstream license is in [LICENSE](LICENSE). Original authorship, the dated modification notice, and warranty information are in [NOTICE.md](NOTICE.md). Original V3/V4 sources are preserved verbatim under `legacy/`.

The repository and download ZIP contain the complete editable PowerShell and C# source, launch scripts, example configuration, and tests. There are no closed-source wrapper components, extra restrictions, or proprietary MRI files in this distribution.

## Authorization and cost

Use only files and USB drives you are authorized to use, and obtain any required workplace approval before running this project. This repository contains no proprietary MRI payload, internal company documents, or store credentials. Users must supply their own authorized source files and media-creation batch.

**Additional TCO:** $0 in software or subscription charges using the existing workstation, official source, and ordered USBs. The drives' already-planned purchase cost, any new hub cost, electricity, and staff time are not known. Verification adds USB reads; actual throughput depends on payload size, drive speeds, and shared port bandwidth. No new purchase is required by the tool.

Sources reviewed 10-10-2026:

- [Upstream repository](https://github.com/JStone21/MRI-Creation), commit dated 12-03-2023; GPL-3.0 license retained.
- [Second matching repository](https://github.com/GS209-BTH/Create-MRI); inspected only, no source copied into this project.
- [Microsoft Windows job objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects): normal descendants are contained; work created through WMI can escape this relationship.
- [Microsoft Robocopy return codes](https://learn.microsoft.com/en-us/troubleshoot/windows-server/backup-and-storage/return-codes-used-robocopy-utility), updated 02-12-2026.
- [Microsoft FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers), updated 10-12-2021.
- [Microsoft Clear-Disk](https://learn.microsoft.com/en-us/powershell/module/storage/clear-disk), [Get-Volume](https://learn.microsoft.com/en-us/powershell/module/storage/get-volume), and [Format-Volume](https://learn.microsoft.com/en-us/powershell/module/storage/format-volume).
