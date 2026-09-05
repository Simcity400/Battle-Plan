use crate::hud_role::validate_sha256;
use crate::{normalize_game_relative_path, ContentLayerKind, LayerStorageKind, UsedContentKey};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(crate) const MAX_LISTED_FILES: usize = 1 << 18;
pub(crate) const MAX_LISTED_TOTAL_BYTES: u64 = 2 << 30;

#[derive(Clone, Debug)]
pub struct EffectiveVfsLayerMount {
    pub ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    pub storage_kind: LayerStorageKind,
    pub local_source: PathBuf,
    pub source_container_sha256: Option<String>,
    pub replace_paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveVfsResolvedFile {
    pub logical_path: String,
    pub content: UsedContentKey,
    pub bytes: Vec<u8>,
}

/// What one enumeration pass reported about an effective file, enough to tell
/// a later pass whether the bytes behind it can have changed without reading
/// them: a directory file is its exact local path, length and modification
/// instant; an archive member is content-keyed by the archive digest the mount
/// already verified against the recorded source-container digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectiveVfsEntryFacts {
    File {
        local_path: PathBuf,
        len: u64,
        modified_unix_secs: i64,
        modified_nanos: u32,
    },
    ArchiveMember {
        archive_sha256: String,
        entry_name: String,
    },
}

/// One path the layered VFS resolves, with the layer that wins it. Produced by
/// [`EffectiveVfs::effective_entries`] under the same replace-path law
/// [`EffectiveVfs::resolve`] applies, so the winner recorded here is the layer
/// `resolve` would read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveVfsEntry {
    pub logical_path: String,
    pub layer_kind: ContentLayerKind,
    layer: usize,
    pub facts: EffectiveVfsEntryFacts,
}

/// A file one directory enumeration reported: its path spelling as stored and
/// the facts the enumeration itself carried (no second stat).
#[derive(Clone, Debug)]
struct ListedFile {
    path: String,
    local_path: PathBuf,
    len: u64,
    modified_unix_secs: i64,
    modified_nanos: u32,
}

#[derive(Clone, Debug)]
struct MountedLayer {
    ordinal: u32,
    layer_kind: ContentLayerKind,
    layer_id: String,
    storage_kind: LayerStorageKind,
    local_source: PathBuf,
    source_container_sha256: Option<String>,
    replace_paths: BTreeSet<String>,
    zip_index: Option<ZipLayerIndex>,
}

#[derive(Clone, Debug)]
struct ZipLayerIndex {
    entries: BTreeMap<String, (String, usize)>,
    archive: Arc<Mutex<zip::ZipArchive<fs::File>>>,
}

#[derive(Clone, Debug)]
pub struct EffectiveVfs {
    authority_manifest_sha256: String,
    layers: Vec<MountedLayer>,
}

impl EffectiveVfs {
    pub fn new(
        authority_manifest_sha256: impl Into<String>,
        mounts: impl IntoIterator<Item = EffectiveVfsLayerMount>,
    ) -> Result<Self, String> {
        let authority_manifest_sha256 = authority_manifest_sha256.into();
        validate_sha256(&authority_manifest_sha256, "effective VFS authority")?;
        let mut layers = Vec::new();
        let mut prior_ordinal = None;
        for mount in mounts {
            if mount.layer_id.trim().is_empty()
                || mount.layer_id.chars().any(char::is_control)
                || prior_ordinal.is_some_and(|prior| prior >= mount.ordinal)
            {
                return Err("effective VFS layer order/identity is invalid".into());
            }
            if layers.is_empty()
                && (mount.ordinal != 0 || mount.layer_kind != ContentLayerKind::Base)
            {
                return Err("effective VFS must begin with ordinal-zero base content".into());
            }
            if !layers.is_empty() && mount.layer_kind == ContentLayerKind::Base {
                return Err("effective VFS repeats base content".into());
            }
            prior_ordinal = Some(mount.ordinal);
            let metadata = fs::symlink_metadata(&mount.local_source)
                .map_err(|error| format!("effective VFS layer source metadata failed: {error}"))?;
            if metadata.file_type().is_symlink() {
                return Err("effective VFS refuses a symlinked layer source".into());
            }
            match mount.storage_kind {
                LayerStorageKind::Directory if !metadata.is_dir() => {
                    return Err("effective VFS directory layer is not a directory".into())
                }
                LayerStorageKind::ZipArchive if !metadata.is_file() => {
                    return Err("effective VFS archive layer is not a file".into())
                }
                _ => {}
            }
            let local_source = fs::canonicalize(&mount.local_source).map_err(|error| {
                format!("canonicalize effective VFS layer source failed: {error}")
            })?;
            let (source_container_sha256, zip_index) = match mount.storage_kind {
                LayerStorageKind::Directory => {
                    if mount.source_container_sha256.is_some() {
                        return Err(
                            "directory VFS layer unexpectedly carries an archive digest".into()
                        );
                    }
                    (None, None)
                }
                LayerStorageKind::ZipArchive => {
                    let expected = mount
                        .source_container_sha256
                        .ok_or("archive VFS layer omits its portable source-container digest")?;
                    validate_sha256(&expected, "archive source-container")?;
                    let zip_index = index_zip_layer(&local_source, &expected)?;
                    (Some(expected), Some(zip_index))
                }
            };
            let mut replace_paths = BTreeSet::new();
            let mut replace_path_collisions = BTreeSet::new();
            for path in mount.replace_paths {
                let normalized = normalize_game_relative_path(&path)?;
                if normalized != path.replace('\\', "/")
                    || !replace_path_collisions.insert(crate::windows_collision_key(&normalized)?)
                    || !replace_paths.insert(normalized)
                {
                    return Err("effective VFS layer has invalid or repeated replace_path".into());
                }
            }
            layers.push(MountedLayer {
                ordinal: mount.ordinal,
                layer_kind: mount.layer_kind,
                layer_id: mount.layer_id,
                storage_kind: mount.storage_kind,
                local_source,
                source_container_sha256,
                replace_paths,
                zip_index,
            });
        }
        if layers.is_empty() {
            return Err("effective VFS has no layers".into());
        }
        Ok(Self {
            authority_manifest_sha256,
            layers,
        })
    }

