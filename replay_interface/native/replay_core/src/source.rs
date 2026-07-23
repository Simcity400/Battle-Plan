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
/// Shipped-player compact expansion ceiling. Two GiB is intentionally aligned
/// with the format's ordinary bounded-entry tier and is small enough for weak
/// offline PCs to reject pathological recordings before reserving product
/// spool storage. The streaming writer enforces this same limit again.
pub const PRODUCT_COMPACT_STREAM_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn admit_product_compact_stream_bytes(bytes: u64) -> Result<(), SourceError> {
    if bytes > PRODUCT_COMPACT_STREAM_MAX_BYTES {
        return Err(SourceError::Unsupported(format!(
            "compact replay expands to {bytes} bytes; shipped playback limit is {} bytes (2 GiB)",
            PRODUCT_COMPACT_STREAM_MAX_BYTES
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecorderSessionBinding {
    active_dlcs: Vec<String>,
    localisation_language: String,
    display_variant: RecorderDisplayVariant,
    campaign_identity: Option<RecordedCampaignIdentity>,
}

/// Shape-validated recorder-manifest campaign metadata retained by replay
/// ingest. This is not proof that `stream_header_sha256` matches CID1 bytes;
/// only the recorder's placement-authoritative stream path can prove that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCampaignIdentity {
    pub state: String,
    pub campaign_id: Option<String>,
    pub game_unique_id: Option<String>,
    pub game_unique_seed: Option<String>,
    pub source: Option<String>,
    pub source_build_profile: Option<String>,
    pub observed_game_hour_raw: Option<i32>,
    pub stream_header_sha256: Option<String>,
    pub resolution_note: Option<String>,
    pub witnesses: RecordedCampaignIdentityWitnesses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCampaignIdentityWitnesses {
    pub start_date_raw: Option<u32>,
    pub session: Option<u32>,
    pub scenario: Option<String>,
    pub initial_human_country_tags: Option<Vec<String>>,
    pub modset_identity: Option<String>,
}

/// Recorder-packaged effective decision catalog admitted by an exact
/// recorder-manifest -> replay-index -> container-manifest -> payload join.
///
/// The bytes are immutable and pointer-free. Replay consumers never reopen a
/// developer output path or an installed game tree to obtain this artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedDecisionCatalog {
    path: String,
    sha256: String,
    content_manifest_digest_sha256: String,
    bytes: Arc<[u8]>,
}

impl PackagedDecisionCatalog {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

/// One recorder-owned static catalog after the complete
/// recorder→index→container-role→content-epoch→bytes join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedStaticArtifact {
    name: String,
    path: String,
    sha256: String,
    content_manifest_digest_sha256: String,
    bytes: Arc<[u8]>,
}

impl PackagedStaticArtifact {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn content_manifest_digest_sha256(&self) -> &str {
        &self.content_manifest_digest_sha256
    }
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
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
    recorder_manifest: Option<Map<String, Value>>,
    g2_engine_capture: Option<crate::g2_engine_capture::PackagedG2EngineCapture>,
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
    /// Internally consistent campaign metadata from the recorder manifest.
    /// This never authorizes stitching without the recorder's
    /// placement-authenticated CID1 path and exact retained payload bytes.
    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        None
    }
    /// Strictly admitted recorder-owned post-process-end engine capture, when the
    /// selected game index declares the dedicated two-entry G2 package.
    fn g2_engine_capture(&self) -> Option<&crate::g2_engine_capture::PackagedG2EngineCapture> {
        None
    }
    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        None
    }
    /// Exact effective decision catalog authenticated through the portable
    /// package's recorder, index, content-manifest, and entry hashes.
    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        None
    }
    fn static_artifact(&self, _name: &str) -> Option<&PackagedStaticArtifact> {
        None
    }
    /// Exact source-byte and decoder identity for an explicitly prefolded
    /// compact-stream container. Snapshot-native and trusted direct-prefold
    /// sources return `None`.
    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        None
    }
    /// Date metadata for one canonical hour, without decoding its frame body.
    ///
    /// Production sources answer this from their authenticated index or
    /// prefold manifest. Backends that do not carry indexed dates return
    /// `None`; callers must not force a frame load merely to populate
    /// timeline navigation metadata.
    fn date_at(&self, _hour_index: i64) -> Result<Option<String>, SourceError> {
        Ok(None)
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

/// Consume an already-open pinned container through the bounded decoder seam.
/// No pathname is reopened: the resulting store and lazy source retain the
/// exact container handle authenticated by the caller.
pub fn open_container_with_spooled_decoder(
    container: snapshot::Container,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_store_impl(
        Store::from_container(container),
        "authenticated retained replay container",
        None,
        None,
        Some((spool_base, decoder)),
    )
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
        decision_catalog: None,
        static_artifacts: BTreeMap::new(),
        prefold_receipt: None,
        g2_engine_capture: None,
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

    open_store_impl(
        store,
        &path.display().to_string(),
        requested_game,
        decoder,
        spool_decoder,
    )
}

fn open_store_impl(
    store: Store,
    source_label: &str,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
) -> Result<Box<dyn ReplaySource>, SourceError> {

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
                    "no game found (no <game_id>/index.json) under {source_label}"
                )))
            }
            1 => games[0].clone(),
            _ => return Err(SourceError::MultipleGames(games)),
        },
    };

    // Route recorder metadata before consulting snapshot availability. An
    // explicit or inconsistent metadata signal can never be downgraded to
    // legacy/unsupported because an index or role entry is missing.
    let mut metadata = recorder_metadata_route(&store, &game_id)?;
    metadata.g2_engine_capture = crate::g2_engine_capture::admit_from_store(
        &store,
        &game_id,
        metadata.recorder_manifest.as_ref(),
    )
    .map_err(|error| SourceError::Open(format!("admit packaged G2 engine capture: {error}")))?;

    let index = store
        .index(&game_id)
        .map_err(|e| SourceError::Open(format!("read index for game {game_id:?}: {e}")))?;
    validate_recorder_portable_container_contract(&store, &game_id, &index, &metadata)?;

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
    let mut dates_by_hour: BTreeMap<i64, Option<String>> = BTreeMap::new();
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
                    dates_by_hour.insert(hour, entry.date.clone());
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
    let static_artifacts = validate_packaged_static_artifact_roster(
        &store,
        &game_id,
        metadata.recorder_manifest.as_ref(),
    )?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        &hours,
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    Ok(Box::new(StoreSource {
        store,
        game_id,
        hours,
        files_by_hour,
        dates_by_hour,
        warnings,
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        g2_engine_capture: metadata.g2_engine_capture,
    }))
}

