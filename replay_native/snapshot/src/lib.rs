//! Forward-compatible READ-ONLY reader for the replay snapshot store.
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

use serde::Deserialize;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod canon;
pub mod container;
pub mod pack;
pub mod sha256;
pub mod zip;

pub use container::{Container, ContainerError, ExpectedIdentity};
pub use pack::{pack, pack_with, PackEntry, PackError, PackIdentity, PackOptions};

/// The `index.json` manifest for one game (our stable format).
#[derive(Debug, Clone, Deserialize)]
pub struct Index {
    pub game_id: String,
    #[serde(default)]
    pub schema_version: Option<Value>,
    #[serde(default)]
    pub snapshots: Vec<IndexEntry>,
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
    /// A `.battleplan` container could not be opened/validated.
    Container(ContainerError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "io: {e}"),
            LoadError::Json(e) => write!(f, "json: {e}"),
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

#[derive(Clone)]
enum Backend {
    Dir {
        root: PathBuf,
    },
    /// A single opened, verified `.battleplan` (Arc so the `Store: Clone` contract
    /// the app/scrub rely on stays cheap and shares one opened/verified archive).
    Replay {
        container: Arc<Container>,
    },
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.backend {
            Backend::Dir { root } => f.debug_struct("Store::Dir").field("root", root).finish(),
            Backend::Replay { container } => f
                .debug_struct("Store::Replay")
                .field("game_id", &container.game_id())
                .finish(),
        }
    }
}

impl Store {
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
                        backend: Backend::Dir { root: path },
                    };
                }
            }
        }
        Store {
            backend: Backend::Dir { root: path },
        }
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

    pub fn game_dir(&self, game: &str) -> PathBuf {
        match &self.backend {
            Backend::Dir { root } => root.join(game),
            // synthetic: the container's logical game dir (not a real fs path).
            Backend::Replay { .. } => PathBuf::from(game),
        }
    }

    /// Game ids. Dir store: subdirectories containing an `index.json`. Container:
    /// the single packed game.
    pub fn games(&self) -> std::io::Result<Vec<String>> {
        match &self.backend {
            Backend::Dir { root } => {
                let mut out = Vec::new();
                for entry in fs::read_dir(root)? {
                    let entry = entry?;
                    if entry.path().join("index.json").is_file() {
                        if let Some(name) = entry.file_name().to_str() {
                            out.push(name.to_string());
                        }
                    }
                }
                out.sort();
                Ok(out)
            }
            Backend::Replay { container } => Ok(vec![container.game_id().to_string()]),
        }
    }

    pub fn index(&self, game: &str) -> Result<Index, LoadError> {
        match &self.backend {
            Backend::Dir { root } => {
                let txt = fs::read_to_string(root.join(game).join("index.json"))?;
                Ok(serde_json::from_str(&txt)?)
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

    fn read_snapshot(path: &Path) -> Result<Snapshot, LoadError> {
        let txt = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&txt)?)
    }

    /// Read a snapshot from the dir backend after validating that `file` is a SAFE,
    /// game-dir-relative path. A `file` field comes from `index.json`, which may be
    /// tampered/corrupt; without this guard a `../../secret.json` (or an absolute /
    /// drive-letter path) would escape the game directory and return arbitrary
    /// on-disk JSON as a "snapshot". The container backend is already safe (it
    /// resolves through the verified `by_path` map). Legitimate names —
    /// `000000000100.snapshot.json` and segment-prefixed `seg3/...json` — pass
    /// `zip::validate_path`'s rules unchanged (Codex bug-hunt FIX-1).
    fn read_dir_snapshot(root: &Path, game: &str, file: &str) -> Result<Snapshot, LoadError> {
        crate::zip::validate_path(file).map_err(|e| {
            LoadError::Container(ContainerError::Reject(format!(
                "unsafe snapshot file name {file:?}: {e}"
            )))
        })?;
        Self::read_snapshot(&root.join(game).join(file))
    }

    /// Load a snapshot by its in-game `hour_index` (the canonical key). The
    /// container resolves the file through the INDEX, never by synthesizing a name,
    /// so a segment-prefixed rollback file loads correctly.
    pub fn load(&self, game: &str, hour_index: i64) -> Result<Snapshot, LoadError> {
        match &self.backend {
            Backend::Dir { root } => {
                // A negative hour_index would format with a leading '-' and break the
                // fixed width; refuse it up front (it can never name a real snapshot,
                // and the synthesized name must stay a safe, game-dir-relative path).
                if hour_index < 0 {
                    return Err(LoadError::Container(ContainerError::Reject(format!(
                        "negative hour_index {hour_index} has no snapshot"
                    ))));
                }
                let fname = format!("{:012}.snapshot.json", hour_index);
                Self::read_dir_snapshot(root, game, &fname)
            }
            Backend::Replay { container } => Ok(container.load(hour_index)?),
        }
    }

    /// Load a snapshot by its explicit file name (from an [`IndexEntry`]).
    pub fn load_file(&self, game: &str, file: &str) -> Result<Snapshot, LoadError> {
        match &self.backend {
            Backend::Dir { root } => Self::read_dir_snapshot(root, game, file),
            Backend::Replay { container } => Ok(container.load_file(file)?),
        }
    }
}
