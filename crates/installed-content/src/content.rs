//! Content coverage policy 1 and local resolution of machine-free layer ids.
//!
//! The vanilla hash includes the base covered paths plus the covered paths of
//! built-in virtual roots in their install-relative namespaces:
//! `integrated_dlc/<folder>/...` (including each descriptor) and
//! `tweakergui_assets/...` or `tweakergui_assets.zip/...`. Those roots are mounted
//! by the pinned engine independently of its selectable DLC roster.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Instant, UNIX_EPOCH},
};

use replay::identity::{COVERAGE_POLICY_VERSION, Dlc, Identity, Mod, Verification};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::parser::{self, Document};

pub mod visual;

const ROOTS: [&str; 7] = [
    "map",
    "common",
    "history",
    "localisation",
    "interface",
    "gfx/flags",
    "gfx/interface",
];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Local paths are kept here, never placed in a replay identity.
#[derive(Clone, Debug)]
pub struct GatherOptions {
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// Optional observed enabled DLC mount order, expressed as folder ids.
    /// Dependency-free vanilla uses the proven descriptor-path byte sort.
    pub dlc_order: Option<Vec<String>>,
}

/// Local locations used to resolve a replay, independently of launcher state.
#[derive(Clone, Debug)]
pub struct ResolveOptions {
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    pub cache_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ResolvedContent {
    pub layers: ResolvedLayers,
    pub stats: HashStats,
}

#[derive(Clone, Debug, Default)]
pub struct HashStats {
    pub files: u64,
    pub bytes: u64,
    pub files_read: u64,
    pub bytes_read: u64,
    pub cache_hits: u64,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug)]
pub enum LayerSource {
    Folder(PathBuf),
    Archive(PathBuf),
}

#[derive(Clone, Debug)]
pub struct ResolvedLayer {
    pub id: String,
    pub source: LayerSource,
    pub descriptor: Option<PathBuf>,
    pub replace_paths: Vec<String>,
    files: BTreeMap<String, Member>,
    hashes: BTreeMap<String, [u8; 32]>,
    archive_stamp: Option<Stamp>,
    descriptor_hash: Option<[u8; 32]>,
}

#[derive(Clone, Debug)]
struct Member {
    /// Archive member index, or a path relative to the folder source.
    archive_index: Option<usize>,
    size: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedLayers {
    visual: Vec<visual::VisualLayer>,
    pub layers: Vec<ResolvedLayer>,
    effective: BTreeMap<String, EffectiveFile>,
    /// Where uncovered paths resolved, so repeats skip the layer scan and its
    /// failed filesystem probes. Each entry records the layer roster it was
    /// resolved against, so layer push/pop (as in the fallback tests) misses
    /// instead of answering stale. Files are assumed stable during one load;
    /// a hit the filesystem no longer reproduces falls back to a fresh scan
    /// rather than failing. The covered path never consults this (it verifies
    /// per read).
    uncovered_memo: std::sync::Arc<std::sync::Mutex<UncoveredMemo>>,
}

/// Memoized uncovered resolutions with the roster they were resolved against.
#[derive(Debug, Default)]
struct UncoveredMemo {
    layers: Vec<String>,
    hits: BTreeMap<String, Option<UncoveredHit>>,
}

/// Where one uncovered path resolved.
#[derive(Clone, Debug)]
enum UncoveredHit {
    Folder(PathBuf),
    Archive { layer: usize, member: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EffectiveFile {
    layer: usize,
    path: String,
    archive: bool,
}

#[derive(Clone, Debug)]
pub struct GatheredContent {
    pub vanilla_hash: [u8; 32],
    pub dlc: Vec<Dlc>,
    pub mods: Vec<Mod>,
    pub language: String,
    pub layers: ResolvedLayers,
    pub stats: HashStats,
    /// A roster match alone must not hide an unproved mount order.
    pub order_note: Option<String>,
    /// Expected successful physical search roots, highest priority first.
    /// Includes built-in roots and the user directory; never serialized.
    pub expected_mounts: Vec<String>,
}

impl GatheredContent {
    pub fn identity(
        &self,
        game_version: String,
        exe_hash: [u8; 32],
        exe_size: u64,
        recorder_version: String,
        collector_hash: [u8; 32],
    ) -> io::Result<Identity> {
        let identity = Identity {
            game_version,
            exe_hash,
            exe_size,
            vanilla_hash: self.vanilla_hash,
            dlc: self.dlc.clone(),
            mods: self.mods.clone(),
            coverage_policy: COVERAGE_POLICY_VERSION,
            language: self.language.clone(),
            recorder_version,
            collector_hash,
            verification: Verification::Unchecked,
        };
        identity.validate()?;
        Ok(identity)
    }
}

/// A coverage path uses `/`, retains case, and cannot escape its layer.
pub fn normalize_relative(path: &str) -> io::Result<String> {
    let path = path.replace('\\', "/");
    if path.is_empty()
        || path.starts_with('/')
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.contains(':')
                || part.chars().any(char::is_control)
        })
    {
        return Err(invalid(format!("unsafe layer path: {path:?}")));
    }
    Ok(path)
}

fn below(path: &str, directory: &str) -> bool {
    path == directory
        || path
            .strip_prefix(directory)
            .is_some_and(|tail| tail.starts_with('/'))
}

/// Music, sound, video, loading screens, launcher, crash reporter and logs are
/// outside this closed set. A descriptor is added separately to the layer hash.
pub fn covered(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ROOTS.iter().any(|root| below(&lower, root))
        && !lower.split('/').any(|part| {
            matches!(
                part,
                "music"
                    | "sound"
                    | "video"
                    | "videos"
                    | "loadingscreens"
                    | "loading_screens"
                    | "launcher"
                    | "crash_reporter"
                    | "logs"
            )
        })
}

/// Policy 1: sorted (u32 LE path byte length, UTF-8 relative path, SHA-256)
/// tuples. Length framing makes the list unambiguous, including empty layers.
pub fn layer_hash(files: &BTreeMap<String, [u8; 32]>) -> io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    for (path, digest) in files {
        let length = u32::try_from(path.len()).map_err(|_| invalid("layer path too long"))?;
        hash.update(length.to_le_bytes());
        hash.update(path.as_bytes());
        hash.update(digest);
    }
    Ok(hash.finalize().into())
}

fn text_path(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| invalid("content path is not Unicode"))
}

fn read_document(path: &Path) -> io::Result<Document> {
    parser::parse(&fs::read(path)?).map_err(|error| invalid(format!("{}: {error}", path.display())))
}

fn scalar(document: &Document, key: &[u8]) -> io::Result<Option<String>> {
    document
        .get(key)
        .map(|value| {
            value.scalar_str().map(str::to_owned).ok_or_else(|| {
                invalid(format!(
                    "descriptor {} is not a UTF-8 scalar",
                    String::from_utf8_lossy(key)
                ))
            })
        })
        .transpose()
}