fn open_compact_stream_with_decoder(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    metadata: MetadataRoute,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let identity_mode = prefold_stream_identity_mode(&metadata);
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
    validate_recorder_compact_stream_join(
        &entry.path,
        entry.bytes,
        &entry.sha256,
        &game_id,
        metadata.recorder_manifest.as_ref(),
    )?;
    let raw_h4cr = container.read_entry_bytes(&entry.path).map_err(|error| {
        SourceError::Open(format!(
            "read verified compact stream role entry {:?}: {error}",
            entry.path
        ))
    })?;
    let request = snapshot::CertifiedPrefoldRequest::try_new_with_identity_mode(
        game_id.clone(),
        raw_h4cr,
        identity_mode,
    )
    .map_err(|error| {
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

    let static_artifacts = validate_packaged_static_artifact_roster(
        &store,
        &game_id,
        metadata.recorder_manifest.as_ref(),
    )?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        certified.replay().index().hours(),
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    let prefold_receipt = Some(certified.receipt().clone());
    Ok(Box::new(PrefoldedSource {
        replay: certified.into_replay(),
        warnings: Vec::new(),
        store: Some(store),
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        prefold_receipt,
        g2_engine_capture: metadata.g2_engine_capture,
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
    let identity_mode = prefold_stream_identity_mode(&metadata);
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
        validate_recorder_compact_stream_join(
            &entry.path,
            entry.bytes,
            &entry.sha256,
            &game_id,
            metadata.recorder_manifest.as_ref(),
        )?;
        admit_product_compact_stream_bytes(entry.bytes)?;
        let receipt = container
            .spool_entry_to_file_transactionally_bounded(
                &entry.path,
                &raw_path,
                PRODUCT_COMPACT_STREAM_MAX_BYTES,
            )
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
        receipt
    };

    let request = snapshot::CertifiedPrefoldSpoolRequest::try_new_with_identity_mode(
        game_id.clone(),
        raw_path,
        snapshot_dir.clone(),
        source_receipt.uncompressed_len(),
        source_receipt.sha256(),
        identity_mode,
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

    let spool_hours = spool
        .entries()
        .iter()
        .map(|entry| entry.hour_index())
        .collect::<Vec<_>>();
    let static_artifacts = validate_packaged_static_artifact_roster(
        &store,
        &game_id,
        metadata.recorder_manifest.as_ref(),
    )?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        &spool_hours,
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    Ok(Box::new(SpooledPrefoldedSource {
        spool,
        snapshot_dir: certified_snapshot_dir,
        owner,
        warnings: Vec::new(),
        store,
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        prefold_receipt,
        g2_engine_capture: metadata.g2_engine_capture,
    }))
}

fn prefold_stream_identity_mode(metadata: &MetadataRoute) -> snapshot::PrefoldStreamIdentityMode {
    prefold_stream_identity_mode_from_campaign(
        metadata
            .binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref()),
    )
}

fn prefold_stream_identity_mode_from_campaign(
    identity: Option<&RecordedCampaignIdentity>,
) -> snapshot::PrefoldStreamIdentityMode {
    match identity.map(|identity| identity.state.as_str()) {
        Some("known") => snapshot::PrefoldStreamIdentityMode::IdentityV1,
        _ => snapshot::PrefoldStreamIdentityMode::Legacy,
    }
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
    hours: &[i64],
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<Option<Arc<crate::design_catalog::PortableDesignCatalog>>, SourceError> {
    let recorder_row = recorder_manifest
        .and_then(|manifest| manifest.get("replay_packaging"))
        .and_then(|packaging| packaging.get("design_catalog_live"))
        .filter(|value| !value.is_null());

    // Legacy stores without a recorder binding keep the historical optional
    // path. A recorder-owned package must join recorder seal, selected index,
    // dedicated role, content-manifest epoch, and the verified payload bytes.
    if recorder_manifest.is_some() {
        let index_bytes = store
            .read_game_file_bytes(game_id, "index.json")
            .map_err(|error| {
                SourceError::Open(format!(
                    "read authenticated index for A27 design-catalog join: {error}"
                ))
            })?
            .ok_or_else(|| {
                SourceError::Open(
                    "selected game index disappeared during A27 design-catalog join".into(),
                )
            })?;
        let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
            SourceError::Open(format!(
                "strict parse selected game index for A27 design-catalog join: {error}"
            ))
        })?;
        let index_row = index
            .get("design_catalog_live")
            .filter(|value| value.get("present").and_then(Value::as_bool) == Some(true));
        match (recorder_row, index_row) {
            (None, None) => {
                if store
                    .container()
                    .is_some_and(|container| {
                        !container.entries_by_role("design_catalog_live").is_empty()
                    })
                {
                    return Err(SourceError::Open(
                        "A27 design-catalog role exists without recorder/index authority".into(),
                    ));
                }
                return Ok(None);
            }
            (None, Some(_)) => {
                return Err(SourceError::Open(
                    "index records A27 design catalog present but recorder manifest omits it"
                        .into(),
                ))
            }
            (Some(_), None) => {
                return Err(SourceError::Open(
                    "recorder manifest records A27 design catalog but index omits it".into(),
                ))
            }
            (Some(recorder_row), Some(index_row)) => {
                let expected_path = crate::design_catalog::PORTABLE_ARTIFACT_PATH;
                let recorder_path = recorder_row.get("path").and_then(Value::as_str);
                let recorder_sha = recorder_row.get("sha256").and_then(Value::as_str);
                let recorder_len = recorder_row.get("bytes").and_then(Value::as_u64);
                let recorder_content_digest = recorder_row
                    .get("content_manifest_digest_sha256")
                    .and_then(Value::as_str);
                if recorder_path != Some(expected_path)
                    || recorder_sha.is_none_or(|value| !lower_sha256(value))
                    || recorder_len.is_none()
                    || recorder_content_digest.is_none_or(|value| !lower_sha256(value))
                {
                    return Err(SourceError::Open(
                        "recorder manifest A27 design-catalog identity is invalid".into(),
                    ));
                }
                let recorder_sha = recorder_sha.expect("validated A27 sha");
                let recorder_len = recorder_len.expect("validated A27 byte length");
                let recorder_content_digest =
                    recorder_content_digest.expect("validated A27 content digest");
                if index_row.get("role").and_then(Value::as_str)
                    != Some("design_catalog_live")
                    || index_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || index_row.get("sha256").and_then(Value::as_str) != Some(recorder_sha)
                    || index_row.get("bytes").and_then(Value::as_u64) != Some(recorder_len)
                    || index_row.get("source_sha256").and_then(Value::as_str)
                        != recorder_row.get("source_sha256").and_then(Value::as_str)
                {
                    return Err(SourceError::Open(
                        "index A27 design-catalog identity disagrees with recorder manifest"
                            .into(),
                    ));
                }

                let container = store.container().ok_or_else(|| {
                    SourceError::Open(
                        "A27 design catalog requires a portable container authority".into(),
                    )
                })?;
                let content_digest = container
                    .content_manifest()
                    .and_then(|manifest| manifest.manifest_digest_sha256.as_deref())
                    .ok_or_else(|| {
                        SourceError::Open(
                            "A27 design catalog has no finalized portable content-manifest epoch"
                                .into(),
                        )
                    })?;
                if content_digest != recorder_content_digest {
                    return Err(SourceError::Open(
                        "A27 design catalog is not bound to the selected content manifest".into(),
                    ));
                }
                let container_path = format!("payload/{game_id}/{expected_path}");
                let entries = container
                    .entries_by_role("design_catalog_live")
                    .into_iter()
                    .filter(|entry| entry.path == container_path)
                    .collect::<Vec<_>>();
                let entry = match entries.as_slice() {
                    [entry] => entry,
                    [] => {
                        return Err(SourceError::Open(
                            "A27 design-catalog index row has no matching dedicated role".into(),
                        ))
                    }
                    _ => {
                        return Err(SourceError::Open(
                            "A27 design catalog has duplicate dedicated role entries".into(),
                        ))
                    }
                };
                if entry.sha256 != recorder_sha || entry.bytes != recorder_len {
                    return Err(SourceError::Open(
                        "A27 design-catalog role entry disagrees with recorder/index hashes"
                            .into(),
                    ));
                }
            }
        }
    }

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
            if let Some(recorder_row) = recorder_row {
                let expected_sha = recorder_row
                    .get("sha256")
                    .and_then(Value::as_str)
                    .expect("recorder-owned A27 identity validated above");
                let expected_len = recorder_row
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .expect("recorder-owned A27 identity validated above");
                let actual_sha =
                    snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&bytes));
                if bytes.len() as u64 != expected_len || actual_sha != expected_sha {
                    return Err(SourceError::Open(
                        "verified A27 design-catalog bytes disagree with package authorities"
                            .into(),
                    ));
                }
            }
            crate::design_catalog::PortableDesignCatalog::decode_a27_live_json(&bytes)
                .and_then(|catalog| {
                    catalog.validate_replay_hours(hours)?;
                    Ok(catalog)
                })
                .map(Arc::new)
                .map_err(|error| {
                    SourceError::Open(format!("invalid A27 design catalog artifact: {error}"))
                })
        })
        .transpose()
}

const REQUIRED_PACKAGED_STATIC_ARTIFACTS: &[(&str, &str)] = &[
    (
        "templates_equipment_catalog",
        "static/templates_equipment_catalog/domain59_static_catalog.json",
    ),
    (
        "localisation_join",
        "static/localisation_join/localisation_catalog.json",
    ),
    (
        "decisions_static_definitions",
        "static/decisions_static_definitions/decision_definitions_catalog.json",
    ),
    (
        "idea_tech_loc_xref_static",
        "static/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
    ),
    (
        "ship_definitions_static",
        "static/ship_definitions_static/ship_definitions_catalog.json",
    ),
    (
        "gfx_icon_asset_manifest_static",
        "static/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json",
    ),
    (
        "technology_sharing_static",
        "static/technology_sharing_static/technology_sharing_static.json",
    ),
    (
        "focus_defs_static",
        "static/focus_defs_static/focus_defs_static.json",
    ),
];
const LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT: usize =
    REQUIRED_PACKAGED_STATIC_ARTIFACTS.len() - 1;