    pub fn authority_manifest_sha256(&self) -> &str {
        &self.authority_manifest_sha256
    }

    pub fn resolve(&self, logical_path: &str) -> Result<Option<EffectiveVfsResolvedFile>, String> {
        let logical_path = normalize_game_relative_path_preserve_case(logical_path)?;
        let normalized_path = normalize_game_relative_path(&logical_path)?;
        let parent = logical_parent(&normalized_path);
        for layer in self.layers.iter().rev() {
            if let Some((stored_path, bytes)) = read_layer_file(layer, &logical_path)? {
                return Ok(Some(self.resolved(layer, stored_path, bytes)?));
            }
            if layer.replace_paths.contains(&parent) {
                return Ok(None);
            }
        }
        Ok(None)
    }

    pub fn resolve_required(
        &self,
        logical_path: &str,
        label: &str,
    ) -> Result<EffectiveVfsResolvedFile, String> {
        self.resolve(logical_path)?
            .ok_or_else(|| format!("missing effective VFS {label}: {logical_path}"))
    }

    pub fn list_recursive(
        &self,
        logical_prefix: &str,
        extension: Option<&str>,
    ) -> Result<Vec<EffectiveVfsResolvedFile>, String> {
        self.list_recursive_bounded(
            logical_prefix,
            extension,
            MAX_LISTED_FILES,
            MAX_LISTED_TOTAL_BYTES,
        )
    }

    pub(crate) fn list_recursive_bounded(
        &self,
        logical_prefix: &str,
        extension: Option<&str>,
        max_files: usize,
        max_total_bytes: u64,
    ) -> Result<Vec<EffectiveVfsResolvedFile>, String> {
        let prefix = normalize_game_relative_path(logical_prefix)?;
        let extension = extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
        let mut effective = BTreeMap::<String, String>::new();
        for layer in &self.layers {
            if !layer.replace_paths.is_empty() {
                effective.retain(|_, path| !layer.replace_paths.contains(&logical_parent(path)));
            }
            for path in list_layer_files(layer, &prefix, extension.as_deref())? {
                effective.insert(path.to_ascii_lowercase(), path);
                if effective.len() > max_files.min(MAX_LISTED_FILES) {
                    return Err("effective VFS listing exceeds its file ceiling".into());
                }
            }
        }
        let byte_ceiling = max_total_bytes.min(MAX_LISTED_TOTAL_BYTES);
        let mut total_bytes = 0u64;
        let mut resolved = Vec::with_capacity(effective.len());
        for path in effective.into_values() {
            let file = self
                .resolve(&path)?
                .ok_or_else(|| format!("effective VFS winner disappeared for {path}"))?;
            total_bytes = total_bytes
                .checked_add(file.content.payload_bytes)
                .ok_or_else(|| "effective VFS listing byte count overflowed".to_string())?;
            if total_bytes > byte_ceiling {
                return Err("effective VFS listing exceeds its byte ceiling".into());
            }
            resolved.push(file);
        }
        Ok(resolved)
    }

    /// Every path this VFS resolves, in ascending case-folded order, each with
    /// the layer that wins it. Winner law is `resolve`'s: a layer's file
    /// shadows every lower spelling of the same path, and a layer that
    /// replaces a directory drops every lower file under it.
    pub fn effective_entries(&self) -> Result<Vec<EffectiveVfsEntry>, String> {
        let mut effective = BTreeMap::<String, EffectiveVfsEntry>::new();
        for (layer_index, layer) in self.layers.iter().enumerate() {
            if !layer.replace_paths.is_empty() {
                effective.retain(|_, entry| {
                    !layer
                        .replace_paths
                        .contains(&logical_parent(&entry.logical_path))
                });
            }
            for listed in list_layer_entries(layer, "", None)? {
                let facts = match layer.storage_kind {
                    LayerStorageKind::Directory => EffectiveVfsEntryFacts::File {
                        local_path: listed.local_path,
                        len: listed.len,
                        modified_unix_secs: listed.modified_unix_secs,
                        modified_nanos: listed.modified_nanos,
                    },
                    LayerStorageKind::ZipArchive => EffectiveVfsEntryFacts::ArchiveMember {
                        archive_sha256: layer
                            .source_container_sha256
                            .clone()
                            .ok_or("effective VFS archive layer has no source digest")?,
                        entry_name: listed.path.clone(),
                    },
                };
                effective.insert(
                    listed.path.to_ascii_lowercase(),
                    EffectiveVfsEntry {
                        logical_path: listed.path,
                        layer_kind: layer.layer_kind,
                        layer: layer_index,
                        facts,
                    },
                );
            }
        }
        Ok(effective.into_values().collect())
    }

    /// The bytes behind one entry, read from its winning layer. A directory
    /// file is re-resolved through the same literal-path law `resolve` uses,
    /// so a path that has become a reparse point or stopped being a regular
    /// file between the listing and this read is refused, never followed.
    pub fn read_entry(&self, entry: &EffectiveVfsEntry) -> Result<Vec<u8>, String> {
        let layer = self
            .layers
            .get(entry.layer)
            .ok_or("effective VFS entry names a layer this VFS does not mount")?;
        let (stored_path, bytes) = read_layer_file(layer, &entry.logical_path)?.ok_or_else(|| {
            format!("effective VFS winner disappeared for {}", entry.logical_path)
        })?;
        if stored_path != entry.logical_path {
            return Err(format!("effective VFS stored path changed for {}", entry.logical_path));
        }
        Ok(bytes)
    }

