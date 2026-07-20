//! Neutral, forward-compatible reader/writer for the portable replay store.
//!
//! The store lives at `<root>/<game_id>/` with:
//!   * `index.json`                       — ordered manifest (our format, stable)
//!   * `<012 hour_index>.snapshot.json`   — one full-world snapshot per in-game hour
//!   * `timeline.json`                    — diff/event stream (not read here)
//!
//! Design: mirror only the STABLE top-level shape of `replay/SCHEMA.md`; every
//! domain payload (countries, states, provinces,
//! divisions, designs, ...) stays a raw `serde_json::Value` so the reader NEVER
//! breaks when the schema grows — old snapshots stay readable. Strong typing happens
//! later at the bind layer, per-domain.
//!
//! No-fabrication is preserved structurally: [`Snapshot::country_path`] returns `None`
//! for an ABSENT key (never captured) and `Some(Value::Null)` for an explicit null
//! (captured-but-unobtained) — the two are distinguishable, never collapsed onto a `0`.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

pub mod canon;
pub mod container;
pub mod design_catalog;
mod dir_store;
pub mod pack;
pub mod prefolded;
pub mod prefolded_decoder;
pub mod sha256;
pub mod stream_verify;
pub mod strict_json;
pub mod zip;

pub use container::{Container, ContainerError, ExpectedIdentity, SpoolEntryReceipt};
pub use pack::{
    pack, pack_borrowed, pack_borrowed_to_writer, pack_borrowed_with, pack_borrowed_with_writer,
    pack_mixed_to_writer, pack_streaming_to_writer, pack_streaming_zstd_to_writer,
    pack_streaming_zstd_to_writer_with_profile, pack_with, BorrowedPackEntry, CompressionProfile,
    FileBackedPackEntry, PackEntry, PackError, PackIdentity, PackOptions, StreamingFileEntry,
};
pub use prefolded::{
    PrefoldedHourEntry, PrefoldedHourIndex, PrefoldedReplay, PrefoldedReplayError,
};
pub use prefolded_decoder::{
    decode_certified_prefolded, decode_certified_prefolded_spool, CertifiedPrefoldDecoder,
    CertifiedPrefoldRequest, CertifiedPrefoldSpoolDecoder, CertifiedPrefoldSpoolRequest,
    CertifiedPrefoldedReplay, CertifiedPrefoldedSpool, PrefoldDecodeError,
    PrefoldSourceReceipt, PrefoldStreamIdentityMode, PrefoldedSpool, PrefoldedSpoolEntry,
    COMPACT_STREAM_ROLE,
};
pub use stream_verify::{
    verify_replay_container_streaming, verify_replay_container_streaming_file,
    verify_replay_container_streaming_file_at_path, StreamVerifyReport,
};
pub use strict_json::from_slice as parse_json_strict;

/// Strictly build one JSON tree, rejecting duplicate members recursively, then
/// project the typed view from that exact validated tree (never from a second
/// permissive byte decoder).
pub(crate) fn decode_json_strict<T: DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, serde_json::Error> {
    serde_json::from_value(strict_json::from_slice(bytes)?)
}

/// The `index.json` manifest for one game (our stable format).
///
/// A plain (v1) index carries only `game_id`/`schema_version`/`snapshots`. A
/// **stitched** (container v2) index additionally carries `index_schema`
/// (`"stitched-main-v1"`), `segments_file`, `main_head`, and visible `coverage`
/// gaps (`STITCHING_DESIGN.md` §7.1). All of the stitched fields are
/// `#[serde(default)]`, so a v1 index still parses and a stitched index never
/// loses its extra fields.
#[derive(Debug, Clone, Deserialize)]
pub struct Index {
    pub game_id: String,
    #[serde(default)]
    pub schema_version: Option<Value>,
    /// Present only on a stitched index (`"stitched-main-v1"`).
    #[serde(default)]
    pub index_schema: Option<String>,
    /// Path (game-relative) to `segments.json` for a stitched index.
    #[serde(default)]
    pub segments_file: Option<String>,
    /// The final main-timeline head `{ segment_id, hour_index }` (stitched).
    #[serde(default)]
    pub main_head: Option<Value>,
    /// Visible main-timeline coverage gaps (stitched); never lists a superseded
    /// snapshot and never hides a main gap.
    #[serde(default)]
    pub coverage: Vec<Value>,
    /// The default-playback snapshot list (visible `Snapshot` hours only, on a
    /// stitched index).
    #[serde(default)]
    pub snapshots: Vec<IndexEntry>,
}

