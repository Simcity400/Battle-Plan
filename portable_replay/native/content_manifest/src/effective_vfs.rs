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

/// What one [`EffectiveVfs::walk_content_keys`] pass covered. Reported so a
/// recording can state the size and cost of its own content census instead of
/// leaving a reader to infer it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectiveVfsWalkStats {
    pub files: u64,
    pub payload_bytes: u64,
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
    /// Case-folded entry identity -> case-preserved spelling and archive index.
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
            let mut replace_path_collisions = BTreeSet::new();
            for path in mount.replace_paths {
                let normalized = normalize_game_relative_path(&path)?;
                // REPETITION is decided on the Windows collision key (the
                // previous `to_ascii_lowercase` was a no-op on an already
                // ASCII-lowercased canonical form, so `gfx/entities` and
                // `gfx/entities.` -- one directory on the host -- both mounted).
                // The set kept for LOOKUP stays canonical: `resolve` matches a
                // declared prefix against a logical parent, and a matching rule
                // must not inherit the collision key's deliberate over-folding.
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

    /// Every logical path this VFS serves, winner-resolved, lowest to highest
    /// priority — the identity half of [`list_all`] without its bytes.
    ///
    /// `list_all` materializes every payload into one `Vec`, which is fine for
    /// a directory of catalogs and impossible for a whole install: the corpus
    /// this walks on a stock the game with its DLC is 58,224 files and 10.01 GB.
    /// This resolves one file at a time, hands its [`UsedContentKey`] to the
    /// sink, and drops the bytes before reading the next, so peak memory is one
    /// file rather than the install.
    ///
    /// The sink decides what to keep and may refuse (returning `Err`), which
    /// stops the walk — that is how a caller enforces its own row/byte ceiling
    /// without this function inventing one.
    pub fn walk_content_keys(
        &self,
        mut sink: impl FnMut(&str, UsedContentKey) -> Result<(), String>,
    ) -> Result<EffectiveVfsWalkStats, String> {
        let mut effective = BTreeMap::<String, String>::new();
        for layer in &self.layers {
            if !layer.replace_paths.is_empty() {
                effective.retain(|_, path| !layer.replace_paths.contains(&logical_parent(path)));
            }
            for path in list_layer_files(layer, "", None)? {
                effective.insert(path.to_ascii_lowercase(), path);
            }
        }
        let mut stats = EffectiveVfsWalkStats::default();
        for path in effective.into_values() {
            let file = self
                .resolve(&path)?
                .ok_or_else(|| format!("effective VFS winner disappeared for {path}"))?;
            stats.files = stats.files.saturating_add(1);
            stats.payload_bytes = stats
                .payload_bytes
                .checked_add(file.content.payload_bytes)
                .ok_or_else(|| "effective VFS walk byte count overflowed".to_string())?;
            sink(&file.logical_path, file.content)?;
        }
        Ok(stats)
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

fn read_layer_file(layer: &MountedLayer, logical_path: &str) -> Result<Option<Vec<u8>>, String> {
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
            let Some((_, entry_index)) = index.entries.get(&logical_path.to_ascii_lowercase()) else {
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
            Ok(Some(bytes))
        }
    }
}

/// THE point lookup inside one content layer, shared by both of this
/// repository's layered-VFS implementations.
///
/// It pushes each requested component onto the layer root **literally** and
/// asks the host filesystem about it. Nothing here folds case, strips anything,
/// or compares one name against another: the volume decides what the name
/// means, exactly as the engine's does.
///
/// WHY THAT IS THE RULE, from `hoi4.exe 1.19.2.0.a729`
/// (`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`) — full
/// working in `extractor/native/runs/la_two_vfs/engine_lookup_rule_from_a729.md`:
///
/// * `PHYSFS_stat` (`0x1424e0b00`) walks the search path and takes the FIRST
///   hit. It has no ambiguity concept, so neither has this.
/// * `sanitizePlatformIndependentPath` (`0x1424e3ff0`) rejects `:`, `\`, `.`
///   and `..` components, collapses `//`, and copies every other byte
///   **verbatim**. It folds nothing.
/// * `verifyPath` (`0x1424e4250`) stats each `/`-separated ancestor prefix and
///   refuses with `PHYSFS_ERR_SYMLINK_FORBIDDEN` if **any** of them is a
///   symlink, without ever asking where the link points — which is why the
///   refusal below is per component and unconditional rather than an
///   "escapes the root" test.
/// * `__PHYSFS_platformStat` (`0x1424ff8e0`) converts UTF-8 to UTF-16 and calls
///   `GetFileAttributesExW` on the LITERAL path.
///
/// AND WHY NO TABLE COULD BE THE RULE, independently of that read: NTFS folds
/// through the `$UpCase` file written when the volume was FORMATTED, and that
/// table has changed across Windows versions — a mapping can be gained OR lost
/// (see [`crate::windows_collision_key`]'s doc comment, which measured it). A
/// fold compiled into this binary is therefore not even an upper bound on the
/// playback volume's. Asking the volume is the only answer that is right on
/// every volume, and it is what the engine does.
///
/// One deliberate departure, in the fail-closed direction, stated at its true
/// width: this refuses EVERY reparse point -- symlink, junction, mount point,
/// Cloud Files placeholder, any tag at all. The engine is narrower. It reads the
/// reparse TAG (`__PHYSFS_platformStat` calls `FindFirstFileW` and compares
/// against `IO_REPARSE_TAG_SYMLINK`) and treats only that one as a link, so
/// content behind a junction or a placeholder is opened by the engine and
/// refused here.
///
/// That is a knowing departure, not an oversight: refusing content is a named
/// failure the operator can see, while following a reparse point out of the
/// mounted layer would silently seal or serve content from somewhere else. An
/// earlier version of this comment described only the junction case, which
/// understated what the code does.
///
/// Returns `Ok(None)` when any component is absent — an absent path is not an
/// error — and the final component's own [`LayerPathFacts`] otherwise, so the caller
/// decides whether it wanted a file or a directory.
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

/// What one existing path IS, in the only three terms any caller here needs.
///
/// Deliberately not `fs::Metadata`: producing one means going back through
/// `std::fs`, and [`stat_ordinary`] exists precisely because `std::fs` is the
/// wrong instrument for this question on Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerPathFacts {
    pub is_dir: bool,
    pub is_file: bool,
    is_reparse_point: bool,
}

