//! Ingest boundary: recording bytes -> raw per-hour frames.
//!
//! The only place recording bytes enter the replay layer
//! (REPLAY_ARCHITECTURE.md, "Ingest boundary"). Backend 1 — the
//! snapshot-flavor `.battleplan` container or directory store — is read
//! through the verified `snapshot` crate reader (reuse, not a fork).
//! Backend 2 — the current production compact-stream-only container — is
//! detected and refused with a precise error instead of guessing record
//! layouts.

use serde_json::{Map, Value};
use snapshot::{ExpectedIdentity, Store};
use std::collections::BTreeMap;
use std::path::Path;

/// Timeline format id the production recorder writes for compact-stream-only
/// recordings (`extractor/native/recorder_host`).
const COMPACT_STREAM_TIMELINE_FORMAT: &str = "hoi4_compact_stream_timeline_v1";
/// Manifest-entry / timeline role of the raw compact domain stream.
const COMPACT_STREAM_ROLE: &str = "collector_compact_stream";

#[derive(Debug)]
pub enum SourceError {
    /// The recording could not be opened, read, or validated.
    Open(String),
    /// The recording was recognized but this build cannot play it back.
    Unsupported(String),
    /// The store holds several games; pick one with [`open_recording_game`].
    MultipleGames(Vec<String>),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Open(m) => write!(f, "open: {m}"),
            SourceError::Unsupported(m) => write!(f, "unsupported recording: {m}"),
            SourceError::MultipleGames(games) => write!(
                f,
                "store holds {} games ({}); select one explicitly with open_recording_game",
                games.len(),
                games.join(", ")
            ),
        }
    }
}
impl std::error::Error for SourceError {}

/// One raw, schema-shaped frame (REPLAY_SCHEMA.md §1).
///
/// `hour_index` is the INDEX's hour for this frame — the index is the
/// canonical authority (§0.3). `rest` carries every other top-level domain
/// untouched, so unknown schema growth is preserved and reachable, never
/// dropped. This type deliberately does not expose `snapshot` crate types.
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub hour_index: i64,
    pub date: Option<String>,
    pub save_version: Option<String>,
    pub schema_version: Option<Value>,
    pub rest: Map<String, Value>,
}

/// A recording backend. Backend 1 is the snapshot-flavor container/dir store.
/// Backend 2 (compact-stream-only production containers) fails closed in
/// [`open_recording`] until the extraction layer settles its decode contract.
pub trait ReplaySource: Send + Sync {
    fn game_id(&self) -> &str;
    /// Sorted, deduplicated canonical hour indexes, taken from the recording
    /// INDEX (the authority). Entries without an `hour_index` are skipped and
    /// reported in [`ReplaySource::warnings`], never guessed.
    fn hours(&self) -> &[i64];
    /// Non-fatal ingest findings (index entries skipped, duplicate hours, ...).
    fn warnings(&self) -> &[String];
    /// Raw snapshot frame for one canonical hour from [`ReplaySource::hours`].
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError>;
}

/// Open a recording path. A regular file is opened as a `.battleplan`
/// container; a directory is opened as a snapshot store root (a directory
/// containing `<game_id>/index.json`). A directory store holding more than
/// one game returns [`SourceError::MultipleGames`].
pub fn open_recording(path: &Path) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None)
}

/// As [`open_recording`], selecting one game explicitly (required when a
/// directory store holds several games).
pub fn open_recording_game(
    path: &Path,
    game_id: &str,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id))
}

fn open_recording_impl(
    path: &Path,
    requested_game: Option<&str>,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let store = if path.is_file() {
        Store::open_replay(path, &ExpectedIdentity::default())
            .map_err(|e| SourceError::Open(format!("open .battleplan: {e}")))?
    } else if path.is_dir() {
        Store::open(path)
    } else {
        return Err(SourceError::Open(format!(
            "path does not exist (expected a .battleplan file or a snapshot store directory): {}",
            path.display()
        )));
    };

    let games = store
        .games()
        .map_err(|e| SourceError::Open(format!("list games in store: {e}")))?;
    let game_id = match requested_game {
        Some(g) => {
            if !games.iter().any(|have| have == g) {
                return Err(SourceError::Open(format!(
                    "game {g:?} not found in store (available: {})",
                    if games.is_empty() {
                        "none".to_string()
                    } else {
                        games.join(", ")
                    }
                )));
            }
            g.to_string()
        }
        None => match games.len() {
            0 => {
                return Err(SourceError::Open(format!(
                    "no game found (no <game_id>/index.json) under {}",
                    path.display()
                )))
            }
            1 => games[0].clone(),
            _ => return Err(SourceError::MultipleGames(games)),
        },
    };

    let index = store
        .index(&game_id)
        .map_err(|e| SourceError::Open(format!("read index for game {game_id:?}: {e}")))?;

    // FAIL-CLOSED: an empty snapshot index is the signature of the production
    // compact-stream recording (recorder_host packs `"snapshots": []` plus a
    // `collector_compact_stream` payload entry). Refuse with a precise error
    // rather than guessing compact record layouts.
    if index.snapshots.is_empty() {
        return Err(SourceError::Unsupported(empty_index_message(
            &store, path, &game_id,
        )));
    }

    let mut files_by_hour: BTreeMap<i64, String> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    for entry in &index.snapshots {
        match entry.hour_index {
            None => warnings.push(format!(
                "index entry {:?} has no hour_index; skipped (never guessed)",
                entry.file
            )),
            Some(hour) => {
                if let Some(existing) = files_by_hour.get(&hour) {
                    warnings.push(format!(
                        "duplicate index entries for hour_index {hour} \
                         ({existing:?} and {:?}); keeping the first",
                        entry.file
                    ));
                } else {
                    files_by_hour.insert(hour, entry.file.clone());
                }
            }
        }
    }
    if files_by_hour.is_empty() {
        return Err(SourceError::Unsupported(format!(
            "recording {game_id:?} has {} snapshot index entries but none carries an \
             hour_index; nothing is playable (hour_index is the canonical key and is \
             never guessed)",
            index.snapshots.len()
        )));
    }

    let hours: Vec<i64> = files_by_hour.keys().copied().collect();
    Ok(Box::new(StoreSource {
        store,
        game_id,
        hours,
        files_by_hour,
        warnings,
    }))
}

