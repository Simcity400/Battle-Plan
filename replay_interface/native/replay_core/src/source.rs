//! Ingest boundary: recording bytes -> raw per-hour frames.
//!
//! The only place recording bytes enter the replay layer
//! (REPLAY_ARCHITECTURE.md, "Ingest boundary"). Backend 1 — the
//! snapshot-flavor `.battleplan` container or directory store — is read
//! through the verified `snapshot` crate reader (reuse, not a fork).
//! Backend 2 — a compact-stream-only container — remains rejected by the
//! ordinary product open path. An explicit caller may inject either neutral
//! certified capability; the production-shaped route authenticates to a
//! private spool through [`snapshot::CertifiedPrefoldSpoolDecoder`] and lazily
//! loads one immutable certified hour at a time.

use serde_json::{Map, Value};
use snapshot::{ExpectedIdentity, Store};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Timeline format id the production recorder writes for compact-stream-only
/// recordings (`extractor/native/recorder_host`).
const COMPACT_STREAM_TIMELINE_FORMAT: &str = "hoi4_compact_stream_timeline_v1";
/// Manifest-entry / timeline role of the raw compact domain stream.
const COMPACT_STREAM_ROLE: &str = snapshot::COMPACT_STREAM_ROLE;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecorderSessionBinding {
    active_dlcs: Vec<String>,
    localisation_language: String,
    display_variant: RecorderDisplayVariant,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RecorderDisplayVariant {
    physical_width: u32,
    physical_height: u32,
    min_gui_width: u32,
    min_gui_height: u32,
    gui_scale_f32_bits: u32,
    refresh_rate_hz: u32,
    max_refresh_rate_hz: Option<u32>,
    fullscreen: bool,
    borderless: bool,
    display_mode: String,
    settings_file_sha256: String,
}

struct MetadataRoute {
    binding: Option<RecorderSessionBinding>,
    timeline: Option<Value>,
}

#[derive(Debug)]
pub enum SourceError {
    /// The recording could not be opened, read, or validated.
    Open(String),
    /// The recording path does not exist. A sibling of [`SourceError::Open`]
    /// (identical Display shape) typed so consumers can classify "not found"
    /// without matching on error text.
    NotFound(String),
    /// The recording's manifest mod identity disagrees with its snapshot
    /// provenance ([`snapshot::ContainerError::ModsetMismatch`]), typed through
    /// so the product surface classifies a modset mismatch without string
    /// matching across crate boundaries.
    ModsetMismatch(String),
    /// The recording was recognized but this build cannot play it back.
    Unsupported(String),
    /// The store holds several games; pick one with [`open_recording_game`].
    MultipleGames(Vec<String>),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // NotFound/ModsetMismatch keep the exact "open: ..." rendering the
            // untyped Open produced, so every message contract holds verbatim.
            SourceError::Open(m) | SourceError::NotFound(m) | SourceError::ModsetMismatch(m) => {
                write!(f, "open: {m}")
            }
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

/// A recording backend. Portable production recordings carry indexed,
/// canonical per-hour snapshots inside the file.
pub trait ReplaySource: Send + Sync {
    fn game_id(&self) -> &str;
    /// Sorted, deduplicated canonical hour indexes, taken from the recording
    /// INDEX (the authority). Entries without an `hour_index` are skipped and
    /// reported in [`ReplaySource::warnings`], never guessed.
    fn hours(&self) -> &[i64];
    /// Non-fatal ingest findings (index entries skipped, duplicate hours, ...).
    fn warnings(&self) -> &[String];
    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        None
    }
    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        None
    }
    /// Exact source-byte and decoder identity for an explicitly prefolded
    /// compact-stream container. Snapshot-native and trusted direct-prefold
    /// sources return `None`.
    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        None
    }
    /// Raw snapshot frame for one canonical hour from [`ReplaySource::hours`].
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError>;
}

/// Open a recording path. A regular file is opened as a `.battleplan`
/// container; a directory is opened as a snapshot store root (a directory
/// containing `<game_id>/index.json`). A directory store holding more than
/// one game returns [`SourceError::MultipleGames`].
pub fn open_recording(path: &Path) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, None, None)
}

/// As [`open_recording`], selecting one game explicitly (required when a
/// directory store holds several games).
pub fn open_recording_game(
    path: &Path,
    game_id: &str,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), None, None)
}

/// Open a recording with an explicitly supplied portable certified pre-folder.
///
/// Snapshot-flavor recordings follow the ordinary path and do not invoke the
/// decoder. For a compact-stream-only `.battleplan`, the verified unique
/// `collector_compact_stream` entry is transferred to `decoder`; replay_core
/// admits only the complete [`snapshot::PrefoldedReplay`] it returns. The
/// default [`open_recording`] policy remains fail-closed and no extractor crate
/// is a dependency of replay_core.
pub fn open_recording_with_decoder(
    path: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, Some(decoder), None)
}

/// As [`open_recording_with_decoder`], selecting one game explicitly.
pub fn open_recording_game_with_decoder(
    path: &Path,
    game_id: &str,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), Some(decoder), None)
}

/// Open a compact-stream-only container through the bounded authenticated
/// spool seam. `spool_base` must be an existing regular directory; replay_core
/// creates a private identity-marked child, removes it on every failed open,
/// and retains it only for the lifetime of the returned lazy source.
pub fn open_recording_with_spooled_decoder(
    path: &Path,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, None, Some((spool_base, decoder)))
}

/// As [`open_recording_with_spooled_decoder`], selecting one game explicitly.
pub fn open_recording_game_with_spooled_decoder(
    path: &Path,
    game_id: &str,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), None, Some((spool_base, decoder)))
}

/// Admit a neutral set of canonical snapshots that was fully folded before it
/// crossed into the standalone replay layer.
///
/// This is intentionally a different boundary from [`open_recording`]. It does
/// not accept H4CR bytes, a compact-stream cursor, or an extractor callback.
/// Compact-only packages therefore remain fail-closed until an extractor-side
/// producer has completed/certified the fold and supplied [`snapshot::PrefoldedReplay`].
/// This is a trusted-capability API for an already authenticated producer; use
/// [`open_recording_with_decoder`] when source-byte length/hash and decoder
/// identity must be bound mechanically by the portable invocation receipt.
pub fn open_prefolded_replay(
    replay: snapshot::PrefoldedReplay,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    validate_index_schema_version(Some(replay.index().schema_version()))?;
    Ok(Box::new(PrefoldedSource {
        replay,
        warnings: Vec::new(),
        store: None,
        recorder_binding: None,
        design_catalog: None,
        prefold_receipt: None,
    }))
}

fn open_recording_impl(
    path: &Path,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl_with_post_anchor(path, requested_game, decoder, spool_decoder, |_| {})
}

fn open_recording_impl_with_post_anchor<F>(
    path: &Path,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
    post_anchor: F,
) -> Result<Box<dyn ReplaySource>, SourceError>
where
    F: FnOnce(&Store),
{
    let store = if path.is_file() {
        Store::open_replay(path, &ExpectedIdentity::default()).map_err(|e| {
            let message = format!("open .battleplan: {e}");
            // Preserve the snapshot layer's typed modset mismatch across the
            // adapter (no string matching): everything else stays Open.
            match &e {
                snapshot::LoadError::Container(snapshot::ContainerError::ModsetMismatch {
                    ..
                }) => SourceError::ModsetMismatch(message),
                _ => SourceError::Open(message),
            }
        })?
    } else if path.is_dir() {
        Store::open_directory(path)
            .map_err(|e| SourceError::Open(format!("anchor directory store: {e}")))?
    } else {
        return Err(SourceError::NotFound(format!(
            "path does not exist (expected a .battleplan file or a snapshot store directory): {}",
            path.display()
        )));
    };
    post_anchor(&store);

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

    // Route recorder metadata before consulting snapshot availability. An
    // explicit or inconsistent metadata signal can never be downgraded to
    // legacy/unsupported because an index or role entry is missing.
    let metadata = recorder_metadata_route(&store, &game_id)?;

    let index = store
        .index(&game_id)
        .map_err(|e| SourceError::Open(format!("read index for game {game_id:?}: {e}")))?;

    // FAIL-CLOSED by default. Only an explicit portable certified pre-folder
    // can turn a verified compact-only container into immutable canonical
    // snapshots; replay_core never sees H4CR records or extractor fold types.
    if index.snapshots.is_empty() {
        if let Some((spool_base, spool_decoder)) = spool_decoder {
            return open_compact_stream_with_spooled_decoder(
                store,
                game_id,
                index.schema_version.clone(),
                metadata,
                spool_base,
                spool_decoder,
            );
        }
        if let Some(decoder) = decoder {
            return open_compact_stream_with_decoder(
                store,
                game_id,
                index.schema_version.clone(),
                metadata,
                decoder,
            );
        }
        return Err(SourceError::Unsupported(empty_index_message(
            &store,
            metadata.timeline.as_ref(),
            &game_id,
        )));
    }
    validate_index_schema_version(index.schema_version.as_ref())?;

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
    let design_catalog = read_portable_design_catalog(&store, &game_id)?;
    Ok(Box::new(StoreSource {
        store,
        game_id,
        hours,
        files_by_hour,
        warnings,
        recorder_binding: metadata.binding,
        design_catalog,
    }))
}

fn open_compact_stream_with_decoder(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    metadata: MetadataRoute,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let container = store.container().ok_or_else(|| {
        SourceError::Unsupported(format!(
            "recording {game_id:?} is a compact-stream-only directory store; the portable decoder seam accepts authenticated .battleplan container role entries only"
        ))
    })?;
    let entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
    let entry = match entries.as_slice() {
        [entry] => entry,
        [] => {
            return Err(SourceError::Unsupported(format!(
                "recording {game_id:?} has an empty snapshot index but no manifest entry with role {:?}",
                snapshot::COMPACT_STREAM_ROLE
            )))
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recording {game_id:?} has {} manifest entries with role {:?}; expected exactly one",
                entries.len(),
                snapshot::COMPACT_STREAM_ROLE
            )))
        }
    };
    validate_index_schema_version(declared_schema_version.as_ref())?;
    validate_compact_stream_entry_identity(&entry.path, &game_id, metadata.timeline.as_ref())?;
    let raw_h4cr = container.read_entry_bytes(&entry.path).map_err(|error| {
        SourceError::Open(format!(
            "read verified compact stream role entry {:?}: {error}",
            entry.path
        ))
    })?;
    let request =
        snapshot::CertifiedPrefoldRequest::try_new(game_id.clone(), raw_h4cr).map_err(|error| {
            SourceError::Open(format!(
                "compact stream role entry {:?} violates the portable pre-fold request: {error}",
                entry.path
            ))
        })?;
    let certified = snapshot::decode_certified_prefolded(decoder, request).map_err(|error| {
        SourceError::Open(format!(
            "portable decoder {:?} rejected compact stream role entry {:?}: {error}",
            decoder.decoder_id(),
            entry.path
        ))
    })?;
    validate_index_schema_version(Some(certified.index().schema_version()))?;
    if let Some(declared) = declared_schema_version.as_ref() {
        if !same_supported_schema_version(declared, certified.index().schema_version()) {
            return Err(SourceError::Open(format!(
                "compact-only index schema_version {declared} does not match decoder {:?} output schema_version {}",
                decoder.decoder_id(),
                certified.index().schema_version()
            )));
        }
    }
    validate_prefolded_container_identity(container, certified.replay())?;

    let design_catalog = read_portable_design_catalog(&store, &game_id)?;
    let prefold_receipt = Some(certified.receipt().clone());
    Ok(Box::new(PrefoldedSource {
        replay: certified.into_replay(),
        warnings: Vec::new(),
        store: Some(store),
        recorder_binding: metadata.binding,
        design_catalog,
        prefold_receipt,
    }))
}