/// Consume the complete recorder-owned static-artifact roster at replay
/// admission.  Semantic adapters may select individual catalogs later, but no
/// producer binding is allowed to sit in the portable file unchecked.
fn validate_packaged_static_artifact_roster(
    store: &Store,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<BTreeMap<String, PackagedStaticArtifact>, SourceError> {
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(BTreeMap::new());
    };
    let recorder_rows = recorder_manifest
        .get("replay_packaging")
        .and_then(|packaging| packaging.get("static_artifacts"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open(
                "recorder-owned replay has no static-artifact packaging roster".into(),
            )
        })?;
    let index_bytes = store
        .read_game_file_bytes(game_id, "index.json")
        .map_err(|error| {
            SourceError::Open(format!(
                "read authenticated index for static-artifact roster join: {error}"
            ))
        })?
        .ok_or_else(|| {
            SourceError::Open(
                "selected game index disappeared during static-artifact roster join".into(),
            )
        })?;
    let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse selected game index for static-artifact roster join: {error}"
        ))
    })?;
    let index_rows = index
        .get("static_artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open("recorder-owned replay index has no static-artifact roster".into())
        })?;
    let legacy_without_focus_defs =
        index_rows.len() == LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT;
    if !legacy_without_focus_defs
        && index_rows.len() != REQUIRED_PACKAGED_STATIC_ARTIFACTS.len()
    {
        return Err(SourceError::Open(format!(
            "static-artifact index roster has {} rows; expected current {} or legacy {}",
            index_rows.len(),
            REQUIRED_PACKAGED_STATIC_ARTIFACTS.len(),
            LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT,
        )));
    }
    let container = store.container().ok_or_else(|| {
        SourceError::Open("static-artifact roster requires a portable container authority".into())
    })?;
    let content_digest = container
        .content_manifest()
        .and_then(|manifest| manifest.manifest_digest_sha256.as_deref())
        .ok_or_else(|| {
            SourceError::Open(
                "static-artifact roster has no finalized portable content-manifest epoch".into(),
            )
        })?;

    let mut expected_container_paths = std::collections::BTreeSet::new();
    let mut admitted = BTreeMap::new();
    for &(name, expected_path) in REQUIRED_PACKAGED_STATIC_ARTIFACTS {
        let recorder_matches = recorder_rows
            .iter()
            .filter(|row| row.get("name").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>();
        if recorder_matches.len() > 1 {
            return Err(SourceError::Open(format!(
                "recorder manifest declares static artifact {name:?} more than once"
            )));
        }
        let index_matches = index_rows
            .iter()
            .filter(|row| row.get("name").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>();
        if legacy_without_focus_defs
            && name == "focus_defs_static"
            && recorder_matches.is_empty()
            && index_matches.is_empty()
        {
            // A pre-focus-carrier recording remains readable but exposes no
            // focus definitions. A31 stays unavailable; there is no local VFS
            // or playback-machine fallback.
            continue;
        }
        if index_matches.len() != 1 {
            return Err(SourceError::Open(format!(
                "index must declare static artifact {name:?} exactly once"
            )));
        }
        let index_row = index_matches[0];
        let present = index_row.get("present").and_then(Value::as_bool);
        match (present, recorder_matches.as_slice()) {
            (Some(false), []) => continue,
            (Some(false), [_]) => {
                return Err(SourceError::Open(format!(
                    "recorder manifest records {name:?} present while index records it absent"
                )))
            }
            (Some(true), [recorder_row]) => {
                let sha = recorder_row.get("sha256").and_then(Value::as_str);
                let bytes = recorder_row.get("bytes").and_then(Value::as_u64);
                let epoch = recorder_row
                    .get("content_manifest_digest_sha256")
                    .and_then(Value::as_str);
                if recorder_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || sha.is_none_or(|value| !lower_sha256(value))
                    || bytes.is_none()
                    || epoch != Some(content_digest)
                {
                    return Err(SourceError::Open(format!(
                        "recorder manifest static artifact {name:?} identity is invalid"
                    )));
                }
                let sha = sha.expect("validated static-artifact sha");
                let bytes = bytes.expect("validated static-artifact length");
                if index_row.get("role").and_then(Value::as_str) != Some("static_artifact")
                    || index_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || index_row.get("sha256").and_then(Value::as_str) != Some(sha)
                    || index_row.get("bytes").and_then(Value::as_u64) != Some(bytes)
                {
                    return Err(SourceError::Open(format!(
                        "index static artifact {name:?} disagrees with recorder manifest"
                    )));
                }
                let container_path = format!("payload/{game_id}/{expected_path}");
                let entries = container
                    .entries_by_role("static_artifact")
                    .into_iter()
                    .filter(|entry| entry.path == container_path)
                    .collect::<Vec<_>>();
                let entry = match entries.as_slice() {
                    [entry] => entry,
                    [] => {
                        return Err(SourceError::Open(format!(
                            "static artifact {name:?} has no matching container role"
                        )))
                    }
                    _ => {
                        return Err(SourceError::Open(format!(
                            "static artifact {name:?} has duplicate container roles"
                        )))
                    }
                };
                if entry.sha256 != sha || entry.bytes != bytes {
                    return Err(SourceError::Open(format!(
                        "container static artifact {name:?} disagrees with recorder/index hashes"
                    )));
                }
                let payload = store
                    .read_game_file_bytes(game_id, expected_path)
                    .map_err(|error| {
                        SourceError::Open(format!(
                            "read verified static artifact {name:?}: {error}"
                        ))
                    })?
                    .ok_or_else(|| {
                        SourceError::Open(format!(
                            "verified static artifact {name:?} payload is absent"
                        ))
                    })?;
                let actual_sha =
                    snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&payload));
                if payload.len() as u64 != bytes || actual_sha != sha {
                    return Err(SourceError::Open(format!(
                        "verified static artifact {name:?} bytes disagree with package authorities"
                    )));
                }
                admitted.insert(
                    name.to_string(),
                    PackagedStaticArtifact {
                        name: name.to_string(),
                        path: expected_path.to_string(),
                        sha256: actual_sha,
                        content_manifest_digest_sha256: content_digest.to_string(),
                        bytes: Arc::from(payload),
                    },
                );
                expected_container_paths.insert(container_path);
            }
            (Some(true), []) => {
                return Err(SourceError::Open(format!(
                    "index records static artifact {name:?} present but recorder manifest omits it"
                )))
            }
            _ => {
                return Err(SourceError::Open(format!(
                    "index static artifact {name:?} has no boolean present disposition"
                )))
            }
        }
    }
    let actual_container_paths = container
        .entries_by_role("static_artifact")
        .into_iter()
        .map(|entry| entry.path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if actual_container_paths != expected_container_paths {
        return Err(SourceError::Open(
            "container static-artifact roles do not exactly match the recorder/index roster"
                .into(),
        ));
    }
    Ok(admitted)
}

const DECISION_CATALOG_NAME: &str = "decisions_static_definitions";
const DECISION_CATALOG_PATH: &str =
    "static/decisions_static_definitions/decision_definitions_catalog.json";

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Admit A34's immutable effective decision catalog without any machine-local
/// fallback. A usable artifact must be named identically by all four package
/// authorities:
///
/// 1. recorder_manifest.replay_packaging.static_artifacts;
/// 2. the selected game's authenticated index static-artifact roster;
/// 3. the portable content-manifest digest sealed into the recorder row; and
/// 4. the container manifest entry whose verified bytes are exposed by Store.
///
/// Legacy packages that predate the content-manifest digest field leave A34
/// explicitly unavailable. Contradictory declarations are corruption and
/// reject the open.
fn read_packaged_decision_catalog(
    store: &Store,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<Option<PackagedDecisionCatalog>, SourceError> {
    let recorder_rows = recorder_manifest
        .and_then(|manifest| manifest.get("replay_packaging"))
        .and_then(|packaging| packaging.get("static_artifacts"))
        .and_then(Value::as_array);
    let recorder_matches: Vec<&Value> = recorder_rows
        .into_iter()
        .flatten()
        .filter(|row| row.get("name").and_then(Value::as_str) == Some(DECISION_CATALOG_NAME))
        .collect();
    if recorder_matches.len() > 1 {
        return Err(SourceError::Open(
            "recorder manifest declares the A34 decision catalog more than once".into(),
        ));
    }

    let index_bytes = store
        .read_game_file_bytes(game_id, "index.json")
        .map_err(|error| {
            SourceError::Open(format!(
                "read authenticated index for A34 decision catalog join: {error}"
            ))
        })?
        .ok_or_else(|| {
            SourceError::Open(
                "selected game index disappeared during A34 decision catalog join".into(),
            )
        })?;
    let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse selected game index for A34 decision catalog join: {error}"
        ))
    })?;
    let index_matches: Vec<&Value> = index
        .get("static_artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("name").and_then(Value::as_str) == Some(DECISION_CATALOG_NAME))
        .collect();
    if index_matches.len() > 1 {
        return Err(SourceError::Open(
            "selected game index declares the A34 decision catalog more than once".into(),
        ));
    }
    let Some(index_row) = index_matches.first().copied() else {
        if recorder_matches.is_empty() {
            return Ok(None);
        }
        return Err(SourceError::Open(
            "recorder manifest declares A34 decision catalog but index roster omits it".into(),
        ));
    };
    if index_row.get("present").and_then(Value::as_bool) != Some(true) {
        if recorder_matches.is_empty() {
            return Ok(None);
        }
        return Err(SourceError::Open(
            "recorder manifest declares A34 decision catalog but index records it absent".into(),
        ));
    }
    // Snapshot-only and legacy directory stores may have an unauthenticated
    // index roster but no recorder manifest at all. They remain playable with
    // XUI19 unavailable; there is no recorder claim to contradict.
    if recorder_manifest.is_none() {
        return Ok(None);
    }
    let recorder_row = recorder_matches.first().copied().ok_or_else(|| {
        SourceError::Open(
            "index records A34 decision catalog present but recorder manifest omits it".into(),
        )
    })?;

    let recorder_path = recorder_row.get("path").and_then(Value::as_str);
    let recorder_sha = recorder_row.get("sha256").and_then(Value::as_str);
    let recorder_len = recorder_row.get("bytes").and_then(Value::as_u64);
    let recorder_content_digest = recorder_row
        .get("content_manifest_digest_sha256")
        .and_then(Value::as_str);
    // The field was introduced with the production A34 package join. Old
    // packages remain playable, but this feature is unavailable rather than
    // consulting a local generated catalog.
    let Some(recorder_content_digest) = recorder_content_digest else {
        return Ok(None);
    };
    if recorder_path != Some(DECISION_CATALOG_PATH)
        || recorder_sha.is_none_or(|value| !lower_sha256(value))
        || recorder_len.is_none()
        || !lower_sha256(recorder_content_digest)
    {
        return Err(SourceError::Open(
            "recorder manifest A34 decision catalog identity is invalid".into(),
        ));
    }
    let recorder_sha = recorder_sha.expect("validated recorder A34 sha");
    let recorder_len = recorder_len.expect("validated recorder A34 length");

    let index_path = index_row.get("path").and_then(Value::as_str);
    let index_sha = index_row.get("sha256").and_then(Value::as_str);
    let index_len = index_row.get("bytes").and_then(Value::as_u64);
    if index_row.get("role").and_then(Value::as_str) != Some("static_artifact")
        || index_path != Some(DECISION_CATALOG_PATH)
        || index_sha != Some(recorder_sha)
        || index_len != Some(recorder_len)
    {
        return Err(SourceError::Open(
            "index A34 decision catalog identity disagrees with recorder manifest".into(),
        ));
    }

    let container = store.container().ok_or_else(|| {
        SourceError::Open(
            "A34 decision catalog requires a portable container content-manifest authority"
                .into(),
        )
    })?;
    let content_digest = container
        .content_manifest()
        .and_then(|manifest| manifest.manifest_digest_sha256.as_deref())
        .ok_or_else(|| {
            SourceError::Open(
                "A34 decision catalog is present but portable content manifest is absent or unfinalized"
                    .into(),
            )
        })?;
    if content_digest != recorder_content_digest {
        return Err(SourceError::Open(
            "A34 decision catalog recorder row is not bound to the selected content manifest"
                .into(),
        ));
    }

    let container_path = format!("payload/{game_id}/{DECISION_CATALOG_PATH}");
    let entries: Vec<_> = container
        .entries_by_role("static_artifact")
        .into_iter()
        .filter(|entry| entry.path == container_path)
        .collect();
    let entry = match entries.as_slice() {
        [entry] => entry,
        [] => {
            return Err(SourceError::Open(
                "A34 decision catalog index row has no matching container-manifest entry".into(),
            ))
        }
        _ => {
            return Err(SourceError::Open(
                "A34 decision catalog has duplicate container-manifest entries".into(),
            ))
        }
    };
    if entry.sha256 != recorder_sha || entry.bytes != recorder_len {
        return Err(SourceError::Open(
            "A34 decision catalog container entry disagrees with recorder/index hashes".into(),
        ));
    }

    let bytes = store
        .read_game_file_bytes(game_id, DECISION_CATALOG_PATH)
        .map_err(|error| {
            SourceError::Open(format!(
                "read verified A34 decision catalog payload: {error}"
            ))
        })?
        .ok_or_else(|| SourceError::Open("verified A34 decision catalog payload is absent".into()))?;
    let actual_sha =
        snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(bytes.as_slice()));
    if bytes.len() as u64 != recorder_len || actual_sha != recorder_sha {
        return Err(SourceError::Open(
            "verified A34 decision catalog bytes disagree with package authorities".into(),
        ));
    }

    Ok(Some(PackagedDecisionCatalog {
        path: DECISION_CATALOG_PATH.into(),
        sha256: actual_sha,
        content_manifest_digest_sha256: content_digest.into(),
        bytes: Arc::from(bytes),
    }))
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

