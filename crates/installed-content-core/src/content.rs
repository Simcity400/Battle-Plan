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
pub use crate::script_files::ScriptPath;

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
    /// Recorded layers whose installed bytes differ: the installed bytes are read (owner, 28 Oct).
    pub changed: Vec<ChangedLayer>,
}

impl ResolvedContent {
    /// This content when every layer is the recording's own, or the first changed layer's refusal.
    pub fn exact(self) -> io::Result<Self> {
        match self.changed.first() {
            Some(changed) => Err(changed.refusal()),
            None => Ok(self),
        }
    }

    /// Refresh `warnings` with one line naming each changed layer.
    pub fn append_changed_to(&self, warnings: &mut Vec<String>) {
        warnings.retain(|line| !line.ends_with(CHANGED_LAYER));
        warnings.extend(self.changed.iter().map(ToString::to_string));
    }
}

const CHANGED_LAYER: &str =
    "changed since the recording (content hash mismatch); the installed files are used";

/// A recorded content layer whose installed bytes differ from the recording's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedLayer {
    /// "Game" (the executable or the vanilla files), "DLC" or "Mod".
    pub kind: &'static str,
    pub id: String,
    pub recorded: [u8; 32],
    pub installed: [u8; 32],
}

impl ChangedLayer {
    /// What a reader that needs the recorded bytes refuses with.
    pub fn refusal(&self) -> io::Error {
        invalid(format!(
            "{} {}: content hash mismatch (expected {}, found {})",
            self.kind,
            self.id,
            hex(self.recorded),
            hex(self.installed)
        ))
    }
}

impl std::fmt::Display for ChangedLayer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} {} {CHANGED_LAYER}", self.kind, self.id)
    }
}

fn hex(hash: [u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
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
/// without case folding (see [`ResolvedLayer::archive_open_observed`]). Folder paths are their file names'
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
    /// What consumers refused to draw, by stable name, whichever stack they read
    /// ([`ResolvedLayers::note_refused`]).
    refused: BTreeSet<String>,
}

/// The folders and files a visual mod's files show under ([`ResolvedLayers::overlay_sees`]): the
/// artwork folders, and the world normal map in each form the game reads it.
const PRESENTATION_ROOTS: [&str; 7] = [
    "gfx",
    "interface",
    "localisation",
    "map/terrain",
    "map/world_normal.dds",
    "map/world_normal.tga",
    "map/world_normal.bmp",
];
/// The defines scripts a presentation defines stack also shows.
const DEFINES_ROOT: &str = "common/defines";

/// [`ResolvedLayers::overlay_sees`] for a stack that does or does not show mods' defines.
fn overlay_sees(defines_visible: bool, lower: &[u8]) -> bool {
    PRESENTATION_ROOTS
        .iter()
        .any(|root| below(lower, root.as_bytes()))
        || (defines_visible && below(lower, DEFINES_ROOT.as_bytes()))
}

/// The start of the lines [`UncoveredReads::append_to`] keeps current for visual mod reads.
const OVERLAY_READS: &str = "Artwork from visual mods";

/// The line naming and counting `refusals`: artwork the selected visual mods could not supply,
/// drawn in the recording's own style instead ([`ResolvedLayers::note_overlay_refusal`]). The
/// map reports its content's refusals with it and each widget the refusals its own reply
/// fell back on ([`observe_overlay_refusals`]), so both read alike.
pub fn overlay_refusal_line(refusals: &BTreeSet<String>) -> Option<String> {
    (!refusals.is_empty()).then(|| {
        format!(
            "{OVERLAY_READS} not shown: {} file(s) could not be loaded, so the recorded artwork is drawn: {}.",
            refusals.len(),
            refusals.iter().cloned().collect::<Vec<_>>().join("; ")
        )
    })
}

thread_local! {
    /// What each [`observe_overlay_refusals`] active on this thread has seen, innermost last.
    static OVERLAY_OBSERVERS: std::cell::RefCell<Vec<BTreeSet<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The first observer refusals reach: the innermost [`observe_overlay_refusals_isolated`]
    /// one, whose notes reach no observer outside it.
    static OVERLAY_ISOLATED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Whether artwork loaders on this thread draw without falling back ([`strict`]).
    static STRICT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `run` with every artwork loader drawing strictly: what the selected visual mods cannot
/// supply fails rather than falling back to the recording ([`ResolvedLayers::artwork`],
/// [`ResolvedLayers::graphics_define`], [`ResolvedLayers::artwork_item`] and the interface
/// texture loader honour it). A caller that redraws a whole operation from the recording when
/// it fails uses it, so the recording's own choice is made again rather than one stage of the
/// mods' choice patched.
pub fn strict<T>(run: impl FnOnce() -> T) -> T {
    struct Exit(bool);
    impl Drop for Exit {
        fn drop(&mut self) {
            STRICT.with(|strict| strict.set(self.0));
        }
    }
    let _exit = Exit(STRICT.with(|strict| strict.replace(true)));
    run()
}

/// Whether artwork loaders on this thread draw strictly ([`strict`]).
pub fn is_strict() -> bool {
    STRICT.with(std::cell::Cell::get)
}

/// Whether artwork loaders on this thread may fall back to the recording: what every artwork
/// memo keeps results apart by, so a strict draw ([`strict`]) never reads or fills a result a
/// fallback-permitted draw made, nor the other way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtworkMode {
    #[default]
    Ordinary,
    Strict,
}

/// The mode artwork loaders on this thread draw in.
pub fn artwork_mode() -> ArtworkMode {
    if is_strict() {
        ArtworkMode::Strict
    } else {
        ArtworkMode::Ordinary
    }
}

/// A memo kept once per [`ArtworkMode`]: each use reads and fills the current mode's.
#[derive(Default)]
pub struct ByMode<T> {
    ordinary: T,
    strict: T,
}

impl<T> ByMode<T> {
    /// The current mode's ([`artwork_mode`]).
    pub fn current(&self) -> &T {
        match artwork_mode() {
            ArtworkMode::Ordinary => &self.ordinary,
            ArtworkMode::Strict => &self.strict,
        }
    }

    /// The fallback-permitted one, which every ordinary read uses.
    pub fn ordinary(&self) -> &T {
        &self.ordinary
    }
}

/// Runs `run` and returns, with its value, every refusal [`ResolvedLayers::note_overlay_refusal`]
/// noted on this thread meanwhile, whichever stack noted it: what `run` drew from the recording
/// because the selected visual mods could not supply it. Observers nest; each sees what is
/// noted inside it. A consumer that shares one stack with others reports its own fallbacks
/// with it rather than the stack's whole tally.
pub fn observe_overlay_refusals<T>(run: impl FnOnce() -> T) -> (T, BTreeSet<String>) {
    // Pops the observer when `run` returns or unwinds.
    struct Exit<'a>(&'a mut BTreeSet<String>);
    impl Drop for Exit<'_> {
        fn drop(&mut self) {
            if let Some(seen) = OVERLAY_OBSERVERS.with(|observers| observers.borrow_mut().pop()) {
                *self.0 = seen;
            }
        }
    }
    OVERLAY_OBSERVERS.with(|observers| observers.borrow_mut().push(BTreeSet::new()));
    let mut seen = BTreeSet::new();
    let value = {
        let _exit = Exit(&mut seen);
        run()
    };
    (value, seen)
}