/// Precise Unsupported message for a recording whose snapshot index is empty,
/// naming the compact-stream evidence when it is present.
fn empty_index_message(store: &Store, path: &Path, game_id: &str) -> String {
    let contract = "playback requires either per-hour snapshot entries per \
                    HOI4REPLAY_CONTAINER_SPEC.md or a shared decoder crate from the \
                    extraction layer; replay_core does not guess compact record layouts";
    match compact_stream_evidence(store, path, game_id) {
        Some(evidence) => format!(
            "recording {game_id:?} is a compact-stream-only production recording \
             (empty snapshot index; {evidence}): it carries only the raw compact \
             domain stream, and {contract}"
        ),
        None => format!(
            "recording {game_id:?} has an empty snapshot index (\"snapshots\": []) \
             and no per-hour snapshots; {contract}"
        ),
    }
}

/// Evidence that a recording is the production compact-stream flavor: a
/// container manifest entry with role `collector_compact_stream`, or a
/// `timeline.json` declaring format `hoi4_compact_stream_timeline_v1`.
fn compact_stream_evidence(store: &Store, path: &Path, game_id: &str) -> Option<String> {
    if let Some(container) = store.container() {
        let compact = container.entries_by_role(COMPACT_STREAM_ROLE);
        if let Some(entry) = compact.first() {
            return Some(format!(
                "manifest entry {:?} has role {COMPACT_STREAM_ROLE:?}",
                entry.path
            ));
        }
        let timeline_path = format!("payload/{game_id}/timeline.json");
        if let Ok(bytes) = container.read_entry_bytes(&timeline_path) {
            if let Ok(timeline) = serde_json::from_slice::<Value>(&bytes) {
                return timeline_compact_evidence(&timeline);
            }
        }
        return None;
    }
    // Directory store: probe <root>/<game_id>/timeline.json for the marker.
    let timeline_file = path.join(game_id).join("timeline.json");
    let text = std::fs::read_to_string(&timeline_file).ok()?;
    let timeline = serde_json::from_str::<Value>(&text).ok()?;
    timeline_compact_evidence(&timeline)
}

fn timeline_compact_evidence(timeline: &Value) -> Option<String> {
    let format = timeline.get("format").and_then(Value::as_str);
    let role = timeline.get("raw_stream_role").and_then(Value::as_str);
    if format == Some(COMPACT_STREAM_TIMELINE_FORMAT) || role == Some(COMPACT_STREAM_ROLE) {
        Some(format!(
            "timeline.json declares format {:?} / raw_stream_role {:?}",
            format.unwrap_or("<missing>"),
            role.unwrap_or("<missing>")
        ))
    } else {
        None
    }
}

/// Backend 1: a snapshot-flavor container or directory store, read through
/// the verified `snapshot` crate. Never leaks `snapshot` types.
///
/// F2 audit note (justified O(recorded-hours) floor): `hours` (8 B/hour) and
/// `files_by_hour` (one recorded file name per hour) are the random-access
/// index of the recording — built once at open, never growing with scrub
/// count. They are required to serve `frame(hour)` without re-reading the
/// store index and are the documented per-open floor of the source's working
/// set (~210 KB + file-name strings for a 3-year hourly recording).
/// `warnings` is bounded by the number of malformed index entries.
struct StoreSource {
    store: Store,
    game_id: String,
    hours: Vec<i64>,
    files_by_hour: BTreeMap<i64, String>,
    warnings: Vec<String>,
}

impl ReplaySource for StoreSource {
    fn game_id(&self) -> &str {
        &self.game_id
    }
    fn hours(&self) -> &[i64] {
        &self.hours
    }
    fn warnings(&self) -> &[String] {
        &self.warnings
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let file = self.files_by_hour.get(&hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no snapshot for hour_index {hour_index} in game {:?}",
                self.game_id
            ))
        })?;
        let snap = self
            .store
            .load_file(&self.game_id, file)
            .map_err(|e| SourceError::Open(format!("load snapshot {file:?}: {e}")))?;
        // The index is the authority for the hour; a body that names a
        // DIFFERENT hour is an inconsistent recording and is refused, not
        // silently re-keyed.
        if let Some(body_hour) = snap.hour_index {
            if body_hour != hour_index {
                return Err(SourceError::Open(format!(
                    "snapshot {file:?} body hour_index {body_hour} != index hour_index \
                     {hour_index} (index is the authority; refusing inconsistent recording)"
                )));
            }
        }
        Ok(RawFrame {
            hour_index,
            date: snap.date,
            save_version: snap.save_version,
            schema_version: snap.schema_version,
            rest: snap.rest,
        })
    }
}