impl Index {
    /// True when this is a stitched (reload/relaunch) main index.
    pub fn is_stitched(&self) -> bool {
        self.index_schema.as_deref() == Some("stitched-main-v1")
    }
}

/// The authoritative branch graph of a stitched container (`segments.json`;
/// `STITCHING_DESIGN.md` §7.2). Read-only and forward-compatible: known fields are
/// typed, everything else stays a raw `Value` so the reader never breaks as the
/// graph grows.
#[derive(Debug, Clone, Deserialize)]
pub struct SegmentGraph {
    #[serde(default)]
    pub stitch_input_digest: Option<String>,
    #[serde(default)]
    pub play_order_ledger_digest: Option<String>,
    #[serde(default)]
    pub stream_major: Option<u64>,
    #[serde(default)]
    pub campaign: Option<Value>,
    #[serde(default)]
    pub main_head: Option<Value>,
    #[serde(default)]
    pub main_slices: Vec<Value>,
    #[serde(default)]
    pub segments: Vec<SegmentRecord>,
}

/// One segment entry in the branch graph. Visible and superseded spans are span
/// properties (§6), not whole-segment labels — a segment can carry both.
#[derive(Debug, Clone, Deserialize)]
pub struct SegmentRecord {
    pub segment_id: u32,
    #[serde(default)]
    pub play_order: Option<u64>,
    #[serde(default)]
    pub in_session_ordinal: Option<u32>,
    #[serde(default)]
    pub cutover_hour: Option<i64>,
    #[serde(default)]
    pub head_hour: Option<i64>,
    #[serde(default)]
    pub parent_segment: Option<u32>,
    #[serde(default)]
    pub empty_recovery: bool,
    #[serde(default)]
    pub index_file: Option<String>,
    #[serde(default)]
    pub visible_spans: Vec<Value>,
    #[serde(default)]
    pub superseded_spans: Vec<Value>,
    /// Every other field (source_ref, provenance, reasons, ...) stays raw.
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexEntry {
    #[serde(default)]
    pub hour_index: Option<i64>,
    #[serde(default)]
    pub date: Option<String>,
    pub file: String,
    #[serde(default)]
    pub save_version: Option<String>,
    #[serde(default)]
    pub counts: Option<Value>,
}

/// One full-world snapshot. Stable header fields are typed; every other top-level
/// domain stays raw and forward-compatible.
#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub game_id: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub hour_index: Option<i64>,
    #[serde(default)]
    pub save_version: Option<String>,
    #[serde(default)]
    pub schema_version: Option<Value>,
    /// All other top-level domains (countries, states, provinces, divisions,
    /// designs, diplomacy_world, map, provenance, ...), untouched.
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Snapshot {
    pub fn domain(&self, key: &str) -> Option<&Value> {
        self.rest.get(key)
    }
    pub fn countries(&self) -> Option<&Map<String, Value>> {
        self.rest.get("countries").and_then(Value::as_object)
    }
    pub fn country(&self, tag: &str) -> Option<&Value> {
        self.countries()?.get(tag)
    }
    pub fn states(&self) -> Option<&Map<String, Value>> {
        self.rest.get("states").and_then(Value::as_object)
    }
    pub fn state(&self, sid: &str) -> Option<&Value> {
        self.states()?.get(sid)
    }

    /// Walk a dotted path inside a country object.
    /// * `None`              — a key along the path is ABSENT (never captured).
    /// * `Some(Value::Null)` — present but null (captured-but-unobtained).
    /// * `Some(v)`           — a measured value (including a measured `0`).
    pub fn country_path<'a>(&'a self, tag: &str, dotted: &str) -> Option<&'a Value> {
        path_walk(self.country(tag)?, dotted)
    }
    /// As [`Snapshot::country_path`] but rooted at a state object.
    pub fn state_path<'a>(&'a self, sid: &str, dotted: &str) -> Option<&'a Value> {
        path_walk(self.state(sid)?, dotted)
    }
}

