//! The `.battleplan` container READER (HOI4REPLAY_CONTAINER_SPEC.md).
//!
//! Opens a portable recording, validates its manifest + integrity + identity, and
//! serves the snapshot series through the SAME shapes the directory store uses
//! ([`crate::Index`], [`crate::Snapshot`]), so the app opens a `.battleplan` with
//! zero changes elsewhere ([`crate::Store`] dispatches to this transparently).
//!
//! Verification is STAGED (spec §4). Open validates ZIP structure and caps,
//! the manifest digest/signature, versioned path rules, and the required
//! `index.json`/`timeline.json` spine. Per-entry CRC, size, and SHA-256 checks
//! happen lazily in the same bounded pass that supplies a requested payload.
//! No bytes cross an exposure boundary before every integrity check matches.

use crate::canon::manifest_canonical_bytes;
use crate::sha256::{to_hex, Sha256};
use crate::stream_verify::FileBackedArchive;
use crate::zip::{
    legacy_path_key, portable_path_key, validate_payload_path, validate_payload_path_legacy_v1,
    ZipArchive, MAX_CONTAINER_ENTRY_BYTES, STREAMING_MAX_CONTAINER_ENTRY_BYTES, ZIP_METHOD_STORE,
};
use crate::{Index, SegmentGraph, Snapshot};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use unicode_normalization::UnicodeNormalization;

/// Highest container version this reader understands. Bumped 1 -> 2 for stitched
/// (reload/relaunch) archives, which carry the segment/timeline graph
/// (`STITCHING_DESIGN.md` §9). A reader still rejects any version ABOVE this, and
/// a pre-bump (v1-max) reader rejects a v2 stitched archive rather than silently
/// hiding its rollback graph.
pub const MAX_CONTAINER_VERSION: u64 = 2;
/// The container version at/above which the stitched segment feature is mandatory.
const STITCHED_CONTAINER_VERSION: u64 = 2;
/// The required feature a stitched (v2) container must declare (§9).
const FEATURE_TIMELINE_SEGMENTS_V1: &str = "timeline_segments_v1";
const FEATURE_PORTABLE_CONTENT_MANIFEST_V2: &str = "portable_content_manifest_v2";
const STITCH_INDEX_SCHEMA: &str = "stitched-main-v1";
const STITCH_ENTRY_ORDER: &str = "lexicographic-path-manifest-last-v1";
/// Zip-bomb structural caps (spec §4.1). Generous for a real campaign yet bounded.
pub(crate) const MAX_ENTRIES: usize = 2_000_000;
pub(crate) const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024 * 1024; // 64 GiB total
static SPOOL_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Canonical digest domain shared by the stitch planner and the v2 reader. The
/// serialized graph is authoritative; only the digest field itself is removed
/// from the preimage. This makes open-time recomputation independent of any
/// planner object while binding every serialized required input and campaign key.
pub fn canonical_stitch_input_digest(
    game_id: &str,
    graph: &Value,
    segment_indexes: &[Value],
) -> String {
    let mut serialized_graph = graph.clone();
    if let Some(object) = serialized_graph.as_object_mut() {
        object.remove("stitch_input_digest");
    }
    let preimage = json!({
        "game_id": game_id,
        "container_version": STITCHED_CONTAINER_VERSION,
        "required_feature": FEATURE_TIMELINE_SEGMENTS_V1,
        "index_schema": STITCH_INDEX_SCHEMA,
        "serialized_stitch_inputs": serialized_graph,
        "serialized_segment_indexes": segment_indexes,
    });
    let mut canonical = String::new();
    crate::canon::write_canonical(&preimage, &mut canonical);
    Sha256::hex(canonical.as_bytes())
}

/// Lexically canonicalize a Windows source path without touching the filesystem.
/// The stitcher records already-observed files, so filesystem canonicalization
/// would introduce time-of-check races and would fail for portable containers.
pub fn normalize_stitch_source_path(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("source path is empty or contains NUL".into());
    }
    let mut normalized = path
        .replace('\\', "/")
        .nfc()
        .collect::<String>()
        .to_lowercase();
    if normalized.starts_with("//?/unc/") {
        normalized = format!("//{}", &normalized[8..]);
    } else if normalized.starts_with("//?/") {
        normalized = normalized[4..].to_string();
    }

    enum Prefix {
        Relative,
        Root,
        Drive(String),
        Unc,
    }
    let (prefix, rest, protected_components) = if normalized.starts_with("//") {
        (Prefix::Unc, normalized.trim_start_matches('/'), 2usize)
    } else if normalized.as_bytes().get(1) == Some(&b':') {
        if normalized.len() < 3 || normalized.as_bytes()[2] != b'/' {
            return Err("drive-relative source path is ambiguous".into());
        }
        (
            Prefix::Drive(normalized[..3].to_string()),
            &normalized[3..],
            0,
        )
    } else if let Some(rest) = normalized.strip_prefix('/') {
        (Prefix::Root, rest, 0)
    } else {
        (Prefix::Relative, normalized.as_str(), 0)
    };

    let mut components: Vec<&str> = Vec::new();
    for component in rest.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.len() <= protected_components {
                    return Err("source path escapes its lexical root".into());
                }
                components.pop();
            }
            value => components.push(value),
        }
    }
    if matches!(prefix, Prefix::Unc) && components.len() < 2 {
        return Err("UNC source path lacks server/share".into());
    }
    if components.is_empty() && matches!(prefix, Prefix::Relative) {
        return Err("source path has no canonical components".into());
    }
    let joined = components.join("/");
    Ok(match prefix {
        Prefix::Relative => joined,
        Prefix::Root => format!("/{joined}"),
        Prefix::Drive(drive) => format!("{drive}{joined}"),
        Prefix::Unc => format!("//{joined}"),
    })
}

#[derive(Debug)]
pub enum ContainerError {
    Io(std::io::Error),
    Zip(crate::zip::ZipError),
    Json(serde_json::Error),
    /// A finalized manifest is missing — unfinalized / recovery-only file.
    NoManifest,
    /// A semantic/integrity/identity rejection, with a human-readable reason.
    Reject(String),
    /// Retained payload bytes disagree with the manifest digest that names them.
    PayloadIntegrity(String),
    /// The manifest's mod identity disagrees with snapshot provenance. Typed
    /// (rather than a [`ContainerError::Reject`] string) so downstream layers
    /// can classify a modset mismatch without matching on error text.
    ModsetMismatch {
        /// `manifest.mod_identity.modset_id`
        manifest: String,
        /// the first snapshot's `provenance.mod.modset_id`
        snapshot: String,
    },
}

impl std::fmt::Display for ContainerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerError::Io(e) => write!(f, "io: {e}"),
            ContainerError::Zip(e) => write!(f, "container zip: {e}"),
            ContainerError::Json(e) => write!(f, "container json: {e}"),
            ContainerError::NoManifest => write!(
                f,
                "no manifest.json: unfinalized/recovery-only .battleplan (refused by the normal reader)"
            ),
            ContainerError::Reject(s) | ContainerError::PayloadIntegrity(s) => {
                write!(f, "container rejected: {s}")
            }
            // Renders the EXACT text the pre-typed Reject produced, so every
            // existing message contract (fixtures, product surfaces) holds.
            ContainerError::ModsetMismatch { manifest, snapshot } => write!(
                f,
                "container rejected: manifest modset_id {manifest:?} disagrees with snapshot provenance.mod {snapshot:?}"
            ),
        }
    }
}
impl std::error::Error for ContainerError {}
impl From<std::io::Error> for ContainerError {
    fn from(e: std::io::Error) -> Self {
        ContainerError::Io(e)
    }
}
impl From<crate::zip::ZipError> for ContainerError {
    fn from(e: crate::zip::ZipError) -> Self {
        ContainerError::Zip(e)
    }
}
impl From<serde_json::Error> for ContainerError {
    fn from(e: serde_json::Error) -> Self {
        ContainerError::Json(e)
    }
}

/// What identity the caller expects (e.g. the reader-pinned build, the app's known
/// container version). All optional: `None` skips that check. The container always
/// self-checks its own digest + internal consistency regardless.
#[derive(Debug, Clone, Default)]
pub struct ExpectedIdentity {
    /// Reject if the recorded `build.exe_sha256` != this (the reader-pinned build).
    pub build_exe_sha256: Option<String>,
    /// Reject if the recorded `schema_version` != this.
    pub schema_version: Option<String>,
    /// Reject if the recorded `mod_identity.modset_id` != this.
    pub modset_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryVerificationState {
    Pending,
    Verified,
}

enum ContainerArchive {
    Memory(ZipArchive),
    File(FileBackedArchive),
}

struct ArchiveEntry<'a> {
    name: &'a str,
    uncomp_size: u64,
    method: u16,
    local_header_off: u64,
}

impl ContainerArchive {
    fn len(&self) -> usize {
        match self {
            ContainerArchive::Memory(archive) => archive.entries().len(),
            ContainerArchive::File(archive) => archive.entries().len(),
        }
    }

    fn entry(&self, index: usize) -> Option<ArchiveEntry<'_>> {
        match self {
            ContainerArchive::Memory(archive) => {
                archive.entries().get(index).map(|entry| ArchiveEntry {
                    name: &entry.name,
                    uncomp_size: entry.uncomp_size,
                    method: entry.method,
                    local_header_off: entry.local_header_off,
                })
            }
            ContainerArchive::File(archive) => {
                archive.entries().get(index).map(|entry| ArchiveEntry {
                    name: &entry.name,
                    uncomp_size: entry.uncomp_size,
                    method: entry.method,
                    local_header_off: entry.local_header_off,
                })
            }
        }
    }

    /// Central-directory metadata for every entry, in archive order — the
    /// variant-neutral view the v2 stitched validators iterate.
    fn entries_meta(&self) -> Vec<ArchiveEntry<'_>> {
        (0..self.len()).filter_map(|i| self.entry(i)).collect()
    }

    fn validate_local_headers(&self) -> Result<(), ContainerError> {
        match self {
            ContainerArchive::Memory(archive) => {
                archive.validate_local_headers().map_err(Into::into)
            }
            ContainerArchive::File(archive) => {
                archive.validate_local_headers().map_err(file_archive_error)
            }
        }
    }

    fn validate_portable_paths(&self, unicode_v2: bool) -> Result<(), ContainerError> {
        let mut collision_keys = HashSet::with_capacity(self.len());
        for index in 0..self.len() {
            let entry = self.entry(index).ok_or_else(|| {
                ContainerError::Reject(format!("container entry index {index} is out of range"))
            })?;
            if entry.name != "manifest.json" {
                if unicode_v2 {
                    validate_payload_path(entry.name)?;
                } else {
                    validate_payload_path_legacy_v1(entry.name)?;
                }
            }
            let key = if unicode_v2 {
                portable_path_key(entry.name)?
            } else {
                legacy_path_key(entry.name)?
            };
            if !collision_keys.insert(key) {
                return Err(ContainerError::Reject(format!(
                    "duplicate or portable-path-colliding zip entry {}",
                    entry.name
                )));
            }
        }
        Ok(())
    }

    fn read_entry(
        &self,
        index: usize,
        calculate_sha256: bool,
    ) -> Result<(Vec<u8>, Option<String>), ContainerError> {
        match self {
            ContainerArchive::Memory(archive) => {
                let entry = archive.entries().get(index).ok_or_else(|| {
                    ContainerError::Reject(format!("container entry index {index} is out of range"))
                })?;
                let bytes = archive.read_entry(entry)?;
                let sha256 = calculate_sha256.then(|| to_hex(&Sha256::digest(&bytes)));
                Ok((bytes, sha256))
            }
            ContainerArchive::File(archive) => archive
                .read_entry(index, calculate_sha256)
                .map_err(file_archive_entry_error),
        }
    }

    fn copy_entry_to_writer<W: std::io::Write>(
        &self,
        index: usize,
        writer: &mut W,
        maximum_uncompressed_bytes: u64,
    ) -> Result<(String, u64), ContainerError> {
        match self {
            ContainerArchive::Memory(_) => Err(ContainerError::Reject(
                "bounded entry spooling requires a file-backed container".to_string(),
            )),
            ContainerArchive::File(archive) => {
                let (digest, len) = archive
                    .copy_entry_to_writer(index, writer, true, maximum_uncompressed_bytes)
                    .map_err(file_archive_entry_error)?;
                let digest = digest.ok_or_else(|| {
                    ContainerError::Reject(
                        "file-backed archive omitted requested spool SHA-256".to_string(),
                    )
                })?;
                Ok((digest, len))
            }
        }
    }

    fn requires_sha256_each_read(&self) -> bool {
        matches!(self, ContainerArchive::File(_))
    }

    fn whole_file_receipt(&self) -> Result<(u64, String), ContainerError> {
        match self {
            ContainerArchive::Memory(_) => Err(ContainerError::Reject(
                "whole-file receipts require a retained file-backed container".to_string(),
            )),
            ContainerArchive::File(archive) => {
                archive.whole_file_receipt().map_err(file_archive_error)
            }
        }
    }

    fn entry_size_cap(&self) -> u64 {
        match self {
            ContainerArchive::Memory(_) => MAX_CONTAINER_ENTRY_BYTES,
            ContainerArchive::File(_) => STREAMING_MAX_CONTAINER_ENTRY_BYTES,
        }
    }
}

fn file_archive_error(error: String) -> ContainerError {
    ContainerError::Reject(format!("file-backed zip: {error}"))
}

fn file_archive_entry_error(error: crate::stream_verify::EntryFailure) -> ContainerError {
    let integrity = error.is_integrity();
    let message = format!("file-backed zip: {}", error.into_reason());
    if integrity {
        ContainerError::PayloadIntegrity(message)
    } else {
        ContainerError::Reject(message)
    }
}

fn open_retained_container_file(path: &Path) -> std::io::Result<File> {
    #[cfg(windows)]
    {
        use std::io::{Error, ErrorKind};
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use std::path::{Component, Prefix};

        // Permit other readers but deny any new writer or delete/rename handle
        // for the lifetime of the retained archive. A writer that predates this
        // open is still covered by the pre/post same-handle receipt equality.
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

        let unresolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut components = unresolved.components();
        let prefix = match components.next() {
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
            {
                prefix.as_os_str().to_os_string()
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "replay container must resolve to a local drive path; UNC, device, and network paths are refused",
                ))
            }
        };
        if !matches!(components.next(), Some(Component::RootDir)) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "replay container path has no rooted local-drive form",
            ));
        }
        // Normalize dot components lexically before opening anything. This does
        // not follow the filesystem (and therefore cannot hide a reparse
        // point); every resulting component is still opened and inspected with
        // OPEN_REPARSE_POINT below.
        let mut absolute = PathBuf::from(&prefix);
        absolute.push("\\");
        let mut depth = 0usize;
        for component in components {
            match component {
                Component::Normal(name) => {
                    absolute.push(name);
                    depth += 1;
                }
                Component::CurDir => {}
                Component::ParentDir if depth > 0 => {
                    absolute.pop();
                    depth -= 1;
                }
                Component::ParentDir => {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        "replay container path escapes its local-drive root",
                    ));
                }
                Component::Prefix(_) | Component::RootDir => {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        "replay container path contains an invalid rooted component",
                    ));
                }
            }
        }

        // Retain every existing parent without delete sharing until the final
        // object is open. OPEN_REPARSE_POINT makes each component inspection
        // no-follow, preventing junction/symlink ancestry and pathname rebinding.
        let parent = absolute.parent().ok_or_else(|| {
            Error::new(ErrorKind::InvalidInput, "replay container has no parent directory")
        })?;
        let mut retained = Vec::new();
        let mut cursor = PathBuf::from(prefix);
        cursor.push("\\");
        for component in parent.components().skip(2) {
            cursor.push(component.as_os_str());
            let directory = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&cursor)?;
            let metadata = directory.metadata()?;
            if !metadata.is_dir()
                || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    format!(
                        "replay container ancestor is not a regular no-follow directory: {}",
                        cursor.display()
                    ),
                ));
            }
            retained.push(directory);
        }

        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&absolute)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "replay container is not a regular no-follow file: {}",
                    absolute.display()
                ),
            ));
        }
        drop(retained);
        Ok(file)
    }
    #[cfg(not(windows))]
    {
        File::open(path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SpoolFileIdentity {
    volume: u64,
    file: u64,
}

/// Retains the complete destination-directory chain while a spool file is
/// authenticated and published. Windows handles omit delete sharing, so no
/// accepted component can be renamed or replaced by a junction during the
/// later pathname-based hard-link call.
struct SpoolDirectoryGuard {
    destination: PathBuf,
    parent: PathBuf,
    #[cfg(windows)]
    _handles: Vec<File>,
}

impl SpoolDirectoryGuard {
    fn acquire(destination: &Path) -> Result<Self, ContainerError> {
        use std::path::Component;

        if destination.as_os_str().is_empty()
            || destination
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        {
            return Err(ContainerError::Reject(format!(
                "transactional spool destination is empty or contains . or ..: {}",
                destination.display()
            )));
        }
        let destination = if destination.is_absolute() {
            destination.to_path_buf()
        } else {
            std::env::current_dir()?.join(destination)
        };
        let parent = destination
            .parent()
            .ok_or_else(|| {
                ContainerError::Reject(
                    "transactional spool destination has no parent directory".to_string(),
                )
            })?
            .to_path_buf();
        if destination.file_name().is_none() {
            return Err(ContainerError::Reject(
                "transactional spool destination has no file name".to_string(),
            ));
        }

        #[cfg(windows)]
        let handles = retain_windows_directory_chain(&parent).map_err(|error| {
            ContainerError::Reject(format!(
                "retain transactional spool directory chain {}: {error}",
                parent.display()
            ))
        })?;
        #[cfg(not(windows))]
        validate_spool_directory_chain(&parent)?;

        Ok(Self {
            destination,
            parent,
            #[cfg(windows)]
            _handles: handles,
        })
    }
}

#[cfg(not(windows))]
fn validate_spool_directory_chain(path: &Path) -> Result<(), ContainerError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if current.parent().is_none() {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ContainerError::Reject(format!(
                "transactional spool directory component is not a regular directory: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn retain_windows_directory_chain(path: &Path) -> Result<Vec<File>, ContainerError> {
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
    use std::path::Component;

    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    let mut current = PathBuf::new();
    let mut handles = Vec::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        let handle = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&current)?;
        let metadata = handle.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ContainerError::Reject(format!(
                "transactional spool directory component is not a non-reparse directory: {}",
                current.display()
            )));
        }
        handles.push(handle);
    }
    if handles.is_empty() {
        return Err(ContainerError::Reject(
            "transactional spool destination may not be a filesystem root".to_string(),
        ));
    }
    Ok(handles)
}

fn create_unique_spool_temp(
    guard: &SpoolDirectoryGuard,
) -> Result<(PathBuf, File, SpoolFileIdentity), ContainerError> {
    let destination_name = guard.destination.file_name().ok_or_else(|| {
        ContainerError::Reject("transactional spool destination has no file name".to_string())
    })?;
    for _ in 0..128 {
        let sequence = SPOOL_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(destination_name);
        temp_name.push(format!(
            ".authenticated-spool.{}.{}.part",
            std::process::id(),
            sequence
        ));
        let temp = guard.parent.join(temp_name);
        match open_new_locked_spool_file(&temp) {
            Ok(file) => {
                let identity = spool_file_identity(&file, &temp)?;
                return Ok((temp, file, identity));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(ContainerError::Reject(
        "could not allocate a unique authenticated spool temporary file".to_string(),
    ))
}

#[cfg(windows)]
fn open_new_locked_spool_file(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(windows))]
fn open_new_locked_spool_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(windows)]
fn open_spool_path_for_identity(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        // The retained writer already denies other writers. This inspection
        // handle must nevertheless share that writer's existing write access
        // or Windows rejects the reopen; delete sharing stays omitted.
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(windows))]
fn open_spool_path_for_identity(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
fn spool_file_identity(file: &File, path: &Path) -> Result<SpoolFileIdentity, ContainerError> {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut c_void,
            info: *mut ByHandleFileInformation,
        ) -> i32;
    }
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    let mut info = MaybeUninit::<ByHandleFileInformation>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) } == 0 {
        return Err(ContainerError::Io(std::io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    if info.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
        return Err(ContainerError::Reject(format!(
            "authenticated spool handle is not a regular non-reparse file: {}",
            path.display()
        )));
    }
    Ok(SpoolFileIdentity {
        volume: u64::from(info.volume_serial_number),
        file: (u64::from(info.file_index_high) << 32) | u64::from(info.file_index_low),
    })
}

#[cfg(unix)]
fn spool_file_identity(file: &File, path: &Path) -> Result<SpoolFileIdentity, ContainerError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(ContainerError::Reject(format!(
            "authenticated spool handle is not a regular file: {}",
            path.display()
        )));
    }
    Ok(SpoolFileIdentity {
        volume: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
fn spool_file_identity(file: &File, path: &Path) -> Result<SpoolFileIdentity, ContainerError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(ContainerError::Reject(format!(
            "authenticated spool handle is not a regular file: {}",
            path.display()
        )));
    }
    Ok(SpoolFileIdentity {
        volume: 0,
        file: metadata.len(),
    })
}