/// [`observe_overlay_refusals`] keeping what is noted inside to itself: no observer outside it
/// sees it, so a caller can publish it ([`credit_overlay_refusals`]) or drop it.
pub fn observe_overlay_refusals_isolated<T>(run: impl FnOnce() -> T) -> (T, BTreeSet<String>) {
    struct Exit(usize);
    impl Drop for Exit {
        fn drop(&mut self) {
            OVERLAY_ISOLATED.with(|isolated| isolated.set(self.0));
        }
    }
    let depth = OVERLAY_OBSERVERS.with(|observers| observers.borrow().len());
    let _exit = Exit(OVERLAY_ISOLATED.with(|isolated| isolated.replace(depth)));
    observe_overlay_refusals(run)
}

/// Adds `refusals` to every observer active on this thread, as if noted here: a memo of
/// artwork loaded once replays what its load fell back on to each later user. Inside an
/// isolated observer ([`observe_overlay_refusals_isolated`]) it reaches that observer and those
/// within it only.
pub fn credit_overlay_refusals(refusals: &BTreeSet<String>) {
    if refusals.is_empty() {
        return;
    }
    let from = OVERLAY_ISOLATED.with(std::cell::Cell::get);
    OVERLAY_OBSERVERS.with(|observers| {
        for seen in observers.borrow_mut().iter_mut().skip(from) {
            seen.extend(refusals.iter().cloned());
        }
    });
}

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

    /// Everything consumers of this content refused to draw, by stable name
    /// ([`ResolvedLayers::note_refused`]).
    pub fn refused(&self) -> BTreeSet<String> {
        self.overlays
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .refused
            .clone()
    }

    /// The artwork that could not be loaded from visual mods, which the recording's replaced.
    pub fn overlay_refusals(&self) -> Option<String> {
        overlay_refusal_line(
            &self
                .overlays
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .refusals,
        )
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadPayload {
    pub length: u64,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    Bytes {
        length: u64,
        digest: [u8; 32],
    },
    Probe(bool),
    Error {
        kind: String,
        message: String,
        payload: Option<ReadPayload>,
    },
}

#[derive(Clone, Debug)]
pub struct ContentRead {
    pub path: LayerPath,
    pub purpose: String,
    pub layer: Option<usize>,
    pub outcome: ReadOutcome,
}

#[derive(Clone)]
struct ReadTrace {
    callback: std::sync::Arc<dyn Fn(ContentRead) + Send + Sync>,
    purpose: String,
}

#[derive(Debug)]
enum ReadSuccess<T> {
    Untraced(T),
    Traced { value: T, payload: ReadPayload },
}
impl<T> ReadSuccess<T> {
    fn into_value(self) -> T {
        match self {
            Self::Untraced(value) | Self::Traced { value, .. } => value,
        }
    }

    fn map<U>(self, map: impl FnOnce(T) -> U) -> ReadSuccess<U> {
        match self {
            Self::Untraced(value) => ReadSuccess::Untraced(map(value)),
            Self::Traced { value, payload } => ReadSuccess::Traced {
                value: map(value),
                payload,
            },
        }
    }

    fn verified(self, result: io::Result<()>) -> ObservedResult<Self> {
        match result {
            Ok(()) => Ok(self),
            Err(error) => Err(ReadFailure {
                error,
                payload: match self {
                    Self::Untraced(_) => None,
                    Self::Traced { payload, .. } => Some(payload),
                },
            }),
        }
    }
}

#[derive(Debug)]
struct ReadFailure {
    error: io::Error,
    payload: Option<ReadPayload>,
}
type ObservedResult<T> = Result<T, ReadFailure>;
impl From<io::Error> for ReadFailure {
    fn from(error: io::Error) -> Self {
        Self {
            error,
            payload: None,
        }
    }
}
impl ReadFailure {
    fn outcome(&self) -> ReadOutcome {
        ReadOutcome::Error {
            kind: format!("{:?}", self.error.kind()),
            message: self.error.to_string(),
            payload: self.payload.clone(),
        }
    }

    fn context(mut self, context: impl FnOnce(io::Error) -> io::Error) -> Self {
        self.error = context(self.error);
        self
    }
}

#[derive(Default)]
struct ReadEvidence {
    length: u64,
    digest: Sha256,
}
impl ReadEvidence {
    fn update(&mut self, bytes: &[u8]) {
        self.length += bytes.len() as u64;
        self.digest.update(bytes);
    }

    fn finish(self) -> ReadPayload {
        ReadPayload {
            length: self.length,
            digest: self.digest.finalize().into(),
        }
    }
}

struct EvidenceReader<'a, R> {
    reader: &'a mut R,
    evidence: &'a mut ReadEvidence,
}
impl<R: Read> Read for EvidenceReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.reader.read(bytes)?;
        self.evidence.update(&bytes[..count]);
        Ok(count)
    }
}

#[derive(Default)]
struct ReadObservation {
    enabled: bool,
    layer: Option<usize>,
}
impl ReadObservation {
    fn at(&mut self, layer: usize) {
        self.layer = Some(layer);
    }

    fn bytes<T>(&self, value: T, bytes: &[u8]) -> ReadSuccess<T> {
        if self.enabled {
            self.hashed(value, bytes.len() as u64, payload_digest(bytes))
        } else {
            ReadSuccess::Untraced(value)
        }
    }

    fn hashed<T>(&self, value: T, length: u64, digest: [u8; 32]) -> ReadSuccess<T> {
        if self.enabled {
            ReadSuccess::Traced {
                value,
                payload: ReadPayload { length, digest },
            }
        } else {
            ReadSuccess::Untraced(value)
        }
    }
}

fn payload_digest(bytes: &[u8]) -> [u8; 32] {
    #[cfg(test)]
    tests::PAYLOAD_HASHES.with(|hashes| hashes.set(hashes.get() + 1));
    Sha256::digest(bytes).into()
}
impl std::fmt::Debug for ReadTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadTrace")
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedLayers {
    read_trace: Option<ReadTrace>,
    coverage: CoveragePolicy,
    pub layers: Vec<ResolvedLayer>,
    /// Built-in layers the engine mounts above every mod (the tweaker assets): visual mods
    /// mount below them, as an enabled mod would.
    above_mods: usize,
    /// The same recorded layers with the selected visual mods mounted above them, or `None`
    /// when no visual mod is selected ([`Self::presentation`]).
    presentation: Option<std::sync::Arc<ResolvedLayers>>,
    /// The presentation stack whose visual mods also show their defines scripts
    /// ([`Self::presentation_defines`]).
    presentation_defines: Option<std::sync::Arc<ResolvedLayers>>,
    /// Whether a visual mod's `common/defines` shows through ([`Self::overlay_sees`]).
    defines_visible: bool,
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
    /// The recorded stack's roster below this presentation stack's visual mods, built once
    /// per roster ([`Self::recorded_descriptor_roster`]).
    recorded_descriptor_roster: Memo<crate::mesh::Roster>,
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
/// through its one verified lookup ([`ResolvedLayer::archive_open_observed`]).
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
    /// The recorded layers whose installed bytes differ; empty for a new recording.
    pub changed: Vec<ChangedLayer>,
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
    record_digest_observed(archive, entry, &mut ReadObservation::default())
        .map(ReadSuccess::into_value)
        .map_err(|failure| failure.error)
}