fn open_compact_stream_with_spooled_decoder(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    metadata: MetadataRoute,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let owner = PrefoldSpoolOwner::create(spool_base)?;
    let raw_path = owner.root().join("source.h4cr");
    let snapshot_dir = owner.snapshot_dir().to_path_buf();

    let source_receipt = {
        let container = store.container().ok_or_else(|| {
            SourceError::Unsupported(format!(
                "recording {game_id:?} is a compact-stream-only directory store; the bounded portable decoder seam accepts authenticated .battleplan role entries only"
            ))
        })?;
        let entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
        let entry = match entries.as_slice() {
            [entry] => entry,
            [] => {
                return Err(SourceError::Unsupported(format!(
                    "recording {game_id:?} has an empty snapshot index but no manifest entry with role {:?}",
                    snapshot::COMPACT_STREAM_ROLE
                )))
            }
            _ => {
                return Err(SourceError::Open(format!(
                    "recording {game_id:?} has {} manifest entries with role {:?}; expected exactly one",
                    entries.len(),
                    snapshot::COMPACT_STREAM_ROLE
                )))
            }
        };
        validate_index_schema_version(declared_schema_version.as_ref())?;
        validate_compact_stream_entry_identity(&entry.path, &game_id, metadata.timeline.as_ref())?;
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&raw_path)
            .map_err(|error| {
                SourceError::Open(format!(
                    "create private compact-stream spool {}: {error}",
                    raw_path.display()
                ))
            })?;
        let mut writer = std::io::BufWriter::with_capacity(1024 * 1024, file);
        let receipt = container
            .spool_entry_to_writer(&entry.path, &mut writer)
            .map_err(|error| {
                SourceError::Open(format!(
                    "authenticate compact stream role entry {:?} into private spool: {error}",
                    entry.path
                ))
            })?;
        if receipt.path() != entry.path.as_str() {
            return Err(SourceError::Open(
                "container spool receipt path does not match selected compact role entry"
                    .to_string(),
            ));
        }
        writer.flush().map_err(|error| {
            SourceError::Open(format!(
                "flush private compact-stream spool {}: {error}",
                raw_path.display()
            ))
        })?;
        writer.get_ref().sync_all().map_err(|error| {
            SourceError::Open(format!(
                "sync private compact-stream spool {}: {error}",
                raw_path.display()
            ))
        })?;
        receipt
    };

    let request = snapshot::CertifiedPrefoldSpoolRequest::try_new(
        game_id.clone(),
        raw_path,
        snapshot_dir.clone(),
        source_receipt.uncompressed_len(),
        source_receipt.sha256(),
    )
    .map_err(|error| {
        SourceError::Open(format!(
            "authenticated compact stream violates bounded pre-fold request: {error}"
        ))
    })?;
    let certified =
        snapshot::decode_certified_prefolded_spool(decoder, request).map_err(|error| {
            SourceError::Open(format!(
                "bounded portable decoder {:?} rejected authenticated compact stream: {error}",
                decoder.decoder_id()
            ))
        })?;
    validate_index_schema_version(Some(certified.spool().schema_version()))?;
    if let Some(declared) = declared_schema_version.as_ref() {
        if !same_supported_schema_version(declared, certified.spool().schema_version()) {
            return Err(SourceError::Open(format!(
                "compact-only index schema_version {declared} does not match bounded decoder {:?} output schema_version {}",
                decoder.decoder_id(),
                certified.spool().schema_version()
            )));
        }
    }
    if certified.snapshot_dir() != snapshot_dir {
        return Err(SourceError::Open(
            "bounded decoder result changed the caller-owned snapshot spool path".to_string(),
        ));
    }
    let (spool, certified_snapshot_dir, prefold_receipt) = certified.into_parts();
    if prefold_receipt.source_len() != source_receipt.uncompressed_len()
        || prefold_receipt.source_sha256() != source_receipt.sha256()
    {
        return Err(SourceError::Open(
            "bounded decoder receipt is not bound to the authenticated container entry".to_string(),
        ));
    }
    owner.remove_authenticated_source()?;

    let first_entry = spool.entries().first().expect("portable spool is nonempty");
    let first = read_spooled_hour(&owner, &certified_snapshot_dir, &spool, first_entry)?;
    validate_snapshot_container_identity(
        store
            .container()
            .expect("spooled compact route requires container"),
        &first,
    )?;

    let design_catalog = read_portable_design_catalog(&store, &game_id)?;
    Ok(Box::new(SpooledPrefoldedSource {
        spool,
        snapshot_dir: certified_snapshot_dir,
        owner,
        warnings: Vec::new(),
        store,
        recorder_binding: metadata.binding,
        design_catalog,
        prefold_receipt,
    }))
}

fn validate_compact_stream_entry_identity(
    entry_path: &str,
    game_id: &str,
    timeline: Option<&Value>,
) -> Result<(), SourceError> {
    let game_prefix = format!("payload/{game_id}/");
    if !entry_path.starts_with(&game_prefix) {
        return Err(SourceError::Open(format!(
            "compact stream role entry {entry_path:?} is outside manifest game namespace {game_prefix:?}"
        )));
    }

    let Some(timeline) = timeline.and_then(Value::as_object) else {
        return Ok(());
    };
    let format = timeline.get("format").and_then(Value::as_str);
    let role = timeline.get("raw_stream_role").and_then(Value::as_str);
    let declares_compact =
        format == Some(COMPACT_STREAM_TIMELINE_FORMAT) || role == Some(COMPACT_STREAM_ROLE);
    if !declares_compact {
        return Ok(());
    }
    if format != Some(COMPACT_STREAM_TIMELINE_FORMAT) || role != Some(COMPACT_STREAM_ROLE) {
        return Err(SourceError::Open(format!(
            "compact timeline identity is incomplete: format={format:?}, raw_stream_role={role:?}"
        )));
    }
    let relative = timeline
        .get("stream")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            SourceError::Open("compact timeline declares no nonblank stream path".to_string())
        })?;
    if relative.starts_with('/')
        || relative.contains(char::from(92))
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || snapshot::zip::validate_path(relative).is_err()
    {
        return Err(SourceError::Open(format!(
            "compact timeline stream path is not a valid game-relative forward-slash path: {relative:?}"
        )));
    }
    let expected_path = format!("{game_prefix}{relative}");
    if entry_path != expected_path {
        return Err(SourceError::Open(format!(
            "compact stream role entry {entry_path:?} does not byte-match timeline stream {expected_path:?}"
        )));
    }
    Ok(())
}

fn validate_prefolded_container_identity(
    container: &snapshot::Container,
    replay: &snapshot::PrefoldedReplay,
) -> Result<(), SourceError> {
    let first = replay
        .entries()
        .first()
        .expect("PrefoldedReplay construction guarantees at least one hour")
        .snapshot();
    validate_snapshot_container_identity(container, first)
}

fn validate_snapshot_container_identity(
    container: &snapshot::Container,
    first: &snapshot::Snapshot,
) -> Result<(), SourceError> {
    let manifest = container.manifest();
    let provenance = first.rest.get("provenance");
    let manifest_modset = manifest
        .get("mod_identity")
        .and_then(|value| value.get("modset_id"))
        .and_then(Value::as_str);
    let snapshot_modset = provenance
        .and_then(|value| value.get("mod"))
        .and_then(|value| value.get("modset_id"))
        .and_then(Value::as_str);
    if let (Some(manifest), Some(snapshot_modset_id)) = (manifest_modset, snapshot_modset) {
        if manifest != snapshot_modset_id {
            let error = snapshot::ContainerError::ModsetMismatch {
                manifest: manifest.to_string(),
                snapshot: snapshot_modset_id.to_string(),
            };
            return Err(SourceError::ModsetMismatch(format!(
                "open .battleplan: {error}"
            )));
        }
    }

    let manifest_sha = manifest
        .get("build")
        .and_then(|value| value.get("exe_sha256"))
        .and_then(Value::as_str);
    for (label, snapshot_sha) in [
        (
            "collector",
            provenance
                .and_then(|value| value.get("collector"))
                .and_then(|value| value.get("exe_sha256"))
                .and_then(Value::as_str),
        ),
        (
            "memory",
            provenance
                .and_then(|value| value.get("memory"))
                .and_then(|value| value.get("exe_sha256"))
                .and_then(Value::as_str),
        ),
    ] {
        if let (Some(manifest), Some(snapshot)) = (manifest_sha, snapshot_sha) {
            if manifest != snapshot {
                return Err(SourceError::Open(format!(
                    "open .battleplan: container rejected: manifest build.exe_sha256 {manifest:?} disagrees with prefolded snapshot provenance.{label}.exe_sha256 {snapshot:?}"
                )));
            }
        }
    }
    Ok(())
}

fn read_portable_design_catalog(
    store: &Store,
    game_id: &str,
) -> Result<Option<Arc<crate::design_catalog::PortableDesignCatalog>>, SourceError> {
    let bytes = store
        .read_game_file_bytes(game_id, crate::design_catalog::PORTABLE_ARTIFACT_PATH)
        .map_err(|error| {
            SourceError::Open(format!(
                "read verified A27 design catalog artifact {:?}: {error}",
                crate::design_catalog::PORTABLE_ARTIFACT_PATH
            ))
        })?;
    bytes
        .map(|bytes| {
            crate::design_catalog::PortableDesignCatalog::decode_a27_live_json(&bytes)
                .map(Arc::new)
                .map_err(|error| {
                    SourceError::Open(format!("invalid A27 design catalog artifact: {error}"))
                })
        })
        .transpose()
}

fn validate_index_schema_version(version: Option<&Value>) -> Result<(), SourceError> {
    match version {
        // The canonical JSON writer uses a string. Numeric 1 is retained for
        // legacy directory stores that predate the canonical string spelling.
        None => Ok(()),
        Some(Value::String(value)) if value == "1" => Ok(()),
        Some(Value::Number(value)) if value.as_u64() == Some(1) => Ok(()),
        Some(value) => Err(SourceError::Unsupported(format!(
            "incompatible replay index schema_version {value}; this standalone replayer supports schema_version \"1\""
        ))),
    }
}

fn same_supported_schema_version(left: &Value, right: &Value) -> bool {
    fn is_v1(value: &Value) -> bool {
        matches!(value, Value::String(text) if text == "1")
            || matches!(value, Value::Number(number) if number.as_u64() == Some(1))
    }
    is_v1(left) && is_v1(right)
}