/// Rebind the spool temporary's pathname to the retained object: reopen the
/// name and refuse unless it still resolves to the identity captured when the
/// retained handle was created. Extracted verbatim from the publication
/// sequence so the refusal has a directly executable test on a platform whose
/// share modes make an end-to-end rebind impossible to stage.
///
/// Returns the inspection handle so the caller can keep it open for exactly as
/// long as the inline code did: it was a closure local, and therefore lived
/// across the hard link and any post-publication work.
fn bind_spool_temp_pathname_identity(
    temp: &Path,
    retained_identity: SpoolFileIdentity,
) -> Result<File, ContainerError> {
    let temp_path_handle = open_spool_path_for_identity(temp).map_err(|error| {
        ContainerError::Reject(format!(
            "reopen authenticated spool temporary for identity: {error}"
        ))
    })?;
    if spool_file_identity(&temp_path_handle, temp)? != retained_identity {
        return Err(ContainerError::Reject(
            "authenticated spool temporary pathname no longer names the retained file".to_string(),
        ));
    }
    Ok(temp_path_handle)
}

fn verify_spool_receipt_on_handle(
    file: &mut File,
    receipt: &SpoolEntryReceipt,
) -> Result<(), ContainerError> {
    let expected_len = receipt.uncompressed_len();
    if file.metadata()?.len() != expected_len {
        return Err(ContainerError::Reject(
            "authenticated spool length changed before publication".to_string(),
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut read_len = 0u64;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        read_len = read_len.checked_add(read as u64).ok_or_else(|| {
            ContainerError::Reject("authenticated spool length overflow".to_string())
        })?;
        digest.update(&buffer[..read]);
    }
    let observed_sha256 = to_hex(&digest.finalize());
    if read_len != expected_len || observed_sha256 != receipt.sha256() {
        return Err(ContainerError::Reject(format!(
            "authenticated spool changed before publication (len {read_len}, sha256 {observed_sha256})"
        )));
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

/// One manifest-authenticated entry, indexed by its exact container path.
struct EntryRec {
    archive_index: usize,
    bytes: u64,
    expected_sha256: String,
    /// Transitions exactly once after decompressed bytes match expected_sha256.
    sha256_state: EntryVerificationState,
}

impl EntryRec {
    fn require_sha256_verified(&self, path: &str) -> Result<(), ContainerError> {
        if self.sha256_state != EntryVerificationState::Verified {
            return Err(ContainerError::Reject(format!(
                "entry {path}: bytes reached the exposure boundary before SHA-256 verification"
            )));
        }
        Ok(())
    }
}

/// Public metadata for one manifest-listed payload entry. The bytes are not
/// included here; callers must use [`Container::read_entry_bytes`] so integrity
/// verification stays centralized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEntry {
    pub path: String,
    pub role: Option<String>,
    pub format: Option<String>,
    pub bytes: u64,
    pub sha256: String,
}

/// Authentication receipt for one manifest entry copied through the bounded
/// file-backed container path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoolEntryReceipt {
    path: String,
    uncompressed_len: u64,
    sha256: String,
}

impl SpoolEntryReceipt {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn uncompressed_len(&self) -> u64 {
        self.uncompressed_len
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

impl ContainerEntry {
    fn from_manifest(ent: &Value) -> Option<ContainerEntry> {
        Some(ContainerEntry {
            path: ent.get("path")?.as_str()?.to_string(),
            role: ent.get("role").and_then(Value::as_str).map(str::to_string),
            format: ent
                .get("format")
                .and_then(Value::as_str)
                .map(str::to_string),
            bytes: ent.get("bytes")?.as_u64()?,
            sha256: ent.get("sha256")?.as_str()?.to_string(),
        })
    }
}

/// An opened, manifest-verified `.battleplan`. Path opens retain one pinned file
/// handle; byte-buffer opens retain their supplied archive. Snapshot bytes are
/// verified lazily on read.
pub struct Container {
    archive: ContainerArchive,
    manifest: Value,
    content_manifest: Option<portable_content_manifest::ContentManifest>,
    game_id: String,
    /// container-path -> entry record (post-verification of manifest↔zip agreement)
    by_path: Mutex<HashMap<String, EntryRec>>,
    /// the verified, parsed index (its `file` names are the only filename authority)
    index: Index,
    /// hour_index -> index `file` name, built once at open (first entry wins,
    /// matching the historical first-match scan) so per-hour lookup is O(log n)
    /// instead of a linear index scan per scrub step.
    hour_to_file: BTreeMap<i64, String>,
    /// Every snapshot file name an INDEX authorizes (spec §4.3: the index is the
    /// filename authority): the main index's `file` names plus, for a stitched
    /// container, every per-segment-index retained snapshot — superseded
    /// rollback files stay loadable by explicit name, while manifest-listed
    /// non-index entries (segments.json, stray payloads) never decode as a
    /// Snapshot.
    indexed_snapshot_files: BTreeSet<String>,
    /// signed | unsigned | sig-invalid(rejected before construction)
    pub signed: bool,
    pub unsigned: bool,
}

impl Container {
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    pub fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.content_manifest.as_ref()
    }
    pub fn game_id(&self) -> &str {
        &self.game_id
    }
    pub fn index(&self) -> &Index {
        &self.index
    }
    /// Manifest-listed payload entries in manifest order.
    pub fn entries(&self) -> Vec<ContainerEntry> {
        self.manifest
            .get("entries")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(ContainerEntry::from_manifest)
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Manifest-listed payload entries with a specific role.
    pub fn entries_by_role(&self, role: &str) -> Vec<ContainerEntry> {
        self.entries()
            .into_iter()
            .filter(|entry| entry.role.as_deref() == Some(role))
            .collect()
    }

    /// Open + verify a `.battleplan` from a path. `expected` enforces identity
    /// (build/schema/mod) when its fields are set; pass `Default::default()` for
    /// structural and identity consistency checks. Payload integrity remains
    /// fail-closed and is verified lazily at each exposure boundary.
    pub fn open<P: AsRef<Path>>(
        path: P,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        Self::open_strict(path, expected)
    }

    /// Open through one pinned file handle and retain only bounded ZIP metadata.
    /// Payload CRC, size, and SHA-256 checks complete lazily in the same pass
    /// that supplies authenticated contents to a caller.
    pub fn open_strict<P: AsRef<Path>>(
        path: P,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        let file = open_retained_container_file(path.as_ref())?;
        let archive =
            FileBackedArchive::open(file, MAX_ENTRIES as u64).map_err(file_archive_error)?;
        Self::from_archive(ContainerArchive::File(archive), expected)
    }

    /// Open through one pinned handle and bind a whole-file receipt to that
    /// exact handle. The whole-file length and SHA-256 must agree before and
    /// after manifest/index parsing. External package-set descriptors can
    /// therefore authenticate and consume one file object without a pathname
    /// reopen window.
    pub fn open_strict_with_file_receipt<P: AsRef<Path>>(
        path: P,
        expected: &ExpectedIdentity,
    ) -> Result<(Container, u64, String), ContainerError> {
        let file = open_retained_container_file(path.as_ref())?;
        Self::open_strict_with_file_receipt_from_handle(file, expected, || Ok(()))
    }

    /// Open and retain an already-open file object, binding the same whole-file
    /// receipt used by the path-based strict opener. This is intended for an
    /// unpublished sibling temp whose creation handle has stayed open from pack
    /// through verification; no pathname reopen is involved.
    pub fn open_strict_with_file_receipt_from_file(
        file: File,
        expected: &ExpectedIdentity,
    ) -> Result<(Container, u64, String), ContainerError> {
        Self::open_strict_with_file_receipt_from_handle(file, expected, || Ok(()))
    }

    fn open_strict_with_file_receipt_from_handle<F>(
        mut file: File,
        expected: &ExpectedIdentity,
        after_initial_receipt: F,
    ) -> Result<(Container, u64, String), ContainerError>
    where
        F: FnOnce() -> Result<(), ContainerError>,
    {
        // Receipt the exact retained handle before even the central directory
        // is parsed. The same handle is then moved into the archive and hashed
        // again after all manifest/index parsing, closing the entire parse
        // interval rather than only the higher-level manifest interval.
        let initial_receipt = FileBackedArchive::whole_file_receipt_before_open(&mut file)
            .map_err(file_archive_error)?;
        after_initial_receipt()?;
        let archive =
            FileBackedArchive::open(file, MAX_ENTRIES as u64).map_err(file_archive_error)?;
        let container = Self::from_archive(ContainerArchive::File(archive), expected)?;
        let final_receipt = container.archive.whole_file_receipt()?;
        if final_receipt != initial_receipt {
            return Err(ContainerError::Reject(
                "retained container changed while its manifest and index were being parsed"
                    .to_string(),
            ));
        }
        Ok((container, final_receipt.0, final_receipt.1))
    }

    /// As [`open`](Self::open) but from in-memory bytes (used by tests).
    pub fn from_bytes(
        data: Vec<u8>,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        Self::from_bytes_strict(data, expected)
    }

    /// Strict in-memory open. The strict pass returns the semantic value, so
    /// man_bytes is never decoded ordinarily before or after validation.
    pub fn from_bytes_strict(
        data: Vec<u8>,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        let archive = ContainerArchive::Memory(ZipArchive::parse_with_entry_limit(
            data,
            MAX_ENTRIES as u64,
        )?);
        Self::from_archive(archive, expected)
    }

    fn from_archive(
        archive: ContainerArchive,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        // Validate the physical half of every ZIP entry before reading even the
        // manifest payload. This covers v2 entries that semantic validation does
        // not otherwise materialize, including embedded raw sources.
        archive.validate_local_headers()?;

        // structural zip-bomb caps (spec §4.1, step 10)
        if archive.len() > MAX_ENTRIES {
            return Err(ContainerError::Reject(format!(
                "too many entries ({} > {})",
                archive.len(),
                MAX_ENTRIES
            )));
        }
        let mut total: u64 = 0;
        for index in 0..archive.len() {
            let e = archive.entry(index).ok_or_else(|| {
                ContainerError::Reject(format!("container entry index {index} is out of range"))
            })?;
            if e.uncomp_size > archive.entry_size_cap() {
                return Err(ContainerError::Reject(format!(
                    "entry {} exceeds per-entry cap",
                    e.name
                )));
            }
            total = total.saturating_add(e.uncomp_size);
        }
        if total > MAX_TOTAL_BYTES {
            return Err(ContainerError::Reject(
                "total uncompressed size exceeds cap".into(),
            ));
        }

        // One borrowed exact-name index supports O(1) manifest joins without
        // cloning every retained central-directory path a second time. EntryRec
        // keeps only the resulting stable numeric index after construction.
        let archive_name_index: HashMap<&str, usize> = (0..archive.len())
            .filter_map(|index| archive.entry(index).map(|entry| (entry.name, index)))
            .collect();
        if archive_name_index.len() != archive.len() {
            return Err(ContainerError::Reject(
                "container name index lost an entry".into(),
            ));
        }

        // (5) manifest present? absent => recovery-only, refused.
        let man_entry = archive_name_index
            .get("manifest.json")
            .copied()
            .ok_or(ContainerError::NoManifest)?;
        let (man_bytes, _) = archive.read_entry(man_entry, false)?;
        let manifest = crate::strict_json::from_slice(&man_bytes)?;
        validate_strict_manifest(&manifest)?;

        // (6) format + version
        if manifest.get("format").and_then(Value::as_str) != Some("hoi4replay") {
            return Err(ContainerError::Reject(
                "manifest.format != \"hoi4replay\"".into(),
            ));
        }
        let cv = manifest
            .get("container_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| ContainerError::Reject("manifest.container_version missing".into()))?;
        if cv > MAX_CONTAINER_VERSION {
            return Err(ContainerError::Reject(format!(
                "container_version {cv} > reader max {MAX_CONTAINER_VERSION}"
            )));
        }
        // (7) required_features the reader must understand. FAIL-CLOSED on shape:
        // (collect the declared feature names first so the version<->feature
        // coupling below can be checked.)
        let mut declared_features: Vec<String> = Vec::new();
        // absent or JSON null = "none required" (allowed); a PRESENT value of any
        // other type, or any non-string element, is malformed and MUST be rejected
        // — otherwise a tampered manifest could smuggle an unknown required feature
        // past this gate by encoding it as a non-array (e.g. a bare string), which
        // `as_array` would silently treat as "no required features" (Codex b).
        match manifest.get("required_features") {
            None | Some(Value::Null) => {}
            Some(Value::Array(rf)) => {
                for feat in rf {
                    let name = feat.as_str().ok_or_else(|| {
                        ContainerError::Reject(
                            "manifest.required_features has a non-string element (malformed)"
                                .into(),
                        )
                    })?;
                    if !KNOWN_FEATURES.contains(&name) {
                        return Err(ContainerError::Reject(format!(
                            "unknown required feature {name:?} in required_features"
                        )));
                    }
                    declared_features.push(name.to_string());
                }
            }
            Some(_) => {
                return Err(ContainerError::Reject(
                    "manifest.required_features must be an array (malformed; fail-closed)".into(),
                ));
            }
        }

        // Historical container v1 used the ASCII-only collision namespace.
        // A stitched v2 container or the current portable-content-v2 feature
        // opts into spec section 4.2's NFC plus Unicode-caseless identity. The
        // explicit feature keeps new ordinary (non-stitched) v1 packages strict
        // while preserving pre-feature recordings byte-for-byte.
        let unicode_paths = cv >= STITCHED_CONTAINER_VERSION
            || declared_features
                .iter()
                .any(|feature| feature == FEATURE_PORTABLE_CONTENT_MANIFEST_V2);
        archive.validate_portable_paths(unicode_paths)?;

        // (7b) version<->feature coupling (§9): a stitched container declares
        // container_version 2 AND required feature `timeline_segments_v1`. A v2+
        // container WITHOUT that feature is malformed — reject fail-closed so a
        // tampered manifest cannot advertise the segment format while omitting the
        // flag old readers key on.
        if cv >= STITCHED_CONTAINER_VERSION
            && !declared_features
                .iter()
                .any(|f| f == FEATURE_TIMELINE_SEGMENTS_V1)
        {
            return Err(ContainerError::Reject(format!(
                "container_version {cv} must declare required feature \
                 {FEATURE_TIMELINE_SEGMENTS_V1:?} (stitched-format coupling; §9)"
            )));
        }
        if cv < STITCHED_CONTAINER_VERSION
            && declared_features
                .iter()
                .any(|f| f == FEATURE_TIMELINE_SEGMENTS_V1)
        {
            return Err(ContainerError::Reject(format!(
                "required feature {FEATURE_TIMELINE_SEGMENTS_V1:?} requires container_version {STITCHED_CONTAINER_VERSION}"
            )));
        }

        let content_feature = declared_features
            .iter()
            .any(|feature| feature == FEATURE_PORTABLE_CONTENT_MANIFEST_V2);
        let content_manifest = match manifest.get("content_manifest") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let parsed: portable_content_manifest::ContentManifest =
                    serde_json::from_value(value.clone()).map_err(|err| {
                        ContainerError::Reject(format!(
                            "manifest.content_manifest is malformed: {err}"
                        ))
                    })?;
                parsed.validate().map_err(|err| {
                    ContainerError::Reject(format!(
                        "manifest.content_manifest failed validation: {err}"
                    ))
                })?;
                if parsed.manifest_digest_sha256.is_none() {
                    return Err(ContainerError::Reject(
                        "manifest.content_manifest has no finalized digest".into(),
                    ));
                }
                Some(parsed)
            }
        };
        if content_feature != content_manifest.is_some() {
            return Err(ContainerError::Reject(
                "portable_content_manifest_v2 feature/content_manifest coupling mismatch".into(),
            ));
        }

        // (8) manifest_digest over the canonical manifest (strips digest+signature)
        let declared_digest = manifest
            .get("manifest_digest")
            .and_then(Value::as_str)
            .ok_or_else(|| ContainerError::Reject("manifest.manifest_digest missing".into()))?
            .to_string();
        let recomputed = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
        if recomputed != declared_digest {
            return Err(ContainerError::Reject(format!(
                "manifest_digest mismatch (recomputed {recomputed}, declared {declared_digest})"
            )));
        }

        // (9) signature: present-and-valid => signed; present-and-invalid => reject;
        // absent => unsigned (allowed). v1 ships the verification SLOT; an actual
        // Ed25519 verify is wired when a key policy is configured (the digest is the
        // signed payload). Until then a NON-NULL signature with no verifier policy
        // is conservatively REJECTED rather than trusted (fail-closed).
        let sig = manifest.get("signature");
        let (signed, unsigned) = match sig {
            None | Some(Value::Null) => (false, true),
            Some(_) => {
                return Err(ContainerError::Reject(
                    "manifest.signature present but no signature-verification policy is \
                     configured in this reader build (fail-closed; spec §3.2)"
                        .into(),
                ));
            }
        };

        // identity expectations (spec §4.1 step optional / §2)
        let schema_version = manifest.get("schema_version").and_then(Value::as_str);
        if let Some(exp) = &expected.schema_version {
            if schema_version != Some(exp.as_str()) {
                return Err(ContainerError::Reject(format!(
                    "schema_version {:?} != expected {exp:?}",
                    schema_version
                )));
            }
        }
        let build_sha = manifest
            .get("build")
            .and_then(|b| b.get("exe_sha256"))
            .and_then(Value::as_str);
        if let Some(exp) = &expected.build_exe_sha256 {
            if build_sha != Some(exp.as_str()) {
                return Err(ContainerError::Reject(format!(
                    "build.exe_sha256 {:?} != expected {exp:?}",
                    build_sha
                )));
            }
        }
        let modset_id = manifest
            .get("mod_identity")
            .and_then(|m| m.get("modset_id"))
            .and_then(Value::as_str);
        if let Some(exp) = &expected.modset_id {
            if modset_id != Some(exp.as_str()) {
                return Err(ContainerError::Reject(format!(
                    "mod_identity.modset_id {:?} != expected {exp:?}",
                    modset_id
                )));
            }
        }

        let game_id = manifest
            .get("game_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ContainerError::Reject("manifest.game_id missing".into()))?
            .to_string();

        // Build the path -> expected-hash map from manifest.entries, and confirm
        // every manifest entry actually exists in the zip with the declared size
        // (and vice-versa: a zip entry not in the manifest is a tamper signal).
        let entries = manifest
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| ContainerError::Reject("manifest.entries missing".into()))?;
        if entries.len() > MAX_ENTRIES {
            return Err(ContainerError::Reject(format!(
                "manifest has too many entries ({} > {MAX_ENTRIES})",
                entries.len()
            )));
        }
        let mut by_path: HashMap<String, EntryRec> = HashMap::with_capacity(entries.len());
        let mut manifest_collision_keys: HashSet<String> = HashSet::with_capacity(entries.len());
        // (filled below; later verify-marks need it mutable)
        for ent in entries {
            let p = ent
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| ContainerError::Reject("entry.path missing".into()))?;
            let collision_key = if unicode_paths {
                validate_payload_path(p)?;
                portable_path_key(p)?
            } else {
                validate_payload_path_legacy_v1(p)?;
                legacy_path_key(p)?
            };
            if !manifest_collision_keys.insert(collision_key) {
                return Err(ContainerError::Reject(format!(
                    "duplicate or Unicode-case-colliding manifest entry {p}"
                )));
            }
            let sha = ent
                .get("sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| ContainerError::Reject(format!("entry {p}: sha256 missing")))?;
            portable_content_manifest::hud_role::validate_sha256(sha, "manifest entry sha256")
                .map_err(|error| ContainerError::Reject(format!("entry {p}: {error}")))?;
            let declared_bytes = ent
                .get("bytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| ContainerError::Reject(format!("entry {p}: bytes missing")))?;
            let archive_index = archive_name_index.get(p).copied().ok_or_else(|| {
                ContainerError::Reject(format!("manifest entry {p} not present in zip"))
            })?;
            let ze = archive.entry(archive_index).ok_or_else(|| {
                ContainerError::Reject(format!(
                    "container entry index {archive_index} is out of range"
                ))
            })?;
            // declared bytes must equal the zip's uncompressed size (zip-bomb guard
            // + manifest-can't-lie-about-size). (Codex b)
            if ze.uncomp_size != declared_bytes {
                return Err(ContainerError::Reject(format!(
                    "entry {p}: manifest bytes {declared_bytes} != zip uncompressed {}",
                    ze.uncomp_size
                )));
            }
            if by_path
                .insert(
                    p.to_string(),
                    EntryRec {
                        archive_index,
                        bytes: declared_bytes,
                        expected_sha256: sha.to_string(),
                        sha256_state: EntryVerificationState::Pending,
                    },
                )
                .is_some()
            {
                return Err(ContainerError::Reject(format!(
                    "duplicate manifest entry {p}"
                )));
            }
        }
        // every payload/* zip entry must be covered by the manifest (no stowaways)
        for archive_index in 0..archive.len() {
            let ze = archive.entry(archive_index).ok_or_else(|| {
                ContainerError::Reject(format!(
                    "container entry index {archive_index} is out of range"
                ))
            })?;
            if ze.name == "manifest.json" {
                continue;
            }
            if !by_path.contains_key(ze.name) {
                return Err(ContainerError::Reject(format!(
                    "zip entry {} is not listed in the manifest (tamper signal)",
                    ze.name
                )));
            }
        }
        drop(archive_name_index);

        // (11) verify + parse index.json NOW (it is the spine the app reads first).
        // `by_path` is already `mut` (its verify-marks need it); verify_and_read
        // hashes the bytes against the manifest before returning them.
        let index_path = format!("payload/{game_id}/index.json");
        let index_bytes = verify_and_read(&archive, &mut by_path, &index_path)?;
        // Strict (duplicate-key-rejecting) decode per replay-loader hardening,
        // while retaining the parsed Value that stitched-v2 validation consumes.
        let index_value: Value = crate::parse_json_strict(&index_bytes)?;
        let index: Index = serde_json::from_value(index_value.clone())?;
        // index.game_id must agree with the manifest game_id
        if index.game_id != game_id {
            return Err(ContainerError::Reject(format!(
                "index.game_id {:?} != manifest.game_id {game_id:?}",
                index.game_id
            )));
        }

        let mut indexed_snapshot_files: BTreeSet<String> = index
            .snapshots
            .iter()
            .map(|entry| entry.file.clone())
            .collect();
        if cv == STITCHED_CONTAINER_VERSION {
            let retained =
                validate_stitched_v2(&archive, &mut by_path, &manifest, &game_id, &index_value)?;
            indexed_snapshot_files.extend(retained);
        }

        // (12) verify + parse timeline.json now if it is declared (spec §4.1
        // step 12): a malformed or duplicate-key timeline must be an open-time
        // refusal, never a later consumption-time surprise.
        let timeline_path = format!("payload/{game_id}/timeline.json");
        if by_path.contains_key(&timeline_path) {
            let timeline_bytes = verify_and_read(&archive, &mut by_path, &timeline_path)?;
            crate::parse_json_strict(&timeline_bytes).map_err(|e| {
                ContainerError::Reject(format!("timeline.json failed strict JSON parse: {e}"))
            })?;
        }

        // (13) identity consistency: manifest.mod_identity.modset_id must match a
        // representative snapshot's provenance.mod.modset_id (a manifest must not be
        // able to lie about identity). We check the FIRST snapshot in the index.
        if let Some(first) = index.snapshots.first() {
            let snap_path = format!("payload/{game_id}/{}", first.file);
            let snap_bytes = verify_and_read(&archive, &mut by_path, &snap_path)?;
            let snap: Value = crate::strict_json::from_slice(&snap_bytes)?;
            let snap_modset = snap
                .get("provenance")
                .and_then(|p| p.get("mod"))
                .and_then(|m| m.get("modset_id"))
                .and_then(Value::as_str);
            // Both null is fine (mod-set not read either place). If the manifest
            // claims a modset id, a snapshot that names a DIFFERENT one is a lie.
            if let (Some(m), Some(s)) = (modset_id, snap_modset) {
                if m != s {
                    return Err(ContainerError::ModsetMismatch {
                        manifest: m.to_string(),
                        snapshot: s.to_string(),
                    });
                }
            }
            // Same principle for the build identity (spec §2): the manifest's
            // build.exe_sha256 must not contradict what the snapshots recorded.
            // Absence on either side is a measured absence and passes.
            for (label, snap_sha) in [
                (
                    "collector",
                    snap.get("provenance")
                        .and_then(|p| p.get("collector"))
                        .and_then(|c| c.get("exe_sha256"))
                        .and_then(Value::as_str),
                ),
                (
                    "memory",
                    snap.get("provenance")
                        .and_then(|p| p.get("memory"))
                        .and_then(|m| m.get("exe_sha256"))
                        .and_then(Value::as_str),
                ),
            ] {
                if let (Some(m), Some(s)) = (build_sha, snap_sha) {
                    if m != s {
                        return Err(ContainerError::Reject(format!(
                            "manifest build.exe_sha256 {m:?} disagrees with snapshot provenance.{label}.exe_sha256 {s:?}"
                        )));
                    }
                }
            }
        }

        // hour_index -> file resolution map. FIRST entry wins, preserving the
        // previous `.find()` first-match semantics (hour_index uniqueness is not
        // validated at open for a plain v1 index).
        let mut hour_to_file: BTreeMap<i64, String> = BTreeMap::new();
        for entry in &index.snapshots {
            if let Some(hour) = entry.hour_index {
                hour_to_file
                    .entry(hour)
                    .or_insert_with(|| entry.file.clone());
            }
        }

        Ok(Container {
            archive,
            manifest,
            content_manifest,
            game_id,
            by_path: Mutex::new(by_path),
            index,
            hour_to_file,
            indexed_snapshot_files,
            signed,
            unsigned,
        })
    }

    /// Load a snapshot by its index `file` name (the only filename authority).
    /// Verifies the entry's hash lazily; never exposes unverified bytes (Codex d).
    pub fn load_file(&self, file: &str) -> Result<Snapshot, ContainerError> {
        // Spec §4.3 step 1: the index (main index, plus per-segment indexes for
        // a stitched container's retained rollback snapshots) is the filename
        // authority for snapshot reads. Refuse to decode a manifest-listed
        // non-index entry (e.g. segments.json or a stray JSON payload) as a
        // Snapshot.
        if !self.indexed_snapshot_files.contains(file) {
            return Err(ContainerError::Reject(format!(
                "snapshot file {file:?} is not listed in the index (index is the filename authority)"
            )));
        }
        let path = format!("payload/{}/{}", self.game_id, file);
        let bytes = self.read_entry_bytes(&path)?;
        Ok(crate::decode_json_strict(&bytes)?)
    }

    /// Read any manifest-listed payload entry as verified raw bytes. This is the
    /// generic path used for optional non-snapshot streams such as
    /// `collector_raw_stream`.
    pub fn read_entry_bytes(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        let mut guard = self.by_path.lock().map_err(|_| {
            ContainerError::Reject("container entry-verification lock poisoned".into())
        })?;
        verify_and_read(&self.archive, &mut guard, path)
    }

    /// Authenticate one file-backed manifest entry without exposing its bytes.
    pub fn authenticate_entry(&self, path: &str) -> Result<SpoolEntryReceipt, ContainerError> {
        self.spool_entry_to_writer(path, &mut std::io::sink())
    }

    /// Copy one authenticated entry into a caller-owned unpublished sink using
    /// the retained container handle and bounded decompression buffers. The
    /// caller must not expose or publish the sink until this method returns its
    /// receipt successfully.
    pub fn copy_entry_to_unpublished_writer_bounded<W: Write>(
        &self,
        path: &str,
        writer: &mut W,
        maximum_uncompressed_bytes: u64,
    ) -> Result<SpoolEntryReceipt, ContainerError> {
        self.spool_entry_to_writer_bounded(path, writer, maximum_uncompressed_bytes)
    }

    /// Authenticate and decompress one file-backed manifest entry into a new
    /// file, publishing the destination name only after every integrity check,
    /// flush, and file sync succeeds.
    ///
    /// A unique sibling temporary file is created with `create_new`; on Windows
    /// its retained handle denies write/delete sharing and every destination
    /// directory ancestor is retained as a non-reparse handle. After sync, the
    /// same handle is re-hashed and its pathname identity is rebound. The
    /// no-replace hard link is the commit point; production performs no
    /// fallible operation after the destination becomes visible.
    pub fn spool_entry_to_file_transactionally<P: AsRef<Path>>(
        &self,
        path: &str,
        destination: P,
    ) -> Result<SpoolEntryReceipt, ContainerError> {
        self.spool_entry_to_file_transactionally_with_hooks(
            path,
            destination.as_ref(),
            STREAMING_MAX_CONTAINER_ENTRY_BYTES,
            |_, _| Ok(()),
            |_| Ok(()),
        )
    }

    /// Product-facing bounded variant. The manifest declaration is rejected
    /// before creating a spool file, and the decompressor enforces the same
    /// ceiling while writing so a forged ZIP size cannot bypass admission.
    pub fn spool_entry_to_file_transactionally_bounded<P: AsRef<Path>>(
        &self,
        path: &str,
        destination: P,
        maximum_uncompressed_bytes: u64,
    ) -> Result<SpoolEntryReceipt, ContainerError> {
        self.spool_entry_to_file_transactionally_with_hooks(
            path,
            destination.as_ref(),
            maximum_uncompressed_bytes,
            |_, _| Ok(()),
            |_| Ok(()),
        )
    }

    fn spool_entry_to_file_transactionally_with_hook<F>(
        &self,
        path: &str,
        destination: &Path,
        before_publish: F,
    ) -> Result<SpoolEntryReceipt, ContainerError>
    where
        F: FnOnce(&Path, &mut File) -> Result<(), ContainerError>,
    {
        self.spool_entry_to_file_transactionally_with_hooks(
            path,
            destination,
            STREAMING_MAX_CONTAINER_ENTRY_BYTES,
            before_publish,
            |_| Ok(()),
        )
    }

    fn spool_entry_to_file_transactionally_with_hooks<F, G>(
        &self,
        path: &str,
        destination: &Path,
        maximum_uncompressed_bytes: u64,
        before_publish: F,
        after_publish: G,
    ) -> Result<SpoolEntryReceipt, ContainerError>
    where
        F: FnOnce(&Path, &mut File) -> Result<(), ContainerError>,
        G: FnOnce(&Path) -> Result<(), ContainerError>,
    {
        self.spool_entry_to_file_transactionally_with_release_hook(
            path,
            destination,
            maximum_uncompressed_bytes,
            before_publish,
            after_publish,
            |_, _| {},
        )
    }

    fn spool_entry_to_file_transactionally_with_release_hook<F, G, H>(
        &self,
        path: &str,
        destination: &Path,
        maximum_uncompressed_bytes: u64,
        before_publish: F,
        after_publish: G,
        after_release: H,
    ) -> Result<SpoolEntryReceipt, ContainerError>
    where
        F: FnOnce(&Path, &mut File) -> Result<(), ContainerError>,
        G: FnOnce(&Path) -> Result<(), ContainerError>,
        H: FnOnce(&Path, &Path),
    {
        let directory = SpoolDirectoryGuard::acquire(destination)?;
        let destination = directory.destination.clone();
        let (temp, file, retained_identity) =
            create_unique_spool_temp(&directory).map_err(|error| {
                ContainerError::Reject(format!(
                    "create authenticated spool temporary file in {}: {error}",
                    directory.parent.display()
                ))
            })?;
        let mut writer = std::io::BufWriter::with_capacity(1024 * 1024, file);
        let mut published = false;
        let result = (|| {
            let receipt = self
                .spool_entry_to_writer_bounded(path, &mut writer, maximum_uncompressed_bytes)
                .map_err(|error| {
                    ContainerError::Reject(format!(
                        "stream authenticated container entry into spool temporary: {error}"
                    ))
                })?;
            writer.flush().map_err(|error| {
                ContainerError::Reject(format!("flush authenticated spool temporary: {error}"))
            })?;
            writer.get_ref().sync_all().map_err(|error| {
                ContainerError::Reject(format!("sync authenticated spool temporary: {error}"))
            })?;
            // Test-only mutation window. Production passes a no-op closure; the
            // hook receives the retained handle as well as the pathname so a
            // regression test can model BOTH post-sync substitution shapes: a
            // pathname rebind by another process, and a same-handle byte change
            // that no share mode can deny.
            before_publish(&temp, writer.get_mut())?;

            // Re-authenticate through the retained handle after the final
            // mutation window and before any pathname is published.
            verify_spool_receipt_on_handle(writer.get_mut(), &receipt)?;
            // Held to the end of this closure, exactly as the inline local was.
            let _temp_path_handle = bind_spool_temp_pathname_identity(&temp, retained_identity)?;

            // Commit point: the source pathname is identity-bound to the
            // retained, immutable handle and every directory ancestor remains
            // locked. A successful no-replace hard link therefore publishes
            // that exact object. Production performs no fallible operation
            // after this call.
            fs::hard_link(&temp, &destination).map_err(|error| {
                ContainerError::Reject(format!(
                    "publish authenticated spool file by no-replace hard link: {error}"
                ))
            })?;
            published = true;
            after_publish(&destination)?;
            Ok(receipt)
        })();
        drop(writer);
        // Release every deny-delete file/directory handle before a test-only
        // injected post-commit failure is rolled back.
        drop(directory);
        // Test-only window. This is the interval in which the destination
        // pathname is no longer identity-pinned by any retained handle, which
        // is exactly what the rollback below re-checks before unlinking.
        // Production passes a no-op closure; the hook returns nothing, so it
        // cannot influence control flow.
        after_release(&temp, &destination);

        let result = if published && result.is_err() {
            let destination_still_retained = open_spool_path_for_identity(&destination)
                .ok()
                .and_then(|file| spool_file_identity(&file, &destination).ok())
                == Some(retained_identity);
            if !destination_still_retained {
                Err(ContainerError::Reject(
                    "post-publication failure could not identity-bind destination cleanup"
                        .to_string(),
                ))
            } else {
                match fs::remove_file(&destination) {
                    Ok(()) => result,
                    Err(error) => Err(ContainerError::Reject(format!(
                        "post-publication failure could not remove authenticated destination: {error}"
                    ))),
                }
            }
        } else {
            result
        };

        // Never unlink a pathname that an adversary rebound while publication
        // was failing. Successful publication (and ordinary verification
        // failures without a namespace swap) still remove the retained temp.
        let temp_still_retained = open_spool_path_for_identity(&temp)
            .ok()
            .and_then(|file| spool_file_identity(&file, &temp).ok())
            == Some(retained_identity);
        if temp_still_retained {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    /// Low-level authenticated copy primitive. It remains private so no caller
    /// can accidentally expose bytes written before the final SHA-256 match.
    fn spool_entry_to_writer<W: Write>(
        &self,
        path: &str,
        writer: &mut W,
    ) -> Result<SpoolEntryReceipt, ContainerError> {
        self.spool_entry_to_writer_bounded(path, writer, STREAMING_MAX_CONTAINER_ENTRY_BYTES)
    }

    fn spool_entry_to_writer_bounded<W: Write>(
        &self,
        path: &str,
        writer: &mut W,
        maximum_uncompressed_bytes: u64,
    ) -> Result<SpoolEntryReceipt, ContainerError> {
        if maximum_uncompressed_bytes == 0 {
            return Err(ContainerError::Reject(
                "entry spool byte ceiling must be positive".to_string(),
            ));
        }
        let mut guard = self.by_path.lock().map_err(|_| {
            ContainerError::Reject("container entry-verification lock poisoned".into())
        })?;
        let rec = guard
            .get_mut(path)
            .ok_or_else(|| ContainerError::Reject(format!("entry {path} not in manifest")))?;
        if rec.bytes > maximum_uncompressed_bytes {
            return Err(ContainerError::Reject(format!(
                "entry {path}: declared uncompressed bytes {} exceed spool ceiling {maximum_uncompressed_bytes}",
                rec.bytes
            )));
        }
        let (digest, uncompressed_len) = self
            .archive
            .copy_entry_to_writer(rec.archive_index, writer, maximum_uncompressed_bytes)?;
        if digest != rec.expected_sha256 {
            return Err(ContainerError::Reject(format!(
                "entry {path}: sha256 {digest} != manifest {}",
                rec.expected_sha256
            )));
        }
        rec.sha256_state = EntryVerificationState::Verified;
        rec.require_sha256_verified(path)?;
        Ok(SpoolEntryReceipt {
            path: path.to_string(),
            uncompressed_len,
            sha256: digest,
        })
    }

    /// The stitched branch graph (`segments.json`), verified and parsed, or `None`
    /// when this is a plain (non-stitched) container. The graph is the
    /// authoritative rollback/branch map (`STITCHING_DESIGN.md` §7.2); the main
    /// `index()` remains the default-playback view.
    pub fn segments(&self) -> Result<Option<SegmentGraph>, ContainerError> {
        let path = format!("payload/{}/segments.json", self.game_id);
        if !self.has_entry(&path)? {
            return Ok(None);
        }
        let bytes = self.read_entry_bytes(&path)?;
        // Strict (duplicate-member-rejecting) decode: the graph is trusted
        // rollback authority, so last-wins collapse is an ambiguity to refuse.
        Ok(Some(crate::decode_json_strict(&bytes)?))
    }

    /// Whether a manifest-listed payload entry exists at `path` (O(1); no
    /// per-call manifest re-projection). A poisoned entry-verification lock is
    /// the SAME typed refusal [`Container::read_entry_bytes`] raises — never a
    /// silent "absent" (a caught panic mid-verification must fail closed, not
    /// downgrade an existing entry into `Ok(None)` at the caller).
    pub fn has_entry(&self, path: &str) -> Result<bool, ContainerError> {
        let guard = self.by_path.lock().map_err(|_| {
            ContainerError::Reject("container entry-verification lock poisoned".into())
        })?;
        Ok(guard.contains_key(path))
    }

    /// Load a snapshot by hour_index — resolved through the INDEX (`file` field),
    /// not by synthesizing a filename, so segment-prefixed rollback files load too.
    pub fn load(&self, hour_index: i64) -> Result<Snapshot, ContainerError> {
        let file = self.hour_to_file.get(&hour_index).cloned().ok_or_else(|| {
            ContainerError::Reject(format!("no snapshot for hour_index {hour_index}"))
        })?;
        self.load_file(&file)
    }

    /// Test-only: poison the entry-verification lock by panicking while a
    /// thread holds it, mimicking a caught panic mid-verification.
    #[cfg(test)]
    pub(crate) fn poison_entry_lock_for_test(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.by_path.lock().unwrap();
            panic!("poison entry-verification lock");
        }));
        assert!(self.by_path.is_poisoned(), "lock must be poisoned");
    }

    /// Full background verification: hash EVERY declared entry. Returns Ok(()) when
    /// all match; the first mismatch is an error. (Optional; never gates open.)
    ///
    /// An entry larger than the materializing cap is verified by streaming it
    /// past a sink instead — same decompression, same CRC, same SHA-256, no
    /// resident copy. That case exists because the certified checkpoint cache is
    /// now packed under the streaming ceiling: capture t32's cache is
    /// 2,209,767,138 bytes, and a `verify_all` that insisted on a `Vec` would
    /// refuse to check the very container it had just sealed. Every entry that
    /// still fits the cap goes through the unchanged materializing path, so no
    /// previously verifiable container changes route.
    pub fn verify_all(&self) -> Result<(), ContainerError> {
        let mut guard = self.by_path.lock().map_err(|_| {
            ContainerError::Reject("container entry-verification lock poisoned".into())
        })?;
        let paths: Vec<String> = guard.keys().cloned().collect();
        for p in paths {
            let oversized = guard
                .get(&p)
                .is_some_and(|rec| rec.bytes > MAX_CONTAINER_ENTRY_BYTES);
            if oversized {
                verify_streamed(&self.archive, &mut guard, &p)?;
            } else {
                verify_and_read(&self.archive, &mut guard, &p)?;
            }
        }
        Ok(())
    }

    /// Full retained-file verification without materializing any payload. Each
    /// entry is decompressed, CRC-checked, and SHA-256 checked into a bounded
    /// sink through the already-open archive handle.
    pub fn verify_all_streaming(&self) -> Result<(), ContainerError> {
        let mut guard = self.by_path.lock().map_err(|_| {
            ContainerError::Reject("container entry-verification lock poisoned".into())
        })?;
        let paths = guard.keys().cloned().collect::<Vec<_>>();
        for path in paths {
            verify_streamed(&self.archive, &mut guard, &path)?;
        }
        Ok(())
    }

    /// Recompute a bounded whole-file receipt through the retained file handle.
    /// File-backed callers use this after streaming verification to prove the
    /// object did not change across the complete validation interval.
    pub fn retained_file_receipt(&self) -> Result<(u64, String), ContainerError> {
        self.archive.whole_file_receipt()
    }
}

/// Strict states that ContainerEntry's permissive projection cannot express.
/// This runs before format, digest, identity, or role-table semantics.
fn validate_strict_manifest(manifest: &Value) -> Result<(), ContainerError> {
    let game_id = manifest.get("game_id").and_then(Value::as_str);
    let timeline_path = game_id.map(|game| format!("payload/{game}/timeline.json"));
    let Some(entries) = manifest.get("entries").and_then(Value::as_array) else {
        return Ok(());
    };

    for (index, entry) in entries.iter().enumerate() {
        let path = entry.get("path").and_then(Value::as_str);
        if let Some(role) = entry.get("role") {
            if !role.is_string() {
                return Err(ContainerError::Reject(format!(
                    "manifest entry {} role is present but is not a string",
                    path.map(|value| format!("{value:?}"))
                        .unwrap_or_else(|| format!("at index {index}"))
                )));
            }
        }
        if let (Some(path), Some(canonical)) = (path, timeline_path.as_deref()) {
            if path != canonical && path.eq_ignore_ascii_case(canonical) {
                return Err(ContainerError::Reject(format!(
                    "manifest entry path {path:?} is an ASCII-case lookalike of canonical timeline {canonical:?}"
                )));
            }
        }
    }
    Ok(())
}

/// Features this reader build understands. A `required_features` entry not here is
/// a hard reject (forward-compat fail-closed; Codex b).
const KNOWN_FEATURES: &[&str] = &[
    FEATURE_TIMELINE_SEGMENTS_V1,
    FEATURE_PORTABLE_CONTENT_MANIFEST_V2,
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedHour {
    segment_id: u32,
    kind: String,
    file: Option<String>,
    sha256: Option<String>,
    reason: Option<String>,
    evidence_id: Option<String>,
    ownership: Value,
}

#[derive(Debug, Clone)]
struct ValidatedParticipation {
    campaign_key_digest: String,
    scheme_major: u32,
    identity_evidence_digest: String,
    play_order: u64,
    session_nonce: String,
    process_start_identity: String,
    record_seq_bounds: (u64, u64),
    byte_bounds: (u64, u64),
}

#[derive(Debug, Clone)]
struct ValidatedRawSource {
    session_nonce: String,
    process_start_identity: String,
    session_path: String,
    stored_digest: String,
    decoded_digest: String,
    record_seq_bounds: (u64, u64),
    byte_bounds: (u64, u64),
    participations: HashMap<String, ValidatedParticipation>,
}

fn reject(message: impl Into<String>) -> ContainerError {
    ContainerError::Reject(message.into())
}

fn required_object<'a>(
    value: &'a Value,
    context: &str,
) -> Result<&'a serde_json::Map<String, Value>, ContainerError> {
    value
        .as_object()
        .ok_or_else(|| reject(format!("{context} must be an object")))
}

fn required_array<'a>(
    value: &'a Value,
    key: &str,
    context: &str,
) -> Result<&'a Vec<Value>, ContainerError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| reject(format!("{context}.{key} must be an array")))
}