fn record_digest_observed(
    archive: &mut physfs_zip::Archive,
    entry: &physfs_zip::Entry,
    observed: &ReadObservation,
) -> ObservedResult<ReadSuccess<([u8; 32], Option<Vec<u8>>, u64)>> {
    #[cfg(test)]
    tests::LOADS.with(|loads| loads.set(loads.get() + 1));
    let result = match archive.read_observed(entry, None, observed.enabled)? {
        Some(bytes) => {
            let digest = payload_digest(&bytes);
            observed.hashed((digest, Some(bytes), entry.size), entry.size, digest)
        }
        None => {
            let stored = archive.stored_observed(entry, observed.enabled)?;
            let mut hash = Sha256::new();
            hash.update(b"\0unopened archive record\0");
            hash.update(entry.opening());
            hash.update(&stored);
            let length = stored.len() as u64;
            observed.bytes((hash.finalize().into(), None, length), &stored)
        }
    };
    Ok(result)
}

/// The files of `listed` directly in `directory` whose name ends in `suffix`
/// (case-sensitive), in the byte order of their
/// names. See [`ResolvedLayers::directory_files`].
fn directory_order(directory: &[u8], suffix: &str, listed: Vec<ScriptPath>) -> Vec<ScriptPath> {
    let mut end = directory.len();
    while end > 0 && directory[end - 1] == b'/' {
        end -= 1;
    }
    let directory = &directory[..end];
    let mut files: Vec<(LayerPath, ScriptPath)> = listed
        .into_iter()
        .filter_map(|path| {
            let bytes = path.as_bytes();
            let split = bytes.iter().rposition(|byte| *byte == b'/')?;
            let (parent, name) = (&bytes[..split], bytes[split + 1..].to_vec());
            (parent.eq_ignore_ascii_case(directory) && name.ends_with(suffix.as_bytes()))
                .then_some((name, path))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
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
                    if let Some(path) = folded_path(root, directory)? {
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
                let paths = self
                    .files
                    .keys()
                    .map(|relative| Ok(root.join(folder_text(relative)?)))
                    .collect::<io::Result<Vec<_>>>()?;
                // Files before the first failing one keep their digests, as
                // hashing them one by one would.
                for (relative, probe) in self.files.keys().zip(cache.probe_all(&paths)) {
                    hashes.insert(relative.clone(), cache.settle(probe?, stats));
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
    fn folder_read(
        &self,
        real: &[u8],
        file: impl Read,
        observed: &ReadObservation,
    ) -> ObservedResult<ReadSuccess<Vec<u8>>> {
        #[cfg(test)]
        tests::LOADS.with(|loads| loads.set(loads.get() + 1));
        self.folder_read_from(real, file, observed)
    }

    fn folder_read_from(
        &self,
        real: &[u8],
        mut file: impl Read,
        observed: &ReadObservation,
    ) -> ObservedResult<ReadSuccess<Vec<u8>>> {
        let mut bytes = Vec::new();
        if let Err(error) = file.read_to_end(&mut bytes) {
            return Err(ReadFailure {
                error,
                payload: observed.enabled.then(|| ReadPayload {
                    length: bytes.len() as u64,
                    digest: payload_digest(&bytes),
                }),
            });
        }
        if self.coverage.covered_path(real) {
            let length = bytes.len() as u64;
            let digest = payload_digest(&bytes);
            observed
                .hashed(bytes, length, digest)
                .verified(self.verify(real, &digest))
        } else {
            let result = observed.bytes((), &bytes);
            Ok(result.map(|()| bytes))
        }
    }

    fn folder_open(
        &self,
        relative: &[u8],
        observed: &ReadObservation,
    ) -> ObservedResult<Option<(LayerPath, ReadSuccess<Vec<u8>>)>> {
        let Some((real, file)) = self.folder_lookup(relative)? else {
            return Ok(None);
        };
        let bytes = self.folder_read(&real, file, observed)?;
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
    /// ([`Self::archive_open_observed`]).
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
    #[cfg(test)]
    fn archive_open(
        &self,
        request: &[u8],
        observed: &mut ReadObservation,
    ) -> io::Result<Option<Vec<u8>>> {
        self.archive_open_observed(request, observed)
            .map(|bytes| bytes.map(ReadSuccess::into_value))
            .map_err(|failure| failure.error)
    }

    fn archive_open_observed(
        &self,
        request: &[u8],
        observed: &ReadObservation,
    ) -> ObservedResult<Option<ReadSuccess<Vec<u8>>>> {
        let (mut archive, mounted) = self.verified_archive()?;
        let Some((entry, password)) = mounted.lookup(request) else {
            return Ok(None);
        };
        let context = |error: io::Error| invalid(format!("{}: {error}", display(request)));
        if self.verifies() && self.files.contains_key(&entry.name) {
            let record = record_digest_observed(&mut archive, entry, observed)
                .map_err(|failure| failure.context(context))?;
            let digest = match &record {
                ReadSuccess::Untraced((digest, _, _))
                | ReadSuccess::Traced {
                    value: (digest, _, _),
                    ..
                } => *digest,
            };
            let record = record.verified(self.verify(&entry.name, &digest))?;
            if password.is_none() {
                return Ok(match record {
                    ReadSuccess::Untraced((_, bytes, _)) => bytes.map(ReadSuccess::Untraced),
                    ReadSuccess::Traced {
                        value: (_, bytes, _),
                        payload,
                    } => bytes.map(|value| ReadSuccess::Traced { value, payload }),
                });
            }
        }
        let bytes = archive
            .read_observed(entry, password, observed.enabled)
            .map_err(|failure| failure.context(context))?;
        Ok(bytes.map(|bytes| observed.bytes((), &bytes).map(|()| bytes)))
    }

    fn archive_exists(&self, request: &[u8]) -> io::Result<bool> {
        let (mut archive, mounted) = self.verified_archive()?;
        match mounted.lookup(request) {
            Some((entry, password)) => archive.opens(entry, password),
            None => Ok(false),
        }
    }

    /// A lookup this layer answers nothing to, verified as one it answered would be: an archive
    /// layer's by its one lookup ([`Self::archive_open_observed`], which must still answer nothing), a
    /// folder layer's by no covered file having appeared under the request's Windows aliases.
    /// An uncovered folder file is not verified, as the uncovered reads say.
    fn verify_miss(&self, relative: &[u8], observed: &mut ReadObservation) -> ObservedResult<()> {
        if !self.verifies() {
            return Ok(());
        }
        match &self.source {
            LayerSource::Archive(_) => match self.archive_open_observed(relative, observed)? {
                Some(bytes) => bytes.verified(Err(changed(relative))).map(|_| ()),
                None => Ok(()),
            },
            LayerSource::Folder(_) => {
                if let Some((real, _)) = self.folder_lookup(relative)?
                    && self.coverage.covered_path(&real)
                {
                    return Err(changed(&real).into());
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
    pub fn tracing_reads(&self) -> bool {
        self.read_trace.is_some()
    }

    /// Starts a fresh tracing scope whose callback and caches support parallel retained readers.
    pub fn with_read_trace(
        &self,
        trace: std::sync::Arc<dyn Fn(ContentRead) + Send + Sync>,
    ) -> Self {
        Self {
            read_trace: Some(ReadTrace {
                callback: trace,
                purpose: String::new(),
            }),
            derived: Default::default(),
            key_names: Default::default(),
            texture_index: Default::default(),
            descriptor_roster: Default::default(),
            recorded_descriptor_roster: Default::default(),
            faction_templates: Default::default(),
            ..self.clone()
        }
    }

    /// Isolates a named reader's lazy caches so scheduling cannot change their traced purpose.
    pub fn with_read_purpose(&self, purpose: impl Into<String>) -> Self {
        let Some(trace) = &self.read_trace else {
            return self.clone();
        };
        let mut layers = self.with_read_trace(trace.callback.clone());
        layers.read_trace.as_mut().expect("traced clone").purpose = purpose.into();
        layers
    }

    fn observation(&self) -> ReadObservation {
        ReadObservation {
            enabled: self.tracing_reads(),
            ..Default::default()
        }
    }

    fn finish_read<T>(
        &self,
        path: &[u8],
        observed: ReadObservation,
        result: ObservedResult<ReadSuccess<T>>,
    ) -> io::Result<T> {
        match result {
            Ok(ReadSuccess::Untraced(value)) => Ok(value),
            Ok(ReadSuccess::Traced { value, payload }) => {
                self.trace_read(
                    path,
                    observed.layer,
                    ReadOutcome::Bytes {
                        length: payload.length,
                        digest: payload.digest,
                    },
                );
                Ok(value)
            }
            Err(failure) => {
                if self.tracing_reads() {
                    self.trace_read(path, observed.layer, failure.outcome());
                }
                Err(failure.error)
            }
        }
    }

    fn trace_read(&self, path: &[u8], layer: Option<usize>, outcome: ReadOutcome) {
        if let Some(trace) = &self.read_trace {
            (trace.callback)(ContentRead {
                path: path.to_vec(),
                purpose: trace.purpose.clone(),
                layer,
                outcome,
            });
        }
    }

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
            let defines_visible = self.defines_visible;
            let sees = |lower: &[u8]| overlay_sees(defines_visible, lower);
            let mut effective = std::mem::take(&mut self.effective);
            apply_overlay(
                &mut effective,
                index,
                matches!(layer.source, LayerSource::Archive(_)),
                layer.files.keys(),
                &layer.replace_paths,
                layer.overlay.is_some().then_some(&sees as Sees<'_>),
            )?;
            self.effective = effective;
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

    /// The game loads non-UTF-8 archive paths too; read these with [`Self::read_path`].
    pub fn paths(&self) -> impl Iterator<Item = &[u8]> {
        self.effective.values().map(|file| file.path.as_slice())
    }

    /// Paths retain non-UTF-8 bytes; read them with [`Self::read_path`].
    pub fn effective_paths(&self) -> impl Iterator<Item = (&[u8], usize)> {
        self.effective
            .values()
            .map(|file| (file.path.as_slice(), file.layer))
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
        // Each replaced root, and whether a visual mod replaced it (that hides only what the
        // mod shows, `Self::overlay_sees`).
        let mut hidden: Vec<(LayerPath, bool)> = Vec::new();
        for layer in self.layers.iter().rev() {
            for path in layer.files.keys() {
                let lower = path.to_ascii_lowercase();
                if lower.starts_with(&prefix)
                    && !self.hides(layer, &lower)
                    && !hidden.iter().any(|(root, overlay)| {
                        below(&lower, root) && (!overlay || self.overlay_sees(&lower))
                    })
                {
                    listed.insert(path.clone());
                }
            }
            hidden.extend(
                layer
                    .replace_paths
                    .iter()
                    .map(|path| (path.to_ascii_lowercase(), layer.overlay.is_some())),
            );
        }
        listed
    }

    #[cfg(test)]
    pub(crate) fn listed_paths(&self, prefix: &str) -> BTreeSet<String> {
        self.listed_layer_paths(prefix.as_bytes())
            .into_iter()
            .filter_map(|path| String::from_utf8(path).ok())
            .collect()
    }

    /// Listing is byte-exact; lookup ignores folder case but requires exact archive names.
    pub fn descriptor_paths(
        &self,
        prefix: impl AsRef<[u8]>,
        extensions: &[&str],
    ) -> io::Result<Vec<ScriptPath>> {
        let prefix = normalize_path(prefix.as_ref())?.to_ascii_lowercase();
        let extensions: Vec<String> = extensions.iter().map(|e| e.to_ascii_lowercase()).collect();
        if extensions
            .iter()
            .any(|e| !e.starts_with('.') || e.len() < 2 || e.contains('/'))
        {
            return Err(invalid("descriptor extension must be a dot and a suffix"));
        }
        let mut listings = Vec::with_capacity(self.layers.len());
        for (index, layer) in self.layers.iter().enumerate() {
            listings.push(listed_files(index, layer, &prefix, &extensions)?);
        }
        // Folder lookup ignores case and retains the inventory's first spelling.
        let folded: Vec<BTreeMap<LayerPath, &UncoveredHit>> = self
            .layers
            .iter()
            .zip(&listings)
            .map(|(layer, listing)| {
                let mut folded = BTreeMap::new();
                if matches!(layer.source, LayerSource::Folder(_)) {
                    for (path, hit) in listing {
                        folded.entry(path.to_ascii_lowercase()).or_insert(hit);
                    }
                }
                folded
            })
            .collect();
        let answer = |name: &LayerPath| -> Option<(usize, &UncoveredHit)> {
            let lower = name.to_ascii_lowercase();
            for (index, layer) in self.layers.iter().enumerate().rev() {
                if self.hides(layer, &lower) {
                    continue;
                }
                let hit = match layer.source {
                    LayerSource::Folder(_) => folded[index].get(&lower).copied(),
                    LayerSource::Archive(_) => listings[index].get(name),
                };
                if let Some(hit) = hit {
                    return Some((index, hit));
                }
                if layer
                    .replace_paths
                    .iter()
                    .any(|path| below(&lower, &path.to_ascii_lowercase()))
                {
                    return None;
                }
            }
            None
        };
        let mut seen = BTreeSet::new();
        let mut hidden: Vec<(LayerPath, bool)> = Vec::new();
        let mut files: Vec<(usize, LayerPath, UncoveredHit)> = Vec::new();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            for path in listings[index].keys() {
                let lower = path.to_ascii_lowercase();
                if self.hides(layer, &lower)
                    || hidden.iter().any(|(root, overlay)| {
                        below(&lower, root) && (!overlay || self.overlay_sees(&lower))
                    })
                    || !seen.insert(path.clone())
                {
                    continue;
                }
                let (answering, hit) = answer(path).ok_or_else(|| {
                    invalid(format!("listed file {} answers no lookup", display(path)))
                })?;
                files.push((answering, path.clone(), hit.clone()));
            }
            hidden.extend(
                layer
                    .replace_paths
                    .iter()
                    .map(|path| (path.to_ascii_lowercase(), layer.overlay.is_some())),
            );
        }
        files.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let roster: Vec<String> = self.layers.iter().map(|layer| layer.id.clone()).collect();
        let mut memo = self
            .uncovered_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (_, path, hit) in &files {
            if !self.covered_path(path) {
                memo.insert(roster.clone(), path.clone(), Some(hit.clone()));
            }
        }
        Ok(files
            .into_iter()
            .map(|(_, path, _)| ScriptPath::new(path))
            .collect())
    }

    /// One directory's files as a database loader lists them, without recursion. Merged
    /// PhysFS entries are keyed by name in bytewise, case-sensitive order; names ending
    /// in the filter are emitted once each in that order; subdirectories are not entered.
    /// `suffix` must start with
    /// a dot (see [`Self::descriptor_paths`]); the engine's filter has none, so
    /// a name such as `footxt` is not listed here. Covered or not: read each
    /// with [`ScriptPath::read_uncovered`]. Names are their bytes.
    pub fn directory_files(
        &self,
        directory: impl AsRef<[u8]>,
        suffix: &str,
    ) -> io::Result<Vec<ScriptPath>> {
        let directory = normalize_path(directory.as_ref())?;
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

    /// The descriptor roster of the recorded stack below this stack's visual mods, built once
    /// per roster and shared by every model library; `None` without visual mods.
    pub fn recorded_descriptor_roster(
        &self,
    ) -> io::Result<Option<std::sync::Arc<crate::mesh::Roster>>> {
        let Some(recorded) = self.without_overlays() else {
            return Ok(None);
        };
        self.memoized(&self.recorded_descriptor_roster, |_| {
            crate::mesh::Roster::read(&recorded)
        })
        .map(Some)
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
        let mut observed = self.observation();
        let result = self.has_file_untraced(relative, &mut observed);
        if self.tracing_reads() {
            let outcome = match &result {
                Ok(value) => ReadOutcome::Probe(*value),
                Err(error) => error.outcome(),
            };
            self.trace_read(relative.as_bytes(), observed.layer, outcome);
        }
        result.map_err(|failure| failure.error)
    }

    fn has_file_untraced(
        &self,
        relative: &str,
        observed: &mut ReadObservation,
    ) -> ObservedResult<bool> {
        let Some(relative) = sanitize(relative.as_bytes()) else {
            return Ok(false);
        };
        if self.covered_path(&relative) {
            let hit = self.lookup_path(&relative)?;
            observed.layer = hit.as_ref().map(|hit| hit.layer);
            return Ok(hit.is_some());
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
            self.verify_passed(&relative, answered, observed)?;
            match hit {
                None => {
                    observed.layer = None;
                    return Ok(false);
                }
                Some(UncoveredHit::Folder(index, path)) => {
                    if self.layers[index].folder_lookup(&path)?.is_some() {
                        observed.layer = Some(index);
                        return Ok(true);
                    }
                }
                Some(UncoveredHit::Archive(index)) => {
                    let present = self.layers[index].archive_exists(&relative)?;
                    observed.layer = present.then_some(index);
                    return Ok(present);
                }
            }
        }
        let found = self.scan_uncovered_with(&relative, false, observed)?;
        observed.layer = found.as_ref().map(|(_, hit, _)| match hit {
            UncoveredHit::Folder(index, _) | UncoveredHit::Archive(index) => *index,
        });
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
        let mut observed = self.observation();
        let result = self.read_prefix_observed(relative, limit, &mut observed);
        self.finish_read(relative.as_bytes(), observed, result)
    }

    fn read_prefix_observed(
        &self,
        relative: &str,
        limit: usize,
        observed: &mut ReadObservation,
    ) -> ObservedResult<ReadSuccess<(Vec<u8>, u64)>> {
        let path = sanitize(relative.as_bytes()).ok_or_else(|| not_found(relative.as_bytes()))?;
        if self.covered_path(&path)
            && let Some(mut hit) = self.lookup_path(&path)?
            && let Some(mut file) = hit.folder.take()
        {
            self.verify_passed(&path, Some(hit.layer), observed)?;
            observed.at(hit.layer);
            let mut digest = Sha256::new();
            let mut prefix = Vec::new();
            let mut length = 0;
            let mut buffer = [0; 64 * 1024];
            loop {
                let count = match file.read(&mut buffer) {
                    Ok(count) => count,
                    Err(error) => {
                        return Err(ReadFailure {
                            error,
                            payload: observed.enabled.then(|| ReadPayload {
                                length,
                                digest: digest.finalize().into(),
                            }),
                        });
                    }
                };
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
                length += count as u64;
                let kept = count.min(limit.saturating_sub(prefix.len()));
                prefix.extend_from_slice(&buffer[..kept]);
            }
            let digest: [u8; 32] = digest.finalize().into();
            let result = observed
                .hashed((prefix, length), length, digest)
                .verified(self.layers[hit.layer].verify(&hit.path, &digest))?;
            self.note_overlay(hit.layer, &hit.path);
            return Ok(result);
        }
        self.read_path_untraced(relative.as_bytes(), observed)
            .map(|bytes| {
                bytes.map(|mut bytes| {
                    let length = bytes.len() as u64;
                    bytes.truncate(limit);
                    (bytes, length)
                })
            })
    }

    /// A file by its path's bytes, sanitized and resolved by [`Self::lookup_path`].
    /// Covered bytes are verified. Uncovered reads require a reporting consumer
    /// to retain the shared diagnostics from [`Self::report_uncovered_reads`].
    pub fn read_path(&self, relative: &[u8]) -> io::Result<Vec<u8>> {
        let mut observed = self.observation();
        let result = self.read_path_untraced(relative, &mut observed);
        self.finish_read(relative, observed, result)
    }

    fn read_path_untraced(
        &self,
        relative: &[u8],
        observed: &mut ReadObservation,
    ) -> ObservedResult<ReadSuccess<Vec<u8>>> {
        let Some(relative) = sanitize(relative) else {
            return Err(not_found(relative).into());
        };
        match self.lookup_path(&relative)? {
            Some(file) => {
                observed.layer = Some(file.layer);
                self.verify_passed(&relative, Some(file.layer), observed)?;
                observed.at(file.layer);
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
                    )).into());
                }
                let bytes = match file.folder {
                    Some(handle) => layer.folder_read(&file.path, handle, observed),
                    None => layer
                        .archive_open_observed(&file.path, observed)?
                        .ok_or_else(|| ReadFailure::from(changed(&relative))),
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
                self.verify_passed(&relative, None, observed)?;
                observed.layer = None;
            }
        }
        Err(not_found(&relative).into())
    }

    /// Every path read as [`Self::read_path`] reads it, each result in its path's place. The
    /// reads are independent (each resolves, verifies and reports as it would alone), so they
    /// run on the machine's threads: a stack's every lookup probes each layer above the one
    /// that answers, and a bulk reader spends its time in those probes.
    pub fn read_paths<P: AsRef<[u8]> + Sync>(&self, paths: &[P]) -> Vec<io::Result<Vec<u8>>> {
        crate::parallel::map(paths, |path| self.read_path(path.as_ref()))
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

    /// Records that a consumer refused to draw `key`, a stable name of what (a model entity, a
    /// city building), whichever stack it read: a map compares what it refuses with and without
    /// visual mods by these names.
    pub fn note_refused(&self, key: String) {
        self.overlay_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .refused
            .insert(key);
    }

    /// Everything consumers of this content refused to draw ([`Self::note_refused`]).
    pub fn refused(&self) -> BTreeSet<String> {
        self.overlay_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .refused
            .clone()
    }

    /// Names artwork a visual mod supplies that is drawn from the recording instead, with why
    /// ([`UncoveredReads::overlay_refusals`]).
    pub fn note_overlay_refusal(&self, refusal: String) {
        credit_overlay_refusals(&BTreeSet::from([refusal.clone()]));
        self.overlay_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .refusals
            .insert(refusal);
    }

    /// Artwork as `load` reads it from the [presentation stack](Self::presentation). When a
    /// visual mod is mounted and the artwork cannot be loaded from it (a format the reader
    /// refuses, or a lower file its `replace_path` hides), the recorded artwork is loaded instead
    /// and `name` is reported with why ([`UncoveredReads::overlay_refusals`]); an error loading the
    /// recorded artwork is returned as it would be without visual mods.
    ///
    /// Called on a presentation stack itself, it falls back to the recorded layers below its
    /// visual mods; on the recorded stack, to that stack without its presentation. Either way
    /// the fallback reads nothing a visual mod supplies.
    pub fn artwork<T>(
        &self,
        name: &str,
        mut load: impl FnMut(&ResolvedLayers) -> io::Result<T>,
    ) -> io::Result<T> {
        let presentation = self.presentation();
        if !presentation.shows_visual_mods() {
            return load(self);
        }
        load(presentation).or_else(|error| {
            if is_strict() {
                return Err(error);
            }
            let recorded = if std::ptr::eq(presentation, self) {
                self.without_overlays()
                    .ok_or_else(|| invalid("visual mods without recorded layers"))?
            } else {
                self.recorded_only()
            };
            let loaded = load(&recorded)?;
            self.note_overlay_refusal(format!("{name} ({error})"));
            Ok(loaded)
        })
    }

    /// A value an artwork loader reads from the graphics defines (the visual mods' scripts
    /// merged with the recorded ones), or, when they cannot give it, from the `recorded`
    /// defines, named with why like [`Self::artwork`]. Without visual mods' defines both are
    /// the recorded defines and it is read once. Simulation operands never come from here.
    pub fn graphics_define<T, E: std::fmt::Display>(
        &self,
        name: &str,
        graphics: &crate::defines::Defines,
        recorded: &crate::defines::Defines,
        read: impl Fn(&crate::defines::Defines) -> Result<T, E>,
    ) -> Result<T, E> {
        if std::ptr::eq(graphics, recorded) || is_strict() {
            return read(graphics);
        }
        read(graphics).or_else(|error| {
            let value = read(recorded)?;
            self.note_overlay_refusal(format!("graphics defines for the {name} ({error})"));
            Ok(value)
        })
    }

    /// One item of artwork a loader reads apart from the rest (a tree kind, an arrow texture, an
    /// aircraft rig), as `load` reads it from the stack it names: `false` the stack the loader
    /// was handed, `true` the recorded layers below its visual mods, which the caller holds
    /// when `below`. An item the shown stack cannot give is read from the recorded layers,
    /// named with why like [`Self::artwork`]; one neither gives is refused by its stable `key`
    /// for the map's fallback ([`Self::note_refused`]) and its shown error returned.
    pub fn artwork_item<T, E: std::fmt::Display>(
        &self,
        key: &str,
        below: bool,
        mut load: impl FnMut(bool) -> Result<T, E>,
    ) -> Result<T, E> {
        let error = match load(false) {
            Ok(item) => return Ok(item),
            Err(error) => error,
        };
        if is_strict() {
            return Err(error);
        }
        if below && let Ok(item) = load(true) {
            self.note_overlay_refusal(format!("{key} ({error})"));
            return Ok(item);
        }
        self.note_refused(key.to_owned());
        Err(error)
    }

    /// Whether the file at `relative` comes from a visual mod: the layer a read answers it
    /// from is one of the mods mounted above the recording. Consumers tag what they draw with
    /// it, so only failures of a visual mod's own content count against the mods.
    pub fn supplied_by_visual_mod(&self, relative: impl AsRef<[u8]>) -> bool {
        if !self.shows_visual_mods() {
            return false;
        }
        let Some(relative) = sanitize(relative.as_ref()) else {
            return false;
        };
        let layer = if self.covered_path(&relative) {
            self.lookup_path(&relative)
                .ok()
                .flatten()
                .map(|hit| hit.layer)
        } else {
            self.uncovered_answer(&relative, &mut ReadObservation::default())
                .ok()
                .map(|(_, layer)| layer)
        };
        layer.is_some_and(|layer| self.layers[layer].overlay.is_some())
    }

    /// Whether this stack mounts visual mods (a presentation stack).
    pub fn shows_visual_mods(&self) -> bool {
        self.layers.iter().any(|layer| layer.overlay.is_some())
    }

    /// This stack sharing every inventory, memo and cache, but whose visual mod diagnostics
    /// (files they supplied, artwork drawn from the recording instead, refusals) start empty and
    /// are its own: a presentation [`visual::apply`] mounts on it reports apart from the stack it
    /// came from and from any other presentation of it.
    pub fn with_own_overlay_diagnostics(&self) -> ResolvedLayers {
        ResolvedLayers {
            overlay_reads: OverlayReads::default(),
            ..self.clone()
        }
    }

    /// This recorded stack without its presentation: what is drawn when no visual mod is.
    /// Its caches are shared with this stack.
    pub fn recorded_only(&self) -> ResolvedLayers {
        ResolvedLayers {
            presentation: None,
            presentation_defines: None,
            ..self.clone()
        }
    }

    /// The stack artwork is read from: these recorded layers with the selected visual mods
    /// mounted above every recorded mod, in the engine's order among themselves
    /// ([`visual::apply`]). Without a selection it is this stack. Map geometry, simulation and
    /// every recorded fact keep reading this stack itself; only consumers that draw artwork read
    /// the presentation stack, whose own derived caches start fresh.
    pub fn presentation(&self) -> &ResolvedLayers {
        self.presentation.as_deref().unwrap_or(self)
    }

    /// The presentation stack graphics defines are evaluated over: the visual mods'
    /// `common/defines` scripts are merged natively with the recorded ones, in the game's own
    /// file order. `None` without visual mods. Simulation consumers keep the recorded defines.
    pub fn presentation_defines(&self) -> Option<&ResolvedLayers> {
        self.presentation_defines.as_deref()
    }

    /// The recorded stack below a presentation stack's visual mods, sharing their inventories,
    /// or `None` when no visual mod is mounted here. Artwork readers fall back to it.
    pub fn without_overlays(&self) -> Option<ResolvedLayers> {
        if self.layers.iter().all(|layer| layer.overlay.is_none()) {
            return None;
        }
        let mut recorded = ResolvedLayers {
            coverage: self.coverage,
            layers: self
                .layers
                .iter()
                .filter(|layer| layer.overlay.is_none())
                .cloned()
                .collect(),
            above_mods: self.above_mods,
            overlay_reads: self.overlay_reads.clone(),
            uncovered_reads: self.uncovered_reads.clone(),
            shader_warnings: self.shader_warnings.clone(),
            ..ResolvedLayers::default()
        };
        recorded.finish().ok()?;
        Some(recorded)
    }

    /// Whether a visual mod's file at `lower` (an ASCII-lowercase path) is part of the
    /// presentation. Visual mods supply artwork, interface, fonts and text: `gfx`, `interface`,
    /// `localisation` and the terrain textures below `map/terrain`. Every other path (scripts,
    /// history, map geometry and the rest of `map`) resolves from the recorded layers alone, with
    /// a mod's files and `replace_path` there out of sight, so a reader handed a presentation
    /// stack still reads recorded simulation content. A mod's defines show only in
    /// [`Self::presentation_defines`].
    fn overlay_sees(&self, lower: &[u8]) -> bool {
        overlay_sees(self.defines_visible, lower)
    }

    /// Whether `layer` stays out of a lookup of the lowercase `lower`.
    fn hides(&self, layer: &ResolvedLayer, lower: &[u8]) -> bool {
        layer.overlay.is_some() && !self.overlay_sees(lower)
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

    /// This presentation stack with its visual mods' defines scripts showing too, for
    /// [`Self::presentation_defines`]; its memos and derived caches are its own.
    fn with_defines(&self) -> io::Result<ResolvedLayers> {
        let mut stack = ResolvedLayers {
            coverage: self.coverage,
            layers: self.layers.clone(),
            above_mods: self.above_mods,
            defines_visible: true,
            overlay_reads: self.overlay_reads.clone(),
            uncovered_reads: self.uncovered_reads.clone(),
            shader_warnings: self.shader_warnings.clone(),
            ..ResolvedLayers::default()
        };
        stack.finish()?;
        Ok(stack)
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
            presentation_defines: None,
            defines_visible: false,
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
            if self.hides(layer, &lower) {
                continue;
            }
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
        let mut observed = self.observation();
        let result = self.read_uncovered_observed(relative, &mut observed);
        self.finish_read(relative, observed, result)
    }

    fn read_uncovered_observed(
        &self,
        relative: &[u8],
        observed: &mut ReadObservation,
    ) -> ObservedResult<ReadSuccess<Vec<u8>>> {
        let Some(relative) = sanitize(relative) else {
            return Err(not_found(relative).into());
        };
        if self.covered_path(&relative) {
            return self.read_path_untraced(&relative, observed);
        }
        // A file a visual mod answered is counted as that mod's; every other attempt, missing
        // or failed included, is an uncovered recorded read.
        let answer = self.uncovered_answer(&relative, observed);
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
    fn uncovered_answer(
        &self,
        relative: &[u8],
        observed: &mut ReadObservation,
    ) -> ObservedResult<(ReadSuccess<Vec<u8>>, usize)> {
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
                    self.verify_passed(&relative, Some(layer), observed)?;
                    if let Some(bytes) = self.read_hit(&hit, &relative, observed)? {
                        return Ok((bytes, layer));
                    }
                }
                None => {
                    self.verify_passed(&relative, None, observed)?;
                    observed.layer = None;
                    return Err(not_found(&relative).into());
                }
            }
        }
        let found = self.scan_uncovered(&relative, observed)?;
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
            None => {
                observed.layer = None;
                Err(not_found(&relative).into())
            }
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
        observed: &mut ReadObservation,
    ) -> ObservedResult<Option<(usize, UncoveredHit, ReadSuccess<Vec<u8>>)>> {
        self.scan_uncovered_with(relative, true, observed)
    }

    fn scan_uncovered_with(
        &self,
        relative: &[u8],
        load: bool,
        observed: &mut ReadObservation,
    ) -> ObservedResult<Option<(usize, UncoveredHit, ReadSuccess<Vec<u8>>)>> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            if self.hides(layer, &lower) {
                continue;
            }
            match &layer.source {
                // A path that is not UTF-8 opens no folder file (see `read_path`).
                LayerSource::Folder(_) => {
                    if let Some((real, handle)) = layer.folder_lookup(relative)? {
                        let bytes = if load {
                            observed.at(index);
                            layer.folder_read(&real, handle, observed)?
                        } else {
                            ReadSuccess::Untraced(Vec::new())
                        };
                        return Ok(Some((index, UncoveredHit::Folder(index, real), bytes)));
                    }
                }
                // ZIP lookup is exact, by the record's name (or up to a `$`), through the
                // layer's one verified lookup, as it is for covered paths.
                LayerSource::Archive(_) => {
                    let opened = if load {
                        observed.at(index);
                        layer.archive_open_observed(relative, observed)?
                    } else {
                        layer
                            .archive_exists(relative)?
                            .then(|| ReadSuccess::Untraced(Vec::new()))
                    };
                    if let Some(bytes) = opened {
                        return Ok(Some((index, UncoveredHit::Archive(index), bytes)));
                    }
                }
            }
            if !load {
                observed.at(index);
                layer.verify_miss(relative, observed)?;
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
        Ok(matches!(
            self.scan_uncovered(&relative, &mut ReadObservation::default())
                .map_err(|failure| failure.error)?,
            Some((0, _, _))
        ))
    }

    /// Reads a memoized hit of `relative`: a folder file by its real spelling (verified if covered),
    /// an archive layer through its one verified lookup ([`ResolvedLayer::archive_open_observed`]).
    /// `Ok(None)` means the hit is stale and the caller rescans; `Err` is a hard error, which
    /// aborts exactly like the scan.
    fn read_hit(
        &self,
        hit: &UncoveredHit,
        relative: &[u8],
        observed: &mut ReadObservation,
    ) -> ObservedResult<Option<ReadSuccess<Vec<u8>>>> {
        match hit {
            UncoveredHit::Folder(index, path) => match self.layers.get(*index) {
                Some(layer) if matches!(layer.source, LayerSource::Folder(_)) => {
                    observed.at(*index);
                    Ok(layer.folder_open(path, observed)?.map(|(_, bytes)| bytes))
                }
                _ => Ok(None),
            },
            UncoveredHit::Archive(index) => match self.layers.get(*index) {
                Some(layer) if matches!(layer.source, LayerSource::Archive(_)) => {
                    observed.at(*index);
                    layer.archive_open_observed(relative, observed)
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
    fn verify_passed(
        &self,
        relative: &[u8],
        answered: Option<usize>,
        observed: &mut ReadObservation,
    ) -> ObservedResult<()> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            if Some(index) == answered {
                break;
            }
            if self.hides(layer, &lower) {
                continue;
            }
            observed.at(index);
            layer.verify_miss(relative, observed)?;
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

/// Read and identify through one Windows handle, sharing reads, writes and deletion.
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
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, GetFinalPathNameByHandleW, OPEN_EXISTING,
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
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
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

/// What a visual mod shows ([`ResolvedLayers::overlay_sees`]), for [`apply_overlay`].
type Sees<'a> = &'a dyn Fn(&[u8]) -> bool;

fn apply_overlay<'a>(
    effective: &mut BTreeMap<LayerPath, EffectiveFile>,
    index: usize,
    archive: bool,
    paths: impl Iterator<Item = &'a LayerPath>,
    replace_paths: &[LayerPath],
    // A visual mod's visibility (`ResolvedLayers::overlay_sees`): its files and `replace_path`
    // count only where it shows. `None` for a recorded layer.
    sees: Option<Sees<'_>>,
) -> io::Result<()> {
    let overlay = sees.is_some();
    let shows = |lower: &[u8]| sees.is_none_or(|sees| sees(lower));
    for replaced in replace_paths {
        let replaced = replaced.to_ascii_lowercase();
        effective.retain(|path, _| !(below(path, &replaced) && shows(path)));
    }
    for path in paths {
        if !shows(&path.to_ascii_lowercase()) {
            continue;
        }
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
        let probe = self.probe(path)?;
        Ok(self.settle(probe, stats))
    }

    /// A file's stamp and digest, from the cache or read now; the cache is
    /// changed only by [`HashCache::settle`].
    fn probe(&self, path: &Path) -> io::Result<Probe> {
        #[cfg(test)]
        hash_cache_tests::PROBED
            .lock()
            .expect("probed paths")
            .push((path.to_owned(), std::thread::current().id()));
        let stamp = Stamp::read(path)?;
        let key = text_path(path)?.to_owned();
        if let Some(hash) = self.get(&key, &stamp) {
            return Ok(Probe {
                key,
                stamp,
                hash,
                read: false,
            });
        }
        let (hash, size) = digest_reader(&mut File::open(path)?)?;
        if size != stamp.size || Stamp::read(path)? != stamp {
            return Err(invalid(format!(
                "content file changed while hashing: {}",
                path.display()
            )));
        }
        Ok(Probe {
            key,
            stamp,
            hash,
            read: true,
        })
    }

    /// Probes every path on the machine's cores, returning each result in
    /// path order.
    fn probe_all(&self, paths: &[PathBuf]) -> Vec<io::Result<Probe>> {
        let priority = job_priority::JobPriority::current();
        crate::parallel::map_entering(
            paths,
            || priority.clone().enter(job_priority::Level::Normal),
            |path| self.probe(path),
        )
    }

    /// Counts a probe and keeps a digest read now.
    fn settle(&mut self, probe: Probe, stats: &mut HashStats) -> [u8; 32] {
        stats.files += 1;
        stats.bytes += probe.stamp.size;
        if probe.read {
            stats.files_read += 1;
            stats.bytes_read += probe.stamp.size;
            self.insert(probe.key, probe.stamp, probe.hash);
        } else {
            stats.cache_hits += 1;
        }
        probe.hash
    }
}

/// One content file's stamp and digest, and whether its bytes were read.
struct Probe {
    key: String,
    stamp: Stamp,
    hash: [u8; 32],
    read: bool,
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
    let directory = game_dir.join("dlc");
    let mut descriptors = Vec::new();
    if !recorded.is_empty() && directory.try_exists()? {
        find_descriptors(&directory, &mut descriptors)?;
    }
    let candidates: Vec<(String, PathBuf)> = descriptors
        .into_iter()
        .filter_map(|path| {
            let id = path.parent()?.file_name()?.to_str()?.to_owned();
            Some((id, path))
        })
        .collect();
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

fn check_hash(
    changed: &mut Vec<ChangedLayer>,
    kind: &'static str,
    id: &str,
    actual: [u8; 32],
    expected: [u8; 32],
) {
    if actual != expected {
        changed.push(ChangedLayer {
            kind,
            id: id.to_owned(),
            recorded: expected,
            installed: actual,
        });
    }
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
        changed: content.changed,
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
        let mut changed = Vec::new();
        if let Some(identity) = recorded {
            let executable = game_dir.join("hoi4.exe");
            fs::metadata(&executable).map_err(|error| named_error("Game", "hoi4.exe", error))?;
            check_hash(
                &mut changed,
                "Game",
                "hoi4.exe",
                cache.file(&executable, &mut stats)?,
                identity.exe_hash,
            );
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
            recorded_descriptor_roster: Default::default(),
            key_names: Default::default(),
            faction_templates: Default::default(),
            derived: Default::default(),
            read_trace: None,
            above_mods: 0,
            presentation: None,
            presentation_defines: None,
            defines_visible: false,
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
                check_hash(&mut changed, "DLC", &id, dlc.hash, expected.hash);
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
            // The game mounts only descriptors directly in `mod/`, never nested ones.
            if !descriptor.split_once('/').is_some_and(|(folder, file)| {
                folder.eq_ignore_ascii_case("mod") && !file.contains('/')
            }) {
                order_notes.push(format!(
                    "enabled mod descriptor {descriptor} is not directly in mod/, where the game lists mods, so the game does not mount it"
                ));
                continue;
            }
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
                check_hash(&mut changed, "Mod", &id, hash, expected.hash);
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
            check_hash(
                &mut changed,
                "Game",
                "vanilla",
                vanilla_hash,
                identity.vanilla_hash,
            );
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
            changed,
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

#[cfg(all(test, windows))]
mod sharing_tests;