fn path_walk<'a>(mut cur: &'a Value, dotted: &str) -> Option<&'a Value> {
    for seg in dotted.split('.') {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// Anchored directory traversal or namespace validation failed.
    Directory(String),
    /// A `.battleplan` container could not be opened/validated.
    Container(ContainerError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "io: {e}"),
            LoadError::Json(e) => write!(f, "json: {e}"),
            LoadError::Directory(e) => write!(f, "directory store rejected: {e}"),
            LoadError::Container(e) => write!(f, "container: {e}"),
        }
    }
}
impl std::error::Error for LoadError {}
impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}
impl From<serde_json::Error> for LoadError {
    fn from(e: serde_json::Error) -> Self {
        LoadError::Json(e)
    }
}
impl From<ContainerError> for LoadError {
    fn from(e: ContainerError) -> Self {
        LoadError::Container(e)
    }
}

/// Read-only handle to a snapshot store. Backed by EITHER a directory
/// (`<root>/<game>/index.json + *.snapshot.json`) OR a `.battleplan` portable
/// recording — the public interface is identical so every consumer (scrub, app)
/// opens a container with zero changes. `Store::open` auto-detects which by the
/// path: a regular file is treated as a container, a directory as the dir store.
#[derive(Clone)]
pub struct Store {
    backend: Backend,
}

#[derive(Clone, Debug)]
struct DirGame {
    dir: dir_store::BoundDir,
    index_bytes: Option<Vec<u8>>,
}

#[derive(Clone)]
enum Backend {
    Dir {
        root: Option<dir_store::AnchoredDir>,
        games: Arc<OnceLock<Result<BTreeMap<String, DirGame>, dir_store::DirAccessError>>>,
        label: PathBuf,
        open_error: Option<dir_store::DirAccessError>,
    },
    /// A single opened, verified `.battleplan` (Arc so the `Store: Clone` contract
    /// the app/scrub rely on stays cheap and shares one opened/verified archive).
    Replay { container: Arc<Container> },
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.backend {
            Backend::Dir {
                root,
                label,
                open_error,
                ..
            } => f
                .debug_struct("Store::Dir")
                .field("root", label)
                .field("anchored", &root.is_some())
                .field("open_error", open_error)
                .finish(),
            Backend::Replay { container } => f
                .debug_struct("Store::Replay")
                .field("game_id", &container.game_id())
                .finish(),
        }
    }
}

impl Store {
    /// Consume an already-open, pinned replay container without reopening its
    /// pathname. This is the admission path for descriptor-authenticated
    /// package members whose exact file handle has already been joined to an
    /// external whole-file receipt.
    pub fn from_container(container: Container) -> Self {
        Self {
            backend: Backend::Replay {
                container: Arc::new(container),
            },
        }
    }

    /// Open a store. If `path` is a regular file it is opened as a `.battleplan`
    /// portable recording (integrity-verified, identity-only); if it is a
    /// directory it is the on-disk snapshot store. A missing path defaults to the
    /// directory backend (its reads then error per-call, preserving old behavior).
    pub fn open<P: Into<PathBuf>>(path: P) -> Self {
        let path = path.into();
        if path.is_file() {
            // Treat any regular file as a container; identity checks are opt-in via
            // `open_replay`. A malformed file surfaces on first use, mirroring how
            // the dir backend surfaces a missing index on first read.
            match Container::open(&path, &ExpectedIdentity::default()) {
                Ok(c) => {
                    return Store {
                        backend: Backend::Replay {
                            container: Arc::new(c),
                        },
                    }
                }
                Err(_) => {
                    // fall through to a Dir backend so the error is reported on the
                    // first `index`/`load` call (consistent with the old contract).
                    return Store {
                        backend: Self::dir_backend(path),
                    };
                }
            }
        }
        Store {
            backend: Self::dir_backend(path),
        }
    }

