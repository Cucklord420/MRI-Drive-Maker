# MRI Drive Maker

[Download v0.3.0](https://github.com/Cucklord420/MRI-Drive-Maker/raw/refs/heads/main/downloads/MRI-Drive-Maker-v0.3.0.zip). Extract the whole ZIP, then right-click **START-MRI-DRIVE-MAKER.cmd → Run as administrator**. This is the only launcher you need.

Lists USBs, cleans the drives you select, formats them as FAT32, and copies your source folder's contents directly to each USB root. Several drives build at once. No BDE utility or setup batch is required.

## Use it

1. Choose **1** and select the extracted MRI folder. Select the folder whose contents should appear at the USB root. An actual ISO or ZIP file is also supported.
2. Choose **2**, check the displayed USB models, serials and disk numbers, then enter the intended numbers separated by commas.
3. Type the displayed confirmation, for example **ERASE 6,7,8**. **This deletes all files and partitions on those selected USBs.** Approval applies only to this batch; registration is unnecessary.
4. Leave the drives connected until **READY**. The batch finishes after every selected drive succeeds or fails. Use Windows **Safely Remove Hardware** before unplugging.

New configurations allow **4 simultaneous builds**. Choose **4** in the menu to set **1–30**; existing saved limits are preserved. Remaining selected drives wait for a slot. Shared hubs and USB controllers share bandwidth, so 30 slots is not a measured 30-drive speedup. Press **Q** during a session to cancel unfinished builds.

The download contains the launcher, this quick guide, license/notices, and editable source/tests inside **App/**. Configuration and **Data/** are created beside the launcher. Historical launchers and test reports stay in this repository, outside the download.

## What happens to a drive

The selected batch cleans existing partitions, creates one active MBR/FAT32 partition using up to **32,000,000,000 bytes**, and copies the source contents to its root. Additional drive capacity remains unused. Each copied file is checked with SHA-256 after writes are flushed; failed or interrupted builds never receive a new success receipt. No report or marker is added to the USB.

Physical identity is checked again before storage operations. System/boot disks, source/application disks, non-USB disks, offline/write-protected disks, and missing or duplicate hardware serials are blocked. If a selected drive disappears or its disk number changes while waiting, its build fails rather than following another disk.

Empty sources, files larger than FAT32 permits, insufficient capacity, unsafe paths, and source links/junctions are rejected. ISO files are mounted read-only; ZIP files use a temporary extraction cache. A selected folder's contents are copied exactly, without silently flattening a child directory.

## Automatic watcher and compatibility

**A Advanced** retains registration and continuous watching for successive groups of drives. That mode ignores unregistered USBs and refuses unexpected existing files even on erase-enabled drives. It does not inherit the selected batch's one-time erase approval. Verified unchanged watcher media can report **ALREADY_READY** without copying again.

Existing command-line actions and optional setup-batch recipes remain available. See [advanced settings](docs/ADVANCED.md), [source review](docs/SOURCE-REVIEW.md), and [validation](VALIDATION.txt). Old launcher aliases are archived as non-executable text under `legacy/compatibility/`.

## Rust source-release generator

The candidate download is generated and verified by Rust directly from the pinned Git revision:

```text
cargo test --locked --manifest-path rust/mri-release/Cargo.toml
cargo run --locked --manifest-path rust/mri-release/Cargo.toml -- --output candidate.zip
cargo run --locked --manifest-path rust/mri-release/Cargo.toml -- --verify candidate.zip
```

The Rust program is the only supported release builder and verifier. The former Python entry point was retired after the Rust archive passed byte-for-byte parity checks during the migration; it is no longer present in the working tree or used by CI. CI tests the Rust CLI in debug and optimized release builds, then opens the generated download on Windows using the shipped launcher and startup tests. The existing published v0.3.0 download is not automatically replaced by this migration. No proprietary MRI source files or physical USB media are used in packaging tests.

## Validation, license and cost

Windows CI checks real entry points, selection/confirmation, simulated failures, ISO mounting, native process cancellation, and FAT32/copy/hash operations on owned disposable virtual disks. Four actual copy-worker lifetimes overlap in the concurrency test. The selected-batch check also cleans existing multi-partition contents and ends without writing permanent erase registrations. See [Actions](https://github.com/Cucklord420/MRI-Drive-Maker/actions) for results tied to source revisions.

**READY verifies copied files; it does not establish MRI bootability.** Physical USB throughput, firmware booting and exact BDE-to-USB equivalence have not been tested. No proprietary MRI files or BDE program are included.

Based on [JStone21/MRI-Creation](https://github.com/JStone21/MRI-Creation), upstream revision `aa7bdc528eb95952be3c95710e277025b3eea5be` (12-03-2023). Original V3/V4 bytes remain in `legacy/`. The unchanged [LICENSE](LICENSE) is **GPL-3.0-only**; [NOTICE.md](NOTICE.md) records modifications. No affiliation with Best Buy or Geek Squad is claimed.

**Additional TCO:** $0 software/subscription fees using the existing workstation and USBs. Staff time, electricity and hardware costs vary.

Sources: JStone21 — 12-03-2023 — https://github.com/JStone21/MRI-Creation