    /// Streams one winning entry through SHA-256 without holding the whole
    /// payload: directory files are read in 1 MiB chunks through `buffer`,
    /// archive members are copied straight out of the member reader.
    pub fn hash_entry(
        &self,
        entry: &EffectiveVfsEntry,
        buffer: &mut Vec<u8>,
        progress: &mut dyn FnMut(usize),
    ) -> Result<(u64, String), String> {
        let layer = self
            .layers
            .get(entry.layer)
            .ok_or("effective VFS entry names a layer this VFS does not mount")?;
        hash_layer_file(layer, &entry.logical_path, buffer, progress)?.ok_or_else(|| {
            format!(
                "effective VFS winner disappeared for {}",
                entry.logical_path
            )
        })
    }

    /// The content key `resolve` would attach to this entry, given the digest
    /// and length of the bytes `read_entry` returned.
    pub fn content_key(
        &self,
        entry: &EffectiveVfsEntry,
        payload_sha256: String,
        payload_bytes: u64,
    ) -> Result<UsedContentKey, String> {
        let layer = self
            .layers
            .get(entry.layer)
            .ok_or("effective VFS entry names a layer this VFS does not mount")?;
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: self.authority_manifest_sha256.clone(),
            stored_game_relative_path: entry.logical_path.clone(),
            normalized_game_relative_path: normalize_game_relative_path(&entry.logical_path)?,
            winning_layer_ordinal: layer.ordinal,
            layer_kind: layer.layer_kind,
            layer_id: layer.layer_id.clone(),
            source_sha256: layer
                .source_container_sha256
                .clone()
                .unwrap_or_else(|| payload_sha256.clone()),
            payload_sha256,
            payload_bytes,
        };
        content.validate()?;
        Ok(content)
    }

    pub fn list_all(
        &self,
        extension: Option<&str>,
    ) -> Result<Vec<EffectiveVfsResolvedFile>, String> {
        let extension = extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
        let mut effective = BTreeMap::<String, String>::new();
        for layer in &self.layers {
            if !layer.replace_paths.is_empty() {
                effective.retain(|_, path| !layer.replace_paths.contains(&logical_parent(path)));
            }
            for path in list_layer_files(layer, "", extension.as_deref())? {
                effective.insert(path.to_ascii_lowercase(), path);
            }
        }
        effective
            .into_values()
            .map(|path| {
                self.resolve(&path)?
                    .ok_or_else(|| format!("effective VFS winner disappeared for {path}"))
            })
            .collect()
    }

    fn resolved(
        &self,
        layer: &MountedLayer,
        logical_path: String,
        bytes: Vec<u8>,
    ) -> Result<EffectiveVfsResolvedFile, String> {
        let payload_sha256 = sha256_bytes(&bytes);
        let normalized_path = normalize_game_relative_path(&logical_path)?;
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: self.authority_manifest_sha256.clone(),
            stored_game_relative_path: logical_path.clone(),
            normalized_game_relative_path: normalized_path,
            winning_layer_ordinal: layer.ordinal,
            layer_kind: layer.layer_kind,
            layer_id: layer.layer_id.clone(),
            source_sha256: layer
                .source_container_sha256
                .clone()
                .unwrap_or_else(|| payload_sha256.clone()),
            payload_sha256,
            payload_bytes: bytes.len() as u64,
        };
        content.validate()?;
        Ok(EffectiveVfsResolvedFile {
            logical_path,
            content,
            bytes,
        })
    }
}

/// `progress` is called with the byte count of every chunk hashed, so a
/// caller can report a multi-gigabyte file as moving while it is read.
fn hash_layer_file(
    layer: &MountedLayer,
    logical_path: &str,
    buffer: &mut Vec<u8>,
    progress: &mut dyn FnMut(usize),
) -> Result<Option<(u64, String)>, String> {
    use sha2::Digest;
    const CHUNK: usize = 1 << 20;
    if buffer.len() < CHUNK {
        buffer.resize(CHUNK, 0);
    }
    let mut hasher = sha2::Sha256::new();
    let mut total = 0u64;
    match layer.storage_kind {
        LayerStorageKind::Directory => {
            match resolve_literal_layer_path(&layer.local_source, logical_path)? {
                Some((path, facts)) if facts.is_file => {
                    let canonical = fs::canonicalize(&path).map_err(|error| {
                        format!("canonicalize effective VFS file failed: {error}")
                    })?;
                    if !canonical.starts_with(&layer.local_source) {
                        return Err("effective VFS file escaped its layer root".into());
                    }
                    if stored_directory_path(&layer.local_source, &canonical)? != logical_path {
                        return Err(format!("effective VFS stored path changed for {logical_path}"));
                    }
                    let mut file = fs::File::open(canonical)
                        .map_err(|error| format!("open effective VFS file failed: {error}"))?;
                    loop {
                        let read = file
                            .read(&mut buffer[..CHUNK])
                            .map_err(|error| format!("read effective VFS file failed: {error}"))?;
                        if read == 0 {
                            break;
                        }
                        hasher.update(&buffer[..read]);
                        total += read as u64;
                        progress(read);
                    }
                }
                Some(_) | None => return Ok(None),
            }
        }
        LayerStorageKind::ZipArchive => {
            let index = layer
                .zip_index
                .as_ref()
                .ok_or("effective VFS archive layer is missing its mount-time index")?;
            let Some((stored_path, entry_index)) = index.entries.get(&logical_path.to_ascii_lowercase())
            else {
                return Ok(None);
            };
            if stored_path != logical_path {
                return Err(format!("effective VFS stored path changed for {logical_path}"));
            }
            let mut archive = index
                .archive
                .lock()
                .map_err(|_| "effective VFS archive handle is poisoned".to_string())?;
            let mut entry = archive
                .by_index(*entry_index)
                .map_err(|error| format!("read effective VFS archive entry failed: {error}"))?;
            loop {
                let read = entry
                    .read(&mut buffer[..CHUNK])
                    .map_err(|error| format!("read effective VFS archive payload failed: {error}"))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                total += read as u64;
                progress(read);
            }
        }
    }
    Ok(Some((total, format!("{:x}", hasher.finalize()))))
}