    fn dir_backend(path: PathBuf) -> Backend {
        match dir_store::AnchoredDir::open(&path) {
            Ok(root) => Backend::Dir {
                root: Some(root),
                games: Arc::new(OnceLock::new()),
                label: path,
                open_error: None,
            },
            Err(error) => Backend::Dir {
                root: None,
                games: Arc::new(OnceLock::new()),
                label: path,
                open_error: Some(error),
            },
        }
    }

    /// Open and anchor a directory store, returning root-open failures eagerly.
    pub fn open_directory<P: AsRef<Path>>(path: P) -> Result<Self, LoadError> {
        let path = path.as_ref().to_path_buf();
        let root = dir_store::AnchoredDir::open(&path)
            .map_err(|error| LoadError::Directory(error.message))?;
        Ok(Self {
            backend: Backend::Dir {
                root: Some(root),
                games: Arc::new(OnceLock::new()),
                label: path,
                open_error: None,
            },
        })
    }

    /// Open a `.battleplan` with explicit identity expectations (build/schema/mod).
    /// Returns the validation error directly (unlike [`open`], which defers).
    pub fn open_replay<P: AsRef<Path>>(
        path: P,
        expected: &ExpectedIdentity,
    ) -> Result<Self, LoadError> {
        let c = Container::open(path, expected)?;
        Ok(Store {
            backend: Backend::Replay {
                container: Arc::new(c),
            },
        })
    }

    /// The container handle, if this store is a `.battleplan` (lets the app surface
    /// the manifest / signed-state / identity to the UI). `None` for a dir store.
    pub fn container(&self) -> Option<&Container> {
        match &self.backend {
            Backend::Replay { container } => Some(container),
            Backend::Dir { .. } => None,
        }
    }

    fn directory_games(&self) -> Result<&BTreeMap<String, DirGame>, LoadError> {
        let (root, cache) = match &self.backend {
            Backend::Dir {
                root: Some(root),
                games,
                ..
            } => (root, games),
            Backend::Dir {
                open_error: Some(error),
                ..
            } => return Err(LoadError::Directory(error.message.clone())),
            Backend::Dir { .. } => {
                return Err(LoadError::Directory(
                    "directory root is not anchored".to_string(),
                ))
            }
            Backend::Replay { .. } => {
                return Err(LoadError::Directory(
                    "container has no directory games".to_string(),
                ))
            }
        };
        cache
            .get_or_init(|| Self::discover_directory_games(root))
            .as_ref()
            .map_err(|error| LoadError::Directory(error.message.clone()))
    }

    fn discover_directory_games(
        root: &dir_store::AnchoredDir,
    ) -> Result<BTreeMap<String, DirGame>, dir_store::DirAccessError> {
        let mut games = BTreeMap::new();
        for entry in root.root_entries()? {
            if !entry.directory_or_link {
                continue;
            }
            let dir = root.bind_child_dir(&entry.name)?;
            let index_bytes = dir.read_optional(&["index.json"])?;
            games.insert(entry.name, DirGame { dir, index_bytes });
        }
        Ok(games)
    }

    fn directory_game(&self, game: &str) -> Result<&DirGame, LoadError> {
        self.directory_games()?.get(game).ok_or_else(|| {
            LoadError::Directory(format!(
                "game directory {game:?} was not selected from the anchored store root"
            ))
        })
    }

    /// Game ids. Dir store: subdirectories containing an `index.json`. Container:
    /// the single packed game.
    pub fn games(&self) -> Result<Vec<String>, LoadError> {
        match &self.backend {
            Backend::Dir { .. } => {
                let mut out = Vec::new();
                for (name, game) in self.directory_games()? {
                    if game.index_bytes.is_some() {
                        game.dir
                            .revalidate()
                            .map_err(|error| LoadError::Directory(error.message))?;
                        out.push(name.clone());
                    }
                }
                Ok(out)
            }
            Backend::Replay { container } => Ok(vec![container.game_id().to_string()]),
        }
    }

