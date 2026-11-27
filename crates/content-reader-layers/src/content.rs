
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, UNIX_EPOCH},
};

use replay_identity::{COVERAGE_POLICY_VERSION, Dlc, Identity, Mod, Verification};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use content_reader_parser::parser::{self, Document};
pub use crate::script_files::ScriptPath;

pub mod facts;
mod changes;
pub mod graph;
pub use content_reader_image::Bytes;
pub(crate) mod generation;
pub use generation::Generation;
pub use generation::{Loading, carry as carry_generation};
pub mod io_counts;
mod physfs_zip;
pub mod proof;
pub mod visual;

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

    fn portable_mod_descriptor(self) -> bool {
        self.0 >= 5
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

#[derive(Clone, Debug)]
pub struct GatherOptions {
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub dlc_order: Option<Vec<String>>,
}

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
    pub changed: Vec<ChangedLayer>,
    pub notes: Vec<String>,
}

impl ResolvedContent {
    pub fn exact(self) -> io::Result<Self> {
        match self.changed.first() {
            Some(changed) => Err(changed.refusal()),
            None => Ok(self),
        }
    }

    pub fn append_changed_to(&self, warnings: &mut Vec<String>) {
        warnings.retain(|line| {
            !line.ends_with(CHANGED_LAYER) && !self.notes.iter().any(|note| note == line)
        });
        warnings.extend(self.changed.iter().map(ToString::to_string));
        warnings.extend(self.notes.iter().cloned());
    }
}

const CHANGED_LAYER: &str = "changed since the recording (content hash mismatch)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedLayer {
    pub kind: &'static str,
    pub id: String,
    pub recorded: [u8; 32],
    pub installed: [u8; 32],
}

impl ChangedLayer {
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
    pub cache_unsaved: Option<String>,
}

#[derive(Clone, Debug)]
pub enum LayerSource {
    Folder(PathBuf),
    Archive(PathBuf),
}

pub type LayerPath = Vec<u8>;

pub mod diagnostics;

#[derive(Clone, Debug)]
pub struct UncoveredReads {
    policy: u32,
    paths: std::sync::Arc<std::sync::Mutex<Option<BTreeSet<LayerPath>>>>,
    overlays: OverlayReads,
}

type OverlayReads = std::sync::Arc<std::sync::Mutex<OverlayDiagnostics>>;

#[derive(Debug, Default)]
struct OverlayDiagnostics {
    reads: BTreeMap<String, BTreeSet<LayerPath>>,
    refusals: BTreeSet<String>,
    refused: BTreeSet<String>,
}

const PRESENTATION_ROOTS: [&str; 7] = [
    "gfx",
    "interface",
    "localisation",
    "map/terrain",
    "map/world_normal.dds",
    "map/world_normal.tga",
    "map/world_normal.bmp",
];
const DEFINES_ROOT: &str = "common/defines";

fn overlay_sees(defines_visible: bool, lower: &[u8]) -> bool {
    PRESENTATION_ROOTS
        .iter()
        .any(|root| below(lower, root.as_bytes()))
        || (defines_visible && below(lower, DEFINES_ROOT.as_bytes()))
}

const OVERLAY_READS: &str = "Artwork from visual mods";

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
    static OVERLAY_OBSERVERS: std::cell::RefCell<Vec<BTreeSet<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static OVERLAY_ISOLATED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static STRICT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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

