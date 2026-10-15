# Original source review — 10-15-2026

Read before this cleanup: JStone21/MRI-Creation at `aa7bdc528eb95952be3c95710e277025b3eea5be`, including both V3/V4 and README. The preserved `legacy/MRI_CreatorV3.ps1.txt` and V4 text blobs match upstream exactly (`5b2849f1ed147085ad2e34768355b0756d7efd23` and `7d282ea0571168b0a9aa494a23bc1ad6d203dbde`).

| Original behavior | Cleanup |
| --- | --- |
| Enumerate every USB disk and ask to format all | Display USBs, select exact disk numbers, confirm the displayed erase list for one batch |
| Format every drive first, then copy to each in a sequential foreach loop | Bounded independent per-drive builds; default 4, configurable 1–30 |
| Clear, create an active partition capped at 32 billion bytes, format FAT32 | Explicitly initialize RAW disks as MBR, then prepare active FAT32 with capacity headroom |
| Copy selected folder contents to each drive-letter root | Preserve exact selected folder root; copy to the volume GUID without consuming drive letters |
| Copy errors can be nonterminating; completion is printed unconditionally | Stop failed work; flush and hash every copied file before READY |
| V4 adds elevation checking and a graphical folder picker | Retain both, default to the extracted-folder workflow; ISO/ZIP input remains optional |

The original source does not invoke a BDE utility or any setup batch. Its two foreach loops explain sequential copying. It relies on the initial disk list and drive letters without binding writes to fresh physical identities. It does not verify file hashes or provide per-drive verified results.

Compared with main v0.2.0 and the pending parallel-work branch, the cleanup removes mandatory registration and preparation-mode choices from the primary task. A fresh erase-list confirmation permits selected drives with existing contents, multiple partitions or a failed partial copy to be cleaned. The automatic watcher retains its stricter empty/receipt-matched content rule; it receives no one-time erase approval.

Boot/system/source/application protection, serial/PNP/model/capacity identity checks, cancellation, authenticated runtime plans, path validation, the native copy backend, and SHA-256 verification remain. Legacy batch behavior is retained behind the advanced workflow rather than added to the normal copy flow.

Only START-MRI-DRIVE-MAKER.cmd remains a root launcher. Compatibility wrappers are archived as non-executable text. The user ZIP includes current editable source/tests, license and guide; Git bundles, old releases and historical logs stay in the repository. The builder records exact Git source origins and produces a reproducible ZIP.

The separately merged static-analysis workflow from main commit `5f0aebd8984fba3910bc5dfba5ee5f70173fdc01` is retained unchanged.

Validation uses owned temporary virtual disks, not physical store USBs. It cannot establish MRI firmware boot compatibility or a measured speedup for 30 USB drives.

Sources: JStone21 — 12-03-2023 — https://github.com/JStone21/MRI-Creation/tree/aa7bdc528eb95952be3c95710e277025b3eea5be