fn required(document: &Document, key: &[u8]) -> io::Result<String> {
    scalar(document, key)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("descriptor lacks {}", String::from_utf8_lossy(key))))
}

fn source_path(base: &Path, value: &str, absolute_allowed: bool) -> io::Result<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        if !absolute_allowed {
            return Err(invalid("DLC source must be relative to the game folder"));
        }
        return fs::canonicalize(path);
    }
    let relative = normalize_relative(value)?;
    fs::canonicalize(base.join(relative))
}

fn descriptor_layer(
    id: String,
    path: PathBuf,
    base: &Path,
    is_mod: bool,
) -> io::Result<(ResolvedLayer, Document)> {
    let descriptor_bytes = fs::read(&path)?;
    let document = parser::parse(&descriptor_bytes)?;
    let descriptor_hash = Sha256::digest(&descriptor_bytes).into();
    let folder = scalar(&document, b"path")?;
    let archive = scalar(&document, b"archive")?;
    let source = match (folder, archive) {
        (Some(folder), None) => LayerSource::Folder(source_path(
            base,
            folder.trim_end_matches(['/', '\\']),
            is_mod,
        )?),
        (None, Some(archive)) => LayerSource::Archive(source_path(base, &archive, is_mod)?),
        _ => {
            return Err(invalid(format!(
                "{} must declare exactly one of path or archive",
                path.display()
            )));
        }
    };
    let replace_paths = document
        .get_all(b"replace_path")
        .map(|value| {
            value
                .scalar_str()
                .ok_or_else(|| invalid("replace_path is not a UTF-8 scalar"))
                .and_then(|path| normalize_relative(path.trim_end_matches(['/', '\\'])))
        })
        .collect::<io::Result<Vec<_>>>()?;
    if !is_mod && !replace_paths.is_empty() {
        return Err(invalid(
            "DLC replace_path is not supported by this coverage policy",
        ));
    }
    Ok((
        ResolvedLayer {
            id,
            source,
            descriptor: Some(path),
            replace_paths,
            files: BTreeMap::new(),
            hashes: BTreeMap::new(),
            archive_stamp: None,
            descriptor_hash: Some(descriptor_hash),
        },
        document,
    ))
}

fn directory_entries(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn collect_folder(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, Member>,
) -> io::Result<()> {
    for entry in directory_entries(path)? {
        let full_path = entry.path();
        let relative = normalize_relative(text_path(
            full_path
                .strip_prefix(root)
                .map_err(|_| invalid("layer path escaped root"))?,
        )?)?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(invalid(format!(
                "content layer contains a symbolic link: {}",
                full_path.display()
            )));
        }
        if kind.is_dir() {
            collect_folder(root, &full_path, files)?;
        } else if kind.is_file() && covered(&relative) {
            files.insert(
                relative,
                Member {
                    archive_index: None,
                    size: entry.metadata()?.len(),
                },
            );
        }
    }
    Ok(())
}

impl ResolvedLayer {
    fn inventory(&mut self) -> io::Result<()> {
        match &self.source {
            LayerSource::Folder(root) => {
                for directory in ROOTS {
                    let path = root.join(directory);
                    if path.try_exists()? {
                        collect_folder(root, &path, &mut self.files)?;
                    }
                }
            }
            LayerSource::Archive(path) => {
                let stamp = Stamp::read(path)?;
                let mut archive = zip::ZipArchive::new(File::open(path)?)?;
                for index in 0..archive.len() {
                    let member = archive.by_index(index)?;
                    let name = member.name().trim_end_matches('/');
                    if name.is_empty() {
                        continue;
                    }
                    let relative = normalize_relative(name)?;
                    if member.is_dir() {
                        continue;
                    }
                    if member
                        .unix_mode()
                        .is_some_and(|mode| mode & 0o170000 == 0o120000)
                    {
                        return Err(invalid(format!(
                            "archive contains a symbolic link: {relative}"
                        )));
                    }
                    if covered(&relative)
                        && self
                            .files
                            .insert(
                                relative.clone(),
                                Member {
                                    archive_index: Some(index),
                                    size: member.size(),
                                },
                            )
                            .is_some()
                    {
                        return Err(invalid(format!(
                            "archive contains duplicate path: {relative}"
                        )));
                    }
                }
                if Stamp::read(path)? != stamp {
                    return Err(invalid("archive changed during inventory"));
                }
                self.archive_stamp = Some(stamp);
            }
        }
        let mut folded = BTreeSet::new();
        for path in self.files.keys() {
            if !folded.insert(path.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "layer contains case-ambiguous path: {path}"
                )));
            }
        }
        Ok(())
    }

    fn hash(&mut self, cache: &mut HashCache, stats: &mut HashStats) -> io::Result<[u8; 32]> {
        let mut hashes = BTreeMap::new();
        match &self.source {
            LayerSource::Folder(root) => {
                for relative in self.files.keys() {
                    hashes.insert(relative.clone(), cache.file(&root.join(relative), stats)?);
                }
            }
            LayerSource::Archive(path) => {
                let stamp = Stamp::read(path)?;
                if self.archive_stamp.as_ref() != Some(&stamp) {
                    return Err(invalid("archive changed since inventory"));
                }
                let mut archive = zip::ZipArchive::new(File::open(path)?)?;
                for (relative, member) in &self.files {
                    let key = format!("{}\0{relative}", text_path(path)?);
                    stats.files += 1;
                    stats.bytes += member.size;
                    let digest = if let Some(digest) = cache.get(&key, &stamp) {
                        stats.cache_hits += 1;
                        digest
                    } else {
                        let mut input = archive.by_index(member.archive_index.unwrap())?;
                        if normalize_relative(input.name())? != *relative
                            || input.size() != member.size
                        {
                            return Err(invalid("archive member changed since inventory"));
                        }
                        let (digest, size) = digest_reader(&mut input)?;
                        if size != member.size {
                            return Err(invalid("archive member size changed"));
                        }
                        stats.files_read += 1;
                        stats.bytes_read += size;
                        cache.insert(key, stamp.clone(), digest);
                        digest
                    };
                    hashes.insert(relative.clone(), digest);
                }
                if Stamp::read(path)? != stamp {
                    return Err(invalid("archive changed while hashing"));
                }
            }
        }
        self.hashes = hashes.clone();
        if let Some(descriptor) = &self.descriptor {
            let name = text_path(Path::new(
                descriptor
                    .file_name()
                    .ok_or_else(|| invalid("descriptor has no name"))?,
            ))?;
            let key = normalize_relative(name)?;
            let digest = cache.file(descriptor, stats)?;
            if self.descriptor_hash != Some(digest) {
                return Err(invalid("descriptor changed between parsing and hashing"));
            }
            if hashes.insert(key, digest).is_some() {
                return Err(invalid(
                    "descriptor path collides with a covered layer file",
                ));
            }
        }
        layer_hash(&hashes)
    }

    fn read(&self, relative: &str) -> io::Result<Vec<u8>> {
        let member = self
            .files
            .get(relative)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, relative))?;
        let bytes = match &self.source {
            LayerSource::Folder(root) => fs::read(root.join(relative)),
            LayerSource::Archive(path) => {
                let mut archive = zip::ZipArchive::new(File::open(path)?)?;
                let mut entry = archive.by_index(member.archive_index.unwrap())?;
                if normalize_relative(entry.name())? != relative || entry.size() != member.size {
                    return Err(invalid("archive changed since content resolution"));
                }
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                Ok(bytes)
            }
        }?;
        let hash: [u8; 32] = Sha256::digest(&bytes).into();
        if self.hashes.get(relative) != Some(&hash) {
            return Err(invalid(format!(
                "content file changed since verification: {relative}"
            )));
        }
        Ok(bytes)
    }
}