    pub fn index(&self, game: &str) -> Result<Index, LoadError> {
        match &self.backend {
            Backend::Dir { .. } => {
                let selected = self.directory_game(game)?;
                selected
                    .dir
                    .revalidate()
                    .map_err(|error| LoadError::Directory(error.message))?;
                let bytes = selected.index_bytes.as_deref().ok_or_else(|| {
                    LoadError::Directory(format!(
                        "required index.json is missing from selected game directory {game:?}"
                    ))
                })?;
                let index: Index = decode_json_strict(bytes)?;
                if index.game_id != game {
                    return Err(LoadError::Container(ContainerError::Reject(format!(
                        "index.game_id {:?} != selected directory name {game:?}",
                        index.game_id
                    ))));
                }
                Ok(index)
            }
            Backend::Replay { container } => {
                if container.game_id() != game {
                    return Err(LoadError::Container(ContainerError::Reject(format!(
                        "container holds game {:?}, not {game:?}",
                        container.game_id()
                    ))));
                }
                Ok(container.index().clone())
            }
        }
    }

    /// Read a snapshot from the dir backend after validating that `file` is a SAFE,
    /// game-dir-relative path. A `file` field comes from `index.json`, which may be
    /// tampered/corrupt; without this guard a `../../secret.json` (or an absolute /
    /// drive-letter path) would escape the game directory and return arbitrary
    /// on-disk JSON as a "snapshot". The container backend is already safe (it
    /// resolves through the verified `by_path` map). Legitimate names —
    /// `000000000100.snapshot.json` and segment-prefixed `seg3/...json` — pass
    /// `zip::validate_path`'s rules unchanged (Codex bug-hunt FIX-1).
    fn read_dir_snapshot(&self, game: &str, file: &str) -> Result<Snapshot, LoadError> {
        crate::zip::validate_path(file).map_err(|e| {
            LoadError::Container(ContainerError::Reject(format!(
                "unsafe snapshot file name {file:?}: {e}"
            )))
        })?;
        let bytes = self.read_game_file_bytes_required(game, file)?;
        Ok(decode_json_strict(&bytes)?)
    }

    /// Load a snapshot by its in-game `hour_index` (the canonical key). The
    /// container resolves the file through the INDEX, never by synthesizing a name,
    /// so a segment-prefixed rollback file loads correctly.
    pub fn load(&self, game: &str, hour_index: i64) -> Result<Snapshot, LoadError> {
        match &self.backend {
            Backend::Dir { .. } => {
                // Resolve the file through the INDEX, exactly like the container
                // backend: `index.snapshots[*].file` is the only filename
                // authority (spec §1/§4.3), and both in-the-wild forms —
                // `%012d.snapshot.json` and segment-prefixed rollback names —
                // must load. Never synthesize a name from hour_index.
                let index = self.index(game)?;
                let file = index
                    .snapshots
                    .iter()
                    .find(|e| e.hour_index == Some(hour_index))
                    .map(|e| e.file.clone())
                    .ok_or_else(|| {
                        LoadError::Container(ContainerError::Reject(format!(
                            "no snapshot for hour_index {hour_index}"
                        )))
                    })?;
                self.read_dir_snapshot(game, &file)
            }
            Backend::Replay { container } => Ok(container.load(hour_index)?),
        }
    }

    /// Load a snapshot by its explicit file name (from an [`IndexEntry`]).
    pub fn load_file(&self, game: &str, file: &str) -> Result<Snapshot, LoadError> {
        match &self.backend {
            Backend::Dir { .. } => self.read_dir_snapshot(game, file),
            Backend::Replay { container } => Ok(container.load_file(file)?),
        }
    }