pub fn is_strict() -> bool {
    STRICT.with(std::cell::Cell::get)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtworkMode {
    #[default]
    Ordinary,
    Strict,
}

pub fn artwork_mode() -> ArtworkMode {
    if is_strict() {
        ArtworkMode::Strict
    } else {
        ArtworkMode::Ordinary
    }
}

#[derive(Default)]
pub struct ByMode<T> {
    ordinary: T,
    strict: T,
}

impl<T> ByMode<T> {
    pub fn current(&self) -> &T {
        match artwork_mode() {
            ArtworkMode::Ordinary => &self.ordinary,
            ArtworkMode::Strict => &self.strict,
        }
    }

    pub fn ordinary(&self) -> &T {
        &self.ordinary
    }
}

pub fn observe_overlay_refusals<T>(run: impl FnOnce() -> T) -> (T, BTreeSet<String>) {
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

pub fn credit_overlay_refusals(refusals: &BTreeSet<String>) {
    if refusals.is_empty() {
        return;
    }
    diagnostics::Diagnostics::credit_fallbacks(refusals);
}

fn credit_overlay_observers(refusals: &BTreeSet<String>) {
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

    pub fn refused(&self) -> BTreeSet<String> {
        self.overlays
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .refused
            .clone()
    }

    pub fn overlay_refusals(&self) -> Option<String> {
        overlay_refusal_line(
            &self
                .overlays
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .refusals,
        )
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerRole {
    Vanilla,
    Builtin,
    UserDirectory,
    Dlc,
    Mod,
    AboveMods,
    Visual,
}

impl LayerRole {
    fn byte(self) -> u8 {
        match self {
            Self::Vanilla => 0,
            Self::Builtin => 1,
            Self::UserDirectory => 2,
            Self::Dlc => 3,
            Self::Mod => 4,
            Self::AboveMods => 5,
            Self::Visual => 6,
        }
    }

    fn from_byte(byte: u8) -> io::Result<Self> {
        Ok(match byte {
            0 => Self::Vanilla,
            1 => Self::Builtin,
            3 => Self::Dlc,
            4 => Self::Mod,
            5 => Self::AboveMods,
            _ => return Err(invalid("unknown recorded layer role")),
        })
    }
}

#[derive(Clone, Debug)]
struct Facts {
    pack: Arc<facts::Pack>,
    layer: usize,
}

impl Facts {
    fn layer(&self) -> &facts::PackLayer {
        &self.pack.layers[self.layer]
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedLayer {
    content_digest: [u8; 32],
    inventory_witnesses: Arc<generation::Witnesses>,
    inventory_paths: Option<Arc<BTreeSet<LayerPath>>>,
    coverage: CoveragePolicy,
    pub role: LayerRole,
    facts: Option<Facts>,
    pub id: String,
    pub source: LayerSource,
    pub descriptor: Option<PathBuf>,
    pub replace_paths: Vec<LayerPath>,
    files: std::sync::Arc<BTreeMap<LayerPath, Member>>,
    lookup: std::sync::Arc<LookupIndex>,
    hashes: std::sync::Arc<BTreeMap<LayerPath, [u8; 32]>>,
    archive_stamp: Option<Stamp>,
    descriptor_hash: Option<DescriptorHash>,
    overlay: Option<Overlay>,
    pub display_name: Option<String>,
    pub art_unmounted: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtFileSource {
    Mounted { layer: usize, path: LayerPath },
    Unmounted { layer: usize, reason: String },
    Missing,
}

#[derive(Clone, Copy, Debug)]
struct DescriptorHash {
    bytes: [u8; 32],
    portable: Option<[u8; 32]>,
}

#[derive(Clone, Debug)]
struct Overlay {
    name: String,
    verified: bool,
}

#[derive(Clone, Debug)]
struct Member {
    archive_entry: Option<physfs_zip::Entry>,
    size: u64,
    stamp: Option<Stamp>,
}

#[derive(Clone, Debug, Default)]
struct LookupIndex {
    folders: BTreeMap<LayerPath, LayerPath>,
    folder_aliases: Arc<std::sync::Mutex<BTreeMap<LayerPath, LayerPath>>>,
    #[cfg(windows)]
    short_names: BTreeMap<(String, String), String>,
    archive: Option<physfs_zip::Index>,
}

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
    product: Option<Arc<graph::Node>>,
    credited: Arc<std::sync::Mutex<BTreeSet<(graph::Digest, graph::Digest)>>>,
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
impl ReadTrace {
    fn new(
        callback: std::sync::Arc<dyn Fn(ContentRead) + Send + Sync>,
        purpose: String,
        product: Option<Arc<graph::Node>>,
    ) -> Self {
        Self {
            callback,
            purpose,
            product,
            credited: Arc::default(),
        }
    }

    fn credit_once(&self, product: &graph::Product) -> bool {
        self.credited
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(product.identity())
    }

    fn for_layers(&self, layers: Vec<usize>) -> Self {
        let callback = self.callback.clone();
        Self {
            callback: std::sync::Arc::new(move |mut read: ContentRead| {
                read.layer = read.layer.and_then(|index| layers.get(index).copied());
                callback(read)
            }),
            purpose: self.purpose.clone(),
            product: self.product.clone(),
            credited: Arc::clone(&self.credited),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedLayers {
    context: [u8; 32],
    recorded_projection: Arc<std::sync::OnceLock<Result<Arc<ResolvedLayers>, Arc<io::Error>>>>,
    generation: Arc<graph::Graph>,
    read_trace: Option<ReadTrace>,
    coverage: CoveragePolicy,
    pub layers: Vec<ResolvedLayer>,
    above_mods: usize,
    presentation: Option<std::sync::Arc<ResolvedLayers>>,
    presentation_defines: Option<std::sync::Arc<ResolvedLayers>>,
    defines_visible: bool,
    overlay_reads: OverlayReads,
    effective: Arc<BTreeMap<LayerPath, EffectiveFile>>,
    uncovered_reads: std::sync::Arc<std::sync::Mutex<Option<BTreeSet<LayerPath>>>>,
    derived: std::sync::Arc<std::sync::Mutex<Derived>>,
}

#[derive(Default)]
struct RetainedProducts {
    values: std::sync::Mutex<
        BTreeMap<
            (&'static str, u32, Vec<u8>),
            Arc<std::sync::Mutex<Option<Arc<graph::Product>>>>,
        >,
    >,
}

#[derive(Default)]
struct Derived {
    context: Option<[u8; 32]>,
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
    fact: Option<u32>,
}

struct ResolvedFile {
    layer: usize,
    path: LayerPath,
    folder: std::sync::Mutex<Option<File>>,
    fact: Option<u32>,
}

impl From<RecordedHit> for ResolvedFile {
    fn from(hit: RecordedHit) -> Self {
        Self {
            layer: hit.layer,
            path: hit.path,
            folder: std::sync::Mutex::new(hit.folder),
            fact: hit.fact,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GatheredContent {
    pub vanilla_hash: [u8; 32],
    pub dlc: Vec<Dlc>,
    pub mods: Vec<Mod>,
    pub language: String,
    pub layers: ResolvedLayers,
    pub stats: HashStats,
    pub order_note: Option<String>,
    pub expected_mounts: Vec<String>,
    pub mod_descriptors: Vec<String>,
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

fn replace_path(path: &[u8]) -> io::Result<LayerPath> {
    let mut end = path.len();
    while end > 0 && matches!(path[end - 1], b'/' | b'\\') {
        end -= 1;
    }
    normalize_path(&path[..end])
}

pub fn normalize_relative(path: &str) -> io::Result<String> {
    normalize_path(path.as_bytes())
        .map(|path| String::from_utf8(path).expect("normalizing keeps UTF-8 text UTF-8"))
}

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

fn folder_opens(relative: &[u8]) -> bool {
    !relative.is_empty()
        && !matches!(
            relative.rsplit(|byte| *byte == b'/').next(),
            Some(b"." | b"..")
        )
}

const RECORD_DIGEST_SCHEME: &str = "archive-record-digest-2";

enum RecordBytes {
    Plain(Vec<u8>),
    Stored(physfs_zip::Stored),
}

fn record_digest(
    archive: &mut physfs_zip::Archive,
    entry: &physfs_zip::Entry,
) -> io::Result<([u8; 32], Option<Vec<u8>>, u64)> {
    record_digest_observed(archive, entry, &mut ReadObservation::default())
        .map(ReadSuccess::into_value)
        .map(|(digest, bytes, size)| {
            let plain = match bytes {
                RecordBytes::Plain(bytes) => Some(bytes),
                RecordBytes::Stored(_) => None,
            };
            (digest, plain, size)
        })
        .map_err(|failure| failure.error)
}

fn record_digest_observed(
    archive: &mut physfs_zip::Archive,
    entry: &physfs_zip::Entry,
    observed: &ReadObservation,
) -> ObservedResult<ReadSuccess<([u8; 32], RecordBytes, u64)>> {
    #[cfg(test)]
    tests::LOADS.with(|loads| loads.set(loads.get() + 1));
    let plain = if entry.encrypted() {
        None
    } else {
        archive.read_observed(entry, None, observed.enabled)?
    };
    let result = match plain {
        Some(bytes) => {
            let digest = payload_digest(&bytes);
            observed.hashed(
                (digest, RecordBytes::Plain(bytes), entry.size),
                entry.size,
                digest,
            )
        }
        None => {
            let stored = archive.stored_observed(entry, observed.enabled)?;
            let digest_bytes = stored.digest_bytes();
            let mut hash = Sha256::new();
            hash.update(b"\0unopened archive record\0");
            hash.update(entry.opening());
            hash.update(digest_bytes);
            let length = digest_bytes.len() as u64;
            let digest = hash.finalize().into();
            observed
                .bytes((), digest_bytes)
                .map(|()| (digest, RecordBytes::Stored(stored), length))
        }
    };
    Ok(result)
}

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

const DESCRIPTOR_ROOT: &str = "gfx";

fn is_descriptor(lower: &[u8]) -> bool {
    below(lower, DESCRIPTOR_ROOT.as_bytes())
        && (lower.ends_with(b".asset") || lower.ends_with(b".gfx"))
}

pub fn covered(path: &str) -> bool {
    covered_path(path.as_bytes())
}

pub fn covered_path(path: &[u8]) -> bool {
    CoveragePolicy::default().covered_path(path)
}

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
    generation::document(path)
}

fn scalar(document: &Document, key: &[u8]) -> io::Result<Option<String>> {
    document
        .get(key)
        .map(|value| {
            value.scalar_str().map(str::to_owned).ok_or_else(|| {
                crate::content_error(format!(
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
        .ok_or_else(|| crate::content_error(format!("descriptor lacks {}", String::from_utf8_lossy(key))))
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
    let descriptor_bytes = generation::metadata(&path)?;
    let document = generation::document(&path)?;
    let descriptor_hash = descriptor_bytes.digest();
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
    let portable = (is_mod && coverage.portable_mod_descriptor())
        .then(|| portable_descriptor(&document, &replace_paths))
        .transpose()?;
    Ok((
        ResolvedLayer {
            content_digest: [0; 32],
            inventory_witnesses: Default::default(),
            inventory_paths: Default::default(),
            coverage,
            role: if is_mod {
                LayerRole::Mod
            } else {
                LayerRole::Dlc
            },
            facts: None,
            id,
            source,
            descriptor: Some(path),
            replace_paths,
            files: Default::default(),
            lookup: Default::default(),
            hashes: Default::default(),
            archive_stamp: None,
            descriptor_hash: Some(DescriptorHash {
                bytes: descriptor_hash,
                portable,
            }),
            overlay: None,
            display_name: scalar(&document, b"name").ok().flatten().filter(|name| !name.is_empty()),
            art_unmounted: None,
        },
        document,
    ))
}

const PORTABLE_DESCRIPTOR: &[u8] = b"\0descriptor";

fn portable_descriptor(document: &Document, replace_paths: &[LayerPath]) -> io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    update_count(&mut hash, replace_paths.len())?;
    for path in replace_paths {
        update_framed(&mut hash, path)?;
    }
    let dependencies: Vec<_> = document.get_all(b"dependencies").collect();
    update_count(&mut hash, dependencies.len())?;
    for value in dependencies {
        update_value(&mut hash, value)?;
    }
    Ok(hash.finalize().into())
}

fn update_count(hash: &mut Sha256, count: usize) -> io::Result<()> {
    let count = u32::try_from(count).map_err(|_| invalid("descriptor value too large"))?;
    hash.update(count.to_le_bytes());
    Ok(())
}

fn update_framed(hash: &mut Sha256, bytes: &[u8]) -> io::Result<()> {
    update_count(hash, bytes.len())?;
    hash.update(bytes);
    Ok(())
}

fn update_value(hash: &mut Sha256, value: &parser::Value) -> io::Result<()> {
    let (tag, items) = match value {
        parser::Value::Scalar(bytes) => {
            hash.update([0u8]);
            return update_framed(hash, bytes);
        }
        parser::Value::Block(items) => (None, items),
        parser::Value::Tagged { tag, items } => (Some(tag), items),
    };
    match tag {
        None => hash.update([1u8]),
        Some(tag) => {
            hash.update([2u8]);
            update_framed(hash, tag)?;
        }
    }
    update_count(hash, items.len())?;
    for item in items {
        match &item.key {
            None => hash.update([0u8]),
            Some(key) => {
                hash.update([1u8]);
                update_framed(hash, key)?;
            }
        }
        hash.update(item.operator.text());
        update_value(hash, &item.value)?;
    }
    Ok(())
}

fn directory_entries(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    generation::listing(path)?;
    let mut entries = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn collect_folder(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<LayerPath, Member>,
    coverage: CoveragePolicy,
    overlay: bool,
    inventory_paths: &mut BTreeSet<LayerPath>,
) -> io::Result<()> {
    collect_folder_with(
        root,
        path,
        files,
        coverage,
        overlay,
        &entry_kind,
        inventory_paths,
    )
}

fn collect_folder_with(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<LayerPath, Member>,
    coverage: CoveragePolicy,
    overlay: bool,
    kind: &dyn Fn(&fs::DirEntry) -> io::Result<Listed>,
    inventory_paths: &mut BTreeSet<LayerPath>,
) -> io::Result<()> {
    let names_folder = |link: &Path| fs::metadata(link).is_ok_and(|metadata| metadata.is_dir());
    let refused = |link: &Path| {
        invalid(format!(
            "symbolic link {} names a folder, whose files the game lists only from a folder below it",
            link.display()
        ))
    };
    if overlay
        && fs::symlink_metadata(path)?.file_type().is_symlink()
        && link_kind(path)? == Listed::LinkedFile
        && names_folder(path)
    {
        return Err(refused(path));
    }
    let checked = |entry: &fs::DirEntry| -> io::Result<Listed> {
        let listed = kind(entry)?;
        if overlay && listed == Listed::LinkedFile && names_folder(&entry.path()) {
            return Err(refused(&entry.path()));
        }
        Ok(listed)
    };
    walk_folder(root, path, overlay, &checked, &mut |relative, entry| {
        inventory_paths.insert(relative.clone());
        if coverage.covered_path(&relative) {
            let metadata = entry.metadata()?;
            let stamp = (!overlay).then(|| Stamp::of(&metadata).ok()).flatten();
            files.insert(
                relative,
                Member {
                    archive_entry: None,
                    size: metadata.len(),
                    stamp,
                },
            );
        }
        Ok(())
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listed {
    File,
    Directory,
    LinkedFile,
    LinkedDirectory,
    Other,
}

fn entry_kind(entry: &fs::DirEntry) -> io::Result<Listed> {
    let kind = entry.file_type()?;
    Ok(if kind.is_symlink() {
        link_kind(&entry.path())?
    } else if kind.is_dir() {
        Listed::Directory
    } else if kind.is_file() {
        Listed::File
    } else {
        Listed::Other
    })
}

#[cfg(windows)]
fn link_kind(path: &Path) -> io::Result<Listed> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FindClose, FindFirstFileW,
            WIN32_FIND_DATAW,
        },
    };
    const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
    let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let handle = unsafe { FindFirstFileW(name.as_ptr(), &raw mut data) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    unsafe { FindClose(handle) };
    let symbolic = data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        && data.dwReserved0 == IO_REPARSE_TAG_SYMLINK;
    Ok(
        if !symbolic && data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            Listed::LinkedDirectory
        } else {
            Listed::LinkedFile
        },
    )
}

#[cfg(not(windows))]
fn link_kind(_path: &Path) -> io::Result<Listed> {
    Ok(Listed::LinkedFile)
}

#[cfg(windows)]
fn check_find_error(dir: &Path, error: io::Error, done: u32) -> io::Result<()> {
    if error.raw_os_error() == Some(done as i32) {
        return Ok(());
    }
    Err(io::Error::new(
        error.kind(),
        format!("short-name enumeration {}: {error}", dir.display()),
    ))
}

#[cfg(windows)]
fn collect_folder_short_names(
    root: &Path,
    files: &BTreeMap<LayerPath, Member>,
) -> io::Result<BTreeMap<(String, String), String>> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW},
    };
    struct FindHandle(HANDLE);
    impl Drop for FindHandle {
        fn drop(&mut self) {
            // SAFETY: self.0 is a valid search handle returned by FindFirstFileW.
            unsafe { FindClose(self.0) };
        }
    }
    let mut dirs = BTreeSet::new();
    for path in files.keys() {
        let Ok(text) = std::str::from_utf8(path) else {
            continue;
        };
        let mut current = text;
        while let Some((parent, _)) = current.rsplit_once('/') {
            if !dirs.insert(parent.to_owned()) {
                break;
            }
            current = parent;
        }
        dirs.insert(String::new());
    }
    let mut short_names = BTreeMap::new();
    for dir in dirs {
        let mut dir_path = root.to_path_buf();
        if !dir.is_empty() {
            for part in dir.split('/') {
                dir_path.push(part);
            }
        }
        let pattern = dir_path.join("*");
        let wide: Vec<u16> = pattern.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: WIN32_FIND_DATAW is plain data that accepts all-zero initialization.
        let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
        // SAFETY: wide is NUL-terminated and data is a valid mutable pointer.
        let handle = unsafe { FindFirstFileW(wide.as_ptr(), &raw mut data) };
        if handle == INVALID_HANDLE_VALUE {
            check_find_error(&dir_path, io::Error::last_os_error(), ERROR_FILE_NOT_FOUND)?;
            continue;
        }
        let _guard = FindHandle(handle);
        let parent_folded = dir.to_lowercase();
        loop {
            let alt_len = data
                .cAlternateFileName
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(data.cAlternateFileName.len());
            if alt_len > 0 {
                let name_len = data
                    .cFileName
                    .iter()
                    .position(|&unit| unit == 0)
                    .unwrap_or(data.cFileName.len());
                if let (Ok(short), Ok(long)) = (
                    OsString::from_wide(&data.cAlternateFileName[..alt_len]).into_string(),
                    OsString::from_wide(&data.cFileName[..name_len]).into_string(),
                ) && long != "."
                    && long != ".."
                {
                    short_names.insert((parent_folded.clone(), short.to_lowercase()), long);
                }
            }
            // SAFETY: handle is open and valid until _guard drops.
            if unsafe { FindNextFileW(handle, &raw mut data) } == 0 {
                check_find_error(&dir_path, io::Error::last_os_error(), ERROR_NO_MORE_FILES)?;
                break;
            }
        }
    }
    Ok(short_names)
}

fn walk_folder(
    root: &Path,
    directory: &Path,
    overlay: bool,
    kind: &dyn Fn(&fs::DirEntry) -> io::Result<Listed>,
    visit: &mut dyn FnMut(LayerPath, &fs::DirEntry) -> io::Result<()>,
) -> io::Result<()> {
    let start = if !overlay {
        directory.to_path_buf()
    } else {
        if fs::symlink_metadata(directory)?.file_type().is_symlink()
            && link_kind(directory)? == Listed::LinkedFile
        {
            return Ok(());
        }
        match fs::canonicalize(directory) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            start => start?,
        }
    };
    if overlay && let Some(graph) = generation::current() {
        graph.watch_root(&start, true);
    }
    walk_below(root, directory, &mut vec![start], overlay, kind, visit)
}

fn walk_below(
    root: &Path,
    directory: &Path,
    held: &mut Vec<PathBuf>,
    overlay: bool,
    kind: &dyn Fn(&fs::DirEntry) -> io::Result<Listed>,
    visit: &mut dyn FnMut(LayerPath, &fs::DirEntry) -> io::Result<()>,
) -> io::Result<()> {
    for entry in directory_entries(directory)? {
        let full_path = entry.path();
        let listed = kind(&entry)?;
        if !overlay && matches!(listed, Listed::LinkedFile | Listed::LinkedDirectory) {
            return Err(invalid(format!(
                "content layer contains a symbolic link: {}",
                full_path.display()
            )));
        }
        let real = match listed {
            Listed::File | Listed::LinkedFile => {
                let relative = normalize_path(
                    text_path(
                        full_path
                            .strip_prefix(root)
                            .map_err(|_| invalid("layer path escaped root"))?,
                    )?
                    .as_bytes(),
                )?;
                visit(relative, &entry)?;
                continue;
            }
            Listed::Other => continue,
            Listed::Directory => held
                .last()
                .expect("a walk holds its own folder")
                .join(entry.file_name()),
            Listed::LinkedDirectory => {
                let target = match fs::canonicalize(&full_path) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    target => target?,
                };
                if held.iter().any(|folder| folder.starts_with(&target)) {
                    return Err(invalid(format!(
                        "folder link {} leads back to {}, which the game lists only as deep as its own limits allow",
                        full_path.display(),
                        target.display()
                    )));
                }
                if let Some(graph) = generation::current() {
                    graph.watch_root(&target, true);
                }
                target
            }
        };
        held.push(real);
        let walked = walk_below(root, &full_path, held, overlay, kind, visit);
        held.pop();
        walked?;
    }
    Ok(())
}

impl ResolvedLayer {
    pub fn visual_mod_name(&self) -> Option<&str> {
        self.overlay.as_ref().map(|overlay| overlay.name.as_str())
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name
            .as_deref()
            .or_else(|| self.visual_mod_name())
    }

    pub fn art_unmounted(&self) -> Option<&str> {
        self.art_unmounted.as_deref()
    }

    fn verifies(&self) -> bool {
        self.facts.is_none() && self.overlay.as_ref().is_none_or(|overlay| overlay.verified)
    }

    fn listed_as_game(&self) -> bool {
        self.overlay.is_some() || self.facts.is_some()
    }

    fn fact_path(&self, path: &[u8]) -> bool {
        self.facts
            .as_ref()
            .is_some_and(|facts| fact_class(path) && !facts.pack.is_art(path))
    }

    fn fact_lookup(&self, path: &[u8]) -> io::Result<Option<(LayerPath, u32)>> {
        let Some(facts) = self.facts.as_ref().filter(|_| self.fact_path(path)) else {
            return Ok(None);
        };
        if !self.coverage.covered_path(path) {
            return Err(unproven(path, self.coverage));
        }
        Ok(facts
            .layer()
            .find(path)?
            .map(|(real, file)| (real.clone(), file)))
    }

    fn fact_listing_refusal(&self, prefix: &[u8], suffixes: &[String]) -> Option<io::Error> {
        let facts = self.facts.as_ref()?;
        if !lists_facts(prefix, suffixes) {
            return None;
        }
        if let Some(reason) = facts.layer().withheld() {
            return Some(invalid(format!(
                "game facts of {} are withheld: {reason}",
                self.id
            )));
        }
        (!self.coverage.covered_path(prefix) && fact_class(prefix))
            .then(|| unproven(prefix, self.coverage))
    }

    pub fn dlc_name(&self) -> io::Result<Option<String>> {
        if let Some(facts) = &self.facts {
            let layer = facts.layer();
            return match (&layer.dlc_name, self.role) {
                (Some(name), _) => Ok(Some(name.clone())),
                (None, LayerRole::Dlc) => Err(crate::content_error(format!(
                    "the name of DLC {} is withheld: {}",
                    self.id,
                    layer.withheld().unwrap_or("the recording does not give it")
                ))),
                (None, _) => Ok(None),
            };
        }
        let Some(descriptor) = self.descriptor.as_ref().filter(|path| {
            path.extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("dlc"))
        }) else {
            return Ok(None);
        };
        let bytes = generation::metadata(descriptor)?;
        let actual: [u8; 32] = Sha256::digest(&bytes).into();
        if self.descriptor_hash.map(|parsed| parsed.bytes) != Some(actual) {
            return Err(invalid(format!(
                "DLC descriptor {} changed after content verification",
                descriptor.display()
            )));
        }
        required(&parser::parse(&bytes)?, b"name").map(Some)
    }

    fn exact_names(&self) -> bool {
        match &self.facts {
            Some(facts) => facts.layer().archive,
            None => matches!(self.source, LayerSource::Archive(_)),
        }
    }

    fn listed_keys(&self) -> impl Iterator<Item = &LayerPath> {
        let facts = self
            .facts
            .as_ref()
            .and_then(|facts| facts.layer().paths().ok())
            .into_iter()
            .flatten();
        facts.chain(self.files.keys())
    }

    fn inventory(&mut self) -> io::Result<()> {
        let _collection = generation::Collect::enter(Arc::clone(&self.inventory_witnesses));
        if let LayerSource::Folder(root) = &mut self.source {
            if let Some(graph) = generation::current() {
                graph.watch_root(root, true);
                graph.witnesses.directory(root)?;
            }
            *root = fs::canonicalize(&*root)?;
        }
        match &self.source {
            LayerSource::Folder(root) => {
                self.inventory_witnesses.directory(root)?;
                let listed_as_game = self.listed_as_game();
                let walked = self
                    .coverage
                    .roots()
                    .filter(|directory| !below(directory.as_bytes(), DESCRIPTOR_ROOT.as_bytes()))
                    .chain(std::iter::once(DESCRIPTOR_ROOT));
                let mut paths = BTreeSet::new();
                for directory in walked {
                    if let Some(path) = folded_path(root, directory)? {
                        collect_folder(
                            root,
                            &path,
                            std::sync::Arc::make_mut(&mut self.files),
                            self.coverage,
                            listed_as_game,
                            &mut paths,
                        )?;
                    }
                }
                self.inventory_paths = Some(Arc::new(paths));
            }
            LayerSource::Archive(path) => {
                self.inventory_witnesses.file(path, &File::open(path)?)?;
                let stamp = Stamp::read(path)?;
                let mut archive = physfs_zip::Archive::open(path)?;
                let mounted = archive.mount()?;
                for entry in &mounted.entries {
                    if entry.directory {
                        continue;
                    }
                    if entry.symlink && !self.listed_as_game() {
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
                                stamp: None,
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
                if self.listed_as_game() {
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
        if let LayerSource::Folder(root) = &self.source {
            let verifies = self.verifies();
            let lookup = std::sync::Arc::make_mut(&mut self.lookup);
            lookup.folders = folded;
            #[cfg(windows)]
            if verifies {
                lookup.short_names = collect_folder_short_names(root, &self.files)?;
            }
            #[cfg(not(windows))]
            let _ = (root, verifies);
        }
        Ok(())
    }

    fn hash(&mut self, cache: &mut HashCache, stats: &mut HashStats) -> io::Result<[u8; 32]> {
        cache.continuing()?;
        let mut hashes = BTreeMap::new();
        match &self.source {
            LayerSource::Folder(root) => {
                let paths = self
                    .files
                    .iter()
                    .map(|(relative, member)| {
                        Ok((root.join(folder_text(relative)?), member.stamp.clone()))
                    })
                    .collect::<io::Result<Vec<_>>>()?;
                for (relative, probe) in self.files.keys().zip(cache.probe_all(&paths)) {
                    hashes.insert(relative.clone(), cache.settle(probe?, stats));
                }
            }
            LayerSource::Archive(path) => {
                self.inventory_witnesses.file(path, &File::open(path)?)?;
                let stamp = Stamp::read(path)?;
                if self.archive_stamp.as_ref() != Some(&stamp) {
                    return Err(invalid("archive changed since inventory"));
                }
                let mut archive = physfs_zip::Archive::open(path)?;
                for (relative, member) in self.files.iter() {
                    cache.continuing()?;
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
            let digest = cache.file(descriptor, stats)?;
            let parsed = self
                .descriptor_hash
                .filter(|parsed| parsed.bytes == digest)
                .ok_or_else(|| invalid("descriptor changed between parsing and hashing"))?;
            let (key, digest) = match parsed.portable {
                Some(portable) => (PORTABLE_DESCRIPTOR.to_vec(), portable),
                None => (normalize_path(name.as_bytes())?, digest),
            };
            if hashes.insert(key, digest).is_some() {
                return Err(invalid(
                    "descriptor path collides with a covered layer file",
                ));
            }
        }
        layer_hash(&hashes)
    }

    fn covered_folder_entry(&self, relative: &[u8]) -> Option<LayerPath> {
        let lower = relative.to_ascii_lowercase();
        if let Some(path) = self.lookup.folders.get(&lower) {
            return Some(path.clone());
        }
        if self.verifies() {
            if let Some(path) = self
                .lookup
                .folder_aliases
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&lower)
            {
                return Some(path.clone());
            }
            #[cfg(windows)]
            {
                let text = std::str::from_utf8(relative).ok()?;
                let mut parent_folded = String::new();
                let mut resolved_ascii = Vec::with_capacity(relative.len());
                for part in text.split('/') {
                    let trimmed = part.trim_end_matches(['.', ' ']);
                    let part_folded = trimmed.to_lowercase();
                    let canonical = self
                        .lookup
                        .short_names
                        .get(&(parent_folded.clone(), part_folded))
                        .map(String::as_str)
                        .unwrap_or(trimmed);
                    if !resolved_ascii.is_empty() {
                        resolved_ascii.push(b'/');
                        parent_folded.push('/');
                    }
                    resolved_ascii.extend_from_slice(&canonical.as_bytes().to_ascii_lowercase());
                    parent_folded.push_str(&canonical.to_lowercase());
                }
                if let Some(path) = self.lookup.folders.get(&resolved_ascii) {
                    return Some(path.clone());
                }
                if !parent_folded.is_ascii() {
                    return self
                        .lookup
                        .folders
                        .values()
                        .find(|path| {
                            std::str::from_utf8(path)
                                .is_ok_and(|existing| existing.to_lowercase() == parent_folded)
                        })
                        .cloned();
                }
            }
        }
        None
    }

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
                return match self.covered_folder_entry(relative) {
                    Some(path) if self.verifies() => Err(changed(&path)),
                    _ => Ok(None),
                };
            }
            opened => opened?,
        };
        let real = match path.strip_prefix(root) {
            Ok(real) => normalize_path(text_path(real)?.as_bytes())?,
            Err(_)
                if self.listed_as_game()
                    && !(self.verifies() && self.coverage.covered_path(relative)) =>
            {
                relative.to_vec()
            }
            Err(_) => {
                return Err(invalid(format!(
                    "folder file resolves outside its layer: {}",
                    path.display()
                )));
            }
        };
        if self.verifies() && self.coverage.covered_path(&real) {
            if self.lookup.folders.get(&real.to_ascii_lowercase()) != Some(&real) {
                return Err(changed(&real));
            }
            let lower = relative.to_ascii_lowercase();
            if self.lookup.folders.get(&lower) != Some(&real) {
                self.lookup
                    .folder_aliases
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(lower, real.clone());
            }
        }
        if self.fact_path(&real) {
            return Err(fact_alias(relative, &real));
        }
        Ok(Some((real, file)))
    }

    #[cfg(test)]
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
            io_counts::layer_digest(self, real, digest, length);
            observed
                .hashed(bytes, length, digest)
                .verified(self.verify(real, &digest))
        } else {
            io_counts::layer_bytes(self, real, &bytes);
            let result = observed.bytes((), &bytes);
            Ok(result.map(|()| bytes))
        }
    }

    fn verify(&self, relative: &[u8], digest: &[u8; 32]) -> io::Result<()> {
        if self.verifies() && self.hashes.get(relative) != Some(digest) {
            return Err(changed(relative));
        }
        Ok(())
    }

    fn verified_archive(&self) -> io::Result<(physfs_zip::Archive, physfs_zip::Mounted)> {
        let LayerSource::Archive(path) = &self.source else {
            return Err(invalid(format!("layer {} is not an archive", self.id)));
        };
        let unchanged =
            Stamp::read(path).is_ok_and(|stamp| self.archive_stamp.as_ref() == Some(&stamp));
        if !unchanged && self.verifies() {
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
        if entry.symlink {
            return Err(unfollowed_link(request).into());
        }
        if self.fact_path(&entry.name) {
            return Err(fact_alias(request, &entry.name).into());
        }
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
            return match record {
                ReadSuccess::Untraced((_, RecordBytes::Plain(bytes), _)) => {
                    if password.is_some() {
                        Ok(None)
                    } else {
                        io_counts::layer_digest(self, &entry.name, digest, bytes.len() as u64);
                        Ok(Some(ReadSuccess::Untraced(bytes)))
                    }
                }
                ReadSuccess::Traced {
                    value: (_, RecordBytes::Plain(bytes), _),
                    payload,
                } => {
                    if password.is_some() {
                        Ok(None)
                    } else {
                        io_counts::layer_digest(self, &entry.name, digest, bytes.len() as u64);
                        Ok(Some(ReadSuccess::Traced {
                            value: bytes,
                            payload,
                        }))
                    }
                }
                ReadSuccess::Untraced((_, RecordBytes::Stored(stored), _))
                | ReadSuccess::Traced {
                    value: (_, RecordBytes::Stored(stored), _),
                    ..
                } => {
                    if password.is_none() {
                        return Ok(None);
                    }
                    let bytes =
                        physfs_zip::decode_stored(entry, &stored, password, observed.enabled)
                            .map_err(|failure| failure.context(context))?;
                    Ok(bytes.map(|bytes| {
                        io_counts::layer_bytes(self, &entry.name, &bytes);
                        observed.bytes((), &bytes).map(|()| bytes)
                    }))
                }
            };
        }
        #[cfg(test)]
        tests::LOADS.with(|loads| loads.set(loads.get() + 1));
        let bytes = archive
            .read_observed(entry, password, observed.enabled)
            .map_err(|failure| failure.context(context))?;
        Ok(bytes.map(|bytes| {
            io_counts::layer_bytes(self, &entry.name, &bytes);
            observed.bytes((), &bytes).map(|()| bytes)
        }))
    }

    fn archive_exists(&self, request: &[u8]) -> io::Result<bool> {
        let (mut archive, mounted) = self.verified_archive()?;
        match mounted.lookup(request) {
            Some((entry, _)) if entry.symlink => Err(unfollowed_link(request)),
            Some((entry, _)) if self.fact_path(&entry.name) => {
                Err(fact_alias(request, &entry.name))
            }
            Some((entry, password)) => {
                let opening = archive.opening(entry)?;
                if self.verifies()
                    && self.files.contains_key(&entry.name)
                    && self
                        .lookup
                        .archive
                        .as_ref()
                        .map(|index| index.opening(&entry.name))
                        != Some(opening.as_ref())
                {
                    return Err(changed(&entry.name));
                }
                Ok(opening.is_some_and(|opening| opening.opens(password)))
            }
            None => Ok(false),
        }
    }

    fn record_name<'a>(&'a self, request: &'a [u8]) -> &'a [u8] {
        self.lookup
            .archive
            .as_ref()
            .and_then(|archive| archive.mounted.lookup(request))
            .map_or(request, |(entry, _)| entry.name.as_slice())
    }

    fn reaches_link(&self, request: &[u8]) -> bool {
        self.lookup
            .archive
            .as_ref()
            .and_then(|archive| archive.mounted.lookup(request))
            .is_some_and(|(entry, _)| entry.symlink)
    }

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

    fn verify_miss_probe(&self, relative: &[u8]) -> io::Result<()> {
        if !self.verifies() {
            return Ok(());
        }
        match &self.source {
            LayerSource::Archive(_) => {
                if self.archive_exists(relative)? {
                    return Err(changed(relative));
                }
                Ok(())
            }
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

fn fact_class(path: &[u8]) -> bool {
    CoveragePolicy::default().covered_path(path)
}

fn lists_facts(prefix: &[u8], suffixes: &[String]) -> bool {
    let lower = prefix.to_ascii_lowercase();
    fact_class(&lower)
        || lower.is_empty()
        || ((lower == b"gfx"
            || below(&lower, b"gfx")
            || lower == b"gfx/"
            || lower.starts_with(b"gfx/"))
            && (suffixes.is_empty()
                || suffixes.iter().any(|suffix| {
                    [".gfx", ".asset", ".txt"]
                        .iter()
                        .any(|fact| fact.ends_with(suffix.as_str()) || suffix.ends_with(fact))
                })))
}

fn unproven(path: &[u8], coverage: CoveragePolicy) -> io::Error {
    invalid(format!(
        "{} is a game fact this recording's content identity (coverage policy {}) never hashed, so it is withheld",
        display(path),
        coverage.0
    ))
}

fn fact_alias(request: &[u8], fact: &[u8]) -> io::Error {
    invalid(format!(
        "{} names the recorded game fact {} by another spelling, which playback does not read from the installed game",
        display(request),
        display(fact)
    ))
}

fn changed(relative: &[u8]) -> io::Error {
    invalid(format!(
        "content file changed since verification: {}",
        display(relative)
    ))
}

fn unfollowed_link(relative: &[u8]) -> io::Error {
    invalid(format!(
        "{} is a symbolic link record, which the port does not follow to the member it names",
        display(relative)
    ))
}

fn folder_text(relative: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(relative).map_err(|_| invalid("folder layer path is not UTF-8"))
}

pub fn open_request(layers: &ResolvedLayers, path: &[u8]) -> io::Result<Option<Bytes>> {
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

    pub fn with_read_trace(
        &self,
        trace: std::sync::Arc<dyn Fn(ContentRead) + Send + Sync>,
    ) -> Self {
        Self {
            read_trace: Some(ReadTrace::new(
                trace,
                String::new(),
                self.read_trace
                    .as_ref()
                    .and_then(|trace| trace.product.clone()),
            )),
            derived: Default::default(),

            ..self.clone()
        }
    }

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

    pub fn derived<T: Default + Send + Sync + 'static>(&self) -> std::sync::Arc<T> {
        let mut derived = self
            .derived
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if derived.context != Some(self.context)
            || derived.roster.len() != self.layers.len()
            || !derived.roster.iter().eq(self.layers.iter().map(|layer| &layer.id))
        {
            derived.context = Some(self.context);
            derived.roster = self.layers.iter().map(|layer| layer.id.clone()).collect();
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

    pub fn product<T: Send + Sync + 'static>(
        &self,
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.product_at(
            graph::Key {
                context: self.product_context(),
                parser,
                schema,
                parameters: parameters.to_vec(),
            },
            None,
            build,
        )
    }

    pub fn file_product<T: Send + Sync + 'static>(
        &self,
        path: &[u8],
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        build: impl FnOnce(&Bytes) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        let normalized = sanitize(path).ok_or_else(|| not_found(path))?;
        let path = normalized.as_slice();
        let resolved = match self.resolved_file(path, true) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.note_uncovered_attempt(path, None);
                self.trace_read(path, None, ReadFailure::from(io::Error::new(error.kind(), error.to_string())).outcome());
                return Err(error);
            }
        };
        let Some(file) = resolved.as_ref().as_ref() else {
            self.note_uncovered_attempt(path, None);
            let error = not_found(path);
            self.trace_read(path, None, ReadFailure::from(io::Error::new(error.kind(), error.to_string())).outcome());
            return Err(error);
        };
        let mut key_parameters = (file.path.len() as u64).to_le_bytes().to_vec();
        key_parameters.extend_from_slice(&file.path);
        key_parameters.push(u8::from(file.fact.is_some()));
        if let Some(fact) = file.fact {
            key_parameters.extend_from_slice(&fact.to_le_bytes());
        }
        key_parameters.extend_from_slice(parameters);
        let product = self.product_at(
            graph::Key {
                context: self.layer_context(file.layer),
                parser,
                schema,
                parameters: key_parameters,
            },
            Some(file.layer),
            |reader| build(&reader.shared_read_in(path, true, Some(file))?),
        );
        file.folder
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        product
    }

    fn product_at<T: Send + Sync + 'static>(
        &self,
        key: graph::Key,
        file_layer: Option<usize>,
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        let (product, built) = self.product_entry(key, file_layer, build)?;
        self.credit_product(&product, built, file_layer);
        product.value()
    }

    fn credit_product(&self, product: &graph::Product, built: bool, file_layer: Option<usize>) {
        if !built {
            for read in &product.reads {
                self.trace_read(&read.path, file_layer.or(read.layer), read.outcome.clone());
            }
        }
        if built {
            product.diagnostics.credit();
        } else {
            product.diagnostics.credit_to(self);
        }
    }

    fn product_entry<T: Send + Sync + 'static>(
        &self,
        key: graph::Key,
        file_layer: Option<usize>,
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<(Arc<graph::Product>, bool)> {
        self.product_entry_with_retry(key, file_layer, true, build)
    }

    fn product_entry_with_retry<T: Send + Sync + 'static>(
        &self,
        key: graph::Key,
        file_layer: Option<usize>,
        retry_errors: bool,
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<(Arc<graph::Product>, bool)> {
        let parent = self
            .read_trace
            .as_ref()
            .and_then(|trace| trace.product.as_ref());
        let (product, built) = self.generation.product_with_retry(key, parent, retry_errors, |node| {
            let reads = Arc::new(std::sync::Mutex::new(Vec::new()));
            let kept = Arc::clone(&reads);
            let original = self.read_trace.clone();
            let mut reader = self.clone();
            reader.read_trace = Some(ReadTrace::new(
                Arc::new(move |read: ContentRead| {
                    let mut witness = read.clone();
                    if file_layer.is_some() {
                        witness.layer = None;
                    }
                    kept.lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(witness);
                    if let Some(trace) = &original {
                        (trace.callback)(read);
                    }
                }),
                self.read_trace
                    .as_ref()
                    .map_or_else(String::new, |trace| trace.purpose.clone()),
                Some(Arc::clone(node)),
            ));
            let (value, diagnostics) = diagnostics::capture(|| build(&reader));
            let reads =
                std::mem::take(&mut *reads.lock().unwrap_or_else(|error| error.into_inner()));
            (value, reads, diagnostics)
        })?;
        Ok((product, built))
    }

    pub fn retained_product<T: Send + Sync + 'static>(
        &self,
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.retained_product_with_errors(parser, schema, parameters, false, build)
    }

    pub fn retained_product_with_errors<T: Send + Sync + 'static>(
        &self,
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        cache_errors: bool,
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        let cache = self.derived::<RetainedProducts>();
        let cell = Arc::clone(
            cache.values.lock().unwrap_or_else(|error| error.into_inner())
                .entry((parser, schema, parameters.to_vec())).or_default(),
        );
        let mut kept = cell.lock().unwrap_or_else(|error| error.into_inner());
        let (product, built, from_holder, first_credit) = match kept.as_ref() {
            Some(product) => {
                let product = Arc::clone(product);
                let first_credit = self
                    .read_trace
                    .as_ref()
                    .is_some_and(|trace| trace.credit_once(&product));
                (product, false, true, first_credit)
            }
            None => {
                let (product, built) = self.product_entry_with_retry(
                    graph::Key {
                        context: self.context,
                        parser,
                        schema,
                        parameters: parameters.to_vec(),
                    },
                    None,
                    !cache_errors,
                    build,
                )?;
                if cache_errors || product.successful() {
                    *kept = Some(Arc::clone(&product));
                }
                let first_credit = self
                    .read_trace
                    .as_ref()
                    .is_some_and(|trace| trace.credit_once(&product));
                (product, built, false, first_credit)
            }
        };
        drop(kept);
        if let Some(parent) = self.read_trace.as_ref().and_then(|trace| trace.product.as_ref()) {
            self.generation.depends(parent, &product)?;
        }
        if from_holder && !first_credit {
            product.diagnostics.credit_to(self);
        } else {
            self.credit_product(&product, built, None);
        }
        product.value()
    }

    pub fn retained_recorded_product<T: Send + Sync + 'static>(
        &self,
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        let reader = self.recorded_or_self()?;
        reader.retained_product(parser, schema, parameters, build)
    }

    pub fn recorded_or_self(&self) -> io::Result<ResolvedLayers> {
        Ok(self.project_recorded()?.unwrap_or_else(|| self.clone()))
    }

    pub fn recorded_product<T: Send + Sync + 'static>(
        &self,
        parser: &'static str,
        schema: u32,
        parameters: &[u8],
        build: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        let reader = if self.shows_visual_mods() {
            self.project_recorded()?.ok_or_else(|| {
                invalid("the recorded content stack cannot be projected below the visual mods")
            })?
        } else {
            self.clone()
        };
        reader.product(parser, schema, parameters, build)
    }

    pub fn reads_visual_defines(&self) -> bool {
        self.defines_visible
    }

    pub fn product_counts(&self) -> Vec<graph::ProductCount> {
        self.generation.counts()
    }

    pub fn content_counts(&self) -> graph::Counts {
        self.generation.snapshot()
    }

    pub fn generation_is_current(&self) -> bool {
        !self
            .generation
            .stale
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn validate_generation(&self) -> io::Result<()> {
        self.validate_generation_in(true)
    }

    pub fn admit_generation(&self) -> io::Result<()> {
        self.validate_generation_in(false)
    }

    pub fn with_file_witness<T>(&self, path: &Path, read: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        if let Some(parent) = path.parent() {
            self.generation.watch_root(parent, false);
        }
        self.generation.witnesses.directory(path)?;
        let value = read()?;
        match path.canonicalize() {
            Ok(actual) => {
                if let Some(parent) = actual.parent() {
                    self.generation.watch_root(parent, false);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => self.generation.changes.unwatchable(path, &error),
        }
        if let Err(error) = self.generation.witnesses.validate_path(path) {
            self.generation.stale.store(true, std::sync::atomic::Ordering::Release);
            return Err(error);
        }
        Ok(value)
    }

    pub fn is_generation_scoped(&self) -> bool {
        self.generation.is_scoped()
    }

    fn validate_generation_in(&self, force: bool) -> io::Result<()> {
        if !self.generation_is_current() {
            return Err(invalid("content generation needs to be refreshed"));
        }
        let force = force || self.generation.has_unwatched_roots();
        let result = self.generation.changes.validate(force, || {
            self.generation.witnesses.validate()?;
            let inventories: Vec<_> = self.generation.inventories
                .lock().unwrap_or_else(|error| error.into_inner())
                .iter().filter_map(std::sync::Weak::upgrade).collect();
            for inventory in inventories {
                inventory.validate()?;
            }
            Ok(())
        });
        if result.is_err() {
            self.generation
                .stale
                .store(true, std::sync::atomic::Ordering::Release);
        }
        result
    }

    pub fn enter_generation(&self) -> Generation {
        Generation::attach(Arc::clone(&self.generation))
    }

    pub fn loading(&self) -> Loading {
        Loading::new(Arc::clone(&self.generation))
    }

    fn watch_directory(&self, layer: &ResolvedLayer, relative: &[u8]) -> io::Result<()> {
        if let LayerSource::Folder(root) = &layer.source {
            if let Ok(relative) = std::str::from_utf8(relative) {
                let parent = Path::new(relative)
                    .parent()
                    .unwrap_or_else(|| Path::new(""));
                self.generation
                    .witnesses
                    .adopt(&root.join(parent), &layer.inventory_witnesses)?;
            }
        }
        Ok(())
    }

    fn product_context(&self) -> [u8; 32] {
        self.context
    }

    pub fn recorded_language(&self) -> Option<&str> {
        self.layers
            .iter()
            .find_map(|layer| layer.facts.as_ref())
            .filter(|facts| {
                facts
                    .pack
                    .layers
                    .iter()
                    .any(|layer| layer.withheld().is_none())
            })
            .map(|facts| facts.pack.language_name())
    }

    pub fn covered(&self, path: &str) -> bool {
        self.covered_path(path.as_bytes())
    }

    pub fn covered_path(&self, path: &[u8]) -> bool {
        self.coverage.covered_path(path)
    }

    pub fn base_only(&self) -> io::Result<Self> {
        let layer = self
            .layers
            .first()
            .ok_or_else(|| invalid("Content source audit: no recorded base game layer"))?;
        let mut base = Self {
            generation: Arc::clone(&self.generation),
            coverage: self.coverage,
            layers: vec![layer.clone()],
            ..Self::default()
        };
        base.finish()?;
        Ok(base)
    }

    fn finish(&mut self) -> io::Result<()> {
        for layer in &self.layers {
            self.generation.inventory(&layer.inventory_witnesses);
            match &layer.source {
                LayerSource::Folder(root) => self.generation.watch_root(root, true),
                LayerSource::Archive(path) => {
                    if let Some(parent) = path.parent() {
                        self.generation.watch_root(parent, true);
                    }
                }
            }
            if let Some(path) = &layer.descriptor {
                if let Some(parent) = path.parent() {
                    self.generation.watch_root(parent, true);
                }
            }
        }
        fn field(hash: &mut Sha256, value: &[u8]) {
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value);
        }
        let mut context = Sha256::new();
        context.update(b"content-generation-stack/v2");
        context.update(self.coverage.0.to_le_bytes());
        context.update([u8::from(self.defines_visible)]);
        for layer in &mut self.layers {
            let mut hash = Sha256::new();
            hash.update(b"resolved-layer-content/v2");
            field(&mut hash, layer.id.as_bytes());
            field(&mut hash, format!("{:?}", layer.source).as_bytes());
            hash.update([u8::from(layer.verifies())]);
            if let Some(facts) = &layer.facts {
                hash.update(facts.layer().hash);
                if let Some(dlc_name) = &facts.layer().dlc_name {
                    field(&mut hash, dlc_name.as_bytes());
                }
            }
            if let Some(reason) = &layer.art_unmounted {
                hash.update([1]);
                field(&mut hash, reason.as_bytes());
            } else {
                hash.update([0]);
            }
            if let Some(stamp) = &layer.archive_stamp {
                hash.update(stamp.size.to_le_bytes());
                hash.update(stamp.modified_seconds.to_le_bytes());
                hash.update(stamp.modified_nanos.to_le_bytes());
            }
            for (path, digest) in layer.hashes.iter() {
                field(&mut hash, path);
                hash.update(digest);
            }
            for (path, member) in layer.files.iter() {
                field(&mut hash, path);
                hash.update(member.size.to_le_bytes());
                if let Some(stamp) = &member.stamp {
                    hash.update(stamp.size.to_le_bytes());
                    hash.update(stamp.modified_seconds.to_le_bytes());
                    hash.update(stamp.modified_nanos.to_le_bytes());
                }
            }
            layer.content_digest = hash.finalize().into();
            context.update([layer.role.byte()]);
            context.update(layer.content_digest);
            for path in &layer.replace_paths {
                field(&mut context, path);
            }
        }
        let context = context.finalize().into();
        if self.context != context {
            self.derived = Default::default();
            self.recorded_projection = Default::default();
        }
        self.context = context;
        let mut effective = BTreeMap::new();
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.coverage != self.coverage {
                return Err(invalid("content layers use different coverage policies"));
            }
            let defines_visible = self.defines_visible;
            let pack = self
                .layers
                .iter()
                .find_map(|layer| layer.facts.as_ref())
                .map(|facts| Arc::clone(&facts.pack));
            let sees = |lower: &[u8]| {
                overlay_sees(defines_visible, lower)
                    && pack
                        .as_ref()
                        .is_none_or(|pack| !fact_class(lower) || pack.is_art(lower))
            };
            let all = |_: &[u8]| true;
            apply_overlay(
                &mut effective,
                index,
                match &layer.facts {
                    Some(facts) => facts.layer().archive,
                    None => matches!(layer.source, LayerSource::Archive(_)),
                },
                layer.listed_keys(),
                &layer.replace_paths,
                if layer.overlay.is_some() {
                    Some(&sees as Sees<'_>)
                } else if layer.facts.is_some() {
                    Some(&all as Sees<'_>)
                } else {
                    None
                },
            )?;
        }
        self.effective = Arc::new(effective);
        Ok(())
    }

    #[cfg(any(test, feature = "layer-fixtures"))]
    pub fn from_test_layers(layers: Vec<ResolvedLayer>) -> io::Result<Self> {
        let mut assembled = Self {
            layers,
            ..Self::default()
        };
        assembled.finish()?;
        Ok(assembled)
    }

    #[cfg(any(test, feature = "layer-fixtures"))]
    pub fn from_test_common_folders(folders: &[(&Path, &[&str])]) -> io::Result<Self> {
        let (mut cache, mut stats) = (HashCache::default(), HashStats::default());
        let mut layers = Vec::new();
        for (index, (root, replace_paths)) in folders.iter().enumerate() {
            let root = fs::canonicalize(root)?;
            let mut layer = ResolvedLayer {
                content_digest: [0; 32],
                inventory_witnesses: Default::default(),
                inventory_paths: Default::default(),
                coverage: CoveragePolicy::default(),
                role: LayerRole::Mod,
                facts: None,
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
                display_name: None,
                art_unmounted: None,
            };
            let directory = root.join("common");
            if directory.try_exists()? {
                collect_folder(
                    &root,
                    &directory,
                    std::sync::Arc::make_mut(&mut layer.files),
                    layer.coverage,
                    false,
                    &mut BTreeSet::new(),
                )?;
            }
            layer.finish_inventory()?;
            layer.hash(&mut cache, &mut stats)?;
            layers.push(layer);
        }
        Self::from_test_layers(layers)
    }

    #[cfg(any(test, feature = "test-support", feature = "layer-fixtures"))]
    pub fn from_folders(folders: &[(&Path, &[&str])]) -> io::Result<Self> {
        Self::from_folders_with_coverage_policy(folders, COVERAGE_POLICY_VERSION)
    }

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
                content_digest: [0; 32],
                inventory_witnesses: Default::default(),
                inventory_paths: Default::default(),
                coverage: layers.coverage,
                role: LayerRole::Mod,
                facts: None,
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
                display_name: None,
                art_unmounted: None,
            };
            layer.inventory()?;
            layer.hash(&mut cache, &mut stats)?;
            layers.layers.push(layer);
        }
        layers.finish()?;
        Ok(layers)
    }

    pub fn paths(&self) -> io::Result<impl Iterator<Item = &[u8]>> {
        self.refuse_listing(b"", &[])?;
        Ok(self.effective.values().map(|file| file.path.as_slice()))
    }

    pub fn effective_paths(&self) -> io::Result<impl Iterator<Item = (&[u8], usize)>> {
        self.refuse_listing(b"", &[])?;
        Ok(self
            .effective
            .values()
            .map(|file| (file.path.as_slice(), file.layer)))
    }

    pub fn art_paths(&self) -> impl Iterator<Item = &[u8]> {
        let pack = self
            .layers
            .iter()
            .find_map(|layer| layer.facts.as_ref())
            .map(|facts| Arc::clone(&facts.pack));
        self.effective
            .values()
            .map(|file| file.path.as_slice())
            .filter(move |path| {
                pack.as_ref()
                    .is_none_or(|pack| !fact_class(path) || pack.is_art(path))
            })
    }

    fn refuse_listing(&self, prefix: &[u8], suffixes: &[String]) -> io::Result<()> {
        match self
            .layers
            .iter()
            .find_map(|layer| layer.fact_listing_refusal(prefix, suffixes))
        {
            Some(refusal) => Err(refusal),
            None => Ok(()),
        }
    }

    pub fn dlc_names(&self) -> io::Result<BTreeSet<String>> {
        let mut names = BTreeSet::new();
        for layer in &self.layers {
            names.extend(layer.dlc_name()?);
        }
        Ok(names)
    }

    pub fn listed_layer_paths(&self, prefix: &[u8]) -> io::Result<BTreeSet<LayerPath>> {
        self.product("covered file listing", 1, prefix, |reader| {
            reader.watch_listing(prefix);
            reader.listed_layer_paths_uncached(prefix)
        })
        .map(|paths| paths.as_ref().clone())
    }

    fn listed_layer_paths_uncached(&self, prefix: &[u8]) -> io::Result<BTreeSet<LayerPath>> {
        self.refuse_listing(prefix, &[])?;
        let prefix = prefix.to_ascii_lowercase();
        let mut listed = BTreeSet::new();
        let mut hidden: Vec<(LayerPath, bool)> = Vec::new();
        for layer in self.layers.iter().rev() {
            for path in layer.listed_keys() {
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
        Ok(listed)
    }

    #[cfg(any(test, feature = "layer-fixtures"))]
    pub fn listed_paths(&self, prefix: &str) -> BTreeSet<String> {
        self.listed_layer_paths(prefix.as_bytes())
            .unwrap()
            .into_iter()
            .filter_map(|path| String::from_utf8(path).ok())
            .collect()
    }

    pub fn descriptor_paths(
        &self,
        prefix: impl AsRef<[u8]>,
        extensions: &[&str],
    ) -> io::Result<Vec<ScriptPath>> {
        let extensions: Vec<String> = extensions.iter().map(|e| e.to_ascii_lowercase()).collect();
        if extensions
            .iter()
            .any(|e| !e.starts_with('.') || e.len() < 2 || e.contains('/'))
        {
            return Err(invalid("descriptor extension must be a dot and a suffix"));
        }
        self.listed_with_suffixes(prefix.as_ref(), &extensions)
    }

    fn listed_with_suffixes(
        &self,
        prefix: &[u8],
        extensions: &[String],
    ) -> io::Result<Vec<ScriptPath>> {
        let mut parameters = Vec::new();
        for part in
            std::iter::once(prefix).chain(extensions.iter().map(|extension| extension.as_bytes()))
        {
            parameters.extend_from_slice(&(part.len() as u64).to_le_bytes());
            parameters.extend_from_slice(part);
        }
        self.product("descriptor file listing", 1, &parameters, |reader| {
            reader.listed_with_suffixes_uncached(prefix, extensions)
        })
        .map(|paths| paths.as_ref().clone())
    }

    fn watch_listing(&self, prefix: &[u8]) {
        for layer in &self.layers {
            if let LayerSource::Folder(root) = &layer.source {
                self.generation
                    .witnesses
                    .inherit_listing(root, prefix, &layer.inventory_witnesses);
            }
        }
    }

    fn listed_with_suffixes_uncached(
        &self,
        prefix: &[u8],
        extensions: &[String],
    ) -> io::Result<Vec<ScriptPath>> {
        let prefix = normalize_path(prefix)?.to_ascii_lowercase();
        self.watch_listing(&prefix);
        let _collection = generation::Collect::enter(Arc::clone(&self.generation.witnesses));
        let mut listings = Vec::with_capacity(self.layers.len());
        for layer in &self.layers {
            listings.push(listed_files(layer, &prefix, extensions)?);
        }
        let folded: Vec<BTreeMap<LayerPath, &LayerPath>> = self
            .layers
            .iter()
            .zip(&listings)
            .map(|(layer, listing)| {
                let mut folded = BTreeMap::new();
                if !layer.exact_names() {
                    for path in listing {
                        folded.entry(path.to_ascii_lowercase()).or_insert(path);
                    }
                }
                folded
            })
            .collect();
        let answer = |name: &LayerPath| -> Option<usize> {
            let lower = name.to_ascii_lowercase();
            for (index, layer) in self.layers.iter().enumerate().rev() {
                if self.hides(layer, &lower) {
                    continue;
                }
                let hit = if layer.exact_names() {
                    listings[index].get(name)
                } else {
                    folded[index].get(&lower).copied()
                };
                if hit.is_some() {
                    return Some(index);
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
        let mut files: Vec<(usize, LayerPath)> = Vec::new();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            for path in &listings[index] {
                let lower = path.to_ascii_lowercase();
                if self.hides(layer, &lower)
                    || hidden.iter().any(|(root, overlay)| {
                        below(&lower, root) && (!overlay || self.overlay_sees(&lower))
                    })
                    || !seen.insert(path.clone())
                {
                    continue;
                }
                let answering = answer(path).ok_or_else(|| {
                    invalid(format!("listed file {} answers no lookup", display(path)))
                })?;
                files.push((answering, path.clone()));
            }
            hidden.extend(
                layer
                    .replace_paths
                    .iter()
                    .map(|path| (path.to_ascii_lowercase(), layer.overlay.is_some())),
            );
        }
        files.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        Ok(files
            .into_iter()
            .map(|(_, path)| ScriptPath::new(path))
            .collect())
    }

    pub fn directory_files(
        &self,
        directory: impl AsRef<[u8]>,
        suffix: &str,
    ) -> io::Result<Vec<ScriptPath>> {
        let directory = normalize_path(directory.as_ref())?;
        let listed = self.listed_with_suffixes(&directory, &[suffix.to_ascii_lowercase()])?;
        Ok(directory_order(&directory, suffix, listed))
    }

    pub fn retained_recorded_roster_product<T: Send + Sync + 'static>(
        &self,
        name: &'static str,
        schema: u32,
        read: impl FnOnce(&ResolvedLayers) -> io::Result<T>,
        complete: impl FnOnce(&T) -> bool,
    ) -> io::Result<Arc<T>> {
        let cache = self.derived::<RetainedProducts>();
        let cell = Arc::clone(
            cache
                .values
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .entry((name, schema, Vec::new()))
                .or_default(),
        );
        let mut kept = cell.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(product) = kept.as_ref() {
            let product = Arc::clone(product);
            let first_credit = self
                .read_trace
                .as_ref()
                .is_some_and(|trace| trace.credit_once(&product));
            drop(kept);
            if let Some(parent) = self
                .read_trace
                .as_ref()
                .and_then(|trace| trace.product.as_ref())
            {
                self.generation.depends(parent, &product)?;
            }
            if first_credit {
                let reader = self.recorded_or_self()?;
                reader.credit_product(&product, false, None);
            } else {
                product.diagnostics.credit_to(self);
            }
            return product.value();
        }
        let reader = self.recorded_or_self()?;
        let mut roster = Vec::new();
        for layer in &reader.layers {
            roster.extend_from_slice(&(layer.id.len() as u64).to_le_bytes());
            roster.extend_from_slice(layer.id.as_bytes());
        }
        let key = graph::Key {
            context: reader.context,
            parser: name,
            schema,
            parameters: roster,
        };
        let parent = reader
            .read_trace
            .as_ref()
            .and_then(|trace| trace.product.as_ref());
        let (product, built) = reader.generation.product_shared_if(
            key,
            parent,
            |node| {
                let reads = Arc::new(std::sync::Mutex::new(Vec::new()));
                let kept_reads = Arc::clone(&reads);
                let original = reader.read_trace.clone();
                let mut sub = reader.clone();
                sub.derived = Default::default();
                sub.read_trace = Some(ReadTrace::new(
                    Arc::new(move |read: ContentRead| {
                        kept_reads
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .push(read.clone());
                        if let Some(trace) = &original {
                            (trace.callback)(read);
                        }
                    }),
                    reader
                        .read_trace
                        .as_ref()
                        .map_or_else(String::new, |trace| trace.purpose.clone()),
                    Some(Arc::clone(node)),
                ));
                let (value, diagnostics) =
                    diagnostics::capture(|| read(&sub));
                let reads = std::mem::take(
                    &mut *reads.lock().unwrap_or_else(|error| error.into_inner()),
                );
                (value, reads, diagnostics)
            },
            |names, reads| {
                complete(names)
                    && !reads
                        .iter()
                        .any(|read| matches!(read.outcome, ReadOutcome::Error { .. }))
            },
        )?;
        if product.successful() {
            *kept = Some(Arc::clone(&product));
        }
        if let Some(trace) = &self.read_trace {
            trace.credit_once(&product);
        }
        drop(kept);
        reader.credit_product(&product, built, None);
        product.value()
    }

    pub fn retained_value<T: Send + Sync + 'static>(&self, name: &'static str, schema: u32, parameters: &[u8]) -> Option<io::Result<Arc<T>>> {
        Some(self.derived::<RetainedProducts>()
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&(name, schema, parameters.to_vec()))?
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()?
            .value::<T>())
    }

    pub fn stamped(&self, relative: &str) -> io::Result<bool> {
        let Some(path) = sanitize(relative.as_bytes()) else {
            return Ok(false);
        };
        let Some(hit) = self.lookup_path(&path)? else {
            return Ok(false);
        };
        if hit.fact.is_some() {
            return Ok(true);
        }
        let layer = &self.layers[hit.layer];
        Ok(layer.overlay.is_none()
            && self.covered_path(layer.record_name(&hit.path))
            && match layer.source {
                LayerSource::Folder(_) => {
                    layer.lookup.folders.get(&hit.path.to_ascii_lowercase()) == Some(&hit.path)
                }
                LayerSource::Archive(_) => true,
            })
    }

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
        let (hit, inventory_hit) = if !self.covered_path(&relative) {
            if self.recorded_fact(&relative) {
                return Err(unproven(&relative, self.coverage).into());
            }
            (self.scan_uncovered_probe(&relative)?, false)
        } else {
            let hit = self.lookup_path(&relative)?;
            self.verify_passed_probe(&relative, hit.as_ref().map(|hit| hit.layer))?;
            (hit, true)
        };
        observed.layer = hit.as_ref().map(|hit| hit.layer);
        if let Some(hit) = &hit
            && hit.fact.is_none()
        {
            let layer = &self.layers[hit.layer];
            if layer.reaches_link(&hit.path) {
                return Err(unfollowed_link(&relative).into());
            }
            if inventory_hit
                && matches!(layer.source, LayerSource::Archive(_))
                && layer.verifies()
                && !layer.archive_exists(&hit.path)?
            {
                return Err(changed(&hit.path).into());
            }
        }
        Ok(hit.is_some())
    }

    pub fn layer_index(&self, relative: impl AsRef<[u8]>) -> Option<usize> {
        let relative = sanitize(relative.as_ref())?;
        let hit = self
            .resolved_file(&relative, !self.covered_path(&relative))
            .ok()?;
        let hit = hit.as_ref().as_ref()?;
        hit.folder
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        Some(hit.layer)
    }

    pub fn context_digest(&self) -> [u8; 32] {
        self.context
    }

    pub fn art_stack_key(&self) -> [u8; 32] {
        fn field(hash: &mut Sha256, value: &[u8]) {
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value);
        }
        let mut hash = Sha256::new();
        hash.update(b"art-stack-key/v1");
        hash.update(self.context);
        hash.update([match artwork_mode() {
            ArtworkMode::Ordinary => 0,
            ArtworkMode::Strict => 1,
        }]);
        for layer in &self.layers {
            hash.update([layer.role.byte()]);
            field(&mut hash, layer.id.as_bytes());
            if let Some(name) = layer.display_name() {
                field(&mut hash, name.as_bytes());
            } else {
                field(&mut hash, b"");
            }
            if let Some(reason) = &layer.art_unmounted {
                hash.update([1]);
                field(&mut hash, reason.as_bytes());
            } else {
                hash.update([0]);
            }
            if let Ok(Some(dlc)) = layer.dlc_name() {
                field(&mut hash, dlc.as_bytes());
            } else {
                field(&mut hash, b"");
            }
            hash.update((layer.replace_paths.len() as u64).to_le_bytes());
            for path in &layer.replace_paths {
                field(&mut hash, path);
            }
        }
        if let Some(presentation) = &self.presentation {
            hash.update([1]);
            hash.update(presentation.art_stack_key());
        } else {
            hash.update([0]);
        }
        hash.finalize().into()
    }

    pub fn unmounted_art_above(&self, answered_layer: Option<usize>, relative: &[u8]) -> Option<usize> {
        let lower = relative.to_ascii_lowercase();
        let start = answered_layer.map_or(0, |index| index + 1);
        for (offset, layer) in self.layers[start..].iter().enumerate().rev() {
            if self.hides(layer, &lower) {
                continue;
            }
            if layer.art_unmounted.is_some() {
                return Some(start + offset);
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
    }

    pub fn art_file_source(&self, relative: &[u8]) -> io::Result<ArtFileSource> {
        let Some(path) = sanitize(relative) else {
            return Ok(ArtFileSource::Missing);
        };
        let hit = self.resolved_file(&path, !self.covered_path(&path))?;
        let answered = hit.as_ref().as_ref().map(|file| {
            file.folder
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            (file.layer, file.path.clone())
        });
        if let Some(unmounted) = self.unmounted_art_above(answered.as_ref().map(|(layer, _)| *layer), &path) {
            let reason = self.layers[unmounted]
                .art_unmounted
                .clone()
                .unwrap_or_else(|| "not mounted".to_owned());
            return Ok(ArtFileSource::Unmounted {
                layer: unmounted,
                reason,
            });
        }
        Ok(match answered {
            Some((layer, path)) => ArtFileSource::Mounted { layer, path },
            None => ArtFileSource::Missing,
        })
    }

    pub fn read(&self, relative: &str) -> io::Result<Bytes> {
        self.read_path(relative.as_bytes())
    }

    pub fn read_prefix(&self, relative: &str, limit: usize) -> io::Result<(Bytes, u64)> {
        let bytes = self.read_path(relative.as_bytes())?;
        Ok((bytes.prefix(limit), bytes.len() as u64))
    }

    pub fn read_path(&self, relative: &[u8]) -> io::Result<Bytes> {
        self.shared_read(relative, false)
    }

    pub fn read_pieces(&self, relative: &str, piece: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        let path = sanitize(relative.as_bytes()).ok_or_else(|| not_found(relative.as_bytes()))?;
        if self.covered_path(&path)
            && let Some(hit) = self.lookup_path(&path)?
            && let (Some(fact), Some(facts)) = (hit.fact, self.layers[hit.layer].facts.as_ref())
        {
            return facts.pack.read_pieces(fact, piece);
        }
        piece(&self.read(relative)?)
    }

    pub fn read_paths<P: AsRef<[u8]> + Sync>(&self, paths: &[P]) -> Vec<io::Result<Bytes>> {
        crate::parallel::map(paths, |path| self.read_path(path.as_ref()))
    }

    fn resolved_file(
        &self,
        relative: &[u8],
        uncovered: bool,
    ) -> io::Result<Arc<Option<ResolvedFile>>> {
        let path = sanitize(relative).ok_or_else(|| not_found(relative))?;
        let mut parameters = vec![u8::from(uncovered)];
        parameters.extend_from_slice(&path);
        self.product("effective file resolution", 1, &parameters, |reader| {
            let mut observed = reader.observation();
            let hit = if uncovered && !reader.covered_path(&path) {
                if reader.recorded_fact(&path) {
                    return Err(unproven(&path, reader.coverage));
                }
                reader
                    .scan_uncovered(&path, &mut observed)
                    .map_err(|failure| failure.error)?
            } else {
                reader.lookup_path(&path)?
            };
            reader
                .verify_passed(&path, hit.as_ref().map(|hit| hit.layer), &mut observed)
                .map_err(|failure| failure.error)?;
            Ok(hit.map(ResolvedFile::from))
        })
    }

    fn shared_read(&self, relative: &[u8], explicit_uncovered: bool) -> io::Result<Bytes> {
        self.shared_read_in(relative, explicit_uncovered, None)
    }

    fn shared_read_in(
        &self,
        relative: &[u8],
        explicit_uncovered: bool,
        file: Option<&ResolvedFile>,
    ) -> io::Result<Bytes> {
        let path = sanitize(relative).ok_or_else(|| not_found(relative))?;
        let mut observed = self.observation();
        let result = (|| -> ObservedResult<ReadSuccess<Bytes>> {
            struct ReleaseFolder<'a>(&'a std::sync::Mutex<Option<File>>);
            impl Drop for ReleaseFolder<'_> {
                fn drop(&mut self) {
                    self.0
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .take();
                }
            }
            let resolved;
            let file = match file {
                Some(file) => file,
                None => {
                    resolved = self.resolved_file(&path, explicit_uncovered)?;
                    let Some(file) = resolved.as_ref() else {
                        return Err(not_found(&path).into());
                    };
                    file
                }
            };
            let _release = ReleaseFolder(&file.folder);
            observed.at(file.layer);
            let layer = &self.layers[file.layer];
            let real = layer.record_name(&file.path);
            let uncovered = !self.covered_path(real) && layer.overlay.is_none();
            if uncovered
                && !explicit_uncovered
                && self
                    .uncovered_reads
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .is_none()
            {
                return Err(invalid(format!(
                    "content file is outside recorded coverage policy {} and this reader does not report uncovered reads: {}",
                    self.coverage.0, display(real),
                )).into());
            }
            let key = graph::Key {
                context: self.layer_context(file.layer),
                parser: "resolved bytes",
                schema: 1,
                parameters: file.path.clone(),
            };
            let parent = self
                .read_trace
                .as_ref()
                .and_then(|trace| trace.product.as_ref());
            let (product, _) = self.generation.product(key, parent, |node| {
                let ((result, reads), diagnostics) = diagnostics::capture(|| {
                    let raw = self.load_resolved(
                        file,
                        node,
                        &mut ReadObservation {
                            enabled: true,
                            layer: Some(file.layer),
                        },
                    );
                    let (result, outcome) = match raw {
                        Ok(ReadSuccess::Traced { value, payload }) => {
                            let outcome = ReadOutcome::Bytes {
                                length: payload.length,
                                digest: payload.digest,
                            };
                            (Ok(value), outcome)
                        }
                        Ok(ReadSuccess::Untraced(value)) => {
                            let bytes = value;
                            let outcome = ReadOutcome::Bytes {
                                length: bytes.len() as u64,
                                digest: bytes.digest(),
                            };
                            (Ok(bytes), outcome)
                        }
                        Err(failure) => {
                            let outcome = failure.outcome();
                            (Err(failure.error), outcome)
                        }
                    };
                    (
                        result,
                        vec![ContentRead {
                            path: real.to_vec(),
                            purpose: String::new(),
                            layer: None,
                            outcome,
                        }],
                    )
                });
                (result, reads, diagnostics)
            })?;
            file.folder
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            product.diagnostics.credit();
            let bytes = product.value::<Bytes>().map_err(|error| ReadFailure {
                error,
                payload: product.reads.first().and_then(|read| match &read.outcome {
                    ReadOutcome::Error { payload, .. } => payload.clone(),
                    _ => None,
                }),
            })?;
            self.note_overlay(file.layer, real);
            if uncovered
                || (explicit_uncovered && !self.covered_path(&path) && layer.overlay.is_none())
            {
                diagnostics::Diagnostics::uncovered(self.coverage.0, &path);
                if let Some(paths) = self
                    .uncovered_reads
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_mut()
                {
                    paths.insert(path.clone());
                }
            }
            Ok(ReadSuccess::Traced {
                value: bytes.as_ref().clone().with_origin(
                    product.request(),
                    Arc::clone(&self.generation) as Arc<dyn std::any::Any + Send + Sync>,
                ),
                payload: ReadPayload {
                    length: bytes.len() as u64,
                    digest: bytes.digest(),
                },
            })
        })();
        if explicit_uncovered {
            self.note_uncovered_attempt(&path, observed.layer);
        }
        self.finish_read(&path, observed, result)
    }

    fn note_uncovered_attempt(&self, path: &[u8], layer: Option<usize>) {
        if !self.covered_path(path) && layer.is_none_or(|index| self.layers[index].overlay.is_none()) {
            diagnostics::Diagnostics::uncovered(self.coverage.0, path);
            if let Some(paths) = self.uncovered_reads.lock().unwrap_or_else(|error| error.into_inner()).as_mut() {
                paths.insert(path.to_vec());
            }
        }
    }

    fn load_resolved(
        &self,
        file: &ResolvedFile,
        node: &Arc<graph::Node>,
        observed: &mut ReadObservation,
    ) -> ObservedResult<ReadSuccess<Bytes>> {
        let layer = &self.layers[file.layer];
        if let Some(fact) = file.fact {
            #[cfg(test)]
            tests::LOADS.with(|loads| loads.set(loads.get() + 1));
            let (bytes, payload) = layer
                .facts
                .as_ref()
                .expect("a fact hit is a played-back layer's")
                .pack
                .read_payload(fact)?;
            io_counts::layer_digest(layer, &file.path, payload.digest, payload.length);
            return Ok(observed.hashed(
                Bytes::verified(bytes, payload.digest),
                payload.length,
                payload.digest,
            ));
        }
        match &layer.source {
            LayerSource::Folder(root) => {
                let handle = file
                    .folder
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take();
                let handle = match handle {
                    Some(handle) => {
                        self.witness_file(layer, &file.path, &handle)?;
                        handle
                    }
                    None => {
                        let Some((real, handle)) = layer.folder_lookup(&file.path)? else {
                            return Err(changed(&file.path).into());
                        };
                        self.witness_file(layer, &real, &handle)?;
                        handle
                    }
                };
                let bytes = generation::source(
                    &self.generation,
                    &root.join(folder_text(&file.path)?),
                    Some(handle),
                    Some(node),
                )?;
                let result = observed.hashed(bytes.clone(), bytes.len() as u64, bytes.digest());
                if self.covered_path(&file.path) {
                    result.verified(layer.verify(&file.path, &bytes.digest()))
                } else {
                    Ok(result)
                }
            }
            LayerSource::Archive(path) => {
                let archive = File::open(path)?;
                self.generation.witnesses.record_file(path, &archive)?;
                layer
                    .archive_open_observed(&file.path, observed)?
                    .map(|bytes| bytes.map(Bytes::from))
                    .ok_or_else(|| changed(&file.path).into())
            }
        }
    }

    fn witness_file(&self, layer: &ResolvedLayer, path: &[u8], file: &File) -> io::Result<()> {
        if let LayerSource::Folder(root) = &layer.source {
            let relative = std::str::from_utf8(path)
                .map_err(|_| invalid("folder content path is not UTF-8"))?;
            self.generation
                .witnesses
                .record_file(&root.join(relative), file)?;
        }
        Ok(())
    }

    fn layer_context(&self, index: usize) -> [u8; 32] {
        self.layers[index].content_digest
    }

    #[must_use]
    pub fn report_uncovered_reads(&self) -> UncoveredReads {
        self.uncovered_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_or_insert_with(BTreeSet::new);
        self.read_warnings()
    }

    pub fn read_warnings(&self) -> UncoveredReads {
        UncoveredReads {
            policy: self.coverage.0,
            paths: self.uncovered_reads.clone(),
            overlays: self.overlay_reads.clone(),
        }
    }

    fn note_overlay(&self, layer: usize, path: &[u8]) {
        if let Some(overlay) = &self.layers[layer].overlay {
            diagnostics::Diagnostics::overlay(&overlay.name, path);
            self.overlay_reads
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .reads
                .entry(overlay.name.clone())
                .or_default()
                .insert(path.to_vec());
        }
    }

    pub fn note_refused(&self, key: String) {
        diagnostics::Diagnostics::refused(key.clone());
        self.overlay_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .refused
            .insert(key);
    }

    pub fn refused(&self) -> BTreeSet<String> {
        self.overlay_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .refused
            .clone()
    }

    pub fn note_overlay_refusal(&self, refusal: String) {
        credit_overlay_refusals(&BTreeSet::from([refusal.clone()]));
        self.overlay_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .refusals
            .insert(refusal);
    }

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

    pub fn graphics_define<D, T, E: std::fmt::Display>(
        &self,
        name: &str,
        graphics: &D,
        recorded: &D,
        read: impl Fn(&D) -> Result<T, E>,
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

    pub fn supplied_by_visual_mod(&self, relative: impl AsRef<[u8]>) -> bool {
        self.shows_visual_mods()
            && self
                .layer_index(relative)
                .is_some_and(|layer| self.layers[layer].overlay.is_some())
    }

    pub fn shows_visual_mods(&self) -> bool {
        self.layers.iter().any(|layer| layer.overlay.is_some())
    }

    pub fn visual_mod_names(&self) -> Vec<String> {
        self.layers
            .iter()
            .filter_map(|layer| layer.visual_mod_name().map(str::to_owned))
            .collect()
    }

    pub fn with_own_overlay_diagnostics(&self) -> ResolvedLayers {
        let enabled = self.uncovered_reads.lock().unwrap_or_else(|error| error.into_inner()).is_some();
        self.with_reporting(
            OverlayReads::default(),
            Arc::new(std::sync::Mutex::new(enabled.then(BTreeSet::new))),
        )
    }

    fn with_reporting(
        &self,
        overlays: OverlayReads,
        paths: Arc<std::sync::Mutex<Option<BTreeSet<LayerPath>>>>,
    ) -> Self {
        Self {
            overlay_reads: Arc::clone(&overlays),
            uncovered_reads: Arc::clone(&paths),
            presentation: self.presentation.as_ref().map(|layers| {
                Arc::new(layers.with_reporting(Arc::clone(&overlays), Arc::clone(&paths)))
            }),
            presentation_defines: self.presentation_defines.as_ref().map(|layers| {
                Arc::new(layers.with_reporting(Arc::clone(&overlays), Arc::clone(&paths)))
            }),
            ..self.clone()
        }
    }

    pub fn recorded_only(&self) -> ResolvedLayers {
        ResolvedLayers {
            presentation: None,
            presentation_defines: None,
            ..self.clone()
        }
    }

    pub fn presentation(&self) -> &ResolvedLayers {
        self.presentation.as_deref().unwrap_or(self)
    }

    pub fn presentation_defines(&self) -> Option<&ResolvedLayers> {
        self.presentation_defines.as_deref()
    }

    pub fn without_overlays(&self) -> Option<ResolvedLayers> {
        self.project_recorded().ok().flatten()
    }

    fn project_recorded(&self) -> io::Result<Option<ResolvedLayers>> {
        if self.layers.iter().all(|layer| layer.overlay.is_none()) {
            return Ok(None);
        }
        let projection = self.recorded_projection.get_or_init(|| {
            let mut recorded = ResolvedLayers {
                generation: Arc::clone(&self.generation),
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
                ..ResolvedLayers::default()
            };
            recorded
                .finish()
                .map(|()| Arc::new(recorded))
                .map_err(Arc::new)
        });
        let mut recorded = projection
            .as_ref()
            .map_err(|error| io::Error::new(error.kind(), Arc::clone(error)))?
            .as_ref()
            .clone();
        let kept = self
            .layers
            .iter()
            .enumerate()
            .filter_map(|(index, layer)| layer.overlay.is_none().then_some(index))
            .collect();
        recorded.overlay_reads = Arc::clone(&self.overlay_reads);
        recorded.uncovered_reads = Arc::clone(&self.uncovered_reads);
        recorded.read_trace = self.read_trace.as_ref().map(|trace| trace.for_layers(kept));
        Ok(Some(recorded))
    }

    fn overlay_sees(&self, lower: &[u8]) -> bool {
        (self.defines_visible && below(lower, DEFINES_ROOT.as_bytes()))
            || (overlay_sees(self.defines_visible, lower) && !self.recorded_fact(lower))
    }

    fn recorded_fact(&self, path: &[u8]) -> bool {
        self.layers
            .iter()
            .find_map(|layer| layer.facts.as_ref())
            .is_some_and(|facts| fact_class(path) && !facts.pack.is_art(path))
    }

    fn hides(&self, layer: &ResolvedLayer, lower: &[u8]) -> bool {
        layer.overlay.is_some() && !self.overlay_sees(lower)
    }

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

    fn with_defines(&self) -> io::Result<ResolvedLayers> {
        let mut stack = ResolvedLayers {
            generation: Arc::clone(&self.generation),
            coverage: self.coverage,
            layers: self.layers.clone(),
            above_mods: self.above_mods,
            defines_visible: true,
            overlay_reads: self.overlay_reads.clone(),
            uncovered_reads: self.uncovered_reads.clone(),
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
            generation: Arc::clone(&self.generation),
            coverage: self.coverage,
            layers,
            above_mods: self.above_mods,
            presentation: None,
            presentation_defines: None,
            defines_visible: false,
            overlay_reads: self.overlay_reads.clone(),
            uncovered_reads: self.uncovered_reads.clone(),
            ..ResolvedLayers::default()
        };
        presentation.finish()?;
        Ok(presentation)
    }

    pub fn note_shader_warning(&self, warning: String) {
        diagnostics::Diagnostics::shader(warning);
    }

    fn lookup_path(&self, relative: &[u8]) -> io::Result<Option<RecordedHit>> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            if self.hides(layer, &lower) {
                continue;
            }
            if layer.fact_path(relative) {
                if let Some((path, file)) = layer.fact_lookup(relative)? {
                    return Ok(Some(RecordedHit {
                        layer: index,
                        path,
                        folder: None,
                        fact: Some(file),
                    }));
                }
                if layer
                    .replace_paths
                    .iter()
                    .any(|path| below(&lower, &path.to_ascii_lowercase()))
                {
                    break;
                }
                continue;
            }
            let hit = match &layer.source {
                LayerSource::Folder(_) => {
                    self.watch_directory(layer, relative)?;
                    layer
                        .folder_lookup(relative)?
                        .map(|(path, handle)| RecordedHit {
                            layer: index,
                            path,
                            folder: Some(handle),
                            fact: None,
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
                        fact: None,
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

    pub fn read_uncovered(&self, relative: &str) -> io::Result<Bytes> {
        self.read_uncovered_path(relative.as_bytes())
    }

    pub fn read_uncovered_path(&self, relative: &[u8]) -> io::Result<Bytes> {
        self.shared_read(relative, true)
    }

    fn scan_uncovered(
        &self,
        relative: &[u8],
        observed: &mut ReadObservation,
    ) -> ObservedResult<Option<RecordedHit>> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            if self.hides(layer, &lower) {
                continue;
            }
            match &layer.source {
                LayerSource::Folder(_) => {
                    self.watch_directory(layer, relative)?;
                    if let Some((path, file)) = layer.folder_lookup(relative)? {
                        return Ok(Some(RecordedHit {
                            layer: index,
                            path,
                            folder: Some(file),
                            fact: None,
                        }));
                    }
                }
                LayerSource::Archive(_) if layer.archive_exists(relative)? => {
                    return Ok(Some(RecordedHit {
                        layer: index,
                        path: relative.to_vec(),
                        folder: None,
                        fact: None,
                    }));
                }
                LayerSource::Archive(_) => {}
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
        Ok(None)
    }

    pub fn uncovered_from_base(&self, relative: &str) -> io::Result<bool> {
        let hit = self.resolved_file(relative.as_bytes(), true)?;
        if let Some(file) = hit.as_ref() {
            file.folder
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
        }
        Ok(hit.as_ref().as_ref().is_some_and(|file| file.layer == 0))
    }

    fn scan_uncovered_probe(&self, relative: &[u8]) -> io::Result<Option<RecordedHit>> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            if self.hides(layer, &lower) {
                continue;
            }
            match &layer.source {
                LayerSource::Folder(_) => {
                    self.watch_directory(layer, relative)?;
                    if let Some((path, _)) = layer.folder_lookup(relative)? {
                        return Ok(Some(RecordedHit {
                            layer: index,
                            path,
                            folder: None,
                            fact: None,
                        }));
                    }
                }
                LayerSource::Archive(_) if layer.archive_exists(relative)? => {
                    return Ok(Some(RecordedHit {
                        layer: index,
                        path: relative.to_vec(),
                        folder: None,
                        fact: None,
                    }));
                }
                LayerSource::Archive(_) => {}
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

    fn verify_passed_probe(&self, relative: &[u8], answered: Option<usize>) -> io::Result<()> {
        let lower = relative.to_ascii_lowercase();
        for (index, layer) in self.layers.iter().enumerate().rev() {
            if Some(index) == answered {
                break;
            }
            if self.hides(layer, &lower) {
                continue;
            }
            layer.verify_miss_probe(relative)?;
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
        return Err(
            if error.kind() != io::ErrorKind::NotFound && path.is_dir() {
                not_found(relative.as_bytes())
            } else {
                error
            },
        );
    }
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

fn listed_files(
    layer: &ResolvedLayer,
    prefix: &[u8],
    extensions: &[String],
) -> io::Result<BTreeSet<LayerPath>> {
    let mut paths = BTreeSet::new();
    let mut folded = BTreeSet::new();
    let overlay = layer.listed_as_game();
    let mut admit = |relative: LayerPath| -> io::Result<()> {
        let lower = relative.to_ascii_lowercase();
        if below(&lower, prefix) && extensions.iter().any(|e| lower.ends_with(e.as_bytes())) {
            if !folded.insert(lower) && !overlay {
                return Err(invalid(format!(
                    "layer holds case-ambiguous paths: {}",
                    display(&relative)
                )));
            }
            paths.insert(relative);
        }
        Ok(())
    };
    if let Some(refusal) = layer.fact_listing_refusal(prefix, extensions) {
        return Err(refusal);
    }
    if let Some(facts) = &layer.facts
        && let Ok(carried) = facts.layer().paths()
    {
        for path in carried {
            admit(path.clone())?;
        }
    }
    match &layer.source {
        LayerSource::Folder(root) => {
            let Some(directory) = folded_path(root, folder_text(prefix)?)? else {
                return Ok(paths);
            };
            if !directory.is_dir() {
                return Ok(paths);
            }
            let covered_inventory = layer
                .coverage
                .roots()
                .chain(std::iter::once(DESCRIPTOR_ROOT))
                .any(|known| below(prefix, known.as_bytes()));
            if covered_inventory
                && let Some(inventory_paths) = &layer.inventory_paths
            {
                for relative in inventory_paths.iter() {
                    if !layer.fact_path(relative) {
                        admit(relative.clone())?;
                    }
                }
            } else {
                walk_folder(
                    root,
                    &directory,
                    overlay,
                    &entry_kind,
                    &mut |relative, _| {
                        if layer.fact_path(&relative) {
                            return Ok(());
                        }
                        admit(relative)
                    },
                )?;
            }
        }
        LayerSource::Archive(_) => {
            let entries = match layer.verified_archive() {
                Ok((_, mounted)) => mounted.entries,
                Err(_) if layer.facts.is_some() => Vec::new(),
                Err(error) => return Err(error),
            };
            for entry in entries {
                if !entry.directory && !layer.fact_path(&entry.name) {
                    admit(entry.name)?;
                }
            }
        }
    }
    Ok(paths)
}

type Sees<'a> = &'a dyn Fn(&[u8]) -> bool;

fn apply_overlay<'a>(
    effective: &mut BTreeMap<LayerPath, EffectiveFile>,
    index: usize,
    archive: bool,
    paths: impl Iterator<Item = &'a LayerPath>,
    replace_paths: &[LayerPath],
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
        #[cfg(test)]
        hash_cache_tests::STAMPED
            .lock()
            .expect("stamped paths")
            .push(path.to_owned());
        Self::of(&fs::metadata(path)?)
    }

    fn of(metadata: &fs::Metadata) -> io::Result<Self> {
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

struct CachedHash {
    stamp: Stamp,
    hash: [u8; 32],
}

const HASH_CACHE: &str = "content-hashes-v2.bin";
const HASH_CACHE_MAGIC: &[u8; 8] = b"HOI4HC\x00\x02";

#[derive(Default)]
struct HashCache<'a> {
    entries: BTreeMap<String, CachedHash>,
    dirty: bool,
    cancelled: &'a [&'a AtomicBool],
}

fn take<'a>(rest: &mut &'a [u8], count: usize) -> Option<&'a [u8]> {
    if rest.len() < count {
        return None;
    }
    let (head, tail) = rest.split_at(count);
    *rest = tail;
    Some(head)
}

impl HashCache<'_> {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = HASH_CACHE_MAGIC.to_vec();
        bytes.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        for (key, entry) in &self.entries {
            bytes.extend_from_slice(&(key.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key.as_bytes());
            bytes.extend_from_slice(&entry.stamp.size.to_le_bytes());
            bytes.extend_from_slice(&entry.stamp.modified_seconds.to_le_bytes());
            bytes.extend_from_slice(&entry.stamp.modified_nanos.to_le_bytes());
            bytes.extend_from_slice(&entry.hash);
        }
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut rest = bytes.strip_prefix(HASH_CACHE_MAGIC)?;
        let count = u64::from_le_bytes(take(&mut rest, 8)?.try_into().ok()?);
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            let length = u32::from_le_bytes(take(&mut rest, 4)?.try_into().ok()?);
            let key = std::str::from_utf8(take(&mut rest, length as usize)?).ok()?;
            let size = u64::from_le_bytes(take(&mut rest, 8)?.try_into().ok()?);
            let modified_seconds = u64::from_le_bytes(take(&mut rest, 8)?.try_into().ok()?);
            let modified_nanos = u32::from_le_bytes(take(&mut rest, 4)?.try_into().ok()?);
            let hash = take(&mut rest, 32)?.try_into().ok()?;
            let stamp = Stamp {
                size,
                modified_seconds,
                modified_nanos,
            };
            entries.insert(key.to_owned(), CachedHash { stamp, hash });
        }
        rest.is_empty().then_some(Self {
            entries,
            dirty: false,
            cancelled: &[],
        })
    }

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

    fn probe(&self, path: &Path) -> io::Result<Probe> {
        self.probe_listed(path, None)
    }

    fn probe_listed(&self, path: &Path, listed: Option<&Stamp>) -> io::Result<Probe> {
        self.continuing()?;
        #[cfg(test)]
        hash_cache_tests::PROBED
            .lock()
            .expect("probed paths")
            .push((path.to_owned(), std::thread::current().id()));
        let key = text_path(path)?.to_owned();
        if let Some(stamp) = listed
            && let Some(hash) = self.get(&key, stamp)
        {
            return Ok(Probe {
                key,
                stamp: stamp.clone(),
                hash,
                read: false,
            });
        }
        let stamp = Stamp::read(path)?;
        if let Some(hash) = self.get(&key, &stamp) {
            return Ok(Probe {
                key,
                stamp,
                hash,
                read: false,
            });
        }
        let (hash, size) = if generation::current().is_some() {
            let bytes = generation::metadata(path)?;
            (bytes.digest(), bytes.len() as u64)
        } else {
            let (hash, size) = digest_reader(&mut File::open(path)?)?;
            io_counts::external_digest(path, hash, size);
            (hash, size)
        };
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

    fn probe_all(&self, paths: &[(PathBuf, Option<Stamp>)]) -> Vec<io::Result<Probe>> {
        let priority = job_priority::JobPriority::current();
        crate::parallel::map_entering(
            paths,
            || priority.clone().enter(job_priority::Level::Normal),
            |(path, listed)| self.probe_listed(path, listed.as_ref()),
        )
    }

    fn continuing(&self) -> io::Result<()> {
        if self
            .cancelled
            .iter()
            .any(|cancelled| cancelled.load(Ordering::Acquire))
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "content verification was stopped",
            ));
        }
        Ok(())
    }

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

fn listed_mod_descriptor(name: &str) -> bool {
    name.ends_with(".mod")
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
                || !listed_mod_descriptor(&entry.file_name().to_string_lossy())
            {
                continue;
            }
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
    role: LayerRole,
    id: String,
    source: LayerSource,
    descriptor: Option<PathBuf>,
    coverage: CoveragePolicy,
) -> io::Result<ResolvedLayer> {
    let descriptor_hash = descriptor
        .as_ref()
        .map(|path| {
            generation::metadata(path).map(|bytes| DescriptorHash {
                bytes: bytes.digest(),
                portable: None,
            })
        })
        .transpose()?;
    Ok(ResolvedLayer {
        content_digest: [0; 32],
        inventory_witnesses: Default::default(),
        inventory_paths: Default::default(),
        coverage,
        role,
        facts: None,
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
        display_name: None,
        art_unmounted: None,
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
            layer.descriptor_hash.unwrap().bytes,
        );
    }
    layers.layers.push(layer);
    Ok(())
}

pub fn gather(options: &GatherOptions) -> io::Result<GatheredContent> {
    gather_selected(options, None, &[])
}

pub fn gather_cancellable(
    options: &GatherOptions,
    cancelled: Arc<AtomicBool>,
) -> io::Result<GatheredContent> {
    gather_selected(options, None, &[&*cancelled])
}

pub fn resolve(identity: &Identity, options: &ResolveOptions) -> io::Result<ResolvedContent> {
    resolve_cancellable(identity, options, &[])
}

pub fn resolve_cancellable(
    identity: &Identity,
    options: &ResolveOptions,
    cancelled: &[&AtomicBool],
) -> io::Result<ResolvedContent> {
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
        cancelled,
    )?;
    Ok(ResolvedContent {
        layers: content.layers,
        stats: content.stats,
        changed: content.changed,
        notes: Vec::new(),
    })
}

pub fn playback(
    identity: &Identity,
    facts: Result<Arc<facts::Pack>, String>,
    options: &ResolveOptions,
) -> io::Result<ResolvedContent> {
    let _generation = Generation::inherit();
    identity.validate()?;
    let coverage = CoveragePolicy::recorded(identity.coverage_policy)?;
    let graph = generation::current().expect("playback generation in scope");
    graph.watch_root(&options.game_dir, true);
    graph.watch_root(&options.user_dir, false);
    let game_dir = graph.witnesses.root(&options.game_dir);
    let user_dir = graph.witnesses.root(&options.user_dir);
    if let Some(root) = game_dir.as_deref().and_then(visual::workshop_root) {
        graph.watch_root(&root, false);
    }
    let missing = options.cache_dir.join("art-not-installed");
    let pack = facts
        .and_then(|pack| match pack.binding_differs(identity) {
            Some(why) => Err(why),
            None => Ok(pack),
        })
        .unwrap_or_else(|reason| {
            Arc::new(facts::Pack::withheld(
                identity,
                &reason,
                game_dir.as_deref().map(builtin_roots).unwrap_or_default(),
            ))
        });
    let mut notes = Vec::new();
    let mut unshown = Vec::new();
    let mut layers = ResolvedLayers {
        coverage,
        ..ResolvedLayers::default()
    };
    let descriptors = Descriptors::installed(game_dir.as_deref(), user_dir.as_deref());
    for (index, recorded) in pack.layers.iter().enumerate() {
        let installed = match &game_dir {
            Some(game_dir) => {
                installed_source(recorded, &descriptors, game_dir, coverage)
            }
            None => Err("the game folder is not set".to_owned()),
        };
        let mut layer = ResolvedLayer {
            content_digest: [0; 32],
            inventory_witnesses: Default::default(),
            inventory_paths: Default::default(),
            coverage,
            role: recorded.role,
            facts: Some(Facts {
                pack: Arc::clone(&pack),
                layer: index,
            }),
            id: recorded.id.clone(),
            source: LayerSource::Folder(missing.clone()),
            descriptor: None,
            replace_paths: recorded.replace_paths.clone(),
            files: Default::default(),
            lookup: Default::default(),
            hashes: Default::default(),
            archive_stamp: None,
            descriptor_hash: None,
            overlay: None,
            display_name: Some(layer_display_name(recorded, identity)),
            art_unmounted: None,
        };
        match installed {
            Ok((source, descriptor)) => {
                layer.source = source;
                layer.descriptor = descriptor;
                if let Err(error) = layer.inventory() {
                    let why = error.to_string();
                    unshown.push(format!("{} ({why})", layer_name(recorded, identity)));
                    layer.source = LayerSource::Folder(missing.clone());
                    layer.descriptor = None;
                    layer.files = Default::default();
                    layer.lookup = Default::default();
                    layer.archive_stamp = None;
                    layer.art_unmounted = Some(why);
                } else {
                    Arc::make_mut(&mut layer.files).retain(|path, _| pack.is_art(path));
                    Arc::make_mut(&mut layer.lookup)
                        .folders
                        .retain(|_, path| pack.is_art(path));
                }
            }
            Err(why) => {
                unshown.push(format!("{} ({why})", layer_name(recorded, identity)));
                layer.art_unmounted = Some(why);
            }
        }
        if let Some(reason) = recorded.withheld() {
            notes.push(format!(
                "Game facts of {} are withheld: {reason}",
                layer_name(recorded, identity)
            ));
        }
        if recorded.role == LayerRole::AboveMods {
            layers.above_mods += 1;
        }
        layers.layers.push(layer);
    }
    layers.finish()?;
    notes.push(
        "Artwork, fonts and sounds come from the game installed now, not from the recording; names, numbers and the map are the recording's own."
            .to_owned(),
    );
    if !unshown.is_empty() {
        notes.push(format!(
            "Artwork of {} is not installed, so placeholders are shown.",
            unshown.join("; ")
        ));
    }
    if let Err(reason) = pack.tables() {
        notes.push(format!(
            "The game's executable tables are withheld: {reason}"
        ));
    }
    Ok(ResolvedContent {
        layers,
        stats: HashStats::default(),
        changed: Vec::new(),
        notes,
    })
}

fn layer_display_name(layer: &facts::PackLayer, identity: &Identity) -> String {
    match layer.role {
        LayerRole::Vanilla => "base game".to_owned(),
        LayerRole::Dlc => layer
            .dlc_name
            .clone()
            .unwrap_or_else(|| layer.id.clone()),
        LayerRole::Mod => identity
            .mods
            .iter()
            .find(|recorded| recorded.id == layer.id)
            .map_or_else(|| layer.id.clone(), |recorded| recorded.display_name.clone()),
        _ => layer.id.clone(),
    }
}

fn layer_name(layer: &facts::PackLayer, identity: &Identity) -> String {
    match layer.role {
        LayerRole::Vanilla => "the base game".to_owned(),
        LayerRole::Dlc => format!("DLC {}", layer.id),
        LayerRole::Mod => identity
            .mods
            .iter()
            .find(|recorded| recorded.id == layer.id)
            .map_or_else(
                || format!("mod {}", layer.id),
                |recorded| format!("mod {}", recorded.display_name),
            ),
        _ => format!("the game's {}", layer.id),
    }
}

fn builtin_roots(game_dir: &Path) -> Vec<(LayerRole, String)> {
    let mut roots = Vec::new();
    let mut integrated = Vec::new();
    if find_descriptors(&game_dir.join("integrated_dlc"), &mut integrated).is_ok()
        && sort_descriptors(&mut integrated).is_ok()
    {
        for descriptor in integrated {
            if let Some(prefix) = descriptor
                .parent()
                .and_then(|parent| parent.strip_prefix(game_dir).ok())
                .and_then(|relative| relative.to_str())
                .and_then(|relative| normalize_relative(relative).ok())
            {
                roots.push((LayerRole::Builtin, prefix));
            }
        }
    }
    if game_dir.join("tweakergui_assets").is_dir() {
        roots.push((LayerRole::AboveMods, "tweakergui_assets".to_owned()));
    } else if game_dir.join("tweakergui_assets.zip").is_file() {
        roots.push((LayerRole::AboveMods, "tweakergui_assets.zip".to_owned()));
    }
    roots
}

#[derive(Default)]
struct Descriptors {
    dlc: BTreeMap<String, Vec<PathBuf>>,
    mods: Vec<visual::Candidate>,
}

impl Descriptors {
    fn installed(game_dir: Option<&Path>, user_dir: Option<&Path>) -> Self {
        let mut found = Self::default();
        let mut dlc = Vec::new();
        if let Some(game_dir) = game_dir
            && find_descriptors(&game_dir.join("dlc"), &mut dlc).is_ok()
        {
            for path in dlc {
                if let Some(id) = path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                {
                    found.dlc.entry(id.to_owned()).or_default().push(path);
                }
            }
        }
        if let Ok(mods) = visual::discover_installed(game_dir, user_dir, &AtomicBool::new(false)) {
            found.mods = mods;
        }
        found
    }

    fn find_mod(&self, id: &str) -> Result<(LayerSource, Option<PathBuf>), String> {
        let matched: Vec<&visual::Candidate> = self
            .mods
            .iter()
            .filter(|candidate| {
                candidate.item.recorded_id == id
                    || candidate.workshop_item.as_deref() == Some(id)
            })
            .collect();
        match matched.as_slice() {
            [candidate] => {
                if let Some(problem) = &candidate.item.problem {
                    return Err(problem.clone());
                }
                let source = candidate
                    .source
                    .clone()
                    .ok_or_else(|| "not installed".to_owned())?;
                let descriptor = (!candidate.descriptor.as_os_str().is_empty())
                    .then(|| candidate.descriptor.clone());
                Ok((source, descriptor))
            }
            [] => Err("not installed".to_owned()),
            _ => Err("several installed descriptors have its id".to_owned()),
        }
    }
}

fn installed_source(
    layer: &facts::PackLayer,
    descriptors: &Descriptors,
    game_dir: &Path,
    coverage: CoveragePolicy,
) -> Result<(LayerSource, Option<PathBuf>), String> {
    let one = |found: Option<&Vec<PathBuf>>| match found.map(Vec::as_slice) {
        Some([path]) => Ok(path.clone()),
        Some(_) => Err("several installed descriptors have its id".to_owned()),
        None => Err("not installed".to_owned()),
    };
    let found = match layer.role {
        LayerRole::Vanilla => return Ok((LayerSource::Folder(game_dir.to_owned()), None)),
        LayerRole::Builtin | LayerRole::AboveMods => {
            let path = game_dir.join(&layer.id);
            return if path.is_dir() {
                Ok((LayerSource::Folder(path), None))
            } else if path.is_file() {
                Ok((LayerSource::Archive(path), None))
            } else {
                Err("not installed".to_owned())
            };
        }
        LayerRole::Dlc => {
            let path = one(descriptors.dlc.get(&layer.id))?;
            descriptor_layer(layer.id.clone(), path, game_dir, false, coverage)
        }
        LayerRole::Mod => return descriptors.find_mod(&layer.id),
        LayerRole::UserDirectory | LayerRole::Visual => {
            return Err("not a recorded layer".to_owned());
        }
    };
    found
        .map(|(resolved, _)| (resolved.source, resolved.descriptor))
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => "not installed".to_owned(),
            _ => error.to_string(),
        })
}

fn gather_selected(
    options: &GatherOptions,
    recorded: Option<&Identity>,
    cancelled: &[&AtomicBool],
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
    let _ = fs::create_dir_all(&options.cache_dir);
    let cache_path = options.cache_dir.join(HASH_CACHE);
    let mut cache: HashCache = fs::read(&cache_path)
        .ok()
        .and_then(|bytes| HashCache::decode(&bytes))
        .unwrap_or_else(|| HashCache {
            dirty: true,
            ..HashCache::default()
        });
    cache.cancelled = cancelled;
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
            content_digest: [0; 32],
            inventory_witnesses: Default::default(),
            inventory_paths: Default::default(),
            coverage,
            role: LayerRole::Vanilla,
            facts: None,
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
            display_name: Some("base game".to_owned()),
            art_unmounted: None,
        };
        vanilla.inventory()?;
        vanilla.hash(&mut cache, &mut stats)?;
        let mut vanilla_files = (*vanilla.hashes).clone();
        let mut layers = ResolvedLayers {
            context: [0; 32],
            recorded_projection: Default::default(),
            generation: Default::default(),
            coverage,
            layers: vec![vanilla],
            effective: Arc::new(BTreeMap::new()),
            uncovered_reads: Default::default(),
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
                LayerRole::UserDirectory,
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
                    LayerRole::Builtin,
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
        if has_dependencies && options.dlc_order.is_none() {
            order_notes.push(
                "DLC dependencies can change mount order; observed mount order is required"
                    .to_owned(),
            );
        }
        let mut selected = Vec::new();
        for (index, descriptor) in launcher.enabled_mods.into_iter().enumerate() {
            let descriptor = normalize_relative(&descriptor)?;
            if !descriptor.split_once('/').is_some_and(|(folder, file)| {
                folder.eq_ignore_ascii_case("mod") && !file.contains('/')
            }) {
                order_notes.push(format!(
                    "enabled mod descriptor {descriptor} is not directly in mod/, where the game lists mods, so the game does not mount it"
                ));
                continue;
            }
            if !listed_mod_descriptor(&descriptor) {
                order_notes.push(format!(
                    "enabled mod descriptor {descriptor} does not end in .mod, the suffix the game lists mods by, so the game does not mount it"
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
            let layer = builtin_layer(LayerRole::AboveMods, prefix.into(), source, None, coverage)?;
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
    let unsaved = if cache.dirty {
        let published = publish_hash_cache(&cache_path, cache);
        if published.is_ok() {
            let _ = fs::remove_file(options.cache_dir.join("content-hashes-v1.json"));
        }
        published
            .err()
            .map(|error| format!("{}: {error}", cache_path.display()))
    } else {
        None
    };
    result.map(|mut content: GatheredContent| {
        content.stats.elapsed_ms = started.elapsed().as_millis();
        content.stats.cache_unsaved = unsaved;
        content
    })
}

fn publish_hash_cache(path: &Path, mut cache: HashCache) -> io::Result<()> {
    static PUBLISHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if let Some(published) = fs::read(path)
        .ok()
        .and_then(|bytes| HashCache::decode(&bytes))
    {
        for (key, entry) in published.entries {
            cache.entries.entry(key).or_insert(entry);
        }
    }
    let staged = path.with_file_name(format!(
        "{HASH_CACHE}.{}.{}.tmp",
        std::process::id(),
        PUBLISHED.fetch_add(1, Ordering::Relaxed)
    ));
    let published = fs::write(&staged, cache.encode()).and_then(|()| fs::rename(&staged, path));
    if published.is_err() {
        let _ = fs::remove_file(&staged);
    }
    published
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hash_cache_tests;

#[cfg(all(test, windows))]
mod sharing_tests;

#[cfg(any(test, feature = "layer-fixtures"))]
pub mod fixtures;