/// Stat one path the way the ENGINE stats it, and in particular NOT the way
/// `std::fs` does.
///
/// `std::fs` is unusable for this question on Windows, for a reason that is not
/// obvious and cost this lane a review round: `maybe_verbatim` silently rewrites
/// a path into the `\\?\` namespace once it approaches the legacy limit -- at
/// Rust's conservative threshold, which is BELOW `MAX_PATH`, not at it. So a
/// path in that window is handed to the kernel with Win32 parsing suppressed
/// even though it is short enough for the engine to open normally, and it would
/// answer differently from the engine on exactly the spellings this primitive
/// exists to get right (trailing dots, `//`, DOS devices). Checking the length
/// before calling `std::fs` does NOT fix that: the promotion happens below the
/// limit being checked.
///
/// Calling `GetFileAttributesExW` directly is what removes the question. It is
/// the same call `__PHYSFS_platformStat` (`0x1424ff8e0`) makes, on the same kind
/// of ordinary path, so what Win32 does to the spelling here is what it does to
/// the engine's.
///
/// Past `MAX_PATH` this reports ABSENT rather than erroring, because that is
/// what the path IS to a process that is not long-path aware -- and the pinned
/// executable is not (empty application manifest; `longPathAware` occurs zero
/// times in the image; working cited on [`engine_namespace_root`]).
#[cfg(windows)]
fn stat_ordinary(path: &Path) -> Result<Option<LayerPathFacts>, String> {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    /// `FILE_ATTRIBUTE_DEVICE | FILE_ATTRIBUTE_OFFLINE`. The engine tests
    /// exactly this mask and classifies a match as its OTHER kind rather than
    /// REGULAR (`__PHYSFS_platformStat`, `0x1424ff8e0`:
    /// `if ((attributes & 0x1040) == 0) type = REGULAR; else type = OTHER;`).
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
    // NUL is checked BEFORE the length, so an over-long path that also contains
    // one is still refused rather than quietly reported absent -- implementation
    // A rejects NUL before it ever gets here, and the two must not part company
    // over which failure they saw first.
    if wide.contains(&0) {
        return Err(format!("content path {} contains a NUL", path.display()));
    }
    // The NUL is part of what must fit: a non-verbatim Win32 path is at most
    // MAX_PATH units INCLUDING it, so the string itself is at most 259.
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
            // "It is not there" in all the shapes Win32 says it. A malformed or
            // over-long name is ALSO absent rather than an error: it names no
            // file, which is exactly what the engine would conclude.
            // ERROR_ACCESS_DENIED, ERROR_NOT_READY and ERROR_INVALID_DRIVE are
            // deliberately NOT here: they mean inaccessible or unavailable, not
            // proven absent, and a mounted layer whose drive has gone away must
            // fail loudly rather than resolve to nothing.
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
        // NOT simply `!is_dir`. `GetFileAttributesExW` has no positive
        // "regular file" bit, so a device or an offline placeholder would
        // otherwise be handed to callers as an ordinary file to read -- and both
        // callers treat `is_file` as permission to read payload bytes. The
        // engine does not: it classifies `attributes & 0x1040` as OTHER rather
        // than REGULAR, and this mirrors that test exactly.
        is_file: !is_dir && attributes & FILE_ATTRIBUTE_NOT_REGULAR == 0,
        is_reparse_point: attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
    }))
}