fn required_i64(value: &Value, key: &str, context: &str) -> Result<i64, ContainerError> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| reject(format!("{context}.{key} must be an integer")))
}

fn required_u32(value: &Value, key: &str, context: &str) -> Result<u32, ContainerError> {
    let raw = value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| reject(format!("{context}.{key} must be an unsigned integer")))?;
    u32::try_from(raw).map_err(|_| reject(format!("{context}.{key} exceeds u32")))
}

fn required_u64(value: &Value, key: &str, context: &str) -> Result<u64, ContainerError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| reject(format!("{context}.{key} must be an unsigned integer")))
}

fn required_string<'a>(
    value: &'a Value,
    key: &str,
    context: &str,
) -> Result<&'a str, ContainerError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| reject(format!("{context}.{key} must be a non-empty string")))
}

fn optional_i64(value: &Value, key: &str, context: &str) -> Result<Option<i64>, ContainerError> {
    match value.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| reject(format!("{context}.{key} must be integer or null"))),
    }
}

fn optional_u32(value: &Value, key: &str, context: &str) -> Result<Option<u32>, ContainerError> {
    match value.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(v) => {
            let raw = v
                .as_u64()
                .ok_or_else(|| reject(format!("{context}.{key} must be unsigned or null")))?;
            Ok(Some(u32::try_from(raw).map_err(|_| {
                reject(format!("{context}.{key} exceeds u32"))
            })?))
        }
    }
}