/// Require the recorder-owned content authority to be the exact authority the
/// container reader exposed to playback.  Both objects are individually
/// digest-validated, but without this join a package could carry two valid,
/// different manifests and the replay would silently follow only the outer
/// container copy.
fn validate_recorder_content_manifest_join(
    recorder_session_manifest: &Map<String, Value>,
    container_manifest: Option<&portable_content_manifest::ContentManifest>,
) -> Result<(), SourceError> {
    let recorder_value = recorder_session_manifest.get("portable_content_manifest");
    match (recorder_value, container_manifest) {
        (None | Some(Value::Null), None) => Ok(()),
        (Some(Value::Object(_)), Some(container_manifest)) => {
            let recorder_manifest: portable_content_manifest::ContentManifest =
                serde_json::from_value(
                    recorder_value
                        .expect("object arm has a recorder content manifest")
                        .clone(),
                )
                .map_err(|error| {
                    SourceError::Open(format!(
                        "recorder portable_content_manifest is malformed: {error}"
                    ))
                })?;
            recorder_manifest.validate().map_err(|error| {
                SourceError::Open(format!(
                    "recorder portable_content_manifest failed validation: {error}"
                ))
            })?;
            if &recorder_manifest != container_manifest {
                return Err(SourceError::Open(
                    "recorder portable_content_manifest disagrees with container content_manifest"
                        .into(),
                ));
            }
            let modset = recorder_session_manifest
                .get("modset")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound modset authority".into(),
                    )
                })?;
            let recorder_executable = modset
                .get("game")
                .and_then(Value::as_object)
                .and_then(|game| game.get("exe_sha256"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound executable SHA-256".into(),
                    )
                })?;
            if recorder_executable != container_manifest.source_game.executable_sha256 {
                return Err(SourceError::Open(
                    "recorder modset executable disagrees with portable content source game"
                        .into(),
                ));
            }
            let recorder_language = modset
                .get("localisation_language")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound localisation language"
                            .into(),
                    )
                })?;
            if recorder_language != container_manifest.localisation_language {
                return Err(SourceError::Open(
                    "recorder modset language disagrees with portable content language".into(),
                ));
            }
            let display: RecorderDisplayVariant = serde_json::from_value(
                modset
                    .get("display_variant")
                    .cloned()
                    .ok_or_else(|| {
                        SourceError::Open(
                            "recorder portable content has no launch-bound display variant".into(),
                        )
                    })?,
            )
            .map_err(|error| {
                SourceError::Open(format!(
                    "recorder portable-content display variant is malformed: {error}"
                ))
            })?;
            let expected_settings = BTreeMap::from([
                ("physical_width".into(), display.physical_width.to_string()),
                ("physical_height".into(), display.physical_height.to_string()),
                ("min_gui_width".into(), display.min_gui_width.to_string()),
                ("min_gui_height".into(), display.min_gui_height.to_string()),
                (
                    "gui_scale_f32_bits".into(),
                    display.gui_scale_f32_bits.to_string(),
                ),
                (
                    "refresh_rate_hz".into(),
                    display.refresh_rate_hz.to_string(),
                ),
                (
                    "max_refresh_rate_hz".into(),
                    display
                        .max_refresh_rate_hz
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unavailable".into()),
                ),
                ("fullscreen".into(), display.fullscreen.to_string()),
                ("borderless".into(), display.borderless.to_string()),
                ("display_mode".into(), display.display_mode),
                (
                    "settings_file_sha256".into(),
                    display.settings_file_sha256,
                ),
            ]);
            if container_manifest.required_settings != expected_settings {
                return Err(SourceError::Open(
                    "recorder display variant disagrees with portable content required settings"
                        .into(),
                ));
            }
            Ok(())
        }
        (None | Some(Value::Null), Some(_)) => Err(SourceError::Open(
            "container declares portable content but recorder manifest omits its matching authority"
                .into(),
        )),
        (Some(value), Some(_)) => Err(SourceError::Open(format!(
            "recorder portable_content_manifest must be an object when the container declares one; found {value}"
        ))),
        (Some(_), None) => Err(SourceError::Open(
            "recorder declares portable content but container manifest omits it".into(),
        )),
    }
}

/// Reconcile the recorder's portable-file declarations before selecting either
/// the snapshot or compact-prefold playback path.
///
/// Legacy directory fixtures do not have a container manifest and remain
/// covered by their existing compatibility path.  A recorder-owned portable
/// container, however, must present one exact role/schema graph: manifest and
/// index schema v1, one canonical index role, and a one-to-one index ↔
/// role:snapshot mapping including hour metadata.  This prevents an entry from
/// being selected by path while its declared role says something else.
fn validate_recorder_portable_container_contract(
    store: &Store,
    game_id: &str,
    index: &snapshot::Index,
    metadata: &MetadataRoute,
) -> Result<(), SourceError> {
    let Some(recorder_manifest) = metadata.recorder_manifest.as_ref() else {
        return Ok(());
    };
    let container = store.container().ok_or_else(|| {
        SourceError::Open(
            "recorder-owned replay metadata requires a portable container authority".into(),
        )
    })?;
    validate_recorder_content_manifest_join(recorder_manifest, container.content_manifest())?;
    if let Some(declared_game_id) = recorder_manifest.get("replay_game_id") {
        if declared_game_id.as_str() != Some(game_id) {
            return Err(SourceError::Open(format!(
                "recorder replay_game_id {declared_game_id} disagrees with selected container game_id {game_id:?}"
            )));
        }
    }

    let manifest_schema = container
        .manifest()
        .get("schema_version")
        .ok_or_else(|| {
            SourceError::Open(
                "recorder-owned container manifest has no schema_version authority".into(),
            )
        })?;
    let index_schema = index.schema_version.as_ref().ok_or_else(|| {
        SourceError::Open("recorder-owned replay index has no schema_version authority".into())
    })?;
    if !same_supported_schema_version(manifest_schema, index_schema) {
        return Err(SourceError::Open(format!(
            "recorder-owned container manifest schema_version {manifest_schema} disagrees with index schema_version {index_schema}"
        )));
    }

    let canonical_index = format!("payload/{game_id}/index.json");
    let index_roles = container.entries_by_role("index");
    if index_roles.len() != 1 || index_roles[0].path != canonical_index {
        return Err(SourceError::Open(format!(
            "recorder-owned replay requires exactly one canonical index role {canonical_index:?}; found {:?}",
            index_roles
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>()
        )));
    }
    let index_bytes = container
        .read_entry_bytes(&canonical_index)
        .map_err(|error| {
            SourceError::Open(format!(
                "read canonical index for recorder fold-disposition join: {error}"
            ))
        })?;
    let raw_index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse canonical index for recorder fold-disposition join: {error}"
        ))
    })?;
    if let Some(disposition) = raw_index.get("host_fold_skipped") {
        let skipped = disposition.as_bool().ok_or_else(|| {
            SourceError::Open(
                "recorder-owned index host_fold_skipped must be a boolean".into(),
            )
        })?;
        if skipped != index.snapshots.is_empty() {
            return Err(SourceError::Open(format!(
                "recorder-owned index host_fold_skipped={skipped} contradicts snapshot count {}",
                index.snapshots.len()
            )));
        }
    }

    let snapshot_roles = container.entries_by_role("snapshot");
    if snapshot_roles.len() != index.snapshots.len() {
        return Err(SourceError::Open(format!(
            "recorder-owned replay index lists {} snapshots but container declares {} snapshot roles",
            index.snapshots.len(),
            snapshot_roles.len()
        )));
    }
    for indexed in &index.snapshots {
        let expected_path = format!("payload/{game_id}/{}", indexed.file);
        let matches = snapshot_roles
            .iter()
            .filter(|entry| entry.path == expected_path)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(SourceError::Open(format!(
                "indexed snapshot {:?} requires exactly one matching snapshot role; found {}",
                indexed.file,
                matches.len()
            )));
        }
        let snapshot_bytes = container
            .read_entry_bytes(&matches[0].path)
            .map_err(|error| {
                SourceError::Open(format!(
                    "read indexed snapshot {:?} for hour binding: {error}",
                    matches[0].path
                ))
            })?;
        let snapshot_value = snapshot::parse_json_strict(&snapshot_bytes).map_err(|error| {
            SourceError::Open(format!(
                "strict parse indexed snapshot {:?} for hour binding: {error}",
                matches[0].path
            ))
        })?;
        let body_hour = snapshot_value.get("hour_index").and_then(Value::as_i64);
        if body_hour != indexed.hour_index {
            return Err(SourceError::Open(format!(
                "snapshot role {:?} hour_index {:?} disagrees with index hour_index {:?}",
                matches[0].path, body_hour, indexed.hour_index
            )));
        }
    }

    let timeline = metadata
        .timeline
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open("recorder-owned replay has no timeline object".into())
        })?;
    for (field, expected) in [
        ("format", COMPACT_STREAM_TIMELINE_FORMAT),
        ("stream_format", "compact"),
        ("raw_stream_role", COMPACT_STREAM_ROLE),
        ("session_manifest_role", "recorder_manifest"),
    ] {
        if timeline.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(SourceError::Open(format!(
                "recorder-owned timeline {field} must be {expected:?}"
            )));
        }
    }
    Ok(())
}

