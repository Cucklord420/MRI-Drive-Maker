# License, attribution, and modification notice

MRI Drive Maker is based on **MRI-Creation by JStone21**:
https://github.com/JStone21/MRI-Creation

Upstream reference: `aa7bdc528eb95952be3c95710e277025b3eea5be`, dated 12-03-2023.
Copyright in the original work remains with its original authors. Original
source files and their notices are retained verbatim under `legacy/`.

Copyright (C) 2026 Cucklord420 for the modifications and additions.

**Modified 10-10-2026:** replaced blanket USB formatting with registered-drive
selection, continuous insertion detection, guarded copying and optional disk
preparation, contained batch execution, verified completion, and tests.
The former V3/V4 entry points now open the new menu. These changes are an
independent modification; the upstream project was not changed.

**Modified 10-13-2026:** corrected Windows PowerShell 5.1 startup path
initialization and validation-runner startup, added entry-point regression
checks and Windows CI, and refreshed the source download.

**Modified 10-14-2026:** added direct ISO/ZIP input and verified copying
without a setup batch, corrected cancellation and storage-object checks before
preparation, clarified the single launcher, and added archive, package, and
owned Windows ISO/VHD regression checks.

**Modified 10-14-2026 (v0.3.0):** added adjustable parallel preparation/copying
and letter-free direct-copy targets, removed redundant direct-copy hashing,
added authenticated runtime data and cancellable hashing, strengthened path
and exact-content checks, and added owned parallel Windows storage tests.

**Modified 10-15-2026:** re-read original V3/V4 logic and restored a simple
selected-drive clean/FAT32/copy-to-root workflow with one batch erase-list
confirmation. Automatic watching and legacy batch recipes remain advanced
options. Archived old launcher aliases as text and removed historical logs
and Git bundles from the user ZIP; original source bytes remain in the repo.
Added selection/consent and real finite-batch Windows VHD checks.

This entire modified project is released under the **GNU General Public
License, version 3 only** (`GPL-3.0-only`). Recipients may copy, modify, and
redistribute it under that license. The full upstream license is preserved
unchanged in `LICENSE`; no additional restrictions are imposed.

This software is provided **WITHOUT ANY WARRANTY** to the extent allowed by
law. See sections 15 and 16 of `LICENSE`.

The distribution includes the complete editable source and supporting scripts.
MRI payload files are not included. No Best Buy or Geek Squad affiliation,
endorsement, or approval is claimed.
