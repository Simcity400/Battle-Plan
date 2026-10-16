//! Content identity coverage and local resolution of machine-free layer ids.
//!
//! Covered roots include event scripts, text icons, leader portraits, fonts,
//! effect sources and post-effect volumes, plus every `.asset` and `.gfx`
//! descriptor below `gfx`: the engine registers entities, meshes and particles
//! from every such file.
//! Other meshes, animations and textures stay outside and are read unverified.
//! The policy version lives in replay.
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

use replay_identity::{COVERAGE_POLICY_VERSION, Dlc, Identity, Mod, Verification};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::parser::{self, Document};

mod physfs_zip;
pub mod visual;

// Historical roots from 2ae47c8122 (policy 2), 2df8adf4b3 (3) and 5aea650ab7 (4).
// Each policy hashes its own roots; `.asset`/`.gfx` coverage is shared since policy 2.
const COVERAGE_ROOTS: [(&str, u32); 13] = [
    ("map", 2),
    ("common", 2),
    ("history", 2),
    ("events", 3),
    ("localisation", 2),
    ("interface", 2),
    ("gfx/flags", 2),
    ("gfx/interface", 2),
    ("gfx/texticons", 3),
    ("gfx/leaders", 3),
    ("gfx/fonts", 4),
    ("gfx/fx", 4),
    ("gfx/posteffect_volumes.txt", 4),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CoveragePolicy(u32);

impl Default for CoveragePolicy {
    fn default() -> Self {
        Self(COVERAGE_POLICY_VERSION)
    }
}

impl CoveragePolicy {
    fn recorded(version: u32) -> io::Result<Self> {
        match version {
            2..=COVERAGE_POLICY_VERSION => Ok(Self(version)),
            _ => Err(invalid(format!(
                "unsupported recorded content coverage policy {version} (supported 2 through {COVERAGE_POLICY_VERSION})"
            ))),
        }
    }

    fn roots(self) -> impl Iterator<Item = &'static str> {
        COVERAGE_ROOTS
            .iter()
            .filter_map(move |(root, since)| (*since <= self.0).then_some(*root))
    }

    fn covered_path(self, path: &[u8]) -> bool {
        let lower = path.to_ascii_lowercase();
        (is_descriptor(&lower) || self.roots().any(|root| below(&lower, root.as_bytes())))
            && !lower.split(|byte| *byte == b'/').any(|part| {
                matches!(
                    part,
                    b"music"
                        | b"sound"
                        | b"video"
                        | b"videos"
                        | b"loadingscreens"
                        | b"loading_screens"
                        | b"launcher"
                        | b"crash_reporter"
                        | b"logs"
                )
            })
    }
}

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
    /// The canonical root after inventory construction.
    Folder(PathBuf),
    Archive(PathBuf),
}

/// A layer path as the game's virtual file system compares it: raw bytes. PhysFS keys a ZIP
/// member by its central-directory name bytes, without code-page conversion (see
/// [`physfs_zip`]), and looks it up by the requested bytes with a djb2 hash and byte comparison,
/// without case folding (see [`ResolvedLayer::archive_open`]). Folder paths are their file names'
/// UTF-8. Every layer
/// index, `replace_path` and lookup keeps these bytes; text is only a view of them.
pub type LayerPath = Vec<u8>;

/// Shared uncovered dependencies, queried when a consumer reports diagnostics.
/// Without an explicit reporting opt-in, ordinary reads remain strict.
#[derive(Clone, Debug)]
pub struct UncoveredReads {
    policy: u32,
    paths: std::sync::Arc<std::sync::Mutex<Option<BTreeSet<LayerPath>>>>,
    overlays: OverlayReads,
}

/// What visual mods supplied and what of theirs could not be shown, shared by a recorded stack
/// and its presentation stack.
type OverlayReads = std::sync::Arc<std::sync::Mutex<OverlayDiagnostics>>;

#[derive(Debug, Default)]
struct OverlayDiagnostics {
    /// Files each visual mod supplied, by the mod's display name: presentation reads the
    /// recording never verified, named and counted apart from uncovered recorded reads.
    reads: BTreeMap<String, BTreeSet<LayerPath>>,
    /// Artwork a consumer could not load from the presentation stack and drew from the
    /// recording instead ([`ResolvedLayers::artwork`]), with why.
    refusals: BTreeSet<String>,
}

/// The start of the lines [`UncoveredReads::append_to`] keeps current for visual mod reads.
const OVERLAY_READS: &str = "Artwork from visual mods";