impl ResolvedLayers {
    fn finish(&mut self) -> io::Result<()> {
        self.effective.clear();
        for (index, layer) in self.layers.iter().enumerate() {
            apply_overlay(
                &mut self.effective,
                index,
                matches!(layer.source, LayerSource::Archive(_)),
                layer.files.keys(),
                &layer.replace_paths,
            )?;
        }
        Ok(())
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.effective.values().map(|file| file.path.as_str())
    }

    /// Every distinct covered path under `prefix` that a visible layer lists,
    /// with its own case, as a PhysFS enumeration of the merged tree lists it:
    /// names that differ only in case stay separate entries, and each reads
    /// through [`ResolvedLayers::read`]. A `replace_path` hides the layers
    /// below it.
    pub fn listed_paths(&self, prefix: &str) -> BTreeSet<String> {
        let prefix = prefix.to_ascii_lowercase();
        let mut listed = BTreeSet::new();
        let mut hidden: Vec<String> = Vec::new();
        for layer in self.layers.iter().rev() {
            for path in layer.files.keys() {
                let lower = path.to_ascii_lowercase();
                if lower.starts_with(&prefix) && !hidden.iter().any(|root| below(&lower, root)) {
                    listed.insert(path.clone());
                }
            }
            hidden.extend(
                layer
                    .replace_paths
                    .iter()
                    .map(|path| path.to_ascii_lowercase()),
            );
        }
        listed
    }

    /// Load-order position of the layer an effective covered path comes from.
    pub(crate) fn layer_index(&self, relative: &str) -> Option<usize> {
        self.effective
            .get(&relative.to_ascii_lowercase())
            .map(|file| file.layer)
    }

    pub fn read(&self, relative: &str) -> io::Result<Vec<u8>> {
        let relative = normalize_relative(relative)?;
        if let Some(bytes) = visual::read(&self.visual, &relative)? {
            return Ok(bytes);
        }
        if let Some(file) = self.effective.get(&relative.to_ascii_lowercase())
            && (!file.archive || file.path == relative)
        {
            return self.layers[file.layer].read(&file.path);
        }
        // Native Windows folder lookup ignores case; ZIP lookup is exact.
        // Scan the layers so a case-missing archive can fall through to a folder.
        for layer in self.layers.iter().rev() {
            if layer.files.contains_key(&relative) {
                return layer.read(&relative);
            }
            if matches!(layer.source, LayerSource::Folder(_))
                && let Some(path) = layer
                    .files
                    .keys()
                    .find(|path| path.eq_ignore_ascii_case(&relative))
            {
                return layer.read(path);
            }
            if layer
                .replace_paths
                .iter()
                .any(|path| below(&relative.to_ascii_lowercase(), &path.to_ascii_lowercase()))
            {
                break;
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, relative))
    }

    /// Reads a path outside the coverage policy, such as `gfx/fonts`,
    /// `gfx/world` or `gfx/mapitems`, from the resolved layers.
    ///
    /// The returned bytes are **not** hash-verified: the layer hash does not
    /// cover them, so a changed file cannot be detected here and the viewer
    /// must state that limitation. A covered path is passed straight to
    /// [`ResolvedLayers::read`], which does verify it.
    pub fn read_uncovered(&self, relative: &str) -> io::Result<Vec<u8>> {
        let relative = normalize_relative(relative)?;
        if let Some(bytes) = visual::read(&self.visual, &relative)? {
            return Ok(bytes);
        }
        if covered(&relative) {
            return self.read(&relative);
        }
        let roster: Vec<String> = self.layers.iter().map(|layer| layer.id.clone()).collect();
        if let Some(memo) = self
            .uncovered_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&roster, &relative)
        {
            match memo {
                // A stale hit falls back to a fresh scan below, which
                // refreshes the memo; only a hit the scan reproduces silences
                // it.
                Some(hit) => match self.read_hit(&hit) {
                    Ok(Some(bytes)) => return Ok(bytes),
                    Ok(None) => {}
                    Err(error) => return Err(error),
                },
                None => return Err(io::Error::new(io::ErrorKind::NotFound, relative)),
            }
        }
        let found = self
            .scan_uncovered(&relative)?
            .map(|(_, hit, bytes)| (hit, bytes));
        self.uncovered_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                roster,
                relative.clone(),
                found.as_ref().map(|(hit, _)| hit.clone()),
            );
        match found {
            Some((_, bytes)) => Ok(bytes),
            None => Err(io::Error::new(io::ErrorKind::NotFound, relative)),
        }
    }

    /// The layer scan behind [`ResolvedLayers::read_uncovered`], returning the
    /// hit alongside its bytes. Semantics match the previous inline scan
    /// exactly: folder misses fall through with a case-insensitive retry,
    /// archive lookup is exact, hard errors abort, and `replace_path` stops
    /// the scan.
    fn scan_uncovered(&self, relative: &str) -> io::Result<Option<(usize, UncoveredHit, Vec<u8>)>> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            match &layer.source {
                LayerSource::Folder(root) => match fs::read(root.join(relative)) {
                    Ok(bytes) => {
                        return Ok(Some((
                            index,
                            UncoveredHit::Folder(root.join(relative)),
                            bytes,
                        )));
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        // Native Windows folder lookup ignores case; keep that
                        // behaviour explicit so a layer resolves the same way
                        // whatever case the calling parser carries.
                        if let Some(path) = folded_path(root, relative)? {
                            let bytes = fs::read(&path)?;
                            return Ok(Some((index, UncoveredHit::Folder(path), bytes)));
                        }
                    }
                    Err(error) => return Err(error),
                },
                LayerSource::Archive(path) => {
                    // ZIP lookup is exact, as it is for covered paths.
                    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
                    if let Ok(mut entry) = archive.by_name(relative) {
                        let mut bytes = Vec::new();
                        entry.read_to_end(&mut bytes)?;
                        return Ok(Some((
                            index,
                            UncoveredHit::Archive {
                                layer: index,
                                member: relative.to_owned(),
                            },
                            bytes,
                        )));
                    }
                }
            }
            if layer
                .replace_paths
                .iter()
                .any(|path| below(&lower, &path.to_ascii_lowercase()))
            {
                break;
            }
        }
        Ok(None)
    }

    /// Whether an uncovered path resolves from the base game layer, with no
    /// DLC, mod, `replace_path` or cosmetic overlay in its place.
    pub fn uncovered_from_base(&self, relative: &str) -> io::Result<bool> {
        let relative = normalize_relative(relative)?;
        if visual::read(&self.visual, &relative)?.is_some() {
            return Ok(false);
        }
        Ok(matches!(self.scan_uncovered(&relative)?, Some((0, _, _))))
    }

    /// Reads a memoized hit. `Ok(None)` means the hit is stale and the caller
    /// rescans; `Err` is a hard error, which aborts exactly like the scan.
    fn read_hit(&self, hit: &UncoveredHit) -> io::Result<Option<Vec<u8>>> {
        match hit {
            UncoveredHit::Folder(path) => match fs::read(path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            },
            UncoveredHit::Archive { layer, member } => {
                let path = match self.layers.get(*layer).map(|layer| &layer.source) {
                    Some(LayerSource::Archive(path)) => path,
                    // The layer at this index is no longer an archive: stale.
                    _ => return Ok(None),
                };
                let mut archive = zip::ZipArchive::new(File::open(path)?)?;
                // The serial scan continues past an archive miss, so any
                // lookup failure rescans rather than failing here.
                match archive.by_name(member) {
                    Ok(mut entry) => {
                        let mut bytes = Vec::new();
                        entry.read_to_end(&mut bytes)?;
                        Ok(Some(bytes))
                    }
                    Err(_) => Ok(None),
                }
            }
        }
    }
}