fn read_layer_file(layer: &MountedLayer, logical_path: &str) -> Result<Option<(String, Vec<u8>)>, String> {
    match layer.storage_kind {
        LayerStorageKind::Directory => {
            match resolve_literal_layer_path(&layer.local_source, logical_path)? {
                Some((path, facts)) if facts.is_file => {
                    let canonical = fs::canonicalize(&path).map_err(|error| {
                        format!("canonicalize effective VFS file failed: {error}")
                    })?;
                    if !canonical.starts_with(&layer.local_source) {
                        return Err("effective VFS file escaped its layer root".into());
                    }
                    let stored_path = stored_directory_path(&layer.local_source, &canonical)?;
                    fs::read(canonical)
                        .map(|bytes| Some((stored_path, bytes)))
                        .map_err(|error| format!("read effective VFS file failed: {error}"))
                }
                Some(_) | None => Ok(None),
            }
        }
        LayerStorageKind::ZipArchive => {
            let index = layer
                .zip_index
                .as_ref()
                .ok_or("effective VFS archive layer is missing its mount-time index")?;
            let Some((stored_path, entry_index)) = index.entries.get(&logical_path.to_ascii_lowercase())
            else {
                return Ok(None);
            };
            let mut archive = index
                .archive
                .lock()
                .map_err(|_| "effective VFS archive handle is poisoned".to_string())?;
            let mut entry = archive
                .by_index(*entry_index)
                .map_err(|error| format!("read effective VFS archive entry failed: {error}"))?;
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|error| format!("read effective VFS archive payload failed: {error}"))?;
            Ok(Some((stored_path.clone(), bytes)))
        }
    }
}

fn stored_directory_path(root: &Path, canonical: &Path) -> Result<String, String> {
    let stored = canonical.strip_prefix(root)
        .map_err(|_| "effective VFS file escaped its layer root")?
        .to_str().ok_or("effective VFS stored path is not UTF-8")?
        .replace('\\', "/");
    crate::validate_stored_game_relative_path(&stored, &normalize_game_relative_path(&stored)?)?;
    Ok(stored)
}

