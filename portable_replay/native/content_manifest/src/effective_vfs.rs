//! Neutral launcher-equivalent effective VFS over already selected content layers.
//!
//! Local roots exist only while recording or playing. They are never serialized;
//! portable identity is the exact [`UsedContentKey`] returned with every read.

use crate::hud_role::validate_sha256;
use crate::{normalize_game_relative_path, ContentLayerKind, LayerStorageKind, UsedContentKey};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

#[derive(Clone, Debug)]
struct MountedLayer {
    ordinal: u32,
    layer_kind: ContentLayerKind,
    layer_id: String,
    storage_kind: LayerStorageKind,
    local_source: PathBuf,
    source_container_sha256: Option<String>,
    replace_paths: BTreeSet<String>,
    /// Mount-time index for archive layers: the archive is opened and its
    /// central directory scanned exactly once; every entry name is normalized
    /// (and case-insensitive duplicates refused) up front so per-file resolve
    /// and listing never re-parse the archive.
    zip_index: Option<ZipLayerIndex>,
}

#[derive(Clone, Debug)]
struct ZipLayerIndex {
    /// Normalized (lowercase, game-relative) entry name -> archive entry index.
    entries: BTreeMap<String, usize>,
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
            // Strictly increasing ordinals already make every
            // (ordinal, kind, id) tuple unique; no separate identity set is
            // needed.
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
            for path in mount.replace_paths {
                let normalized = normalize_game_relative_path(&path)?;
                if normalized != path.replace('\\', "/")
                    || !replace_paths.insert(normalized.to_ascii_lowercase())
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
        let logical_path = normalize_game_relative_path(logical_path)?;
        let parent = logical_parent(&logical_path);
        for layer in self.layers.iter().rev() {
            if let Some(bytes) = read_layer_file(layer, &logical_path)? {
                return Ok(Some(self.resolved(layer, logical_path, bytes)?));
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
        let prefix = normalize_game_relative_path(logical_prefix)?;
        let extension = extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
        let mut effective = BTreeMap::<String, String>::new();
        for layer in &self.layers {
            if !layer.replace_paths.is_empty() {
                effective.retain(|_, path| !layer.replace_paths.contains(&logical_parent(path)));
            }
            for path in list_layer_files(layer, &prefix, extension.as_deref())? {
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
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: self.authority_manifest_sha256.clone(),
            normalized_game_relative_path: logical_path.clone(),
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

fn read_layer_file(layer: &MountedLayer, logical_path: &str) -> Result<Option<Vec<u8>>, String> {
    match layer.storage_kind {
        LayerStorageKind::Directory => {
            match checked_directory_path(&layer.local_source, logical_path)? {
                Some((path, metadata)) if metadata.is_file() => {
                    let canonical = fs::canonicalize(&path).map_err(|error| {
                        format!("canonicalize effective VFS file failed: {error}")
                    })?;
                    if !canonical.starts_with(&layer.local_source) {
                        return Err("effective VFS file escaped its layer root".into());
                    }
                    fs::read(canonical)
                        .map(Some)
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
            let Some(&entry_index) = index.entries.get(logical_path) else {
                return Ok(None);
            };
            let mut archive = index
                .archive
                .lock()
                .map_err(|_| "effective VFS archive handle is poisoned".to_string())?;
            let mut entry = archive
                .by_index(entry_index)
                .map_err(|error| format!("read effective VFS archive entry failed: {error}"))?;
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|error| format!("read effective VFS archive payload failed: {error}"))?;
            Ok(Some(bytes))
        }
    }
}

fn checked_directory_path(
    root: &Path,
    logical_path: &str,
) -> Result<Option<(PathBuf, fs::Metadata)>, String> {
    let mut path = root.to_path_buf();
    let mut last_metadata = None;
    for part in logical_path.split('/') {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("effective VFS refuses symlinked content".into());
            }
            Ok(metadata) => last_metadata = Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("inspect effective VFS file failed: {error}"));
            }
        }
    }
    Ok(last_metadata.map(|metadata| (path, metadata)))
}

/// Open and scan an archive layer exactly once at mount, refusing invalid or
/// case-insensitively duplicate entry names up front (the same refusals the
/// per-read/per-list paths used to raise lazily).
fn index_zip_layer(local_source: &Path, expected_sha256: &str) -> Result<ZipLayerIndex, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        // Retain one exact archive object for the mount lifetime and refuse
        // concurrent replacement or writer opens on the production platform.
        // FILE_SHARE_READ is deliberately allowed for independent readers.
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
        if entries.insert(name, index).is_some() {
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
    let mut paths = BTreeMap::<String, String>::new();
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
            // Entry names were normalized and de-duplicated at mount; listing
            // is a pure walk over the prebuilt index with no archive I/O.
            for path in index.entries.keys() {
                if path_is_under(path, prefix) && extension_matches(path, extension) {
                    paths.insert(path.to_ascii_lowercase(), path.clone());
                }
            }
        }
    }
    Ok(paths.into_values().collect())
}

fn walk_directory(
    root: &Path,
    directory: &Path,
    extension: Option<&str>,
    out: &mut BTreeMap<String, String>,
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
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect effective VFS entry failed: {error}"))?;
        if file_type.is_symlink() {
            return Err("effective VFS refuses symlinked content".into());
        }
        if file_type.is_dir() {
            walk_directory(root, &entry.path(), extension, out)?;
        } else if file_type.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| "effective VFS file escaped its layer root")?
                .to_string_lossy()
                .replace('\\', "/");
            let path = normalize_game_relative_path(&relative)?;
            if extension_matches(&path, extension)
                && out.insert(path.to_ascii_lowercase(), path).is_some()
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
    // PHYSFS treats duplicate separators and `.` components as the same
    // virtual name. Canonicalize those spellings before collision selection,
    // while parent traversal and drive/URI grammars remain hard failures.
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
    normalize_game_relative_path(&parts.join("/"))
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
            fs::remove_dir_all(root).unwrap();
        }
        assert_eq!(resolved[0], resolved[1]);
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
        let archive_sha = sha256_bytes(&fs::read(&archive_path).unwrap());
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
        fs::remove_dir_all(root).unwrap();
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
        let archive_sha = sha256_bytes(&fs::read(&archive_path).unwrap());
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
        fs::remove_dir_all(root).unwrap();
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
        let archive_sha = sha256_bytes(&fs::read(&archive_path).unwrap());
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
        fs::remove_dir_all(root).unwrap();
    }
}
