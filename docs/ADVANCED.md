# Advanced settings and compatibility

The primary menu builds a selected batch without permanent registration. These settings document the retained automatic watcher and optional legacy batch recipes. To copy without formatting, use a trusted saved configuration with `PrepareDisk: false` and the advanced watcher; selected batches always clean and format.

Runtime configuration, source manifests and native copy plans are hashed from the parent's in-memory serialized data; each child reads once and checks its expected hash before use. Modified runtime plans cannot redirect the copy. Windows device aliases such as CON/NUL are rejected in source/archive paths. These checks do not protect against an administrator, modified program source, a malicious optional batch, or changes to the initial trusted configuration/registry. Use trusted source and program files; this is not a sandbox for untrusted administrative code.

The menu creates `mri-config.json`. The complete example is in `App/mri-config.example.json` in the download and at the repository root. Restart the watcher after changing settings.

| Setting | Default and purpose |
| --- | --- |
| `SourceKind` / `SourcePath` | `Folder`, `ISO`, or `ZIP`, and the selected source path. The menu chooses the type. Older configurations without `SourceKind` remain folder recipes. |
| `SetupBatch` | Empty: copy and verify only. Advanced compatibility recipes may specify a real relative `.bat`/`.cmd` media-preparation path. |
| `RequiredFiles` | Empty. Optional configured required files must be present/nonempty in direct-copy sources before writing. |
| `PrepareDisk` | False. True requires each drive's erase registration and prepares active MBR/FAT32. |
| `MaxParallel` | 4 for new configurations; permitted range 1–30. Existing limits are preserved; menu 4 changes them. |
| `PollSeconds` / `StablePolls` | 2 / 3. Registered drives must remain visible across three inventories before queuing. |
| `MinimumDiskBytes` / `MaximumDiskBytes` | 4,000,000,000 / 2,000,000,000,000. Extra checks, not authorization. |
| `CopyTimeoutSeconds` / `SetupTimeoutSeconds` | 3600 / 900. Timeouts stop the contained command tree and fail the drive. |
| `SetupArguments` / `MutableFiles` / `SetupInteractive` | Empty / empty / false. Only optional batch recipes may use these; direct-copy mode rejects them. |
| `SerializeSetup` | True. Optional batch processes run one at a time. Copies remain parallel. |

A FAT32 file cannot exceed 4,294,967,295 bytes. Oversized files, insufficient free space, and any nonzero copy-worker exit are rejected; a zero exit still requires full file verification. Sources with oversized files need an appropriate official media recipe rather than an invented split operation.

For advanced batch recipes, a zero exit code and completion of the normal descendant process tree are also required. A batch must target its USB working directory or an explicit supported argument; `{DriveLetter}`, `{DriveRoot}`, `{DiskNumber}`, and `{Label}` placeholders are available. Commands with shell metacharacters are rejected. The builder cannot infer the target of an arbitrary batch, detect a false internal success code, or contain work launched through services/WMI/scheduled tasks. A batch that repartitions or changes drive letters during setup is unsupported because the volume mapping guard stops it.


Command-line actions: `Configure`, `Build`, `Enroll`, `List`, `Watch`, and `SelfTest`. `Build` displays a selection and requires the erase-list confirmation; it does not silently approve drives. Configuration and Data paths can be passed explicitly. The program requires Windows PowerShell 5.1 and administrator privileges for storage operations.

The initial source/configuration/registry and program files must be trusted. Runtime hashes detect changed child-process inputs; they do not create an administrator security boundary. Keep the source outside the application folder. Optional legacy batches are administrative code and may perform their own actions.