fn recorder_metadata_route(store: &Store, game_id: &str) -> Result<MetadataRoute, SourceError> {
    if let Some(container) = store.container() {
        let timeline_path = format!("payload/{game_id}/timeline.json");
        let timeline_roles = container.entries_by_role("timeline");
        if timeline_roles.len() > 1 {
            return Err(SourceError::Open(format!(
                "timeline role reconciliation found multiple entries: {:?}",
                timeline_roles
                    .iter()
                    .map(|entry| entry.path.as_str())
                    .collect::<Vec<_>>()
            )));
        }
        if let Some(entry) = timeline_roles.first() {
            if entry.path != timeline_path {
                return Err(SourceError::Open(format!(
                    "timeline role entry {:?} does not byte-match canonical {timeline_path:?}",
                    entry.path
                )));
            }
        }

        let recorder_roles = container.entries_by_role("recorder_manifest");
        let timeline_listed = container
            .entries()
            .iter()
            .any(|entry| entry.path == timeline_path);
        if !timeline_listed {
            if recorder_roles.is_empty() {
                return Ok(MetadataRoute {
                    binding: None,
                    timeline: None,
                });
            }
            return Err(SourceError::Open(format!(
                "recorder_manifest role entry exists but canonical timeline {timeline_path:?} is absent"
            )));
        }

        let timeline_bytes = container.read_entry_bytes(&timeline_path).map_err(|error| {
            SourceError::Open(format!(
                "listed canonical timeline {timeline_path:?} could not be read and verified: {error}"
            ))
        })?;
        let timeline = parse_metadata_object(&timeline_bytes, "timeline.json")?;
        let reference = session_manifest_reference(&timeline)?;
        let Some(reference) = reference else {
            if recorder_roles.is_empty() {
                return Ok(MetadataRoute {
                    binding: None,
                    timeline: Some(Value::Object(timeline)),
                });
            }
            return Err(SourceError::Open(
                "timeline.json.session_manifest is absent but recorder_manifest role entry exists"
                    .into(),
            ));
        };

        if recorder_roles.len() != 1 {
            return Err(SourceError::Open(format!(
                "timeline.json.session_manifest {reference:?} requires exactly one recorder_manifest role entry; found {}",
                recorder_roles.len()
            )));
        }
        let expected_path = format!("payload/{game_id}/{reference}");
        let entry = &recorder_roles[0];
        if entry.path != expected_path {
            return Err(SourceError::Open(format!(
                "timeline.json.session_manifest {reference:?} expects recorder_manifest path {expected_path:?}, found {:?}",
                entry.path
            )));
        }
        let manifest_bytes = container.read_entry_bytes(&entry.path).map_err(|error| {
            SourceError::Open(format!(
                "read recorder_manifest {:?} referenced by timeline.json.session_manifest: {error}",
                entry.path
            ))
        })?;
        let binding = recorder_manifest_binding(&manifest_bytes, &reference)?;
        return Ok(MetadataRoute {
            binding: Some(binding),
            timeline: Some(Value::Object(timeline)),
        });
    }

    directory_recorder_metadata_route_with_reader(|relative| {
        store
            .read_game_file_bytes(game_id, relative)
            .map_err(|error| {
                SourceError::Open(format!(
                    "handle-relative read of {game_id:?}/{relative}: {error}"
                ))
            })
    })
}

/// Directory metadata classification and binding over a read-once object API.
/// The production closure performs one handle-relative final-handle read; the
/// counting test uses this same route and makes any second read observable.
fn directory_recorder_metadata_route_with_reader<F>(
    mut read_once: F,
) -> Result<MetadataRoute, SourceError>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>, SourceError>,
{
    let timeline =
        read_optional_metadata_object_once(|| read_once("timeline.json"), "timeline.json")?;
    let Some(timeline) = timeline else {
        return Ok(MetadataRoute {
            binding: None,
            timeline: None,
        });
    };
    let Some(reference) = session_manifest_reference(&timeline)? else {
        return Ok(MetadataRoute {
            binding: None,
            timeline: Some(Value::Object(timeline)),
        });
    };
    let manifest = read_optional_metadata_object_once(
        || read_once(&reference),
        &format!("recorder manifest {reference:?}"),
    )?
    .ok_or_else(|| {
        SourceError::Open(format!(
            "timeline.json.session_manifest references missing recorder manifest {reference:?}"
        ))
    })?;
    let binding = recorder_manifest_binding_value(manifest, &reference)?;
    Ok(MetadataRoute {
        binding: Some(binding),
        timeline: Some(Value::Object(timeline)),
    })
}

fn parse_metadata_object(bytes: &[u8], context: &str) -> Result<Map<String, Value>, SourceError> {
    let value = snapshot::parse_json_strict(bytes)
        .map_err(|error| SourceError::Open(format!("strict parse {context}: {error}")))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| SourceError::Open(format!("{context} must decode as a JSON object")))
}

fn read_optional_metadata_object_once<F>(
    read_once: F,
    context: &str,
) -> Result<Option<Map<String, Value>>, SourceError>
where
    F: FnOnce() -> Result<Option<Vec<u8>>, SourceError>,
{
    read_once()?
        .map(|bytes| parse_metadata_object(&bytes, context))
        .transpose()
}

fn session_manifest_reference(
    timeline: &Map<String, Value>,
) -> Result<Option<String>, SourceError> {
    let Some(value) = timeline.get("session_manifest") else {
        return Ok(None);
    };
    let reference = value.as_str().ok_or_else(|| {
        SourceError::Open("timeline.json.session_manifest is present but is not a string".into())
    })?;
    if reference.trim().is_empty()
        || reference.starts_with('/')
        || reference.contains(char::from(92))
        || reference
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || snapshot::zip::validate_path(reference).is_err()
    {
        return Err(SourceError::Open(format!(
            "timeline.json.session_manifest is not a valid game-relative forward-slash path: {reference:?}"
        )));
    }
    Ok(Some(reference.to_string()))
}

fn recorder_manifest_binding(
    bytes: &[u8],
    reference: &str,
) -> Result<RecorderSessionBinding, SourceError> {
    let root = parse_metadata_object(bytes, &format!("recorder manifest {reference:?}"))?;
    recorder_manifest_binding_value(root, reference)
}

fn recorder_manifest_binding_value(
    root: Map<String, Value>,
    reference: &str,
) -> Result<RecorderSessionBinding, SourceError> {
    let modset = root
        .get("modset")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset is missing or not an object"
            ))
        })?;

    match modset.get("resolved") {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => {
            let attributed = modset
                .get("errors")
                .and_then(Value::as_array)
                .map(|errors| {
                    errors
                        .iter()
                        .map(|error| {
                            error
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| format!("<non-string {error}>"))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec!["<missing or malformed errors>".into()]);
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} modset is unresolved; attributed errors: {attributed:?}"
            )));
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} modset.resolved is missing or not boolean"
            )))
        }
    }

    match modset.get("errors") {
        Some(Value::Array(errors)) if errors.is_empty() => {}
        Some(Value::Array(errors)) => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} resolved modset must have errors == []; found {errors:?}"
            )))
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} resolved modset errors is missing or not an array"
            )))
        }
    }

    let dlc = modset
        .get("dlc")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc is missing or not an object"
            ))
        })?;
    let enabled = dlc
        .get("enabled")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc.enabled is missing or not an array"
            ))
        })?;
    let enabled_paths: Vec<&str> = enabled
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder manifest {reference:?} modset.dlc.enabled[{index}] is not a string"
                ))
            })
        })
        .collect::<Result<_, _>>()?;

    let display_names = modset
        .get("dlc_display_names")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc_display_names is missing or not an object"
            ))
        })?;
    let mut active_dlcs = Vec::with_capacity(enabled_paths.len());
    for path in enabled_paths {
        let display = display_names
            .get(path)
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder metadata binding check: enabled DLC path {path:?} has no exactly-mapped nonblank display name"
                ))
            })?;
        active_dlcs.push(display.to_string());
    }
    let localisation_language = modset
        .get("localisation_language")
        .and_then(Value::as_str)
        .filter(|language| !language.trim().is_empty())
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.localisation_language is missing or blank"
            ))
        })?
        .to_string();
    let display_variant: RecorderDisplayVariant =
        serde_json::from_value(modset.get("display_variant").cloned().ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.display_variant is missing"
            ))
        })?)
        .map_err(|error| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.display_variant has invalid shape: {error}"
            ))
        })?;
    validate_display_variant(&display_variant, reference)?;

    Ok(RecorderSessionBinding {
        active_dlcs,
        localisation_language,
        display_variant,
    })
}

fn validate_display_variant(
    display: &RecorderDisplayVariant,
    reference: &str,
) -> Result<(), SourceError> {
    let scale = f32::from_bits(display.gui_scale_f32_bits);
    let expected_mode = match (display.fullscreen, display.borderless) {
        (true, false) => "fullscreen",
        (false, true) => "borderless_fullscreen",
        (false, false) => "windowed",
        (true, true) => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} display_variant flags are ambiguous"
            )))
        }
    };
    let sha_ok = display.settings_file_sha256.len() == 64
        && display
            .settings_file_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if display.physical_width == 0
        || display.physical_height == 0
        || display.min_gui_width == 0
        || display.min_gui_height == 0
        || display.refresh_rate_hz == 0
        || !scale.is_finite()
        || scale <= 0.0
        || display.display_mode != expected_mode
        || !sha_ok
    {
        return Err(SourceError::Open(format!(
            "recorder manifest {reference:?} modset.display_variant violates the exact launch-settings contract"
        )));
    }
    Ok(())
}