/// Bind the recorder's capture-time compact-stream authority to the exact role
/// entry selected for prefolding. Older recorder manifests may omit these
/// objects; once either object exists, all of its current identity fields are
/// required and must agree with the authenticated container entry.
fn validate_recorder_compact_stream_join(
    entry_path: &str,
    entry_bytes: u64,
    entry_sha256: &str,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<(), SourceError> {
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(());
    };
    let relative_path = entry_path
        .strip_prefix(&format!("payload/{game_id}/"))
        .ok_or_else(|| {
            SourceError::Open(format!(
                "compact stream role entry {entry_path:?} is outside selected game {game_id:?}"
            ))
        })?;

    if let Some(storage_value) = recorder_manifest.get("stream_storage") {
        let storage = storage_value.as_object().ok_or_else(|| {
            SourceError::Open("recorder stream_storage authority is not an object".into())
        })?;
        if storage.get("decoded_bytes").and_then(Value::as_u64) != Some(entry_bytes)
            || storage.get("decoded_sha256").and_then(Value::as_str) != Some(entry_sha256)
        {
            return Err(SourceError::Open(
                "recorder stream_storage decoded identity disagrees with selected compact stream role"
                    .into(),
            ));
        }
    }

    if let Some(entries_value) = recorder_manifest.get("replay_entries") {
        let entries = entries_value.as_object().ok_or_else(|| {
            SourceError::Open("recorder replay_entries authority is not an object".into())
        })?;
        if entries.get("stream").and_then(Value::as_str) != Some(relative_path)
            || entries.get("stream_role").and_then(Value::as_str)
                != Some(snapshot::COMPACT_STREAM_ROLE)
        {
            return Err(SourceError::Open(
                "recorder replay_entries stream binding disagrees with selected compact stream role"
                    .into(),
            ));
        }
    }
    Ok(())
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
                    recorder_manifest: None,
                    g2_engine_capture: None,
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
                    recorder_manifest: None,
                    g2_engine_capture: None,
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
        let recorder_manifest =
            parse_metadata_object(&manifest_bytes, &format!("recorder manifest {reference:?}"))?;
        validate_recorder_product_admission(&recorder_manifest)?;
        let binding = recorder_manifest_binding_value(&recorder_manifest, &reference)?;
        return Ok(MetadataRoute {
            binding: Some(binding),
            timeline: Some(Value::Object(timeline)),
            recorder_manifest: Some(recorder_manifest),
            g2_engine_capture: None,
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
            recorder_manifest: None,
            g2_engine_capture: None,
        });
    };
    let Some(reference) = session_manifest_reference(&timeline)? else {
        return Ok(MetadataRoute {
            binding: None,
            timeline: Some(Value::Object(timeline)),
            recorder_manifest: None,
            g2_engine_capture: None,
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
    validate_recorder_product_admission(&manifest)?;
    let binding = recorder_manifest_binding_value(&manifest, &reference)?;
    Ok(MetadataRoute {
        binding: Some(binding),
        timeline: Some(Value::Object(timeline)),
        recorder_manifest: Some(manifest),
        g2_engine_capture: None,
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

/// Reject recorder manifests that explicitly describe a recovery/quarantined
/// artifact or an incomplete production profile. Missing fields remain legacy
/// compatibility; once an authority is present it must positively admit the
/// artifact rather than relying on defaults.
fn validate_recorder_product_admission(
    manifest: &Map<String, Value>,
) -> Result<(), SourceError> {
    if let Some(value) = manifest.get("replay_artifact") {
        let artifact = value.as_object().ok_or_else(|| {
            SourceError::Open("recorder replay_artifact authority is not an object".into())
        })?;
        if artifact.get("recovery").and_then(Value::as_bool) != Some(false)
            || artifact.get("reason").and_then(Value::as_str) != Some("gate_passed")
            || !artifact
                .get("gate_problems")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        {
            return Err(SourceError::Open(
                "recorder replay_artifact is recovery-only or did not positively pass its gate"
                    .into(),
            ));
        }
    }
    if let Some(value) = manifest.get("production_profile") {
        let profile = value.as_object().ok_or_else(|| {
            SourceError::Open("recorder production_profile authority is not an object".into())
        })?;
        if profile.get("enabled").and_then(Value::as_bool) != Some(true) {
            return Err(SourceError::Open(
                "recorder production profile is not positively enabled".into(),
            ));
        }
        for field in [
            "mandatory_domains_not_production_complete",
            "profile_domains_not_scheduled",
            "missing_required_static_artifacts",
        ] {
            let rows = profile
                .get(field)
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SourceError::Open(format!(
                        "recorder production_profile.{field} is absent or is not an array"
                    ))
                })?;
            if !rows.is_empty() {
                return Err(SourceError::Open(format!(
                    "recorder production_profile.{field} is nonempty"
                )));
            }
        }
    }
    Ok(())
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

fn recorder_manifest_binding_value(
    root: &Map<String, Value>,
    reference: &str,
) -> Result<RecorderSessionBinding, SourceError> {
    let campaign_identity = recorder_campaign_identity(&root, reference)?;
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
        campaign_identity,
    })
}

fn recorder_campaign_identity(
    root: &Map<String, Value>,
    reference: &str,
) -> Result<Option<RecordedCampaignIdentity>, SourceError> {
    const OBJECT_KEYS: &[&str] = &[
        "schema",
        "state",
        "campaign_id",
        "game_unique_id",
        "game_unique_seed",
        "source",
        "source_build_profile",
        "observed_game_hour_raw",
        "stream_header_sha256",
        "resolution_note",
        "witnesses",
    ];
    const WITNESS_KEYS: &[&str] = &[
        "start_date_raw",
        "session",
        "scenario",
        "initial_human_country_tags",
        "modset_identity",
    ];
    let Some(value) = root.get("campaign_identity") else {
        return Ok(None);
    };
    let fail = |message: String| {
        SourceError::Open(format!(
            "recorder manifest {reference:?} campaign_identity {message}"
        ))
    };
    let object = value
        .as_object()
        .ok_or_else(|| fail("is not an object".into()))?;
    if object.len() != OBJECT_KEYS.len() || OBJECT_KEYS.iter().any(|key| !object.contains_key(*key))
    {
        return Err(fail("does not have the exact recorder-written key set".into()));
    }
    if object.get("schema").and_then(Value::as_str) != Some("hoi4.campaign_identity.v1") {
        return Err(fail("has an unsupported schema".into()));
    }
    let witnesses = object
        .get("witnesses")
        .and_then(Value::as_object)
        .ok_or_else(|| fail("witnesses is not an object".into()))?;
    if witnesses.len() != WITNESS_KEYS.len()
        || WITNESS_KEYS.iter().any(|key| !witnesses.contains_key(*key))
    {
        return Err(fail("witnesses does not have the exact recorder-written key set".into()));
    }
    for key in ["start_date_raw", "session"] {
        let field = &witnesses[key];
        if !field.is_null()
            && field
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .is_none()
        {
            return Err(fail(format!("witness {key} is not null or u32")));
        }
    }
    for key in ["scenario", "modset_identity"] {
        let field = &witnesses[key];
        if !field.is_null() && field.as_str().is_none() {
            return Err(fail(format!("witness {key} is not null or string")));
        }
    }
    let roster = &witnesses["initial_human_country_tags"];
    if !roster.is_null()
        && roster
            .as_array()
            .map_or(true, |rows| rows.iter().any(|row| row.as_str().is_none()))
    {
        return Err(fail(
            "witness initial_human_country_tags is not null or a string array".into(),
        ));
    }
    let recorded_witnesses = RecordedCampaignIdentityWitnesses {
        start_date_raw: witnesses["start_date_raw"]
            .as_u64()
            .map(|value| value as u32),
        session: witnesses["session"].as_u64().map(|value| value as u32),
        scenario: witnesses["scenario"].as_str().map(str::to_string),
        initial_human_country_tags: roster.as_array().map(|rows| {
            rows.iter()
                .map(|row| row.as_str().expect("witness roster validated").to_string())
                .collect()
        }),
        modset_identity: witnesses["modset_identity"].as_str().map(str::to_string),
    };

    let state = object
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("state is not a string".into()))?;
    let nullable_string = |key: &str| -> Result<Option<String>, SourceError> {
        let value = &object[key];
        if value.is_null() {
            Ok(None)
        } else {
            value
                .as_str()
                .map(|value| Some(value.to_string()))
                .ok_or_else(|| fail(format!("{key} is not null or string")))
        }
    };
    let campaign_id = nullable_string("campaign_id")?;
    let game_unique_id = nullable_string("game_unique_id")?;
    let game_unique_seed = nullable_string("game_unique_seed")?;
    let source = nullable_string("source")?;
    let source_build_profile = nullable_string("source_build_profile")?;
    let stream_header_sha256 = nullable_string("stream_header_sha256")?;
    let resolution_note = nullable_string("resolution_note")?;
    if source_build_profile
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
    {
        return Err(fail("source_build_profile violates its closed grammar".into()));
    }
    let observed_game_hour_raw = if object["observed_game_hour_raw"].is_null() {
        None
    } else {
        let raw = object["observed_game_hour_raw"]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| fail("observed_game_hour_raw is not null or i32".into()))?;
        let min = (1850 * 365 * 24) + 43_799_999;
        let max = (2200 * 365 * 24) + 43_799_999;
        if !(min..=max).contains(&raw) {
            return Err(fail("observed_game_hour_raw is outside the compact calendar".into()));
        }
        Some(raw)
    };
    let lower_hex64 = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };

    match state {
        "known" => {
            let campaign_id = campaign_id
                .as_deref()
                .filter(|value| lower_hex64(value))
                .ok_or_else(|| fail("known campaign_id is not lowercase SHA-256".into()))?;
            let uuid = game_unique_id
                .as_deref()
                .filter(|value| canonical_campaign_uuid(value))
                .ok_or_else(|| fail("known game_unique_id is not a canonical lowercase UUID".into()))?;
            let seed = game_unique_seed
                .as_deref()
                .and_then(|value| value.parse::<i32>().ok().map(|parsed| (value, parsed)))
                .filter(|(value, parsed)| *value == parsed.to_string())
                .map(|(value, _)| value)
                .ok_or_else(|| fail("known game_unique_seed is not canonical signed i32".into()))?;
            let mut preimage = b"hoi4-campaign-id-v1\0".to_vec();
            preimage.extend_from_slice(uuid.as_bytes());
            preimage.push(0);
            preimage.extend_from_slice(seed.as_bytes());
            let recomputed = snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&preimage));
            if recomputed != campaign_id {
                return Err(fail("known campaign digest does not recompute".into()));
            }
            if source.as_deref() != Some("live_memory")
                || source_build_profile.is_none()
                || observed_game_hour_raw.is_none()
                || stream_header_sha256
                    .as_deref()
                    .map_or(true, |value| !lower_hex64(value))
                || resolution_note.is_some()
            {
                return Err(fail("known state fields are incoherent".into()));
            }
        }
        "unavailable" | "unbound" | "identity_component_conflict" => {
            if campaign_id.is_some()
                || game_unique_id.is_some()
                || game_unique_seed.is_some()
                || source.is_some()
                || observed_game_hour_raw.is_some()
                || stream_header_sha256.is_some()
                || resolution_note.as_deref().map_or(true, str::is_empty)
            {
                return Err(fail("non-known state contains positive identity authority".into()));
            }
        }
        _ => return Err(fail(format!("has unknown state {state:?}"))),
    }
    Ok(Some(RecordedCampaignIdentity {
        state: state.to_string(),
        campaign_id,
        game_unique_id,
        game_unique_seed,
        source,
        source_build_profile,
        observed_game_hour_raw,
        stream_header_sha256,
        resolution_note,
        witnesses: recorded_witnesses,
    }))
}