    /// Read one game-relative file through the anchored directory walk, or one
    /// verified manifest entry for a container.
    pub fn read_game_file_bytes(
        &self,
        game: &str,
        relative: &str,
    ) -> Result<Option<Vec<u8>>, LoadError> {
        let logical = format!("{game}/{relative}");
        crate::zip::validate_path(&logical).map_err(|error| {
            LoadError::Container(ContainerError::Reject(format!(
                "unsafe game-relative path {logical:?}: {error}"
            )))
        })?;
        match &self.backend {
            Backend::Dir { .. } => {
                let components: Vec<&str> = relative.split('/').collect();
                self.directory_game(game)?
                    .dir
                    .read_optional(&components)
                    .map_err(|error| LoadError::Directory(error.message))
            }
            Backend::Replay { container } => {
                if container.game_id() != game {
                    return Err(LoadError::Container(ContainerError::Reject(format!(
                        "container holds game {:?}, not {game:?}",
                        container.game_id()
                    ))));
                }
                let path = format!("payload/{game}/{relative}");
                // O(1) membership via the verified entry map — never re-project
                // the whole manifest per read. A poisoned verification lock
                // propagates as the container's typed refusal instead of
                // collapsing into an "absent entry" Ok(None) (fail-closed).
                if !container.has_entry(&path)? {
                    return Ok(None);
                }
                Ok(Some(container.read_entry_bytes(&path)?))
            }
        }
    }

    fn read_game_file_bytes_required(
        &self,
        game: &str,
        relative: &str,
    ) -> Result<Vec<u8>, LoadError> {
        self.read_game_file_bytes(game, relative)?.ok_or_else(|| {
            LoadError::Directory(format!(
                "required game-relative file {game:?}/{relative} is missing"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(tag: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "hoi4_snapshot_lib_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// Spec §1/§4.3: `index.snapshots[*].file` is the ONLY filename authority.
    /// A directory store whose index names a segment-prefixed rollback file must
    /// load it by hour_index — never synthesize `%012d.snapshot.json`.
    #[test]
    fn dir_store_load_resolves_segment_prefixed_files_through_the_index() {
        let root = temp_root("dir_load_index");
        let game = root.join("g");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.json"),
            br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":17000461,"file":"s00000001.000017000461.snapshot.json"}]}"#,
        )
        .unwrap();
        fs::write(
            game.join("s00000001.000017000461.snapshot.json"),
            br#"{"game_id":"g","hour_index":17000461}"#,
        )
        .unwrap();

        let store = Store::open_directory(&root).expect("anchor dir store");
        let snapshot = store
            .load("g", 17000461)
            .expect("segment-prefixed rollback snapshot loads by hour_index");
        assert_eq!(snapshot.hour_index, Some(17000461));

        // A missing hour is a typed Reject naming the hour, not a
        // synthesized-filename missing-file error.
        let err = store.load("g", 5).expect_err("missing hour is refused");
        match err {
            LoadError::Container(ContainerError::Reject(message)) => assert!(
                message.contains("no snapshot for hour_index 5"),
                "unexpected message: {message}"
            ),
            other => panic!("expected typed Reject, got {other:?}"),
        }
        let _ = fs::remove_dir_all(root);
    }

    /// A caught panic during container entry verification poisons the entry
    /// lock. `read_game_file_bytes` must then propagate the container's typed
    /// lock-poison refusal — never Ok(None) "file absent" (fail-closed).
    #[test]
    fn replay_store_poisoned_entry_lock_refuses_instead_of_absent() {
        let bytes = crate::pack::pack(
            "g",
            &[crate::pack::PackEntry {
                path: "payload/g/index.json".into(),
                bytes: br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#.to_vec(),
                role: "index",
                hour_index: None,
            }],
            &crate::pack::PackIdentity::default(),
            false,
        )
        .unwrap();
        let container = Container::from_bytes(bytes, &ExpectedIdentity::default())
            .expect("container opens");
        let store = Store {
            backend: Backend::Replay {
                container: Arc::new(container),
            },
        };
        assert!(store
            .read_game_file_bytes("g", "index.json")
            .expect("healthy read succeeds")
            .is_some());
        store.container().unwrap().poison_entry_lock_for_test();
        let err = store
            .read_game_file_bytes("g", "index.json")
            .expect_err("a poisoned lock must refuse, never report absence");
        match err {
            LoadError::Container(ContainerError::Reject(message)) => assert!(
                message.contains("entry-verification lock poisoned"),
                "unexpected message: {message}"
            ),
            other => panic!("expected typed container Reject, got {other:?}"),
        }
    }
}