/// Precise Unsupported message for a recording whose snapshot index is empty,
/// naming the compact-stream evidence when it is present.
fn empty_index_message(store: &Store, timeline: Option<&Value>, game_id: &str) -> String {
    let contract = "playback requires either per-hour snapshot entries per \
                    HOI4REPLAY_CONTAINER_SPEC.md or an explicitly supplied certified \
                    portable pre-folder; ordinary product open does not guess or fold \
                    compact record layouts";
    match compact_stream_evidence(store, timeline) {
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
fn compact_stream_evidence(store: &Store, timeline: Option<&Value>) -> Option<String> {
    if let Some(container) = store.container() {
        let compact = container.entries_by_role(COMPACT_STREAM_ROLE);
        if let Some(entry) = compact.first() {
            return Some(format!(
                "manifest entry {:?} has role {COMPACT_STREAM_ROLE:?}",
                entry.path
            ));
        }
        return timeline.and_then(timeline_compact_evidence);
    }
    timeline.and_then(timeline_compact_evidence)
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
static PREFOLD_SPOOL_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const PREFOLD_SPOOL_MARKER: &str = ".hoi4-prefold-owner";

/// RAII ownership for one private bounded pre-fold workspace. Cleanup is
/// attempted only when the exact canonical directory and unique-per-open
/// marker still match; an identity discrepancy leaves evidence in place rather
/// than recursively deleting an unknown target.
struct PrefoldSpoolOwner {
    root: PathBuf,
    canonical_root: PathBuf,
    snapshot_dir: PathBuf,
    marker_value: String,
}

impl PrefoldSpoolOwner {
    fn create(base: &Path) -> Result<Self, SourceError> {
        let meta = fs::symlink_metadata(base).map_err(|error| {
            SourceError::Open(format!(
                "inspect prefold spool base {}: {error}",
                base.display()
            ))
        })?;
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return Err(SourceError::Open(format!(
                "prefold spool base is not a regular non-symlink directory: {}",
                base.display()
            )));
        }
        let canonical_base = fs::canonicalize(base).map_err(|error| {
            SourceError::Open(format!(
                "canonicalize prefold spool base {}: {error}",
                base.display()
            ))
        })?;
        let sequence = PREFOLD_SPOOL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| SourceError::Open(format!("prefold spool clock: {error}")))?
            .as_nanos();
        let marker_value = format!("{}-{now:032x}-{sequence:016x}", std::process::id());

        for attempt in 0..64u64 {
            let root = canonical_base.join(format!(
                ".hoi4-prefold-{}-{now:032x}-{:016x}",
                std::process::id(),
                sequence.wrapping_add(attempt)
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let canonical_root = match fs::canonicalize(&root) {
                        Ok(path) if path.parent() == Some(canonical_base.as_path()) => path,
                        Ok(_) => {
                            let _ = fs::remove_dir(&root);
                            return Err(SourceError::Open(
                                "private prefold spool escaped its canonical base".to_string(),
                            ));
                        }
                        Err(error) => {
                            let _ = fs::remove_dir(&root);
                            return Err(SourceError::Open(format!(
                                "canonicalize private prefold spool {}: {error}",
                                root.display()
                            )));
                        }
                    };
                    let setup = (|| -> Result<PathBuf, SourceError> {
                        let marker_path = canonical_root.join(PREFOLD_SPOOL_MARKER);
                        let mut marker = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&marker_path)
                            .map_err(|error| {
                                SourceError::Open(format!(
                                    "create prefold ownership marker {}: {error}",
                                    marker_path.display()
                                ))
                            })?;
                        marker.write_all(marker_value.as_bytes()).map_err(|error| {
                            SourceError::Open(format!(
                                "write prefold ownership marker {}: {error}",
                                marker_path.display()
                            ))
                        })?;
                        marker.sync_all().map_err(|error| {
                            SourceError::Open(format!(
                                "sync prefold ownership marker {}: {error}",
                                marker_path.display()
                            ))
                        })?;
                        let snapshot_dir = canonical_root.join("snapshots");
                        fs::create_dir(&snapshot_dir).map_err(|error| {
                            SourceError::Open(format!(
                                "create private snapshot spool {}: {error}",
                                snapshot_dir.display()
                            ))
                        })?;
                        Ok(snapshot_dir)
                    })();
                    match setup {
                        Ok(snapshot_dir) => {
                            return Ok(Self {
                                root: canonical_root.clone(),
                                canonical_root,
                                snapshot_dir,
                                marker_value,
                            });
                        }
                        Err(error) => {
                            if fs::canonicalize(&root).ok().as_deref()
                                == Some(canonical_root.as_path())
                            {
                                let _ = fs::remove_dir_all(&root);
                            }
                            return Err(error);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(SourceError::Open(format!(
                        "create private prefold spool {}: {error}",
                        root.display()
                    )))
                }
            }
        }
        Err(SourceError::Open(
            "could not allocate a unique private prefold spool directory".to_string(),
        ))
    }

    fn root(&self) -> &Path {
        &self.root
    }
    fn snapshot_dir(&self) -> &Path {
        &self.snapshot_dir
    }

    fn identity_intact(&self) -> bool {
        let Ok(meta) = fs::symlink_metadata(&self.root) else {
            return false;
        };
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return false;
        }
        if fs::canonicalize(&self.root).ok().as_deref() != Some(self.canonical_root.as_path()) {
            return false;
        }
        let Ok(snapshot_meta) = fs::symlink_metadata(&self.snapshot_dir) else {
            return false;
        };
        if !snapshot_meta.file_type().is_dir() || snapshot_meta.file_type().is_symlink() {
            return false;
        }
        if fs::canonicalize(&self.snapshot_dir).ok().as_deref() != Some(self.snapshot_dir.as_path())
        {
            return false;
        }
        let marker = self.root.join(PREFOLD_SPOOL_MARKER);
        let Ok(marker_meta) = fs::symlink_metadata(&marker) else {
            return false;
        };
        if !marker_meta.file_type().is_file() || marker_meta.file_type().is_symlink() {
            return false;
        }
        if fs::read_to_string(marker).ok().as_deref() != Some(self.marker_value.as_str()) {
            return false;
        }
        let Ok(root_entries) = fs::read_dir(&self.root) else {
            return false;
        };
        for entry in root_entries {
            let Ok(entry) = entry else {
                return false;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return false;
            };
            if name != PREFOLD_SPOOL_MARKER && name != "source.h4cr" && name != "snapshots" {
                return false;
            }
            if name == "source.h4cr" {
                let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                    return false;
                };
                if !meta.file_type().is_file() || meta.file_type().is_symlink() {
                    return false;
                }
                let Ok(canonical) = fs::canonicalize(entry.path()) else {
                    return false;
                };
                if canonical.parent() != Some(self.root.as_path()) {
                    return false;
                }
            }
        }
        let Ok(snapshot_entries) = fs::read_dir(&self.snapshot_dir) else {
            return false;
        };
        for entry in snapshot_entries {
            let Ok(entry) = entry else {
                return false;
            };
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                return false;
            };
            if !meta.file_type().is_file() || meta.file_type().is_symlink() {
                return false;
            }
            let Ok(canonical) = fs::canonicalize(entry.path()) else {
                return false;
            };
            if canonical.parent() != Some(self.snapshot_dir.as_path()) {
                return false;
            }
        }
        true
    }

    fn require_identity(&self) -> Result<(), SourceError> {
        if self.identity_intact() {
            Ok(())
        } else {
            Err(SourceError::Open(
                "private prefold spool identity changed; refusing read and automatic cleanup"
                    .to_string(),
            ))
        }
    }

    fn remove_authenticated_source(&self) -> Result<(), SourceError> {
        self.require_identity()?;
        let path = self.root.join("source.h4cr");
        let meta = fs::symlink_metadata(&path).map_err(|error| {
            SourceError::Open(format!(
                "inspect authenticated source spool {}: {error}",
                path.display()
            ))
        })?;
        let canonical = fs::canonicalize(&path).map_err(|error| {
            SourceError::Open(format!(
                "canonicalize authenticated source spool {}: {error}",
                path.display()
            ))
        })?;
        if !meta.file_type().is_file()
            || meta.file_type().is_symlink()
            || canonical.parent() != Some(self.root.as_path())
        {
            return Err(SourceError::Open(
                "authenticated source spool identity changed before cleanup".to_string(),
            ));
        }
        fs::remove_file(&path).map_err(|error| {
            SourceError::Open(format!(
                "remove authenticated source spool {}: {error}",
                path.display()
            ))
        })
    }
}

impl Drop for PrefoldSpoolOwner {
    fn drop(&mut self) {
        if self.identity_intact() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn read_spooled_hour(
    owner: &PrefoldSpoolOwner,
    snapshot_dir: &Path,
    spool: &snapshot::PrefoldedSpool,
    entry: &snapshot::PrefoldedSpoolEntry,
) -> Result<snapshot::Snapshot, SourceError> {
    owner.require_identity()?;
    if fs::canonicalize(snapshot_dir).ok().as_deref() != Some(owner.snapshot_dir()) {
        return Err(SourceError::Open(
            "private snapshot spool identity changed".to_string(),
        ));
    }
    let path = snapshot_dir.join(entry.file_name());
    let canonical = fs::canonicalize(&path).map_err(|error| {
        SourceError::Open(format!(
            "canonicalize spooled snapshot {}: {error}",
            path.display()
        ))
    })?;
    if canonical.parent() != Some(snapshot_dir) {
        return Err(SourceError::Open(format!(
            "spooled snapshot escaped private directory: {}",
            path.display()
        )));
    }
    let path_meta = fs::symlink_metadata(&path).map_err(|error| {
        SourceError::Open(format!(
            "inspect spooled snapshot {}: {error}",
            path.display()
        ))
    })?;
    if !path_meta.file_type().is_file()
        || path_meta.file_type().is_symlink()
        || path_meta.len() != entry.byte_len()
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot metadata changed for hour {}",
            entry.hour_index()
        )));
    }
    let file = fs::File::open(&canonical).map_err(|error| {
        SourceError::Open(format!(
            "open spooled snapshot {}: {error}",
            canonical.display()
        ))
    })?;
    if file
        .metadata()
        .map_err(|error| SourceError::Open(error.to_string()))?
        .len()
        != entry.byte_len()
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot length changed while opening hour {}",
            entry.hour_index()
        )));
    }
    let mut bytes = Vec::new();
    file.take(entry.byte_len().saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            SourceError::Open(format!(
                "read spooled snapshot for hour {}: {error}",
                entry.hour_index()
            ))
        })?;
    if bytes.len() as u64 != entry.byte_len() {
        return Err(SourceError::Open(format!(
            "spooled snapshot byte count changed for hour {}",
            entry.hour_index()
        )));
    }
    let actual_sha = snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&bytes));
    if actual_sha != entry.sha256() {
        return Err(SourceError::Open(format!(
            "spooled snapshot SHA-256 changed for hour {}",
            entry.hour_index()
        )));
    }
    let strict = snapshot::parse_json_strict(&bytes).map_err(|error| {
        SourceError::Open(format!(
            "spooled hour {} is not strict snapshot JSON: {error}",
            entry.hour_index()
        ))
    })?;
    let snapshot: snapshot::Snapshot = serde_json::from_value(strict).map_err(|error| {
        SourceError::Open(format!(
            "spooled hour {} is not canonical snapshot JSON: {error}",
            entry.hour_index()
        ))
    })?;
    if snapshot.hour_index != Some(entry.hour_index())
        || snapshot.game_id.as_deref() != Some(spool.game_id())
        || snapshot.schema_version.as_ref() != Some(spool.schema_version())
        || snapshot.date.as_deref() != Some(entry.date())
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot identity disagrees with certified manifest at hour {}",
            entry.hour_index()
        )));
    }
    Ok(snapshot)
}

struct StoreSource {
    store: Store,
    game_id: String,
    hours: Vec<i64>,
    files_by_hour: BTreeMap<i64, String>,
    warnings: Vec<String>,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
}

struct SpooledPrefoldedSource {
    spool: snapshot::PrefoldedSpool,
    snapshot_dir: PathBuf,
    owner: PrefoldSpoolOwner,
    warnings: Vec<String>,
    store: Store,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    prefold_receipt: snapshot::PrefoldSourceReceipt,
}

/// Neutral, immutable already-folded backend. The portable contract owns the
/// index/entry identity checks; this adapter only projects each typed snapshot
/// into replay_core's existing raw-frame boundary.
struct PrefoldedSource {
    replay: snapshot::PrefoldedReplay,
    warnings: Vec<String>,
    store: Option<Store>,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    prefold_receipt: Option<snapshot::PrefoldSourceReceipt>,
}

impl ReplaySource for SpooledPrefoldedSource {
    fn game_id(&self) -> &str {
        self.spool.game_id()
    }
    fn hours(&self) -> &[i64] {
        self.spool.hours()
    }
    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .container()
            .and_then(|container| container.content_manifest())
    }

    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        Some(&self.prefold_receipt)
    }

    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let entry = self.spool.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no spooled prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        let snapshot = read_spooled_hour(&self.owner, &self.snapshot_dir, &self.spool, entry)?;
        let mut rest = snapshot.rest;
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snapshot.date,
            save_version: snapshot.save_version,
            schema_version: snapshot.schema_version,
            rest,
        })
    }
}

impl ReplaySource for PrefoldedSource {
    fn game_id(&self) -> &str {
        self.replay.index().game_id()
    }

    fn hours(&self) -> &[i64] {
        self.replay.index().hours()
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .as_ref()
            .and_then(Store::container)
            .and_then(|container| container.content_manifest())
    }

    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        self.prefold_receipt.as_ref()
    }

    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let entry = self.replay.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        let snapshot = entry.snapshot();
        let mut rest = snapshot.rest.clone();
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snapshot.date.clone(),
            save_version: snapshot.save_version.clone(),
            schema_version: snapshot.schema_version.clone(),
            rest,
        })
    }
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
    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .container()
            .and_then(|container| container.content_manifest())
    }
    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
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
        let mut rest = snap.rest;
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snap.date,
            save_version: snap.save_version,
            schema_version: snap.schema_version,
            rest,
        })
    }
}