fn required_u64_pair(
    value: &Value,
    key: &str,
    context: &str,
) -> Result<(u64, u64), ContainerError> {
    let pair = required_array(value, key, context)?;
    if pair.len() != 2 {
        return Err(reject(format!(
            "{context}.{key} must contain exactly two bounds"
        )));
    }
    let start = pair[0]
        .as_u64()
        .ok_or_else(|| reject(format!("{context}.{key}[0] must be unsigned")))?;
    let end = pair[1]
        .as_u64()
        .ok_or_else(|| reject(format!("{context}.{key}[1] must be unsigned")))?;
    Ok((start, end))
}

fn required_sha256(value: &Value, key: &str, context: &str) -> Result<String, ContainerError> {
    let digest = required_string(value, key, context)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(reject(format!(
            "{context}.{key} must be canonical lowercase SHA-256"
        )));
    }
    Ok(digest.to_string())
}

fn required_evidence_id<'a>(
    value: &'a Value,
    key: &str,
    context: &str,
) -> Result<&'a str, ContainerError> {
    let evidence_id = required_string(value, key, context)?;
    let bytes = evidence_id.as_bytes();
    if bytes.len() > 256
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
    {
        return Err(reject(format!(
            "{context}.{key} has invalid evidence-id grammar"
        )));
    }
    Ok(evidence_id)
}

fn required_reason_text<'a>(
    value: &'a Value,
    key: &str,
    context: &str,
) -> Result<&'a str, ContainerError> {
    let reason = required_string(value, key, context)?;
    if reason.len() > 4096 || reason.trim() != reason || reason.chars().any(char::is_control) {
        return Err(reject(format!(
            "{context}.{key} has invalid reason-text grammar"
        )));
    }
    Ok(reason)
}

fn required_hex_bytes(value: &Value, key: &str, context: &str) -> Result<Vec<u8>, ContainerError> {
    let encoded = required_string(value, key, context)?;
    if encoded.len() % 2 != 0
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(reject(format!(
            "{context}.{key} must be canonical lowercase even-length hex"
        )));
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                _ => unreachable!("hex shape validated"),
            };
            Ok((nibble(pair[0]) << 4) | nibble(pair[1]))
        })
        .collect()
}

