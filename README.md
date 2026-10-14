# MRI Drive Maker

[Download v0.2.0](https://github.com/Cucklord420/MRI-Drive-Maker/raw/refs/heads/main/downloads/MRI-Drive-Maker-v0.2.0.zip). Extract the whole ZIP, then right-click **START-MRI-DRIVE-MAKER.cmd** and choose **Run as administrator**. It is the only launcher at the top of the download. `App/` contains the editable program and tests; `Developer/` contains historical source and validation records.

Choose the current authorized MRI ISO once, register the intended physical USB drives once, then leave the watcher running as you connect successive groups of drives. The program mounts an ISO read-only, copies its contained files, verifies every copied file, flushes writes, and reports each drive's result. A ZIP containing the extracted files or one ISO, and an already-extracted folder, are also supported. No media-creation batch is required for this workflow.

This is an independent, GPL-3.0-only media builder based on [JStone21/MRI-Creation](https://github.com/JStone21/MRI-Creation), upstream commit `aa7bdc528eb95952be3c95710e277025b3eea5be` dated 12-03-2023. It is not affiliated with or endorsed by Best Buy or Geek Squad. **The proprietary MRI payload and BDE-to-USB utility are not included.** This builder has not been shown to reproduce every operation of BDE-to-USB. Actual MRI booting, Secure Boot, and legacy BIOS compatibility remain unverified.

## What to open and select

1. Extract the ZIP to a separate folder on the workstation's internal NTFS drive. Open **START-MRI-DRIVE-MAKER.cmd**, using **Run as administrator**. It uses Windows PowerShell 5.1 and process-scoped RemoteSigned; enforced signing policy still applies.
2. Choose **1 Choose MRI ISO / ZIP / extracted folder** and select the official source. For a normal ISO, select the `.iso` file directly; no manual extraction is needed. The default recipe requires `efi\boot\bootx64.efi` and rejects files exceeding FAT32's limit before a USB is written.
3. Choose copy mode for already-empty FAT32 drives, or prepare mode to create an active MBR/FAT32 partition. Prepare mode requires separate erase authorization when registering each drive. It uses up to 32,000,000,000 bytes; additional drive capacity remains unused.
4. Choose **2 Register USB drives**. Match the displayed model, capacity, and hardware serial to each intended physical USB. Select their disk numbers and type the displayed confirmation for each. Label the physical drives with their assigned MRI labels. Registration changes no drive contents. Register drives in groups if necessary.
5. Choose **4 Watch and build**. Plug in registered drives. Two builds run concurrently by default. Leave each connected until READY, then use Windows **Safely Remove Hardware** before unplugging it. Press **Q** to stop the watcher and cancel unfinished builds.

In the download, the root folder contains the launcher and guide. Configuration and `Data/` are created beside that launcher. Supporting `.ps1` files inside `App/` are source files; the old launchers are retained as non-executable text under `Developer/compatibility/`. In a Git checkout the same launcher uses the root source files.

## What READY verifies

READY means the source and final copied files passed SHA-256 comparison, the configured required files exist and are nonempty, Windows accepted the write flush, and the registered USB's physical identity and volume mapping were checked again. With the default direct-copy recipe, no setup batch runs. **READY is a file verification result, not a firmware boot test.** Finding an EFI filename does not establish that its contents are a valid or compatible boot loader. This release focuses on copying the source's UEFI file tree; it does not install a separate legacy BIOS boot sector.

A successful local receipt belongs to the physical drive and source recipe. Reinserting unchanged, previously verified media checks its contents again and reports ALREADY_READY without recopying. Source image length and modification time are checked during the session; changing the source requires restarting the watcher.

## Drive protection and recovery

Unknown USBs are ignored. Boot/system, offline, read-only, non-USB, source, and automation disks are excluded. Hardware identity combines the serial, PNP identity, model, and capacity; blank or duplicate serials are rejected. Capacity limits alone never authorize a write. Before destructive operations, fresh Storage and Win32_DiskDrive objects are matched to the registration; operations use the bound storage object rather than a stale disk number. Cancellation is checked before preparation, including on unpartitioned drives.

Both copy and preparation reject unexpected files. Existing files are accepted only when they match this tool's previous successful local receipt. Client files, modified files, or an unrecognized nonempty drive block processing before erasure. A failed partial build gets no new success receipt and is not retried repeatedly while attached. Inspect and reset that USB through the normal authorized process before retrying. Removing `Data/` removes registrations and receipts; nonempty drives will then require inspection/reset.

Keep drives connected throughout a build. Polling and cancellation cannot make unplugging during storage operations safe. Logs, manifests, state, and receipts stay in the workstation's `Data/`; the tool adds no marker or report files to the MRI USB. Links and junctions in source/target paths are rejected. ZIP extraction rejects traversal paths, duplicate names, symbolic links, and oversized archives, and uses an isolated cache removed when the source closes. ISO mounts created by the tool are removed when it finishes; existing user mounts are preserved.

## Settings and optional batch compatibility

The menu creates `mri-config.json`. The complete example is in `App/mri-config.example.json` in the download and at the repository root. Restart the watcher after changing settings.

| Setting | Default and purpose |
| --- | --- |
| `SourceKind` / `SourcePath` | `Folder`, `ISO`, or `ZIP`, and the selected source path. The menu chooses the type. Older configurations without `SourceKind` remain folder recipes. |
| `SetupBatch` | Empty: copy and verify only. Advanced compatibility recipes may specify a real relative `.bat`/`.cmd` media-preparation path. |
| `RequiredFiles` | `efi\boot\bootx64.efi`. Direct-copy recipes require these nonempty files in the source before writing. |
| `PrepareDisk` | False. True requires each drive's erase registration and prepares active MBR/FAT32. |
| `MaxParallel` | 2; permitted range 1–8. USB bandwidth and verification reads determine useful concurrency. |
| `PollSeconds` / `StablePolls` | 2 / 3. Registered drives must remain visible across three inventories before queuing. |
| `MinimumDiskBytes` / `MaximumDiskBytes` | 4,000,000,000 / 2,000,000,000,000. Extra checks, not authorization. |
| `CopyTimeoutSeconds` / `SetupTimeoutSeconds` | 3600 / 900. Timeouts stop the contained command tree and fail the drive. |
| `SetupArguments` / `MutableFiles` / `SetupInteractive` | Empty / empty / false. Only optional batch recipes may use these; direct-copy mode rejects them. |
| `SerializeSetup` | True. Optional batch processes run one at a time. Copies remain parallel. |

A FAT32 file cannot exceed 4,294,967,295 bytes. Oversized files, insufficient free space, and any nonzero copy-worker exit are rejected; a zero exit still requires full file verification. Sources with oversized files need an appropriate official media recipe rather than an invented split operation.

For advanced batch recipes, a zero exit code and completion of the normal descendant process tree are also required. A batch must target its USB working directory or an explicit supported argument; `{DriveLetter}`, `{DriveRoot}`, `{DiskNumber}`, and `{Label}` placeholders are available. Commands with shell metacharacters are rejected. The builder cannot infer the target of an arbitrary batch, detect a false internal success code, or contain work launched through services/WMI/scheduled tasks. A batch that repartitions or changes drive letters during setup is unsupported because the volume mapping guard stops it.

## Validation and limitations

On 10-14-2026 the tested runtime passed eight startup checks, 56 simulated build cases, ten source/identity checks, two native process checks, and eight real owned ISO/VHD checks on Windows PowerShell 5.1.26100.33438 / Windows Server 2025. [Windows run](https://github.com/Cucklord420/MRI-Drive-Maker/actions/runs/38510584392). Current checks and evidence are recorded in [VALIDATION.txt](VALIDATION.txt) and [GitHub Actions](https://github.com/Cucklord420/MRI-Drive-Maker/actions). The v0.2.0 audit adds direct-copy, cancellation-before-formatting, source archive, storage-object identity, package-layout, and actual packaged launcher regressions. The Windows storage smoke test creates its own temporary ISO and file-backed VHD, then exercises real ISO mounting, Storage cmdlets, FAT32 volume GUID paths, the actual Windows file-copy worker, hashing, native process handling, and flushing. Only USB classification is simulated for the owned VHD; the destructive guard must still reject that non-USB disk.

`tests/SelfTest.ps1` uses temporary files with simulated storage/process APIs. `tests/SourceSmoke.ps1` uses temporary archive and disk-identity fixtures. `tests/StartupSmoke.ps1` launches real child processes from an unrelated working directory; only menu input in an isolated copy is mocked to Q. `tests/PackageSmoke.ps1` verifies the actual ZIP hashes/layout and launches its top-level CMD file on Windows. `tests/WindowsProcessSmoke.ps1` checks real descendant completion/cancellation. `tests/WindowsStorageSmoke.ps1` formats only its newly created, explicitly checked file-backed fixture, never an attached physical USB.

Historical 10-10-2026 validation completed 100 simulated runs with 48 tests each: 4,800 passes. The 10-13-2026 v0.1.1 fix passed eight Windows startup checks, 48 simulated cases, and two real process checks. Those historical results apply to their recorded source revisions and do not establish v0.2.0 or physical MRI bootability. The original suite missed the parameter-binding startup error because it imported the module directly instead of launching the app with `powershell.exe -File`.

No actual MRI image, BDE-to-USB executable, physical USB, Windows 10/11 workstation, or firmware boot test is available in this test environment. There is no measured estimate for preparing 30 drives. The source download includes complete editable source, tests, attribution, historical evidence, and a portable Git history bundle.

## License and cost

The entire modified project is **GPL-3.0-only**. The complete upstream [LICENSE](LICENSE) is unchanged. [NOTICE.md](NOTICE.md) preserves original authorship and dated modification notices; original V3/V4 sources remain verbatim as text under `legacy/`. No extra license restrictions are imposed.

Use only source files and drives you are authorized to use. This distribution contains no proprietary MRI files, internal company documents, store credentials, or BDE-to-USB program.

**Additional TCO:** $0 in software or subscription charges with the existing workstation and USBs. Hardware, electricity, and staff time vary; new hubs or drives are not required by this program. Verification adds reads and depends on shared USB bandwidth.

Sources: JStone21 — 12-03-2023 — https://github.com/JStone21/MRI-Creation

Sources: Microsoft — 06-14-2023 — https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/install-windows-from-a-usb-flash-drive?view=windows-11

Sources reviewed 10-14-2026: Microsoft Storage cmdlet documentation — https://learn.microsoft.com/en-us/powershell/module/storage/mount-diskimage?view=windowsserver2025-ps and https://learn.microsoft.com/en-us/powershell/module/storage/new-partition?view=windowsserver2025-ps