impl UncoveredReads {
    pub fn warning(&self) -> Option<String> {
        let paths = self.paths.lock().unwrap_or_else(|error| error.into_inner());
        let paths = paths.as_ref().filter(|paths| !paths.is_empty())?;
        Some(format!(
            "Content: {} file(s) read outside the recorded content coverage policy {}; these bytes are not hash-verified: {}.",
            paths.len(),
            self.policy,
            paths
                .iter()
                .map(|path| display(path))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// The files visual mods supplied, counted per mod in no particular precedence.
    pub fn overlay_warning(&self) -> Option<String> {
        let diagnostics = self
            .overlays
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let total: usize = diagnostics.reads.values().map(BTreeSet::len).sum();
        (total > 0).then(|| {
            format!(
                "{OVERLAY_READS}: {total} file(s) shown from mods that are not part of the recording, not its recorded artwork: {}.",
                diagnostics
                    .reads
                    .iter()
                    .map(|(name, paths)| format!("{name} ({})", paths.len()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    }

    /// The artwork that could not be loaded from visual mods, which the recording's replaced.
    pub fn overlay_refusals(&self) -> Option<String> {
        let diagnostics = self
            .overlays
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (!diagnostics.refusals.is_empty()).then(|| {
            format!(
                "{OVERLAY_READS} not shown: {} file(s) could not be loaded, so the recorded artwork is drawn: {}.",
                diagnostics.refusals.len(),
                diagnostics
                    .refusals
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })
    }

    /// Refresh a cached warning list rather than accumulating stale counts.
    pub fn append_to(&self, warnings: &mut Vec<String>) {
        warnings.retain(|line| {
            let uncovered = line.starts_with("Content: ")
                && line.contains("file(s) read outside the recorded content coverage policy");
            !uncovered && !line.starts_with(OVERLAY_READS)
        });
        warnings.extend(self.warning());
        warnings.extend(self.overlay_warning());
        warnings.extend(self.overlay_refusals());
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedLayer {
    coverage: CoveragePolicy,
    pub id: String,
    pub source: LayerSource,
    pub descriptor: Option<PathBuf>,
    pub replace_paths: Vec<LayerPath>,
    /// Inventory, lookup index and hashes are shared by clones: a presentation stack mounts
    /// the recorded layers without copying them.
    files: std::sync::Arc<BTreeMap<LayerPath, Member>>,
    lookup: std::sync::Arc<LookupIndex>,
    hashes: std::sync::Arc<BTreeMap<LayerPath, [u8; 32]>>,
    archive_stamp: Option<Stamp>,
    descriptor_hash: Option<[u8; 32]>,
    /// A visual mod mounted for presentation only ([`ResolvedLayers::presentation`]).
    overlay: Option<Overlay>,
}

/// A presentation mount: the mod's display name, and whether its covered bytes still verify
/// against the recording (a recorded mod selected again keeps its recorded hashes).
#[derive(Clone, Debug)]
struct Overlay {
    name: String,
    verified: bool,
}

#[derive(Clone, Debug)]
struct Member {
    /// The archive record it reads from; `None` for a folder file.
    archive_entry: Option<physfs_zip::Entry>,
    size: u64,
}

#[derive(Clone, Debug, Default)]
struct LookupIndex {
    folders: BTreeMap<LayerPath, LayerPath>,
    archive: Option<physfs_zip::Index>,
}

/// Text for a layer path in messages and in the hash cache's keys, which a non-UTF-8 path
/// cannot share with a UTF-8 one (a layer path holds no control character).
fn display(path: &[u8]) -> String {
    match std::str::from_utf8(path) {
        Ok(text) => text.to_owned(),
        Err(_) => format!(
            "\u{1}{}",
            path.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedLayers {
    coverage: CoveragePolicy,
    pub layers: Vec<ResolvedLayer>,
    /// Built-in layers the engine mounts above every mod (the tweaker assets): visual mods
    /// mount below them, as an enabled mod would.
    above_mods: usize,
    /// The same recorded layers with the selected visual mods mounted above them, or `None`
    /// when no visual mod is selected ([`Self::presentation`]).
    presentation: Option<std::sync::Arc<ResolvedLayers>>,
    overlay_reads: OverlayReads,
    effective: BTreeMap<LayerPath, EffectiveFile>,
    /// Where uncovered paths resolved, so repeats skip the folder layers' failed filesystem
    /// probes. Each entry records the layer roster it was resolved against, so layer push/pop
    /// (as in the fallback tests) misses instead of answering stale. Uncovered folder files are
    /// assumed stable during one load; a hit the filesystem no longer reproduces falls back to a
    /// fresh scan rather than failing. Every answer it gives, hit or miss, first passes the
    /// verification the scan applies ([`ResolvedLayers::verify_passed`], and the hit layer's own
    /// lookup for an archive hit): it never skips an archive's.
    uncovered_memo: std::sync::Arc<std::sync::Mutex<UncoveredMemo>>,
    /// Ordinary successful uncovered reads and explicit uncovered attempts.
    /// `None` keeps readers strict until their reporting boundary opts in.
    uncovered_reads: std::sync::Arc<std::sync::Mutex<Option<BTreeSet<LayerPath>>>>,
    shader_warnings: std::sync::Arc<std::sync::Mutex<BTreeSet<String>>>,
    /// The `gfx/models` texture index and the `gfx` descriptor roster, each
    /// built once per layer roster and shared by every model library.
    texture_index: Memo<crate::mesh::TextureIndex>,
    descriptor_roster: Memo<crate::mesh::Roster>,
    /// The content names every trigger body is read against, built once per
    /// layer roster and shared by every content reader.
    key_names: Memo<crate::key_names::KeyNames>,
    /// Faction definitions and read failures shared by every naming consumer. A
    /// transient I/O failure also stays cached until the layer roster changes.
    faction_templates: Memo<Result<std::sync::Arc<crate::faction_templates::Catalog>, String>>,
    derived: std::sync::Arc<std::sync::Mutex<Derived>>,
}

#[derive(Default)]
struct Derived {
    roster: Vec<String>,
    values: std::collections::HashMap<
        std::any::TypeId,
        std::sync::Arc<dyn std::any::Any + Send + Sync>,
    >,
}

impl std::fmt::Debug for Derived {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Derived")
            .field("roster", &self.roster)
            .field("entries", &self.values.len())
            .finish()
    }
}

type Memo<T> = std::sync::Arc<std::sync::Mutex<Option<(Vec<String>, std::sync::Arc<T>)>>>;

/// Memoized uncovered resolutions with the roster they were resolved against.
#[derive(Debug, Default)]
struct UncoveredMemo {
    layers: Vec<String>,
    hits: BTreeMap<LayerPath, Option<UncoveredHit>>,
}

/// Where one uncovered path resolved: a folder file, or an archive layer, which answers it again
/// through its one verified lookup ([`ResolvedLayer::archive_open`]).
#[derive(Clone, Debug)]
enum UncoveredHit {
    /// The folder layer at the index, and the opened file's real relative path.
    Folder(usize, LayerPath),
    /// The archive layer at the index.
    Archive(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EffectiveFile {
    layer: usize,
    path: LayerPath,
    archive: bool,
}

struct RecordedHit {
    layer: usize,
    path: LayerPath,
    folder: Option<File>,
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
    /// Each mod's descriptor file name under `mod/`, in `mods` order: the
    /// name the game's mod map holds while the mod is active.
    pub mod_descriptors: Vec<String>,
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

/// A descriptor's `replace_path` as the overlay compares it: its bytes without trailing
/// separators, then normalized like every layer path.
fn replace_path(path: &[u8]) -> io::Result<LayerPath> {
    let mut end = path.len();
    while end > 0 && matches!(path[end - 1], b'/' | b'\\') {
        end -= 1;
    }
    normalize_path(&path[..end])
}

/// A coverage path uses `/`, retains case, and cannot escape its layer.
pub fn normalize_relative(path: &str) -> io::Result<String> {
    normalize_path(path.as_bytes())
        .map(|path| String::from_utf8(path).expect("normalizing keeps UTF-8 text UTF-8"))
}

/// [`normalize_relative`] over a path's bytes: `\` made `/`, and a path that is empty, starts
/// with `/`, or has an empty, `.`, `..`, `:`-holding or control-holding part refused (a control
/// character of the text for UTF-8 bytes, a control byte otherwise).
pub fn normalize_path(path: &[u8]) -> io::Result<LayerPath> {
    let path: LayerPath = path
        .iter()
        .map(|byte| if *byte == b'\\' { b'/' } else { *byte })
        .collect();
    let control = |part: &[u8]| match std::str::from_utf8(part) {
        Ok(text) => text.chars().any(char::is_control),
        Err(_) => part.iter().any(|byte| byte.is_ascii_control()),
    };
    if path.is_empty()
        || path.starts_with(b"/")
        || path.split(|byte| *byte == b'/').any(|part| {
            part.is_empty()
                || part == b"."
                || part == b".."
                || part.contains(&b':')
                || control(part)
        })
    {
        return Err(invalid(format!("unsafe layer path: {:?}", display(&path))));
    }
    Ok(path)
}

/// A requested path as [`ResolvedLayers::lookup_path`] hands it to every layer, through
/// PhysFS's `sanitizePlatformIndependentPath`: cut at its first NUL,
/// its leading `/`s dropped, runs of `/` made one and a final `/` dropped; `None`, which fails
/// the open before any layer is searched (`PHYSFS_ERR_BAD_FILENAME`), for a `:` or a `\`
/// anywhere, or a `.` or `..` part other than the last (the whole path included). A `\` is
/// never a separator here: callers that accept it convert `\` to `/` before lookup (see
/// [`crate::gui::native::fonts::texture_candidates`]), and an archive record whose name
/// holds a `\` (from a host other than FAT) is found by no request.
pub fn sanitize(request: &[u8]) -> Option<LayerPath> {
    let request = parser::c_string(request);
    let start = request
        .iter()
        .position(|byte| *byte != b'/')
        .unwrap_or(request.len());
    let rest = &request[start..];
    if rest == b"." || rest == b".." {
        return None;
    }
    let mut out = Vec::with_capacity(rest.len());
    let (mut part, mut at) = (0, 0);
    while at < rest.len() {
        let byte = rest[at];
        at += 1;
        match byte {
            b':' | b'\\' => return None,
            b'/' => {
                if matches!(&out[part..], b"." | b"..") {
                    return None;
                }
                while rest.get(at) == Some(&b'/') {
                    at += 1;
                }
                if at == rest.len() {
                    break;
                }
                out.push(b'/');
                part = out.len();
            }
            _ => out.push(byte),
        }
    }
    Some(out)
}

/// Whether a sanitized request can open a folder file at all: PhysFS's Windows open
/// ([`folder_file`]) is `CreateFileW` without `FILE_FLAG_BACKUP_SEMANTICS`, which
/// opens no directory, so the empty path (the root) or one whose last part is `.` or `..`
/// opens nothing there, and the search goes on.
fn folder_opens(relative: &[u8]) -> bool {
    !relative.is_empty()
        && !matches!(
            relative.rsplit(|byte| *byte == b'/').next(),
            Some(b"." | b"..")
        )
}

/// The scheme [`record_digest`] follows, part of every archive record's key in the persistent
/// hash cache (`content-hashes-v1.json`), so an entry an earlier scheme wrote (a record's content
/// read through the `zip` crate, or an unopened record hashed without its central fields) is
/// never served for this one: it is a miss, and the record is hashed again.
const RECORD_DIGEST_SCHEME: &str = "archive-record-digest-2";

/// How the layer hash holds an archive record, the same at gather and at every verification:
/// the content a request without a password reads, or, for a record the game does not open that
/// way (its local header disagrees with its central record, or it is encrypted), everything that
/// decides what any request opens: the central record's fields that govern opening and decoding
/// ([`physfs_zip::Entry::opening`], among them the DOS time whose high byte checks a password
/// under flag 8, with no comparison to the local header's time) and the bytes the record
/// occupies as stored ([`physfs_zip::Archive::stored`]),
/// marked apart from content. With the content read, if any, and the bytes read.
fn record_digest(
    archive: &mut physfs_zip::Archive,
    entry: &physfs_zip::Entry,
) -> io::Result<([u8; 32], Option<Vec<u8>>, u64)> {
    #[cfg(test)]
    tests::LOADS.with(|loads| loads.set(loads.get() + 1));
    Ok(match archive.read(entry, None)? {
        Some(bytes) => (Sha256::digest(&bytes).into(), Some(bytes), entry.size),
        None => {
            let stored = archive.stored(entry)?;
            let mut hash = Sha256::new();
            hash.update(b"\0unopened archive record\0");
            hash.update(entry.opening());
            hash.update(&stored);
            (hash.finalize().into(), None, stored.len() as u64)
        }
    })
}

/// The files of `listed` directly in `directory` whose name ends in `suffix`
/// (case-sensitive), in the byte order of their
/// names. See [`ResolvedLayers::directory_files`].
fn directory_order(directory: &str, suffix: &str, listed: Vec<String>) -> Vec<String> {
    let directory = directory.trim_end_matches('/').to_ascii_lowercase();
    let mut files: Vec<(String, String)> = listed
        .into_iter()
        .filter_map(|path| {
            let (parent, name) = path.rsplit_once('/')?;
            let name = name.to_owned();
            (parent.eq_ignore_ascii_case(&directory) && name.ends_with(suffix))
                .then_some((name, path))
        })
        .collect();
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    files.into_iter().map(|(_, path)| path).collect()
}

fn below(path: &[u8], directory: &[u8]) -> bool {
    path == directory
        || path
            .strip_prefix(directory)
            .is_some_and(|tail| tail.starts_with(b"/"))
}

/// The root whose `.asset` and `.gfx` descriptors are covered.
const DESCRIPTOR_ROOT: &str = "gfx";

fn is_descriptor(lower: &[u8]) -> bool {
    below(lower, DESCRIPTOR_ROOT.as_bytes())
        && (lower.ends_with(b".asset") || lower.ends_with(b".gfx"))
}

/// Whether the current policy covers a text path. A recording's consumers use
/// [`ResolvedLayers::covered`] for its own policy. Music, sound, video, loading
/// screens, launcher, crash reporter and logs are outside this closed set.
/// A descriptor is added separately to the layer hash.
pub fn covered(path: &str) -> bool {
    covered_path(path.as_bytes())
}

/// [`covered`] over a layer path's bytes.
pub fn covered_path(path: &[u8]) -> bool {
    CoveragePolicy::default().covered_path(path)
}

/// Sorted (u32 LE path byte length, relative path bytes, SHA-256)
/// tuples. Length framing makes the list unambiguous, including empty layers.
pub fn layer_hash(files: &BTreeMap<LayerPath, [u8; 32]>) -> io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    for (path, digest) in files {
        let length = u32::try_from(path.len()).map_err(|_| invalid("layer path too long"))?;
        hash.update(length.to_le_bytes());
        hash.update(path);
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
    coverage: CoveragePolicy,
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
                .scalar()
                .ok_or_else(|| invalid("replace_path is not a scalar"))
                .and_then(replace_path)
        })
        .collect::<io::Result<Vec<_>>>()?;
    if !is_mod && !replace_paths.is_empty() {
        return Err(invalid(
            "DLC replace_path is not supported by this coverage policy",
        ));
    }
    Ok((
        ResolvedLayer {
            coverage,
            id,
            source,
            descriptor: Some(path),
            replace_paths,
            files: Default::default(),
            lookup: Default::default(),
            hashes: Default::default(),
            archive_stamp: None,
            descriptor_hash: Some(descriptor_hash),
            overlay: None,
        },
        document,
    ))
}

fn directory_entries(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

/// A visual mod's folder or archive is mounted as the game mounts it: a symbolic link is not
/// followed (the game's file system does not permit them), and names that differ only in case
/// stay apart in an archive and the first in a folder. A recorded layer refuses both instead, as
/// verification cannot prove what the game read.
fn collect_folder(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<LayerPath, Member>,
    coverage: CoveragePolicy,
    overlay: bool,
) -> io::Result<()> {
    for entry in directory_entries(path)? {
        let full_path = entry.path();
        let relative = normalize_path(
            text_path(
                full_path
                    .strip_prefix(root)
                    .map_err(|_| invalid("layer path escaped root"))?,
            )?
            .as_bytes(),
        )?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            if overlay {
                continue;
            }
            return Err(invalid(format!(
                "content layer contains a symbolic link: {}",
                full_path.display()
            )));
        }
        if kind.is_dir() {
            collect_folder(root, &full_path, files, coverage, overlay)?;
        } else if kind.is_file() && coverage.covered_path(&relative) {
            files.insert(
                relative,
                Member {
                    archive_entry: None,
                    size: entry.metadata()?.len(),
                },
            );
        }
    }
    Ok(())
}

impl ResolvedLayer {
    /// Whether reads check this layer's covered bytes against the recording. A visual mod the
    /// recording does not hold is presentation only and has no recorded bytes to check.
    fn verifies(&self) -> bool {
        self.overlay.as_ref().is_none_or(|overlay| overlay.verified)
    }

    fn inventory(&mut self) -> io::Result<()> {
        if let LayerSource::Folder(root) = &mut self.source {
            *root = fs::canonicalize(&*root)?;
        }
        match &self.source {
            LayerSource::Folder(root) => {
                // `gfx` holds the descriptors and the covered `gfx/*` roots.
                let walked = self
                    .coverage
                    .roots()
                    .filter(|directory| !below(directory.as_bytes(), DESCRIPTOR_ROOT.as_bytes()))
                    .chain(std::iter::once(DESCRIPTOR_ROOT));
                for directory in walked {
                    let path = root.join(directory);
                    if path.try_exists()? {
                        collect_folder(
                            root,
                            &path,
                            std::sync::Arc::make_mut(&mut self.files),
                            self.coverage,
                            self.overlay.is_some(),
                        )?;
                    }
                }
            }
            LayerSource::Archive(path) => {
                let stamp = Stamp::read(path)?;
                let mut archive = physfs_zip::Archive::open(path)?;
                let mounted = archive.mount()?;
                // Every file record by its tree name, as the game's lookups find it: raw bytes,
                // never normalized (a `\\` from a host other than FAT stays, and no request finds
                // it; see [`sanitize`]).
                for entry in &mounted.entries {
                    if entry.directory {
                        continue;
                    }
                    if entry.symlink {
                        if self.overlay.is_some() {
                            continue;
                        }
                        return Err(invalid(format!(
                            "archive contains a symbolic link: {}",
                            display(&entry.name)
                        )));
                    }
                    if self.coverage.covered_path(&entry.name) {
                        std::sync::Arc::make_mut(&mut self.files).insert(
                            entry.name.clone(),
                            Member {
                                size: entry.size,
                                archive_entry: Some(entry.clone()),
                            },
                        );
                    }
                }
                std::sync::Arc::make_mut(&mut self.lookup).archive = Some(archive.index(mounted)?);
                if Stamp::read(path)? != stamp {
                    return Err(invalid("archive changed during inventory"));
                }
                self.archive_stamp = Some(stamp);
            }
        }
        self.finish_inventory()
    }

    fn finish_inventory(&mut self) -> io::Result<()> {
        let mut folded = BTreeMap::new();
        for path in self.files.keys() {
            if folded.contains_key(&path.to_ascii_lowercase()) {
                // See `collect_folder`: a visual mod keeps the first spelling.
                if self.overlay.is_some() {
                    continue;
                }
            }
            if folded
                .insert(path.to_ascii_lowercase(), path.clone())
                .is_some()
            {
                return Err(invalid(format!(
                    "layer contains case-ambiguous path: {}",
                    display(path)
                )));
            }
        }
        if matches!(self.source, LayerSource::Folder(_)) {
            std::sync::Arc::make_mut(&mut self.lookup).folders = folded;
        }
        Ok(())
    }

    fn hash(&mut self, cache: &mut HashCache, stats: &mut HashStats) -> io::Result<[u8; 32]> {
        let mut hashes = BTreeMap::new();
        match &self.source {
            LayerSource::Folder(root) => {
                for relative in self.files.keys() {
                    hashes.insert(
                        relative.clone(),
                        cache.file(&root.join(folder_text(relative)?), stats)?,
                    );
                }
            }
            LayerSource::Archive(path) => {
                let stamp = Stamp::read(path)?;
                if self.archive_stamp.as_ref() != Some(&stamp) {
                    return Err(invalid("archive changed since inventory"));
                }
                let mut archive = physfs_zip::Archive::open(path)?;
                for (relative, member) in self.files.iter() {
                    let key = format!(
                        "{RECORD_DIGEST_SCHEME}\0{}\0{}",
                        text_path(path)?,
                        display(relative)
                    );
                    stats.files += 1;
                    stats.bytes += member.size;
                    let digest = if let Some(digest) = cache.get(&key, &stamp) {
                        stats.cache_hits += 1;
                        digest
                    } else {
                        let entry = member.archive_entry.as_ref().unwrap();
                        let (digest, _, size) = record_digest(&mut archive, entry)
                            .map_err(|error| invalid(format!("{}: {error}", display(relative))))?;
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
        self.hashes = std::sync::Arc::new(hashes.clone());
        if let Some(descriptor) = &self.descriptor {
            let name = text_path(Path::new(
                descriptor
                    .file_name()
                    .ok_or_else(|| invalid("descriptor has no name"))?,
            ))?;
            let key = normalize_path(name.as_bytes())?;
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

    /// Opens and identifies a folder file without reading its payload. Windows aliases
    /// (dots/spaces, 8.3 and Unicode case) resolve to the inventoried spelling.
    fn folder_lookup(&self, relative: &[u8]) -> io::Result<Option<(LayerPath, File)>> {
        let LayerSource::Folder(root) = &self.source else {
            return Err(invalid(format!("layer {} is not a folder", self.id)));
        };
        if !folder_opens(relative) {
            return Ok(None);
        }
        let Ok(text) = std::str::from_utf8(relative) else {
            return Ok(None);
        };
        let (file, path) = match folder_file(root, text) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return match self.lookup.folders.get(&relative.to_ascii_lowercase()) {
                    Some(path) => Err(changed(path)),
                    None => Ok(None),
                };
            }
            opened => opened?,
        };
        let real = match path.strip_prefix(root) {
            Ok(real) => real,
            // A link the game does not follow (see `collect_folder`) opens nothing in a mod.
            Err(_) if self.overlay.is_some() => return Ok(None),
            Err(_) => {
                return Err(invalid(format!(
                    "folder file resolves outside its layer: {}",
                    path.display()
                )));
            }
        };
        let real = normalize_path(text_path(real)?.as_bytes())?;
        if self.verifies()
            && self.coverage.covered_path(&real)
            && self.lookup.folders.get(&real.to_ascii_lowercase()) != Some(&real)
        {
            return Err(changed(&real));
        }
        Ok(Some((real, file)))
    }

    /// Reads and verifies the real spelling through the handle that resolved it.
    fn folder_read(&self, real: &[u8], mut file: File) -> io::Result<Vec<u8>> {
        #[cfg(test)]
        tests::LOADS.with(|loads| loads.set(loads.get() + 1));
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if self.coverage.covered_path(real) {
            self.verify(real, &Sha256::digest(&bytes).into())?;
        }
        Ok(bytes)
    }

    fn folder_open(&self, relative: &[u8]) -> io::Result<Option<(LayerPath, Vec<u8>)>> {
        let Some((real, file)) = self.folder_lookup(relative)? else {
            return Ok(None);
        };
        let bytes = self.folder_read(&real, file)?;
        Ok(Some((real, bytes)))
    }

    fn verify(&self, relative: &[u8], digest: &[u8; 32]) -> io::Result<()> {
        if self.verifies() && self.hashes.get(relative) != Some(digest) {
            return Err(changed(relative));
        }
        Ok(())
    }

    /// The one verification point of an archive layer, which every lookup in it passes first,
    /// covered or not, hit or miss, with or without a `$`: the archive must be the file
    /// inventoried (its stamp), and its covered records as it mounts now must be the ones
    /// inventoried, central record for central record (none changed, gone or added). What a
    /// lookup answers depends on all of it: which record a request names, whether it holds an
    /// encrypted record (the `$` split), and whether a password opens it (the
    /// central time under flag 8, as [`record_digest`] describes). A difference is "changed since
    /// verification". The records' payloads are verified where a lookup names one
    /// ([`Self::archive_open`]).
    fn verified_archive(&self) -> io::Result<(physfs_zip::Archive, physfs_zip::Mounted)> {
        let LayerSource::Archive(path) = &self.source else {
            return Err(invalid(format!("layer {} is not an archive", self.id)));
        };
        let unchanged =
            Stamp::read(path).is_ok_and(|stamp| self.archive_stamp.as_ref() == Some(&stamp));
        if !unchanged {
            return Err(invalid(format!(
                "archive changed since verification: {}",
                path.display()
            )));
        }
        let mut archive = physfs_zip::Archive::open(path)?;
        let mounted = archive.mount()?;
        if !self.verifies() {
            return Ok((archive, mounted));
        }
        let mut covered = 0;
        for entry in &mounted.entries {
            if entry.directory || !self.coverage.covered_path(&entry.name) {
                continue;
            }
            covered += 1;
            if self
                .files
                .get(&entry.name)
                .and_then(|member| member.archive_entry.as_ref())
                != Some(entry)
            {
                return Err(changed(&entry.name));
            }
        }
        if covered != self.files.len() {
            let gone = self
                .files
                .keys()
                .find(|name| !mounted.entries.iter().any(|entry| entry.name == **name))
                .cloned()
                .unwrap_or_default();
            return Err(changed(&gone));
        }
        Ok((archive, mounted))
    }

    /// The one lookup of a request in an archive layer, after [`Self::verified_archive`]: the
    /// record the request names (exactly, or up to its last `$` with the rest as the password,
    /// [`physfs_zip::Mounted::lookup`]), with local-header and password admission. A covered
    /// record is verified by its [`record_digest`] whatever the open does, so a record changed
    /// so that it no longer opens is "changed since verification", never a silent pass to the
    /// layers below. `None` when the game's open fails, and its search goes on
    /// (see [`ResolvedLayers::lookup_path`]).
    fn archive_open(&self, request: &[u8]) -> io::Result<Option<Vec<u8>>> {
        let (mut archive, mounted) = self.verified_archive()?;
        let Some((entry, password)) = mounted.lookup(request) else {
            return Ok(None);
        };
        let context = |error: io::Error| invalid(format!("{}: {error}", display(request)));
        if self.verifies() && self.files.contains_key(&entry.name) {
            let (digest, bytes, _) = record_digest(&mut archive, entry).map_err(context)?;
            self.verify(&entry.name, &digest)?;
            if password.is_none() {
                return Ok(bytes);
            }
        }
        archive.read(entry, password).map_err(context)
    }

    fn archive_exists(&self, request: &[u8]) -> io::Result<bool> {
        let (mut archive, mounted) = self.verified_archive()?;
        match mounted.lookup(request) {
            Some((entry, password)) => archive.opens(entry, password),
            None => Ok(false),
        }
    }

    /// A lookup this layer answers nothing to, verified as one it answered would be: an archive
    /// layer's by its one lookup ([`Self::archive_open`], which must still answer nothing), a
    /// folder layer's by no covered file having appeared under the request's Windows aliases.
    /// An uncovered folder file is not verified, as the uncovered reads say.
    fn verify_miss(&self, relative: &[u8]) -> io::Result<()> {
        if !self.verifies() {
            return Ok(());
        }
        match &self.source {
            LayerSource::Archive(_) => match self.archive_open(relative)? {
                Some(_) => Err(changed(relative)),
                None => Ok(()),
            },
            LayerSource::Folder(_) => {
                if let Some((real, _)) = self.folder_lookup(relative)?
                    && self.coverage.covered_path(&real)
                {
                    return Err(changed(&real));
                }
                Ok(())
            }
        }
    }
}

/// A covered record that differs from what was verified: changed, gone or added.
fn changed(relative: &[u8]) -> io::Error {
    invalid(format!(
        "content file changed since verification: {}",
        display(relative)
    ))
}

/// A folder layer's path as the file system names it. Every folder path in an index came from
/// the file system's own (UTF-8) names.
fn folder_text(relative: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(relative).map_err(|_| invalid("folder layer path is not UTF-8"))
}

/// A file the game requests by a literal path, opened as [`ResolvedLayers::read_path`] opens it:
/// `None` when no layer holds it (the requester passes a missing file), an error when the open
/// fails otherwise (a verification failure included).
pub(crate) fn open_request(layers: &ResolvedLayers, path: &[u8]) -> io::Result<Option<Vec<u8>>> {
    match layers.read_path(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn not_found(relative: &[u8]) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, display(relative))
}

impl ResolvedLayers {
    /// Physical inputs in recorded layer order for plain metadata cache reuse.
    /// Archive members share their archive stamp; folder files keep their own.
    pub fn recorded_source_files(&self) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for layer in self.layers.iter().filter(|layer| layer.overlay.is_none()) {
            paths.extend(layer.descriptor.iter().cloned());
            match &layer.source {
                LayerSource::Archive(path) => paths.push(path.clone()),
                LayerSource::Folder(root) => {
                    for relative in layer.files.keys() {
                        let relative = std::str::from_utf8(relative)
                            .map_err(|_| invalid("Folder content path is not UTF-8"))?;
                        paths.push(root.join(relative));
                    }
                }
            }
        }
        Ok(paths)
    }

    /// Consumer caches belong to this resolved stack and are shared by its clones.
    /// A changed mount roster starts fresh; successful reads retain their diagnostics.
    pub fn derived<T: Default + Send + Sync + 'static>(&self) -> std::sync::Arc<T> {
        let roster: Vec<_> = self.layers.iter().map(|layer| layer.id.clone()).collect();
        let mut derived = self
            .derived
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if derived.roster != roster {
            derived.roster = roster;
            derived.values.clear();
        }
        std::sync::Arc::clone(
            derived
                .values
                .entry(std::any::TypeId::of::<T>())
                .or_insert_with(|| std::sync::Arc::new(T::default())),
        )
        .downcast::<T>()
        .expect("cache type owns its entry")
    }
    /// Whether this recording's coverage policy hashes a text path.
    pub fn covered(&self, path: &str) -> bool {
        self.covered_path(path.as_bytes())
    }

    /// [`Self::covered`] over a layer path's bytes.
    pub fn covered_path(&self, path: &[u8]) -> bool {
        self.coverage.covered_path(path)
    }

    /// The base layer alone, with this recording's coverage policy, for source audits.
    pub fn base_only(&self) -> io::Result<Self> {
        let layer = self
            .layers
            .first()
            .ok_or_else(|| invalid("Content source audit: no recorded base game layer"))?;
        let mut base = Self {
            coverage: self.coverage,
            layers: vec![layer.clone()],
            ..Self::default()
        };
        base.finish()?;
        Ok(base)
    }

    fn finish(&mut self) -> io::Result<()> {
        self.effective.clear();
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.coverage != self.coverage {
                return Err(invalid("content layers use different coverage policies"));
            }
            apply_overlay(
                &mut self.effective,
                index,
                matches!(layer.source, LayerSource::Archive(_)),
                layer.files.keys(),
                &layer.replace_paths,
                layer.overlay.is_some(),
            )?;
        }
        Ok(())
    }

    /// Reuse inventoried test layers while rebuilding the overlay and memos.
    #[cfg(test)]
    pub(crate) fn from_test_layers(layers: Vec<ResolvedLayer>) -> io::Result<Self> {
        let mut assembled = Self {
            layers,
            ..Self::default()
        };
        assembled.finish()?;
        Ok(assembled)
    }

    /// Syntax audits whose readers and key-name dependencies all use `common`.
    #[cfg(test)]
    pub(crate) fn from_test_common_folders(folders: &[(&Path, &[&str])]) -> io::Result<Self> {
        let (mut cache, mut stats) = (HashCache::default(), HashStats::default());
        let mut layers = Vec::new();
        for (index, (root, replace_paths)) in folders.iter().enumerate() {
            let root = fs::canonicalize(root)?;
            let mut layer = ResolvedLayer {
                coverage: CoveragePolicy::default(),
                id: format!("test{index}"),
                source: LayerSource::Folder(root.clone()),
                descriptor: None,
                replace_paths: replace_paths
                    .iter()
                    .map(|path| replace_path(path.as_bytes()))
                    .collect::<io::Result<_>>()?,
                files: Default::default(),
                lookup: Default::default(),
                hashes: Default::default(),
                archive_stamp: None,
                descriptor_hash: None,
                overlay: None,
            };
            let directory = root.join("common");
            if directory.try_exists()? {
                collect_folder(
                    &root,
                    &directory,
                    std::sync::Arc::make_mut(&mut layer.files),
                    layer.coverage,
                    false,
                )?;
            }
            layer.finish_inventory()?;
            layer.hash(&mut cache, &mut stats)?;
            layers.push(layer);
        }
        Self::from_test_layers(layers)
    }

    /// Folder layers for consumers' unit tests, in load order: each
    /// `(root, replace_paths)` is inventoried and hashed as `gather` does and
    /// overlaid on the ones before it, so reads verify and same-path files
    /// and `replace_path` directories replace as they do for real content;
    /// each replace path is normalized as a descriptor's is.
    /// This crate's tests and the opt-in `layer-fixtures`/`test-support`
    /// features build it; consumers enable fixtures from dev-dependencies.
    #[cfg(any(test, feature = "test-support", feature = "layer-fixtures"))]
    pub fn from_folders(folders: &[(&Path, &[&str])]) -> io::Result<Self> {
        Self::from_folders_with_coverage_policy(folders, COVERAGE_POLICY_VERSION)
    }

    /// [`Self::from_folders`] under a historical policy, for consumers' coverage tests.
    #[cfg(any(test, feature = "test-support", feature = "layer-fixtures"))]
    pub fn from_folders_with_coverage_policy(
        folders: &[(&Path, &[&str])],
        coverage_policy: u32,
    ) -> io::Result<Self> {
        let (mut cache, mut stats) = (HashCache::default(), HashStats::default());
        let mut layers = Self {
            coverage: CoveragePolicy::recorded(coverage_policy)?,
            ..Self::default()
        };
        for (index, (root, replace_paths)) in folders.iter().enumerate() {
            let mut layer = ResolvedLayer {
                coverage: layers.coverage,
                id: format!("test{index}"),
                source: if root.is_file()
                    && root
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
                {
                    LayerSource::Archive(root.to_path_buf())
                } else {
                    LayerSource::Folder(root.to_path_buf())
                },
                descriptor: None,
                replace_paths: replace_paths
                    .iter()
                    .map(|path| replace_path(path.as_bytes()))
                    .collect::<io::Result<_>>()?,
                files: Default::default(),
                lookup: Default::default(),
                hashes: Default::default(),
                archive_stamp: None,
                descriptor_hash: None,
                overlay: None,
            };
            layer.inventory()?;
            layer.hash(&mut cache, &mut stats)?;
            layers.layers.push(layer);
        }
        layers.finish()?;
        Ok(layers)
    }

    /// Every effective covered path that is text. A listing is text for the text databases
    /// that consume it; a path whose bytes are not UTF-8 is read by its bytes
    /// ([`Self::read_path`]) and is not listed here.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.effective
            .values()
            .filter_map(|file| std::str::from_utf8(&file.path).ok())
    }

    /// The `name` of every recorded DLC layer, which `has_dlc` tests: its
    /// descriptor is reread and must still hash as it did when verified.
    pub fn dlc_names(&self) -> io::Result<BTreeSet<String>> {
        let mut names = BTreeSet::new();
        for layer in &self.layers {
            let Some(descriptor) = layer.descriptor.as_ref().filter(|path| {
                path.extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("dlc"))
            }) else {
                continue;
            };
            let bytes = fs::read(descriptor)?;
            let actual: [u8; 32] = Sha256::digest(&bytes).into();
            if layer.descriptor_hash != Some(actual) {
                return Err(invalid(format!(
                    "DLC descriptor {} changed after content verification",
                    descriptor.display()
                )));
            }
            names.insert(required(&parser::parse(&bytes)?, b"name")?);
        }
        Ok(names)
    }

    /// Every distinct covered path under `prefix` that a visible layer lists,
    /// with its own case, as a PhysFS enumeration of the merged tree lists it:
    /// names that differ only in case stay separate entries, and each reads
    /// through [`ResolvedLayers::read_path`]. A `replace_path` hides the layers
    /// below it.
    ///
    /// The names are the layers' own bytes, as the game's enumeration returns
    /// them: an archive record's name (`physfs_zip::Entry::name`), whatever its bytes;
    /// a folder file's name as PhysFS's Windows enumeration converts it from
    /// UTF-16 (`PHYSFS_utf16ToUtf8`), always UTF-8. A folder path that is not
    /// UTF-8 is never listed, as none opens (the conversion described by [`Self::lookup_path`]
    /// makes each invalid byte `?`, which [`folder_file`] cannot open): a folder layer's
    /// index holds only its files' UTF-8 names.
    pub fn listed_layer_paths(&self, prefix: &[u8]) -> BTreeSet<LayerPath> {
        let prefix = prefix.to_ascii_lowercase();
        let mut listed = BTreeSet::new();
        let mut hidden: Vec<LayerPath> = Vec::new();
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

    /// [`Self::listed_layer_paths`] for the text databases: only the paths
    /// that are text. The GUI and GFX readers enumerate by bytes.
    pub fn listed_paths(&self, prefix: &str) -> BTreeSet<String> {
        self.listed_layer_paths(prefix.as_bytes())
            .into_iter()
            .filter_map(|path| String::from_utf8(path).ok())
            .collect()
    }

    /// Every effective file under `prefix` ending in one of `extensions`,
    /// covered or not, in (layer, path) order. Each layer's folder or archive is listed
    /// directly, so this works whatever the coverage policy includes, and
    /// `replace_path` and case precedence follow the covered overlay. Read the
    /// results with [`Self::read_uncovered`], which verifies covered paths;
    /// the listing seeds its lookup so each read opens one file.
    /// Text listing only: see [`Self::paths`].
    pub fn descriptor_paths(&self, prefix: &str, extensions: &[&str]) -> io::Result<Vec<String>> {
        let prefix = normalize_path(prefix.as_bytes())?.to_ascii_lowercase();
        let extensions: Vec<String> = extensions.iter().map(|e| e.to_ascii_lowercase()).collect();
        if extensions
            .iter()
            .any(|e| !e.starts_with('.') || e.len() < 2 || e.contains('/'))
        {
            return Err(invalid("descriptor extension must be a dot and a suffix"));
        }
        let mut effective = BTreeMap::new();
        let mut listings = Vec::with_capacity(self.layers.len());
        for (index, layer) in self.layers.iter().enumerate() {
            let listing = listed_files(index, layer, &prefix, &extensions)?;
            apply_overlay(
                &mut effective,
                index,
                matches!(layer.source, LayerSource::Archive(_)),
                listing.keys(),
                &layer.replace_paths,
                layer.overlay.is_some(),
            )?;
            listings.push(listing);
        }
        let mut files: Vec<_> = effective.into_values().collect();
        files.sort_by(|a, b| a.layer.cmp(&b.layer).then_with(|| a.path.cmp(&b.path)));
        let roster: Vec<String> = self.layers.iter().map(|layer| layer.id.clone()).collect();
        let mut memo = self
            .uncovered_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for file in &files {
            if !self.covered_path(&file.path) {
                let hit = listings[file.layer].get(&file.path).cloned();
                memo.insert(roster.clone(), file.path.clone(), hit);
            }
        }
        Ok(files
            .into_iter()
            .filter_map(|file| String::from_utf8(file.path).ok())
            .collect())
    }

    /// One directory's files as a database loader lists them, without recursion. Merged
    /// PhysFS entries are keyed by name in bytewise, case-sensitive order; names ending
    /// in the filter are emitted once each in that order; subdirectories are not entered.
    /// `suffix` must start with
    /// a dot (see [`Self::descriptor_paths`]); the engine's filter has none, so
    /// a name such as `footxt` is not listed here. Covered or not: read each
    /// with [`Self::read_uncovered`].
    pub fn directory_files(&self, directory: &str, suffix: &str) -> io::Result<Vec<String>> {
        let directory = normalize_relative(directory)?;
        let listed = self.descriptor_paths(&directory, &[suffix])?;
        Ok(directory_order(&directory, suffix, listed))
    }

    /// The engine's `gfx/models` texture index.
    pub fn texture_index(&self) -> io::Result<std::sync::Arc<crate::mesh::TextureIndex>> {
        self.memoized(&self.texture_index, crate::mesh::TextureIndex::read)
    }

    /// Every descriptor below `gfx`, as [`crate::mesh::Roster`] reads it.
    pub fn descriptor_roster(&self) -> io::Result<std::sync::Arc<crate::mesh::Roster>> {
        self.memoized(&self.descriptor_roster, crate::mesh::Roster::read)
    }

    /// The content names trigger bodies are read against
    /// ([`crate::key_names::KeyNames::read`]).
    pub fn key_names(&self) -> io::Result<std::sync::Arc<crate::key_names::KeyNames>> {
        self.memoized(&self.key_names, crate::key_names::KeyNames::read)
    }

    /// Faction templates read once per layer roster, including an unreadable result.
    pub fn faction_templates(
        &self,
    ) -> Result<std::sync::Arc<crate::faction_templates::Catalog>, String> {
        self.memoized(&self.faction_templates, |layers| {
            Ok(crate::faction_templates::read(layers)
                .map(std::sync::Arc::new)
                .map_err(|error| format!("Faction templates are unreadable: {error}")))
        })
        .expect("the template read result is cached even when reading fails")
        .as_ref()
        .clone()
    }

    /// Diagnostics from an already-used template reader; this never loads content.
    pub fn faction_template_diagnostics(&self) -> Vec<String> {
        let roster: Vec<String> = self.layers.iter().map(|layer| layer.id.clone()).collect();
        let memo = self
            .faction_templates
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match memo.as_ref() {
            Some((cached, result)) if *cached == roster => result
                .as_ref()
                .as_ref()
                .map(|templates| templates.warnings.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn memoized<T>(
        &self,
        memo: &Memo<T>,
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<std::sync::Arc<T>> {
        let roster: Vec<String> = self.layers.iter().map(|layer| layer.id.clone()).collect();
        let mut memo = memo.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached, value)) = memo.as_ref()
            && *cached == roster
        {
            return Ok(std::sync::Arc::clone(value));
        }
        let value = std::sync::Arc::new(build(self)?);
        *memo = Some((roster, std::sync::Arc::clone(&value)));
        Ok(value)
    }

    /// Whether the native lookup opens `relative` (folder metadata and archive opening
    /// headers, without reading the payload; loading still verifies the covered bytes).
    pub fn has_file(&self, relative: &str) -> io::Result<bool> {
        let Some(relative) = sanitize(relative.as_bytes()) else {
            return Ok(false);
        };
        if self.covered_path(&relative) {
            return Ok(self.lookup_path(&relative)?.is_some());
        }
        let roster: Vec<String> = self.layers.iter().map(|layer| layer.id.clone()).collect();
        let memo = self
            .uncovered_memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&roster, &relative);
        if let Some(hit) = memo {
            let answered = hit.as_ref().map(|hit| match hit {
                UncoveredHit::Folder(i, _) | UncoveredHit::Archive(i) => *i,
            });
            self.verify_passed(&relative, answered)?;
            match hit {
                None => return Ok(false),
                Some(UncoveredHit::Folder(index, path)) => {
                    if self.layers[index].folder_lookup(&path)?.is_some() {
                        return Ok(true);
                    }
                }
                Some(UncoveredHit::Archive(index)) => {
                    return self.layers[index].archive_exists(&relative);
                }
            }
        }
        let found = self.scan_uncovered_with(&relative, false)?;
        self.uncovered_memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                roster,
                relative,
                found.as_ref().map(|(_, hit, _)| hit.clone()),
            );
        Ok(found.is_some())
    }

    /// Load-order position of the layer an effective covered path comes from.
    pub fn layer_index(&self, relative: &str) -> Option<usize> {
        self.effective
            .get(&relative.as_bytes().to_ascii_lowercase())
            .map(|file| file.layer)
    }

    /// [`Self::read_path`] of a text path.
    pub fn read(&self, relative: &str) -> io::Result<Vec<u8>> {
        self.read_path(relative.as_bytes())
    }

    /// Retain only a prefix while verifying the complete covered payload. Folder
    /// assets stream through a fixed buffer; archive assets share their
    /// normal reader. The returned length is the verified payload's full length.
    pub fn read_prefix(&self, relative: &str, limit: usize) -> io::Result<(Vec<u8>, u64)> {
        let path = sanitize(relative.as_bytes()).ok_or_else(|| not_found(relative.as_bytes()))?;
        if self.covered_path(&path)
            && let Some(mut hit) = self.lookup_path(&path)?
            && let Some(mut file) = hit.folder.take()
        {
            self.verify_passed(&path, Some(hit.layer))?;
            let mut digest = Sha256::new();
            let mut prefix = Vec::new();
            let mut length = 0;
            let mut buffer = [0; 64 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
                length += count as u64;
                let kept = count.min(limit.saturating_sub(prefix.len()));
                prefix.extend_from_slice(&buffer[..kept]);
            }
            let digest: [u8; 32] = digest.finalize().into();
            self.layers[hit.layer].verify(&hit.path, &digest)?;
            self.note_overlay(hit.layer, &hit.path);
            return Ok((prefix, length));
        }
        let mut bytes = self.read(relative)?;
        let length = bytes.len() as u64;
        bytes.truncate(limit);
        Ok((bytes, length))
    }

    /// A file by its path's bytes, sanitized and resolved by [`Self::lookup_path`].
    /// Covered bytes are verified. Uncovered reads require a reporting consumer
    /// to retain the shared diagnostics from [`Self::report_uncovered_reads`].
    pub fn read_path(&self, relative: &[u8]) -> io::Result<Vec<u8>> {
        let Some(relative) = sanitize(relative) else {
            return Err(not_found(relative));
        };
        match self.lookup_path(&relative)? {
            Some(file) => {
                self.verify_passed(&relative, Some(file.layer))?;
                let layer = &self.layers[file.layer];
                let path = layer
                    .lookup
                    .archive
                    .as_ref()
                    .and_then(|archive| archive.mounted.lookup(&file.path))
                    .map_or(file.path.as_slice(), |(entry, _)| entry.name.as_slice());
                // A visual mod's bytes are reported as its own, never as recorded content.
                let uncovered = !self.covered_path(path) && layer.overlay.is_none();
                if uncovered
                    && self
                        .uncovered_reads
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .is_none()
                {
                    return Err(invalid(format!(
                        "content file is outside recorded coverage policy {} and this reader does not report uncovered reads: {}",
                        self.coverage.0,
                        display(path)
                    )));
                }
                let bytes = match file.folder {
                    Some(handle) => layer.folder_read(&file.path, handle),
                    None => layer
                        .archive_open(&file.path)?
                        .ok_or_else(|| changed(&relative)),
                }?;
                self.note_overlay(file.layer, path);
                if uncovered
                    && let Some(paths) = self
                        .uncovered_reads
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .as_mut()
                {
                    paths.insert(path.to_vec());
                }
                return Ok(bytes);
            }
            None => {
                self.verify_passed(&relative, None)?;
            }
        }
        Err(not_found(&relative))
    }

    /// Opt into ordinary uncovered reads and retain this handle at the reporting
    /// boundary. Layer clones and lazy readers contribute to the same live set.
    /// Explicit `read_uncovered` attempts also retain missing candidates.
    #[must_use]
    pub fn report_uncovered_reads(&self) -> UncoveredReads {
        self.uncovered_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_or_insert_with(BTreeSet::new);
        self.read_warnings()
    }

    /// Current diagnostics without enabling uncovered reads for a strict reader.
    pub fn read_warnings(&self) -> UncoveredReads {
        UncoveredReads {
            policy: self.coverage.0,
            paths: self.uncovered_reads.clone(),
            overlays: self.overlay_reads.clone(),
        }
    }

    /// Counts a file a visual mod supplied, by the mod's name.
    fn note_overlay(&self, layer: usize, path: &[u8]) {
        if let Some(overlay) = &self.layers[layer].overlay {
            self.overlay_reads
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .reads
                .entry(overlay.name.clone())
                .or_default()
                .insert(path.to_vec());
        }
    }

    /// Artwork as `load` reads it from the [presentation stack](Self::presentation). When a
    /// visual mod is mounted and the artwork cannot be loaded from it (a format the reader
    /// refuses, or a lower file its `replace_path` hides), the recorded artwork is loaded instead
    /// and `name` is reported with why ([`UncoveredReads::overlay_refusals`]); an error loading the
    /// recorded artwork is returned as it would be without visual mods.
    pub fn artwork<T>(
        &self,
        name: &str,
        load: impl Fn(&ResolvedLayers) -> io::Result<T>,
    ) -> io::Result<T> {
        let presentation = self.presentation();
        if std::ptr::eq(presentation, self) {
            return load(self);
        }
        load(presentation).or_else(|error| {
            let recorded = load(self)?;
            self.overlay_reads
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .refusals
                .insert(format!("{name} ({error})"));
            Ok(recorded)
        })
    }

    /// The stack artwork is read from: these recorded layers with the selected visual mods
    /// mounted above every recorded mod, in the engine's order among themselves
    /// ([`visual::apply`]). Without a selection it is this stack. Map geometry, simulation and
    /// every recorded fact keep reading this stack itself; only consumers that draw artwork read
    /// the presentation stack, whose own derived caches start fresh.
    pub fn presentation(&self) -> &ResolvedLayers {
        self.presentation.as_deref().unwrap_or(self)
    }

    /// The presentation stack: `overlays` (in mount order, lowest first) inserted above the
    /// recorded mods and below the built-in layers the engine mounts above every mod. The
    /// recorded layers keep their order and verification and share their inventories; the
    /// presentation stack shares this stack's read diagnostics and has its own lookup memos and
    /// derived caches. When the whole selection cannot be mounted together, each overlay is
    /// added in order and only those that cannot be are left out, with why.
    fn with_overlays(
        &self,
        overlays: Vec<ResolvedLayer>,
    ) -> (Option<ResolvedLayers>, Vec<(ResolvedLayer, io::Error)>) {
        if let Ok(presentation) = self.mounted(overlays.clone()) {
            return (Some(presentation), Vec::new());
        }
        let (mut kept, mut refused) = (Vec::new(), Vec::new());
        for overlay in overlays {
            let mut trial = kept.clone();
            trial.push(overlay.clone());
            match self.mounted(trial) {
                Ok(_) => kept.push(overlay),
                Err(error) => refused.push((overlay, error)),
            }
        }
        let presentation = (!kept.is_empty())
            .then(|| self.mounted(kept).ok())
            .flatten();
        (presentation, refused)
    }

    fn mounted(&self, overlays: Vec<ResolvedLayer>) -> io::Result<ResolvedLayers> {
        let mut layers = self.layers.clone();
        let at = layers.len().saturating_sub(self.above_mods);
        layers.splice(at..at, overlays);
        let mut presentation = ResolvedLayers {
            coverage: self.coverage,
            layers,
            above_mods: self.above_mods,
            presentation: None,
            overlay_reads: self.overlay_reads.clone(),
            uncovered_reads: self.uncovered_reads.clone(),
            shader_warnings: self.shader_warnings.clone(),
            ..ResolvedLayers::default()
        };
        presentation.finish()?;
        Ok(presentation)
    }

    /// Diagnostics from lazily read artwork effects, shared by this resolved stack.
    pub fn shader_warning_lines(&self) -> Vec<String> {
        self.shader_warnings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub fn note_shader_warning(&self, warning: String) {
        self.shader_warnings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(warning);
    }

    /// The one native lookup, without payload reads.
    /// Folders use Windows aliases; a non-UTF-8 request opens none: each invalid byte becomes
    /// `?`, which [`folder_file`] (`CreateFileW`) cannot open.
    fn lookup_path(&self, relative: &[u8]) -> io::Result<Option<RecordedHit>> {
        let lower = relative.to_ascii_lowercase();
        // Resolve each layer's native open: a higher folder's Windows alias can name a
        // file the ASCII-folded effective inventory does not match. ZIP lookup stays indexed.
        // A case-missing archive, directory or failed open passes to the next layer
        // under PhysFS lookup rules; `replace_path` stops the scan below its layer.
        for (index, layer) in self.layers.iter().enumerate().rev() {
            let hit = match &layer.source {
                LayerSource::Folder(_) => {
                    layer
                        .folder_lookup(relative)?
                        .map(|(path, handle)| RecordedHit {
                            layer: index,
                            path,
                            folder: Some(handle),
                        })
                }
                LayerSource::Archive(_) => layer
                    .lookup
                    .archive
                    .as_ref()
                    .is_some_and(|archive| archive.opens(relative))
                    .then(|| RecordedHit {
                        layer: index,
                        path: relative.to_vec(),
                        folder: None,
                    }),
            };
            if let Some(hit) = hit {
                return Ok(Some(hit));
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

    /// Reads a path outside the coverage policy, such as `gfx/world` or
    /// `gfx/mapitems`, from the resolved layers.
    ///
    /// The returned bytes are **not** hash-verified: the layer hash does not
    /// cover them, so a changed file cannot be detected here and the viewer
    /// must state that limitation. A covered path is passed straight to
    /// [`ResolvedLayers::read`], which does verify it. A folder alias whose real
    /// path is covered is verified too, on every read including memoized hits.
    pub fn read_uncovered(&self, relative: &str) -> io::Result<Vec<u8>> {
        self.read_uncovered_path(relative.as_bytes())
    }

    /// [`Self::read_uncovered`] by a path's bytes, looked up as [`Self::read_path`] looks up.
    pub fn read_uncovered_path(&self, relative: &[u8]) -> io::Result<Vec<u8>> {
        let Some(relative) = sanitize(relative) else {
            return Err(not_found(relative));
        };
        if self.covered_path(&relative) {
            return self.read_path(&relative);
        }
        // A file a visual mod answered is counted as that mod's; every other attempt, missing
        // or failed included, is an uncovered recorded read.
        let answer = self.uncovered_answer(&relative);
        match &answer {
            Ok((_, layer)) if self.layers[*layer].overlay.is_some() => {
                self.note_overlay(*layer, &relative)
            }
            _ => {
                if let Some(paths) = self
                    .uncovered_reads
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_mut()
                {
                    paths.insert(relative.clone());
                }
            }
        }
        answer.map(|(bytes, _)| bytes)
    }

    /// [`Self::read_uncovered_path`]'s lookup of an uncovered path: its bytes and the layer that
    /// answered.
    fn uncovered_answer(&self, relative: &[u8]) -> io::Result<(Vec<u8>, usize)> {
        let relative = relative.to_vec();
        let roster: Vec<String> = self.layers.iter().map(|layer| layer.id.clone()).collect();
        if let Some(memo) = self
            .uncovered_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&roster, &relative)
        {
            // Every memoized answer passes the verification the scan applies to the layers it
            // passes; a stale hit falls back to a fresh scan below, which refreshes the memo.
            match memo {
                Some(hit) => {
                    let (UncoveredHit::Folder(layer, _) | UncoveredHit::Archive(layer)) = hit;
                    self.verify_passed(&relative, Some(layer))?;
                    if let Some(bytes) = self.read_hit(&hit, &relative)? {
                        return Ok((bytes, layer));
                    }
                }
                None => {
                    self.verify_passed(&relative, None)?;
                    return Err(not_found(&relative));
                }
            }
        }
        let found = self.scan_uncovered(&relative)?;
        self.uncovered_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                roster,
                relative.clone(),
                found.as_ref().map(|(_, hit, _)| hit.clone()),
            );
        match found {
            Some((layer, _, bytes)) => Ok((bytes, layer)),
            None => Err(not_found(&relative)),
        }
    }

    /// The layer scan behind [`ResolvedLayers::read_uncovered`], returning the
    /// hit alongside its bytes: folders use their Windows open and real-path verification,
    /// archive lookup is exact (or a `$` password, [`physfs_zip::Mounted::lookup`]), a layer
    /// whose open fails is passed as [`Self::read_path`] passes it, hard errors abort, and
    /// `replace_path` stops the scan.
    fn scan_uncovered(
        &self,
        relative: &[u8],
    ) -> io::Result<Option<(usize, UncoveredHit, Vec<u8>)>> {
        self.scan_uncovered_with(relative, true)
    }

    fn scan_uncovered_with(
        &self,
        relative: &[u8],
        load: bool,
    ) -> io::Result<Option<(usize, UncoveredHit, Vec<u8>)>> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            match &layer.source {
                // A path that is not UTF-8 opens no folder file (see `read_path`).
                LayerSource::Folder(_) => {
                    if let Some((real, handle)) = layer.folder_lookup(relative)? {
                        let bytes = if load {
                            layer.folder_read(&real, handle)?
                        } else {
                            Vec::new()
                        };
                        return Ok(Some((index, UncoveredHit::Folder(index, real), bytes)));
                    }
                }
                // ZIP lookup is exact, by the record's name (or up to a `$`), through the
                // layer's one verified lookup, as it is for covered paths.
                LayerSource::Archive(_) => {
                    let opened = if load {
                        layer.archive_open(relative)?
                    } else {
                        layer.archive_exists(relative)?.then(Vec::new)
                    };
                    if let Some(bytes) = opened {
                        return Ok(Some((index, UncoveredHit::Archive(index), bytes)));
                    }
                }
            }
            if !load {
                layer.verify_miss(relative)?;
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
    /// DLC, mod, `replace_path` or visual mod in its place.
    pub fn uncovered_from_base(&self, relative: &str) -> io::Result<bool> {
        let Some(relative) = sanitize(relative.as_bytes()) else {
            return Ok(false);
        };
        Ok(matches!(self.scan_uncovered(&relative)?, Some((0, _, _))))
    }

    /// Reads a memoized hit of `relative`: a folder file by its real spelling (verified if covered),
    /// an archive layer through its one verified lookup ([`ResolvedLayer::archive_open`]).
    /// `Ok(None)` means the hit is stale and the caller rescans; `Err` is a hard error, which
    /// aborts exactly like the scan.
    fn read_hit(&self, hit: &UncoveredHit, relative: &[u8]) -> io::Result<Option<Vec<u8>>> {
        match hit {
            UncoveredHit::Folder(layer, path) => match self.layers.get(*layer) {
                Some(layer) if matches!(layer.source, LayerSource::Folder(_)) => {
                    Ok(layer.folder_open(path)?.map(|(_, bytes)| bytes))
                }
                _ => Ok(None),
            },
            UncoveredHit::Archive(layer) => match self.layers.get(*layer) {
                Some(layer) if matches!(layer.source, LayerSource::Archive(_)) => {
                    layer.archive_open(relative)
                }
                // The layer at this index is no longer an archive: stale.
                _ => Ok(None),
            },
        }
    }

    /// Verifies, as the scan finds them answering nothing, the layers a lookup of `relative`
    /// passes before the one that `answered` it (every layer the scan reaches, down to a
    /// `replace_path`, for a miss): the check every memoized answer goes through, so it
    /// never skips a layer the scan would verify
    /// ([`ResolvedLayer::verify_miss`]).
    fn verify_passed(&self, relative: &[u8], answered: Option<usize>) -> io::Result<()> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            if Some(index) == answered {
                break;
            }
            layer.verify_miss(relative)?;
            if layer
                .replace_paths
                .iter()
                .any(|path| below(&lower, &path.to_ascii_lowercase()))
            {
                break;
            }
        }
        Ok(())
    }
}

impl UncoveredMemo {
    fn get(&self, roster: &[String], relative: &[u8]) -> Option<Option<UncoveredHit>> {
        (self.layers == roster)
            .then(|| self.hits.get(relative).cloned())
            .flatten()
    }

    fn insert(&mut self, roster: Vec<String>, relative: LayerPath, hit: Option<UncoveredHit>) {
        if self.layers != roster {
            self.layers = roster;
            self.hits.clear();
        }
        self.hits.insert(relative, hit);
    }
}

/// The Windows PhysFS read open: ordinary CreateFileW,
/// GENERIC_READ, share read/write, OPEN_EXISTING. Read and identify through the same handle.
#[cfg(windows)]
fn folder_file(root: &Path, relative: &str) -> io::Result<(File, PathBuf)> {
    use std::{
        ffi::OsString,
        os::windows::{
            ffi::OsStringExt,
            io::{AsRawHandle, FromRawHandle},
        },
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
            GetFinalPathNameByHandleW, OPEN_EXISTING,
        },
    };

    let path = root.join(relative);
    let text = text_path(&path)?;
    // A canonicalized root has Rust's verbatim prefix; PhysFS uses ordinary Win32 names.
    let ordinary = match text.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(text).to_owned(),
    };
    let name: Vec<_> = ordinary.encode_utf16().chain([0]).collect();
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        return Err(if path.is_dir() {
            not_found(relative.as_bytes())
        } else {
            error
        });
    }
    // The successful handle is owned and closed by File, including on identity failures.
    let file = unsafe { File::from_raw_handle(handle) };
    let mut name = vec![0; 260];
    loop {
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                name.as_mut_ptr(),
                name.len() as u32,
                0,
            )
        } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length < name.len() {
            return Ok((file, PathBuf::from(OsString::from_wide(&name[..length]))));
        }
        name.resize(length + 1, 0);
    }
}

#[cfg(not(windows))]
fn folder_file(root: &Path, relative: &str) -> io::Result<(File, PathBuf)> {
    let path = root.join(relative);
    let (file, path) = match File::open(&path) {
        Ok(file) => (file, path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let path =
                folded_path(root, relative)?.ok_or_else(|| not_found(relative.as_bytes()))?;
            (File::open(&path)?, path)
        }
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(not_found(relative.as_bytes()));
    }
    Ok((file, fs::canonicalize(path)?))
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

/// One layer's files below a lower-case `prefix` with a lower-case suffix,
/// with the layer's own case and where each is read from. Links and
/// case-ambiguous names are refused.
fn listed_files(
    index: usize,
    layer: &ResolvedLayer,
    prefix: &[u8],
    extensions: &[String],
) -> io::Result<BTreeMap<LayerPath, UncoveredHit>> {
    let mut paths = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let overlay = layer.overlay.is_some();
    let mut admit = |relative: LayerPath, hit: UncoveredHit| -> io::Result<()> {
        let lower = relative.to_ascii_lowercase();
        if below(&lower, prefix) && extensions.iter().any(|e| lower.ends_with(e.as_bytes())) {
            // A visual mod's case variants are listed as the game lists them (`collect_folder`).
            if !folded.insert(lower) && !overlay {
                return Err(invalid(format!(
                    "layer holds case-ambiguous paths: {}",
                    display(&relative)
                )));
            }
            paths.insert(relative, hit);
        }
        Ok(())
    };
    match &layer.source {
        LayerSource::Folder(root) => {
            let Some(directory) = folded_path(root, folder_text(prefix)?)? else {
                return Ok(paths);
            };
            if !directory.is_dir() {
                return Ok(paths);
            }
            let mut pending = vec![directory];
            while let Some(directory) = pending.pop() {
                for entry in directory_entries(&directory)? {
                    let kind = entry.file_type()?;
                    if kind.is_symlink() {
                        if overlay {
                            continue;
                        }
                        return Err(invalid(format!(
                            "content layer contains a symbolic link: {}",
                            entry.path().display()
                        )));
                    }
                    if kind.is_dir() {
                        pending.push(entry.path());
                    } else if kind.is_file() {
                        let path = entry.path();
                        let relative = normalize_path(
                            text_path(
                                path.strip_prefix(root)
                                    .map_err(|_| invalid("layer path escaped root"))?,
                            )?
                            .as_bytes(),
                        )?;
                        admit(relative.clone(), UncoveredHit::Folder(index, relative))?;
                    }
                }
            }
        }
        LayerSource::Archive(_) => {
            for entry in layer.verified_archive()?.1.entries {
                if !entry.directory {
                    admit(entry.name, UncoveredHit::Archive(index))?;
                }
            }
        }
    }
    Ok(paths)
}

fn apply_overlay<'a>(
    effective: &mut BTreeMap<LayerPath, EffectiveFile>,
    index: usize,
    archive: bool,
    paths: impl Iterator<Item = &'a LayerPath>,
    replace_paths: &[LayerPath],
    overlay: bool,
) -> io::Result<()> {
    for replaced in replace_paths {
        effective.retain(|path, _| !below(path, &replaced.to_ascii_lowercase()));
    }
    for path in paths {
        // Lookups resolve each layer natively; this index only lists. A recorded stack refuses
        // what verification cannot order, while a visual mod's spelling replaces the entry.
        if !overlay
            && effective
                .get(&path.to_ascii_lowercase())
                .is_some_and(|old| old.path != *path && (old.archive || archive))
        {
            return Err(invalid(format!(
                "folder and archive layers have case-dependent path precedence: {}",
                display(path)
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
    #[serde(skip)]
    dirty: bool,
}

impl HashCache {
    fn get(&self, key: &str, stamp: &Stamp) -> Option<[u8; 32]> {
        self.entries
            .get(key)
            .filter(|entry| &entry.stamp == stamp)
            .map(|entry| entry.hash)
    }

    fn insert(&mut self, key: String, stamp: Stamp, hash: [u8; 32]) {
        if self
            .entries
            .get(&key)
            .is_some_and(|entry| entry.stamp == stamp && entry.hash == hash)
        {
            return;
        }
        self.entries.insert(key, CachedHash { stamp, hash });
        self.dirty = true;
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

/// Enabled mods, each with its requested descriptor path and parsed descriptor,
/// in the order the engine mounts them, lowest priority first. The launcher's
/// list order is not used: each descriptor whose path equals a requested path byte for byte
/// starts at weight 10000 (DLC 0). Until nothing changes, an entry whose display name another
/// entry's `dependencies` lists loses one weight while it is not below that entry's. Every
/// dependencies block contributes names; a dependency that is not enabled changes nothing.
/// Entries sort by weight, then descriptor path bytes. Each mount is prepended, so later
/// layers win. The collector's live mount check confirms the result for every recording.
fn native_mod_order<T>(
    selected: Vec<(String, T, Document)>,
    notes: &mut Vec<String>,
) -> io::Result<Vec<(String, T, Document)>> {
    const MOD_WEIGHT: i64 = 10_000;
    let mut names = Vec::new();
    let mut dependencies = Vec::new();
    for (path, _, document) in &selected {
        let name = required(document, b"name").map_err(|error| named_error("Mod", path, error))?;
        let mut listed = Vec::new();
        for value in document.get_all(b"dependencies") {
            let items = value
                .items()
                .ok_or_else(|| invalid(format!("Mod {path}: dependencies is not a list")))?;
            for item in items {
                match (&item.key, item.value.scalar_str()) {
                    (None, Some(dependency)) => listed.push(dependency.to_owned()),
                    _ => {
                        return Err(invalid(format!(
                            "Mod {path}: dependencies lists something other than a name"
                        )));
                    }
                }
            }
        }
        names.push(name);
        dependencies.push(listed);
    }
    let mut by_name = BTreeMap::<&str, Vec<usize>>::new();
    for (index, name) in names.iter().enumerate() {
        by_name.entry(name).or_default().push(index);
    }
    // The engine keys descriptors by display name and keeps the first,
    // so it mounts only one of a same-named set.
    for (name, indices) in &by_name {
        if indices.len() > 1 {
            notes.push(format!(
                "enabled mods {} share the display name {name:?}; the engine mounts only one",
                indices
                    .iter()
                    .map(|&index| selected[index].0.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let count = i64::try_from(selected.len()).map_err(|_| invalid("too many enabled mods"))?;
    let mut weights = vec![MOD_WEIGHT; selected.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for (dependent, listed) in dependencies.iter().enumerate() {
            let weight = weights[dependent];
            for dependency in listed {
                for &index in by_name.get(dependency.as_str()).into_iter().flatten() {
                    if weight <= weights[index] {
                        weights[index] -= 1;
                        changed = true;
                    }
                }
            }
        }
        // Acyclic dependencies settle above 10000 - count, still above the
        // DLC; the engine never finishes a cycle.
        if let Some(index) = weights
            .iter()
            .position(|&weight| weight <= (MOD_WEIGHT - count).max(0))
        {
            return Err(invalid(format!(
                "Mod {}: dependency cycle or chain the engine cannot order",
                selected[index].0
            )));
        }
    }
    let mut weighted = weights.into_iter().zip(selected).collect::<Vec<_>>();
    weighted.sort_by(|(left, (left_path, ..)), (right, (right_path, ..))| {
        left.cmp(right)
            .then_with(|| left_path.as_bytes().cmp(right_path.as_bytes()))
    });
    Ok(weighted.into_iter().map(|(_, entry)| entry).collect())
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
    coverage: CoveragePolicy,
) -> io::Result<ResolvedLayer> {
    let descriptor_hash = descriptor
        .as_ref()
        .map(|path| fs::read(path).map(|bytes| Sha256::digest(bytes).into()))
        .transpose()?;
    Ok(ResolvedLayer {
        coverage,
        id,
        source,
        descriptor,
        replace_paths: Vec::new(),
        files: Default::default(),
        lookup: Default::default(),
        hashes: Default::default(),
        archive_stamp: None,
        descriptor_hash,
        overlay: None,
    })
}

fn append_builtin(
    mut layer: ResolvedLayer,
    prefix: &str,
    vanilla_files: &mut BTreeMap<LayerPath, [u8; 32]>,
    layers: &mut ResolvedLayers,
    cache: &mut HashCache,
    stats: &mut HashStats,
) -> io::Result<()> {
    layer.inventory()?;
    layer.hash(cache, stats)?;
    // Built-in virtual roots belong to the vanilla install identity. Keep their
    // physical install-relative namespace, even when virtual file names overlap.
    for (path, digest) in layer.hashes.iter() {
        vanilla_files.insert([prefix.as_bytes(), b"/", path].concat(), *digest);
    }
    if let Some(descriptor) = &layer.descriptor {
        let name = descriptor
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("built-in descriptor has no Unicode name"))?;
        vanilla_files.insert(
            format!("{prefix}/{name}").into_bytes(),
            layer.descriptor_hash.unwrap(),
        );
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
    CoveragePolicy::recorded(identity.coverage_policy)?;
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
    let coverage = recorded
        .map(|identity| CoveragePolicy::recorded(identity.coverage_policy))
        .transpose()?
        .unwrap_or_default();
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
        .unwrap_or_else(|| HashCache {
            dirty: true,
            ..HashCache::default()
        });
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
            coverage,
            id: "vanilla".into(),
            source: LayerSource::Folder(game_dir.clone()),
            descriptor: None,
            replace_paths: Vec::new(),
            files: Default::default(),
            lookup: Default::default(),
            hashes: Default::default(),
            archive_stamp: None,
            descriptor_hash: None,
            overlay: None,
        };
        vanilla.inventory()?;
        vanilla.hash(&mut cache, &mut stats)?;
        let mut vanilla_files = (*vanilla.hashes).clone();
        let mut layers = ResolvedLayers {
            coverage,
            layers: vec![vanilla],
            effective: BTreeMap::new(),
            uncovered_memo: Default::default(),
            uncovered_reads: Default::default(),
            shader_warnings: Default::default(),
            texture_index: Default::default(),
            descriptor_roster: Default::default(),
            key_names: Default::default(),
            faction_templates: Default::default(),
            derived: Default::default(),
            above_mods: 0,
            presentation: None,
            overlay_reads: Default::default(),
        };
        let mut order_notes = Vec::new();
        if recorded.is_none() {
            let mut user_layer = builtin_layer(
                "user directory".into(),
                LayerSource::Folder(user_dir.clone()),
                None,
                coverage,
            )?;
            user_layer.inventory()?;
            if !user_layer.files.is_empty() {
                order_notes.push(format!("The game's mounted user directory contains {} covered files outside the declared content layers", user_layer.files.len()));
                user_layer.hash(&mut cache, &mut stats)?;
            }
            layers.layers.push(user_layer);
        }
        // Startup mounts integrated descriptor parent folders before
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
                    coverage,
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
            let (mut layer, document) =
                descriptor_layer(id.clone(), path, &game_dir, false, coverage)
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
        // Native ordering follows `native_mod_order`: dependency weight, then descriptor
        // path bytes. All installed vanilla DLC have
        // weight zero and no dependencies; PHYSFS mount prepends each sorted layer.
        if has_dependencies && options.dlc_order.is_none() {
            order_notes.push(
                "DLC dependencies can change mount order; observed mount order is required"
                    .to_owned(),
            );
        }
        let mut selected = Vec::new();
        for (index, descriptor) in launcher.enabled_mods.into_iter().enumerate() {
            let descriptor = normalize_relative(&descriptor)?;
            let path = user_dir.join(&descriptor);
            let requested_id = recorded
                .map(|identity| identity.mods[index].id.as_str())
                .unwrap_or(&descriptor);
            let (layer, document) =
                descriptor_layer(String::new(), path, &user_dir, true, coverage)
                    .map_err(|error| named_error("Mod", requested_id, error))?;
            selected.push((descriptor, layer, document));
        }
        // A replay keeps its recorded order; a new recording takes the engine's.
        if recorded.is_none() {
            selected = native_mod_order(selected, &mut order_notes)?;
        }
        let order_note = (!order_notes.is_empty()).then(|| order_notes.join("; "));
        let mut mods = Vec::new();
        let mut mod_ids = BTreeSet::new();
        let mut mod_descriptors = Vec::new();
        for (index, (descriptor, mut layer, document)) in selected.into_iter().enumerate() {
            let id = mod_id(&document, Path::new(&descriptor))?;
            let file = Path::new(&descriptor)
                .file_name()
                .ok_or_else(|| invalid(format!("Mod {id}: descriptor has no file name")))?;
            mod_descriptors.push(text_path(Path::new(file))?.to_owned());
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
        // The engine mounts the optional tweaker assets last, preferring
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
            let layer = builtin_layer(prefix.into(), source, None, coverage)?;
            append_builtin(
                layer,
                prefix,
                &mut vanilla_files,
                &mut layers,
                &mut cache,
                &mut stats,
            )?;
            layers.above_mods += 1;
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
            mod_descriptors,
        })
    })();
    // Preserve successfully hashed files even when a later descriptor is bad.
    // A valid, unchanged cache needs no serialization or write. Missing or
    // unreadable caches are rebuilt and remain disposable after interruptions.
    if cache.dirty {
        fs::write(cache_path, serde_json::to_vec(&cache)?)?;
    }
    result.map(|mut content: GatheredContent| {
        content.stats.elapsed_ms = started.elapsed().as_millis();
        content
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hash_cache_tests;