fn validate_existing_recorder_metadata(
    rest: &Map<String, Value>,
    binding: &RecorderSessionBinding,
) -> Result<(), SourceError> {
    if let Some(session_value) = rest.get("session") {
        let session = session_value
            .as_object()
            .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
        check_string_list(
            session.get("active_dlcs"),
            "session.active_dlcs",
            &binding.active_dlcs,
        )?;
        check_string_list(session.get("dlcs"), "session.dlcs", &binding.active_dlcs)?;
        check_string(
            session.get("localisation_language"),
            "session.localisation_language",
            &binding.localisation_language,
        )?;
        check_string(
            session.get("language"),
            "session.language",
            &binding.localisation_language,
        )?;
        check_exact_value(
            session.get("display_variant"),
            "session.display_variant",
            &serde_json::json!(binding.display_variant),
        )?;
    }

    if let Some(world_value) = rest.get("world") {
        let world = world_value
            .as_object()
            .ok_or_else(|| SourceError::Open("snapshot world is not an object".into()))?;
        if let Some(scenario_value) = world.get("scenario") {
            let scenario = scenario_value.as_object().ok_or_else(|| {
                SourceError::Open("snapshot world.scenario is not an object".into())
            })?;
            check_string_list(
                scenario.get("dlcs"),
                "world.scenario.dlcs",
                &binding.active_dlcs,
            )?;
            check_string(
                scenario.get("localisation_language"),
                "world.scenario.localisation_language",
                &binding.localisation_language,
            )?;
            check_string(
                scenario.get("language"),
                "world.scenario.language",
                &binding.localisation_language,
            )?;
        }
    }
    Ok(())
}

fn check_exact_value(
    value: Option<&Value>,
    context: &str,
    expected: &Value,
) -> Result<(), SourceError> {
    if value.is_some_and(|actual| actual != expected) {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict: {context} does not match launch-bound manifest metadata"
        )));
    }
    Ok(())
}

fn check_string_list(
    value: Option<&Value>,
    context: &str,
    expected: &[String],
) -> Result<(), SourceError> {
    let Some(value) = value else {
        return Ok(());
    };
    let array = value.as_array().ok_or_else(|| {
        SourceError::Open(format!(
            "recorder metadata conflict: {context} is not a string array"
        ))
    })?;
    let actual: Vec<String> = array
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder metadata conflict: {context}[{index}] is not a string"
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    if actual != expected {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict at {context}: snapshot {actual:?} != recorder binding {expected:?}"
        )));
    }
    Ok(())
}

fn check_string(value: Option<&Value>, context: &str, expected: &str) -> Result<(), SourceError> {
    let Some(value) = value else {
        return Ok(());
    };
    let actual = value.as_str().ok_or_else(|| {
        SourceError::Open(format!(
            "recorder metadata conflict: {context} is not a string"
        ))
    })?;
    if actual != expected {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict at {context}: snapshot {actual:?} != recorder binding {expected:?}"
        )));
    }
    Ok(())
}