fn canonical_campaign_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
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
    /// Authenticated index dates parallel to `files_by_hour`. A missing date
    /// stays missing; reading navigation metadata never opens a frame body.
    dates_by_hour: BTreeMap<i64, Option<String>>,
    warnings: Vec<String>,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    g2_engine_capture: Option<crate::g2_engine_capture::PackagedG2EngineCapture>,
}

struct SpooledPrefoldedSource {
    spool: snapshot::PrefoldedSpool,
    snapshot_dir: PathBuf,
    owner: PrefoldSpoolOwner,
    warnings: Vec<String>,
    store: Store,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    prefold_receipt: snapshot::PrefoldSourceReceipt,
    g2_engine_capture: Option<crate::g2_engine_capture::PackagedG2EngineCapture>,
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
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    prefold_receipt: Option<snapshot::PrefoldSourceReceipt>,
    g2_engine_capture: Option<crate::g2_engine_capture::PackagedG2EngineCapture>,
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

    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn g2_engine_capture(&self) -> Option<&crate::g2_engine_capture::PackagedG2EngineCapture> {
        self.g2_engine_capture.as_ref()
    }

    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }

    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        Some(&self.prefold_receipt)
    }

    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        let entry = self.spool.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no spooled prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        Ok(Some(entry.date().to_string()))
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

    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn g2_engine_capture(&self) -> Option<&crate::g2_engine_capture::PackagedG2EngineCapture> {
        self.g2_engine_capture.as_ref()
    }

    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }

    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        self.prefold_receipt.as_ref()
    }

    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        let entry = self.replay.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        Ok(entry.snapshot().date.clone())
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
    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }
    fn g2_engine_capture(&self) -> Option<&crate::g2_engine_capture::PackagedG2EngineCapture> {
        self.g2_engine_capture.as_ref()
    }
    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }
    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }
    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }
    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        self.dates_by_hour.get(&hour_index).cloned().ok_or_else(|| {
            SourceError::Open(format!(
                "no snapshot for hour_index {hour_index} in game {:?}",
                self.game_id
            ))
        })
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
    fn shipped_compact_stream_budget_is_closed_at_two_gibibytes() {
        assert!(admit_product_compact_stream_bytes(PRODUCT_COMPACT_STREAM_MAX_BYTES).is_ok());
        let error = admit_product_compact_stream_bytes(PRODUCT_COMPACT_STREAM_MAX_BYTES + 1)
            .expect_err("one byte beyond the product budget must fail before spooling");
        assert!(error.to_string().contains("2 GiB"));
        assert!(PRODUCT_COMPACT_STREAM_MAX_BYTES < 64 * 1024 * 1024 * 1024);
    }

    #[test]
    fn recorder_recovery_and_incomplete_production_profiles_are_not_playable() {
        let admitted = json!({
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
                "gate_problems": [],
            },
            "production_profile": {
                "enabled": true,
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        validate_recorder_product_admission(admitted.as_object().unwrap()).unwrap();

        let mut recovery = admitted.clone();
        recovery["replay_artifact"]["recovery"] = json!(true);
        recovery["replay_artifact"]["reason"] = json!("gate_failed");
        recovery["replay_artifact"]["gate_problems"] = json!(["domain_missing"]);
        let error = validate_recorder_product_admission(recovery.as_object().unwrap())
            .expect_err("recovery replay must never enter production playback");
        assert!(error.to_string().contains("recovery-only"));

        let mut incomplete = admitted;
        incomplete["production_profile"]["mandatory_domains_not_production_complete"] =
            json!(["weather_live"]);
        let error = validate_recorder_product_admission(incomplete.as_object().unwrap())
            .expect_err("non-complete mandatory production domain must fail closed");
        assert!(
            error
                .to_string()
                .contains("mandatory_domains_not_production_complete")
        );

        let mut incomplete = json!({
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
                "gate_problems": [],
            },
            "production_profile": {
                "enabled": true,
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        incomplete["production_profile"]["profile_domains_not_scheduled"] =
            json!(["technology_mio_live"]);
        let error = validate_recorder_product_admission(incomplete.as_object().unwrap())
            .expect_err("incomplete production profile must fail closed");
        assert!(error.to_string().contains("profile_domains_not_scheduled"));

        let missing_positive_authority = json!({
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
            },
            "production_profile": {
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        assert!(
            validate_recorder_product_admission(
                missing_positive_authority.as_object().unwrap()
            )
            .is_err(),
            "present admission authorities must be complete and positive"
        );
    }

    #[test]
    fn recorder_compact_stream_identity_must_match_selected_role() {
        let manifest = json!({
            "stream_storage": {
                "decoded_bytes": 4096,
                "decoded_sha256": "a".repeat(64),
            },
            "replay_entries": {
                "stream": "collector_raw/collector_stream.bin",
                "stream_role": "collector_compact_stream",
            },
        });
        let manifest = manifest.as_object().unwrap();
        validate_recorder_compact_stream_join(
            "payload/game/collector_raw/collector_stream.bin",
            4096,
            &"a".repeat(64),
            "game",
            Some(manifest),
        )
        .unwrap();

        let error = validate_recorder_compact_stream_join(
            "payload/game/collector_raw/collector_stream.bin",
            4097,
            &"a".repeat(64),
            "game",
            Some(manifest),
        )
        .expect_err("different role bytes must not enter prefolding");
        assert!(error.to_string().contains("decoded identity"));
    }

    fn portable_content_manifest_fixture() -> portable_content_manifest::ContentManifest {
        let mut manifest = portable_content_manifest::ContentManifest {
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: "a".repeat(64),
            source_game: portable_content_manifest::SourceGameIdentity {
                version: "1.0".into(),
                build: "fixture".into(),
                executable_sha256: "b".repeat(64),
                build_profile: "fixture-profile".into(),
            },
            source_profile: portable_content_manifest::SourceProfileIdentity {
                profile_id: "fixture-profile".into(),
                playset_id: "fixture-playset".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: "c".repeat(64),
            ordered_layers: Vec::new(),
            dlc_selection: Vec::new(),
            localisation_language: "english".into(),
            required_settings: BTreeMap::from([
                ("physical_width".into(), "1920".into()),
                ("physical_height".into(), "1080".into()),
                ("min_gui_width".into(), "1920".into()),
                ("min_gui_height".into(), "1080".into()),
                ("gui_scale_f32_bits".into(), 1.0f32.to_bits().to_string()),
                ("refresh_rate_hz".into(), "60".into()),
                ("max_refresh_rate_hz".into(), "unavailable".into()),
                ("fullscreen".into(), "false".into()),
                ("borderless".into(), "true".into()),
                ("display_mode".into(), "borderless_fullscreen".into()),
                ("settings_file_sha256".into(), "e".repeat(64)),
            ]),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 =
            manifest.computed_selection_sha256().unwrap();
        manifest.with_computed_digest().unwrap()
    }

    #[test]
    fn recorder_and_container_content_authorities_must_join_exactly() {
        let container = portable_content_manifest_fixture();
        let recorder = json!({
            "portable_content_manifest": serde_json::to_value(&container).unwrap(),
            "modset": {
                "game": {"exe_sha256": container.source_game.executable_sha256.clone()},
                "localisation_language": container.localisation_language.clone(),
                "display_variant": {
                    "physical_width": 1920,
                    "physical_height": 1080,
                    "min_gui_width": 1920,
                    "min_gui_height": 1080,
                    "gui_scale_f32_bits": 1.0f32.to_bits(),
                    "refresh_rate_hz": 60,
                    "max_refresh_rate_hz": null,
                    "fullscreen": false,
                    "borderless": true,
                    "display_mode": "borderless_fullscreen",
                    "settings_file_sha256": "e".repeat(64),
                },
            }
        })
        .as_object()
        .unwrap()
        .clone();
        validate_recorder_content_manifest_join(&recorder, Some(&container)).unwrap();

        let mut wrong_launch = recorder.clone();
        wrong_launch
            .get_mut("modset")
            .unwrap()
            .get_mut("game")
            .unwrap()["exe_sha256"] = json!("d".repeat(64));
        let error = validate_recorder_content_manifest_join(&wrong_launch, Some(&container))
            .expect_err("content authority must join the recorded launch executable");
        assert!(error.to_string().contains("executable disagrees"));

        let mut wrong_display = recorder.clone();
        wrong_display
            .get_mut("modset")
            .unwrap()
            .get_mut("display_variant")
            .unwrap()["physical_width"] = json!(2560);
        let error = validate_recorder_content_manifest_join(&wrong_display, Some(&container))
            .expect_err("content settings must join the recorded display variant");
        assert!(error.to_string().contains("display variant disagrees"));

        let missing = Map::new();
        let error = validate_recorder_content_manifest_join(&missing, Some(&container))
            .expect_err("container-only authority must fail closed");
        assert!(error.to_string().contains("recorder manifest omits"));

        let mut different = container.clone();
        different.product_modset_sha256 = "d".repeat(64);
        different.manifest_digest_sha256 = None;
        different = different.with_computed_digest().unwrap();
        let error = validate_recorder_content_manifest_join(&recorder, Some(&different))
            .expect_err("different valid authorities must fail closed");
        assert!(error.to_string().contains("disagrees"));

        let error = validate_recorder_content_manifest_join(&recorder, None)
            .expect_err("recorder-only authority must fail closed");
        assert!(error.to_string().contains("container manifest omits"));
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

    fn write_portable_recorder_fixture(
        path: &Path,
        game: &str,
        label: &str,
        campaign_identity: Option<Value>,
    ) {
        let content = portable_content_manifest_fixture();
        let absent_static_artifacts = REQUIRED_PACKAGED_STATIC_ARTIFACTS
            .iter()
            .map(|(name, path)| {
                json!({
                    "name": name,
                    "path": path,
                    "present": false,
                })
            })
            .collect::<Vec<_>>();
        let mut recorder_manifest = json!({
            "replay_game_id": game,
            "replay_packaging": {
                "static_artifacts": [],
            },
            "portable_content_manifest": serde_json::to_value(&content).unwrap(),
            "modset": {
                "resolved": true,
                "errors": [],
                "game": {"exe_sha256": content.source_game.executable_sha256.clone()},
                "dlc": {"enabled": []},
                "dlc_display_names": {},
                "localisation_language": content.localisation_language.clone(),
                "display_variant": {
                    "physical_width": 1920,
                    "physical_height": 1080,
                    "min_gui_width": 1920,
                    "min_gui_height": 1080,
                    "gui_scale_f32_bits": 1.0f32.to_bits(),
                    "refresh_rate_hz": 60,
                    "max_refresh_rate_hz": null,
                    "fullscreen": false,
                    "borderless": true,
                    "display_mode": "borderless_fullscreen",
                    "settings_file_sha256": "e".repeat(64),
                },
            },
        });
        if let Some(campaign_identity) = campaign_identity {
            recorder_manifest["campaign_identity"] = campaign_identity;
        }
        let entries = vec![
            snapshot::PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game,
                    "schema_version": "1",
                    "host_fold_skipped": false,
                    "static_artifacts": absent_static_artifacts,
                    "snapshots": [{
                        "hour_index": 1,
                        "file": "000000000001.snapshot.json",
                    }],
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/timeline.json"),
                bytes: serde_json::to_vec(&json!({
                    "format": COMPACT_STREAM_TIMELINE_FORMAT,
                    "stream_format": "compact",
                    "raw_stream_role": COMPACT_STREAM_ROLE,
                    "session_manifest": "recorder/native_session_manifest.json",
                    "session_manifest_role": "recorder_manifest",
                }))
                .unwrap(),
                role: "timeline",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/recorder/native_session_manifest.json"),
                bytes: serde_json::to_vec(&recorder_manifest).unwrap(),
                role: "recorder_manifest",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/000000000001.snapshot.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game,
                    "schema_version": "1",
                    "hour_index": 1,
                    "marker": label,
                }))
                .unwrap(),
                role: "snapshot",
                hour_index: Some(1),
            },
        ];
        let packed = snapshot::pack(
            game,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        fs::write(path, packed).unwrap();
    }

    fn known_campaign_identity_value() -> Value {
        json!({
            "schema": "hoi4.campaign_identity.v1",
            "state": "known",
            "campaign_id": "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f",
            "game_unique_id": "a4b17fed-ed12-4361-9064-fb6576638110",
            "game_unique_seed": "1026046527",
            "source": "live_memory",
            "source_build_profile": "hoi4-a729",
            "observed_game_hour_raw": 60759359,
            "stream_header_sha256": "cc".repeat(32),
            "resolution_note": null,
            "witnesses": {
                "start_date_raw": null,
                "session": null,
                "scenario": null,
                "initial_human_country_tags": null,
                "modset_identity": null
            }
        })
    }

    #[test]
    fn recorder_campaign_identity_is_recomputed_before_replay_exposure() {
        let mut root = Map::new();
        root.insert(
            "campaign_identity".to_string(),
            known_campaign_identity_value(),
        );
        let identity = recorder_campaign_identity(&root, "test manifest")
            .unwrap()
            .expect("known identity must be exposed");
        assert_eq!(identity.state, "known");
        assert_eq!(
            identity.game_unique_id.as_deref(),
            Some("a4b17fed-ed12-4361-9064-fb6576638110")
        );
        assert_eq!(identity.game_unique_seed.as_deref(), Some("1026046527"));
        assert_eq!(
            identity.campaign_id.as_deref(),
            Some("b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f")
        );
        assert_eq!(identity.source.as_deref(), Some("live_memory"));
        assert_eq!(identity.resolution_note, None);
        assert_eq!(identity.witnesses.session, None);

        root.get_mut("campaign_identity")
            .unwrap()["game_unique_seed"] = json!("1026046528");
        assert!(recorder_campaign_identity(&root, "tampered manifest").is_err());
    }

    #[test]
    fn validated_known_manifest_selects_identity_v1_without_stream_sniffing() {
        let mut root = Map::new();
        root.insert(
            "campaign_identity".to_string(),
            known_campaign_identity_value(),
        );
        let known = recorder_campaign_identity(&root, "known mode")
            .unwrap()
            .expect("known identity");
        assert_eq!(
            prefold_stream_identity_mode_from_campaign(Some(&known)),
            snapshot::PrefoldStreamIdentityMode::IdentityV1
        );
        assert_eq!(
            prefold_stream_identity_mode_from_campaign(None),
            snapshot::PrefoldStreamIdentityMode::Legacy
        );
    }

    #[test]
    fn recorder_campaign_identity_rejects_extra_or_positive_non_known_authority() {
        let mut value = known_campaign_identity_value();
        value["unexpected"] = json!(true);
        let mut root = Map::new();
        root.insert("campaign_identity".to_string(), value);
        assert!(recorder_campaign_identity(&root, "extra key").is_err());

        let mut unavailable = known_campaign_identity_value();
        unavailable["state"] = json!("unavailable");
        unavailable["resolution_note"] = json!("live reader unavailable");
        unavailable["campaign_id"] = Value::Null;
        unavailable["game_unique_id"] = Value::Null;
        unavailable["game_unique_seed"] = Value::Null;
        unavailable["source"] = Value::Null;
        unavailable["observed_game_hour_raw"] = Value::Null;
        unavailable["stream_header_sha256"] = Value::Null;
        let mut root = Map::new();
        root.insert("campaign_identity".to_string(), unavailable.clone());
        let unavailable_identity = recorder_campaign_identity(&root, "unavailable")
            .unwrap()
            .expect("non-known metadata must remain inspectable");
        assert_eq!(
            unavailable_identity.resolution_note.as_deref(),
            Some("live reader unavailable")
        );

        unavailable["game_unique_seed"] = json!("1");
        root.insert("campaign_identity".to_string(), unavailable);
        assert!(recorder_campaign_identity(&root, "fabricated unavailable").is_err());
    }

    #[test]
    fn opened_recording_exposes_only_the_validated_recorder_campaign_identity() {
        let root = temp_root("campaign_identity_exposure");
        let recording = root.join("campaign.battleplan");
        write_portable_recorder_fixture(
            &recording,
            "g",
            "CAMPAIGN",
            Some(known_campaign_identity_value()),
        );

        let source = open_recording(&recording).unwrap();
        let identity = source
            .campaign_identity()
            .expect("valid recorder identity must survive source admission");
        assert_eq!(identity.state, "known");
        assert_eq!(identity.observed_game_hour_raw, Some(60759359));
        assert_eq!(
            identity.stream_header_sha256.as_deref(),
            Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );
        drop(source);
        let _ = fs::remove_dir_all(root);
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
        let recording = parent.join("store.battleplan");
        write_portable_recorder_fixture(&recording, "g", "ANCHORED", None);
        let moved = parent.join("anchored_object.battleplan");
        let source = open_recording_impl_with_post_anchor(&recording, None, None, None, |_| {
            // Windows gives the retained container handle delete-denial
            // semantics, so pathname replacement itself must fail. Unix
            // permits unlink/rename of an open object; there the replacement
            // succeeds and the retained handle must still supply ANCHORED.
            #[cfg(windows)]
            {
                fs::rename(&recording, &moved)
                    .expect_err("the retained container handle must deny pathname replacement");
                assert!(recording.is_file());
                assert!(!moved.exists());
            }
            #[cfg(not(windows))]
            {
                fs::rename(&recording, &moved).unwrap();
                write_portable_recorder_fixture(&recording, "g", "IMPOSTOR", None);
            }
        })
        .unwrap();

        let frame = source.frame(1).unwrap();
        assert_eq!(frame.rest["marker"], "ANCHORED");
        assert_eq!(frame.rest["session"]["active_dlcs"], json!([]));
        assert_eq!(frame.rest["session"]["localisation_language"], "english");
        assert_eq!(
            frame.rest["session"]["display_variant"]["physical_width"],
            1920
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
        // Directory stores are a legacy compatibility surface. Recorder-owned
        // metadata is portable-container-only, so this object-swap test keeps
        // the directory fixture explicitly legacy while exercising the same
        // retained selected-game handle boundary.
        write_playable_store(&root, "g", false, "OBJECT_A");
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
        )
    }

    /// Exact inventory for the hardened portable-container root and output
    /// spool capabilities. The root reader lexically normalizes a caller path,
    /// rejects non-local Windows roots, retains every component no-follow, and
    /// denies delete rebinding. The spool sites create a private output file,
    /// authenticate its retained handle, and publish that exact object with a
    /// no-replace hard link. None is a manifest-selected descendant read.
    ///
    /// Keep every literal exact: a newly shaped open/join/push remains a gate
    /// failure until its ownership and handle-identity contract is reviewed.
    fn classified_snapshot_container_retained_io_site(module: &str, line: &str) -> bool {
        module == "snapshot/container.rs"
            && matches!(
                line,
                "use std::fs::{self, File, OpenOptions};"
                    | "use std::path::{Path, PathBuf};"
                    // String-only stitching alias canonicalization. This is
                    // Slice/String::join, not a filesystem Path::join.
                    | "let joined = components.join(\"/\");"
                    // Retained, no-follow container-root open.
                    | "use std::os::windows::fs::{MetadataExt, OpenOptionsExt};"
                    | "use std::path::{Component, Prefix};"
                    | "std::env::current_dir()?.join(path)"
                    | "let mut absolute = PathBuf::from(&prefix);"
                    | "absolute.push(\"\\\\\");"
                    | "absolute.push(name);"
                    | "let mut cursor = PathBuf::from(prefix);"
                    | "cursor.push(\"\\\\\");"
                    | "cursor.push(component.as_os_str());"
                    | "let directory = OpenOptions::new()"
                    | ".open(&cursor)?;"
                    | "if !metadata.is_dir()"
                    | "let file = OpenOptions::new()"
                    | ".open(&absolute)?;"
                    | "if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {"
                    | "File::open(path)"
                    // Authenticated transactional output-spool ownership and
                    // identity checks. These paths originate from the product
                    // spool owner, never from container manifest content.
                    | "use std::path::Component;"
                    | "std::env::current_dir()?.join(destination)"
                    | "let mut current = PathBuf::new();"
                    | "current.push(component.as_os_str());"
                    | "let metadata = fs::symlink_metadata(&current)?;"
                    | "if metadata.file_type().is_symlink() || !metadata.is_dir() {"
                    | "use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};"
                    | "let handle = OpenOptions::new()"
                    | ".open(&current)?;"
                    | "if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {"
                    | "let temp = guard.parent.join(temp_name);"
                    | "use std::os::windows::fs::OpenOptionsExt;"
                    | "OpenOptions::new()"
                    | ".open(path)"
                    | "use std::os::unix::fs::MetadataExt;"
                    | "if !metadata.is_file() {"
                    | "fs::hard_link(&temp, &destination).map_err(|error| {"
                    | "match fs::remove_file(&destination) {"
                    | "let _ = fs::remove_file(&temp);"
            )
    }

    /// Exact capability census for CID1 rollover descriptor discovery. These
    /// sites only identify/read the caller-selected package-set descriptor or
    /// perform bounded, portable path validation; they do not open a member
    /// selected by an unauthenticated descendant pathname. Keep the list exact
    /// so any new discovery primitive or descriptor read fails closed here.
    fn classified_rollover_descriptor_discovery_site(module: &str, line: &str) -> bool {
        module == "replay_core/rollover_package_set.rs"
            && matches!(
                line,
                "use std::fs;"
                    | "use std::path::{Component, Path, PathBuf};"
                    | "let descriptor_path = if path.is_dir() {"
                    | "let metadata = fs::symlink_metadata(path).map_err(|error| {"
                    | "if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {"
                    | "path.join(ROLLOVER_PACKAGE_SET_FILE)"
                    | ".unwrap_or_else(|| Path::new(\".\"));"
                    | "let parent_metadata = fs::symlink_metadata(descriptor_parent).map_err(|error| {"
                    | "if !parent_metadata.file_type().is_dir() || parent_metadata.file_type().is_symlink() {"
                    | "if source_descriptor != Path::new(ROLLOVER_SOURCE_SET_FILE) {"
                    | "|| Path::new(&source_stream.relative_path) != expected_relative"
                    | "let discovered_descriptor = path.is_dir() && path.join(ROLLOVER_PACKAGE_SET_FILE).is_file();"
                    | "let path = PathBuf::from(value);"
                    | "let metadata = fs::symlink_metadata(path)"
                    | "if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {"
                    | "let file = fs::File::open(path)"
            )
    }

    /// Exact capability census for descriptor-authorized rollover members.
    /// The descriptor supplies strict relative paths and expected digests;
    /// resolution rejects links/escapes, and bytes are consumed only from the
    /// retained file/container handle whose receipt is then checked. This is
    /// intentionally line-exact rather than a module-wide filesystem waiver.
    fn classified_rollover_retained_handle_site(module: &str, line: &str) -> bool {
        module == "replay_core/rollover_package_set.rs"
            && matches!(
                line,
                "let root = fs::canonicalize(root)"
                    | "let candidate = root.join(relative);"
                    | "let metadata = fs::symlink_metadata(&candidate)"
                    | "if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {"
                    | "let canonical = fs::canonicalize(&candidate)"
                    | "let mut file = fs::File::open(&canonical)"
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
                "use std::fs::{self, OpenOptions};"
                    | "use std::io::{Read, Seek, SeekFrom, Write};"
                    | "use std::path::{Path, PathBuf};"
                    | "(Some(a), Some(b), Some(c)) => (PathBuf::from(a), b.clone(), PathBuf::from(c)),"
                    | "let gdir = root.join(&game);"
                    | "let index_path = gdir.join(\"index.json\");"
                    | "let index_bytes = match fs::read(&index_path) {"
                    | "let p = gdir.join(file);"
                    | "let bytes = match fs::read(&p) {"
                    | "let tpath = gdir.join(\"timeline.json\");"
                    | "if let Ok(tb) = fs::read(&tpath) {"
                    | "fs::create_dir_all(parent)"
                    | "let canonical_parent = fs::canonicalize(parent.unwrap_or_else(|| Path::new(\".\")))"
                    | "let anchored_out = canonical_parent.join(file_name);"
                    | "let published = fs::read(&anchored_out)"
                    // Output-only atomic publication helpers. `out` is the
                    // caller-selected destination; the temporary child is
                    // created with create_new beneath its canonicalized
                    // parent and is never used to select replay input.
                    | "fn create_unique_temp(out: &Path) -> Result<(PathBuf, fs::File), String> {"
                    | "let parent = out.parent().unwrap_or_else(|| Path::new(\".\"));"
                    | "let temp = parent.join(format!(\".{name}.pack-tmp-{nonce}.{attempt}\"));"
                    | "match OpenOptions::new()"
                    | ".open(&temp)"
                    | "match fs::symlink_metadata(out) {"
                    | "use std::os::windows::fs::MetadataExt;"
                    | "let cleanup = fs::remove_file(&temp);"
                    | "fs::File::open(out.parent().unwrap_or_else(|| Path::new(\".\")))?.sync_all()"
                    | "fs::rename(temp, out)"
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
            "replay_core/lib.rs" => matches!(
                line,
                // Exact module re-export. The lexical F4 lint sees the `fs`
                // suffix in `defs`; this line exposes no filesystem
                // capability and performs no pathname operation.
                "pub use static_focus_defs::{"
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
                || classified_snapshot_container_retained_io_site(module, line)
                || classified_rollover_descriptor_discovery_site(module, line)
                || classified_rollover_retained_handle_site(module, line)
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
        let violations = loader_pathname_violations("snapshot/container.rs", planted);
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