impl UncoveredMemo {
    fn get(&self, roster: &[String], relative: &str) -> Option<Option<UncoveredHit>> {
        (self.layers == roster)
            .then(|| self.hits.get(relative).cloned())
            .flatten()
    }

    fn insert(&mut self, roster: Vec<String>, relative: String, hit: Option<UncoveredHit>) {
        if self.layers != roster {
            self.layers = roster;
            self.hits.clear();
        }
        self.hits.insert(relative, hit);
    }
}

/// Resolves each path component case-insensitively inside one folder layer.
fn folded_path(root: &Path, relative: &str) -> io::Result<Option<PathBuf>> {
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut found = None;
        for entry in entries {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(part)
            {
                found = Some(entry.file_name());
                break;
            }
        }
        match found {
            Some(name) => path.push(name),
            None => return Ok(None),
        }
    }
    Ok(Some(path))
}

fn apply_overlay<'a>(
    effective: &mut BTreeMap<String, EffectiveFile>,
    index: usize,
    archive: bool,
    paths: impl Iterator<Item = &'a String>,
    replace_paths: &[String],
) -> io::Result<()> {
    for replaced in replace_paths {
        effective.retain(|path, _| !below(path, &replaced.to_ascii_lowercase()));
    }
    for path in paths {
        if effective
            .get(&path.to_ascii_lowercase())
            .is_some_and(|old| old.path != *path && (old.archive || archive))
        {
            return Err(invalid(format!(
                "folder and archive layers have case-dependent path precedence: {path}"
            )));
        }
        effective.insert(
            path.to_ascii_lowercase(),
            EffectiveFile {
                layer: index,
                path: path.clone(),
                archive,
            },
        );
    }
    Ok(())
}

/// Checks an extra physical search root before the recorder can disregard it.
/// Unknown archive formats are errors, not empty covered sets.
pub fn coverage_file_count(source: LayerSource) -> io::Result<usize> {
    let mut layer = ResolvedLayer {
        id: "physical search root".into(),
        source,
        descriptor: None,
        replace_paths: Vec::new(),
        files: BTreeMap::new(),
        hashes: BTreeMap::new(),
        archive_stamp: None,
        descriptor_hash: None,
    };
    layer.inventory()?;
    Ok(layer.files.len())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified_seconds: u64,
    modified_nanos: u32,
}

impl Stamp {
    fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("content timestamp predates Unix epoch"))?;
        Ok(Self {
            size: metadata.len(),
            modified_seconds: modified.as_secs(),
            modified_nanos: modified.subsec_nanos(),
        })
    }
}

#[derive(Deserialize, Serialize)]
struct CachedHash {
    stamp: Stamp,
    hash: [u8; 32],
}

#[derive(Default, Deserialize, Serialize)]
struct HashCache {
    entries: BTreeMap<String, CachedHash>,
}

impl HashCache {
    fn get(&self, key: &str, stamp: &Stamp) -> Option<[u8; 32]> {
        self.entries
            .get(key)
            .filter(|entry| &entry.stamp == stamp)
            .map(|entry| entry.hash)
    }

    fn insert(&mut self, key: String, stamp: Stamp, hash: [u8; 32]) {
        self.entries.insert(key, CachedHash { stamp, hash });
    }

    fn file(&mut self, path: &Path, stats: &mut HashStats) -> io::Result<[u8; 32]> {
        let stamp = Stamp::read(path)?;
        let key = text_path(path)?;
        stats.files += 1;
        stats.bytes += stamp.size;
        if let Some(hash) = self.get(key, &stamp) {
            stats.cache_hits += 1;
            return Ok(hash);
        }
        let (hash, size) = digest_reader(&mut File::open(path)?)?;
        if size != stamp.size || Stamp::read(path)? != stamp {
            return Err(invalid(format!(
                "content file changed while hashing: {}",
                path.display()
            )));
        }
        stats.files_read += 1;
        stats.bytes_read += size;
        self.insert(key.to_owned(), stamp, hash);
        Ok(hash)
    }
}

fn digest_reader(input: &mut impl Read) -> io::Result<([u8; 32], u64)> {
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0; 128 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        size += count as u64;
    }
    Ok((hash.finalize().into(), size))
}

#[derive(Deserialize)]
struct LauncherState {
    enabled_mods: Vec<String>,
    disabled_dlcs: Vec<String>,
}

fn find_descriptors(path: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in directory_entries(path)? {
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(invalid("DLC directory contains a symbolic link"));
        }
        if kind.is_dir() {
            find_descriptors(&entry.path(), out)?;
        } else if kind.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "dlc")
        {
            out.push(entry.path());
        }
    }
    Ok(())
}

fn sort_descriptors(descriptors: &mut [PathBuf]) -> io::Result<()> {
    for path in descriptors.iter() {
        text_path(path)?;
    }
    descriptors.sort_by(|left, right| {
        left.to_str()
            .unwrap()
            .as_bytes()
            .cmp(right.to_str().unwrap().as_bytes())
    });
    Ok(())
}

fn mod_id(document: &Document, descriptor: &Path) -> io::Result<String> {
    Ok(scalar(document, b"remote_file_id")?
        .filter(|id| !id.is_empty())
        .unwrap_or(
            descriptor
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid("mod descriptor has no Unicode file name"))?
                .to_owned(),
        ))
}

fn named_error(kind: &str, id: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{kind} {id}: {error}"))
}