/// Non-Windows hosts have no verbatim namespace and no `MAX_PATH`, so the
/// portable call is the right one there.
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

/// `MAX_PATH`. The full path, NUL included, must fit in this many UTF-16 units
/// for a non-verbatim Win32 call, so the string itself may be at most 259.
#[cfg(windows)]
const ENGINE_MAX_PATH: usize = 260;

/// The layer root as the ENGINE would name it: an ordinary DOS path, never the
/// `\\?\` verbatim namespace.
///
/// Both callers hand this function a root they obtained differently -- the
/// recorder canonicalizes its mount roots (`fs::canonicalize`, which on Windows
/// returns a `\\?\` path), replay does not -- and the prefix is not cosmetic:
/// a verbatim path reaches the kernel with Win32 parsing SUPPRESSED. Measured on
/// the same fixture, same file: `common/ideas.txt.` resolves under the raw root
/// and is absent under the canonicalized one. Two callers, one function, two
/// answers. Stripping the prefix here is what makes the primitive a rule instead
/// of a reflection of how its caller happened to spell the root.
///
/// The engine's spelling is the ordinary one: `__PHYSFS_platformStat`
/// (`0x1424ff8e0`) transcodes the UTF-8 path it is handed straight into a buffer
/// of exactly `strlen * 2 + 2` bytes and calls `GetFileAttributesExW` on it, so
/// it adds no prefix of its own.
///
/// KNOWN LIMIT OF THAT READING, because it is load-bearing here: it proves the
/// stat function adds no prefix, NOT that its caller passed none. The directory
/// archiver's stat entry (`+0x68` in the archiver vtable), which joins the mount
/// root to the sanitized path, is still unread. Full working and the whole-image
/// `\\?\` scan:
/// `extractor/native/runs/lf_vfs_consolidate/engine_path_namespace_from_a729.md`.
/// Works in UTF-16 code units, not `to_string_lossy`, because a lossy
/// conversion would silently REWRITE a root containing unpaired surrogates --
/// turning "the root I was given" into "a root that looks like it" without
/// saying so.
///
/// Only the two verbatim forms that have an ordinary equivalent are converted.
/// `\\?\Volume{GUID}\...` has none: dropping its prefix would leave
/// `Volume{GUID}\...`, a RELATIVE path that resolves against the current
/// directory -- a silently different location. Anything else verbatim is
/// therefore refused rather than guessed at.
#[cfg(windows)]
fn engine_namespace_root(root: &Path) -> Result<PathBuf, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let wide: Vec<u16> = root.as_os_str().encode_wide().collect();
    // ASCII-case-insensitive over UTF-16 units: only `\\?\UNC\`'s letters are
    // ever compared this way, and Windows accepts either case there.
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

    // A verbatim root may exist BECAUSE it needs verbatim semantics: `\\?\C:\a.`
    // names a directory literally called `a.`, which the ordinary spelling
    // `C:\a.` cannot address -- Win32 strips the dot and would reach `a`. If both
    // exist, converting would silently select the wrong content, which is worse
    // than refusing. So a verbatim root is only converted when every component
    // survives ordinary parsing unchanged.
    let needs_verbatim = |from: usize| {
        let mut component = Vec::new();
        let mut offending = false;
        for unit in wide[from..].iter().copied().chain(std::iter::once(u16::from(b'\\'))) {
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
        // A drive-letter root is the one verbatim form with an exact ordinary
        // equivalent: `\\?\C:\x` names the same object as `C:\x`.
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
            for (path, _) in index.entries.values() {
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
    entries.sort_by(|left, right| {
        let left = left.file_name().to_string_lossy().replace('\\', "/");
        let right = right.file_name().to_string_lossy().replace('\\', "/");
        crate::authored_ui::engine_recursive_path_cmp(&left, &right)
    });
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
            let path = normalize_game_relative_path_preserve_case(&relative)?;
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

    /// A mounted layer may not declare one directory twice, and "twice" is a
    /// Windows filesystem question: `gfx/replace` and `gfx/replace.` are one
    /// directory on the playback host (measured: `gfx./u.asset` opened
    /// `gfx/u.asset`). The old check lowercased an already-lowercased canonical
    /// form, which was a no-op, so both mounted and the second declaration was
    /// silently free.
    #[test]
    fn a_layer_declaring_one_directory_under_two_windows_spellings_is_refused() {
        // NOT "gfx/replace " -- normalize_game_relative_path trims the whole
        // string, so that spelling is refused by the already-canonical check
        // and would exercise the wrong branch.
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
        // ... while two genuinely different directories still mount.
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

    /// `sanitize_engine_path` had no direct test, and both callers pre-validate,
    /// so removing its call from the primitive broke nothing. That made it
    /// decorative. This exercises it directly.
    ///
    /// kills: deleting the `.`/`..` arm, the `:`/`\` arm, or the empty-collapse
    /// each fail a row below.
    #[test]
    fn engine_path_grammar_matches_the_sanitizer() {
        assert_eq!(
            crate::sanitize_engine_path("gfx/unit.dds").unwrap(),
            "gfx/unit.dds"
        );
        // Empty components COLLAPSE, they do not fail.
        assert_eq!(
            crate::sanitize_engine_path("gfx//unit.dds").unwrap(),
            "gfx/unit.dds"
        );
        assert_eq!(
            crate::sanitize_engine_path("gfx/unit.dds/").unwrap(),
            "gfx/unit.dds"
        );
        // `.` and `..` FAIL, they are not dropped.
        for bad in ["gfx/./unit.dds", "gfx/../unit.dds", ".", ".."] {
            assert!(
                crate::sanitize_engine_path(bad).is_err(),
                "{bad:?} must be refused by the engine grammar"
            );
        }
        // `:` and `\` fail anywhere.
        for bad in [r"gfx\unit.dds", "c:/gfx/unit.dds", "gfx/a:b"] {
            assert!(
                crate::sanitize_engine_path(bad).is_err(),
                "{bad:?} must be refused by the engine grammar"
            );
        }
        // A path that names nothing.
        assert!(crate::sanitize_engine_path("//").is_err());
    }

    /// A verbatim root with no ordinary equivalent must REFUSE, not silently
    /// become a relative path.
    ///
    /// kills: restoring the blanket verbatim strip turns the volume-GUID root
    /// into `Volume{...}\game`, a path relative to the current directory, and
    /// this test's `is_err` fails.
    #[cfg(windows)]
    #[test]
    fn a_verbatim_root_without_an_ordinary_equivalent_is_refused() {
        // Drive-letter and UNC verbatim roots convert.
        assert_eq!(
            engine_namespace_root(Path::new(r"\\?\C:\game")).unwrap(),
            PathBuf::from(r"C:\game")
        );
        assert_eq!(
            engine_namespace_root(Path::new(r"\\?\UNC\host\share\game")).unwrap(),
            PathBuf::from(r"\\host\share\game")
        );
        // An ordinary root is untouched.
        assert_eq!(
            engine_namespace_root(Path::new(r"C:\game")).unwrap(),
            PathBuf::from(r"C:\game")
        );
        // A volume-GUID root has no ordinary spelling, so it refuses.
        let guid = Path::new(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\game");
        assert!(
            engine_namespace_root(guid).is_err(),
            "a volume-GUID root must refuse rather than become a relative path"
        );
        // A verbatim root that EXISTS because it needs verbatim semantics must
        // refuse too: `C:\mods\foo.` does not name the same object as
        // `\\?\C:\mods\foo.`, so converting could select different content.
        assert!(
            engine_namespace_root(Path::new(r"\\?\C:\mods\foo.")).is_err(),
            "a verbatim root with a dot-terminated component must refuse"
        );
        assert!(
            engine_namespace_root(Path::new(r"\\?\C:\mods\foo \bar")).is_err(),
            "a verbatim root with a space-terminated component must refuse"
        );
    }

    /// The primitive really does apply the engine grammar, rather than relying
    /// on its callers to have done so.
    ///
    /// Both A and B pre-validate, so every path the differential can reach is
    /// already clean by the time it arrives -- which meant deleting
    /// `sanitize_engine_path`'s call from `resolve_literal_layer_path` broke no
    /// test at all. Calling the primitive DIRECTLY is what pins it.
    ///
    /// kills: removing the `crate::sanitize_engine_path(logical_path)?` line
    /// makes the `.`-component row resolve the fixture file instead of refusing.
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

        // The premise: the clean spelling resolves, so a refusal below is about
        // the grammar and not about a missing fixture.
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
        // An empty component COLLAPSES rather than refusing, and still resolves.
        assert!(
            resolve_literal_layer_path(&root, "common//ideas.txt")
                .expect("collapsing spelling")
                .is_some(),
            "an empty component must collapse, not refuse"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