fn bind_or_match(
    object: &mut Map<String, Value>,
    key: &str,
    binding: Value,
) -> Result<(), SourceError> {
    match object.get(key) {
        None => {
            object.insert(key.to_string(), binding);
            Ok(())
        }
        Some(existing) if *existing == binding => Ok(()),
        Some(existing) => Err(SourceError::Open(format!(
            "recorder metadata bind_or_match conflict at session.{key}: snapshot {existing} != recorder binding {binding}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "replay_source_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn private_prefold_spool_cleanup_is_identity_gated() {
        let base = temp_root("prefold_spool_owner");
        let owner = PrefoldSpoolOwner::create(&base).unwrap();
        let owned_root = owner.root().to_path_buf();
        drop(owner);
        assert!(!owned_root.exists(), "intact private spool must be cleaned");

        let owner = PrefoldSpoolOwner::create(&base).unwrap();
        let preserved_root = owner.root().to_path_buf();
        fs::write(preserved_root.join(PREFOLD_SPOOL_MARKER), "different-owner").unwrap();
        drop(owner);
        assert!(
            preserved_root.exists(),
            "identity mismatch must preserve evidence instead of recursive deletion"
        );
        fs::remove_dir_all(&preserved_root).unwrap();
        fs::remove_dir_all(&base).unwrap();
    }

    fn write_playable_store(root: &Path, game: &str, recorder: bool, label: &str) {
        let game_dir = root.join(game);
        fs::create_dir_all(game_dir.join("recorder")).unwrap();
        fs::write(
            game_dir.join("index.json"),
            serde_json::to_vec(&json!({
                "game_id": game,
                "snapshots": [{"hour_index": 1, "file": "000000000001.snapshot.json"}]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            game_dir.join("000000000001.snapshot.json"),
            serde_json::to_vec(&json!({
                "game_id": game,
                "hour_index": 1,
                "marker": label
            }))
            .unwrap(),
        )
        .unwrap();
        if recorder {
            fs::write(
                game_dir.join("timeline.json"),
                br#"{"session_manifest":"recorder/native_session_manifest.json"}"#,
            )
            .unwrap();
            fs::write(
                game_dir
                    .join("recorder")
                    .join("native_session_manifest.json"),
                serde_json::to_vec(&json!({
                    "modset": {
                        "resolved": true,
                        "errors": [],
                        "dlc": {"enabled": ["dlc/a.dlc"]},
                        "dlc_display_names": {"dlc/a.dlc": "DLC A"},
                        "localisation_language": "l_english",
                        "display_variant": {
                            "physical_width": 2752, "physical_height": 1152,
                            "min_gui_width": 1280, "min_gui_height": 720,
                            "gui_scale_f32_bits": 1.0f32.to_bits(),
                            "refresh_rate_hz": 60, "max_refresh_rate_hz": 75,
                            "fullscreen": false, "borderless": true,
                            "display_mode": "borderless_fullscreen",
                            "settings_file_sha256": "aa".repeat(32)
                        }
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        } else {
            fs::write(game_dir.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
        }
    }

    fn prefolded_entry(hour: i64, schema: &str) -> snapshot::PrefoldedHourEntry {
        let bytes = serde_json::to_vec(&json!({
            "game_id": "prefolded-game",
            "schema_version": schema,
            "hour_index": hour,
            "date": "1936.1.1.12",
            "save_version": "a729",
            "countries": {"FRA": {"stability": 0.5}}
        }))
        .unwrap();
        snapshot::PrefoldedHourEntry::from_json(hour, &bytes).unwrap()
    }

    #[test]
    fn neutral_prefolded_hours_are_admitted_through_the_existing_frame_boundary() {
        let replay = snapshot::PrefoldedReplay::try_new(
            "prefolded-game",
            Value::String("1".into()),
            vec![prefolded_entry(10, "1"), prefolded_entry(12, "1")],
        )
        .unwrap();
        let source = open_prefolded_replay(replay).unwrap();

        assert_eq!(source.game_id(), "prefolded-game");
        assert_eq!(source.hours(), &[10, 12]);
        assert!(source.warnings().is_empty());
        let frame = source.frame(12).unwrap();
        assert_eq!(frame.hour_index, 12);
        assert_eq!(frame.date.as_deref(), Some("1936.1.1.12"));
        assert_eq!(frame.save_version.as_deref(), Some("a729"));
        assert_eq!(frame.schema_version, Some(Value::String("1".into())));
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.5));
        assert!(source.frame(11).is_err());
    }

    #[test]
    fn prefolded_admission_rejects_an_unsupported_schema_before_exposing_a_source() {
        let replay = snapshot::PrefoldedReplay::try_new(
            "prefolded-game",
            Value::String("2".into()),
            vec![prefolded_entry(10, "2")],
        )
        .unwrap();
        assert!(matches!(
            open_prefolded_replay(replay),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn raw_h4cr_compact_package_remains_fail_closed_at_the_file_boundary() {
        let root = temp_root("raw_h4cr_stays_closed");
        let game = root.join("g");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.json"),
            br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#,
        )
        .unwrap();
        fs::write(
            game.join("timeline.json"),
            br#"{"format":"hoi4_compact_stream_timeline_v1","raw_stream_role":"collector_compact_stream"}"#,
        )
        .unwrap();
        fs::write(game.join("collector_stream.bin"), b"H4CR").unwrap();

        let error = match open_recording(&root) {
            Err(error) => error,
            Ok(_) => panic!("raw H4CR package must not enter replay_core directly"),
        };
        assert!(matches!(error, SourceError::Unsupported(_)));
        let message = error.to_string();
        assert!(message.contains("compact-stream-only"), "{message}");
        assert!(message.contains("collector_compact_stream"), "{message}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn standalone_replay_manifest_has_no_extractor_dependency() {
        let manifest = include_str!("../Cargo.toml").replace('\\', "/");
        assert!(!manifest.contains("extractor/"), "{manifest}");
    }

    #[test]
    fn root_swap_full_lifecycle_stays_in_anchored_graph() {
        let parent = temp_root("root_swap");
        let root = parent.join("store");
        fs::create_dir_all(&root).unwrap();
        write_playable_store(&root, "g", true, "ANCHORED");
        let moved = parent.join("anchored_object");
        let source = open_recording_impl_with_post_anchor(&root, None, None, None, |_| {
            fs::rename(&root, &moved).unwrap();
            fs::create_dir_all(&root).unwrap();
            write_playable_store(&root, "g", false, "IMPOSTOR");
        })
        .unwrap();

        let frame = source.frame(1).unwrap();
        assert_eq!(frame.rest["marker"], "ANCHORED");
        assert_eq!(frame.rest["session"]["active_dlcs"], json!(["DLC A"]));
        assert_eq!(frame.rest["session"]["localisation_language"], "l_english");
        assert_eq!(
            frame.rest["session"]["display_variant"]["physical_width"],
            2752
        );
        drop(source);
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn empty_index_evidence_after_root_swap_uses_anchored_timeline() {
        let parent = temp_root("empty_root_swap");
        let root = parent.join("store");
        let game = "g";
        let game_dir = root.join(game);
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(
            game_dir.join("index.json"),
            serde_json::to_vec(&json!({"game_id": game, "snapshots": []})).unwrap(),
        )
        .unwrap();
        fs::write(
            game_dir.join("timeline.json"),
            br#"{"format":"hoi4_compact_stream_timeline_v1"}"#,
        )
        .unwrap();
        let moved = parent.join("anchored_object");
        let result = open_recording_impl_with_post_anchor(&root, None, None, None, |_| {
            fs::rename(&root, &moved).unwrap();
            let impostor = root.join(game);
            fs::create_dir_all(&impostor).unwrap();
            fs::write(
                impostor.join("index.json"),
                serde_json::to_vec(&json!({"game_id": game, "snapshots": []})).unwrap(),
            )
            .unwrap();
            fs::write(impostor.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("empty index must be unsupported"),
        };
        let message = error.to_string();
        assert!(
            message.contains("hoi4_compact_stream_timeline_v1"),
            "{message}"
        );
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn directory_route_reads_each_metadata_object_once_and_uses_first_buffer() {
        let mut calls = BTreeMap::<String, usize>::new();
        let route = directory_recorder_metadata_route_with_reader(|relative| {
            let count = calls.entry(relative.to_string()).or_default();
            *count += 1;
            if *count > 1 {
                return Ok(Some(
                    br#"{"legacy":true,"second_read_must_not_bind":true}"#.to_vec(),
                ));
            }
            let bytes = match relative {
                "timeline.json" => {
                    br#"{"session_manifest":"recorder/native_session_manifest.json"}"#.as_slice()
                }
                "recorder/native_session_manifest.json" => br#"{"modset":{"resolved":true,"errors":[],"dlc":{"enabled":["dlc/a.dlc"]},"dlc_display_names":{"dlc/a.dlc":"DLC A"},"localisation_language":"l_english","display_variant":{"physical_width":2752,"physical_height":1152,"min_gui_width":1280,"min_gui_height":720,"gui_scale_f32_bits":1065353216,"refresh_rate_hz":60,"max_refresh_rate_hz":75,"fullscreen":false,"borderless":true,"display_mode":"borderless_fullscreen","settings_file_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}"#,
                other => panic!("unexpected metadata read {other:?}"),
            };
            Ok(Some(bytes.to_vec()))
        })
        .unwrap();

        let binding = route.binding.expect("first buffers must produce a binding");
        assert_eq!(binding.active_dlcs, vec!["DLC A"]);
        assert_eq!(binding.localisation_language, "l_english");
        assert_eq!(binding.display_variant.physical_width, 2752);
        assert_eq!(
            calls.get("timeline.json").copied(),
            Some(1),
            "timeline final object must be opened/read exactly once"
        );
        assert_eq!(
            calls.get("recorder/native_session_manifest.json").copied(),
            Some(1),
            "recorder-manifest final object must be opened/read exactly once"
        );
    }

    #[test]
    fn selected_game_directory_swap_before_frame_is_rejected_without_cross_object_mix() {
        let parent = temp_root("selected_game_swap");
        let root = parent.join("store");
        fs::create_dir_all(&root).unwrap();
        write_playable_store(&root, "g", true, "OBJECT_A");
        let source = open_recording(&root).unwrap();

        let original = root.join("g");
        let moved = root.join("game_object_a");
        fs::rename(&original, &moved).unwrap();
        write_playable_store(&root, "g", false, "OBJECT_B");

        let error = source
            .frame(1)
            .expect_err("a selected-game object swap must fail closed")
            .to_string();
        assert!(
            error.contains("identity") || error.contains("namespace"),
            "{error}"
        );
        drop(source);
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn timeline_to_manifest_game_parent_swap_is_rejected() {
        let parent = temp_root("timeline_manifest_parent_swap");
        let root = parent.join("store");
        fs::create_dir_all(&root).unwrap();
        write_playable_store(&root, "g", true, "OBJECT_A");
        let store = Store::open_directory(&root).unwrap();
        let mut swapped = false;

        let result = directory_recorder_metadata_route_with_reader(|relative| {
            let bytes = store
                .read_game_file_bytes("g", relative)
                .map_err(|error| SourceError::Open(error.to_string()))?;
            if relative == "timeline.json" && !swapped {
                fs::rename(root.join("g"), root.join("game_object_a")).unwrap();
                write_playable_store(&root, "g", true, "OBJECT_B");
                swapped = true;
            }
            Ok(bytes)
        });

        let error = match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("timeline and referenced manifest must share one game parent object"),
        };
        assert!(
            error.contains("identity") || error.contains("namespace"),
            "{error}"
        );
        drop(store);
        let _ = fs::remove_dir_all(parent);
    }

    #[derive(Default)]
    struct FilesystemAliases {
        modules: Vec<String>,
        files: Vec<String>,
        open_options: Vec<String>,
    }

    fn collect_aliases(compact: &str, markers: &[&str]) -> Vec<String> {
        let mut aliases = Vec::new();
        for marker in markers {
            let mut cursor = 0;
            while let Some(relative) = compact[cursor..].find(marker) {
                let start = cursor + relative + marker.len();
                let len = compact[start..]
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                if len == 0 {
                    cursor = start;
                    continue;
                }
                let alias = &compact[start..start + len];
                if !aliases.iter().any(|existing| existing == alias) {
                    aliases.push(alias.to_string());
                }
                cursor = start + len;
            }
        }
        aliases
    }

    fn filesystem_aliases(source: &str) -> FilesystemAliases {
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        FilesystemAliases {
            modules: collect_aliases(
                &compact,
                &["usestd::fsas", "usestd::{fsas", "usestd::fs::{selfas"],
            ),
            files: collect_aliases(&compact, &["usestd::fs::Fileas", "usestd::fs::{Fileas"]),
            open_options: collect_aliases(
                &compact,
                &["usestd::fs::OpenOptionsas", "usestd::fs::{OpenOptionsas"],
            ),
        }
    }

    fn source_references_file_access(source: &str, aliases: &FilesystemAliases) -> bool {
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        [
            "std::fs",
            "std::{fs",
            "File::",
            "OpenOptions",
            ".open(",
            "windows_sys::Win32::Storage::FileSystem",
            "windows_sys::Wdk::Storage::FileSystem",
            "::Storage::FileSystem::",
            "CreateFile(",
            "CreateFile2",
            "CreateFileA",
            "CreateFileW",
            "NtCreateFile",
            "ZwCreateFile",
            "NtOpenFile",
            "ZwOpenFile",
            "OpenFile(",
            "OpenFileById",
            "ReOpenFile",
        ]
        .iter()
        .any(|token| compact.contains(token))
            || aliases
                .modules
                .iter()
                .any(|alias| compact.contains(&format!("{alias}::")))
            || aliases
                .files
                .iter()
                .any(|alias| compact.contains(&format!("{alias}::")))
            || aliases
                .open_options
                .iter()
                .any(|alias| compact.contains(alias))
    }

    fn pathbuf_value_names(source: &str) -> Vec<String> {
        let mut names = Vec::new();
        for line in source.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("let ") else {
                continue;
            };
            let rest = rest.strip_prefix("mut ").unwrap_or(rest);
            let Some((name, value)) = rest.split_once('=') else {
                continue;
            };
            let name = name.trim().split(':').next().unwrap_or("").trim();
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                && (value.contains("PathBuf::")
                    || value.contains(".join(")
                    || value.contains(".to_path_buf("))
                && !names.iter().any(|existing| existing == name)
            {
                names.push(name.to_string());
            }
        }
        names
    }

    fn file_capability_import_lines(source: &str) -> Vec<bool> {
        let mut import_lines = vec![false; source.lines().count()];
        let mut in_import = false;
        for (index, line) in source.lines().enumerate() {
            let compact = line
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            if !in_import
                && (compact.contains("std::fs::{")
                    || compact.contains("std::{fs::{")
                    || (compact.contains("windows_sys::")
                        && compact.contains("::Storage::FileSystem::{")))
            {
                in_import = true;
            }
            if in_import {
                import_lines[index] = true;
                if compact.contains("};") {
                    in_import = false;
                }
            }
        }
        import_lines
    }

    const WINDOWS_OPEN_PRIMITIVES: &[&str] = &[
        "CreateFile",
        "CreateFile2",
        "CreateFileA",
        "CreateFileW",
        "CreateFile2FromAppW",
        "CreateFileFromAppW",
        "CreateFileTransactedA",
        "CreateFileTransactedW",
        "NtCreateFile",
        "ZwCreateFile",
        "NtOpenFile",
        "ZwOpenFile",
        "OpenFile",
        "OpenFileById",
        "ReOpenFile",
    ];

    fn windows_open_primitive_symbol(symbol: &str) -> bool {
        WINDOWS_OPEN_PRIMITIVES.contains(&symbol)
    }

    fn windows_open_primitive_candidate(line: &str) -> bool {
        WINDOWS_OPEN_PRIMITIVES.iter().any(|primitive| {
            line.match_indices(primitive).any(|(start, matched)| {
                let end = start + matched.len();
                let identifier_boundary = |byte: u8| !byte.is_ascii_alphanumeric() && byte != b'_';
                let starts_at_boundary = start == 0
                    || line
                        .as_bytes()
                        .get(start - 1)
                        .copied()
                        .is_some_and(identifier_boundary);
                let ends_at_boundary = end == line.len()
                    || line
                        .as_bytes()
                        .get(end)
                        .copied()
                        .is_some_and(identifier_boundary);
                if !starts_at_boundary || !ends_at_boundary {
                    return false;
                }
                let suffix = line[end..].trim_start();
                suffix.is_empty()
                    || suffix.starts_with('(')
                    || suffix.starts_with(',')
                    || suffix.starts_with(';')
                    || suffix.starts_with(')')
                    || suffix.starts_with('}')
                    || suffix.starts_with("as ")
            })
        })
    }

    fn link_name_open_primitive_candidate(line: &str) -> bool {
        let compact = line
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        let quote = char::from(34);
        let prefix = format!("#[link_name={quote}");
        let Some(attribute) = compact
            .find(&prefix)
            .map(|start| &compact[start + prefix.len()..])
        else {
            return false;
        };
        let Some(end) = attribute.find('"') else {
            return false;
        };
        windows_open_primitive_symbol(&attribute[..end])
    }

    fn ffi_open_declaration_candidate(line: &str) -> bool {
        link_name_open_primitive_candidate(line)
            || (line.contains("extern") && windows_open_primitive_candidate(line))
    }

    fn loader_pathname_candidate(
        line: &str,
        aliases: &FilesystemAliases,
        file_access_capable: bool,
        pathbuf_values: &[String],
    ) -> bool {
        let direct_file_access = [
            "std::fs",
            "std::{fs",
            "fs::",
            "File::",
            "OpenOptions",
            ".open(",
            ".is_file(",
            ".is_dir(",
            ".try_exists(",
            "windows_sys::Win32::Storage::FileSystem",
            "windows_sys::Wdk::Storage::FileSystem",
            "::Storage::FileSystem::",
        ]
        .iter()
        .any(|token| line.contains(token));
        let path_construction = file_access_capable
            && (["std::path", "Path::", "PathBuf::", ".join("]
                .iter()
                .any(|token| line.contains(token))
                || pathbuf_values
                    .iter()
                    .any(|name| line.contains(&format!("{name}.push("))));
        direct_file_access
            || windows_open_primitive_candidate(line)
            || ffi_open_declaration_candidate(line)
            || path_construction
            || aliases
                .modules
                .iter()
                .any(|alias| line.contains(&format!("{alias}::")))
            || aliases
                .files
                .iter()
                .any(|alias| line.contains(&format!("{alias}::open")))
            || aliases
                .open_options
                .iter()
                .any(|alias| line.contains(alias))
    }

    fn discover_rust_sources(label: &str, root: &Path) -> Result<Vec<(String, String)>, String> {
        fn walk(
            label: &str,
            root: &Path,
            directory: &Path,
            sources: &mut Vec<(String, String)>,
        ) -> Result<(), String> {
            let mut entries = fs::read_dir(directory)
                .map_err(|error| format!("discover {}: {error}", directory.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("enumerate {}: {error}", directory.display()))?;
            entries.sort_by_key(|entry| entry.path());
            for entry in entries {
                let file_type = entry.file_type().map_err(|error| {
                    format!("classify discovered {}: {error}", entry.path().display())
                })?;
                if file_type.is_symlink() {
                    return Err(format!(
                        "source discovery refuses symlink {}",
                        entry.path().display()
                    ));
                }
                if file_type.is_dir() {
                    walk(label, root, &entry.path(), sources)?;
                    continue;
                }
                if !file_type.is_file() {
                    return Err(format!(
                        "source discovery found unsupported object {}",
                        entry.path().display()
                    ));
                }
                if entry.path().extension().and_then(|value| value.to_str()) != Some("rs") {
                    continue;
                }
                let path = entry.path();
                let relative = path.strip_prefix(root).map_err(|_| {
                    format!(
                        "discovered source {} escaped root {}",
                        path.display(),
                        root.display()
                    )
                })?;
                let relative = relative
                    .to_str()
                    .ok_or_else(|| format!("source path is not Unicode: {}", path.display()))?
                    .replace(char::from(92), "/");
                let source = fs::read_to_string(&path).map_err(|error| {
                    format!("read discovered source {}: {error}", path.display())
                })?;
                sources.push((format!("{label}/{relative}"), source));
            }
            Ok(())
        }

        let root = fs::canonicalize(root)
            .map_err(|error| format!("canonicalize source root {}: {error}", root.display()))?;
        if !root.is_dir() {
            return Err(format!(
                "source root is not a directory: {}",
                root.display()
            ));
        }
        let mut sources = Vec::new();
        walk(label, &root, &root, &mut sources)?;
        if sources.is_empty() {
            return Err(format!(
                "source discovery found no Rust files under {label}"
            ));
        }
        Ok(sources)
    }

    fn classified_root_or_handle_site(module: &str, line: &str) -> bool {
        matches!(
            (module, line),
            ("replay_core/source.rs", "games.join(\", \")")
                | (
                    "replay_core/source.rs",
                    "let store = if path.is_file() {"
                )
                | ("replay_core/source.rs", "} else if path.is_dir() {")
                | ("snapshot/lib.rs", "if path.is_file() {")
                | (
                    "snapshot/container.rs",
                    "let data = std::fs::read(path.as_ref())?;"
                )
                | ("snapshot/container.rs", "use std::path::Path;")
                // String-only source-path canonicalization (stitching alias
                // ledger): `components.join("/")` builds a normalized String for
                // comparison — it is never opened. Slice/String join, not
                // Path::join. (merge-wave: stitching + replay-loader F4 gate.)
                | (
                    "snapshot/container.rs",
                    "let joined = components.join(\"/\");"
                )
                | ("snapshot/dir_store/linux.rs", "use std::fs::File;")
                | ("snapshot/dir_store/linux.rs", "use std::path::Path;")
                | (
                    "snapshot/dir_store/linux.rs",
                    "let mut file = File::from(opened.fd);"
                )
                | ("snapshot/dir_store/windows.rs", "use std::fs::File;")
                | ("snapshot/dir_store/windows.rs", "use std::path::Path;")
                | (
                    "snapshot/dir_store/windows.rs",
                    "use windows_sys::Wdk::Storage::FileSystem::{"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "use windows_sys::Win32::Storage::FileSystem::{"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,"
                )
                | ("snapshot/dir_store/windows.rs", "};")
                | (
                    "snapshot/dir_store/windows.rs",
                    "CreateFileW, FileAttributeTagInfo, FileIdExtdDirectoryInfo, FileIdExtdDirectoryRestartInfo,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FileIdInfo, FileNormalizedNameInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_DIRECTORY,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_GENERIC_READ, FILE_ID_EXTD_DIR_INFO, FILE_ID_INFO, FILE_NAME_INFO, FILE_SHARE_DELETE,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,"
                )
                | ("snapshot/dir_store/windows.rs", "CreateFileW(")
                | (
                    "snapshot/dir_store/windows.rs",
                    "let mut file = File::from(opened.handle);"
                )
                | ("snapshot/dir_store/windows.rs", "NtCreateFile(")
                | ("snapshot/stream_verify.rs", "use std::fs::File;")
                | ("snapshot/stream_verify.rs", "use std::path::Path;")
                | (
                    "snapshot/stream_verify.rs",
                    "File::open(path).map_err(|e| format!(\"open container {}: {e}\", path.display()))?;"
                )
                // The file-backed container root open (2026-07-20): the one
                // sanctioned caller-supplied pathname the product reader opens.
                | ("snapshot/container.rs", "use std::fs::File;")
                | (
                    "snapshot/container.rs",
                    "let file = File::open(path.as_ref())?;"
                )
        )
    }

    fn classified_non_loader_site(module: &str, line: &str) -> bool {
        match module {
            "snapshot/bin/container_codec_bench.rs" => matches!(
                line,
                "use std::fs::File;"
                    | "use std::path::PathBuf;"
                    | ".map(PathBuf::from)"
                    | "let meta = std::fs::metadata(&path).map_err(|err| format!(\"stat {}: {err}\", path.display()))?;"
                    | "let file = File::open(path).map_err(|err| format!(\"open {}: {err}\", path.display()))?;"
            ),
            "snapshot/bin/pack_replay.rs" => matches!(
                line,
                "use std::fs;"
                    | "use std::path::{Path, PathBuf};"
                    | "(Some(a), Some(b), Some(c)) => (PathBuf::from(a), b.clone(), PathBuf::from(c)),"
                    | "let gdir = root.join(&game);"
                    | "let index_path = gdir.join(\"index.json\");"
                    | "let index_bytes = match fs::read(&index_path) {"
                    | "let p = gdir.join(file);"
                    | "let bytes = match fs::read(&p) {"
                    | "let tpath = gdir.join(\"timeline.json\");"
                    | "if let Ok(tb) = fs::read(&tpath) {"
                    | "if let Err(e) = fs::write(&tmp, &container) {"
                    | "if let Err(e) = fs::rename(&tmp, &out) {"
                    | "let gdir = Path::new(root).join(game);"
                    | "let orig = gdir.join(&s.file);"
                    | "let orig_bytes = match fs::read(&orig) {"
                    | "Some(p) => PathBuf::from(p),"
            ),
            "snapshot/pack.rs" => matches!(
                line,
                "use std::fs;"
                    | "use std::path::Path;"
                    | "return Ok(fs::metadata(entry.file_path)?.len());"
                    | "let mut file = fs::File::open(path)?;"
            ),
            "replay_core/bin/make_demo_recording.rs" => matches!(
                line,
                "use std::path::{Path, PathBuf};"
                    | "bundle_dir = Some(PathBuf::from(args.get(i).ok_or_else(|| USAGE.to_string())?));"
                    | "out_path = Some(PathBuf::from(args.get(i).ok_or_else(|| USAGE.to_string())?));"
                    | "std::fs::write(&out_path, packed).map_err(|e| format!(\"write {}: {e}\", out_path.display()))?;"
                    | "let path = bundle_dir.join(\"state_owner_1936.json\");"
                    | "std::fs::read_to_string(&path).map_err(|e| format!(\"read {}: {e}\", path.display()))?;"
                    | "let path = bundle_dir.join(\"province_to_state.json\");"
            ),
            "replay_core/bin/replay_dump.rs" => matches!(
                line,
                "coverage.domains.join(\", \")" | ".join(\",\")" | ".join(\", \")"
            ),
            "replay_core/world.rs" => matches!(
                line,
                "path[..=i].join(\".\"),"
                    | "path.join(\".\"),"
                    | "let ectx = format!(\"{ctx}.{}[{i}]\", path.join(\".\"));"
                    | "let rctx = format!(\"{ctx}.{}[{i}]\", path.join(\".\"));"
                    | "path.join(\".\")"
            ),
            _ => false,
        }
    }

    /// Exact production file-capability census for the replay-owned prefold
    /// spool. These are not caller-selected descendant reads: replay_core
    /// creates a private, identity-marked child beneath its product-owned
    /// parent, rejects links/replacements/extra entries before every read, and
    /// removes only that authenticated child. Keep this list exact so any new
    /// filesystem operation still fails the F4 gate pending review.
    fn classified_private_prefold_spool_site(module: &str, line: &str) -> bool {
        module == "replay_core/source.rs"
            && matches!(
                line,
                "use std::fs;"
                    | "use std::path::{Path, PathBuf};"
                    | "let raw_path = owner.root().join(\"source.h4cr\");"
                    | "let file = fs::OpenOptions::new()"
                    | ".open(&raw_path)"
                    | "let meta = fs::symlink_metadata(base).map_err(|error| {"
                    | "if !meta.file_type().is_dir() || meta.file_type().is_symlink() {"
                    | "let canonical_base = fs::canonicalize(base).map_err(|error| {"
                    | "let root = canonical_base.join(format!("
                    | "match fs::create_dir(&root) {"
                    | "let canonical_root = match fs::canonicalize(&root) {"
                    | "let _ = fs::remove_dir(&root);"
                    | "let _ = fs::remove_dir_all(&root);"
                    | "let marker_path = canonical_root.join(PREFOLD_SPOOL_MARKER);"
                    | "let mut marker = fs::OpenOptions::new()"
                    | ".open(&marker_path)"
                    | "let snapshot_dir = canonical_root.join(\"snapshots\");"
                    | "fs::create_dir(&snapshot_dir).map_err(|error| {"
                    | "if fs::canonicalize(&root).ok().as_deref()"
                    | "let Ok(meta) = fs::symlink_metadata(&self.root) else {"
                    | "if fs::canonicalize(&self.root).ok().as_deref() != Some(self.canonical_root.as_path()) {"
                    | "let Ok(snapshot_meta) = fs::symlink_metadata(&self.snapshot_dir) else {"
                    | "if !snapshot_meta.file_type().is_dir() || snapshot_meta.file_type().is_symlink() {"
                    | "if fs::canonicalize(&self.snapshot_dir).ok().as_deref() != Some(self.snapshot_dir.as_path())"
                    | "let marker = self.root.join(PREFOLD_SPOOL_MARKER);"
                    | "let Ok(marker_meta) = fs::symlink_metadata(&marker) else {"
                    | "if !marker_meta.file_type().is_file() || marker_meta.file_type().is_symlink() {"
                    | "if fs::read_to_string(marker).ok().as_deref() != Some(self.marker_value.as_str()) {"
                    | "let Ok(root_entries) = fs::read_dir(&self.root) else {"
                    | "let Ok(meta) = fs::symlink_metadata(entry.path()) else {"
                    | "if !meta.file_type().is_file() || meta.file_type().is_symlink() {"
                    | "let Ok(canonical) = fs::canonicalize(entry.path()) else {"
                    | "let Ok(snapshot_entries) = fs::read_dir(&self.snapshot_dir) else {"
                    | "let path = self.root.join(\"source.h4cr\");"
                    | "let meta = fs::symlink_metadata(&path).map_err(|error| {"
                    | "let canonical = fs::canonicalize(&path).map_err(|error| {"
                    | "if !meta.file_type().is_file()"
                    | "fs::remove_file(&path).map_err(|error| {"
                    | "let _ = fs::remove_dir_all(&self.root);"
                    | "if fs::canonicalize(snapshot_dir).ok().as_deref() != Some(owner.snapshot_dir()) {"
                    | "let path = snapshot_dir.join(entry.file_name());"
                    | "let path_meta = fs::symlink_metadata(&path).map_err(|error| {"
                    | "if !path_meta.file_type().is_file()"
                    | "let file = fs::File::open(&canonical).map_err(|error| {"
            )
    }

    fn cfg_test_item_end(source: &str, mut cursor: usize) -> Result<usize, String> {
        #[derive(Clone, Copy)]
        enum State {
            Code,
            LineComment,
            BlockComment(usize),
            String(bool),
            RawString(usize),
        }

        fn raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
            let mut cursor = match (bytes.get(index), bytes.get(index + 1)) {
                (Some(b'r'), _) => index + 1,
                (Some(b'b'), Some(b'r')) => index + 2,
                _ => return None,
            };
            let mut hashes = 0;
            while bytes.get(cursor) == Some(&b'#') {
                hashes += 1;
                cursor += 1;
            }
            (bytes.get(cursor) == Some(&b'"')).then_some((cursor + 1, hashes))
        }

        let bytes = source.as_bytes();
        let mut state = State::Code;
        let mut depth = 0usize;
        let mut opened = false;
        while cursor < bytes.len() {
            match state {
                State::Code => {
                    if bytes[cursor..].starts_with(b"//") {
                        state = State::LineComment;
                        cursor += 2;
                        continue;
                    }
                    if bytes[cursor..].starts_with(b"/*") {
                        state = State::BlockComment(1);
                        cursor += 2;
                        continue;
                    }
                    if let Some((next, hashes)) = raw_string_start(bytes, cursor) {
                        state = State::RawString(hashes);
                        cursor = next;
                        continue;
                    }
                    if bytes[cursor] == b'"' {
                        state = State::String(false);
                        cursor += 1;
                        continue;
                    }
                    if bytes[cursor] == b'\'' {
                        let mut end = cursor + 1;
                        if bytes.get(end) == Some(&b'\\') {
                            end += 2;
                        } else if let Some(character) = source[end..].chars().next() {
                            end += character.len_utf8();
                        }
                        if bytes.get(end) == Some(&b'\'') {
                            cursor = end + 1;
                            continue;
                        }
                    }
                    match bytes[cursor] {
                        b'{' => {
                            opened = true;
                            depth += 1;
                        }
                        b'}' if opened => {
                            depth = depth.checked_sub(1).ok_or_else(|| {
                                "cfg(test) item brace depth underflow".to_string()
                            })?;
                            if depth == 0 {
                                return Ok(cursor + 1);
                            }
                        }
                        b';' if !opened => return Ok(cursor + 1),
                        _ => {}
                    }
                    cursor += 1;
                }
                State::LineComment => {
                    if bytes[cursor] == b'\n' {
                        state = State::Code;
                    }
                    cursor += 1;
                }
                State::BlockComment(comment_depth) => {
                    if bytes[cursor..].starts_with(b"/*") {
                        state = State::BlockComment(comment_depth + 1);
                        cursor += 2;
                    } else if bytes[cursor..].starts_with(b"*/") {
                        if comment_depth == 1 {
                            state = State::Code;
                        } else {
                            state = State::BlockComment(comment_depth - 1);
                        }
                        cursor += 2;
                    } else {
                        cursor += 1;
                    }
                }
                State::String(escaped) => {
                    if escaped {
                        state = State::String(false);
                    } else if bytes[cursor] == b'\\' {
                        state = State::String(true);
                    } else if bytes[cursor] == b'"' {
                        state = State::Code;
                    }
                    cursor += 1;
                }
                State::RawString(hashes) => {
                    if bytes[cursor] == b'"'
                        && bytes
                            .get(cursor + 1..cursor + 1 + hashes)
                            .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
                    {
                        state = State::Code;
                        cursor += 1 + hashes;
                    } else {
                        cursor += 1;
                    }
                }
            }
        }
        Err("cfg(test) item did not have a complete body".into())
    }

    fn mask_cfg_test_items(source: &str) -> Result<String, String> {
        const ATTRIBUTE: &str = "#[cfg(test)]";
        let mut masked = source.as_bytes().to_vec();
        let mut cursor = 0;
        while let Some(relative) = source[cursor..].find(ATTRIBUTE) {
            let start = cursor + relative;
            let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
            let line_end = source[start..]
                .find('\n')
                .map_or(source.len(), |index| start + index);
            if source[line_start..line_end].trim() != ATTRIBUTE {
                cursor = start + ATTRIBUTE.len();
                continue;
            }
            let end = cfg_test_item_end(source, start + ATTRIBUTE.len())?;
            for byte in &mut masked[start..end] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
            cursor = end;
        }
        String::from_utf8(masked).map_err(|_| "masked Rust source stopped being UTF-8".into())
    }

    /// F4 dossier/dispatch scope ruling (2026-07-14): this is a
    /// development-time, best-effort honest-mistake lint, not a runtime
    /// security boundary. It cheaply recognizes direct APIs and explicit
    /// link_name FFI declarations. Renamed FFI beyond that check,
    /// macro/proc-macro expansion, and generated/include! code remain known,
    /// ACCEPTED blind spots; the runtime boundary is the handle-relative
    /// loader implementation and its identity/namespace checks.
    fn loader_pathname_audit(module: &str, source: &str) -> Result<(usize, Vec<String>), String> {
        let source = mask_cfg_test_items(source)?;
        let aliases = filesystem_aliases(&source);
        let file_access_capable = source_references_file_access(&source, &aliases);
        let pathbuf_values = pathbuf_value_names(&source);
        let import_lines = file_capability_import_lines(&source);
        let mut classified = 0;
        let mut candidates = 0;
        let mut violations = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let line = line.trim();
            if line.starts_with("//")
                || (!import_lines[index]
                    && !loader_pathname_candidate(
                        line,
                        &aliases,
                        file_access_capable,
                        &pathbuf_values,
                    ))
            {
                continue;
            }
            candidates += 1;
            if classified_root_or_handle_site(module, line)
                || classified_non_loader_site(module, line)
                || classified_private_prefold_spool_site(module, line)
            {
                classified += 1;
            } else {
                violations.push(format!("{module}:{}: {line}", index + 1));
            }
        }
        if file_access_capable && candidates == 0 {
            violations.push(format!(
                "{module}: file-access capability yielded zero recognized source sites"
            ));
        }
        Ok((classified, violations))
    }

    fn loader_pathname_violations(module: &str, source: &str) -> Vec<String> {
        loader_pathname_audit(module, source)
            .expect("pathname audit source must be structurally unambiguous")
            .1
    }

    #[test]
    fn f4_gate_catches_planted_file_and_open_options_descendant_opens() {
        let planted = r#"
use std::fs::{File, OpenOptions};
use std::{fs as aliased_fs, path::Path};
use std::fs as disk;
fn bad(root: &std::path::Path) {
    let _ = File::open(root.join("timeline.json"));
    let _ = OpenOptions::new().read(true).open(root.join("index.json"));
    let _ = aliased_fs::read(root.join("snapshot.json"));
    let _ = disk::read(root.join("aliased.json"));
}
"#;
        let violations = loader_pathname_violations("snapshot/src/container.rs", planted);
        assert!(
            violations.iter().any(|site| site.contains("File::open")),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("OpenOptions")),
            "{violations:?}"
        );
        assert!(
            violations
                .iter()
                .any(|site| site.contains("aliased_fs::read")),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("disk::read")),
            "{violations:?}"
        );
    }

    #[test]
    fn f4_gate_catches_create_file2_and_pathbuf_push_open_sites() {
        for primitive in [
            "CreateFile2",
            "CreateFileA",
            "CreateFileW",
            "NtCreateFile",
            "ZwCreateFile",
            "NtOpenFile",
            "ZwOpenFile",
            "OpenFileById",
            "ReOpenFile",
        ] {
            let planted = format!(
                "use windows_sys::Win32::Storage::FileSystem::{primitive};\n\
                 fn bad() {{ let _ = {primitive}(0); }}\n"
            );
            let violations =
                loader_pathname_violations(&format!("fixture/{primitive}.rs"), &planted);
            assert!(
                violations.iter().any(|site| site.contains(primitive)),
                "{primitive} must never be an invisible zero-site module: {violations:?}"
            );
        }

        let split_primitive = r#"
use windows_sys::Win32::Storage::FileSystem::CreateFileW;
fn bad() {
    let _ = CreateFileW
        (0);
}
"#;
        let violations =
            loader_pathname_violations("fixture/split_create_file.rs", split_primitive);
        assert!(
            violations
                .iter()
                .any(|site| site.contains("let _ = CreateFileW")),
            "a whitespace-split primitive name must still be classified: {violations:?}"
        );

        let future_windows_api = r#"
use windows_sys::Win32::Storage::FileSystem::{
    FutureFileOpenPrimitive,
};
"#;
        let violations =
            loader_pathname_violations("fixture/future_windows_api.rs", future_windows_api);
        assert!(
            violations
                .iter()
                .any(|site| site.contains("FutureFileOpenPrimitive")),
            "an unknown windows-sys FileSystem capability must fail closed: {violations:?}"
        );

        let pathbuf_push_open = r#"
use std::fs::File;
use std::path::PathBuf;
fn bad(root: &std::path::Path) {
    let mut child = PathBuf::from(root);
    child.push("descendant.json");
    let _ = File::open(child);
}
"#;
        let violations =
            loader_pathname_violations("fixture/pathbuf_push_open.rs", pathbuf_push_open);
        assert!(
            violations.iter().any(|site| site.contains(".push(")),
            "PathBuf::push in a file-open-capable module must be classified: {violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("File::open")),
            "the paired file open must be classified: {violations:?}"
        );
    }

    #[test]
    fn f4_gate_recognizes_link_name_file_open_ffi_declaration() {
        let planted = r#"
#[link(name = "kernel32")]
extern "system" {
    #[link_name = "CreateFileW"]
    fn renamed_open();
}
"#;
        let violations = loader_pathname_violations("fixture/ffi_alias.rs", planted);
        assert!(
            violations.iter().any(|site| site.contains("#[link_name")),
            "the FFI capability declaration itself must be a recognized site: {violations:?}"
        );
    }

    #[test]
    fn f4_gate_discovers_new_module_and_scans_production_after_test_item() {
        let root = temp_root("f4_closed_world");
        fs::write(root.join("existing.rs"), "fn existing() {}\n").unwrap();
        fs::write(
            root.join("newly_added_loader_helper.rs"),
            r#"
use std::fs::File;
fn bad(root: &std::path::Path) {
    let _ = File::open(root.join("new-module.json"));
}
"#,
        )
        .unwrap();
        let discovered = discover_rust_sources("fixture", &root).unwrap();
        let (_, new_module) = discovered
            .iter()
            .find(|(module, _)| module == "fixture/newly_added_loader_helper.rs")
            .expect("new module is discovered without a hand-maintained module list");
        let new_module_violations =
            loader_pathname_violations("fixture/newly_added_loader_helper.rs", new_module);
        assert!(
            new_module_violations
                .iter()
                .any(|site| site.contains("File::open")),
            "{new_module_violations:?}"
        );

        let after_test = r#"
#[cfg(test)]
mod tests {
    fn test_only(root: &std::path::Path) {
        let _ = std::fs::File::open(root.join("test-only.json"));
    }
}

fn production(root: &std::path::Path) {
    let _ = std::fs::File::open(root.join("after-test.json"));
}
"#;
        let after_test_violations = loader_pathname_violations("fixture/after_test.rs", after_test);
        assert!(
            after_test_violations
                .iter()
                .any(|site| site.contains("after-test.json")),
            "{after_test_violations:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_sources_have_no_descendant_pathname_read() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut modules = discover_rust_sources("replay_core", &manifest_dir.join("src"))
            .expect("discover every replay_core production source");
        // The REAL container/loader implementation replay_core links lives in
        // portable_replay/native/snapshot (Cargo.toml: snapshot = portable);
        // the extractor-side crate is only a re-export facade + bin tools, but
        // is kept scanned so its bins stay covered too.
        modules.extend(
            discover_rust_sources(
                "snapshot",
                &manifest_dir.join("../../../portable_replay/native/snapshot/src"),
            )
            .expect("discover every portable snapshot production source"),
        );
        modules.extend(
            discover_rust_sources(
                "snapshot",
                &manifest_dir.join("../../../extractor/native/snapshot/src"),
            )
            .expect("discover every snapshot facade production source"),
        );
        let discovered = modules.len();
        let mut classified = 0;
        let mut violations = Vec::new();
        for (module, source) in modules {
            let (module_classified, mut module_violations) =
                loader_pathname_audit(&module, &source).unwrap_or_else(|error| {
                    panic!("pathname audit could not classify {module}: {error}")
                });
            classified += module_classified;
            violations.append(&mut module_violations);
        }
        assert!(
            violations.is_empty(),
            "unclassified or unrecognized file-access sites across {discovered} discovered modules ({classified} classified): {violations:#?}"
        );
    }
}