fn select_recorded_paths<'a>(
    kind: &str,
    ids: impl Iterator<Item = &'a str>,
    candidates: Vec<(String, PathBuf)>,
) -> io::Result<Vec<PathBuf>> {
    let mut available: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (id, path) in candidates {
        available.entry(id).or_default().push(path);
    }
    ids.map(|id| match available.remove(id).as_deref() {
        Some([path]) => Ok(path.clone()),
        Some(_) => Err(invalid(format!(
            "{kind} {id}: multiple installed descriptors match the recorded id"
        ))),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{kind} {id}: recorded layer is missing"),
        )),
    })
    .collect()
}

fn recorded_dlc_paths(game_dir: &Path, recorded: &[Dlc]) -> io::Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    for dlc in recorded {
        let directory = game_dir.join("dlc").join(&dlc.id);
        if !directory.try_exists()? {
            continue;
        }
        for entry in
            directory_entries(&directory).map_err(|error| named_error("DLC", &dlc.id, error))?
        {
            let path = entry.path();
            if entry.file_type()?.is_file()
                && path.extension().is_some_and(|extension| extension == "dlc")
            {
                candidates.push((dlc.id.clone(), path));
            }
        }
    }
    select_recorded_paths(
        "DLC",
        recorded.iter().map(|dlc| dlc.id.as_str()),
        candidates,
    )
}

fn recorded_mod_paths(user_dir: &Path, recorded: &[Mod]) -> io::Result<Vec<String>> {
    if recorded.is_empty() {
        return Ok(Vec::new());
    }
    let directory = user_dir.join("mod");
    let mut candidates = Vec::new();
    if directory.try_exists()? {
        for entry in directory_entries(&directory)? {
            let path = entry.path();
            if !entry.file_type()?.is_file()
                || path.extension().is_none_or(|extension| extension != "mod")
            {
                continue;
            }
            // A broken unrelated descriptor must not prevent another replay
            // from resolving. A selected local descriptor is parsed below.
            let id = match read_document(&path).and_then(|document| mod_id(&document, &path)) {
                Ok(id) => id,
                Err(_) => text_path(Path::new(entry.file_name().as_os_str()))?.to_owned(),
            };
            candidates.push((id, path));
        }
    }
    select_recorded_paths(
        "Mod",
        recorded.iter().map(|layer| layer.id.as_str()),
        candidates,
    )?
    .into_iter()
    .map(|path| {
        normalize_relative(text_path(
            path.strip_prefix(user_dir)
                .map_err(|_| invalid("mod descriptor escaped user folder"))?,
        )?)
    })
    .collect()
}

fn check_hash(kind: &str, id: &str, actual: [u8; 32], expected: [u8; 32]) -> io::Result<()> {
    if actual == expected {
        return Ok(());
    }
    fn hex(hash: [u8; 32]) -> String {
        hash.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    Err(invalid(format!(
        "{kind} {id}: content hash mismatch (expected {}, found {})",
        hex(expected),
        hex(actual)
    )))
}

fn builtin_layer(
    id: String,
    source: LayerSource,
    descriptor: Option<PathBuf>,
) -> io::Result<ResolvedLayer> {
    let descriptor_hash = descriptor
        .as_ref()
        .map(|path| fs::read(path).map(|bytes| Sha256::digest(bytes).into()))
        .transpose()?;
    Ok(ResolvedLayer {
        id,
        source,
        descriptor,
        replace_paths: Vec::new(),
        files: BTreeMap::new(),
        hashes: BTreeMap::new(),
        archive_stamp: None,
        descriptor_hash,
    })
}

fn append_builtin(
    mut layer: ResolvedLayer,
    prefix: &str,
    vanilla_files: &mut BTreeMap<String, [u8; 32]>,
    layers: &mut ResolvedLayers,
    cache: &mut HashCache,
    stats: &mut HashStats,
) -> io::Result<()> {
    layer.inventory()?;
    layer.hash(cache, stats)?;
    // Built-in virtual roots belong to the vanilla install identity. Keep their
    // physical install-relative namespace, even when virtual file names overlap.
    for (path, digest) in &layer.hashes {
        vanilla_files.insert(format!("{prefix}/{path}"), *digest);
    }
    if let Some(descriptor) = &layer.descriptor {
        let name = descriptor
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("built-in descriptor has no Unicode name"))?;
        vanilla_files.insert(format!("{prefix}/{name}"), layer.descriptor_hash.unwrap());
    }
    layers.layers.push(layer);
    Ok(())
}

/// Hashes only covered bytes. Call on a background thread before injection.
pub fn gather(options: &GatherOptions) -> io::Result<GatheredContent> {
    gather_selected(options, None)
}

/// Resolves exactly the replay's recorded ids and order and verifies every
/// content hash. Current launcher selections, language and user-directory
/// overrides are not loaded. This verifies local bytes; callers still enforce
/// the recording's `Verification` state before claiming verified content.
pub fn resolve(identity: &Identity, options: &ResolveOptions) -> io::Result<ResolvedContent> {
    identity.validate()?;
    if identity.coverage_policy != COVERAGE_POLICY_VERSION {
        return Err(invalid(format!(
            "unsupported recorded content coverage policy {} (supported {})",
            identity.coverage_policy, COVERAGE_POLICY_VERSION
        )));
    }
    let content = gather_selected(
        &GatherOptions {
            game_dir: options.game_dir.clone(),
            user_dir: options.user_dir.clone(),
            cache_dir: options.cache_dir.clone(),
            dlc_order: Some(identity.dlc.iter().map(|dlc| dlc.id.clone()).collect()),
        },
        Some(identity),
    )?;
    Ok(ResolvedContent {
        layers: content.layers,
        stats: content.stats,
    })
}

