// SPDX-License-Identifier: GPL-3.0-only
//! Windows source-tree identity snapshots and handle-anchored copy worker.
//! This module is limited to ordinary files and directories. It has no disk
//! enumeration, formatting, erasing, mounting, or volume-preparation APIs.

use serde::{Deserialize, Serialize};
#[cfg(windows)]
use sha2::Digest;
use std::collections::BTreeMap;
#[cfg(windows)]
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct FileIdentity {
    pub volume_serial: String,
    pub file_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct SnapshotEntry {
    pub identity: FileIdentity,
    pub is_directory: bool,
    pub length: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct AncestorIdentity {
    pub path: String,
    pub identity: FileIdentity,
    pub is_reparse_point: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct SourceIdentitySnapshot {
    pub root_path: String,
    pub ancestors: Vec<AncestorIdentity>,
    pub entries: BTreeMap<String, SnapshotEntry>,
}

#[cfg(windows)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ManifestEntry {
    relative_path: String,
    is_directory: bool,
    length: u64,
    sha256: String,
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Manifest {
    entries: Vec<ManifestEntry>,
    source_identity: Option<SourceIdentitySnapshot>,
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CopyPlan {
    source_root: String,
    target_root: String,
    manifest: Manifest,
}

pub fn write_snapshot(root: &Path, output: &Path) -> Result<(), String> {
    let snapshot = capture_snapshot(root)?;
    let bytes = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| format!("Cannot create source identity snapshot: {e}"))?;
    use std::io::Write;
    file.write_all(&bytes)
        .map_err(|e| format!("Cannot write source identity snapshot: {e}"))
}

pub fn copy_plan(plan_path: &Path, expected_hash: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::copy_plan_inner(plan_path, expected_hash)
    }
    #[cfg(not(windows))]
    {
        let _ = (plan_path, expected_hash);
        Err("The copy worker requires Windows".into())
    }
}

#[cfg(windows)]
fn validate_copy_plan(
    plan_path: &Path,
    expected_hash: &str,
) -> Result<(CopyPlan, SourceIdentitySnapshot), String> {
    let raw = std::fs::read(plan_path).map_err(|e| format!("Cannot read copy plan: {e}"))?;
    let actual_hash = format!("{:x}", sha2::Sha256::digest(&raw));
    if !is_sha256(expected_hash) || !actual_hash.eq_ignore_ascii_case(expected_hash) {
        return Err("Runtime data changed or is corrupt. No copy is allowed.".into());
    }
    let plan: CopyPlan = serde_json::from_slice(&raw).map_err(|_| "Invalid copy plan JSON")?;
    let snapshot = plan
        .manifest
        .source_identity
        .clone()
        .ok_or("Copy plan has no source identity snapshot")?;
    validate_manifest(&plan.manifest.entries, &snapshot)?;
    Ok((plan, snapshot))
}

#[cfg(windows)]
fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(windows)]
fn validate_manifest(
    entries: &[ManifestEntry],
    snapshot: &SourceIdentitySnapshot,
) -> Result<(), String> {
    let mut names = BTreeSet::new();
    let mut folded = BTreeSet::new();
    for entry in entries {
        validate_relative_path(&entry.relative_path)?;
        if !names.insert(entry.relative_path.clone())
            || !folded.insert(entry.relative_path.to_lowercase())
        {
            return Err("Copy manifest contains a duplicate or case-colliding path".into());
        }
        if entry.is_directory {
            if entry.length != 0 || !entry.sha256.is_empty() {
                return Err(format!(
                    "Invalid directory metadata: {}",
                    entry.relative_path
                ));
            }
        } else if !is_sha256(&entry.sha256) || entry.length > u32::MAX as u64 {
            return Err(format!(
                "Invalid FAT32 file metadata: {}",
                entry.relative_path
            ));
        }
    }
    if names.len() != snapshot.entries.len()
        || names
            .iter()
            .any(|name| !snapshot.entries.contains_key(name))
    {
        return Err("Source identity snapshot does not match the copy manifest".into());
    }
    for entry in entries {
        let identity = &snapshot.entries[&entry.relative_path];
        if identity.is_directory != entry.is_directory || identity.length != entry.length {
            return Err(format!(
                "Source identity metadata changed: {}",
                entry.relative_path
            ));
        }
        let segments: Vec<_> = entry.relative_path.split('\\').collect();
        let mut parent = String::new();
        for segment in segments.iter().take(segments.len().saturating_sub(1)) {
            if !parent.is_empty() {
                parent.push('\\');
            }
            parent.push_str(segment);
            if !snapshot
                .entries
                .get(&parent)
                .is_some_and(|parent_entry| parent_entry.is_directory)
            {
                return Err(format!("Missing source parent directory: {parent}"));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.starts_with('\\') || path.starts_with('/') || path.contains('/') {
        return Err(format!("Invalid source-relative path: {path}"));
    }
    let device =
        regex::Regex::new(r"(?i)^(?:CON|PRN|AUX|NUL|CLOCK\$|COM[1-9¹²³]|LPT[1-9¹²³])(?:\.|$)")
            .expect("constant device-name regex");
    for segment in path.split('\\') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || matches!(segment.chars().last(), Some(' ' | '.'))
            || segment
                .chars()
                .any(|c| c.is_control() || ":*?\"<>|".contains(c))
            || device.is_match(segment)
        {
            return Err(format!("Unsafe source-relative path: {path}"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_volume_guid_root(path: &str) -> Result<(), String> {
    let lower = path.to_ascii_lowercase();
    if !lower.starts_with(r"\\?\volume{") || !path.ends_with("}\\") {
        return Err("The copy target must be a volume GUID root.".into());
    }
    let guid = &path[11..path.len() - 2];
    if guid.len() != 36
        || !guid.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err("The copy target must be a volume GUID root.".into());
    }
    Ok(())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::ffi::c_void;
    use std::fs;
    use std::ptr::{null, null_mut};

    type HandleRaw = *mut c_void;
    const INVALID_HANDLE_VALUE: HandleRaw = -1_isize as HandleRaw;
    const FILE_READ_DATA: u32 = 0x0001;
    const FILE_LIST_DIRECTORY: u32 = 0x0001;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_WRITE_DATA: u32 = 0x0002;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const OPEN_EXISTING: u32 = 3;
    const CREATE_NEW: u32 = 1;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_BEGIN: u32 = 0;
    const FILE_ATTRIBUTE_TAG_INFO_CLASS: i32 = 9;
    const FILE_STANDARD_INFO_CLASS: i32 = 1;
    const FILE_ID_INFO_CLASS: i32 = 18;
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    const ERROR_PATH_NOT_FOUND: u32 = 3;
    const ERROR_ALREADY_EXISTS: u32 = 183;
    const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;

    #[repr(C)]
    struct FileAttributeTagInfo {
        attributes: u32,
        reparse_tag: u32,
    }

    #[repr(C)]
    struct FileStandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        number_of_links: u32,
        delete_pending: u8,
        directory: u8,
    }

    #[repr(C)]
    struct FileIdInfo {
        volume_serial: u64,
        file_id: [u8; 16],
    }

    // SAFETY: These signatures match the Win32 declarations. The wrappers below
    // provide NUL-terminated paths, valid output buffers, and owned handles.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *const c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: HandleRaw,
        ) -> HandleRaw;
        fn GetFileInformationByHandleEx(
            file: HandleRaw,
            info_class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
        fn ReadFile(
            file: HandleRaw,
            buffer: *mut c_void,
            bytes_to_read: u32,
            bytes_read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn WriteFile(
            file: HandleRaw,
            buffer: *const c_void,
            bytes_to_write: u32,
            bytes_written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn SetFilePointerEx(
            file: HandleRaw,
            distance: i64,
            new_position: *mut i64,
            method: u32,
        ) -> i32;
        fn GetFileAttributesW(name: *const u16) -> u32;
        fn CreateDirectoryW(name: *const u16, security_attributes: *const c_void) -> i32;
        fn GetLastError() -> u32;
        fn CloseHandle(handle: HandleRaw) -> i32;
    }

    struct Handle(HandleRaw);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle was returned by CreateFileW and is owned here.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Handle {
        fn raw(&self) -> HandleRaw {
            self.0
        }
    }

    struct PreparedSource {
        files: BTreeMap<String, Handle>,
        _directory_locks: Vec<Handle>,
        _ancestor_locks: Vec<Handle>,
    }

    fn wide(path: &str) -> Vec<u16> {
        path.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn win_error(context: &str) -> String {
        // SAFETY: GetLastError has no preconditions.
        let code = unsafe { GetLastError() };
        format!("{context} (Windows error {code})")
    }

    fn absolute_extended(path: &Path) -> Result<String, String> {
        let absolute =
            std::path::absolute(path).map_err(|e| format!("Cannot resolve source path: {e}"))?;
        let value = absolute
            .to_str()
            .ok_or("Source path is not valid Unicode")?
            .replace('/', "\\");
        if value.starts_with("\\\\?\\") {
            return Ok(value.trim_end_matches('\\').to_owned() + "\\");
        }
        if value.starts_with(r"\\") {
            return Ok(format!("\\\\?\\UNC\\{}", value.trim_start_matches('\\')));
        }
        if value.as_bytes().get(1) == Some(&b':') {
            return Ok(format!("\\\\?\\{}", value));
        }
        Err("Source path must be a local volume or UNC share path".into())
    }

    fn split_volume_root(path: &str) -> Result<(String, Vec<String>), String> {
        if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
            let mut parts = rest.split('\\');
            let server = parts
                .next()
                .filter(|x| !x.is_empty())
                .ok_or("Invalid UNC source root")?;
            let share = parts
                .next()
                .filter(|x| !x.is_empty())
                .ok_or("Invalid UNC source root")?;
            let root = format!("\\\\?\\UNC\\{}\\{}\\", server, share);
            return Ok((
                root,
                parts.filter(|x| !x.is_empty()).map(str::to_owned).collect(),
            ));
        }
        if path.len() >= 7
            && path.starts_with("\\\\?\\")
            && path.as_bytes()[5] == b':'
            && path.as_bytes()[6] == b'\\'
        {
            let root = path[..7].to_owned();
            let parts = path[7..]
                .split('\\')
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .collect();
            return Ok((root, parts));
        }
        if path.starts_with("\\\\?\\Volume{") {
            let end = path.find("}\\").ok_or("Invalid volume GUID source root")? + 2;
            let root = path[..end].to_owned();
            let parts = path[end..]
                .split('\\')
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .collect();
            return Ok((root, parts));
        }
        Err("Source path must start at a drive, volume, or UNC share root".into())
    }

    fn open(path: &str, directory: bool) -> Result<Handle, String> {
        let name = wide(path);
        let access = (if directory {
            FILE_LIST_DIRECTORY
        } else {
            FILE_READ_DATA
        }) | FILE_READ_ATTRIBUTES
            | SYNCHRONIZE;
        // Omitting FILE_SHARE_WRITE and FILE_SHARE_DELETE prevents another
        // process from modifying or renaming an opened source object.
        // SAFETY: `name` is NUL-terminated, and all other arguments are scalar/null.
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                access,
                FILE_SHARE_READ,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(win_error(&format!("Cannot open source path {path}")));
        }
        let handle = Handle(raw);
        let (identity, is_directory, length, attributes) = query(&handle)?;
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!("Source contains a link or reparse point: {path}"));
        }
        if is_directory != directory {
            return Err(format!("Source path type changed: {path}"));
        }
        let _ = (identity, length);
        Ok(handle)
    }

    fn open_ancestor(
        path: &str,
        is_source_root: bool,
    ) -> Result<(Handle, FileIdentity, bool), String> {
        let name = wide(path);
        // SAFETY: `name` is NUL-terminated, and all other arguments are scalar/null.
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_SHARE_READ,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(win_error(&format!("Cannot open source ancestor {path}")));
        }
        let handle = Handle(raw);
        let (identity, is_directory, _, attributes) = query(&handle)?;
        if !is_directory {
            return Err(format!("Source ancestor is not a directory: {path}"));
        }
        let is_reparse_point = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        if is_reparse_point {
            return Err(if is_source_root {
                "Source root must be a real directory, not a link".into()
            } else {
                format!("Source ancestry contains a link or reparse point: {path}")
            });
        }
        Ok((handle, identity, is_reparse_point))
    }

    fn query(handle: &Handle) -> Result<(FileIdentity, bool, u64, u32), String> {
        let mut id = FileIdInfo {
            volume_serial: 0,
            file_id: [0; 16],
        };
        let mut standard = FileStandardInfo {
            allocation_size: 0,
            end_of_file: 0,
            number_of_links: 0,
            delete_pending: 0,
            directory: 0,
        };
        let mut attributes = FileAttributeTagInfo {
            attributes: 0,
            reparse_tag: 0,
        };
        // SAFETY: each structure has the documented layout and exact size for its info class.
        let ok_id = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                FILE_ID_INFO_CLASS,
                (&mut id as *mut FileIdInfo).cast(),
                std::mem::size_of::<FileIdInfo>() as u32,
            )
        };
        if ok_id == 0 {
            return Err(win_error("Cannot read source file identity"));
        }
        let identity = identity_from_file_id(&id)?;
        // SAFETY: same as above for FILE_STANDARD_INFO.
        let ok_standard = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                FILE_STANDARD_INFO_CLASS,
                (&mut standard as *mut FileStandardInfo).cast(),
                std::mem::size_of::<FileStandardInfo>() as u32,
            )
        };
        if ok_standard == 0 {
            return Err(win_error("Cannot read source file type and size"));
        }
        // SAFETY: same as above for FILE_ATTRIBUTE_TAG_INFO.
        let ok_attributes = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                FILE_ATTRIBUTE_TAG_INFO_CLASS,
                (&mut attributes as *mut FileAttributeTagInfo).cast(),
                std::mem::size_of::<FileAttributeTagInfo>() as u32,
            )
        };
        if ok_attributes == 0 {
            return Err(win_error("Cannot read source reparse-point status"));
        }
        if standard.end_of_file < 0 {
            return Err("Windows returned a negative source file size".into());
        }
        Ok((
            identity,
            standard.directory != 0,
            standard.end_of_file as u64,
            attributes.attributes,
        ))
    }

    fn identity_from_file_id(information: &FileIdInfo) -> Result<FileIdentity, String> {
        if information.file_id.iter().all(|byte| *byte == 0) {
            return Err("Source filesystem does not provide a usable 128-bit file identity".into());
        }
        Ok(FileIdentity {
            volume_serial: format!("{:016x}", information.volume_serial),
            file_id: information
                .file_id
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        })
    }

    fn append_path(root: &str, relative: &str) -> String {
        if relative.is_empty() {
            root.to_owned()
        } else if root.ends_with('\\') {
            format!("{root}{relative}")
        } else {
            format!("{root}\\{relative}")
        }
    }

    fn capture_snapshot_impl(root: &Path) -> Result<SourceIdentitySnapshot, String> {
        let root_path = absolute_extended(root)?;
        let (volume_root, components) = split_volume_root(&root_path)?;
        let mut ancestor_locks = Vec::new();
        let mut ancestors = Vec::new();
        let mut current = volume_root;
        let (root_handle, root_identity, root_reparse) =
            open_ancestor(&current, components.is_empty())?;
        ancestors.push(AncestorIdentity {
            path: current.clone(),
            identity: root_identity,
            is_reparse_point: root_reparse,
        });
        ancestor_locks.push(root_handle);
        let component_count = components.len();
        for (index, component) in components.into_iter().enumerate() {
            current.push_str(&component);
            current.push('\\');
            let (handle, identity, is_reparse) =
                open_ancestor(&current, index + 1 == component_count)?;
            ancestors.push(AncestorIdentity {
                path: current.clone(),
                identity,
                is_reparse_point: is_reparse,
            });
            ancestor_locks.push(handle);
        }
        if !same_path(&current, &root_path) {
            return Err("Source path did not resolve to the captured root".into());
        }
        let mut entries = BTreeMap::new();
        let mut directory_locks = Vec::new();
        walk_tree(&root_path, "", &mut entries, &mut directory_locks)?;
        drop(directory_locks);
        drop(ancestor_locks);
        Ok(SourceIdentitySnapshot {
            root_path,
            ancestors,
            entries,
        })
    }

    fn walk_tree(
        directory: &str,
        relative_directory: &str,
        entries: &mut BTreeMap<String, SnapshotEntry>,
        directory_locks: &mut Vec<Handle>,
    ) -> Result<(), String> {
        let mut children = fs::read_dir(directory)
            .map_err(|e| format!("Cannot enumerate source directory {directory}: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Cannot enumerate source directory {directory}: {e}"))?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let name = child
                .file_name()
                .to_str()
                .ok_or("Source contains a path name that is not valid Unicode")?
                .to_owned();
            let relative = if relative_directory.is_empty() {
                name
            } else {
                format!("{relative_directory}\\{name}")
            };
            validate_relative_path(&relative)?;
            let child_path = append_path(
                directory,
                &relative[relative_directory.len()..].trim_start_matches('\\'),
            );
            let metadata_kind = child
                .file_type()
                .map_err(|e| format!("Cannot inspect source entry {relative}: {e}"))?;
            if metadata_kind.is_symlink() {
                return Err(format!(
                    "Source contains a link or reparse point: {relative}"
                ));
            }
            let is_directory = metadata_kind.is_dir();
            let handle = open(&child_path, is_directory)?;
            let (identity, actual_directory, length, _) = query(&handle)?;
            if actual_directory != is_directory {
                return Err(format!("Source path type changed: {relative}"));
            }
            entries.insert(
                relative.clone(),
                SnapshotEntry {
                    identity,
                    is_directory,
                    length: if is_directory { 0 } else { length },
                },
            );
            if is_directory {
                directory_locks.push(handle);
                walk_tree(&child_path, &relative, entries, directory_locks)?;
            }
        }
        Ok(())
    }

    fn prepare_source_impl(
        source_root: &Path,
        manifest_entries: &[ManifestEntry],
        snapshot: &SourceIdentitySnapshot,
    ) -> Result<PreparedSource, String> {
        validate_manifest(manifest_entries, snapshot)?;
        let root_path = absolute_extended(source_root)?;
        if !same_path(&root_path, &snapshot.root_path) {
            return Err("The source root changed after manifest creation".into());
        }
        let mut ancestor_locks = Vec::new();
        for (index, ancestor) in snapshot.ancestors.iter().enumerate() {
            let (handle, identity, is_reparse) =
                open_ancestor(&ancestor.path, index + 1 == snapshot.ancestors.len())?;
            if identity != ancestor.identity || is_reparse != ancestor.is_reparse_point {
                return Err(format!("Source ancestry changed: {}", ancestor.path));
            }
            ancestor_locks.push(handle);
        }
        let mut expected: BTreeMap<String, &ManifestEntry> = manifest_entries
            .iter()
            .map(|entry| (entry.relative_path.clone(), entry))
            .collect();
        let mut files = BTreeMap::new();
        let mut directory_locks = Vec::new();
        verify_tree(
            &snapshot.root_path,
            "",
            &mut expected,
            snapshot,
            &mut files,
            &mut directory_locks,
        )?;
        if !expected.is_empty() {
            return Err("Source tree changed after manifest creation".into());
        }
        Ok(PreparedSource {
            files,
            _directory_locks: directory_locks,
            _ancestor_locks: ancestor_locks,
        })
    }

    fn verify_tree(
        directory: &str,
        relative_directory: &str,
        expected: &mut BTreeMap<String, &ManifestEntry>,
        snapshot: &SourceIdentitySnapshot,
        files: &mut BTreeMap<String, Handle>,
        directory_locks: &mut Vec<Handle>,
    ) -> Result<(), String> {
        let mut children = fs::read_dir(directory)
            .map_err(|e| format!("Cannot enumerate source directory {directory}: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Cannot enumerate source directory {directory}: {e}"))?;
        children.sort_by_key(|entry| entry.file_name());
        let expected_children: BTreeSet<_> = snapshot
            .entries
            .keys()
            .filter(|path| {
                let parent = path
                    .rsplit_once('\\')
                    .map(|(parent, _)| parent)
                    .unwrap_or("");
                parent == relative_directory
            })
            .cloned()
            .collect();
        let mut actual_names = BTreeSet::new();
        for child in children {
            let name = child
                .file_name()
                .to_str()
                .ok_or("Source contains a path name that is not valid Unicode")?
                .to_owned();
            let relative = if relative_directory.is_empty() {
                name
            } else {
                format!("{relative_directory}\\{name}")
            };
            validate_relative_path(&relative)?;
            actual_names.insert(relative.clone());
            let entry = expected
                .remove(&relative)
                .ok_or_else(|| format!("Source tree contains an unapproved path: {relative}"))?;
            let child_path = append_path(
                directory,
                &relative[relative_directory.len()..].trim_start_matches('\\'),
            );
            let file_type = child
                .file_type()
                .map_err(|e| format!("Cannot inspect source entry {relative}: {e}"))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "Source contains a link or reparse point: {relative}"
                ));
            }
            let handle = open(&child_path, entry.is_directory)?;
            let (identity, is_directory, length, _) = query(&handle)?;
            let expected_identity = snapshot
                .entries
                .get(&relative)
                .ok_or("Missing source identity entry")?;
            if identity != expected_identity.identity || is_directory != entry.is_directory {
                return Err(format!("Source identity changed: {relative}"));
            }
            if !is_directory {
                if length != entry.length {
                    return Err(format!("Source size changed: {relative}"));
                }
                let actual_hash = hash_handle(&handle)?;
                if !actual_hash.eq_ignore_ascii_case(&entry.sha256) {
                    return Err(format!("Source content changed: {relative}"));
                }
                files.insert(relative, handle);
            } else {
                directory_locks.push(handle);
                verify_tree(
                    &child_path,
                    &relative,
                    expected,
                    snapshot,
                    files,
                    directory_locks,
                )?;
            }
        }
        if actual_names != expected_children {
            return Err(format!(
                "Source directory contents changed: {relative_directory}"
            ));
        }
        Ok(())
    }

    fn hash_handle(handle: &Handle) -> Result<String, String> {
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let mut read = 0_u32;
            // SAFETY: `buffer` is writable for its full length and no overlapped I/O is used.
            let ok = unsafe {
                ReadFile(
                    handle.raw(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut read,
                    null_mut(),
                )
            };
            if ok == 0 {
                return Err(win_error("Cannot read source file"));
            }
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read as usize]);
        }
        seek_to_start(handle)?;
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn seek_to_start(handle: &Handle) -> Result<(), String> {
        // SAFETY: the handle is a synchronous file handle and FILE_BEGIN is valid.
        if unsafe { SetFilePointerEx(handle.raw(), 0, null_mut(), FILE_BEGIN) } == 0 {
            Err(win_error("Cannot rewind source file"))
        } else {
            Ok(())
        }
    }

    fn copy_prepared_impl(
        prepared: &PreparedSource,
        target_root: &Path,
        entries: &[ManifestEntry],
    ) -> Result<(), String> {
        let root = absolute_extended(target_root)?;
        let root_handle =
            open(&root, true).map_err(|_| "Copy target root is unavailable or unsafe")?;
        let (_, is_directory, _, attributes) = query(&root_handle)?;
        if !is_directory || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("Copy target root must be a plain directory".into());
        }
        let mut target_directory_locks = vec![root_handle];
        let mut directories: Vec<_> = entries.iter().filter(|entry| entry.is_directory).collect();
        directories.sort_by_key(|entry| {
            (
                entry.relative_path.matches('\\').count(),
                entry.relative_path.clone(),
            )
        });
        for entry in directories {
            let path = append_path(&root, &entry.relative_path);
            // SAFETY: path is NUL-terminated; this creates only the exact child path.
            if unsafe { CreateDirectoryW(wide(&path).as_ptr(), null()) } == 0 {
                // SAFETY: GetLastError has no preconditions.
                let error = unsafe { GetLastError() };
                if error != ERROR_ALREADY_EXISTS {
                    return Err(win_error(&format!(
                        "Cannot create target directory {}",
                        entry.relative_path
                    )));
                }
            }
            let handle = open(&path, true)
                .map_err(|_| format!("Target directory is not plain: {}", entry.relative_path))?;
            let (_, is_directory, _, attributes) = query(&handle)?;
            if !is_directory || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(format!(
                    "Target directory became unsafe: {}",
                    entry.relative_path
                ));
            }
            target_directory_locks.push(handle);
        }
        for entry in entries.iter().filter(|entry| !entry.is_directory) {
            let path = append_path(&root, &entry.relative_path);
            // SAFETY: `wide` produces a NUL-terminated UTF-16 path.
            let attrs = unsafe { GetFileAttributesW(wide(&path).as_ptr()) };
            if attrs != INVALID_FILE_ATTRIBUTES {
                return Err(format!(
                    "Refusing to overwrite target path: {}",
                    entry.relative_path
                ));
            }
            // SAFETY: GetLastError has no preconditions and is read immediately
            // after the failed GetFileAttributesW call on this thread.
            let error = unsafe { GetLastError() };
            if error != ERROR_FILE_NOT_FOUND && error != ERROR_PATH_NOT_FOUND {
                return Err(win_error(&format!(
                    "Cannot inspect target path {}",
                    entry.relative_path
                )));
            }
        }
        for entry in entries.iter().filter(|entry| !entry.is_directory) {
            let source = prepared
                .files
                .get(&entry.relative_path)
                .ok_or("Preflight source handle is missing")?;
            let path = append_path(&root, &entry.relative_path);
            let name = wide(&path);
            // CREATE_NEW guarantees existing destination files are never replaced.
            // SAFETY: path is NUL-terminated, and the returned handle is owned below.
            let raw = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                    0,
                    null(),
                    CREATE_NEW,
                    FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                    null_mut(),
                )
            };
            if raw == INVALID_HANDLE_VALUE {
                return Err(win_error(&format!(
                    "Cannot create target file {}",
                    entry.relative_path
                )));
            }
            let target = Handle(raw);
            transfer(source, &target, entry.length)?;
        }
        drop(target_directory_locks);
        Ok(())
    }

    fn transfer(source: &Handle, target: &Handle, expected_length: u64) -> Result<(), String> {
        seek_to_start(source)?;
        let mut buffer = vec![0_u8; 1024 * 1024];
        let mut total = 0_u64;
        loop {
            let mut read = 0_u32;
            // SAFETY: `buffer` is writable for its full length; both handles are synchronous.
            let ok = unsafe {
                ReadFile(
                    source.raw(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut read,
                    null_mut(),
                )
            };
            if ok == 0 {
                return Err(win_error("Cannot read preflighted source file"));
            }
            if read == 0 {
                break;
            }
            let mut offset = 0;
            while offset < read as usize {
                let mut written = 0_u32;
                // SAFETY: the source slice and target handle are valid for this synchronous write.
                let ok = unsafe {
                    WriteFile(
                        target.raw(),
                        buffer[offset..read as usize].as_ptr().cast(),
                        (read as usize - offset) as u32,
                        &mut written,
                        null_mut(),
                    )
                };
                if ok == 0 || written == 0 {
                    return Err(win_error("Cannot write target file"));
                }
                offset += written as usize;
            }
            total = total
                .checked_add(read as u64)
                .ok_or("Copied file size overflow")?;
        }
        if total != expected_length {
            return Err("Preflighted source size changed during copy".into());
        }
        Ok(())
    }

    fn same_path(left: &str, right: &str) -> bool {
        left.trim_end_matches('\\')
            .eq_ignore_ascii_case(right.trim_end_matches('\\'))
    }

    pub(super) fn capture_snapshot_inner(root: &Path) -> Result<SourceIdentitySnapshot, String> {
        capture_snapshot_impl(root)
    }

    pub(super) fn copy_plan_inner(plan_path: &Path, expected_hash: &str) -> Result<(), String> {
        let (plan, snapshot) = validate_copy_plan(plan_path, expected_hash)?;
        let prepared = prepare_source_impl(
            Path::new(&plan.source_root),
            &plan.manifest.entries,
            &snapshot,
        )?;
        validate_volume_guid_root(&plan.target_root)?;
        copy_prepared_impl(
            &prepared,
            Path::new(&plan.target_root),
            &plan.manifest.entries,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;
        use std::process::Command;

        fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            let target = temp.path().join("target");
            fs::create_dir_all(source.join("nested")).unwrap();
            fs::create_dir_all(&target).unwrap();
            fs::write(
                source.join("nested\\payload.bin"),
                b"approved identical bytes",
            )
            .unwrap();
            (temp, source, target)
        }

        fn manifest_from_snapshot(snapshot: &SourceIdentitySnapshot) -> Vec<ManifestEntry> {
            snapshot
                .entries
                .iter()
                .map(|(path, entry)| ManifestEntry {
                    relative_path: path.clone(),
                    is_directory: entry.is_directory,
                    length: entry.length,
                    sha256: if entry.is_directory {
                        String::new()
                    } else {
                        format!("{:x}", Sha256::digest(b"approved identical bytes"))
                    },
                })
                .collect()
        }

        fn authenticated_copy_plan(
            temp: &tempfile::TempDir,
            source: &Path,
            target: &Path,
            entries: &[ManifestEntry],
            snapshot: &SourceIdentitySnapshot,
        ) -> (PathBuf, String) {
            let entries: Vec<_> = entries
                .iter()
                .map(|entry| {
                    serde_json::json!({
                        "RelativePath": entry.relative_path,
                        "IsDirectory": entry.is_directory,
                        "Length": entry.length,
                        "Sha256": entry.sha256
                    })
                })
                .collect();
            let plan = serde_json::json!({
                "SourceRoot": source,
                "TargetRoot": target,
                "Manifest": {
                    "Entries": entries,
                    "SourceIdentity": snapshot
                }
            });
            let bytes = serde_json::to_vec(&plan).unwrap();
            let hash = format!("{:x}", Sha256::digest(&bytes));
            let path = temp.path().join("copy-plan.json");
            fs::write(&path, bytes).unwrap();
            (path, hash)
        }

        #[test]
        fn append_path_adds_a_separator_only_when_the_root_needs_one() {
            assert_eq!(
                append_path(r"\\?\C:\source", r"nested\payload.bin"),
                r"\\?\C:\source\nested\payload.bin"
            );
            assert_eq!(
                append_path(
                    r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\",
                    "file.bin"
                ),
                r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\file.bin"
            );
        }

        #[test]
        fn file_identity_requires_a_nonzero_128_bit_id() {
            let information = FileIdInfo {
                volume_serial: 0x1234_abcd,
                file_id: [0; 16],
            };
            assert!(identity_from_file_id(&information).is_err());
            let information = FileIdInfo {
                volume_serial: 0x1234_abcd,
                file_id: [0x5a; 16],
            };
            assert_eq!(
                identity_from_file_id(&information).unwrap(),
                FileIdentity {
                    volume_serial: "000000001234abcd".to_owned(),
                    file_id: "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a".to_owned(),
                }
            );
        }

        fn create_directory_junction(link: &Path, target: &Path) {
            let output = Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .expect("cmd.exe is available on Windows");
            assert!(
                output.status.success(),
                "mklink /J failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        #[test]
        fn source_root_beneath_a_junction_ancestor_fails_closed() {
            let temp = tempfile::tempdir().unwrap();
            let real_parent = temp.path().join("real-parent");
            let source = real_parent.join("source");
            let linked_parent = temp.path().join("linked-parent");
            fs::create_dir_all(source.join("nested")).unwrap();
            fs::write(
                source.join("nested\\payload.bin"),
                b"approved identical bytes",
            )
            .unwrap();
            create_directory_junction(&linked_parent, &real_parent);

            let error = capture_snapshot_impl(&linked_parent.join("source")).unwrap_err();
            assert!(error.contains("reparse point"), "{error}");
        }

        fn create_file_symlink(link: &Path, target: &Path) {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn CreateSymbolicLinkW(link: *const u16, target: *const u16, flags: u32) -> u8;
            }
            let link_wide: Vec<u16> = link.as_os_str().encode_wide().chain(Some(0)).collect();
            let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
            // Windows 10+ permits unprivileged symbolic links when this flag is set.
            // SAFETY: both paths are NUL-terminated and output is a fresh fixture path.
            let created =
                unsafe { CreateSymbolicLinkW(link_wide.as_ptr(), target_wide.as_ptr(), 0x2) };
            // SAFETY: GetLastError has no preconditions and is read immediately
            // after the failed symbolic-link call on this thread.
            assert_ne!(created, 0, "CreateSymbolicLinkW failed: {}", unsafe {
                GetLastError()
            });
        }

        #[test]
        fn file_to_link_with_identical_bytes_fails_before_any_target_file_write() {
            let (_temp, source, target) = fixture();
            let snapshot = capture_snapshot_impl(&source).unwrap();
            let manifest = manifest_from_snapshot(&snapshot);
            let (plan_path, expected_hash) =
                authenticated_copy_plan(&_temp, &source, &target, &manifest, &snapshot);
            let original = source.join("nested\\payload.bin");
            let backup = _temp.path().join("payload.original");
            let alternate = _temp.path().join("alternate.bin");
            fs::write(&alternate, b"approved identical bytes").unwrap();
            fs::rename(&original, &backup).unwrap();
            create_file_symlink(&original, &alternate);
            let error = copy_plan_inner(&plan_path, &expected_hash).unwrap_err();
            assert!(error.contains("reparse point"), "{error}");
            assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        }

        #[test]
        fn directory_to_junction_with_identical_bytes_fails_before_any_target_file_write() {
            let (_temp, source, target) = fixture();
            let snapshot = capture_snapshot_impl(&source).unwrap();
            let manifest = manifest_from_snapshot(&snapshot);
            let (plan_path, expected_hash) =
                authenticated_copy_plan(&_temp, &source, &target, &manifest, &snapshot);
            let original = source.join("nested");
            let moved = _temp.path().join("nested.original");
            let alternate = _temp.path().join("alternate");
            fs::create_dir(&alternate).unwrap();
            fs::write(alternate.join("payload.bin"), b"approved identical bytes").unwrap();
            fs::rename(&original, &moved).unwrap();
            create_directory_junction(&original, &alternate);
            let error = copy_plan_inner(&plan_path, &expected_hash).unwrap_err();
            assert!(error.contains("reparse point"), "{error}");
            assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        }

        #[test]
        fn locked_source_ancestry_rejects_toctou_rename_then_copies_original_bytes() {
            let (_temp, source, target) = fixture();
            let snapshot = capture_snapshot_impl(&source).unwrap();
            let manifest = manifest_from_snapshot(&snapshot);
            let prepared = prepare_source_impl(&source, &manifest, &snapshot).unwrap();
            let nested = source.join("nested");
            let moved = source.join("nested.moved");
            assert!(fs::rename(&nested, &moved).is_err());
            copy_prepared_impl(&prepared, &target, &manifest).unwrap();
            assert_eq!(
                fs::read(target.join("nested\\payload.bin")).unwrap(),
                b"approved identical bytes"
            );
        }

        #[test]
        fn content_change_with_stable_file_id_fails_during_preflight_before_target_writes() {
            let (_temp, source, target) = fixture();
            let snapshot = capture_snapshot_impl(&source).unwrap();
            let manifest = manifest_from_snapshot(&snapshot);
            fs::write(
                source.join("nested\\payload.bin"),
                b"rejected different bytes",
            )
            .unwrap();
            assert!(prepare_source_impl(&source, &manifest, &snapshot).is_err());
            assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        }
    }
}

#[cfg(windows)]
pub fn capture_snapshot(root: &Path) -> Result<SourceIdentitySnapshot, String> {
    windows::capture_snapshot_inner(root)
}

#[cfg(not(windows))]
pub fn capture_snapshot(_root: &Path) -> Result<SourceIdentitySnapshot, String> {
    Err("Source identity capture requires Windows".into())
}