pub fn resolve_literal_layer_path(
    root: &Path,
    logical_path: &str,
) -> Result<Option<(PathBuf, LayerPathFacts)>, String> {
    let logical_path = crate::sanitize_engine_path(logical_path)?;
    let root = engine_namespace_root(root)?;
    let mut path = root.clone();
    let mut last_facts = None;
    for part in logical_path.split('/') {
        path.push(part);
        match stat_ordinary(&path)? {
            Some(facts) if facts.is_reparse_point => {
                return Err(format!(
                    "effective VFS refuses content reached through a reparse point: {}",
                    path.display()
                ));
            }
            Some(facts) => last_facts = Some(facts),
            None => return Ok(None),
        }
    }
    Ok(last_facts.map(|facts| (path, facts)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerPathFacts {
    pub is_dir: bool,
    pub is_file: bool,
    is_reparse_point: bool,
}

#[cfg(windows)]
fn stat_ordinary(path: &Path) -> Result<Option<LayerPathFacts>, String> {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_ATTRIBUTE_NOT_REGULAR: u32 = 0x1040;
    const GET_FILE_EX_INFO_STANDARD: i32 = 0;
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    const ERROR_PATH_NOT_FOUND: u32 = 3;
    const ERROR_INVALID_NAME: u32 = 123;
    const ERROR_BAD_PATHNAME: u32 = 161;
    const ERROR_FILENAME_EXCED_RANGE: u32 = 206;
    const ERROR_DIRECTORY: u32 = 267;

    #[repr(C)]
    #[derive(Default)]
    struct Win32FileAttributeData {
        file_attributes: u32,
        creation_time: [u32; 2],
        last_access_time: [u32; 2],
        last_write_time: [u32; 2],
        file_size_high: u32,
        file_size_low: u32,
    }

    extern "system" {
        fn GetFileAttributesExW(
            file_name: *const u16,
            info_level_id: i32,
            file_information: *mut std::ffi::c_void,
        ) -> i32;
        fn GetLastError() -> u32;
    }

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(format!("content path {} contains a NUL", path.display()));
    }
    if wide.len() >= ENGINE_MAX_PATH {
        return Ok(None);
    }
    wide.push(0);

    let mut data = Win32FileAttributeData::default();
    let ok = unsafe {
        GetFileAttributesExW(
            wide.as_ptr(),
            GET_FILE_EX_INFO_STANDARD,
            std::ptr::from_mut(&mut data).cast(),
        )
    };
    if ok == 0 {
        let error = unsafe { GetLastError() };
        return match error {
            ERROR_FILE_NOT_FOUND
            | ERROR_PATH_NOT_FOUND
            | ERROR_INVALID_NAME
            | ERROR_BAD_PATHNAME
            | ERROR_FILENAME_EXCED_RANGE
            | ERROR_DIRECTORY => Ok(None),
            other => Err(format!(
                "inspect effective VFS file failed: GetFileAttributesExW on {} reported Win32 \
                 error {other}",
                path.display()
            )),
        };
    }
    let attributes = data.file_attributes;
    let is_dir = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    Ok(Some(LayerPathFacts {
        is_dir,
        is_file: !is_dir && attributes & FILE_ATTRIBUTE_NOT_REGULAR == 0,
        is_reparse_point: attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
    }))
}

#[cfg(not(windows))]
fn stat_ordinary(path: &Path) -> Result<Option<LayerPathFacts>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(LayerPathFacts {
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            is_reparse_point: metadata.file_type().is_symlink(),
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("inspect effective VFS file failed: {error}")),
    }
}

#[cfg(windows)]
const ENGINE_MAX_PATH: usize = 260;

#[cfg(windows)]
fn engine_namespace_root(root: &Path) -> Result<PathBuf, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let wide: Vec<u16> = root.as_os_str().encode_wide().collect();
    let lower = |unit: u16| match unit {
        0x41..=0x5A => unit + 0x20,
        other => other,
    };
    let starts_with = |needle: &str| {
        let needle: Vec<u16> = needle.encode_utf16().collect();
        wide.len() >= needle.len()
            && wide[..needle.len()]
                .iter()
                .zip(&needle)
                .all(|(a, b)| lower(*a) == lower(*b))
    };
    let rebuild = |from: usize, prefix: &str| {
        let mut units: Vec<u16> = prefix.encode_utf16().collect();
        units.extend_from_slice(&wide[from..]);
        PathBuf::from(OsString::from_wide(&units))
    };

    let needs_verbatim = |from: usize| {
        let mut component = Vec::new();
        let mut offending = false;
        for unit in wide[from..]
            .iter()
            .copied()
            .chain(std::iter::once(u16::from(b'\\')))
        {
            if unit == u16::from(b'\\') {
                if matches!(component.last().copied(), Some(c) if c == u16::from(b'.') || c == u16::from(b' '))
                {
                    offending = true;
                }
                component.clear();
            } else {
                component.push(unit);
            }
        }
        offending
    };

    if starts_with(r"\\?\UNC\") {
        if needs_verbatim(r"\\?\UNC\".len()) {
            return Err(format!(
                "content layer root {} has a component ending in a dot or space, which only the \
                 verbatim namespace can address; the engine addresses content by ordinary DOS \
                 path and would reach a different object",
                root.display()
            ));
        }
        return Ok(rebuild(r"\\?\UNC\".len(), r"\\"));
    }
    if starts_with(r"\\?\") {
        let rest = &wide[r"\\?\".len()..];
        let is_drive = rest.len() >= 3
            && char::from_u32(u32::from(rest[0])).is_some_and(|c| c.is_ascii_alphabetic())
            && rest[1] == u16::from(b':')
            && rest[2] == u16::from(b'\\');
        if is_drive {
            if needs_verbatim(r"\\?\".len()) {
                return Err(format!(
                    "content layer root {} has a component ending in a dot or space, which only \
                     the verbatim namespace can address; the engine addresses content by ordinary \
                     DOS path and would reach a different object",
                    root.display()
                ));
            }
            return Ok(rebuild(r"\\?\".len(), ""));
        }
        return Err(format!(
            "content layer root {} is in a verbatim namespace with no ordinary equivalent; \
             the engine addresses content by ordinary DOS path and this root cannot be \
             expressed as one",
            root.display()
        ));
    }
    Ok(root.to_path_buf())
}

#[cfg(not(windows))]
fn engine_namespace_root(root: &Path) -> Result<PathBuf, String> {
    Ok(root.to_path_buf())
}

fn index_zip_layer(local_source: &Path, expected_sha256: &str) -> Result<ZipLayerIndex, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        // Retain one exact archive object for the mount lifetime and refuse
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        options.share_mode(FILE_SHARE_READ);
    }
    let mut file = options
        .open(local_source)
        .map_err(|error| format!("open effective VFS archive failed: {error}"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|error| format!("hash effective VFS source failed: {error}"))?;
    let actual_sha256 = format!("{:x}", hasher.finalize());
    if actual_sha256 != expected_sha256 {
        return Err("archive VFS layer source-container digest mismatch".into());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind effective VFS archive failed: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("parse effective VFS archive failed: {error}"))?;
    if archive.is_empty() {
        return Err("effective VFS archive is empty".into());
    }
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("read effective VFS archive entry failed: {error}"))?;
        if entry.encrypted() {
            return Err("effective VFS archive contains an encrypted entry".into());
        }
        if entry.is_symlink() {
            return Err("effective VFS archive contains a symbolic-link entry".into());
        }
        if entry.is_dir() {
            continue;
        }
        if !entry.is_file() {
            return Err("effective VFS archive contains an unsupported entry type".into());
        }
        let name = normalize_archive_entry(entry.name())?;
        if entries
            .insert(name.to_ascii_lowercase(), (name, index))
            .is_some()
        {
            return Err("effective VFS archive repeats a case-insensitive path".into());
        }
    }
    if entries.is_empty() {
        return Err("effective VFS archive has no regular files".into());
    }
    Ok(ZipLayerIndex {
        entries,
        archive: Arc::new(Mutex::new(archive)),
    })
}

fn list_layer_files(
    layer: &MountedLayer,
    prefix: &str,
    extension: Option<&str>,
) -> Result<Vec<String>, String> {
    Ok(list_layer_entries(layer, prefix, extension)?
        .into_iter()
        .map(|listed| listed.path)
        .collect())
}

fn list_layer_entries(
    layer: &MountedLayer,
    prefix: &str,
    extension: Option<&str>,
) -> Result<Vec<ListedFile>, String> {
    let mut paths = BTreeMap::<String, ListedFile>::new();
    match layer.storage_kind {
        LayerStorageKind::Directory => {
            let start = if prefix.is_empty() {
                layer.local_source.clone()
            } else {
                path_from_logical(&layer.local_source, prefix)
            };
            if !start.exists() {
                return Ok(Vec::new());
            }
            walk_directory(&layer.local_source, &start, extension, &mut paths)?;
        }
        LayerStorageKind::ZipArchive => {
            let index = layer
                .zip_index
                .as_ref()
                .ok_or("effective VFS archive layer is missing its mount-time index")?;
            for (path, _) in index.entries.values() {
                if path_is_under(path, prefix) && extension_matches(path, extension) {
                    paths.insert(
                        path.to_ascii_lowercase(),
                        ListedFile {
                            path: path.clone(),
                            local_path: layer.local_source.clone(),
                            len: 0,
                            modified_unix_secs: 0,
                            modified_nanos: 0,
                        },
                    );
                }
            }
        }
    }
    Ok(paths.into_values().collect())
}

/// The facts a directory enumeration carries for one entry. On Windows the
/// find data behind `DirEntry::metadata` already holds them, so this is no
/// extra system call per file; the attribute word is what `stat_ordinary`
/// reads, so a reparse point of ANY tag is refused here exactly as `resolve`
/// refuses it, and a listing can stand in for a read on that question.
fn listed_facts(entry: &fs::DirEntry) -> Result<(bool, bool, u64, i64, u32), String> {
    let metadata = entry
        .metadata()
        .map_err(|error| format!("inspect effective VFS entry failed: {error}"))?;
    #[cfg(windows)]
    let (is_dir, is_file, is_reparse_point) = {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        const FILE_ATTRIBUTE_NOT_REGULAR: u32 = 0x1040;
        let attributes = metadata.file_attributes();
        let is_dir = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        (
            is_dir,
            !is_dir && attributes & FILE_ATTRIBUTE_NOT_REGULAR == 0,
            attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        )
    };
    #[cfg(not(windows))]
    let (is_dir, is_file, is_reparse_point) = {
        let file_type = metadata.file_type();
        (
            file_type.is_dir(),
            file_type.is_file(),
            file_type.is_symlink(),
        )
    };
    if is_reparse_point {
        return Err(format!(
            "effective VFS refuses content reached through a reparse point: {}",
            entry.path().display()
        ));
    }
    let (secs, nanos) = match metadata.modified() {
        Ok(modified) => match modified.duration_since(std::time::UNIX_EPOCH) {
            Ok(since) => (i64::try_from(since.as_secs()).unwrap_or(i64::MAX), since.subsec_nanos()),
            Err(before) => {
                let before = before.duration();
                (
                    i64::try_from(before.as_secs()).map(|s| -s).unwrap_or(i64::MIN),
                    before.subsec_nanos(),
                )
            }
        },
        Err(error) => {
            return Err(format!(
                "inspect effective VFS entry modification time failed: {error}"
            ))
        }
    };
    Ok((is_dir, is_file, metadata.len(), secs, nanos))
}

fn walk_directory(
    root: &Path,
    directory: &Path,
    extension: Option<&str>,
    out: &mut BTreeMap<String, ListedFile>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("inspect effective VFS directory failed: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("effective VFS refuses a symlinked content directory".into());
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("list effective VFS directory failed: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("list effective VFS entry failed: {error}"))?;
    entries.sort_by(|left, right| {
        let left = left.file_name().to_string_lossy().replace('\\', "/");
        let right = right.file_name().to_string_lossy().replace('\\', "/");
        crate::authored_ui::engine_recursive_path_cmp(&left, &right)
    });
    for entry in entries {
        let (is_dir, is_file, len, modified_unix_secs, modified_nanos) = listed_facts(&entry)?;
        if is_dir {
            walk_directory(root, &entry.path(), extension, out)?;
        } else if is_file {
            let local_path = entry.path();
            let relative = local_path
                .strip_prefix(root)
                .map_err(|_| "effective VFS file escaped its layer root")?
                .to_string_lossy()
                .replace('\\', "/");
            let path = normalize_game_relative_path_preserve_case(&relative)?;
            if extension_matches(&path, extension)
                && out
                    .insert(
                        path.to_ascii_lowercase(),
                        ListedFile {
                            path,
                            local_path,
                            len,
                            modified_unix_secs,
                            modified_nanos,
                        },
                    )
                    .is_some()
            {
                return Err("effective VFS directory repeats a case-insensitive path".into());
            }
        }
    }
    Ok(())
}

fn normalize_archive_entry(name: &str) -> Result<String, String> {
    let slash = name.replace('\\', "/");
    if slash.starts_with('/') || slash.contains('\0') {
        return Err("effective VFS archive contains an absolute entry path".into());
    }
    let mut parts = Vec::new();
    for part in slash.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return Err("effective VFS archive contains an unsafe entry path".into());
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err("effective VFS archive contains an empty entry path".into());
    }
    normalize_game_relative_path_preserve_case(&parts.join("/"))
}

fn normalize_game_relative_path_preserve_case(path: &str) -> Result<String, String> {
    let preserved = path.trim().replace('\\', "/");
    normalize_game_relative_path(&preserved)?;
    Ok(preserved)
}

fn path_from_logical(root: &Path, logical_path: &str) -> PathBuf {
    logical_path
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn path_is_under(path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || path.eq_ignore_ascii_case(prefix)
        || path
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            && path.as_bytes().get(prefix.len()) == Some(&b'/')
}

fn extension_matches(path: &str, extension: Option<&str>) -> bool {
    extension.is_none_or(|expected| {
        Path::new(path)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
    })
}

fn logical_parent(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_ascii_lowercase())
        .unwrap_or_default()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "portable_effective_vfs_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write(root: &Path, path: &str, bytes: &[u8]) {
        let target = path_from_logical(root, path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }

    fn mounts(root: &Path) -> Vec<EffectiveVfsLayerMount> {
        vec![
            EffectiveVfsLayerMount {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                storage_kind: LayerStorageKind::Directory,
                local_source: root.join("game"),
                source_container_sha256: None,
                replace_paths: Vec::new(),
            },
            EffectiveVfsLayerMount {
                ordinal: 1,
                layer_kind: ContentLayerKind::Mod,
                layer_id: "mod/active.mod".into(),
                storage_kind: LayerStorageKind::Directory,
                local_source: root.join("mod"),
                source_container_sha256: None,
                replace_paths: vec!["gfx/replace".into()],
            },
        ]
    }

    #[test]
    fn stored_spelling_comes_from_the_mod_winner_and_survives_alias_lookup() {
        let root = temp("stored_case");
        write(&root.join("game"), "gfx/models/shared/Texture.dds", b"vanilla");
        write(&root.join("mod"), "gfx/models/shared/Texture.dds", b"mod");
        let vfs = EffectiveVfs::new("f".repeat(64), mounts(&root)).unwrap();
        let resolved = vfs.resolve_required("gfx/models/shared/Texture.dds", "texture").unwrap();
        assert_eq!(resolved.bytes, b"mod");
        assert_eq!(resolved.content.stored_game_relative_path, "gfx/models/shared/Texture.dds");
        assert_eq!(resolved.content.normalized_game_relative_path, "gfx/models/shared/texture.dds");
        #[cfg(windows)]
        assert_eq!(vfs.resolve_required("GFX/MODELS/SHARED/TEXTURE.DDS", "alias").unwrap().content, resolved.content);
        let entry = vfs.effective_entries().unwrap().into_iter().find(|entry| entry.logical_path.ends_with("Texture.dds")).unwrap();
        let bytes = vfs.read_entry(&entry).unwrap();
        assert_eq!(vfs.content_key(&entry, sha256_bytes(&bytes), bytes.len() as u64).unwrap(), resolved.content);
        drop(vfs);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn override_replace_path_and_relocation_are_exact() {
        let mut resolved = Vec::new();
        for label in ["a", "b"] {
            let root = temp(label);
            write(&root.join("game"), "gfx/fx/shared.shader", b"base");
            write(&root.join("game"), "gfx/replace/removed.shader", b"removed");
            write(
                &root.join("game"),
                "gfx/replace/sub/retained.shader",
                b"retained",
            );
            write(&root.join("mod"), "gfx/fx/shared.shader", b"override");
            write(&root.join("mod"), "gfx/fx/new.shader", b"new");
            let vfs = EffectiveVfs::new("a".repeat(64), mounts(&root)).unwrap();
            assert_eq!(
                vfs.resolve_required("gfx/fx/shared.shader", "override")
                    .unwrap()
                    .bytes,
                b"override"
            );
            assert!(vfs.resolve("gfx/replace/removed.shader").unwrap().is_none());
            assert_eq!(
                vfs.resolve_required("gfx/replace/sub/retained.shader", "nested")
                    .unwrap()
                    .bytes,
                b"retained"
            );
            resolved.push(vfs.list_recursive("gfx", Some("shader")).unwrap());
            portable_test_fs_retry::remove_dir_all(root).unwrap();
        }
        assert_eq!(resolved[0], resolved[1]);
    }

    #[test]
    fn a_layer_declaring_one_directory_under_two_windows_spellings_is_refused() {
        for alias in ["gfx/replace.", "gfx/replace..", "gfx /replace"] {
            let root = temp("alias-replace");
            write(&root.join("game"), "gfx/fx/shared.shader", b"base");
            write(&root.join("mod"), "gfx/fx/shared.shader", b"override");
            let mut mounts = mounts(&root);
            mounts[1].replace_paths = vec!["gfx/replace".into(), alias.into()];
            let error = EffectiveVfs::new("a".repeat(64), mounts)
                .expect_err("two spellings of one directory must be refused");
            assert!(
                error.contains("invalid or repeated replace_path"),
                "{alias:?}: {error}"
            );
            portable_test_fs_retry::remove_dir_all(root).unwrap();
        }
        let root = temp("distinct-replace");
        write(&root.join("game"), "gfx/fx/shared.shader", b"base");
        write(&root.join("mod"), "gfx/fx/shared.shader", b"override");
        let mut mounts = mounts(&root);
        mounts[1].replace_paths = vec!["gfx/replace".into(), "gfx/other".into()];
        EffectiveVfs::new("a".repeat(64), mounts).unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_archive_dlc_is_hashed_and_optional() {
        let root = temp("archive");
        fs::create_dir_all(root.join("game")).unwrap();
        write(&root.join("game"), "gfx/fx/value.shader", b"base");
        let archive_path = root.join("selected.zip");
        let file = fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "gfx/fx/value.shader",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"selected-dlc").unwrap();
        writer.finish().unwrap();
        let archive_sha = sha256_bytes(&portable_test_fs_retry::read(&archive_path).unwrap());
        let base_mount = EffectiveVfsLayerMount {
            ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            storage_kind: LayerStorageKind::Directory,
            local_source: root.join("game"),
            source_container_sha256: None,
            replace_paths: Vec::new(),
        };
        let base = EffectiveVfs::new("b".repeat(64), [base_mount.clone()]).unwrap();
        assert_eq!(
            base.resolve_required("gfx/fx/value.shader", "base")
                .unwrap()
                .bytes,
            b"base"
        );
        let selected = EffectiveVfs::new(
            "b".repeat(64),
            [
                base_mount,
                EffectiveVfsLayerMount {
                    ordinal: 4,
                    layer_kind: ContentLayerKind::Dlc,
                    layer_id: "dlc/selected/selected.dlc".into(),
                    storage_kind: LayerStorageKind::ZipArchive,
                    local_source: archive_path,
                    source_container_sha256: Some(archive_sha),
                    replace_paths: Vec::new(),
                },
            ],
        )
        .unwrap();
        let winner = selected
            .resolve_required("gfx/fx/value.shader", "selected DLC")
            .unwrap();
        assert_eq!(winner.bytes, b"selected-dlc");
        assert_eq!(winner.content.winning_layer_ordinal, 4);
        assert_eq!(winner.content.layer_kind, ContentLayerKind::Dlc);
        drop(selected);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_with_case_insensitive_duplicate_entries_is_refused_at_mount() {
        let root = temp("dup_archive");
        fs::create_dir_all(root.join("game")).unwrap();
        write(&root.join("game"), "gfx/fx/value.shader", b"base");
        let archive_path = root.join("duplicate.zip");
        let file = fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        for name in ["gfx/fx/Case.shader", "gfx/fx/case.shader"] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"payload").unwrap();
        }
        writer.finish().unwrap();
        let archive_sha = sha256_bytes(&portable_test_fs_retry::read(&archive_path).unwrap());
        let error = EffectiveVfs::new(
            "c".repeat(64),
            [
                EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: root.join("game"),
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
                EffectiveVfsLayerMount {
                    ordinal: 1,
                    layer_kind: ContentLayerKind::Dlc,
                    layer_id: "dlc/dup/dup.dlc".into(),
                    storage_kind: LayerStorageKind::ZipArchive,
                    local_source: archive_path,
                    source_container_sha256: Some(archive_sha),
                    replace_paths: Vec::new(),
                },
            ],
        )
        .unwrap_err();
        assert!(
            error.contains("repeats a case-insensitive path"),
            "unexpected mount error: {error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_entry_normalization_refuses_absolute_names() {
        assert!(normalize_archive_entry("/gfx/fx/value.shader").is_err());
        assert!(normalize_archive_entry(r"\gfx\fx\value.shader").is_err());
        assert!(normalize_archive_entry("../gfx/fx/value.shader").is_err());
        assert_eq!(
            normalize_archive_entry("gfx//./fx/value.shader").unwrap(),
            "gfx/fx/value.shader"
        );
    }

    #[test]
    fn empty_archive_is_refused_at_mount() {
        let root = temp("empty_archive");
        fs::create_dir_all(root.join("game")).unwrap();
        let archive_path = root.join("empty.zip");
        let writer = zip::ZipWriter::new(fs::File::create(&archive_path).unwrap());
        writer.finish().unwrap();
        let archive_sha = sha256_bytes(&portable_test_fs_retry::read(&archive_path).unwrap());
        let error = EffectiveVfs::new(
            "d".repeat(64),
            [
                EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: root.join("game"),
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
                EffectiveVfsLayerMount {
                    ordinal: 1,
                    layer_kind: ContentLayerKind::Mod,
                    layer_id: "mod/empty.mod".into(),
                    storage_kind: LayerStorageKind::ZipArchive,
                    local_source: archive_path,
                    source_container_sha256: Some(archive_sha),
                    replace_paths: Vec::new(),
                },
            ],
        )
        .unwrap_err();
        assert!(error.contains("empty"), "unexpected mount error: {error}");
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod engine_namespace_tests {
    use super::*;

    #[test]
    fn engine_path_grammar_matches_the_sanitizer() {
        assert_eq!(
            crate::sanitize_engine_path("gfx/unit.dds").unwrap(),
            "gfx/unit.dds"
        );
        assert_eq!(
            crate::sanitize_engine_path("gfx//unit.dds").unwrap(),
            "gfx/unit.dds"
        );
        assert_eq!(
            crate::sanitize_engine_path("gfx/unit.dds/").unwrap(),
            "gfx/unit.dds"
        );
        for bad in ["gfx/./unit.dds", "gfx/../unit.dds", ".", ".."] {
            assert!(
                crate::sanitize_engine_path(bad).is_err(),
                "{bad:?} must be refused by the engine grammar"
            );
        }
        for bad in [r"gfx\unit.dds", "c:/gfx/unit.dds", "gfx/a:b"] {
            assert!(
                crate::sanitize_engine_path(bad).is_err(),
                "{bad:?} must be refused by the engine grammar"
            );
        }
        assert!(crate::sanitize_engine_path("//").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn a_verbatim_root_without_an_ordinary_equivalent_is_refused() {
        assert_eq!(
            engine_namespace_root(Path::new(r"\\?\C:\game")).unwrap(),
            PathBuf::from(r"C:\game")
        );
        assert_eq!(
            engine_namespace_root(Path::new(r"\\?\UNC\host\share\game")).unwrap(),
            PathBuf::from(r"\\host\share\game")
        );
        assert_eq!(
            engine_namespace_root(Path::new(r"C:\game")).unwrap(),
            PathBuf::from(r"C:\game")
        );
        let guid = Path::new(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\game");
        assert!(
            engine_namespace_root(guid).is_err(),
            "a volume-GUID root must refuse rather than become a relative path"
        );
        assert!(
            engine_namespace_root(Path::new(r"\\?\C:\mods\foo.")).is_err(),
            "a verbatim root with a dot-terminated component must refuse"
        );
        assert!(
            engine_namespace_root(Path::new(r"\\?\C:\mods\foo \bar")).is_err(),
            "a verbatim root with a space-terminated component must refuse"
        );
    }

    #[test]
    fn the_primitive_applies_the_engine_grammar_itself() {
        let root = std::env::temp_dir().join(format!(
            "la_two_vfs_primitive_grammar_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(root.join("common")).expect("create fixture");
        fs::write(root.join("common").join("ideas.txt"), b"ideas").expect("write fixture");

        assert!(
            resolve_literal_layer_path(&root, "common/ideas.txt")
                .expect("clean spelling")
                .is_some(),
            "the fixture does not resolve, so this test would prove nothing"
        );

        for bad in ["common/./ideas.txt", "common/../common/ideas.txt"] {
            assert!(
                resolve_literal_layer_path(&root, bad).is_err(),
                "the primitive must apply the engine grammar to {bad:?} itself"
            );
        }
        assert!(
            resolve_literal_layer_path(&root, "common//ideas.txt")
                .expect("collapsing spelling")
                .is_some(),
            "an empty component must collapse, not refuse"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