fn gather_selected(
    options: &GatherOptions,
    recorded: Option<&Identity>,
) -> io::Result<GatheredContent> {
    let started = Instant::now();
    let game_dir = fs::canonicalize(&options.game_dir)?;
    let user_dir = fs::canonicalize(&options.user_dir)?;
    let (launcher, language) = if let Some(identity) = recorded {
        (
            LauncherState {
                enabled_mods: recorded_mod_paths(&user_dir, &identity.mods)?,
                disabled_dlcs: Vec::new(),
            },
            identity.language.clone(),
        )
    } else {
        (
            serde_json::from_slice(&fs::read(user_dir.join("dlc_load.json"))?)?,
            required(&read_document(&user_dir.join("settings.txt"))?, b"language")?,
        )
    };
    fs::create_dir_all(&options.cache_dir)?;
    let cache_path = options.cache_dir.join("content-hashes-v1.json");
    // An incomplete cache is disposable; the installed bytes remain the authority.
    let mut cache: HashCache = fs::read(&cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let result = (|| {
        let mut stats = HashStats::default();
        if let Some(identity) = recorded {
            let executable = game_dir.join("hoi4.exe");
            let size = fs::metadata(&executable)
                .map_err(|error| named_error("Game", "hoi4.exe", error))?
                .len();
            if size != identity.exe_size {
                return Err(invalid(format!(
                    "Game hoi4.exe: byte size mismatch (expected {}, found {size})",
                    identity.exe_size
                )));
            }
            check_hash(
                "Game",
                "hoi4.exe",
                cache.file(&executable, &mut stats)?,
                identity.exe_hash,
            )?;
        }
        let mut vanilla = ResolvedLayer {
            id: "vanilla".into(),
            source: LayerSource::Folder(game_dir.clone()),
            descriptor: None,
            replace_paths: Vec::new(),
            files: BTreeMap::new(),
            hashes: BTreeMap::new(),
            archive_stamp: None,
            descriptor_hash: None,
        };
        vanilla.inventory()?;
        vanilla.hash(&mut cache, &mut stats)?;
        let mut vanilla_files = vanilla.hashes.clone();
        let mut layers = ResolvedLayers {
            layers: vec![vanilla],
            effective: BTreeMap::new(),
            uncovered_memo: Default::default(),
            visual: Vec::new(),
        };
        let mut order_notes = Vec::new();
        if recorded.is_none() {
            let mut user_layer = builtin_layer(
                "user directory".into(),
                LayerSource::Folder(user_dir.clone()),
                None,
            )?;
            user_layer.inventory()?;
            if !user_layer.files.is_empty() {
                order_notes.push(format!("The game's mounted user directory contains {} covered files outside the declared content layers", user_layer.files.len()));
                user_layer.hash(&mut cache, &mut stats)?;
            }
            layers.layers.push(user_layer);
        }
        // Startup 140128700 mounts integrated descriptor parent folders before
        // selectable DLC. Their old descriptor `path=dlc/...` is not used here.
        let integrated_dir = game_dir.join("integrated_dlc");
        if integrated_dir.try_exists()? {
            let mut integrated = Vec::new();
            find_descriptors(&integrated_dir, &mut integrated)?;
            sort_descriptors(&mut integrated)?;
            for descriptor in integrated {
                let parent = descriptor
                    .parent()
                    .ok_or_else(|| invalid("integrated descriptor has no parent"))?
                    .to_owned();
                let prefix = normalize_relative(text_path(
                    parent
                        .strip_prefix(&game_dir)
                        .map_err(|_| invalid("integrated content escaped game folder"))?,
                )?)?;
                let layer = builtin_layer(
                    prefix.clone(),
                    LayerSource::Folder(parent),
                    Some(descriptor),
                )?;
                append_builtin(
                    layer,
                    &prefix,
                    &mut vanilla_files,
                    &mut layers,
                    &mut cache,
                    &mut stats,
                )?;
            }
        }
        let mut descriptors = if let Some(identity) = recorded {
            recorded_dlc_paths(&game_dir, &identity.dlc)?
        } else {
            let mut paths = Vec::new();
            find_descriptors(&game_dir.join("dlc"), &mut paths)?;
            paths
        };
        // The pinned engine compares UTF-8 descriptor bytes, not Windows UTF-16
        // collation. Reject non-Unicode paths instead of lossy conversion.
        sort_descriptors(&mut descriptors)?;
        let disabled = launcher
            .disabled_dlcs
            .iter()
            .map(|path| normalize_relative(path))
            .collect::<io::Result<BTreeSet<_>>>()?;
        let mut dlc_layers = BTreeMap::new();
        let mut discovered_order = Vec::new();
        let mut has_dependencies = false;
        for path in descriptors {
            let relative = normalize_relative(text_path(
                path.strip_prefix(&game_dir)
                    .map_err(|_| invalid("DLC descriptor escaped game folder"))?,
            )?)?;
            if disabled.contains(&relative) {
                continue;
            }
            let id = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid("DLC descriptor lacks a folder id"))?
                .to_owned();
            let (mut layer, document) = descriptor_layer(id.clone(), path, &game_dir, false)
                .map_err(|error| named_error("DLC", &id, error))?;
            has_dependencies |= document
                .get(b"dependencies")
                .is_some_and(|value| value.items().is_none_or(|items| !items.is_empty()));
            layer
                .inventory()
                .map_err(|error| named_error("DLC", &id, error))?;
            let dlc = Dlc {
                id: id.clone(),
                hash: layer
                    .hash(&mut cache, &mut stats)
                    .map_err(|error| named_error("DLC", &id, error))?,
                declared_checksum: scalar(&document, b"checksum")?,
            };
            if let Some(identity) = recorded {
                let expected = identity
                    .dlc
                    .iter()
                    .find(|dlc| dlc.id == id)
                    .ok_or_else(|| {
                        invalid(format!("DLC {id}: descriptor id changed during resolution"))
                    })?;
                check_hash("DLC", &id, dlc.hash, expected.hash)?;
            }
            discovered_order.push(id.clone());
            if dlc_layers.insert(id, (layer, dlc)).is_some() {
                return Err(invalid("multiple DLC descriptors have the same folder id"));
            }
        }
        let order = options.dlc_order.as_ref().unwrap_or(&discovered_order);
        let mut dlc = Vec::new();
        for id in order {
            let (layer, identity) = dlc_layers.remove(id).ok_or_else(|| {
                invalid(format!(
                    "DLC mount order names a missing, disabled or repeated layer: {id}"
                ))
            })?;
            layers.layers.push(layer);
            dlc.push(identity);
        }
        if !dlc_layers.is_empty() {
            return Err(invalid("DLC mount order omits enabled installed layers"));
        }
        // Pinned engine 1420614b0 sorts by dependency weight, then comparator
        // 14205ef20 compares descriptor path bytes. All installed vanilla DLC have
        // weight zero and no dependencies; PHYSFS mount prepends each sorted layer.
        if has_dependencies && options.dlc_order.is_none() {
            order_notes.push(
                "DLC dependencies can change mount order; observed mount order is required"
                    .to_owned(),
            );
        }
        if recorded.is_none() && !launcher.enabled_mods.is_empty() {
            order_notes.push(
            "Mod dependency ordering and duplicate display-name selection have not been measured"
                .to_owned(),
        );
        }
        let order_note = (!order_notes.is_empty()).then(|| order_notes.join("; "));
        let mut mods = Vec::new();
        let mut mod_ids = BTreeSet::new();
        for (index, descriptor) in launcher.enabled_mods.into_iter().enumerate() {
            let descriptor = normalize_relative(&descriptor)?;
            let path = user_dir.join(&descriptor);
            let requested_id = recorded
                .map(|identity| identity.mods[index].id.as_str())
                .unwrap_or(&descriptor);
            let (mut layer, document) = descriptor_layer(String::new(), path, &user_dir, true)
                .map_err(|error| named_error("Mod", requested_id, error))?;
            let id = mod_id(&document, Path::new(&descriptor))?;
            if !mod_ids.insert(id.clone()) {
                return Err(invalid(format!("duplicate enabled mod id: {id}")));
            }
            let display_name = required(&document, b"name")?;
            let declared_version = scalar(&document, b"version")?.unwrap_or_default();
            layer.id = id.clone();
            layer
                .inventory()
                .map_err(|error| named_error("Mod", &id, error))?;
            let hash = layer
                .hash(&mut cache, &mut stats)
                .map_err(|error| named_error("Mod", &id, error))?;
            if let Some(identity) = recorded {
                let expected = &identity.mods[index];
                if id != expected.id {
                    return Err(invalid(format!(
                        "Mod {}: descriptor id changed during resolution to {id}",
                        expected.id
                    )));
                }
                check_hash("Mod", &id, hash, expected.hash)?;
            }
            layers.layers.push(layer);
            mods.push(Mod {
                id,
                display_name,
                declared_version,
                hash,
            });
        }
        // Engine 142069260 mounts the optional tweaker assets last, preferring
        // the folder and falling back to its ZIP. It includes covered GUI files.
        let tweaker_folder = game_dir.join("tweakergui_assets");
        let tweaker_archive = game_dir.join("tweakergui_assets.zip");
        let tweaker = if tweaker_folder.is_dir() {
            Some(("tweakergui_assets", LayerSource::Folder(tweaker_folder)))
        } else if tweaker_archive.is_file() {
            Some((
                "tweakergui_assets.zip",
                LayerSource::Archive(tweaker_archive),
            ))
        } else {
            None
        };
        if let Some((prefix, source)) = tweaker {
            let layer = builtin_layer(prefix.into(), source, None)?;
            append_builtin(
                layer,
                prefix,
                &mut vanilla_files,
                &mut layers,
                &mut cache,
                &mut stats,
            )?;
        }
        let vanilla_hash = layer_hash(&vanilla_files)?;
        if let Some(identity) = recorded {
            check_hash("Game", "vanilla", vanilla_hash, identity.vanilla_hash)?;
        }
        let expected_mounts = layers
            .layers
            .iter()
            .rev()
            .map(|layer| {
                let path = match &layer.source {
                    LayerSource::Folder(path) | LayerSource::Archive(path) => path,
                };
                text_path(path).map(str::to_owned)
            })
            .collect::<io::Result<Vec<_>>>()?;
        layers.finish()?;
        Ok(GatheredContent {
            vanilla_hash,
            dlc,
            mods,
            language,
            layers,
            stats,
            order_note,
            expected_mounts,
        })
    })();
    // Preserve successfully hashed files even when a later descriptor is bad.
    // The cache is disposable and can be rebuilt after an interrupted write.
    fs::write(cache_path, serde_json::to_vec(&cache)?)?;
    result.map(|mut content: GatheredContent| {
        content.stats.elapsed_ms = started.elapsed().as_millis();
        content
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_is_closed_and_paths_cannot_escape_the_layer() {
        for path in [
            "map/provinces.bmp",
            "common/countries/colors.txt",
            "gfx/interface/x.dds",
            "gfx/flags/GER.tga",
            "localisation/english/x.yml",
        ] {
            assert!(covered(path));
        }
        for path in [
            "music/x.ogg",
            "gfx/models/x.mesh",
            "gfx/loadingscreens/x.dds",
            "map_backup/x.bmp",
            "launcher/logs/x.txt",
        ] {
            assert!(!covered(path));
        }
        assert_eq!(normalize_relative("map\\x.bmp").unwrap(), "map/x.bmp");
        for path in [
            "",
            "../x",
            "map/../x",
            "map//x",
            "/x",
            "C:/x",
            "map/x:stream",
            "map/./x",
            "map/x\0",
        ] {
            assert!(normalize_relative(path).is_err(), "{path}");
        }
    }

    #[test]
    fn layer_hash_uses_paths_content_and_an_unambiguous_sorted_list() {
        let first: [u8; 32] = Sha256::digest(b"first").into();
        let second: [u8; 32] = Sha256::digest(b"second").into();
        let files = BTreeMap::from([("map/b".into(), second), ("map/a".into(), first)]);
        let mut expected = Sha256::new();
        for (path, hash) in [("map/a", first), ("map/b", second)] {
            expected.update((path.len() as u32).to_le_bytes());
            expected.update(path.as_bytes());
            expected.update(hash);
        }
        assert_eq!(
            layer_hash(&files).unwrap(),
            <[u8; 32]>::from(expected.finalize())
        );
        let mut renamed = files.clone();
        let moved = renamed.remove("map/a").unwrap();
        renamed.insert("map/c".into(), moved);
        assert_ne!(layer_hash(&files).unwrap(), layer_hash(&renamed).unwrap());
        assert_ne!(
            layer_hash(&BTreeMap::new()).unwrap(),
            layer_hash(&files).unwrap()
        );
    }

    #[test]
    fn overlay_replacement_removes_subdirectories_without_prefix_collisions() {
        let mut effective = BTreeMap::new();
        let base = ["map/a", "map/sub/b", "maps/c", "common/a"].map(str::to_owned);
        apply_overlay(&mut effective, 0, false, base.iter(), &[]).unwrap();
        let additions = ["map/new".to_owned(), "common/a".to_owned()];
        apply_overlay(&mut effective, 1, false, additions.iter(), &["MAP".into()]).unwrap();
        assert_eq!(
            effective
                .into_iter()
                .map(|(path, file)| (path, file.layer))
                .collect::<BTreeMap<_, _>>(),
            BTreeMap::from([
                ("map/new".into(), 1),
                ("maps/c".into(), 0),
                ("common/a".into(), 1)
            ])
        );
    }

    #[test]
    fn overlay_refuses_to_guess_folder_and_archive_case_precedence() {
        let mut effective = BTreeMap::new();
        apply_overlay(
            &mut effective,
            0,
            false,
            ["map/file.txt".to_owned()].iter(),
            &[],
        )
        .unwrap();
        apply_overlay(
            &mut effective,
            1,
            false,
            ["map/FILE.txt".to_owned()].iter(),
            &[],
        )
        .unwrap();
        assert_eq!(effective.len(), 1);
        assert_eq!(effective["map/file.txt"].layer, 1);
        assert_eq!(effective["map/file.txt"].path, "map/FILE.txt");
        assert!(
            apply_overlay(
                &mut effective,
                2,
                true,
                ["map/file.txt".to_owned()].iter(),
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn cache_key_requires_path_size_and_full_timestamp() {
        let stamp = Stamp {
            size: 42,
            modified_seconds: 5,
            modified_nanos: 99,
        };
        let mut cache = HashCache::default();
        cache.insert("a".into(), stamp.clone(), [7; 32]);
        assert_eq!(cache.get("a", &stamp), Some([7; 32]));
        assert_eq!(cache.get("b", &stamp), None);
        assert_eq!(
            cache.get(
                "a",
                &Stamp {
                    size: 43,
                    ..stamp.clone()
                }
            ),
            None
        );
        assert_eq!(
            cache.get(
                "a",
                &Stamp {
                    modified_nanos: 98,
                    ..stamp
                }
            ),
            None
        );
    }

    #[test]
    fn recorded_selection_preserves_order_and_ignores_unrequested_layers() {
        let selected = select_recorded_paths(
            "DLC",
            ["b", "a"].into_iter(),
            vec![
                ("a".into(), PathBuf::from("dlc/a/a.dlc")),
                ("extra".into(), PathBuf::from("dlc/extra/first.dlc")),
                ("b".into(), PathBuf::from("dlc/b/b.dlc")),
                ("extra".into(), PathBuf::from("dlc/extra/second.dlc")),
            ],
        )
        .unwrap();
        assert_eq!(
            selected,
            [PathBuf::from("dlc/b/b.dlc"), PathBuf::from("dlc/a/a.dlc")]
        );
        let missing =
            select_recorded_paths("DLC", ["missing"].into_iter(), Vec::new()).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        assert!(missing.to_string().contains("DLC missing"));
        let ambiguous = select_recorded_paths(
            "Mod",
            ["123"].into_iter(),
            vec![
                ("123".into(), PathBuf::from("mod/one.mod")),
                ("123".into(), PathBuf::from("mod/two.mod")),
            ],
        )
        .unwrap_err();
        assert!(ambiguous.to_string().contains("Mod 123"));
        assert!(ambiguous.to_string().contains("multiple"));
    }

    #[test]
    fn remote_mod_ids_take_precedence_over_local_descriptor_names() {
        let remote = parser::parse(b"remote_file_id=123 name=Example").unwrap();
        let empty = parser::parse(b"remote_file_id=\"\" name=Example").unwrap();
        let local = parser::parse(b"name=Example").unwrap();
        let descriptor = Path::new("mod/example.mod");
        assert_eq!(mod_id(&remote, descriptor).unwrap(), "123");
        assert_eq!(mod_id(&empty, descriptor).unwrap(), "example.mod");
        assert_eq!(mod_id(&local, descriptor).unwrap(), "example.mod");
        assert!(
            select_recorded_paths(
                "Mod",
                ["example.mod"].into_iter(),
                vec![(mod_id(&remote, descriptor).unwrap(), descriptor.to_owned())],
            )
            .is_err()
        );
    }

    /// Builds a folder layer without inventorying it; `read_uncovered` never
    /// consults the covered inventory.
    fn folder_layer(id: &str, root: &Path, replace_paths: Vec<String>) -> ResolvedLayer {
        ResolvedLayer {
            id: id.into(),
            source: LayerSource::Folder(root.to_path_buf()),
            descriptor: None,
            replace_paths,
            files: BTreeMap::new(),
            hashes: BTreeMap::new(),
            archive_stamp: None,
            descriptor_hash: None,
        }
    }

    #[test]
    fn listed_paths_keep_case_variants_and_honour_replace_path() {
        let layer = |id: &str, replace: &[&str], files: &[&str]| {
            let mut layer = folder_layer(
                id,
                Path::new(id),
                replace.iter().map(|p| p.to_string()).collect(),
            );
            for path in files {
                layer.files.insert(
                    path.to_string(),
                    Member {
                        archive_index: None,
                        size: 0,
                    },
                );
            }
            layer
        };
        let layers = ResolvedLayers {
            layers: vec![
                layer(
                    "base",
                    &[],
                    &["common/defines/00_defines.lua", "common/defines/sub/a.lua"],
                ),
                layer(
                    "mod",
                    &["common/defines/sub"],
                    &[
                        "common/defines/00_Defines.lua",
                        "common/defines/00_defines.lua",
                    ],
                ),
            ],
            ..ResolvedLayers::default()
        };
        assert_eq!(
            layers
                .listed_paths("common/defines/")
                .into_iter()
                .collect::<Vec<_>>(),
            [
                "common/defines/00_Defines.lua",
                "common/defines/00_defines.lua"
            ]
        );
    }

    #[test]
    fn uncovered_reads_follow_load_order_replacement_and_folder_case() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_installed_content_uncovered_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        let (base, over) = (root.join("base"), root.join("over"));
        fs::create_dir_all(base.join("gfx/Fonts")).unwrap();
        fs::create_dir_all(over.join("gfx/fonts")).unwrap();
        fs::write(base.join("gfx/Fonts/Hoi.fnt"), b"base").unwrap();
        fs::write(over.join("gfx/fonts/hoi.fnt"), b"over").unwrap();

        // A path outside the coverage roots resolves whatever case it carries.
        assert_eq!(
            folded_path(&base, "gfx/fonts/hoi.fnt").unwrap(),
            Some(base.join("gfx").join("Fonts").join("Hoi.fnt"))
        );
        assert_eq!(folded_path(&base, "gfx/fonts/absent.fnt").unwrap(), None);
        assert_eq!(folded_path(&base, "no/such/dir.fnt").unwrap(), None);

        let mut layers = ResolvedLayers {
            layers: vec![
                folder_layer("base", &base, Vec::new()),
                folder_layer("over", &over, Vec::new()),
            ],
            effective: BTreeMap::new(),
            uncovered_memo: Default::default(),
            visual: Vec::new(),
        };
        assert_eq!(layers.read_uncovered("gfx/fonts/hoi.fnt").unwrap(), b"over");
        assert!(!layers.uncovered_from_base("gfx/fonts/hoi.fnt").unwrap());
        layers.layers.pop();
        assert_eq!(layers.read_uncovered("gfx/fonts/hoi.fnt").unwrap(), b"base");
        assert!(layers.uncovered_from_base("gfx/fonts/hoi.fnt").unwrap());

        // A replacing layer hides the layers below it even when it has no file.
        let replacing = ResolvedLayers {
            layers: vec![
                folder_layer("base", &base, Vec::new()),
                folder_layer("over", &root.join("empty"), vec!["gfx/fonts".into()]),
            ],
            effective: BTreeMap::new(),
            uncovered_memo: Default::default(),
            visual: Vec::new(),
        };
        assert_eq!(
            replacing
                .read_uncovered("gfx/fonts/hoi.fnt")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(!replacing.uncovered_from_base("gfx/fonts/hoi.fnt").unwrap());

        // Covered paths go through the verifying reader, and unsafe paths fail.
        assert_eq!(
            layers
                .read_uncovered("map/provinces.bmp")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(layers.read_uncovered("../escape").is_err());
        assert!(layers.read_uncovered("gfx/fonts/absent.fnt").is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hash_mismatch_names_the_layer_and_both_digests() {
        check_hash("DLC", "dlc034_no_step_back", [7; 32], [7; 32]).unwrap();
        let error = check_hash("DLC", "dlc034_no_step_back", [8; 32], [7; 32])
            .unwrap_err()
            .to_string();
        assert!(error.contains("DLC dlc034_no_step_back"));
        assert!(error.contains(&"07".repeat(32)));
        assert!(error.contains(&"08".repeat(32)));
    }
}