fn raw_source_key(session: &str, process: &str, stored: &str, decoded: &str) -> String {
    let mut bytes = Vec::new();
    for value in [session, process, stored, decoded] {
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

fn parse_head(value: &Value, context: &str) -> Result<Option<(u32, i64)>, ContainerError> {
    if value.is_null() {
        return Ok(None);
    }
    required_object(value, context)?;
    Ok(Some((
        required_u32(value, "segment_id", context)?,
        required_i64(value, "hour_index", context)?,
    )))
}

fn parse_disposition(
    row: &Value,
    segment_id: u32,
    game_id: &str,
    by_path: &HashMap<String, EntryRec>,
    context: &str,
    allow_snapshot: bool,
) -> Result<ValidatedHour, ContainerError> {
    required_object(row, context)?;
    let kind = required_string(row, "kind", context)?;
    match kind {
        "snapshot" if allow_snapshot => {
            let file = required_string(row, "file", context)?.to_string();
            let sha = required_sha256(row, "sha256", context)?;
            let path = format!("payload/{game_id}/{file}");
            let entry = by_path
                .get(&path)
                .ok_or_else(|| reject(format!("{context} references missing snapshot {file:?}")))?;
            if entry.expected_sha256 != sha {
                return Err(reject(format!(
                    "{context} graph/index/hash disagreement for {file:?}"
                )));
            }
            Ok(ValidatedHour {
                segment_id,
                kind: kind.into(),
                file: Some(file),
                sha256: Some(sha),
                reason: None,
                evidence_id: None,
                ownership: Value::Null,
            })
        }
        "snapshot" => Err(reject(format!(
            "{context} transition coverage cannot be snapshot"
        ))),
        "capture_failed" | "coalesced" | "gap" => Ok(ValidatedHour {
            segment_id,
            kind: kind.into(),
            file: None,
            sha256: None,
            reason: Some(required_reason_text(row, "reason", context)?.to_string()),
            evidence_id: Some(required_evidence_id(row, "evidence_id", context)?.to_string()),
            ownership: Value::Null,
        }),
        other => Err(reject(format!(
            "{context} has unknown coverage discriminant {other:?}"
        ))),
    }
}

fn insert_unique(
    map: &mut BTreeMap<i64, ValidatedHour>,
    hour: i64,
    value: ValidatedHour,
    label: &str,
) -> Result<(), ContainerError> {
    if map.insert(hour, value).is_some() {
        return Err(reject(format!("duplicate visible hour {hour} in {label}")));
    }
    Ok(())
}

fn validate_owner(
    row: &Value,
    campaign_key_digest: &str,
    play_order: u64,
    session_nonce: &str,
    process_start_identity: &str,
    in_session_ordinal: u32,
    epoch_token: &str,
    context: &str,
) -> Result<Value, ContainerError> {
    let owner = row
        .get("owner")
        .ok_or_else(|| reject(format!("{context}.owner is mandatory")))?;
    required_object(owner, &format!("{context}.owner"))?;
    let owner_context = format!("{context}.owner");
    let actual_campaign = required_sha256(owner, "campaign_key_digest", &owner_context)?;
    if actual_campaign != campaign_key_digest
        || required_u64(owner, "play_order", &owner_context)? != play_order
        || required_string(owner, "session_nonce", &owner_context)? != session_nonce
        || required_string(owner, "process_start_identity", &owner_context)?
            != process_start_identity
        || required_u32(owner, "in_session_ordinal", &owner_context)? != in_session_ordinal
        || required_string(owner, "epoch_token", &owner_context)? != epoch_token
    {
        return Err(reject(format!(
            "{context}.owner disagrees with exact segment/ticket identity"
        )));
    }
    Ok(owner.clone())
}

fn selected_ownership_value(row: &Value, context: &str) -> Result<Value, ContainerError> {
    let owner = row
        .get("owner")
        .ok_or_else(|| reject(format!("{context}.owner is mandatory")))?
        .clone();
    required_object(&owner, &format!("{context}.owner"))?;
    let selected = row.get("selected_observation_id");
    if selected.is_none() {
        return Ok(json!({"owner": owner}));
    }
    if row.get("selection_rule").and_then(Value::as_str) != Some("greatest_record_sequence_v1") {
        return Err(reject(format!(
            "{context}.selection_rule is unknown/missing"
        )));
    }
    let observation_id = required_evidence_id(row, "selected_observation_id", context)?;
    let record_bounds = required_u64_pair(row, "source_record_seq_bounds", context)?;
    let byte_bounds = required_u64_pair(row, "source_byte_bounds", context)?;
    if record_bounds.0 > record_bounds.1 || byte_bounds.0 >= byte_bounds.1 {
        return Err(reject(format!(
            "{context} selected observation bounds are invalid"
        )));
    }
    Ok(json!({
        "owner": owner,
        "selection_rule": "greatest_record_sequence_v1",
        "selected_observation_id": observation_id,
        "source_record_seq_bounds": [record_bounds.0, record_bounds.1],
        "source_byte_bounds": [byte_bounds.0, byte_bounds.1],
    }))
}

fn validate_observed_hour(
    row: &Value,
    disposition: &mut ValidatedHour,
    campaign_key_digest: &str,
    play_order: u64,
    session_nonce: &str,
    process_start_identity: &str,
    in_session_ordinal: u32,
    epoch_token: &str,
    segment_record_bounds: (u64, u64),
    segment_byte_bounds: (u64, u64),
    all_observation_bounds: &mut Vec<((u64, u64), (u64, u64), i64, String)>,
    context: &str,
) -> Result<(), ContainerError> {
    let count = required_u64(row, "observation_count", context)?;
    let observations = required_array(row, "observations", context)?;
    if count == 0 || count as usize != observations.len() {
        return Err(reject(format!(
            "{context}.observation_count disagrees with retained observations"
        )));
    }
    if row.get("selection_rule").and_then(Value::as_str) != Some("greatest_record_sequence_v1") {
        return Err(reject(format!(
            "{context}.selection_rule is unknown/missing"
        )));
    }
    let selected_id = required_evidence_id(row, "selected_observation_id", context)?;
    let top_owner = validate_owner(
        row,
        campaign_key_digest,
        play_order,
        session_nonce,
        process_start_identity,
        in_session_ordinal,
        epoch_token,
        context,
    )?;
    let top_record = required_u64_pair(row, "source_record_seq_bounds", context)?;
    let top_byte = required_u64_pair(row, "source_byte_bounds", context)?;
    let mut previous_record_end = None;
    let mut previous_byte_end = None;
    let mut ids = BTreeSet::new();
    let mut selected_signature = None;
    for (ordinal, observation) in observations.iter().enumerate() {
        let observation_context = format!("{context}.observations[{ordinal}]");
        required_object(observation, &observation_context)?;
        let observation_id =
            required_evidence_id(observation, "observation_id", &observation_context)?;
        if !ids.insert(observation_id.to_string()) {
            return Err(reject(format!(
                "{context} has duplicate observation evidence"
            )));
        }
        let owner = validate_owner(
            observation,
            campaign_key_digest,
            play_order,
            session_nonce,
            process_start_identity,
            in_session_ordinal,
            epoch_token,
            &observation_context,
        )?;
        let record_bounds =
            required_u64_pair(observation, "record_seq_bounds", &observation_context)?;
        let byte_bounds = required_u64_pair(observation, "byte_bounds", &observation_context)?;
        if record_bounds.0 > record_bounds.1
            || byte_bounds.0 >= byte_bounds.1
            || record_bounds.0 < segment_record_bounds.0
            || record_bounds.1 > segment_record_bounds.1
            || byte_bounds.0 < segment_byte_bounds.0
            || byte_bounds.1 > segment_byte_bounds.1
            || previous_record_end.is_some_and(|end| record_bounds.0 <= end)
            || previous_byte_end.is_some_and(|end| byte_bounds.0 < end)
        {
            return Err(reject(format!(
                "{context} observations have ambiguous/overlapping source order"
            )));
        }
        previous_record_end = Some(record_bounds.1);
        previous_byte_end = Some(byte_bounds.1);
        all_observation_bounds.push((
            record_bounds,
            byte_bounds,
            required_i64(row, "hour_index", context)?,
            observation_id.to_string(),
        ));
        let kind = required_string(observation, "kind", &observation_context)?;
        let signature = match kind {
            "snapshot" => json!({
                "kind": kind,
                "sha256": required_sha256(observation, "sha256", &observation_context)?,
            }),
            "capture_failed" | "coalesced" | "gap" => {
                let evidence =
                    required_evidence_id(observation, "evidence_id", &observation_context)?;
                if evidence != observation_id {
                    return Err(reject(format!(
                        "{observation_context} disposition evidence disagrees with observation id"
                    )));
                }
                json!({
                    "kind": kind,
                    "reason": required_reason_text(observation, "reason", &observation_context)?,
                    "evidence_id": evidence,
                })
            }
            other => {
                return Err(reject(format!(
                    "{observation_context} has unknown coverage discriminant {other:?}"
                )));
            }
        };
        selected_signature = Some((
            observation_id.to_string(),
            owner,
            record_bounds,
            byte_bounds,
            signature,
        ));
    }
    let (last_id, last_owner, last_record, last_byte, signature) = selected_signature.unwrap();
    let top_signature = match disposition.kind.as_str() {
        "snapshot" => json!({"kind":"snapshot", "sha256": disposition.sha256}),
        _ => json!({
            "kind": disposition.kind,
            "reason": disposition.reason,
            "evidence_id": disposition.evidence_id,
        }),
    };
    if selected_id != last_id
        || top_owner != last_owner
        || top_record != last_record
        || top_byte != last_byte
        || top_signature != signature
    {
        return Err(reject(format!(
            "{context} selected observation is not the last complete source-sequence observation"
        )));
    }
    disposition.ownership = selected_ownership_value(row, context)?;
    Ok(())
}

fn validate_raw_sources(
    graph: &Value,
    campaign_key_digest: &str,
    game_id: &str,
    by_path: &HashMap<String, EntryRec>,
) -> Result<HashMap<String, ValidatedRawSource>, ContainerError> {
    let rows = required_array(graph, "raw_sources", "segments.json")?;
    if rows.is_empty() {
        return Err(reject("segments.json.raw_sources cannot be empty"));
    }
    let mut sources = HashMap::new();
    let mut source_paths = HashMap::new();
    for (ordinal, row) in rows.iter().enumerate() {
        let context = format!("segments.json.raw_sources[{ordinal}]");
        required_object(row, &context)?;
        let source_key = required_sha256(row, "source_key", &context)?;
        let session_nonce = required_string(row, "session_nonce", &context)?.to_string();
        let process_start_identity =
            required_string(row, "process_start_identity", &context)?.to_string();
        let session_path = required_string(row, "session_path", &context)?.to_string();
        let canonical_path = normalize_stitch_source_path(&session_path).map_err(|reason| {
            reject(format!(
                "{context}.session_path is not canonicalizable: {reason}"
            ))
        })?;
        let stored_digest = required_sha256(row, "stored_digest", &context)?;
        let decoded_digest = required_sha256(row, "decoded_digest", &context)?;
        if source_key
            != raw_source_key(
                &session_nonce,
                &process_start_identity,
                &stored_digest,
                &decoded_digest,
            )
        {
            return Err(reject(format!(
                "{context}.source_key disagrees with session/process/digests"
            )));
        }
        if let Some(previous_key) = source_paths.insert(canonical_path.clone(), source_key.clone())
        {
            if previous_key != source_key {
                return Err(reject(format!(
                    "{context}.session_path aliases distinct raw source identities"
                )));
            }
        }
        let source_record = required_u64_pair(row, "record_seq_bounds", &context)?;
        let source_byte = required_u64_pair(row, "byte_bounds", &context)?;
        if source_record.0 > source_record.1 || source_byte.0 >= source_byte.1 {
            return Err(reject(format!("{context} has invalid source bounds")));
        }
        let manifest_path = format!("payload/{game_id}/{session_path}");
        let storage = row
            .get("storage")
            .ok_or_else(|| reject(format!("{context}.storage is mandatory")))?;
        required_object(storage, &format!("{context}.storage"))?;
        let storage_kind = required_string(storage, "kind", &format!("{context}.storage"))?;
        let embedded_owner = match storage_kind {
            "standalone" => None,
            "embedded_campaign" => Some(required_sha256(
                storage,
                "campaign_key_digest",
                &format!("{context}.storage"),
            )?),
            other => {
                return Err(reject(format!(
                    "{context}.storage has unknown kind {other:?}"
                )));
            }
        };
        if embedded_owner
            .as_ref()
            .is_some_and(|owner| owner != campaign_key_digest)
        {
            return Err(reject(format!(
                "{context} embedded owner disagrees with stitched campaign"
            )));
        }
        match storage_kind {
            "standalone" => {
                if by_path.contains_key(&manifest_path) {
                    return Err(reject(format!(
                        "{context} standalone raw source must not be embedded in a campaign container"
                    )));
                }
            }
            "embedded_campaign" => {
                let entry = by_path.get(&manifest_path).ok_or_else(|| {
                    reject(format!("{context} references missing embedded raw source"))
                })?;
                if entry.expected_sha256 != stored_digest {
                    return Err(reject(format!(
                        "{context} raw-source stored digest disagrees with manifest"
                    )));
                }
            }
            _ => unreachable!("storage kind validated above"),
        }

        let participation_rows = required_array(row, "participations", &context)?;
        if participation_rows.is_empty() {
            return Err(reject(format!("{context}.participations cannot be empty")));
        }
        let mut participations = HashMap::new();
        let mut previous_record_end: Option<u64> = None;
        let mut previous_byte_end: Option<u64> = None;
        for (participation_ordinal, participation) in participation_rows.iter().enumerate() {
            let participation_context =
                format!("{context}.participations[{participation_ordinal}]");
            required_object(participation, &participation_context)?;
            let participation_id =
                required_sha256(participation, "participation_id", &participation_context)?;
            let mut participation_binding = participation.clone();
            participation_binding
                .as_object_mut()
                .expect("object validated above")
                .remove("participation_id");
            let mut canonical_binding = String::new();
            crate::canon::write_canonical(&participation_binding, &mut canonical_binding);
            if Sha256::hex(canonical_binding.as_bytes()) != participation_id {
                return Err(reject(format!(
                    "{participation_context}.participation_id disagrees with exact binding"
                )));
            }
            let ticket = participation
                .get("ticket")
                .ok_or_else(|| reject(format!("{participation_context}.ticket is mandatory")))?;
            required_object(ticket, &format!("{participation_context}.ticket"))?;
            let ticket_context = format!("{participation_context}.ticket");
            let participation_campaign =
                required_sha256(ticket, "campaign_key_digest", &ticket_context)?;
            let play_order = required_u64(ticket, "play_order", &ticket_context)?;
            let ticket_session =
                required_string(ticket, "session_nonce", &ticket_context)?.to_string();
            let ticket_process =
                required_string(ticket, "process_start_identity", &ticket_context)?.to_string();
            if ticket_session != session_nonce || ticket_process != process_start_identity {
                return Err(reject(format!(
                    "{participation_context} has foreign session/process ownership"
                )));
            }
            let scheme_major = required_u32(participation, "scheme_major", &participation_context)?;
            if scheme_major == 0 {
                return Err(reject(format!(
                    "{participation_context}.scheme_major must be positive"
                )));
            }
            let identity_evidence_digest = required_sha256(
                participation,
                "identity_evidence_digest",
                &participation_context,
            )?;
            let record_bounds =
                required_u64_pair(participation, "record_seq_bounds", &participation_context)?;
            let byte_bounds =
                required_u64_pair(participation, "byte_bounds", &participation_context)?;
            if record_bounds.0 > record_bounds.1
                || byte_bounds.0 >= byte_bounds.1
                || record_bounds.0 < source_record.0
                || record_bounds.1 > source_record.1
                || byte_bounds.0 < source_byte.0
                || byte_bounds.1 > source_byte.1
            {
                return Err(reject(format!(
                    "{participation_context} has overlapping/invalid source bounds"
                )));
            }
            match (previous_record_end, previous_byte_end) {
                (None, None) => {
                    if record_bounds.0 != source_record.0 || byte_bounds.0 != source_byte.0 {
                        return Err(reject(format!(
                            "{context}.participations do not start at the raw-source extent"
                        )));
                    }
                }
                (Some(record_end), Some(byte_end)) => {
                    if record_end.checked_add(1) != Some(record_bounds.0)
                        || byte_end != byte_bounds.0
                    {
                        return Err(reject(format!(
                            "{context}.participations do not form one exact contiguous source partition"
                        )));
                    }
                }
                _ => unreachable!("record and byte participation cursors advance together"),
            }
            if embedded_owner
                .as_ref()
                .is_some_and(|owner| owner != &participation_campaign)
            {
                return Err(reject(format!(
                    "{context} embeds cross-campaign participation"
                )));
            }
            previous_record_end = Some(record_bounds.1);
            previous_byte_end = Some(byte_bounds.1);
            if participations
                .insert(
                    participation_id.clone(),
                    ValidatedParticipation {
                        campaign_key_digest: participation_campaign,
                        scheme_major,
                        identity_evidence_digest,
                        play_order,
                        session_nonce: ticket_session,
                        process_start_identity: ticket_process,
                        record_seq_bounds: record_bounds,
                        byte_bounds,
                    },
                )
                .is_some()
            {
                return Err(reject(format!("{context} has duplicate participation ids")));
            }
        }
        if previous_record_end != Some(source_record.1) || previous_byte_end != Some(source_byte.1)
        {
            return Err(reject(format!(
                "{context}.participations do not cover the raw-source tail"
            )));
        }
        if sources
            .insert(
                source_key,
                ValidatedRawSource {
                    session_nonce,
                    process_start_identity,
                    session_path: canonical_path,
                    stored_digest,
                    decoded_digest,
                    record_seq_bounds: source_record,
                    byte_bounds: source_byte,
                    participations,
                },
            )
            .is_some()
        {
            return Err(reject("segments.json has duplicate raw source keys"));
        }
    }
    Ok(sources)
}

/// Validates the whole stitched (v2) graph. On success returns every retained
/// snapshot file name the per-segment indexes authorize (visible AND
/// superseded rollback snapshots), for [`Container::load_file`]'s
/// index-authority check.
fn validate_stitched_v2(
    archive: &ContainerArchive,
    by_path: &mut HashMap<String, EntryRec>,
    manifest: &Value,
    game_id: &str,
    index: &Value,
) -> Result<BTreeSet<String>, ContainerError> {
    required_object(index, "index")?;
    if index.get("index_schema").and_then(Value::as_str) != Some("stitched-main-v1") {
        return Err(reject("v2 index.index_schema must be \"stitched-main-v1\""));
    }
    if index.get("segments_file").and_then(Value::as_str) != Some("segments.json") {
        return Err(reject("v2 index.segments_file must be \"segments.json\""));
    }

    let graph_path = format!("payload/{game_id}/segments.json");
    if !by_path.contains_key(&graph_path) {
        return Err(reject("v2 container is missing segments.json"));
    }
    let graph_bytes = verify_and_read(archive, by_path, &graph_path)?;
    // Strict (duplicate-member-rejecting) parse: a segments.json carrying a
    // duplicated member must be a typed refusal, never last-wins collapse.
    let graph: Value = crate::parse_json_strict(&graph_bytes)?;
    required_object(&graph, "segments.json")?;
    let declared_stitch_digest = required_sha256(&graph, "stitch_input_digest", "segments.json")?;
    let declared_ledger_digest =
        required_sha256(&graph, "play_order_ledger_digest", "segments.json")?;
    for key in [
        "fold_version",
        "schema_version",
        "canonical_writer_version",
        "packer_codec_version",
    ] {
        required_string(&graph, key, "segments.json")?;
    }
    let publication = graph
        .get("publication")
        .ok_or_else(|| reject("segments.json.publication is mandatory"))?;
    required_object(publication, "segments.json.publication")?;
    let declared_created =
        required_string(publication, "created_utc", "segments.json.publication")?;
    let declared_compression = required_string(
        publication,
        "compression_method",
        "segments.json.publication",
    )?;
    let declared_entry_order =
        required_string(publication, "entry_order", "segments.json.publication")?;
    let compression_parameters = publication
        .get("compression_parameters")
        .and_then(Value::as_object)
        .filter(|parameters| {
            !parameters.is_empty()
                && parameters.iter().all(|(key, value)| {
                    !key.is_empty() && value.as_str().is_some_and(|v| !v.is_empty())
                })
        })
        .ok_or_else(|| {
            reject("segments.json.publication.compression_parameters is missing/invalid")
        })?;
    let manifest_created = required_string(manifest, "created_utc", "manifest")?;
    if declared_created != manifest_created {
        return Err(reject(
            "segments.json.publication.created_utc disagrees with manifest.created_utc",
        ));
    }
    let graph_schema_version = required_string(&graph, "schema_version", "segments.json")?;
    if Some(graph_schema_version) != manifest.get("schema_version").and_then(Value::as_str) {
        return Err(reject(
            "segments.json.schema_version disagrees with manifest.schema_version",
        ));
    }
    let manifest_build = manifest
        .get("build")
        .ok_or_else(|| reject("manifest.build is mandatory for a stitched container"))?;
    required_object(manifest_build, "manifest.build")?;
    let manifest_build_exe_sha256 =
        required_sha256(manifest_build, "exe_sha256", "manifest.build")?;
    let manifest_mod_identity = manifest
        .get("mod_identity")
        .ok_or_else(|| reject("manifest.mod_identity is mandatory for a stitched container"))?;
    required_object(manifest_mod_identity, "manifest.mod_identity")?;
    let manifest_modset_id =
        required_string(manifest_mod_identity, "modset_id", "manifest.mod_identity")?;
    if declared_compression != "store"
        || compression_parameters != json!({"level":"none"}).as_object().unwrap()
    {
        return Err(reject(
            "v2 publication compression must be independently-verifiable store/level=none",
        ));
    }
    if archive
        .entries_meta()
        .iter()
        .any(|entry| entry.method != ZIP_METHOD_STORE)
    {
        return Err(reject(
            "segments.json publication compression disagrees with actual ZIP methods",
        ));
    }
    if declared_entry_order != STITCH_ENTRY_ORDER {
        return Err(reject(
            "segments.json.publication.entry_order is unsupported/ambiguous",
        ));
    }
    let entry_meta = archive.entries_meta();
    let actual_order: Vec<&str> = entry_meta.iter().map(|entry| entry.name).collect();
    let declared_order: Vec<&str> = required_array(manifest, "entries", "manifest")?
        .iter()
        .map(|entry| required_string(entry, "path", "manifest.entries"))
        .collect::<Result<_, _>>()?;
    let mut canonical_order = declared_order.clone();
    canonical_order.sort_unstable();
    if actual_order.last().copied() != Some("manifest.json")
        || actual_order[..actual_order.len() - 1] != declared_order
        || declared_order != canonical_order
        || entry_meta
            .windows(2)
            .any(|pair| pair[0].local_header_off >= pair[1].local_header_off)
    {
        return Err(reject(
            "segments.json publication entry_order disagrees with actual ZIP order",
        ));
    }
    let campaign = graph
        .get("campaign")
        .ok_or_else(|| reject("segments.json.campaign is mandatory"))?;
    required_object(campaign, "segments.json.campaign")?;
    let campaign_scheme_major = required_u32(campaign, "scheme_major", "segments.json.campaign")?;
    let campaign_key_digest =
        required_sha256(campaign, "campaign_key_digest", "segments.json.campaign")?;
    let campaign_key = required_hex_bytes(campaign, "campaign_key", "segments.json.campaign")?;
    if Sha256::hex(&campaign_key) != campaign_key_digest {
        return Err(reject(
            "segments.json.campaign raw key disagrees with campaign_key_digest",
        ));
    }
    let graph_stream_major = graph
        .get("stream_major")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or_else(|| reject("segments.json.stream_major must be a positive integer"))?;
    let segment_rows = required_array(&graph, "segments", "segments.json")?;
    if segment_rows.is_empty() {
        return Err(reject("segments.json.segments cannot be empty"));
    }

    #[derive(Clone)]
    struct TicketAuthority {
        array_position: usize,
        segment_id: u32,
        play_order: u64,
        in_session_ordinal: u32,
        session_nonce: String,
        process_start_identity: String,
        epoch_token: String,
    }
    let mut authorities = Vec::with_capacity(segment_rows.len());
    let mut seen_ranks = BTreeSet::new();
    let mut order_to_ticket: BTreeMap<u64, (String, String)> = BTreeMap::new();
    for (array_position, segment) in segment_rows.iter().enumerate() {
        let context = format!("segments.json.segments[{array_position}]");
        required_object(segment, &context)?;
        let segment_id = required_u32(segment, "segment_id", &context)?;
        let play_order = required_u64(segment, "play_order", &context)?;
        let in_session_ordinal = required_u32(segment, "in_session_ordinal", &context)?;
        let session_nonce = required_string(segment, "session_nonce", &context)?.to_string();
        let process_start_identity =
            required_string(segment, "process_start_identity", &context)?.to_string();
        let epoch_token = required_string(segment, "epoch_token", &context)?.to_string();
        let ticket = segment
            .get("ticket")
            .ok_or_else(|| reject(format!("{context}.ticket is mandatory")))?;
        let ticket_context = format!("{context}.ticket");
        required_object(ticket, &ticket_context)?;
        if required_sha256(ticket, "campaign_key_digest", &ticket_context)? != campaign_key_digest
            || required_u64(ticket, "play_order", &ticket_context)? != play_order
            || required_string(ticket, "session_nonce", &ticket_context)? != session_nonce
            || required_string(ticket, "process_start_identity", &ticket_context)?
                != process_start_identity
        {
            return Err(reject(format!(
                "{context}.ticket disagrees with exact campaign/segment owner"
            )));
        }
        if !seen_ranks.insert((play_order, in_session_ordinal)) {
            return Err(reject(format!(
                "duplicate (play_order {play_order}, in_session_ordinal {in_session_ordinal}); ticket authority is ambiguous"
            )));
        }
        let ticket_identity = (session_nonce.clone(), process_start_identity.clone());
        if order_to_ticket
            .insert(play_order, ticket_identity.clone())
            .is_some_and(|previous| previous != ticket_identity)
        {
            return Err(reject(format!(
                "play_order {play_order} maps to conflicting AppendTickets; ticket authority is ambiguous"
            )));
        }
        authorities.push(TicketAuthority {
            array_position,
            segment_id,
            play_order,
            in_session_ordinal,
            session_nonce,
            process_start_identity,
            epoch_token,
        });
    }
    authorities.sort_by_key(|authority| (authority.play_order, authority.in_session_ordinal));
    for (rank, authority) in authorities.iter().enumerate() {
        if authority.array_position != rank || authority.segment_id as usize != rank {
            return Err(reject(format!(
                "segments.json array/segment_id order disagrees with AppendTicket play_order rank {rank}"
            )));
        }
    }
    let ledger = Value::Array(
        authorities
            .iter()
            .map(|authority| {
                json!({
                    "campaign_key_digest": campaign_key_digest,
                    "play_order": authority.play_order,
                    "in_session_ordinal": authority.in_session_ordinal,
                    "session_nonce": authority.session_nonce,
                    "process_start_identity": authority.process_start_identity,
                    "epoch_token": authority.epoch_token,
                })
            })
            .collect(),
    );
    let mut canonical_ledger = String::new();
    crate::canon::write_canonical(&ledger, &mut canonical_ledger);
    let recomputed_ledger_digest = Sha256::hex(canonical_ledger.as_bytes());
    if declared_ledger_digest != recomputed_ledger_digest {
        return Err(reject(format!(
            "segments.json.play_order_ledger_digest mismatch (recomputed {recomputed_ledger_digest}, declared {declared_ledger_digest})"
        )));
    }

    let raw_sources = validate_raw_sources(&graph, &campaign_key_digest, game_id, by_path)?;
    let mut segment_hours: Vec<BTreeMap<i64, ValidatedHour>> = Vec::new();
    let mut serialized_segment_indexes = Vec::with_capacity(segment_rows.len());
    let mut visible = BTreeMap::new();
    let mut insertion_main: BTreeMap<i64, u32> = BTreeMap::new();
    let mut stitched_compatibility_class: Option<String> = None;
    let mut segment_extents_by_source: HashMap<String, Vec<((u64, u64), (u64, u64), u32)>> =
        HashMap::new();
    for (expected_id, authority) in authorities.iter().enumerate() {
        let segment = &segment_rows[authority.array_position];
        let context = format!("segments.json.segments[{expected_id}]");
        required_object(segment, &context)?;
        let segment_id = required_u32(segment, "segment_id", &context)?;
        if segment_id as usize != expected_id {
            return Err(reject(format!(
                "{context}.segment_id must be dense play order {expected_id}"
            )));
        }
        let play_order = required_u64(segment, "play_order", &context)?;
        let in_session_ordinal = required_u32(segment, "in_session_ordinal", &context)?;
        let session_nonce = required_string(segment, "session_nonce", &context)?.to_string();
        let process_start_identity =
            required_string(segment, "process_start_identity", &context)?.to_string();
        let epoch_token = required_string(segment, "epoch_token", &context)?.to_string();
        let ticket = segment
            .get("ticket")
            .ok_or_else(|| reject(format!("{context}.ticket is mandatory")))?;
        required_object(ticket, &format!("{context}.ticket"))?;
        let ticket_context = format!("{context}.ticket");
        if required_sha256(ticket, "campaign_key_digest", &ticket_context)? != campaign_key_digest
            || required_u64(ticket, "play_order", &ticket_context)? != play_order
            || required_string(ticket, "session_nonce", &ticket_context)? != session_nonce
            || required_string(ticket, "process_start_identity", &ticket_context)?
                != process_start_identity
        {
            return Err(reject(format!(
                "{context}.ticket disagrees with exact campaign/segment owner"
            )));
        }
        let identity = segment
            .get("identity")
            .ok_or_else(|| reject(format!("{context}.identity is mandatory")))?;
        required_object(identity, &format!("{context}.identity"))?;
        let identity_context = format!("{context}.identity");
        let identity_evidence_digest =
            required_sha256(identity, "evidence_digest", &identity_context)?;
        if required_u32(identity, "scheme_major", &identity_context)? != campaign_scheme_major
            || required_sha256(identity, "campaign_key_digest", &identity_context)?
                != campaign_key_digest
        {
            return Err(reject(format!(
                "{context}.identity disagrees with stitched campaign"
            )));
        }
        let build_exe_sha256 = required_sha256(segment, "build_exe_sha256", &context)?;
        let modset_id = required_string(segment, "modset_id", &context)?;
        let segment_schema_version = required_string(segment, "schema_version", &context)?;
        let static_catalog_digest = required_sha256(segment, "static_catalog_digest", &context)?;
        if required_u64(segment, "stream_major", &context)? != graph_stream_major
            || segment_schema_version != graph_schema_version
            || build_exe_sha256 != manifest_build_exe_sha256
            || modset_id != manifest_modset_id
        {
            return Err(reject(format!(
                "{context} stream/schema binding disagrees with stitched publication"
            )));
        }
        let compatibility = segment
            .get("compatibility")
            .ok_or_else(|| reject(format!("{context}.compatibility is mandatory")))?;
        let compatibility_context = format!("{context}.compatibility");
        required_object(compatibility, &compatibility_context)?;
        let compatibility_class =
            required_string(compatibility, "compatibility_class", &compatibility_context)?;
        if required_string(compatibility, "kind", &compatibility_context)? != "proven"
            || required_evidence_id(compatibility, "evidence_id", &compatibility_context)?
                .is_empty()
            || required_sha256(compatibility, "build_exe_sha256", &compatibility_context)?
                != build_exe_sha256
            || required_string(compatibility, "modset_id", &compatibility_context)? != modset_id
            || required_string(compatibility, "schema_version", &compatibility_context)?
                != segment_schema_version
            || required_sha256(
                compatibility,
                "static_catalog_digest",
                &compatibility_context,
            )? != static_catalog_digest
        {
            return Err(reject(format!(
                "{compatibility_context} is not an exact proven segment binding"
            )));
        }
        match &stitched_compatibility_class {
            None => stitched_compatibility_class = Some(compatibility_class.to_string()),
            Some(previous) if previous != compatibility_class => {
                return Err(reject(format!(
                    "{compatibility_context}.compatibility_class conflicts with another segment"
                )));
            }
            Some(_) => {}
        }
        let source_ref = segment
            .get("source_ref")
            .ok_or_else(|| reject(format!("{context}.source_ref is mandatory")))?;
        required_object(source_ref, &format!("{context}.source_ref"))?;
        let source_context = format!("{context}.source_ref");
        let source_key = required_sha256(source_ref, "source_key", &source_context)?;
        let participation_id = required_sha256(source_ref, "participation_id", &source_context)?;
        let raw_source = raw_sources
            .get(&source_key)
            .ok_or_else(|| reject(format!("{source_context} references unknown raw source")))?;
        let participation = raw_source
            .participations
            .get(&participation_id)
            .ok_or_else(|| reject(format!("{source_context} references unknown participation")))?;
        let segment_record_bounds =
            required_u64_pair(source_ref, "record_seq_bounds", &source_context)?;
        let segment_byte_bounds = required_u64_pair(source_ref, "byte_bounds", &source_context)?;
        let source_session_path = required_string(source_ref, "session_path", &source_context)?;
        let canonical_source_session_path = normalize_stitch_source_path(source_session_path)
            .map_err(|reason| {
                reject(format!(
                    "{source_context}.session_path is not canonicalizable: {reason}"
                ))
            })?;
        if canonical_source_session_path != raw_source.session_path
            || required_sha256(source_ref, "stored_digest", &source_context)?
                != raw_source.stored_digest
            || required_sha256(source_ref, "decoded_digest", &source_context)?
                != raw_source.decoded_digest
            || raw_source.session_nonce != session_nonce
            || raw_source.process_start_identity != process_start_identity
            || participation.campaign_key_digest != campaign_key_digest
            || participation.scheme_major != campaign_scheme_major
            || participation.identity_evidence_digest != identity_evidence_digest
            || participation.play_order != play_order
            || participation.session_nonce != session_nonce
            || participation.process_start_identity != process_start_identity
            || segment_record_bounds.0 > segment_record_bounds.1
            || segment_byte_bounds.0 >= segment_byte_bounds.1
            || segment_record_bounds.0 < participation.record_seq_bounds.0
            || segment_record_bounds.1 > participation.record_seq_bounds.1
            || segment_byte_bounds.0 < participation.byte_bounds.0
            || segment_byte_bounds.1 > participation.byte_bounds.1
            || segment_record_bounds.0 < raw_source.record_seq_bounds.0
            || segment_record_bounds.1 > raw_source.record_seq_bounds.1
            || segment_byte_bounds.0 < raw_source.byte_bounds.0
            || segment_byte_bounds.1 > raw_source.byte_bounds.1
        {
            return Err(reject(format!(
                "{source_context} disagrees with exact source/participation ownership"
            )));
        }
        segment_extents_by_source
            .entry(source_key)
            .or_default()
            .push((segment_record_bounds, segment_byte_bounds, segment_id));
        let expected_index = format!("segments/s{segment_id:08}.index.json");
        if required_string(segment, "index_file", &context)? != expected_index {
            return Err(reject(format!(
                "{context} segment index reference is not canonical"
            )));
        }
        let index_path = format!("payload/{game_id}/{expected_index}");
        if !by_path.contains_key(&index_path) {
            return Err(reject(format!(
                "segment index {expected_index:?} is missing"
            )));
        }
        let bytes = verify_and_read(archive, by_path, &index_path)?;
        let segment_index: Value = crate::parse_json_strict(&bytes)?;
        required_object(&segment_index, &expected_index)?;
        serialized_segment_indexes.push(segment_index.clone());
        if required_u32(&segment_index, "segment_id", &expected_index)? != segment_id {
            return Err(reject(format!(
                "segment index {expected_index:?} id disagrees with graph"
            )));
        }
        if segment_index.get("source_ref") != Some(source_ref) {
            return Err(reject(format!(
                "segment index {expected_index:?} source_ref disagrees with graph"
            )));
        }
        let graph_transition_coverage = required_array(segment, "transition_coverage", &context)?;
        let index_transition_coverage =
            required_array(&segment_index, "transition_coverage", &expected_index)?;
        if graph_transition_coverage != index_transition_coverage {
            return Err(reject(format!(
                "segment index {expected_index:?} transition coverage disagrees with graph"
            )));
        }
        let cutover = optional_i64(segment, "cutover_hour", &context)?;
        let head = optional_i64(segment, "head_hour", &context)?;
        if optional_i64(&segment_index, "cutover_hour", &expected_index)? != cutover
            || optional_i64(&segment_index, "head_hour", &expected_index)? != head
        {
            return Err(reject(format!(
                "segment index {expected_index:?} cutover/head disagrees with graph"
            )));
        }
        let mut local = BTreeMap::new();
        let mut classified = BTreeSet::new();
        let mut all_observation_bounds = Vec::new();
        for (ordinal, row) in required_array(&segment_index, "hours", &expected_index)?
            .iter()
            .enumerate()
        {
            let row_context = format!("{expected_index}.hours[{ordinal}]");
            let hour = required_i64(row, "hour_index", &row_context)?;
            let mut disposition =
                parse_disposition(row, segment_id, game_id, by_path, &row_context, true)?;
            validate_observed_hour(
                row,
                &mut disposition,
                &campaign_key_digest,
                play_order,
                &session_nonce,
                &process_start_identity,
                in_session_ordinal,
                &epoch_token,
                segment_record_bounds,
                segment_byte_bounds,
                &mut all_observation_bounds,
                &row_context,
            )?;
            if let Some(file) = &disposition.file {
                let expected_file = format!("s{segment_id:08}.{hour:012}.snapshot.json");
                if file != &expected_file {
                    return Err(reject(format!(
                        "{row_context} filename disagrees with segment/hour"
                    )));
                }
            }
            if local.insert(hour, disposition).is_some() {
                return Err(reject(format!("duplicate hour {hour} in {expected_index}")));
            }
        }
        all_observation_bounds.sort();
        for adjacent in all_observation_bounds.windows(2) {
            if adjacent[1].0 .0 <= adjacent[0].0 .1 || adjacent[1].1 .0 < adjacent[0].1 .1 {
                return Err(reject(format!(
                    "{expected_index} observation evidence overlaps across hours {} and {}",
                    adjacent[0].2, adjacent[1].2
                )));
            }
        }
        let computed_empty_recovery = cutover.is_none() && head.is_none() && local.is_empty();
        let declared_empty_recovery = segment
            .get("empty_recovery")
            .and_then(Value::as_bool)
            .ok_or_else(|| reject(format!("{context}.empty_recovery must be boolean")))?;
        if declared_empty_recovery != computed_empty_recovery {
            return Err(reject(format!(
                "{context}.empty_recovery disagrees with cutover/head/hour evidence"
            )));
        }
        match (cutover, head) {
            (None, None) if local.is_empty() => {}
            (Some(start), Some(end)) if start <= end => {
                // Density check WITHOUT materializing (start..=end): a hostile
                // cutover/head pair (e.g. 0..=i64::MAX) must be a fast typed
                // refusal, not an unbounded allocation. Keys are unique sorted
                // i64s, so len == width && min == start && max == end proves
                // the key set is exactly start..=end.
                let expected_len = (end as i128) - (start as i128) + 1;
                if expected_len != local.len() as i128
                    || local.keys().next() != Some(&start)
                    || local.keys().next_back() != Some(&end)
                {
                    return Err(reject(format!(
                        "truncated segment index {expected_index:?}"
                    )));
                }
            }
            _ => {
                return Err(reject(format!(
                    "segment index {expected_index:?} has invalid empty/cutover/head shape"
                )));
            }
        }
        let mut transition_hours = Vec::new();
        for (ordinal, row) in index_transition_coverage.iter().enumerate() {
            let row_context = format!("{expected_index}.transition_coverage[{ordinal}]");
            let hour = required_i64(row, "hour_index", &row_context)?;
            transition_hours.push(hour);
            let mut disposition =
                parse_disposition(row, segment_id, game_id, by_path, &row_context, false)?;
            validate_owner(
                row,
                &campaign_key_digest,
                play_order,
                &session_nonce,
                &process_start_identity,
                in_session_ordinal,
                &epoch_token,
                &row_context,
            )?;
            disposition.ownership = selected_ownership_value(row, &row_context)?;
            if local.insert(hour, disposition).is_some() {
                return Err(reject(format!(
                    "duplicate transition/observed hour {hour} in {expected_index}"
                )));
            }
        }
        let required_transition =
            insertion_main
                .keys()
                .next_back()
                .copied()
                .and_then(|previous_head| {
                    cutover.and_then(|start| {
                        (start > previous_head.saturating_add(1))
                            .then_some((previous_head.saturating_add(1), start - 1))
                    })
                });
        match required_transition {
            Some((start, end)) => {
                let expected_len = (end as i128) - (start as i128) + 1;
                let is_dense = transition_hours.len() as i128 == expected_len
                    && transition_hours.first() == Some(&start)
                    && transition_hours.last() == Some(&end)
                    && transition_hours
                        .windows(2)
                        .all(|pair| pair[0].checked_add(1) == Some(pair[1]));
                if !is_dense {
                    return Err(reject(format!(
                        "{expected_index} transition coverage must exactly account for {start}..={end}"
                    )));
                }
            }
            None if !transition_hours.is_empty() => {
                return Err(reject(format!(
                    "{expected_index} transition coverage is present without a cross-session gap/cutover"
                )));
            }
            None => {}
        }
        for (span_no, span) in required_array(segment, "visible_spans", &context)?
            .iter()
            .enumerate()
        {
            let span_context = format!("{context}.visible_spans[{span_no}]");
            let start = required_i64(span, "start_hour", &span_context)?;
            let end = required_i64(span, "end_hour", &span_context)?;
            if start > end {
                return Err(reject(format!("{span_context} has reversed bounds")));
            }
            for hour in start..=end {
                let disposition = local.get(&hour).cloned().ok_or_else(|| {
                    reject(format!("{span_context} references uncovered hour {hour}"))
                })?;
                insert_unique(
                    &mut visible,
                    hour,
                    disposition,
                    "segments.json visible spans",
                )?;
                if !classified.insert(hour) {
                    return Err(reject(format!(
                        "{span_context} overlaps another visible/superseded classification"
                    )));
                }
            }
        }
        for (span_no, span) in required_array(segment, "superseded_spans", &context)?
            .iter()
            .enumerate()
        {
            let span_context = format!("{context}.superseded_spans[{span_no}]");
            let start = required_i64(span, "start_hour", &span_context)?;
            let end = required_i64(span, "end_hour", &span_context)?;
            let by = required_u32(span, "superseded_by", &span_context)?;
            if start > end || by <= segment_id || by as usize >= segment_rows.len() {
                return Err(reject(format!("{span_context} has invalid bounds/owner")));
            }
            for hour in start..=end {
                if !local.contains_key(&hour) {
                    return Err(reject(format!(
                        "{span_context} references uncovered hour {hour}"
                    )));
                }
                if !classified.insert(hour) {
                    return Err(reject(format!(
                        "{span_context} overlaps another visible/superseded classification"
                    )));
                }
            }
        }
        if classified.iter().copied().collect::<Vec<_>>()
            != local.keys().copied().collect::<Vec<_>>()
        {
            return Err(reject(format!(
                "{context} visible/superseded spans do not classify every retained hour"
            )));
        }
        let expected_parent = cutover.and_then(|start| {
            insertion_main
                .range(..start)
                .next_back()
                .map(|(_, owner)| *owner)
        });
        if optional_u32(segment, "parent_segment", &context)? != expected_parent {
            return Err(reject(format!(
                "{context}.parent_segment was not captured at insertion time"
            )));
        }
        for hour in transition_hours {
            insertion_main.insert(hour, segment_id);
        }
        if let (Some(start), Some(end)) = (cutover, head) {
            let removed: Vec<i64> = insertion_main
                .range(start..)
                .map(|(hour, _)| *hour)
                .collect();
            for hour in removed {
                insertion_main.remove(&hour);
            }
            for hour in start..=end {
                insertion_main.insert(hour, segment_id);
            }
        }
        segment_hours.push(local);
    }

    for (source_key, extents) in &mut segment_extents_by_source {
        extents.sort();
        for adjacent in extents.windows(2) {
            if adjacent[1].0 .0 <= adjacent[0].0 .1 || adjacent[1].1 .0 < adjacent[0].1 .1 {
                return Err(reject(format!(
                    "raw source {source_key} has overlapping segment extents {} and {}",
                    adjacent[0].2, adjacent[1].2
                )));
            }
        }
    }
    if segment_extents_by_source.len() != raw_sources.len() {
        return Err(reject(
            "segments.json contains an unreferenced raw source descriptor",
        ));
    }

    let mut index_main = BTreeMap::new();
    let mut previous_snapshot: Option<i64> = None;
    for (ordinal, row) in required_array(index, "snapshots", "index")?
        .iter()
        .enumerate()
    {
        let context = format!("index.snapshots[{ordinal}]");
        let hour = required_i64(row, "hour_index", &context)?;
        if previous_snapshot.is_some_and(|previous| hour <= previous) {
            return Err(reject(format!(
                "duplicate visible hour {hour} in index snapshots"
            )));
        }
        previous_snapshot = Some(hour);
        if required_string(row, "kind", &context)? != "snapshot" {
            return Err(reject(format!("{context}.kind must be snapshot")));
        }
        let file = required_string(row, "file", &context)?.to_string();
        let sha256 = required_sha256(row, "sha256", &context)?;
        let ownership = selected_ownership_value(row, &context)?;
        let graph_hour = visible
            .get(&hour)
            .ok_or_else(|| reject(format!("{context} is absent from graph visible spans")))?;
        if graph_hour.kind != "snapshot"
            || graph_hour.file.as_deref() != Some(file.as_str())
            || graph_hour.sha256.as_deref() != Some(sha256.as_str())
            || graph_hour.ownership != ownership
        {
            return Err(reject(format!("{context} graph/index disagreement")));
        }
        insert_unique(&mut index_main, hour, graph_hour.clone(), "index")?;
    }
    let mut previous_coverage_end: Option<i64> = None;
    for (ordinal, row) in required_array(index, "coverage", "index")?
        .iter()
        .enumerate()
    {
        let context = format!("index.coverage[{ordinal}]");
        let start = required_i64(row, "start_hour", &context)?;
        let end = required_i64(row, "end_hour", &context)?;
        if start > end || previous_coverage_end.is_some_and(|previous| start <= previous) {
            return Err(reject(format!(
                "{context} has overlapping/out-of-order bounds"
            )));
        }
        previous_coverage_end = Some(end);
        let segment_id = required_u32(row, "segment_id", &context)?;
        let mut disposition =
            parse_disposition(row, segment_id, game_id, by_path, &context, false)?;
        disposition.ownership = selected_ownership_value(row, &context)?;
        for hour in start..=end {
            // Any hour outside `visible` is guaranteed to fail the final
            // `index_main != visible` equality; rejecting it eagerly keeps a
            // hostile coverage span (e.g. 0..=9e18) from materializing
            // billions of map entries before that comparison can run.
            if !visible.contains_key(&hour) {
                return Err(reject(format!(
                    "{context} covers hour {hour} absent from graph visible spans"
                )));
            }
            insert_unique(&mut index_main, hour, disposition.clone(), "index")?;
        }
    }
    if index_main != visible {
        return Err(reject(
            "main index and segment graph visible coverage disagree",
        ));
    }
    let mut expected_slices: Vec<(u32, i64, i64)> = Vec::new();
    for (hour, disposition) in &visible {
        if let Some((segment_id, _, end)) = expected_slices.last_mut() {
            if *segment_id == disposition.segment_id && *end + 1 == *hour {
                *end = *hour;
                continue;
            }
        }
        expected_slices.push((disposition.segment_id, *hour, *hour));
    }
    let actual_slices: Vec<(u32, i64, i64)> =
        required_array(&graph, "main_slices", "segments.json")?
            .iter()
            .enumerate()
            .map(|(ordinal, slice)| {
                let context = format!("segments.json.main_slices[{ordinal}]");
                Ok((
                    required_u32(slice, "segment_id", &context)?,
                    required_i64(slice, "start_hour", &context)?,
                    required_i64(slice, "end_hour", &context)?,
                ))
            })
            .collect::<Result<_, ContainerError>>()?;
    if actual_slices != expected_slices {
        return Err(reject(
            "segments.json main_slices disagree with visible graph",
        ));
    }
    let expected_head = visible
        .iter()
        .next_back()
        .map(|(hour, value)| (value.segment_id, *hour));
    if parse_head(
        index.get("main_head").unwrap_or(&Value::Null),
        "index.main_head",
    )? != expected_head
        || parse_head(
            graph.get("main_head").unwrap_or(&Value::Null),
            "segments.json.main_head",
        )? != expected_head
    {
        return Err(reject("main_head graph/index disagreement"));
    }
    let recomputed_stitch_digest =
        canonical_stitch_input_digest(game_id, &graph, &serialized_segment_indexes);
    if declared_stitch_digest != recomputed_stitch_digest {
        return Err(reject(format!(
            "segments.json.stitch_input_digest mismatch (recomputed {recomputed_stitch_digest}, declared {declared_stitch_digest})"
        )));
    }
    let mut retained_snapshot_files = BTreeSet::new();
    for hours in &segment_hours {
        for disposition in hours.values() {
            if let Some(file) = &disposition.file {
                retained_snapshot_files.insert(file.clone());
            }
        }
    }
    Ok(retained_snapshot_files)
}

/// Read an entry's bytes and verify them against the manifest hash. Immutable
/// in-memory entries become hash-free after the first success; file-backed entries
/// re-authenticate each exposure. Never returns bytes whose hash did not match.
/// Verify one entry without materializing it.
///
/// The receipt is derived from the same decompression pass, so the SHA-256 this
/// compares is over exactly the bytes the entry decodes to. Only reachable for a
/// file-backed archive; a memory-backed one says so rather than pretending the
/// entry was checked.
fn verify_streamed(
    archive: &ContainerArchive,
    by_path: &mut HashMap<String, EntryRec>,
    path: &str,
) -> Result<(), ContainerError> {
    let rec = by_path
        .get_mut(path)
        .ok_or_else(|| ContainerError::Reject(format!("entry {path} not in manifest")))?;
    let (actual_sha256, _) = archive.copy_entry_to_writer(
        rec.archive_index,
        &mut std::io::sink(),
        STREAMING_MAX_CONTAINER_ENTRY_BYTES,
    )?;
    if actual_sha256 != rec.expected_sha256 {
        return Err(ContainerError::PayloadIntegrity(format!(
            "entry {path}: sha256 {actual_sha256} != manifest {}",
            rec.expected_sha256
        )));
    }
    rec.sha256_state = EntryVerificationState::Verified;
    Ok(())
}

fn verify_and_read(
    archive: &ContainerArchive,
    by_path: &mut HashMap<String, EntryRec>,
    path: &str,
) -> Result<Vec<u8>, ContainerError> {
    let rec = by_path
        .get_mut(path)
        .ok_or_else(|| ContainerError::Reject(format!("entry {path} not in manifest")))?;
    // In-memory archive bytes are immutable, so one successful hash is enough.
    // A retained file handle pins identity, not contents: another writer could
    // still alter bytes in place after open. Re-authenticate every file-backed
    // exposure, during the same decompression pass that produces the bytes.
    let calculate_sha256 =
        rec.sha256_state == EntryVerificationState::Pending || archive.requires_sha256_each_read();
    let (bytes, digest) = archive.read_entry(rec.archive_index, calculate_sha256)?;
    if calculate_sha256 {
        let got = digest.ok_or_else(|| {
            ContainerError::Reject(format!(
                "entry {path}: archive reader omitted requested SHA-256"
            ))
        })?;
        if got != rec.expected_sha256 {
            return Err(ContainerError::PayloadIntegrity(format!(
                "entry {path}: sha256 {got} != manifest {}",
                rec.expected_sha256
            )));
        }
        // Record the successful transition before the exposure guard. In-memory
        // entries may reuse it; file-backed entries still re-authenticate each
        // read because the retained handle does not make file contents immutable.
        rec.sha256_state = EntryVerificationState::Verified;
    }
    rec.require_sha256_verified(path)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{pack, pack_with, PackEntry, PackIdentity, PackOptions};
    use crate::COMPACT_STREAM_ROLE;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempContainerFile(std::path::PathBuf);

    impl TempContainerFile {
        fn write(label: &str, bytes: &[u8]) -> Self {
            let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hoi4replay_container_{label}_{}_{}.battleplan",
                std::process::id(),
                sequence
            ));
            std::fs::write(&path, bytes).expect("write temporary container");
            Self(path)
        }
    }

    impl Drop for TempContainerFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A scratch directory that is removed even when an assertion unwinds. The
    /// spool guarantees are proven by driving failure paths, and on Windows a
    /// leaked deny-delete temporary is sticky, so cleanup may not depend on a
    /// test reaching its last line.
    struct TempSpoolDir(std::path::PathBuf);

    impl TempSpoolDir {
        fn create(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "hoi4_spool_{label}_{}_{}",
                std::process::id(),
                TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create scratch spool directory");
            Self(path)
        }

        fn join<S: AsRef<std::ffi::OsStr>>(&self, name: S) -> std::path::PathBuf {
            self.0.join(name.as_ref())
        }
    }

    impl Drop for TempSpoolDir {
        fn drop(&mut self) {
            if let Ok(entries) = fs::read_dir(&self.0) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    // Never recurse through a reparse point: a junction fixture
                    // must be unlinked, never followed into its target.
                    let Ok(metadata) = fs::symlink_metadata(&path) else {
                        continue;
                    };
                    let file_type = metadata.file_type();
                    if file_type.is_symlink() {
                        // A reparse point: unlink the link itself, never
                        // descend through it.
                        if fs::remove_dir(&path).is_err() {
                            let _ = fs::remove_file(&path);
                        }
                    } else if file_type.is_dir() {
                        let _ = fs::remove_dir_all(&path);
                    } else {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
            let _ = fs::remove_dir(&self.0);
        }
    }

    /// A destination pathname that deliberately lies OUTSIDE a `TempSpoolDir`
    /// -- a volume root, the process working directory, a scratch directory's
    /// parent. Those tests assert a refusal, so if the refusal ever regresses
    /// the destination is genuinely published, and the assertion panics before
    /// any trailing cleanup could run. Removal must therefore be on drop.
    struct TempSpoolPath(std::path::PathBuf);

    impl Drop for TempSpoolPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    /// One file-backed container carrying `payload/g/compact.bin`. Every spool
    /// guarantee below needs a retained-handle container: the transactional
    /// spool refuses an in-memory archive before it reaches any of them.
    /// `verify_all` now picks a route by entry size, so the streaming route has
    /// to be the same check — not a cheaper one that happens to be reachable
    /// only for the entries too big to hold.
    ///
    /// The size crossing itself needs a >2 GiB entry and is proved on capture
    /// t32's real 2,209,767,138-byte checkpoint cache by
    /// `t32_oversized_checkpoint_cache_seals_and_reconstructs`. What this proves
    /// is the part that test cannot: that the two routes AGREE — both accept an
    /// intact container, and both refuse the same corrupted one, naming the same
    /// entry.
    ///
    /// Production change that would make this fail: letting `verify_streamed`
    /// skip the digest compare, forget to mark the entry verified, or accept an
    /// entry whose bytes no longer hash to the manifest.
    #[test]
    fn the_streaming_verification_route_is_not_weaker_than_the_materializing_one() {
        let bytes = compact_container_bytes(b"route equivalence payload");
        let archive = TempContainerFile::write("route_equivalence", &bytes);

        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        container.verify_all().expect("intact container verifies");
        drop(container);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        container
            .verify_all_streaming()
            .expect("intact container verifies streamed");
        drop(container);

        // Corrupt one STORE'd payload byte in place. Both routes decompress the
        // same bytes, so both must refuse, and both must name the entry.
        let mut corrupted = bytes.clone();
        let offset = corrupted
            .windows(b"route equivalence payload".len())
            .position(|window| window == b"route equivalence payload")
            .expect("the stored payload is present");
        corrupted[offset] ^= 0x01;
        let archive = TempContainerFile::write("route_equivalence_bad", &corrupted);

        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let materializing = container
            .verify_all()
            .expect_err("a corrupted entry must not verify");
        drop(container);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let streamed = container
            .verify_all_streaming()
            .expect_err("a corrupted entry must not verify streamed");
        for message in [materializing.to_string(), streamed.to_string()] {
            assert!(
                message.contains("payload/g/compact.bin"),
                "the refusal must name the entry: {message}"
            );
        }
    }

    fn compact_container_bytes(payload: &[u8]) -> Vec<u8> {
        pack(
            "g",
            &[
                empty_index_entry(),
                entry("payload/g/compact.bin", payload, COMPACT_STREAM_ROLE, None),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap()
    }

    fn entry(path: &str, bytes: &[u8], role: &'static str, hour_index: Option<i64>) -> PackEntry {
        PackEntry {
            path: path.into(),
            bytes: bytes.to_vec(),
            role,
            hour_index,
        }
    }

    fn empty_index_entry() -> PackEntry {
        entry(
            "payload/g/index.json",
            br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#,
            "index",
            None,
        )
    }

    #[test]
    fn path_open_is_file_backed_while_byte_open_remains_in_memory() {
        let bytes = pack("g", &[empty_index_entry()], &PackIdentity::default(), false).unwrap();
        let path = TempContainerFile::write("backend_selection", &bytes);

        let file_backed =
            Container::open(&path.0, &ExpectedIdentity::default()).expect("path container opens");
        assert!(matches!(&file_backed.archive, ContainerArchive::File(_)));

        let in_memory = Container::from_bytes(bytes, &ExpectedIdentity::default())
            .expect("byte container opens");
        assert!(matches!(&in_memory.archive, ContainerArchive::Memory(_)));
    }

    #[test]
    fn bounded_transactional_spool_rejects_before_destination_publication() {
        const PAYLOAD: &[u8] = b"bounded compact payload";
        let bytes = pack(
            "g",
            &[
                empty_index_entry(),
                entry("payload/g/compact.bin", PAYLOAD, COMPACT_STREAM_ROLE, None),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        let archive = TempContainerFile::write("bounded_spool", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let root = TempSpoolDir::create("bounded");
        let destination = root.join("compact.bin");
        let error = container
            .spool_entry_to_file_transactionally_bounded(
                "payload/g/compact.bin",
                &destination,
                PAYLOAD.len() as u64 - 1,
            )
            .expect_err("declared bytes beyond the product ceiling must fail closed");
        assert!(error.to_string().contains("spool ceiling"));
        assert!(!destination.exists());
        assert!(fs::read_dir(&root.0).unwrap().next().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn retained_container_open_rejects_unc_and_non_file_inputs() {
        let unc = open_retained_container_file(Path::new(r"\\server\share\hostile.battleplan"))
            .expect_err("UNC replay paths must fail lexically without network access");
        assert_eq!(unc.kind(), std::io::ErrorKind::InvalidInput);

        let root = TempSpoolDir::create("container_type");
        let error = open_retained_container_file(&root.0)
            .expect_err("a directory must not be accepted as a replay file");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn whole_file_receipt_rejects_same_length_mutation_during_parse() {
        use std::io::{Seek, SeekFrom, Write};

        const SENTINEL: &[u8] = b"P0_WHOLE_FILE_RECEIPT_MUTATION_SENTINEL";
        let bytes = pack(
            "g",
            &[
                empty_index_entry(),
                entry("payload/g/optional.bin", SENTINEL, "optional", None),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        let offset = bytes
            .windows(SENTINEL.len())
            .position(|window| window == SENTINEL)
            .expect("stored optional payload contains the unique sentinel")
            as u64;
        let path = TempContainerFile::write("receipt_same_length_mutation", &bytes);

        // Use permissively shared test handles so the hook can model a writer
        // that already existed before the production deny-write handle opened.
        // The production path has no hook and uses open_retained_container_file.
        let reader = File::open(&path.0).expect("open retained test reader");
        let mut mutator = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path.0)
            .expect("open pre-existing test writer");
        let result = Container::open_strict_with_file_receipt_from_handle(
            reader,
            &ExpectedIdentity::default(),
            move || {
                mutator.seek(SeekFrom::Start(offset))?;
                mutator.write_all(b"X")?;
                mutator.flush()?;
                mutator.sync_all()?;
                Ok(())
            },
        );
        let error = match result {
            Ok(_) => panic!("same-length mutation between receipt passes must fail closed"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(
                "retained container changed while its manifest and index were being parsed"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn transactional_spool_never_publishes_post_sync_substitution() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        const AUTHENTICATED: &[u8] = b"authenticated compact stream bytes";
        let attacker_bytes = vec![b'X'; AUTHENTICATED.len()];
        let bytes = pack(
            "g",
            &[
                empty_index_entry(),
                entry(
                    "payload/g/compact.bin",
                    AUTHENTICATED,
                    COMPACT_STREAM_ROLE,
                    None,
                ),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        // The transactional spool is file-backed ONLY (an in-memory container
        // refuses bounded entry spooling outright), so this scenario must be
        // built on a real retained-handle container or it never reaches the
        // publication path it claims to guard.
        let archive = TempContainerFile::write("post_sync_substitution", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        assert!(matches!(&container.archive, ContainerArchive::File(_)));
        let root = TempSpoolDir::create("race");
        let destination = root.join("compact.bin");
        let displaced = root.join("displaced.part");
        let mutated = Arc::new(AtomicBool::new(false));
        let hook_mutated = Arc::clone(&mutated);
        let reached = Arc::new(AtomicBool::new(false));
        let hook_reached = Arc::clone(&reached);

        let result = container.spool_entry_to_file_transactionally_with_hook(
            "payload/g/compact.bin",
            &destination,
            |temp, _retained| {
                hook_reached.store(true, Ordering::SeqCst);
                if fs::rename(temp, &displaced).is_ok() {
                    fs::write(temp, &attacker_bytes)?;
                    hook_mutated.store(true, Ordering::SeqCst);
                } else if let Ok(mut attacker_file) =
                    OpenOptions::new().write(true).truncate(true).open(temp)
                {
                    attacker_file.write_all(&attacker_bytes)?;
                    attacker_file.flush()?;
                    hook_mutated.store(true, Ordering::SeqCst);
                }
                Ok(())
            },
        );

        // Without this the whole test degrades to "the spool succeeded": a
        // refactor that stopped consuming the post-sync window would still be
        // green on the platform whose share mode denies the mutation.
        assert!(
            reached.load(Ordering::SeqCst),
            "the post-sync/pre-publication window must still be reached"
        );

        if mutated.load(Ordering::SeqCst) {
            let error = result.expect_err("post-sync mutation must fail closed");
            assert!(
                error.to_string().contains("authenticated spool"),
                "unexpected error: {error}"
            );
            assert!(
                !destination.exists(),
                "attacker bytes must never reach the published destination"
            );
        } else {
            let receipt = result.expect("Windows retained handles deny the mutation attempt");
            assert_eq!(receipt.uncompressed_len(), AUTHENTICATED.len() as u64);
            assert_eq!(fs::read(&destination).unwrap(), AUTHENTICATED);
        }

    }

    /// The pathname-rebinding variant above cannot execute its mutation on
    /// Windows — the retained spool handle denies write and delete sharing, so
    /// the substitution is refused by the OS instead of by the re-hash. This
    /// test drives the SAME production publication path with a post-sync byte
    /// change made through the retained handle itself, which no share mode can
    /// deny. It is what keeps `verify_spool_receipt_on_handle` a proven step of
    /// the commit sequence on every platform rather than dead code on one.
    #[test]
    fn transactional_spool_rejects_post_sync_change_on_the_retained_handle() {
        const AUTHENTICATED: &[u8] = b"authenticated retained-handle bytes";

        for (label, same_length, expected) in [
            (
                "same_length",
                true,
                "authenticated spool changed before publication",
            ),
            (
                "length_change",
                false,
                "authenticated spool length changed before publication",
            ),
        ] {
            let bytes = pack(
                "g",
                &[
                    empty_index_entry(),
                    entry(
                        "payload/g/retained.bin",
                        AUTHENTICATED,
                        COMPACT_STREAM_ROLE,
                        None,
                    ),
                ],
                &PackIdentity::default(),
                false,
            )
            .unwrap();
            let archive = TempContainerFile::write(&format!("retained_handle_{label}"), &bytes);
            let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
            let root = TempSpoolDir::create(&format!("retained_{label}"));
            let destination = root.join("retained.bin");

            let error = container
                .spool_entry_to_file_transactionally_with_hook(
                    "payload/g/retained.bin",
                    &destination,
                    |_temp, retained| {
                        // After flush + sync_all, before any pathname is
                        // published: exactly the window the re-hash covers.
                        retained.seek(SeekFrom::Start(0))?;
                        if same_length {
                            retained.write_all(&vec![b'X'; AUTHENTICATED.len()])?;
                        } else {
                            retained.write_all(b"short")?;
                            retained.set_len(b"short".len() as u64)?;
                        }
                        retained.flush()?;
                        retained.sync_all()?;
                        Ok(())
                    },
                )
                .expect_err("a post-sync change on the retained handle must fail closed");
            assert!(
                error.to_string().contains(expected),
                "{label}: unexpected error: {error}"
            );
            assert!(
                !destination.exists(),
                "{label}: substituted bytes must never reach the published destination"
            );
            assert!(
                fs::read_dir(&root.0).unwrap().next().is_none(),
                "{label}: the refused spool temporary must be removed"
            );
        }
    }

    #[test]
    fn transactional_spool_post_link_failure_is_failure_atomic() {
        const AUTHENTICATED: &[u8] = b"authenticated post-link bytes";
        let bytes = pack(
            "g",
            &[
                empty_index_entry(),
                entry(
                    "payload/g/post-link.bin",
                    AUTHENTICATED,
                    COMPACT_STREAM_ROLE,
                    None,
                ),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        // File-backed for the same reason as the post-sync substitution test:
        // the failure-atomicity path only exists behind the retained handle.
        let archive = TempContainerFile::write("post_link_failure", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        assert!(matches!(&container.archive, ContainerArchive::File(_)));
        let root = TempSpoolDir::create("post_link");
        let destination = root.join("post-link.bin");

        let error = container
            .spool_entry_to_file_transactionally_with_hooks(
                "payload/g/post-link.bin",
                &destination,
                STREAMING_MAX_CONTAINER_ENTRY_BYTES,
                |_, _| Ok(()),
                |published| {
                    // The hook only runs after the hard-link commit, so the
                    // destination is genuinely visible at injection time; the
                    // rollback below is what makes the failure atomic.
                    assert!(
                        published.exists(),
                        "the post-publication hook must run with the destination visible"
                    );
                    Err(ContainerError::Reject(
                        "injected post-link identity query failure".to_string(),
                    ))
                },
            )
            .expect_err("injected post-link failure must propagate");
        assert!(error.to_string().contains("injected post-link"));
        assert!(
            !destination.exists(),
            "Err must never leave a published destination"
        );
        assert!(
            fs::read_dir(&root.0).unwrap().next().is_none(),
            "a rolled-back publication must leave no spool residue either"
        );

        let receipt = container
            .spool_entry_to_file_transactionally("payload/g/post-link.bin", &destination)
            .expect("hard-link commit with no post-publication work succeeds");
        assert_eq!(receipt.uncompressed_len(), AUTHENTICATED.len() as u64);
        assert_eq!(fs::read(&destination).unwrap(), AUTHENTICATED);
    }

    /// A junction anywhere in the destination's directory chain is refused
    /// before a single byte is spooled. Without this the hard-link commit could
    /// be aimed at a directory an adversary redirects, and the retained
    /// ancestor handles would be retaining the wrong objects.
    #[cfg(windows)]
    #[test]
    fn transactional_spool_refuses_a_reparse_point_directory_component() {
        const PAYLOAD: &[u8] = b"reparse guard payload";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("reparse_component", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let root = TempSpoolDir::create("reparse");

        let target = root.join("target");
        fs::create_dir(&target).unwrap();
        let junction = root.join("junction");
        // `mklink /J` creates a directory junction with no privilege at all,
        // unlike a directory symlink, which needs SeCreateSymbolicLink or
        // Developer Mode. That makes the reparse fixture available headlessly.
        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&junction)
            .arg(&target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run mklink");
        assert!(status.success(), "mklink /J must create a junction");
        assert!(
            fs::symlink_metadata(&junction)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the fixture must be a reparse point, not a plain directory"
        );

        let destination = junction.join("compact.bin");
        let error = container
            .spool_entry_to_file_transactionally("payload/g/compact.bin", &destination)
            .expect_err("a reparse-point directory component must fail closed");
        assert!(
            error
                .to_string()
                .contains("transactional spool directory component is not a non-reparse directory"),
            "unexpected error: {error}"
        );
        assert!(
            fs::read_dir(&target).unwrap().next().is_none(),
            "nothing may be written through the redirected component"
        );
        assert!(!destination.exists());
    }

    /// The destination's parent must contain at least one retainable
    /// component. A volume root has none, so no ancestor could be pinned for
    /// the duration of the commit and the spool refuses outright.
    #[cfg(windows)]
    #[test]
    fn transactional_spool_refuses_a_filesystem_root_destination() {
        const PAYLOAD: &[u8] = b"filesystem root guard payload";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("filesystem_root", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();

        let volume_root = std::env::temp_dir()
            .ancestors()
            .last()
            .expect("the temp directory has a volume root")
            .to_path_buf();
        // Removal is on drop: if the guard ever regresses on a writable root
        // the destination IS published, and the assertion below panics before
        // any trailing cleanup could run.
        let destination = TempSpoolPath(
            volume_root.join(format!("hoi4_spool_root_probe_{}.bin", std::process::id())),
        );
        assert_eq!(destination.0.parent(), Some(volume_root.as_path()));

        let error = container
            .spool_entry_to_file_transactionally("payload/g/compact.bin", &destination.0)
            .expect_err("a volume-root destination must fail closed");
        assert!(
            error
                .to_string()
                .contains("transactional spool destination may not be a filesystem root"),
            "unexpected error: {error}"
        );
        assert!(
            !destination.0.exists(),
            "nothing may be created at the volume root"
        );
    }

    /// `.`, `..` and the empty path are refused lexically, before the chain is
    /// resolved: a destination whose meaning depends on the process working
    /// directory or on traversal cannot be identity-pinned.
    #[test]
    fn transactional_spool_refuses_empty_and_dot_relative_destinations() {
        const PAYLOAD: &[u8] = b"lexical destination guard payload";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("lexical_destination", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let root = TempSpoolDir::create("lexical");

        // Both of these name objects OUTSIDE the scratch root, so a regressed
        // guard would publish them where nothing else cleans up; removal is on
        // drop rather than after the panic-capable assertions.
        let cur_dir_relative = TempSpoolPath(std::path::Path::new(".").join(format!(
            "hoi4_spool_curdir_probe_{}.bin",
            std::process::id()
        )));
        let parent_traversal = TempSpoolPath(root.join("..").join(format!(
            "hoi4_spool_parent_probe_{}.bin",
            std::process::id()
        )));
        for (label, destination) in [
            ("empty", std::path::PathBuf::new()),
            ("cur_dir", cur_dir_relative.0.clone()),
            ("parent_dir", parent_traversal.0.clone()),
        ] {
            let error = container
                .spool_entry_to_file_transactionally("payload/g/compact.bin", &destination)
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("transactional spool destination is empty or contains . or .."),
                "{label}: unexpected error: {error}"
            );
        }
        assert!(
            !cur_dir_relative.0.exists(),
            "a working-directory-relative destination must never be created"
        );
        assert!(
            !parent_traversal.0.exists(),
            "a traversal destination must never be created"
        );
    }

    /// Unique-temporary allocation is bounded: after 128 occupied candidate
    /// names it refuses instead of spinning, and it never opens, truncates or
    /// adopts a name that already exists.
    ///
    /// The property is stated over `SPOOL_TEMP_SEQUENCE`, which is
    /// process-wide, so it is only decidable in a process where nothing else
    /// allocates a spool temporary: the occupied names have to be the names
    /// the allocation will actually try. Under `cargo test` the whole crate
    /// suite shares one process and sibling spool tests advance that sequence
    /// between the load and the spool call, which slides the real candidate
    /// window past the occupied names and lets the allocation succeed.
    /// Rather than assume a runner that gives every test its own process,
    /// this test makes one: it re-executes this test binary filtered to this
    /// test alone, and the child — marked by the environment variable below —
    /// evaluates the property with the sequence quiescent.
    #[test]
    fn transactional_spool_gives_up_after_the_bounded_unique_name_window() {
        const WORKER_MARKER: &str = "BATTLE_PLAN_BOUNDED_UNIQUE_NAME_WINDOW_WORKER";
        const TEST_NAME: &str =
            "container::tests::transactional_spool_gives_up_after_the_bounded_unique_name_window";

        if std::env::var_os(WORKER_MARKER).is_some() {
            bounded_unique_name_window_property();
            return;
        }

        let binary = std::env::current_exe().expect("locate this test binary to re-execute it");
        let output = std::process::Command::new(binary)
            .env(WORKER_MARKER, "1")
            .args(["--exact", TEST_NAME, "--test-threads=1", "--nocapture"])
            .output()
            .expect("run the isolated single-test child");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            output.status.success(),
            "the isolated child run failed (exit {:?})\n--- child stdout ---\n{stdout}\
             \n--- child stderr ---\n{stderr}",
            output.status.code()
        );
        // A child whose filter matched nothing also exits 0, so the pass count
        // is what keeps this outer assertion from passing vacuously — it is
        // also what catches a rename that leaves `TEST_NAME` stale.
        assert!(
            stdout.contains("test result: ok. 1 passed"),
            "the isolated child must have run exactly this one test\n--- child stdout ---\n\
             {stdout}\n--- child stderr ---\n{stderr}"
        );
    }

    /// The candidate-window property itself. Only ever called in the isolated
    /// child process spawned by the test above, where `SPOOL_TEMP_SEQUENCE` is
    /// quiescent and the candidate run is therefore exactly predictable.
    fn bounded_unique_name_window_property() {
        const PAYLOAD: &[u8] = b"unique name window payload";
        const ATTEMPTS: u64 = 128;
        const OCCUPIED: &[u8] = b"occupied by another writer";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("unique_name_window", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let root = TempSpoolDir::create("unique_name_window");
        let destination = root.join("compact.bin");
        let destination_name = destination.file_name().unwrap().to_owned();

        // The candidate names are `.<destination>.authenticated-spool.<pid>.<n>.part`
        // over a run of the process-wide sequence. This process runs this test
        // and nothing else, so nothing advances that sequence between the load
        // below and the spool call, and the run is exactly predictable.
        let first_sequence = SPOOL_TEMP_SEQUENCE.load(Ordering::Relaxed);
        let mut occupied = Vec::new();
        for sequence in first_sequence..first_sequence + ATTEMPTS {
            let mut temp_name = std::ffi::OsString::from(".");
            temp_name.push(&destination_name);
            temp_name.push(format!(
                ".authenticated-spool.{}.{}.part",
                std::process::id(),
                sequence
            ));
            let blocker = root.join(&temp_name);
            fs::write(&blocker, OCCUPIED).unwrap();
            occupied.push(blocker);
        }

        let error = container
            .spool_entry_to_file_transactionally("payload/g/compact.bin", &destination)
            .expect_err("an exhausted candidate window must fail closed");
        let message = error.to_string();
        assert!(
            message.contains("create authenticated spool temporary file in"),
            "unexpected error: {error}"
        );
        assert!(
            message.contains("could not allocate a unique authenticated spool temporary file"),
            "unexpected error: {error}"
        );
        assert_eq!(
            SPOOL_TEMP_SEQUENCE.load(Ordering::Relaxed),
            first_sequence + ATTEMPTS,
            "the allocation window must be exactly {ATTEMPTS} candidates wide"
        );
        assert!(!destination.exists());
        for blocker in &occupied {
            assert_eq!(
                fs::read(blocker).unwrap(),
                OCCUPIED,
                "an occupied candidate name must never be opened or truncated"
            );
        }
    }

    /// The pathname-identity rebind that guards the hard-link source. On
    /// Windows the retained handle's share mode denies the rename/unlink that
    /// would rebind the name (proven by the companion test below), so the
    /// refusal is exercised against the production function directly.
    #[test]
    fn spool_temporary_pathname_identity_rebind_is_refused() {
        let root = TempSpoolDir::create("identity_rebind");
        let retained_path = root.join("retained.part");
        let retained = open_new_locked_spool_file(&retained_path).expect("create retained spool");
        let retained_identity =
            spool_file_identity(&retained, &retained_path).expect("retained identity");

        bind_spool_temp_pathname_identity(&retained_path, retained_identity)
            .expect("the unchanged pathname must still bind to the retained object");

        let substitute_path = root.join("substitute.part");
        let substitute =
            open_new_locked_spool_file(&substitute_path).expect("create substitute object");
        let substitute_identity =
            spool_file_identity(&substitute, &substitute_path).expect("substitute identity");
        assert_ne!(
            substitute_identity, retained_identity,
            "the fixture must be two distinct filesystem objects"
        );
        let error = bind_spool_temp_pathname_identity(&substitute_path, retained_identity)
            .expect_err("a pathname naming a different object must fail closed");
        assert!(
            error.to_string().contains(
                "authenticated spool temporary pathname no longer names the retained file"
            ),
            "unexpected error: {error}"
        );

        let absent = root.join("absent.part");
        let error = bind_spool_temp_pathname_identity(&absent, retained_identity)
            .expect_err("an unopenable pathname cannot be identity-bound");
        assert!(
            error
                .to_string()
                .contains("reopen authenticated spool temporary for identity"),
            "unexpected error: {error}"
        );

        drop(retained);
        drop(substitute);
    }

    /// Why the rebind above cannot be staged end-to-end on Windows, and why the
    /// `temp_still_retained` guard's false arm is unreachable there: while the
    /// retained handle is open, its pathname cannot be renamed or unlinked at
    /// all, so it can never come to name a different object.
    #[cfg(windows)]
    #[test]
    fn a_retained_spool_temporary_cannot_be_renamed_or_unlinked_at_its_pathname() {
        let root = TempSpoolDir::create("rebind_denied");
        let temp = root.join("retained.part");
        let displaced = root.join("displaced.part");
        let retained = open_new_locked_spool_file(&temp).expect("create retained spool");
        let identity = spool_file_identity(&retained, &temp).expect("retained identity");

        let rename_error =
            fs::rename(&temp, &displaced).expect_err("a retained spool must not be renamable");
        let remove_error =
            fs::remove_file(&temp).expect_err("a retained spool must not be unlinkable");
        // Both refusals are ERROR_SHARING_VIOLATION -- the retained handle's
        // omitted delete sharing, not a permission or attribute effect.
        const ERROR_SHARING_VIOLATION: i32 = 32;
        for (label, error) in [("rename", &rename_error), ("remove", &remove_error)] {
            assert_eq!(
                error.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION),
                "{label}: unexpected os error: {error}"
            );
        }
        assert!(!displaced.exists());
        bind_spool_temp_pathname_identity(&temp, identity)
            .expect("the pathname still names the retained object");

        drop(retained);
        // Once the handle is released the same pathname is ordinary again --
        // the denial is the retained handle's, not the filesystem's.
        fs::remove_file(&temp).expect("the released pathname is removable");
    }

    /// Post-publication rollback never unlinks a destination it cannot bind to
    /// the object it published. The release hook models the only window in
    /// which the name is unpinned: after every retained handle is dropped and
    /// before the rollback runs.
    #[test]
    fn post_publication_rollback_refuses_a_rebound_destination() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        const PAYLOAD: &[u8] = b"rollback identity-bind payload";
        const FOREIGN: &[u8] = b"an object this rollback did not publish";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("rollback_rebind", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let root = TempSpoolDir::create("rollback_rebind");
        let destination = root.join("compact.bin");
        let rebound = Arc::new(AtomicBool::new(false));
        let hook_rebound = Arc::clone(&rebound);

        let error = container
            .spool_entry_to_file_transactionally_with_release_hook(
                "payload/g/compact.bin",
                &destination,
                STREAMING_MAX_CONTAINER_ENTRY_BYTES,
                |_, _| Ok(()),
                |published| {
                    assert!(published.exists(), "the commit must have happened");
                    Err(ContainerError::Reject(
                        "injected post-link failure".to_string(),
                    ))
                },
                |_temp, published| {
                    fs::remove_file(published).expect("the released name is unlinkable");
                    fs::write(published, FOREIGN).expect("the released name is rebindable");
                    hook_rebound.store(true, std::sync::atomic::Ordering::SeqCst);
                },
            )
            .expect_err("a rollback that cannot identity-bind must fail closed");

        assert!(
            rebound.load(std::sync::atomic::Ordering::SeqCst),
            "the post-release window must still be reached"
        );
        assert!(
            error
                .to_string()
                .contains("post-publication failure could not identity-bind destination cleanup"),
            "unexpected error: {error}"
        );
        assert_eq!(
            fs::read(&destination).unwrap(),
            FOREIGN,
            "rollback must never unlink an object it did not publish"
        );
        let residue: Vec<_> = fs::read_dir(&root.0)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(
            residue,
            vec![destination.file_name().unwrap().to_owned()],
            "the spool temporary must still be cleaned up"
        );
    }

    /// The other rollback failure mode: the destination is still the published
    /// object, but it cannot be removed. The refusal is reported rather than
    /// swallowed, so a caller never sees a bare Err with a live destination.
    #[cfg(windows)]
    #[test]
    fn post_publication_rollback_reports_an_undeletable_destination() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::sync::{Arc, Mutex};

        const PAYLOAD: &[u8] = b"rollback undeletable payload";
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("rollback_undeletable", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let root = TempSpoolDir::create("rollback_undeletable");
        let destination = root.join("compact.bin");
        let held: Arc<Mutex<Option<File>>> = Arc::new(Mutex::new(None));
        let hook_held = Arc::clone(&held);

        let error = container
            .spool_entry_to_file_transactionally_with_hooks(
                "payload/g/compact.bin",
                &destination,
                STREAMING_MAX_CONTAINER_ENTRY_BYTES,
                |_, _| Ok(()),
                |published| {
                    // The retained spool handle is still open here, so this
                    // handle must share read and write with it; omitting delete
                    // sharing is what makes the rollback's unlink fail.
                    let blocker = OpenOptions::new()
                        .read(true)
                        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                        .open(published)
                        .expect("open a deny-delete handle on the published destination");
                    *hook_held.lock().unwrap() = Some(blocker);
                    Err(ContainerError::Reject(
                        "injected post-link failure".to_string(),
                    ))
                },
            )
            .expect_err("a rollback that cannot unlink must fail closed");

        assert!(
            error.to_string().contains(
                "post-publication failure could not remove authenticated destination"
            ),
            "unexpected error: {error}"
        );
        assert!(
            held.lock().unwrap().is_some(),
            "the deny-delete handle must have been taken"
        );
        assert_eq!(
            fs::read(&destination).unwrap(),
            PAYLOAD,
            "the destination the rollback could not remove is still the published object"
        );

        // Release the blocker so the scratch directory can be cleaned up.
        held.lock().unwrap().take();
        fs::remove_file(&destination).expect("the released destination is removable");
    }

    /// The temporary's own cleanup is identity-gated too, and its refusal arm
    /// is NOT Windows-unreachable: the retained handle is released before the
    /// check runs, so the pathname is genuinely unpinned in that window on
    /// every platform. A temporary whose name was rebound there is left alone
    /// rather than unlinked, exactly as a foreign destination is.
    #[test]
    fn a_rebound_spool_temporary_pathname_is_never_unlinked() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        const PAYLOAD: &[u8] = b"temp cleanup identity payload";
        const FOREIGN: &[u8] = b"an object this spool did not create";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("temp_cleanup_identity", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let root = TempSpoolDir::create("temp_cleanup_identity");
        let destination = root.join("compact.bin");
        let rebound = Arc::new(AtomicBool::new(false));
        let hook_rebound = Arc::clone(&rebound);
        let temp_seen = Arc::new(Mutex::new(std::path::PathBuf::new()));
        let hook_temp = Arc::clone(&temp_seen);

        let receipt = container
            .spool_entry_to_file_transactionally_with_release_hook(
                "payload/g/compact.bin",
                &destination,
                STREAMING_MAX_CONTAINER_ENTRY_BYTES,
                |_, _| Ok(()),
                |_| Ok(()),
                |temp, _published| {
                    // The retained handle is already dropped here, so this
                    // unlink succeeds even on Windows: the temporary's cleanup
                    // window is unpinned on every platform.
                    fs::remove_file(temp).expect("the released temporary is unlinkable");
                    fs::write(temp, FOREIGN).expect("the released name is rebindable");
                    *hook_temp.lock().unwrap() = temp.to_path_buf();
                    hook_rebound.store(true, std::sync::atomic::Ordering::SeqCst);
                },
            )
            .expect("publication itself succeeds; only the temporary was rebound");

        assert!(
            rebound.load(std::sync::atomic::Ordering::SeqCst),
            "the post-release window must still be reached"
        );
        assert_eq!(receipt.uncompressed_len(), PAYLOAD.len() as u64);
        assert_eq!(fs::read(&destination).unwrap(), PAYLOAD);
        let temp = temp_seen.lock().unwrap().clone();
        assert_eq!(
            fs::read(&temp).unwrap(),
            FOREIGN,
            "a rebound temporary pathname must never be unlinked"
        );
    }

    /// Whole-file receipts are a retained-handle property: an in-memory
    /// container has no object to re-read, and answering with anything would
    /// be a receipt for bytes nobody can re-verify.
    #[test]
    fn whole_file_receipts_require_a_retained_file_backed_container() {
        const PAYLOAD: &[u8] = b"whole file receipt payload";
        let bytes = compact_container_bytes(PAYLOAD);

        let in_memory = Container::from_bytes(bytes.clone(), &ExpectedIdentity::default())
            .expect("byte container opens");
        assert!(matches!(&in_memory.archive, ContainerArchive::Memory(_)));
        let error = in_memory
            .retained_file_receipt()
            .expect_err("an in-memory container has no whole-file receipt");
        assert!(
            error
                .to_string()
                .contains("whole-file receipts require a retained file-backed container"),
            "unexpected error: {error}"
        );

        let archive = TempContainerFile::write("whole_file_receipt", &bytes);
        let file_backed =
            Container::open(&archive.0, &ExpectedIdentity::default()).expect("path container opens");
        let (len, sha256) = file_backed
            .retained_file_receipt()
            .expect("a retained container answers with its receipt");
        assert_eq!(len, bytes.len() as u64);
        assert_eq!(sha256, to_hex(&Sha256::digest(&bytes)));
    }

    /// `ContainerArchive::copy_entry_to_writer` refuses a file-backed archive
    /// that answered a `calculate_sha256` request with no digest. That arm is
    /// unreachable while the archive keeps its side of the contract; this pins
    /// the contract itself, which is the only thing that keeps it unreachable.
    #[test]
    fn a_file_backed_archive_never_omits_a_requested_spool_sha256() {
        const PAYLOAD: &[u8] = b"requested spool sha256 payload";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("requested_sha256", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();
        let ContainerArchive::File(file_archive) = &container.archive else {
            panic!("the fixture must be file-backed");
        };
        let index = container
            .by_path
            .lock()
            .unwrap()
            .get("payload/g/compact.bin")
            .expect("the entry is manifest-listed")
            .archive_index;

        let mut requested = Vec::new();
        let (digest, len) = file_archive
            .copy_entry_to_writer(
                index,
                &mut requested,
                true,
                STREAMING_MAX_CONTAINER_ENTRY_BYTES,
            )
            .expect("copy the entry with a requested digest");
        assert_eq!(
            digest.as_deref(),
            Some(to_hex(&Sha256::digest(PAYLOAD)).as_str()),
            "a requested digest must always be delivered"
        );
        assert_eq!(len, PAYLOAD.len() as u64);
        assert_eq!(requested, PAYLOAD);

        let mut unrequested = Vec::new();
        let (absent, _) = file_archive
            .copy_entry_to_writer(
                index,
                &mut unrequested,
                false,
                STREAMING_MAX_CONTAINER_ENTRY_BYTES,
            )
            .expect("copy the entry without a requested digest");
        assert!(
            absent.is_none(),
            "the absent digest is only ever the unrequested case"
        );
    }

    /// `Container::authenticate_entry` has three production consumers and no
    /// test in any crate. It must authenticate against the manifest without
    /// exposing bytes, and refuse everything the spool primitive refuses.
    #[test]
    fn authenticate_entry_verifies_without_exposing_bytes() {
        const PAYLOAD: &[u8] = b"authenticate entry payload";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("authenticate_entry", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();

        let receipt = container
            .authenticate_entry("payload/g/compact.bin")
            .expect("a manifest-listed entry authenticates");
        assert_eq!(receipt.path(), "payload/g/compact.bin");
        assert_eq!(receipt.uncompressed_len(), PAYLOAD.len() as u64);
        assert_eq!(receipt.sha256(), to_hex(&Sha256::digest(PAYLOAD)));

        let error = container
            .authenticate_entry("payload/g/absent.bin")
            .expect_err("an unlisted path must fail closed");
        assert!(
            error.to_string().contains("not in manifest"),
            "unexpected error: {error}"
        );

        // The receipt must come from the archive, not from the manifest record
        // it is checked against. Against an intact container a receipt
        // fabricated out of the manifest's own path/len/sha would be
        // indistinguishable from an authenticated one, so authenticate the
        // same manifest against stored bytes that no longer match it.
        let mut tampered = bytes.clone();
        let offset = tampered
            .windows(PAYLOAD.len())
            .position(|window| window == PAYLOAD)
            .expect("the stored payload is locatable in the packed container");
        tampered[offset] ^= 0xff;
        let tampered_archive = TempContainerFile::write("authenticate_entry_tampered", &tampered);
        let tampered_container = Container::open(&tampered_archive.0, &ExpectedIdentity::default())
            .expect("entry bytes are authenticated lazily, so the container still opens");
        let error = tampered_container
            .authenticate_entry("payload/g/compact.bin")
            .expect_err("a manifest-listed entry whose stored bytes changed must fail closed");
        assert!(
            error.to_string().contains("crc32 mismatch"),
            "unexpected error: {error}"
        );

        let in_memory = Container::from_bytes(bytes, &ExpectedIdentity::default()).unwrap();
        let error = in_memory
            .authenticate_entry("payload/g/compact.bin")
            .expect_err("an in-memory container cannot stream an entry");
        assert!(
            error
                .to_string()
                .contains("bounded entry spooling requires a file-backed container"),
            "unexpected error: {error}"
        );
    }

    /// `Container::copy_entry_to_unpublished_writer_bounded` is the caller-owned
    /// sink variant, and its one production consumer is its only exercise. The
    /// receipt is the caller's permission to publish, so every refusal must
    /// arrive with nothing usable written.
    #[test]
    fn copy_entry_to_unpublished_writer_bounded_refuses_before_it_writes() {
        const PAYLOAD: &[u8] = b"unpublished writer payload";
        let bytes = compact_container_bytes(PAYLOAD);
        let archive = TempContainerFile::write("unpublished_writer", &bytes);
        let container = Container::open(&archive.0, &ExpectedIdentity::default()).unwrap();

        let mut sink = Vec::new();
        let receipt = container
            .copy_entry_to_unpublished_writer_bounded(
                "payload/g/compact.bin",
                &mut sink,
                PAYLOAD.len() as u64,
            )
            .expect("a bound at the declared size admits the entry");
        assert_eq!(sink, PAYLOAD);
        assert_eq!(receipt.uncompressed_len(), PAYLOAD.len() as u64);
        assert_eq!(receipt.sha256(), to_hex(&Sha256::digest(PAYLOAD)));

        let mut sink = Vec::new();
        let error = container
            .copy_entry_to_unpublished_writer_bounded(
                "payload/g/compact.bin",
                &mut sink,
                PAYLOAD.len() as u64 - 1,
            )
            .expect_err("a bound below the declared size must fail closed");
        assert!(
            error.to_string().contains("exceed spool ceiling"),
            "unexpected error: {error}"
        );
        assert!(
            sink.is_empty(),
            "a refused entry must never reach the caller's sink"
        );

        let mut sink = Vec::new();
        let error = container
            .copy_entry_to_unpublished_writer_bounded("payload/g/compact.bin", &mut sink, 0)
            .expect_err("a zero ceiling must fail closed");
        assert!(
            error
                .to_string()
                .contains("entry spool byte ceiling must be positive"),
            "unexpected error: {error}"
        );
        assert!(sink.is_empty());

        let mut sink = Vec::new();
        let error = container
            .copy_entry_to_unpublished_writer_bounded(
                "payload/g/absent.bin",
                &mut sink,
                STREAMING_MAX_CONTAINER_ENTRY_BYTES,
            )
            .expect_err("an unlisted path must fail closed");
        assert!(
            error.to_string().contains("not in manifest"),
            "unexpected error: {error}"
        );
        assert!(sink.is_empty());
    }

    #[test]
    fn segments_accessor_strict_parses_duplicate_members() {
        // A v1 container carrying a stray segments.json with a duplicated
        // member: the accessor must be a typed refusal, never last-wins.
        let bytes = pack(
            "g",
            &[
                empty_index_entry(),
                entry(
                    "payload/g/segments.json",
                    br#"{"segments":[],"segments":[]}"#,
                    "segments",
                    None,
                ),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        let container =
            Container::from_bytes(bytes, &ExpectedIdentity::default()).expect("v1 container opens");
        let err = container
            .segments()
            .expect_err("duplicate-member segments.json must refuse");
        assert!(
            err.to_string().contains("duplicate JSON member"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn poisoned_entry_lock_is_a_typed_refusal_never_absence() {
        // A caught panic during entry verification poisons `by_path`. From that
        // point every membership answer is unknowable: `has_entry` must raise
        // the same typed refusal as `read_entry_bytes`, and `segments()` must
        // never collapse the poison into Ok(None) "no graph" (fail-closed).
        let bytes = pack(
            "g",
            &[
                empty_index_entry(),
                entry(
                    "payload/g/segments.json",
                    br#"{"segments":[]}"#,
                    "segments",
                    None,
                ),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        let container =
            Container::from_bytes(bytes, &ExpectedIdentity::default()).expect("container opens");
        assert!(container
            .has_entry("payload/g/segments.json")
            .expect("healthy lock answers membership"));
        container.poison_entry_lock_for_test();
        let err = container
            .has_entry("payload/g/segments.json")
            .expect_err("a poisoned lock must refuse, never report absence");
        assert!(
            err.to_string().contains("entry-verification lock poisoned"),
            "unexpected error: {err}"
        );
        let err = match container.segments() {
            Ok(graph) => panic!(
                "poisoned lock must refuse, got Ok(present={})",
                graph.is_some()
            ),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("entry-verification lock poisoned"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn v2_duplicate_member_segments_json_is_an_open_refusal() {
        let bytes = pack_with(
            "g",
            &[
                entry(
                    "payload/g/index.json",
                    br#"{"game_id":"g","schema_version":"1","index_schema":"stitched-main-v1","segments_file":"segments.json","snapshots":[]}"#,
                    "index",
                    None,
                ),
                entry(
                    "payload/g/segments.json",
                    br#"{"segments":[],"segments":[]}"#,
                    "segments",
                    None,
                ),
            ],
            &PackIdentity::default(),
            false,
            &PackOptions {
                container_version: Some(2),
                required_features: Some(vec![FEATURE_TIMELINE_SEGMENTS_V1.into()]),
                ..Default::default()
            },
        )
        .unwrap();
        let err = match Container::from_bytes(bytes, &ExpectedIdentity::default()) {
            Ok(_) => panic!("v2 duplicate-member graph must be refused at open"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("duplicate JSON member"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_file_requires_index_listing() {
        let bytes = pack(
            "g",
            &[
                entry(
                    "payload/g/index.json",
                    br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":0,"file":"000000000000.snapshot.json"}]}"#,
                    "index",
                    None,
                ),
                entry(
                    "payload/g/000000000000.snapshot.json",
                    br#"{"game_id":"g","hour_index":0}"#,
                    "snapshot",
                    Some(0),
                ),
                entry(
                    "payload/g/stray.json",
                    br#"{"not":"a snapshot"}"#,
                    "aux",
                    None,
                ),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        let container =
            Container::from_bytes(bytes, &ExpectedIdentity::default()).expect("container opens");
        container
            .load_file("000000000000.snapshot.json")
            .expect("index-listed snapshot loads");
        container
            .load(0)
            .expect("hour-indexed load resolves through the index");
        let err = container
            .load_file("stray.json")
            .expect_err("a manifest-listed non-index entry must not decode as a Snapshot");
        assert!(
            err.to_string().contains("not listed in the index"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn entry_bytes_cross_only_the_verified_state_boundary() {
        let second_bytes = br#"{"game_id":"g","hour_index":1}"#;
        let second_path = "payload/g/000000000001.snapshot.json";
        let bytes = pack(
            "g",
            &[
                entry(
                    "payload/g/index.json",
                    br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":0,"file":"000000000000.snapshot.json"},{"hour_index":1,"file":"000000000001.snapshot.json"}]}"#,
                    "index",
                    None,
                ),
                entry(
                    "payload/g/000000000000.snapshot.json",
                    br#"{"game_id":"g","hour_index":0}"#,
                    "snapshot",
                    Some(0),
                ),
                entry(second_path, second_bytes, "snapshot", Some(1)),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        let container =
            Container::from_bytes(bytes, &ExpectedIdentity::default()).expect("container opens");

        {
            let entries = container.by_path.lock().unwrap();
            assert_eq!(
                entries.get(second_path).unwrap().sha256_state,
                EntryVerificationState::Pending,
                "an untouched non-representative snapshot stays lazy"
            );
        }

        {
            let mut entries = container.by_path.lock().unwrap();
            entries.get_mut(second_path).unwrap().expected_sha256 = "0".repeat(64);
        }
        container
            .read_entry_bytes(second_path)
            .expect_err("a mismatched hash must not expose bytes");
        {
            let entries = container.by_path.lock().unwrap();
            assert_eq!(
                entries.get(second_path).unwrap().sha256_state,
                EntryVerificationState::Pending,
                "a failed hash must not advance the state"
            );
        }

        {
            let mut entries = container.by_path.lock().unwrap();
            entries.get_mut(second_path).unwrap().expected_sha256 =
                to_hex(&Sha256::digest(second_bytes));
        }
        assert_eq!(
            container
                .read_entry_bytes(second_path)
                .expect("matching bytes cross the verified boundary"),
            second_bytes
        );
        let entries = container.by_path.lock().unwrap();
        assert_eq!(
            entries.get(second_path).unwrap().sha256_state,
            EntryVerificationState::Verified
        );
    }

    #[test]
    fn manifest_build_hash_must_match_snapshot_provenance() {
        let entries = [
            entry(
                "payload/g/index.json",
                br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":0,"file":"000000000000.snapshot.json"}]}"#,
                "index",
                None,
            ),
            entry(
                "payload/g/000000000000.snapshot.json",
                br#"{"game_id":"g","hour_index":0,"provenance":{"collector":{"exe_sha256":"aaaa"},"memory":{"exe_sha256":"aaaa"}}}"#,
                "snapshot",
                Some(0),
            ),
        ];
        let agreeing = PackIdentity {
            build_exe_sha256: Some("aaaa".into()),
            ..Default::default()
        };
        Container::from_bytes(
            pack("g", &entries, &agreeing, false).unwrap(),
            &ExpectedIdentity::default(),
        )
        .expect("agreeing build hash opens");

        let lying = PackIdentity {
            build_exe_sha256: Some("bbbb".into()),
            ..Default::default()
        };
        let err = match Container::from_bytes(
            pack("g", &entries, &lying, false).unwrap(),
            &ExpectedIdentity::default(),
        ) {
            Ok(_) => panic!("a manifest lying about the build must be refused"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("disagrees with snapshot provenance.collector.exe_sha256"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn modset_mismatch_is_typed_with_stable_text() {
        let entries = [
            entry(
                "payload/g/index.json",
                br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":0,"file":"000000000000.snapshot.json"}]}"#,
                "index",
                None,
            ),
            entry(
                "payload/g/000000000000.snapshot.json",
                br#"{"game_id":"g","hour_index":0,"provenance":{"mod":{"modset_id":"real"}}}"#,
                "snapshot",
                Some(0),
            ),
        ];
        let identity = PackIdentity {
            mod_identity: Some(json!({"modset_id":"claimed"})),
            ..Default::default()
        };
        let err = match Container::from_bytes(
            pack("g", &entries, &identity, false).unwrap(),
            &ExpectedIdentity::default(),
        ) {
            Ok(_) => panic!("modset lie must be refused"),
            Err(err) => err,
        };
        assert!(
            matches!(err, ContainerError::ModsetMismatch { .. }),
            "expected typed ModsetMismatch, got {err:?}"
        );
        // Display renders the EXACT pre-typed message (fixture/UX contract).
        assert_eq!(
            err.to_string(),
            "container rejected: manifest modset_id \"claimed\" disagrees with snapshot provenance.mod \"real\""
        );
    }

    #[test]
    fn malformed_timeline_is_an_open_refusal() {
        let bytes = pack(
            "g",
            &[
                empty_index_entry(),
                entry(
                    "payload/g/timeline.json",
                    br#"{"a":1,"a":2}"#,
                    "timeline",
                    None,
                ),
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        let err = match Container::from_bytes(bytes, &ExpectedIdentity::default()) {
            Ok(_) => panic!("duplicate-member timeline.json must be refused at open"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("timeline.json failed strict JSON parse"),
            "unexpected error: {err}"
        );
    }
}
