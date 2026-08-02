//! `replay_decoder` — the shared, pull-based decoder for the game compact
//! record stream.
//!
//! # Why this crate exists
//!
//! The canonical full-detail record of a recording is the compact typed
//! **stream** (`collector_compact_stream`), not per-hour folded JSON snapshots.
//! Production finalization runs this decoder once to certify a complete hourly
//! H4CR byte-offset index and a sparse binary checkpoint/delta cache. The
//! renderer (`replay_interface/native/replay_core`) reconstructs through those
//! portable artifacts while H4CR remains the recording authority. Per-hour JSON
//! is retained only by legacy/debug/export paths.
//!
//! # One decoder, one fold (load-bearing invariant)
//!
//! This crate adds **no** new decode or fold logic. It is a facade over:
//!   * [`compact_contract::parse_compact_record_header`] — the single H4CR
//!     record-framing primitive (also used by the production streaming fold and
//!     the `fold_check` gate);
//!   * [`recorder_host::world_fold::WorldFold`] — the single production fold and
//!     per-domain decoder (`ingest_record` / `take_finished`);
//!   * [`snapshot::container::Container`] — the verified `.battleplan` reader.
//!
//! The reconstructed world-state JSON bytes are **byte-identical** to the
//! canonical WorldFold output, so `replay_core`'s `world.rs` parsing remains
//! unchanged.
//!
//! # Portable ownership boundary
//!
//! `portable_replay/native/snapshot` owns [`snapshot::CertifiedPrefoldDecoder`]
//! and [`snapshot::PrefoldedReplay`]. This extractor crate implements that
//! neutral contract with [`ProductionCertifiedPrefoldDecoder`]; replay_core
//! never links this crate, `recorder_host`, or any lane decoder.
//!
//! # Memory model
//!
//! Iteration is bounded: each finished game-hour is drained from the fold the
//! moment its `FRAME_END` closes ([`WorldFold::take_finished`]) and handed to
//! the caller; no previous hour is retained. Peak memory is
//! `O(current fold state + one hour's snapshot)`, independent of stream length.
//! (The legacy `open_container` path is the one exception: it materializes the verified
//! stream entry in RAM first — see [`open`] — because the container reader has
//! a separate explicit streaming-entry API. Use the certified spool capability
//! for production activation and large fold passes.)
//!
//! # Example (a `replay_core` consumer)
//!
//! ```no_run
//! use replay_decoder::open;
//!
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let mut cursor = open("recording.battleplan")?;
//! println!("schema {}", cursor.metadata().schema_version);
//! while let Some(hour) = cursor.next_hour() {
//!     let hour = hour?;
//!     // `hour.snapshot` is the SAME canonical world-state JSON the snapshot
//!     // entries carry; feed it to the existing world-state parser.
//!     let world: serde_json::Value = hour.to_json()?;
//!     let _ = (hour.hour_index, &hour.date, world);
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use compact_contract::campaign_identity::{
    ValidatedCid1, COMPACT_DOMAIN_STREAM_HEADER, COMPACT_RECORD_TYPE_STREAM_HEADER,
};
use compact_contract::campaign_rollover::{RolloverProposeV1, RolloverReason};
pub use compact_contract::stream_validator::StreamIdentityMode;
use compact_contract::stream_validator::{AcceptedRecordKind, StatefulStreamValidator};
use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, CompactRecordHeader,
    RecordFramingError, ASSIGNED_COMPACT_DOMAINS, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_TYPE_FRAME_REFUSED,
};
use recorder_host::stream_fold::{
    strict_stream_certification_verdict, validate_domain_data_payload, BoundaryTracker,
    FoldStreamStats,
};
/// The DOMAIN_DATA payload's own embedded status, from the single payload
/// authority that already decodes every domain. Re-exported through this facade
/// so a certifier reading the stream classifies the payload status with the
/// same call `validate_domain_data_payload` makes, rather than re-deriving each
/// domain's status offset and drifting from `WorldFold`.
pub use recorder_host::stream_fold::domain_data_payload_status;
use recorder_host::world_fold::{schema_version, FoldedSnapshotFile, WorldFold};
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::{to_hex, Sha256};

/// Read granularity for the streaming raw-stream path. Bounded, independent of
/// stream length (mirrors the production streaming fold's chunk size).
const CHUNK: usize = 8 * 1024 * 1024;

/// ZIP local-file header magic (`PK\x03\x04`) — a `.battleplan` container.
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";

/// Persisted format marker for a source-bound, fully certified stream index.
const LEGACY_CERTIFIED_INDEX_SCHEMA: &str = "h4cr-certified-index-v1";
const CERTIFIED_INDEX_SCHEMA: &str = "h4cr-certified-index-v2";

/// Manifest role of the compact stream inside a `.battleplan` container.
pub const STREAM_ROLE: &str = "collector_compact_stream";

/// Default fold game id for a raw stream that carries no container manifest.
/// (Containers use their manifest `game_id` unless [`OpenOptions::game_id`]
/// overrides it.) It participates in the folded snapshot's `game_id` field and
/// therefore in the per-hour hash, so identical stream bytes folded under the
/// same id hash identically.
pub const DEFAULT_GAME_ID: &str = "replay";

/// One recorder-sealed logical stream in a CID1 rollover set. The expected
/// digests come from the durable seal/open ledger, never from file names or
/// replay metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverStreamDescriptor {
    pub logical_stream_id: u64,
    pub path: PathBuf,
    pub expected_last_complete_sequence: u64,
    pub expected_campaign_id: [u8; 32],
    pub expected_cid1_payload_sha256: [u8; 32],
    pub expected_file_sha256: [u8; 32],
}

/// One exact durable CRP1 transition joining adjacent sealed logical streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverTransitionDescriptor {
    pub current_logical_stream_id: u64,
    pub proposed_logical_stream_id: u64,
    pub last_admitted_h4cr_sequence: u64,
    pub boundary_game_hour_raw: i32,
    pub reason: RolloverReason,
    pub proposal_nonce: u64,
    pub current_campaign_id: [u8; 32],
    pub proposed_campaign_id: [u8; 32],
    pub current_cid1_payload_sha256: [u8; 32],
    pub proposed_cid1_payload_sha256: [u8; 32],
}

impl RolloverTransitionDescriptor {
    fn proposal(&self) -> RolloverProposeV1 {
        RolloverProposeV1 {
            proposed_logical_stream_id: self.proposed_logical_stream_id,
            last_admitted_h4cr_sequence: self.last_admitted_h4cr_sequence,
            boundary_game_hour_raw: self.boundary_game_hour_raw,
            reason: self.reason,
            current_campaign_id: self.current_campaign_id,
            proposed_campaign_id: self.proposed_campaign_id,
            current_cid1_payload_sha256: self.current_cid1_payload_sha256,
            proposed_cid1_payload_sha256: self.proposed_cid1_payload_sha256,
            proposal_nonce: self.proposal_nonce,
        }
    }
}

/// Exact ordered recorder publication selected for replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverSetDescriptor {
    pub ordered_streams: Vec<RolloverStreamDescriptor>,
    pub transitions: Vec<RolloverTransitionDescriptor>,
}

/// Placement-authenticated identities for an admitted logical stream set.
#[derive(Debug, Clone)]
pub struct AdmittedRolloverIdentity {
    pub logical_stream_id: u64,
    pub identity: ValidatedCid1,
}

/// Sequential replay view over independently sealed IdentityV1 streams. Each
/// member is validated from sequence-zero CID1 before any cursor is exposed.
pub struct RolloverStreamSet {
    cursors: VecDeque<HourCursor>,
    identities: Vec<AdmittedRolloverIdentity>,
    transitions: Vec<RolloverTransitionDescriptor>,
    completed_streams: usize,
    terminal: bool,
}

/// One decoded hour tagged by the logical stream that owns it. Raw game hours
/// may repeat or decrease across a same-pair rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverWorldHour {
    pub logical_stream_id: u64,
    pub hour: WorldHour,
}

#[derive(Debug)]
pub enum RolloverSetDecodeError {
    Decode {
        logical_stream_id: u64,
        error: DecodeError,
    },
    Certification {
        logical_stream_id: u64,
        error: String,
    },
}

impl std::fmt::Display for RolloverSetDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode {
                logical_stream_id,
                error,
            } => write!(
                f,
                "logical stream {logical_stream_id} decode failed: {error}"
            ),
            Self::Certification {
                logical_stream_id,
                error,
            } => write!(
                f,
                "logical stream {logical_stream_id} failed certification: {error}"
            ),
        }
    }
}

impl std::error::Error for RolloverSetDecodeError {}

impl RolloverStreamSet {
    pub fn identities(&self) -> &[AdmittedRolloverIdentity] {
        &self.identities
    }

    pub fn transitions(&self) -> &[RolloverTransitionDescriptor] {
        &self.transitions
    }

    pub fn current_logical_stream_id(&self) -> Option<u64> {
        self.identities
            .get(self.completed_streams)
            .map(|identity| identity.logical_stream_id)
    }

    pub fn next_hour(&mut self) -> Option<Result<RolloverWorldHour, RolloverSetDecodeError>> {
        if self.terminal {
            return None;
        }
        loop {
            let logical_stream_id = self
                .current_logical_stream_id()
                .unwrap_or(self.completed_streams as u64);
            let cursor = self.cursors.front_mut()?;
            if let Some(hour) = cursor.next_hour() {
                return Some(hour.map_or_else(
                    |error| {
                        Err(RolloverSetDecodeError::Decode {
                            logical_stream_id,
                            error,
                        })
                    },
                    |hour| {
                        Ok(RolloverWorldHour {
                            logical_stream_id,
                            hour,
                        })
                    },
                ));
            }
            let cursor = self
                .cursors
                .pop_front()
                .expect("front cursor remains present after exhausted borrow");
            if let Err(error) = cursor.certify_complete() {
                self.terminal = true;
                return Some(Err(RolloverSetDecodeError::Certification {
                    logical_stream_id,
                    error: error.to_string(),
                }));
            }
            self.completed_streams = self.completed_streams.saturating_add(1);
        }
    }
}

/// Admit an exact ordered logical-stream set. Missing/non-contiguous ids,
/// absent CID1, digest mismatch, malformed first record, and empty input all
/// fail before a single world hour can be returned.
pub fn open_rollover_stream_set(
    descriptor: RolloverSetDescriptor,
    opts: OpenOptions,
) -> Result<RolloverStreamSet, OpenError> {
    let RolloverSetDescriptor {
        ordered_streams,
        transitions,
    } = descriptor;
    if ordered_streams.is_empty() {
        return Err(OpenError::Certification(
            "CID1 rollover stream set is empty".to_string(),
        ));
    }
    let expected_transition_count = ordered_streams.len().checked_sub(1).ok_or_else(|| {
        OpenError::Certification("CID1 rollover stream count underflow".to_string())
    })?;
    if transitions.len() != expected_transition_count {
        return Err(OpenError::Certification(format!(
            "CID1 rollover transition count {} does not join {} ordered streams",
            transitions.len(),
            ordered_streams.len()
        )));
    }

    let mut identities = Vec::with_capacity(ordered_streams.len());
    let mut certified_frontiers = Vec::with_capacity(ordered_streams.len());
    for (index, stream) in ordered_streams.iter().enumerate() {
        let expected_id = u64::try_from(index).map_err(|_| {
            OpenError::Certification("CID1 rollover stream count exceeds u64".to_string())
        })?;
        if stream.logical_stream_id != expected_id {
            return Err(OpenError::Certification(format!(
                "CID1 rollover logical stream {} is not contiguous id {expected_id}",
                stream.logical_stream_id
            )));
        }
        if stream.expected_last_complete_sequence == u64::MAX
            || stream.expected_campaign_id == [0; 32]
            || stream.expected_cid1_payload_sha256 == [0; 32]
            || stream.expected_file_sha256 == [0; 32]
        {
            return Err(OpenError::Certification(format!(
                "CID1 rollover logical stream {expected_id} has an invalid durable receipt"
            )));
        }
        let actual_file_sha256 = hash_stream_file(&stream.path)?;
        if actual_file_sha256 != stream.expected_file_sha256 {
            return Err(OpenError::Certification(format!(
                "CID1 rollover logical stream {expected_id} whole-file hash mismatch"
            )));
        }
        let identity = read_placement_authenticated_cid1(&stream.path)?;
        if identity.campaign_id() != stream.expected_campaign_id
            || identity.payload_sha256() != stream.expected_cid1_payload_sha256
        {
            return Err(OpenError::Certification(format!(
                "CID1 rollover logical stream {expected_id} disagrees with its durable receipt"
            )));
        }
        let actual_last_complete_sequence = certify_rollover_member(&stream.path, opts.clone())?;
        if actual_last_complete_sequence != stream.expected_last_complete_sequence {
            return Err(OpenError::Certification(format!(
                "CID1 rollover logical stream {expected_id} terminal sequence {actual_last_complete_sequence} disagrees with durable frontier {}",
                stream.expected_last_complete_sequence
            )));
        }
        identities.push(AdmittedRolloverIdentity {
            logical_stream_id: expected_id,
            identity,
        });
        certified_frontiers.push(actual_last_complete_sequence);
    }

    let mut previous_nonce = 0u64;
    for (index, transition) in transitions.iter().enumerate() {
        let current_id = u64::try_from(index).map_err(|_| {
            OpenError::Certification("CID1 rollover transition count exceeds u64".to_string())
        })?;
        let proposed_id = current_id.checked_add(1).ok_or_else(|| {
            OpenError::Certification("CID1 rollover transition id exhausted".to_string())
        })?;
        let current = &ordered_streams[index];
        let proposed = &ordered_streams[index + 1];
        transition.proposal().validate().map_err(|error| {
            OpenError::Certification(format!(
                "CID1 rollover transition {current_id}->{proposed_id} has invalid CRP1 semantics: {error}"
            ))
        })?;
        if transition.current_logical_stream_id != current_id
            || transition.proposed_logical_stream_id != proposed_id
            || transition.last_admitted_h4cr_sequence != certified_frontiers[index]
            || transition.last_admitted_h4cr_sequence != current.expected_last_complete_sequence
            || transition.current_campaign_id != current.expected_campaign_id
            || transition.proposed_campaign_id != proposed.expected_campaign_id
            || transition.current_cid1_payload_sha256 != current.expected_cid1_payload_sha256
            || transition.proposed_cid1_payload_sha256 != proposed.expected_cid1_payload_sha256
        {
            return Err(OpenError::Certification(format!(
                "CID1 rollover transition {current_id}->{proposed_id} is not adjacent to its durable members"
            )));
        }
        if transition.proposal_nonce <= previous_nonce {
            return Err(OpenError::Certification(format!(
                "CID1 rollover transition {current_id}->{proposed_id} proposal nonce is not strictly increasing"
            )));
        }
        if transition.boundary_game_hour_raw != identities[index + 1].identity.game_hour_raw() {
            return Err(OpenError::Certification(format!(
                "CID1 rollover transition {current_id}->{proposed_id} boundary hour does not equal the next placement-authenticated CID1 observation"
            )));
        }
        previous_nonce = transition.proposal_nonce;
    }

    let mut cursors = VecDeque::with_capacity(ordered_streams.len());
    for stream in &ordered_streams {
        cursors.push_back(open_stream_file_with_identity_mode(
            &stream.path,
            &opts,
            StreamIdentityMode::IdentityV1,
        )?);
    }
    Ok(RolloverStreamSet {
        cursors,
        identities,
        transitions,
        completed_streams: 0,
        terminal: false,
    })
}

/// Certify a sealed member before the set constructor exposes any cursor and
/// recover the actual terminal H4CR sequence for the durable frontier join.
fn certify_rollover_member(path: &Path, opts: OpenOptions) -> Result<u64, OpenError> {
    let mut cursor =
        open_stream_file_with_identity_mode(path, &opts, StreamIdentityMode::IdentityV1)?;
    while let Some(hour) = cursor.next_hour() {
        hour.map_err(|error| {
            OpenError::Certification(format!(
                "CID1 rollover member {} failed pre-admission decode: {error}",
                path.display()
            ))
        })?;
    }
    let last_complete_sequence = cursor.strict_previous_sequence.ok_or_else(|| {
        OpenError::Certification(format!(
            "CID1 rollover member {} has no complete record frontier",
            path.display()
        ))
    })?;
    cursor.certify_complete().map_err(|error| {
        OpenError::Certification(format!(
            "CID1 rollover member {} failed complete-stream certification: {error}",
            path.display()
        ))
    })?;
    Ok(last_complete_sequence)
}

fn hash_stream_file(path: &Path) -> Result<[u8; 32], OpenError> {
    let mut file = fs::File::open(path).map_err(OpenError::Io)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(OpenError::Io)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize())
}

fn read_placement_authenticated_cid1(path: &Path) -> Result<ValidatedCid1, OpenError> {
    let mut file = fs::File::open(path).map_err(OpenError::Io)?;
    let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
    file.read_exact(&mut header_bytes).map_err(OpenError::Io)?;
    let header = parse_compact_record_header(&header_bytes).map_err(|error| {
        OpenError::Certification(format!("CID1 rollover first header is invalid: {error}"))
    })?;
    let mut payload = vec![0u8; header.payload_len];
    file.read_exact(&mut payload).map_err(OpenError::Io)?;
    let mut validator = StatefulStreamValidator::identity_v1();
    let kind = validator
        .validate_record(&header, &payload)
        .map_err(|error| {
            OpenError::Certification(format!("CID1 rollover first record is invalid: {error}"))
        })?;
    if kind != AcceptedRecordKind::CampaignIdentity {
        return Err(OpenError::Certification(
            "CID1 rollover stream did not begin with campaign identity".to_string(),
        ));
    }
    validator.identity().cloned().ok_or_else(|| {
        OpenError::Certification("CID1 rollover validator retained no identity".to_string())
    })
}

fn stream_validator(mode: StreamIdentityMode) -> StatefulStreamValidator {
    match mode {
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
    }
}

fn compact_identity_mode(mode: snapshot::PrefoldStreamIdentityMode) -> StreamIdentityMode {
    match mode {
        snapshot::PrefoldStreamIdentityMode::Legacy => StreamIdentityMode::Legacy,
        snapshot::PrefoldStreamIdentityMode::IdentityV1 => StreamIdentityMode::IdentityV1,
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

/// One compact domain's registry identity (from `compact_contract`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainInfo {
    pub id: u16,
    pub canonical_name: String,
    pub wire_name: String,
}

/// Session/stream metadata a consumer can read without folding a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamMeta {
    /// The canonical world-state schema version (`schema_version` field of every
    /// folded hour). Stable contract for `replay_core`'s parser.
    pub schema_version: String,
    /// The game id folds are keyed under (container manifest id, or the raw
    /// stream default / override).
    pub game_id: String,
    /// The compact domain registry: id ↔ names. Static per build.
    pub domains: Vec<DomainInfo>,
}

impl StreamMeta {
    fn new(game_id: impl Into<String>) -> StreamMeta {
        let domains = ASSIGNED_COMPACT_DOMAINS
            .iter()
            .map(|d| DomainInfo {
                id: d.id,
                canonical_name: d.canonical_name.to_string(),
                wire_name: d.wire_name.to_string(),
            })
            .collect();
        StreamMeta {
            schema_version: schema_version().to_string(),
            game_id: game_id.into(),
            domains,
        }
    }

    /// Wire name for a domain id, or `None` if unassigned.
    pub fn domain_name(&self, id: u16) -> Option<&str> {
        self.domains
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.wire_name.as_str())
    }
}

// ---------------------------------------------------------------------------
// Per-hour world state
// ---------------------------------------------------------------------------

/// One decoded game-hour world state, yielded by [`HourCursor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldHour {
    /// Canonical sort key: `raw_date - DATE_EPOCH_CONST`.
    pub hour_index: i64,
    /// Clausewitz `"year.month.day.hour"` string.
    pub date: String,
    /// The canonical world-state JSON bytes — byte-identical to the snapshot
    /// entry a container ships for this hour (the input to `replay_core`).
    pub snapshot: Vec<u8>,
    /// DOMAIN_DATA record counts per domain id that fed THIS hour (records seen
    /// since the previous `FRAME_END`). Mirrors `fold_check`'s per-hour counts.
    pub domain_data_counts: BTreeMap<u16, u64>,
}

impl WorldHour {
    /// The canonical snapshot bytes.
    pub fn snapshot_bytes(&self) -> &[u8] {
        &self.snapshot
    }

    /// Parse the snapshot bytes into a `serde_json::Value`.
    pub fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::from_slice(&self.snapshot)
    }

    /// The canonical SHA-256 of the snapshot bytes (the `fold_check` per-hour
    /// hash on the same input).
    pub fn snapshot_sha256(&self) -> String {
        to_hex(&Sha256::digest(&self.snapshot))
    }
}

// ---------------------------------------------------------------------------
// Decode accounting
// ---------------------------------------------------------------------------

/// Running totals for one decode pass. Never grows with stream length. Mirrors
/// the `fold_check`/`stream_fold` totals so the three can be cross-checked.
#[derive(Debug, Default, Clone)]
pub struct DecodeStats {
    /// Total decoded-stream length (file size, or the container entry's
    /// uncompressed size).
    pub stream_bytes: u64,
    /// Bytes consumed as whole, well-framed records.
    pub bytes_folded: u64,
    /// Trailing bytes that did not form a complete record (mid-record EOF).
    pub truncated_tail_bytes: u64,
    /// Accepted ordinary H4CR records. The mandatory IdentityV1 CID1 header is
    /// a stream header and is deliberately excluded from this count.
    pub records_total: u64,
    pub data_records: u64,
    /// Framed DOMAIN_DATA records rejected by the production fold's exact
    /// structural payload validator. Their domain state is degraded/unknown;
    /// they are never eligible for strict certification.
    pub malformed_payload_records: u64,
    pub frame_end_records: u64,
    pub perf_records: u64,
    pub other_records: u64,
    pub hours_emitted: u64,
    pub per_domain_data_records: BTreeMap<u16, u64>,
    /// Set when framing stopped early on a malformed record (bad magic/version).
    pub stop_reason: Option<String>,
    /// Canonical envelope or logical-boundary failure: invalid record flags,
    /// PERF/FRAME_END shape, residual bytes, open/interleaved hour, mismatched
    /// FRAME_END, sequence discontinuity, or no closed hour.
    pub boundary_error: Option<String>,
    /// True only when both the production tracker and strict extractor
    /// certification accept the final closed-hour boundary.
    pub ends_at_frame_end: bool,
}

impl DecodeStats {
    /// Fail-closed certification verdict. Prefix recovery remains available to
    /// ordinary decoder callers, but hash/equivalence gates must require this
    /// result before treating any emitted hours as authoritative.
    pub fn certification_verdict(&self) -> Result<(), String> {
        strict_stream_certification_verdict(
            self.stop_reason.as_deref(),
            self.truncated_tail_bytes,
            self.malformed_payload_records,
            self.ends_at_frame_end,
            self.boundary_error.as_deref(),
        )
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A failure opening a decode source.
#[derive(Debug)]
pub enum OpenError {
    Io(io::Error),
    /// The input's leading bytes matched neither an H4CR stream nor a ZIP
    /// container.
    UnrecognizedInput(String),
    /// A `.battleplan` container could not be opened/validated, or had no (or
    /// more than one) `collector_compact_stream` entry.
    Container(String),
    /// Full-stream certification failed, so no authoritative index exists.
    Certification(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Io(e) => write!(f, "io: {e}"),
            OpenError::UnrecognizedInput(s) => write!(f, "unrecognized input: {s}"),
            OpenError::Container(s) => write!(f, "container: {s}"),
            OpenError::Certification(s) => write!(f, "certification: {s}"),
        }
    }
}
impl std::error::Error for OpenError {}
impl From<io::Error> for OpenError {
    fn from(e: io::Error) -> Self {
        OpenError::Io(e)
    }
}

/// A failure while pulling the next hour (only I/O — malformed framing is a
/// graceful stop recorded in [`DecodeStats::stop_reason`], not an error).
#[derive(Debug)]
pub enum DecodeError {
    Io(io::Error),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Io(e) => write!(f, "io: {e}"),
        }
    }
}
impl std::error::Error for DecodeError {}

/// A failure seeking to a specific hour.
#[derive(Debug)]
pub enum SeekError {
    Open(OpenError),
    Decode(DecodeError),
    /// The target hour is not present in the index / stream.
    HourNotFound(i64),
    /// The persisted index is structurally invalid or is not the certified
    /// index for the exact source/options being sought.
    InvalidIndex(String),
}

impl std::fmt::Display for SeekError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeekError::Open(e) => write!(f, "open: {e}"),
            SeekError::Decode(e) => write!(f, "decode: {e}"),
            SeekError::HourNotFound(h) => write!(f, "hour {h} not found in stream"),
            SeekError::InvalidIndex(s) => write!(f, "invalid stream index: {s}"),
        }
    }
}
impl std::error::Error for SeekError {}

/// Why an in-progress decoder cursor could not mint an authoritative index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexCertificationError {
    NotExhausted,
    Stream(String),
    Length(String),
    UnidentifiedSource,
    InvalidIndex(String),
}

impl std::fmt::Display for IndexCertificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexCertificationError::NotExhausted => {
                write!(f, "cursor has not reached physical EOF")
            }
            IndexCertificationError::Stream(s) => write!(f, "stream is not certified: {s}"),
            IndexCertificationError::Length(s) => write!(f, "source length mismatch: {s}"),
            IndexCertificationError::UnidentifiedSource => write!(
                f,
                "reader-only cursor has no stable source identity; use build_index on Source"
            ),
            IndexCertificationError::InvalidIndex(s) => {
                write!(f, "certified index invariant failed: {s}")
            }
        }
    }
}

impl std::error::Error for IndexCertificationError {}

// ---------------------------------------------------------------------------
// Opening a source
// ---------------------------------------------------------------------------

/// Options controlling how a source is folded.
#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    /// Fold under this game id instead of the source default. Containers
    /// otherwise use their manifest `game_id`; raw streams otherwise use
    /// [`DEFAULT_GAME_ID`]. (The `fold_check` gate sets this to `"fold_check"`
    /// so two files with identical stream bytes hash identically.)
    pub game_id: Option<String>,
}

/// A reusable, re-openable description of a decode source. Needed for
/// [`seek_to_hour`], which re-opens the stream to re-fold from a seek point.
#[derive(Debug, Clone)]
pub enum Source {
    /// A raw or zstd-compressed collector_stream.bin (decodes to H4CR).
    StreamFile(PathBuf),
    /// A `.battleplan` container (its `collector_compact_stream` entry is used).
    Container(PathBuf),
}

impl Source {
    /// Detect the source kind of `path` by its leading bytes (independent of the
    /// filename), exactly as the `fold_check` gate does.
    pub fn detect(path: impl AsRef<Path>) -> Result<Source, OpenError> {
        let path = path.as_ref();
        let mut f = compact_stream::open(path)?;
        let mut magic = [0u8; 4];
        let read = read_up_to(&mut f, &mut magic)?;
        if read >= 4 && &magic == compact_contract::COMPACT_RECORD_MAGIC {
            Ok(Source::StreamFile(path.to_path_buf()))
        } else if read >= 4 && &magic == ZIP_MAGIC {
            Ok(Source::Container(path.to_path_buf()))
        } else {
            Err(OpenError::UnrecognizedInput(format!(
                "{}: decoded leading bytes {:02x?}; expected an H4CR stream or a .battleplan container",
                path.display(),
                &magic[..read.min(4)]
            )))
        }
    }

    /// Open a fresh [`HourCursor`] over this source with the given options.
    pub fn open_cursor(&self, opts: &OpenOptions) -> Result<HourCursor, OpenError> {
        self.open_cursor_with_identity_mode(opts, StreamIdentityMode::Legacy)
    }

    /// Open a fresh cursor under an explicitly selected stream protocol.
    /// IdentityV1 is never inferred from bytes and never falls back to Legacy.
    pub fn open_cursor_with_identity_mode(
        &self,
        opts: &OpenOptions,
        identity_mode: StreamIdentityMode,
    ) -> Result<HourCursor, OpenError> {
        match self {
            Source::StreamFile(path) => {
                let canonical_path = fs::canonicalize(path)?;
                let file = compact_stream::open(&canonical_path)?;
                let compressed = file.compression() == compact_stream::StreamCompression::Zstd;
                let total_len = if compressed { 0 } else { file.encoded_len() };
                let game_id = opts
                    .game_id
                    .clone()
                    .unwrap_or_else(|| DEFAULT_GAME_ID.to_string());
                Ok(HourCursor::from_reader_compressed_with_source(
                    Box::new(file),
                    total_len,
                    game_id,
                    compressed,
                    identity_mode,
                    Some(IndexSourceDescriptor {
                        kind: "stream_file".to_string(),
                        canonical_path: canonical_path.to_string_lossy().into_owned(),
                    }),
                ))
            }
            Source::Container(path) => {
                let canonical_path = fs::canonicalize(path)?;
                let (bytes, manifest_game_id) = extract_container_stream(&canonical_path)?;
                let total_len = bytes.len() as u64;
                let game_id = opts.game_id.clone().unwrap_or(manifest_game_id);
                Ok(HourCursor::from_reader_with_source(
                    Box::new(Cursor::new(bytes)),
                    total_len,
                    game_id,
                    identity_mode,
                    Some(IndexSourceDescriptor {
                        kind: "container".to_string(),
                        canonical_path: canonical_path.to_string_lossy().into_owned(),
                    }),
                ))
            }
        }
    }
}

/// Open a decode source from a path, auto-detecting a raw `collector_stream.bin`
/// vs a `.battleplan` container. Containers fold under their manifest `game_id`.
pub fn open(path: impl AsRef<Path>) -> Result<HourCursor, OpenError> {
    open_with(path, &OpenOptions::default())
}

/// [`open`] with explicit [`OpenOptions`].
pub fn open_with(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<HourCursor, OpenError> {
    Source::detect(path)?.open_cursor(opts)
}

/// [`open_with`] under an explicit, non-sniffed compact-stream identity mode.
pub fn open_with_identity_mode(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<HourCursor, OpenError> {
    Source::detect(path)?.open_cursor_with_identity_mode(opts, identity_mode)
}

/// Open a raw `collector_stream.bin` explicitly (skips kind auto-detection).
pub fn open_stream_file(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
) -> Result<HourCursor, OpenError> {
    Source::StreamFile(path.as_ref().to_path_buf()).open_cursor(opts)
}

/// [`open_stream_file`] under an explicit, non-sniffed identity mode.
pub fn open_stream_file_with_identity_mode(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<HourCursor, OpenError> {
    Source::StreamFile(path.as_ref().to_path_buf())
        .open_cursor_with_identity_mode(opts, identity_mode)
}

/// Open a `.battleplan` container explicitly (its `collector_compact_stream`
/// entry).
pub fn open_container(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<HourCursor, OpenError> {
    Source::Container(path.as_ref().to_path_buf()).open_cursor(opts)
}

/// [`open_container`] under an explicit, non-sniffed identity mode.
pub fn open_container_with_identity_mode(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<HourCursor, OpenError> {
    Source::Container(path.as_ref().to_path_buf())
        .open_cursor_with_identity_mode(opts, identity_mode)
}

/// Open a cursor over an already-obtained reader (in-memory bytes, a socket,
/// …). `total_len` is the full decoded length used to report the truncated
/// tail; pass `0` if unknown (then no tail is reported). This path has no
/// [`seek_to_hour`] support (there is no re-openable [`Source`]).
pub fn open_reader<R: Read + 'static>(
    reader: R,
    total_len: u64,
    game_id: impl Into<String>,
) -> HourCursor {
    open_reader_with_identity_mode(reader, total_len, game_id, StreamIdentityMode::Legacy)
}

/// [`open_reader`] under an explicit, non-sniffed identity mode.
pub fn open_reader_with_identity_mode<R: Read + 'static>(
    reader: R,
    total_len: u64,
    game_id: impl Into<String>,
    identity_mode: StreamIdentityMode,
) -> HourCursor {
    HourCursor::from_reader(Box::new(reader), total_len, game_id.into(), identity_mode)
}

/// Fully fold and certify one physically raw H4CR file, then hand its closed
/// hour index and canonical snapshot entries across the neutral portable seam.
///
/// This production-shaped convenience is intentionally narrower than [`open`]:
/// it rejects ZIP containers and zstd wrappers by leading bytes, exposes no
/// prefix before physical EOF, and constructs the portable object only after
/// [`HourCursor::take_index`] has certified the complete source. The returned
/// type lives in `portable_replay`, so the standalone replay tree does not gain
/// a dependency on this extractor crate.
pub fn fold_raw_h4cr_prefolded(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
) -> Result<snapshot::PrefoldedReplay, OpenError> {
    fold_raw_h4cr_prefolded_with_identity_mode(path, opts, StreamIdentityMode::Legacy)
}

/// [`fold_raw_h4cr_prefolded`] under an explicit, non-sniffed identity mode.
pub fn fold_raw_h4cr_prefolded_with_identity_mode(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<snapshot::PrefoldedReplay, OpenError> {
    let canonical_path = fs::canonicalize(path.as_ref())?;
    let mut file = fs::File::open(&canonical_path)?;
    let total_len = file.metadata()?.len();
    let mut magic = [0u8; 4];
    let read = read_up_to(&mut file, &mut magic)?;
    if read != magic.len() || magic != *compact_contract::COMPACT_RECORD_MAGIC {
        return Err(OpenError::UnrecognizedInput(format!(
            "{}: physically raw H4CR input required; leading bytes were {:02x?}",
            canonical_path.display(),
            &magic[..read]
        )));
    }
    file.seek(SeekFrom::Start(0))?;

    let game_id = opts
        .game_id
        .clone()
        .unwrap_or_else(|| DEFAULT_GAME_ID.to_string());
    let mut cursor = HourCursor::from_reader_with_source(
        Box::new(file),
        total_len,
        game_id.clone(),
        identity_mode,
        Some(IndexSourceDescriptor {
            kind: "stream_file".to_string(),
            canonical_path: canonical_path.to_string_lossy().into_owned(),
        }),
    );
    let schema_version = serde_json::Value::String(cursor.metadata().schema_version.clone());
    let mut folded = Vec::new();
    while let Some(hour) = cursor.next_hour() {
        folded.push(hour.map_err(|error| match error {
            DecodeError::Io(error) => OpenError::Io(error),
        })?);
    }
    validate_prefold_cardinality(&cursor, folded.len())?;
    cursor
        .take_index()
        .map_err(|error| OpenError::Certification(error.to_string()))?;

    let entries = folded
        .into_iter()
        .map(|hour| snapshot::PrefoldedHourEntry::from_json(hour.hour_index, &hour.snapshot))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            OpenError::Certification(format!(
                "certified fold did not satisfy the neutral prefolded contract: {error}"
            ))
        })?;
    snapshot::PrefoldedReplay::try_new(game_id, schema_version, entries).map_err(|error| {
        OpenError::Certification(format!(
            "certified fold did not satisfy the neutral prefolded contract: {error}"
        ))
    })
}

/// The extractor-owned implementation of the neutral portable pre-fold seam.
///
/// This zero-sized adapter does not move or duplicate the fold: it drives this
/// crate's existing cursor, which in turn drives the single production
/// [`WorldFold`]. `replay_core` depends only on the portable trait and never on
/// this type or crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProductionCertifiedPrefoldDecoder;

impl snapshot::CertifiedPrefoldDecoder for ProductionCertifiedPrefoldDecoder {
    fn decoder_id(&self) -> &'static str {
        "extractor.replay_decoder.production_world_fold.v1"
    }

    fn decode_certified(
        &self,
        request: snapshot::CertifiedPrefoldRequest,
    ) -> Result<snapshot::PrefoldedReplay, snapshot::PrefoldDecodeError> {
        let identity_mode = compact_identity_mode(request.identity_mode());
        let (game_id, raw_h4cr) = request.into_parts();
        fold_raw_h4cr_bytes_prefolded_with_identity_mode(raw_h4cr, game_id, identity_mode).map_err(
            |error| {
                snapshot::PrefoldDecodeError::new(format!(
                    "{}: {error}",
                    snapshot::CertifiedPrefoldDecoder::decoder_id(self)
                ))
            },
        )
    }
}

impl snapshot::CertifiedPrefoldSpoolDecoder for ProductionCertifiedPrefoldDecoder {
    fn decoder_id(&self) -> &'static str {
        "extractor.replay_decoder.production_world_fold.spool.v1"
    }

    fn decode_certified_spool(
        &self,
        request: snapshot::CertifiedPrefoldSpoolRequest,
    ) -> Result<snapshot::PrefoldedSpool, snapshot::PrefoldDecodeError> {
        let identity_mode = compact_identity_mode(request.identity_mode());
        fold_raw_h4cr_file_spooled_with_identity_mode(request, identity_mode).map_err(|error| {
            snapshot::PrefoldDecodeError::new(format!(
                "{}: {error}",
                snapshot::CertifiedPrefoldSpoolDecoder::decoder_id(self)
            ))
        })
    }
}

/// Recorder-packaged binary accelerator generated by the one production fold.
///
/// The H4CR source remains canonical. `hour_index_path` binds every emitted
/// hour to its exact H4CR byte range, while `checkpoint_cache_path` contains a
/// sparse full checkpoint and at most `checkpoint_interval_hours - 1` binary
/// state deltas for any seek. The parity pass reconstructs every hour before
/// this receipt can be returned.
#[derive(Debug, Clone)]
pub struct CertifiedReplayArtifacts {
    pub hour_index_path: PathBuf,
    pub checkpoint_cache_path: PathBuf,
    pub hour_count: u64,
    pub checkpoint_count: u64,
    pub checkpoint_interval_hours: u32,
    pub max_seek_deltas: u32,
    pub source_len: u64,
    pub source_sha256: String,
    pub hour_index_len: u64,
    pub hour_index_sha256: String,
    pub checkpoint_cache_len: u64,
    pub checkpoint_cache_sha256: String,
    pub legacy_json_bytes_avoided: u64,
    pub production_render_blob_dir: PathBuf,
    pub production_render_blob_count: u64,
    pub production_render_blob_bytes: u64,
}

/// Fold a complete raw H4CR source once and build the certified production
/// hour index/checkpoint cache. No per-hour JSON file is written.
///
/// The output directory must already exist and be empty. Artifacts are written
/// with `create_new`, flushed, and synced. They remain caller-owned after this
/// function returns; the decoder never deletes the active spill.
pub fn build_certified_replay_artifacts(
    raw_h4cr_path: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
    game_id: impl Into<String>,
    identity_mode: StreamIdentityMode,
    expected_source_len: u64,
    expected_source_sha256: &str,
    checkpoint_interval_hours: u32,
) -> Result<CertifiedReplayArtifacts, OpenError> {
    let raw_h4cr_path = fs::canonicalize(raw_h4cr_path.as_ref())?;
    let output_dir = fs::canonicalize(output_dir.as_ref())?;
    let output_meta = fs::symlink_metadata(&output_dir)?;
    if !output_meta.file_type().is_dir()
        || output_meta.file_type().is_symlink()
        || fs::read_dir(&output_dir)?.next().is_some()
    {
        return Err(OpenError::Certification(format!(
            "replay artifact spill must be an empty regular directory: {}",
            output_dir.display()
        )));
    }
    if expected_source_len < 4 || !is_lower_sha256(expected_source_sha256) {
        return Err(OpenError::Certification(
            "replay artifact source receipt is invalid".to_string(),
        ));
    }
    let source_meta = fs::symlink_metadata(&raw_h4cr_path)?;
    if !source_meta.file_type().is_file()
        || source_meta.file_type().is_symlink()
        || source_meta.len() != expected_source_len
    {
        return Err(OpenError::Certification(format!(
            "raw H4CR source metadata disagrees with its receipt: {}",
            raw_h4cr_path.display()
        )));
    }
    let game_id = game_id.into();
    if game_id.trim().is_empty() {
        return Err(OpenError::Certification(
            "replay artifact game_id is empty".to_string(),
        ));
    }

    let cache_path = output_dir.join(snapshot::CHECKPOINT_CACHE_FILE);
    let index_path = output_dir.join(snapshot::HOUR_INDEX_FILE);
    if let Some(parent) = cache_path.parent() {
        fs::create_dir(parent)?;
    }
    let cache_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cache_path)?;
    let mut cache_encoder = snapshot::ReplayCacheEncoder::new(
        cache_file,
        expected_source_sha256,
        checkpoint_interval_hours,
    )
    .map_err(|error| OpenError::Certification(error.to_string()))?;

    let source_file = fs::File::open(&raw_h4cr_path)?;
    let mut cursor = HourCursor::from_reader_with_source(
        Box::new(source_file),
        expected_source_len,
        game_id.clone(),
        identity_mode,
        Some(IndexSourceDescriptor {
            kind: "stream_file".to_string(),
            canonical_path: raw_h4cr_path.to_string_lossy().into_owned(),
        }),
    );
    let mut pending: Option<(WorldHour, u64)> = None;
    let mut legacy_json_bytes_avoided = 0u64;
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.map_err(|error| match error {
            DecodeError::Io(error) => OpenError::Io(error),
        })?;
        let start_offset = cursor
            .index_hours
            .last()
            .filter(|offset| offset.hour_index == hour.hour_index)
            .map(|offset| offset.start_offset)
            .ok_or_else(|| {
                OpenError::Certification(format!(
                    "decoder emitted hour {} without its offset index row",
                    hour.hour_index
                ))
            })?;
        if let Some((previous, previous_start)) = pending.take() {
            recorder_host::world_fold::preserve_production_render_blobs_from_json(
                &output_dir,
                &previous.snapshot,
            )?;
            legacy_json_bytes_avoided = legacy_json_bytes_avoided
                .checked_add(previous.snapshot.len() as u64)
                .ok_or_else(|| {
                    OpenError::Certification(
                        "legacy JSON byte accounting overflowed u64".to_string(),
                    )
                })?;
            cache_encoder
                .push(snapshot::CacheBuildHour {
                    hour_index: previous.hour_index,
                    date: previous.date,
                    h4cr_start_offset: previous_start,
                    h4cr_end_offset: start_offset,
                    world_json: previous.snapshot,
                })
                .map_err(|error| OpenError::Certification(error.to_string()))?;
        }
        pending = Some((hour, start_offset));
    }
    let stream_index = cursor
        .take_index()
        .map_err(|error| OpenError::Certification(error.to_string()))?;
    if stream_index.source.decoded_len != expected_source_len
        || stream_index.source.decoded_sha256 != expected_source_sha256
    {
        return Err(OpenError::Certification(format!(
            "certified H4CR identity {}:{} disagrees with expected {}:{}",
            stream_index.source.decoded_len,
            stream_index.source.decoded_sha256,
            expected_source_len,
            expected_source_sha256
        )));
    }
    let (last, last_start) = pending.ok_or_else(|| {
        OpenError::Certification("certified H4CR stream emitted no hours".to_string())
    })?;
    recorder_host::world_fold::preserve_production_render_blobs_from_json(
        &output_dir,
        &last.snapshot,
    )?;
    legacy_json_bytes_avoided = legacy_json_bytes_avoided
        .checked_add(last.snapshot.len() as u64)
        .ok_or_else(|| {
            OpenError::Certification("legacy JSON byte accounting overflowed u64".to_string())
        })?;
    cache_encoder
        .push(snapshot::CacheBuildHour {
            hour_index: last.hour_index,
            date: last.date,
            h4cr_start_offset: last_start,
            h4cr_end_offset: expected_source_len,
            world_json: last.snapshot,
        })
        .map_err(|error| OpenError::Certification(error.to_string()))?;

    let cache_identity_mode = match identity_mode {
        StreamIdentityMode::Legacy => snapshot::CacheIdentityMode::Legacy,
        StreamIdentityMode::IdentityV1 => snapshot::CacheIdentityMode::IdentityV1,
    };
    let schema = schema_version().to_string();
    let (mut cache_file, replay_index) = cache_encoder
        .finish(game_id, schema, cache_identity_mode, expected_source_len)
        .map_err(|error| OpenError::Certification(error.to_string()))?;
    cache_file.flush()?;
    cache_file.sync_all()?;
    drop(cache_file);

    if replay_index.hours().len() != stream_index.hours.len()
        || replay_index
            .hours()
            .iter()
            .zip(&stream_index.hours)
            .any(|(artifact, stream)| {
                artifact.hour_index() != stream.hour_index
                    || artifact.h4cr_start_offset() != stream.start_offset
            })
    {
        return Err(OpenError::Certification(
            "binary replay hour index disagrees with the certified H4CR fold index".to_string(),
        ));
    }
    let index_bytes = replay_index
        .to_bytes()
        .map_err(|error| OpenError::Certification(error.to_string()))?;
    let mut index_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&index_path)?;
    index_file.write_all(&index_bytes)?;
    index_file.flush()?;
    index_file.sync_all()?;
    drop(index_file);

    let cache_bytes = fs::read(&cache_path)?;
    let parity_reader = snapshot::ReplayCacheReader::open(
        &index_bytes,
        &cache_bytes,
        expected_source_len,
        expected_source_sha256,
    )
    .map_err(|error| OpenError::Certification(error.to_string()))?;
    let mut max_seek_deltas = 0u32;
    for hour in replay_index.hours() {
        let (_, receipt) = parity_reader
            .reconstruct_json(hour.hour_index())
            .map_err(|error| {
                OpenError::Certification(format!(
                    "binary reconstruction parity failed for hour {}: {error}",
                    hour.hour_index()
                ))
            })?;
        max_seek_deltas = max_seek_deltas.max(receipt.applied_deltas);
    }
    if max_seek_deltas >= checkpoint_interval_hours {
        return Err(OpenError::Certification(format!(
            "binary seek bound failed: observed {max_seek_deltas} delta(s) for interval {checkpoint_interval_hours}"
        )));
    }
    if replay_index.cache_len() >= legacy_json_bytes_avoided {
        return Err(OpenError::Certification(format!(
            "binary checkpoint cache is not smaller than discarded hourly JSON: {} >= {} bytes",
            replay_index.cache_len(),
            legacy_json_bytes_avoided
        )));
    }

    let production_render_blob_dir = output_dir.join("production_render_blobs");
    let mut production_render_blob_count = 0u64;
    let mut production_render_blob_bytes = 0u64;
    if production_render_blob_dir.exists() {
        for entry in fs::read_dir(&production_render_blob_dir)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if !metadata.file_type().is_file() {
                return Err(OpenError::Certification(format!(
                    "production render artifact is not a regular file: {}",
                    entry.path().display()
                )));
            }
            production_render_blob_count =
                production_render_blob_count.checked_add(1).ok_or_else(|| {
                    OpenError::Certification("render blob count overflowed u64".to_string())
                })?;
            production_render_blob_bytes = production_render_blob_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| {
                    OpenError::Certification("render blob bytes overflowed u64".to_string())
                })?;
        }
    }

    Ok(CertifiedReplayArtifacts {
        hour_index_path: index_path,
        checkpoint_cache_path: cache_path,
        hour_count: replay_index.hours().len() as u64,
        checkpoint_count: replay_index
            .hours()
            .iter()
            .filter(|hour| hour.delta_steps() == 0)
            .count() as u64,
        checkpoint_interval_hours,
        max_seek_deltas,
        source_len: expected_source_len,
        source_sha256: expected_source_sha256.to_string(),
        hour_index_len: index_bytes.len() as u64,
        hour_index_sha256: Sha256::hex(&index_bytes),
        checkpoint_cache_len: replay_index.cache_len(),
        checkpoint_cache_sha256: replay_index.cache_sha256().to_string(),
        legacy_json_bytes_avoided,
        production_render_blob_dir,
        production_render_blob_count,
        production_render_blob_bytes,
    })
}

struct SpooledFoldPass {
    files: Vec<FoldedSnapshotFile>,
    stats: FoldStreamStats,
    boundary_error: Option<String>,
    /// Retained for the entire authoritative pass. It is deliberately not
    /// serialized into the portable snapshot spool; the recorder manifest is
    /// the owner of public campaign metadata.
    campaign_identity: Option<ValidatedCid1>,
}

/// Bounded file-backed fold driver whose protocol is selected before reading.
/// CID1 is accepted and retained by the shared stateful validator, then removed
/// from ordinary boundary/count/fold dispatch. All subsequent records pass the
/// same validator before the existing exact payload and WorldFold machinery.
fn fold_compact_stream_file_identity_aware(
    mut reader: fs::File,
    game_id: &str,
    snapshot_dir: &Path,
    identity_mode: StreamIdentityMode,
) -> io::Result<SpooledFoldPass> {
    let total_len = reader.metadata()?.len();
    reader.seek(SeekFrom::Start(0))?;
    let mut fold = WorldFold::new_file_backed(game_id, snapshot_dir)?;
    let mut stats = FoldStreamStats {
        stream_bytes: total_len,
        ..FoldStreamStats::default()
    };
    let mut validator = stream_validator(identity_mode);
    let mut campaign_identity = None;
    let mut carry = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut processed = 0u64;
    let mut decoded_hasher = Sha256::new();
    let mut tracker = BoundaryTracker::default();
    let mut validation_error = None;
    let mut previous_ordinary_sequence = None;

    'read: loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        decoded_hasher.update(&chunk[..n]);
        carry.extend_from_slice(&chunk[..n]);

        let mut off = 0usize;
        let mut hard_stop = false;
        loop {
            let remaining = carry.len() - off;
            if remaining < COMPACT_RECORD_HEADER_LEN {
                break;
            }
            let record_at = processed + off as u64;
            let header = match parse_compact_record_header(&carry[off..]) {
                Ok(header) => header,
                Err(error) => {
                    stats.stop_reason = Some(framing_stop_message(error, record_at));
                    hard_stop = true;
                    break;
                }
            };
            let record_len = header.record_len();
            if record_len > remaining {
                break;
            }
            let payload = &carry[off + COMPACT_RECORD_HEADER_LEN..off + record_len];
            if identity_mode == StreamIdentityMode::Legacy
                && (validator.is_terminal()
                    || (header.sequence == 0
                        && header.domain_id == COMPACT_DOMAIN_STREAM_HEADER
                        && header.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER))
            {
                if validation_error.is_none() {
                    validation_error = Some(format!(
                        "Legacy stream contains an IdentityV1 sequence-zero CID1 header at byte {record_at}"
                    ));
                    let _ = validator.validate_record(&header, payload);
                }
                off += record_len;
                continue;
            }
            if identity_mode == StreamIdentityMode::IdentityV1 && campaign_identity.is_none() {
                match validator.validate_record(&header, payload) {
                    Ok(AcceptedRecordKind::CampaignIdentity) => {
                        campaign_identity = validator.identity().cloned();
                    }
                    Ok(AcceptedRecordKind::Ordinary) => unreachable!(
                        "IdentityV1 validator cannot accept an ordinary record before CID1"
                    ),
                    Err(error) => {
                        if validation_error.is_none() {
                            validation_error = Some(format!(
                                "stream identity validation failed at byte {record_at}: {error}"
                            ));
                        }
                    }
                }
                off += record_len;
                continue;
            }

            // Ordinary records remain diagnostic-readable after a contract
            // failure. Certification is still fail-closed, but retaining the
            // record lets the fold expose the valid prefix/hour and preserves
            // compatibility with pre-canonical Legacy containers. Do not use
            // StatefulStreamValidator here: its intentionally terminal
            // all-in-one semantics would discard this record and every record
            // after it, conflating certification with diagnostic decoding.
            let sequence_error =
                strict_sequence_error(previous_ordinary_sequence, header.sequence, record_at);
            previous_ordinary_sequence = Some(header.sequence);
            if validation_error.is_none() {
                validation_error = sequence_error.or_else(|| {
                    validate_compact_record_envelope(&header, payload)
                        .err()
                        .map(|reason| {
                            format!("record contract violation at byte {record_at}: {reason}")
                        })
                });
            }
            stats.records_total += 1;

            match header.record_type {
                COMPACT_RECORD_TYPE_DOMAIN_DATA => stats.data_records += 1,
                COMPACT_RECORD_TYPE_FRAME_END => stats.frame_end_records += 1,
                COMPACT_RECORD_TYPE_DOMAIN_PERF => stats.perf_records += 1,
                _ => stats.other_records += 1,
            }
            tracker.observe(&header, payload, record_at);

            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                if let Err(reason) =
                    validate_domain_data_payload(header.domain_id, payload, header.record_flags)
                {
                    stats.malformed_payload_records += 1;
                    fold.record_malformed_domain_payload(
                        header.game_hour_raw,
                        header.domain_id,
                        payload,
                        &reason,
                    );
                    off += record_len;
                    continue;
                }
            }
            fold.ingest_record(
                header.sequence,
                header.game_hour_raw,
                header.domain_id,
                header.record_type,
                payload,
                header.record_flags,
            );
            off += record_len;
        }
        processed += off as u64;
        carry.drain(0..off);
        if hard_stop {
            break 'read;
        }
    }

    stats.bytes_folded = processed;
    stats.truncated_tail_bytes = total_len.saturating_sub(processed);
    stats.hours_emitted = fold.emitted_hours_count() as u64;
    stats.decoded_sha256 = to_hex(&decoded_hasher.finalize());

    if identity_mode == StreamIdentityMode::IdentityV1 && campaign_identity.is_none() {
        validation_error.get_or_insert_with(|| {
            "IdentityV1 stream ended without an accepted sequence-zero CID1 header".to_string()
        });
    }
    let tracker_error = tracker.eof_check(carry.len()).err();
    let boundary_error = validation_error.or(tracker_error);
    stats.ends_at_frame_end = boundary_error.is_none();
    let files = fold
        .finish_file_backed()
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
    Ok(SpooledFoldPass {
        files,
        stats,
        boundary_error,
        campaign_identity,
    })
}

/// Bounded production adapter: authenticate the exact raw spool while driving
/// the existing file-backed `WorldFold`, then return only a compact manifest of
/// per-hour files. Neither the H4CR stream nor all snapshots are retained in
/// memory.
pub fn fold_raw_h4cr_file_spooled(
    request: snapshot::CertifiedPrefoldSpoolRequest,
) -> Result<snapshot::PrefoldedSpool, OpenError> {
    let identity_mode = compact_identity_mode(request.identity_mode());
    fold_raw_h4cr_file_spooled_with_identity_mode(request, identity_mode)
}

/// [`fold_raw_h4cr_file_spooled`] under an explicit stream identity mode.
/// The mode must agree with the request's authenticated protocol declaration.
pub fn fold_raw_h4cr_file_spooled_with_identity_mode(
    request: snapshot::CertifiedPrefoldSpoolRequest,
    identity_mode: StreamIdentityMode,
) -> Result<snapshot::PrefoldedSpool, OpenError> {
    let raw_path = request.raw_h4cr_path().to_path_buf();
    let source_meta = fs::symlink_metadata(&raw_path)?;
    if !source_meta.file_type().is_file() || source_meta.file_type().is_symlink() {
        return Err(OpenError::Certification(format!(
            "raw H4CR spool is not a regular non-symlink file: {}",
            raw_path.display()
        )));
    }
    let source = fs::File::open(&raw_path)?;
    fold_raw_h4cr_open_file_spooled_with_identity_mode(request, source, identity_mode)
}

/// Fold a certified compact source from an already-open file capability.
///
/// Unlike [`fold_raw_h4cr_file_spooled`], this entry point never resolves the
/// request's source pathname. The supplied file is authenticated and consumed
/// from the same open object, allowing a process boundary to transfer a
/// retained, deny-write/delete source handle without reintroducing a pathname
/// substitution race. `CertifiedPrefoldSpoolRequest` remains the portable
/// receipt and policy contract.
pub fn fold_raw_h4cr_open_file_spooled(
    request: snapshot::CertifiedPrefoldSpoolRequest,
    source: fs::File,
) -> Result<snapshot::PrefoldedSpool, OpenError> {
    let identity_mode = compact_identity_mode(request.identity_mode());
    fold_raw_h4cr_open_file_spooled_with_identity_mode(request, source, identity_mode)
}

fn fold_raw_h4cr_open_file_spooled_with_identity_mode(
    request: snapshot::CertifiedPrefoldSpoolRequest,
    mut source: fs::File,
    identity_mode: StreamIdentityMode,
) -> Result<snapshot::PrefoldedSpool, OpenError> {
    if compact_identity_mode(request.identity_mode()) != identity_mode {
        return Err(OpenError::Certification(
            "spool prefold request identity mode disagrees with decoder mode".to_string(),
        ));
    }
    let (game_id, raw_path, snapshot_dir, expected_len, expected_sha256) = request.into_parts();
    let source_meta = source.metadata()?;
    if !source_meta.file_type().is_file() || source_meta.file_type().is_symlink() {
        return Err(OpenError::Certification(format!(
            "opened raw H4CR source is not a regular file: {}",
            raw_path.display()
        )));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if source_meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(OpenError::Certification(format!(
                "opened raw H4CR source is a reparse point: {}",
                raw_path.display()
            )));
        }
    }
    if source_meta.len() != expected_len {
        return Err(OpenError::Certification(format!(
            "raw H4CR spool length {} != authenticated source length {expected_len}",
            source_meta.len()
        )));
    }
    source.seek(SeekFrom::Start(0))?;
    let mut magic = [0u8; 4];
    source.read_exact(&mut magic)?;
    if magic != *compact_contract::COMPACT_RECORD_MAGIC {
        return Err(OpenError::UnrecognizedInput(format!(
            "{} is not a physically raw H4CR stream",
            raw_path.display()
        )));
    }
    source.seek(SeekFrom::Start(0))?;

    let output_meta = fs::symlink_metadata(&snapshot_dir)?;
    if !output_meta.file_type().is_dir() || output_meta.file_type().is_symlink() {
        return Err(OpenError::Certification(format!(
            "snapshot spool is not a regular non-symlink directory: {}",
            snapshot_dir.display()
        )));
    }
    if fs::read_dir(&snapshot_dir)?.next().is_some() {
        return Err(OpenError::Certification(format!(
            "snapshot spool directory is not empty: {}",
            snapshot_dir.display()
        )));
    }

    let canonical_snapshot_dir = fs::canonicalize(&snapshot_dir)?;
    let SpooledFoldPass {
        files,
        stats,
        boundary_error,
        campaign_identity: _,
    } = fold_compact_stream_file_identity_aware(source, &game_id, &snapshot_dir, identity_mode)?;
    strict_stream_certification_verdict(
        stats.stop_reason.as_deref(),
        stats.truncated_tail_bytes,
        stats.malformed_payload_records,
        stats.ends_at_frame_end,
        boundary_error.as_deref(),
    )
    .map_err(OpenError::Certification)?;
    if stats.stream_bytes != expected_len || stats.decoded_sha256 != expected_sha256 {
        return Err(OpenError::Certification(format!(
            "folded H4CR source identity mismatch: bytes {} vs {expected_len}, sha256 {} vs {expected_sha256}",
            stats.stream_bytes, stats.decoded_sha256
        )));
    }
    if stats.frame_end_records != stats.hours_emitted || stats.hours_emitted != files.len() as u64 {
        return Err(OpenError::Certification(format!(
            "closed-hour cardinality mismatch: {} FRAME_END record(s), {} WorldFold emission(s), {} spilled snapshot(s)",
            stats.frame_end_records,
            stats.hours_emitted,
            files.len()
        )));
    }

    let mut entries = Vec::with_capacity(files.len());
    let mut expected_files = std::collections::BTreeSet::new();
    for file in files {
        let canonical_path = fs::canonicalize(&file.path)?;
        if canonical_path.parent() != Some(canonical_snapshot_dir.as_path()) {
            return Err(OpenError::Certification(format!(
                "WorldFold snapshot escaped private spool: {}",
                file.path.display()
            )));
        }
        let meta = fs::symlink_metadata(&file.path)?;
        if !meta.file_type().is_file()
            || meta.file_type().is_symlink()
            || meta.len() != file.byte_len as u64
        {
            return Err(OpenError::Certification(format!(
                "WorldFold snapshot identity changed after spill: {}",
                file.path.display()
            )));
        }
        let file_name = file
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                OpenError::Certification(format!(
                    "WorldFold emitted a non-portable snapshot name: {}",
                    file.path.display()
                ))
            })?
            .to_string();
        if !expected_files.insert(file_name.clone()) {
            return Err(OpenError::Certification(format!(
                "WorldFold emitted duplicate snapshot file {file_name:?}"
            )));
        }
        let sha256 = file.sha256.ok_or_else(|| {
            OpenError::Certification(format!(
                "WorldFold omitted snapshot SHA-256 for {file_name:?}"
            ))
        })?;
        let bytes = fs::read(&file.path)?;
        let observed_sha256 = to_hex(&Sha256::digest(&bytes));
        if bytes.len() as u64 != file.byte_len as u64 || observed_sha256 != sha256 {
            return Err(OpenError::Certification(format!(
                "WorldFold snapshot bytes changed after authenticated spill: {file_name:?}"
            )));
        }
        let parsed = snapshot::PrefoldedHourEntry::from_json(file.hour_index, &bytes)
            .map_err(|error| OpenError::Certification(error.to_string()))?;
        match parsed.snapshot().game_id.as_deref() {
            Some(body_game_id) if body_game_id == game_id => {}
            other => {
                return Err(OpenError::Certification(format!(
                    "spooled hour {} game_id {:?} != request {:?}",
                    file.hour_index, other, game_id
                )))
            }
        }
        if parsed.snapshot().schema_version.as_ref()
            != Some(&serde_json::Value::String(schema_version().to_string()))
        {
            return Err(OpenError::Certification(format!(
                "spooled hour {} schema_version does not match production schema {}",
                file.hour_index,
                schema_version()
            )));
        }
        entries.push(
            snapshot::PrefoldedSpoolEntry::try_new(
                file.hour_index,
                file.date,
                file_name,
                file.byte_len as u64,
                sha256,
            )
            .map_err(|error| OpenError::Certification(error.to_string()))?,
        );
    }
    let observed_files = fs::read_dir(&snapshot_dir)?
        .map(|entry| {
            let entry = entry?;
            entry.file_name().into_string().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "snapshot spool contains a non-Unicode file name",
                )
            })
        })
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    if observed_files != expected_files {
        return Err(OpenError::Certification(
            "snapshot spool contains files not emitted by the certified WorldFold".to_string(),
        ));
    }

    snapshot::PrefoldedSpool::try_new(
        game_id,
        serde_json::Value::String(schema_version().to_string()),
        entries,
    )
    .map_err(|error| OpenError::Certification(error.to_string()))
}

/// Fully fold and certify verified, physically raw H4CR bytes transferred by a
/// portable container reader.
///
/// Unlike the pull cursor, this function exposes no prefix. It drains through
/// physical EOF, applies the exact full-stream certification verdict, checks
/// declared-vs-consumed length, and constructs the neutral result only after
/// those gates pass.
pub fn fold_raw_h4cr_bytes_prefolded(
    raw_h4cr: Vec<u8>,
    game_id: impl Into<String>,
) -> Result<snapshot::PrefoldedReplay, OpenError> {
    fold_raw_h4cr_bytes_prefolded_with_identity_mode(raw_h4cr, game_id, StreamIdentityMode::Legacy)
}

/// [`fold_raw_h4cr_bytes_prefolded`] under an explicit, non-sniffed identity mode.
pub fn fold_raw_h4cr_bytes_prefolded_with_identity_mode(
    raw_h4cr: Vec<u8>,
    game_id: impl Into<String>,
    identity_mode: StreamIdentityMode,
) -> Result<snapshot::PrefoldedReplay, OpenError> {
    if !raw_h4cr.starts_with(compact_contract::COMPACT_RECORD_MAGIC) {
        return Err(OpenError::UnrecognizedInput(
            "physically raw H4CR input required at the portable prefold boundary".to_string(),
        ));
    }
    let total_len = raw_h4cr.len() as u64;
    let game_id = game_id.into();
    let mut cursor = HourCursor::from_reader(
        Box::new(Cursor::new(raw_h4cr)),
        total_len,
        game_id.clone(),
        identity_mode,
    );
    let schema_version = serde_json::Value::String(cursor.metadata().schema_version.clone());
    let mut folded = Vec::new();
    while let Some(hour) = cursor.next_hour() {
        folded.push(hour.map_err(|error| match error {
            DecodeError::Io(error) => OpenError::Io(error),
        })?);
    }
    validate_prefold_cardinality(&cursor, folded.len())?;
    cursor
        .certify_complete()
        .map_err(|error| OpenError::Certification(error.to_string()))?;

    let entries = folded
        .into_iter()
        .map(|hour| snapshot::PrefoldedHourEntry::from_json(hour.hour_index, &hour.snapshot))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            OpenError::Certification(format!(
                "certified fold did not satisfy the neutral prefolded contract: {error}"
            ))
        })?;
    snapshot::PrefoldedReplay::try_new(game_id, schema_version, entries).map_err(|error| {
        OpenError::Certification(format!(
            "certified fold did not satisfy the neutral prefolded contract: {error}"
        ))
    })
}

fn validate_prefold_cardinality(cursor: &HourCursor, folded_len: usize) -> Result<(), OpenError> {
    let stats = cursor.stats();
    let folded_len = folded_len as u64;
    if stats.frame_end_records != stats.hours_emitted || stats.hours_emitted != folded_len {
        return Err(OpenError::Certification(format!(
            "closed-hour cardinality mismatch: {} FRAME_END record(s), {} WorldFold emission(s), {folded_len} retained snapshot(s)",
            stats.frame_end_records, stats.hours_emitted
        )));
    }
    Ok(())
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// Open a `.battleplan`, verify it, and return `(stream bytes, manifest game_id)`
/// for the single `collector_compact_stream` entry (integrity-checked against
/// the manifest). Materializes the entry in RAM (subject to the container
/// reader's per-entry cap) — see the crate-level memory note.
fn extract_container_stream(path: &Path) -> Result<(Vec<u8>, String), OpenError> {
    let container = Container::open(path, &ExpectedIdentity::default())
        .map_err(|e| OpenError::Container(format!("open {}: {e}", path.display())))?;
    let game_id = container.game_id().to_string();
    let entries = container.entries_by_role(STREAM_ROLE);
    let bytes = match entries.len() {
        0 => {
            return Err(OpenError::Container(format!(
                "{} has no entry with role {STREAM_ROLE:?}",
                path.display()
            )))
        }
        1 => {
            let expected_prefix = format!("payload/{game_id}/");
            if !entries[0].path.starts_with(&expected_prefix) {
                return Err(OpenError::Container(format!(
                    "stream role entry {:?} is outside manifest game namespace {expected_prefix:?}",
                    entries[0].path
                )));
            }
            container
                .read_entry_bytes(&entries[0].path)
                .map_err(|e| OpenError::Container(format!("read {}: {e}", entries[0].path)))?
        }
        n => {
            return Err(OpenError::Container(format!(
                "{} has {n} entries with role {STREAM_ROLE:?}; expected exactly one",
                path.display()
            )))
        }
    };
    Ok((bytes, game_id))
}

// ---------------------------------------------------------------------------
// The pull-based cursor
// ---------------------------------------------------------------------------

/// A pull-based cursor over a compact stream that yields one [`WorldHour`] per
/// game-hour as its `FRAME_END` closes, in bounded memory.
///
/// Drive it with [`HourCursor::next_hour`] or via the [`Iterator`] impl. After
/// iteration completes, [`HourCursor::stats`] holds the pass totals and
/// [`HourCursor::take_index`] the built [`StreamIndex`].
pub struct HourCursor {
    reader: Box<dyn Read>,
    fold: WorldFold,
    identity_mode: StreamIdentityMode,
    stream_validator: StatefulStreamValidator,
    campaign_identity: Option<ValidatedCid1>,
    /// Buffered, not-yet-consumed bytes. `carry[pos..]` is unframed; `carry[..pos]`
    /// is framed-but-not-yet-drained (drained lazily before each read). Bounded by
    /// ~`2 * CHUNK + largest record`, independent of stream length.
    carry: Vec<u8>,
    /// Framing cursor within `carry`.
    pos: usize,
    chunk: Vec<u8>,
    /// Absolute decoded-stream offset of `carry[0]` (bytes already drained).
    processed: u64,
    /// Total decoded bytes pulled from the reader (needed when zstd hides length).
    decoded_read: u64,
    total_len: u64,
    ready: VecDeque<WorldHour>,
    /// DOMAIN_DATA counts since the last FRAME_END (the domains feeding the hour
    /// currently being assembled).
    cur_counts: BTreeMap<u16, u64>,
    stats: DecodeStats,
    meta: StreamMeta,
    /// EOF or a hard framing stop reached; no more records will be produced.
    done: bool,
    /// Underlying reader is a checksummed zstd stream (see `from_reader_compressed`).
    compressed: bool,
    // --- index building (free during a normal pass) ---
    /// Decoded offset of the first record of the hour currently being assembled.
    cur_hour_start: u64,
    index_hours: Vec<HourOffset>,
    first_hour_seen: bool,
    /// The same hour/sequence/boundary state machine used by the production
    /// streaming fold and seal-time verifier.
    boundary_tracker: BoundaryTracker,
    /// Decoder-owned strict certification state. The production tracker is
    /// intentionally tolerant of legacy streams; extractor certification is
    /// stricter: sequence starts at one and never wraps and every FRAME_END
    /// closes an open DATA hour. The terminal-boundary rule is NOT here: it is
    /// asked of `BoundaryTracker::eof_check`, which is the one place that knows
    /// a canonical FRAME_REFUSED closes a stream as legitimately as a FRAME_END.
    strict_previous_sequence: Option<u64>,
    strict_open_hour: Option<i32>,
    strict_validation_error: Option<String>,
    /// Digest of every decoded source byte pulled from the reader. It is only
    /// published after EOF and strict certification, never for a valid prefix.
    source_hasher: Sha256,
    /// Stable re-openable source identity. Reader-only cursors deliberately
    /// lack one and therefore cannot mint a persisted authoritative index.
    index_source: Option<IndexSourceDescriptor>,
}

#[derive(Debug, Clone)]
struct IndexSourceDescriptor {
    kind: String,
    canonical_path: String,
}

impl HourCursor {
    fn from_reader(
        reader: Box<dyn Read>,
        total_len: u64,
        game_id: String,
        identity_mode: StreamIdentityMode,
    ) -> HourCursor {
        Self::from_reader_compressed(reader, total_len, game_id, false, identity_mode)
    }

    fn from_reader_with_source(
        reader: Box<dyn Read>,
        total_len: u64,
        game_id: String,
        identity_mode: StreamIdentityMode,
        index_source: Option<IndexSourceDescriptor>,
    ) -> HourCursor {
        Self::from_reader_compressed_with_source(
            reader,
            total_len,
            game_id,
            false,
            identity_mode,
            index_source,
        )
    }

    /// `compressed` = the underlying reader is a checksummed zstd stream. On such
    /// a source an H4CR framing stop BEFORE physical EOF must not end gracefully
    /// (the zstd footer + content checksum would never be validated); the cursor
    /// drains to EOF to force that check and hard-errors. Raw sources keep the
    /// tolerant torn-tail behavior the deep-war corpus relies on.
    fn from_reader_compressed(
        reader: Box<dyn Read>,
        total_len: u64,
        game_id: String,
        compressed: bool,
        identity_mode: StreamIdentityMode,
    ) -> HourCursor {
        Self::from_reader_compressed_with_source(
            reader,
            total_len,
            game_id,
            compressed,
            identity_mode,
            None,
        )
    }

    fn from_reader_compressed_with_source(
        reader: Box<dyn Read>,
        total_len: u64,
        game_id: String,
        compressed: bool,
        identity_mode: StreamIdentityMode,
        index_source: Option<IndexSourceDescriptor>,
    ) -> HourCursor {
        HourCursor {
            reader,
            fold: WorldFold::new(game_id.clone()),
            identity_mode,
            stream_validator: stream_validator(identity_mode),
            campaign_identity: None,
            carry: Vec::with_capacity(CHUNK * 2),
            pos: 0,
            chunk: vec![0u8; CHUNK],
            processed: 0,
            decoded_read: 0,
            total_len,
            ready: VecDeque::new(),
            cur_counts: BTreeMap::new(),
            stats: DecodeStats {
                stream_bytes: total_len,
                ..DecodeStats::default()
            },
            meta: StreamMeta::new(game_id),
            done: false,
            compressed,
            cur_hour_start: 0,
            index_hours: Vec::new(),
            first_hour_seen: false,
            boundary_tracker: BoundaryTracker::default(),
            strict_previous_sequence: None,
            strict_open_hour: None,
            strict_validation_error: None,
            source_hasher: Sha256::new(),
            index_source,
        }
    }

    /// Drain the underlying reader to physical EOF, discarding decoded bytes but
    /// forcing a checksummed zstd source to validate its final frame footer. Any
    /// storage-layer corruption surfaces here as the error it should have been.
    fn drain_verify(&mut self) -> Result<u64, DecodeError> {
        let mut sink = [0u8; 64 * 1024];
        let mut residual = 0u64;
        loop {
            match self.reader.read(&mut sink).map_err(DecodeError::Io)? {
                0 => return Ok(residual),
                n => {
                    self.source_hasher.update(&sink[..n]);
                    self.decoded_read = self.decoded_read.saturating_add(n as u64);
                    residual += n as u64;
                }
            }
        }
    }

    /// Session/stream metadata (schema version, game id, domain registry).
    pub fn metadata(&self) -> &StreamMeta {
        &self.meta
    }

    /// Explicit protocol selected before this cursor accepted any bytes.
    pub fn identity_mode(&self) -> StreamIdentityMode {
        self.identity_mode
    }

    /// Placement-authoritative CID1 retained after the sequence-zero header is
    /// accepted. Legacy cursors and pre-header IdentityV1 cursors return `None`.
    pub fn campaign_identity(&self) -> Option<&ValidatedCid1> {
        self.campaign_identity.as_ref()
    }

    /// Pass totals. Meaningful during iteration; final once it returns `None`.
    pub fn stats(&self) -> &DecodeStats {
        &self.stats
    }

    /// Number of finished hours currently buffered but not yet pulled. This is
    /// the cursor's retention: it is at most the hours closed by one
    /// `FRAME_END` (normally 0 or 1), never the whole stream — the structural
    /// bounded-memory guarantee.
    pub fn buffered_hours(&self) -> usize {
        self.ready.len()
    }

    /// Pull the next game-hour, or `None` at end of stream. A malformed record
    /// ends iteration gracefully (see [`DecodeStats::stop_reason`]); only an I/O
    /// error surfaces as `Err`.
    pub fn next_hour(&mut self) -> Option<Result<WorldHour, DecodeError>> {
        loop {
            if let Some(h) = self.ready.pop_front() {
                return Some(Ok(h));
            }
            if self.done {
                return None;
            }
            match self.pump() {
                Ok(()) => {
                    // `pump` either produced ready hours, advanced, or set done.
                    if self.ready.is_empty() && self.done {
                        return None;
                    }
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }

    /// Consume this cursor and return a source-bound [`StreamIndex`]. The cursor
    /// must have reached physical EOF, passed strict stream certification, and
    /// have a stable re-openable source identity. Prefix/reader-only indexes are
    /// intentionally unavailable through this authoritative API.
    pub fn take_index(self) -> Result<StreamIndex, IndexCertificationError> {
        self.validate_complete()?;
        let descriptor = self
            .index_source
            .ok_or(IndexCertificationError::UnidentifiedSource)?;
        let index = StreamIndex {
            source: CertifiedSourceIdentity {
                kind: descriptor.kind,
                canonical_path: descriptor.canonical_path,
                decoded_len: self.decoded_read,
                decoded_sha256: to_hex(&self.source_hasher.finalize()),
            },
            game_id: self.meta.game_id,
            identity_mode: self.identity_mode,
            hours: self.index_hours,
        };
        index
            .validate_structure()
            .map_err(IndexCertificationError::InvalidIndex)?;
        Ok(index)
    }

    /// Consume a reader-only cursor after proving that it reached and certified
    /// physical EOF. Unlike [`HourCursor::take_index`], this does not mint a
    /// persisted seek index and therefore needs no filesystem source identity.
    pub fn certify_complete(self) -> Result<(), IndexCertificationError> {
        self.validate_complete()
    }

    fn validate_complete(&self) -> Result<(), IndexCertificationError> {
        if !self.done {
            return Err(IndexCertificationError::NotExhausted);
        }
        self.stats
            .certification_verdict()
            .map_err(IndexCertificationError::Stream)?;
        if self.processed != self.decoded_read {
            return Err(IndexCertificationError::Length(format!(
                "framed {} byte(s), but read {} decoded source byte(s)",
                self.processed, self.decoded_read
            )));
        }
        if self.total_len != 0 && self.total_len != self.decoded_read {
            return Err(IndexCertificationError::Length(format!(
                "declared decoded length {} does not match {} byte(s) read",
                self.total_len, self.decoded_read
            )));
        }
        Ok(())
    }

    /// Advance framing until at least one hour is queued, EOF, or a hard framing
    /// stop. Frames records one at a time from `carry[pos..]`, reading more only
    /// when the front lacks a complete record, and **pauses the moment a
    /// `FRAME_END` queues a hour** — so at most the hours of one frame (normally
    /// one) are ever buffered. This is the pull-based bounded-memory guarantee.
    fn pump(&mut self) -> Result<(), DecodeError> {
        loop {
            // Frame every complete record currently buffered, pausing on the
            // first frame that produces a ready hour.
            while self.carry.len() - self.pos >= COMPACT_RECORD_HEADER_LEN {
                let remaining = self.carry.len() - self.pos;
                let record_start_abs = self.processed + self.pos as u64;
                let header = match parse_compact_record_header(&self.carry[self.pos..]) {
                    Ok(h) => h,
                    Err(e) => {
                        self.stats.stop_reason = Some(framing_stop_message(e, record_start_abs));
                        if self.compressed {
                            // A framing stop before physical EOF means the zstd
                            // footer + content checksum were never validated.
                            // Drain to EOF to force that check: corruption of the
                            // decoded content fails the checksum here; if the
                            // storage layer is intact, the framing stop itself is
                            // a hard error (valid-checksum bytes that are
                            // H4CR-invalid = capture-time garbage).
                            let residual = self.drain_verify()?;
                            self.finalize_done();
                            return Err(DecodeError::Io(compact_stream::framing_stop_error(
                                residual,
                            )));
                        }
                        self.finalize_done();
                        return Ok(());
                    }
                };
                let record_len = header.record_len();
                if record_len > remaining {
                    break; // payload not fully buffered yet — read more
                }
                if self.identity_mode == StreamIdentityMode::Legacy
                    && (self.stream_validator.is_terminal()
                        || (header.sequence == 0
                            && header.domain_id == COMPACT_DOMAIN_STREAM_HEADER
                            && header.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER))
                {
                    if self.strict_validation_error.is_none() {
                        self.strict_validation_error = Some(format!(
                            "Legacy stream contains an IdentityV1 sequence-zero CID1 header at byte {record_start_abs}"
                        ));
                        let payload = &self.carry
                            [self.pos + COMPACT_RECORD_HEADER_LEN..self.pos + record_len];
                        let _ = self.stream_validator.validate_record(&header, payload);
                    }
                    self.pos += record_len;
                    continue;
                }
                if self.identity_mode == StreamIdentityMode::IdentityV1
                    && self.campaign_identity.is_none()
                {
                    let accepted = {
                        let payload = &self.carry
                            [self.pos + COMPACT_RECORD_HEADER_LEN..self.pos + record_len];
                        self.stream_validator.validate_record(&header, payload)
                    };
                    match accepted {
                        Ok(AcceptedRecordKind::CampaignIdentity) => {
                            self.campaign_identity = self.stream_validator.identity().cloned();
                        }
                        Ok(AcceptedRecordKind::Ordinary) => unreachable!(
                            "IdentityV1 validator cannot accept an ordinary record before CID1"
                        ),
                        Err(error) => {
                            if self.strict_validation_error.is_none() {
                                self.strict_validation_error = Some(format!(
                                    "stream identity validation failed at byte {record_start_abs}: {error}"
                                ));
                            }
                        }
                    }
                    self.pos += record_len;
                    continue;
                }
                self.stats.records_total += 1;
                self.observe_strict_header(&header, record_start_abs);
                let payload =
                    &self.carry[self.pos + COMPACT_RECORD_HEADER_LEN..self.pos + record_len];

                let envelope_error = validate_compact_record_envelope(&header, payload)
                    .err()
                    .map(|reason| {
                        format!("record contract violation at byte {record_start_abs}: {reason}")
                    });
                if self.strict_validation_error.is_none() {
                    self.strict_validation_error = envelope_error.clone();
                }
                let exact_payload_error = match (header.record_type, envelope_error.as_ref()) {
                    (COMPACT_RECORD_TYPE_DOMAIN_DATA, None) => {
                        validate_domain_data_payload(header.domain_id, payload, header.record_flags)
                            .err()
                    }
                    _ => None,
                };
                let malformed_reason = (header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA)
                    .then(|| {
                        envelope_error
                            .clone()
                            .or_else(|| exact_payload_error.clone())
                    })
                    .flatten();

                match header.record_type {
                    COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                        self.stats.data_records += 1;
                        *self
                            .stats
                            .per_domain_data_records
                            .entry(header.domain_id)
                            .or_default() += 1;
                        *self.cur_counts.entry(header.domain_id).or_default() += 1;
                    }
                    COMPACT_RECORD_TYPE_FRAME_END => self.stats.frame_end_records += 1,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF => self.stats.perf_records += 1,
                    _ => self.stats.other_records += 1,
                }
                self.boundary_tracker
                    .observe(&header, payload, record_start_abs);

                if let Some(reason) = malformed_reason {
                    if exact_payload_error.is_some() {
                        self.stats.malformed_payload_records += 1;
                    }
                    self.fold.record_malformed_domain_payload(
                        header.game_hour_raw,
                        header.domain_id,
                        payload,
                        &reason,
                    );
                } else {
                    // The single production fold. PERF / probe records carry no
                    // folded state and are ignored inside `ingest_record`.
                    self.fold.ingest_record(
                        header.sequence,
                        header.game_hour_raw,
                        header.domain_id,
                        header.record_type,
                        payload,
                        header.record_flags,
                    );
                }
                self.pos += record_len;

                if header.record_type == COMPACT_RECORD_TYPE_FRAME_END {
                    let next_hour_start = record_start_abs + record_len as u64;
                    self.drain_finished_hours(next_hour_start);
                    if !self.ready.is_empty() {
                        return Ok(()); // pause: let the caller pull this hour
                    }
                } else if header.record_type == COMPACT_RECORD_TYPE_FRAME_REFUSED {
                    self.close_refused_hour();
                }
            }

            // Front of carry has no complete record: drop the framed prefix and
            // read the next chunk.
            self.compact_carry();
            let n = self.reader.read(&mut self.chunk).map_err(DecodeError::Io)?;
            if n == 0 {
                self.finalize_done();
                return Ok(());
            }
            self.decoded_read = self.decoded_read.saturating_add(n as u64);
            self.source_hasher.update(&self.chunk[..n]);
            self.carry.extend_from_slice(&self.chunk[..n]);
        }
    }

    /// Drop the framed prefix `carry[..pos]`, advancing `processed`, so the
    /// buffer stays bounded across a long stream.
    fn compact_carry(&mut self) {
        if self.pos > 0 {
            self.processed += self.pos as u64;
            self.carry.drain(0..self.pos);
            self.pos = 0;
        }
    }

    /// Record the final byte/hour accounting and mark the cursor exhausted.
    fn finalize_done(&mut self) {
        self.compact_carry();
        if self.total_len == 0 {
            self.stats.stream_bytes = self.decoded_read;
        }
        self.stats.bytes_folded = self.processed;
        self.stats.truncated_tail_bytes = self.stats.stream_bytes.saturating_sub(self.processed);
        self.stats.hours_emitted = self.fold.emitted_hours_count() as u64;
        let shared_boundary_error = self.boundary_tracker.eof_check(self.carry.len()).err();
        if self.strict_validation_error.is_none()
            && self.identity_mode == StreamIdentityMode::IdentityV1
            && self.campaign_identity.is_none()
        {
            self.strict_validation_error = Some(
                "IdentityV1 stream ended without an accepted sequence-zero CID1 header".to_string(),
            );
        }
        if self.strict_validation_error.is_none() {
            // The terminal-boundary rule is asked of `BoundaryTracker::eof_check`
            // and NOWHERE else. This used to carry a second copy of it, keyed on
            // this cursor's own last-record-type field, that admitted only
            // FRAME_END. When the shared rule was widened to accept a canonical
            // FRAME_REFUSED -- a refused hour IS closed, it simply closed
            // without publishing -- the copy here was not, so this crate's
            // cursor path refused every honestly-refused stream while its own
            // spooled path (which only ever consulted the tracker) accepted the
            // same bytes. The tracker subsumes both arms of the copy: a
            // non-terminal final record is "stream ends after record type N",
            // and a stream with no records is "stream closes no game hour".
            self.strict_validation_error = shared_boundary_error;
        }
        self.stats.boundary_error = self.strict_validation_error.take();
        self.stats.ends_at_frame_end = self.stats.boundary_error.is_none();
        self.done = true;
    }

    fn observe_strict_header(&mut self, header: &CompactRecordHeader, at: u64) {
        let sequence_error =
            strict_sequence_error(self.strict_previous_sequence, header.sequence, at);
        self.strict_previous_sequence = Some(header.sequence);

        let boundary_error = match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_DATA => match self.strict_open_hour {
                Some(open) if open != header.game_hour_raw => Some(format!(
                    "hour interleaving at byte {at}: DOMAIN_DATA for hour {} while hour {open} is open",
                    header.game_hour_raw
                )),
                None => {
                    self.strict_open_hour = Some(header.game_hour_raw);
                    None
                }
                Some(_) => None,
            },
            COMPACT_RECORD_TYPE_FRAME_END => match self.strict_open_hour {
                Some(open) if open == header.game_hour_raw => {
                    self.strict_open_hour = None;
                    None
                }
                Some(open) => Some(format!(
                    "mismatched FRAME_END at byte {at}: closes hour {} while hour {open} is open",
                    header.game_hour_raw
                )),
                None => Some(format!(
                    "FRAME_END at byte {at} closes hour {} without an open DOMAIN_DATA hour",
                    header.game_hour_raw
                )),
            },
            // A canonical FRAME_REFUSED closes the open hour too -- it simply
            // closed without publishing. This arm only records THAT the hour is
            // closed; whether the refusal was entitled to close it (it may only
            // refuse the hour that is open, and a refusal over no open hour is
            // the ordinary shape) is the shared `BoundaryTracker`'s rule and is
            // asked there and nowhere else, exactly as the terminal-boundary
            // rule is. A duplicate of a boundary rule on this path is what made
            // this cursor refuse every honestly-refused stream once already.
            //
            // Without this arm the hour stayed open in the strict machine and
            // the FIRST DOMAIN_DATA of the next hour was reported as "hour
            // interleaving", so a refusal followed by any further hour could
            // not certify at all.
            COMPACT_RECORD_TYPE_FRAME_REFUSED => {
                self.strict_open_hour = None;
                None
            }
            _ => None,
        };
        if self.strict_validation_error.is_none() {
            self.strict_validation_error = sequence_error.or(boundary_error);
        }
    }

    /// Close a hour that a canonical `FRAME_REFUSED` ended without publishing.
    ///
    /// A refusal is a real hour boundary: it ends the window over which
    /// DOMAIN_DATA records are attributed to one hour. Only `FRAME_END` used to
    /// reach any reset, so a refused hour's tallies stayed in `cur_counts` and
    /// were handed to the NEXT published hour's
    /// [`WorldHour::domain_data_counts`] -- a stream in which the next hour
    /// carried one d18 record reported `{18: 2}`. The refused hour published no
    /// `WorldHour`, so there is nothing to attribute its counts to and they are
    /// dropped. `DecodeStats::per_domain_data_records` is a whole-stream total
    /// and deliberately still counts them: those records were in the stream.
    ///
    /// `cur_hour_start` is deliberately NOT advanced here, and that is not an
    /// oversight. It is the start of the byte range whose replay reproduces the
    /// next published hour, which is what
    /// [`certified binary artifacts`](CertifiedBinaryArtifacts) turn into each
    /// hour's `h4cr_start_offset`/`h4cr_end_offset`. The refused hour's
    /// DOMAIN_DATA records were folded into live state before the refusal, so
    /// that state is part of what the next published hour is made of. Advancing
    /// past them would leave those bytes inside no hour's range and hand a
    /// reconstruction a checkpoint range that replays to a DIFFERENT hour than
    /// the one it names.
    fn close_refused_hour(&mut self) {
        self.cur_counts.clear();
    }

    /// Drain the hours the fold just closed on this `FRAME_END`, attaching the
    /// per-domain counts and recording their index offsets, then reset the
    /// per-hour count accumulator and advance the next hour's start offset.
    fn drain_finished_hours(&mut self, next_hour_start: u64) {
        let counts = std::mem::take(&mut self.cur_counts);
        let finished = self.fold.take_finished();
        for (hour_index, date, bytes) in finished {
            let cheap = !self.first_hour_seen;
            self.first_hour_seen = true;
            self.index_hours.push(HourOffset {
                hour_index,
                start_offset: self.cur_hour_start,
                cheap_seek_point: cheap,
            });
            self.stats.hours_emitted += 1;
            self.ready.push_back(WorldHour {
                hour_index,
                date,
                snapshot: bytes,
                // All hours closed by one FRAME_END share the counts accumulated
                // for it; normally exactly one hour closes per FRAME_END.
                domain_data_counts: counts.clone(),
            });
        }
        self.cur_hour_start = next_hour_start;
    }
}

impl Iterator for HourCursor {
    type Item = Result<WorldHour, DecodeError>;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_hour()
    }
}

fn framing_stop_message(e: RecordFramingError, at: u64) -> String {
    match e {
        RecordFramingError::TruncatedHeader(actual) => {
            format!("truncated record header ({actual} byte(s)) at byte {at}")
        }
        RecordFramingError::BadMagic => format!("invalid record magic at byte {at}"),
        RecordFramingError::UnsupportedVersion(v) => {
            format!("unsupported record version {v} at byte {at}")
        }
        RecordFramingError::UnsupportedHeaderLen(l) => {
            format!("unsupported header length {l} at byte {at}")
        }
        RecordFramingError::PayloadTooLarge(l) => {
            format!("payload length {l} exceeds the record bound at byte {at}")
        }
    }
}

fn strict_sequence_error(previous: Option<u64>, current: u64, at: u64) -> Option<String> {
    match previous {
        None if current != 1 => Some(format!(
            "record sequence at byte {at} starts at {current}, expected 1"
        )),
        Some(previous) => match previous.checked_add(1) {
            Some(expected) if current != expected => Some(format!(
                "sequence discontinuity at byte {at}: {previous} -> {current} (expected {expected})"
            )),
            None => Some(format!(
                "sequence overflow at byte {at}: {previous} cannot be followed by {current}"
            )),
            _ => None,
        },
        _ => None,
    }
}

/// Validate one fully framed compact record's canonical envelope semantics.
///
/// This check is deliberately performed before any DOMAIN_DATA decoder runs.
/// It rejects record-header flag bits that are reserved or misplaced, requires
/// an exact 40-byte DOMAIN_PERF base payload unless its flag announces a valid
/// exact `PDU1` diagnostic trailer, and requires FRAME_END to use domain 1000,
/// a 32-byte payload, and no flags. Logical cross-record sequencing/hour closure
/// remains the cursor's responsibility. Envelope semantics delegate to the
/// dependency-free `compact_contract` source of truth.
pub fn validate_compact_record_envelope(
    header: &CompactRecordHeader,
    payload: &[u8],
) -> Result<(), String> {
    if payload.len() != header.payload_len as usize {
        return Err(format!(
            "payload slice length {} does not match header payload length {}",
            payload.len(),
            header.payload_len
        ));
    }
    validate_compact_record_semantics(
        header.domain_id,
        header.record_type,
        header.record_flags,
        payload,
    )
}

// ---------------------------------------------------------------------------
// Random-access index + seek
// ---------------------------------------------------------------------------

/// One emitted game-hour's location in the decoded stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourOffset {
    pub hour_index: i64,
    /// Decoded-stream byte offset of the first record belonging to this hour
    /// (the byte just after the previous hour's `FRAME_END`; `0` for the first).
    pub start_offset: u64,
    /// Whether folding forward from `start_offset` (with a fresh fold)
    /// reproduces this hour's state exactly — a "cheap seek point".
    ///
    /// **Honest cost model.** Delta-emitting domains (map_control, divisions,
    /// d41/d44/d48/d51/d53, …) carry a baseline once and then only changed rows;
    /// to reconstruct an arbitrary hour you must fold from a point where every
    /// such domain still had its baseline ahead of it. The only point guaranteed
    /// to satisfy that for ALL domains is the **start of the stream** (every
    /// delta tracker begins empty there), so today exactly one hour — the first
    /// emitted — is a cheap seek point. Every other hour's honest seek cost is
    /// "fold from the first hour up to it" (`O(bytes)` from stream start).
    /// A future explicit keyframe / global baseline-reset marker in the stream
    /// (see `COMPACT_STATUS_TRANSITION_BASELINE_RESET`) would add more cheap
    /// points; the machinery here already keys off this flag.
    pub cheap_seek_point: bool,
}

/// A lightweight game-hour → byte-offset index over a compact stream. Built on a
/// decode pass ([`build_index`] or [`HourCursor::take_index`]) and JSON
/// (de)serializable so a consumer can persist it next to a container and load it
/// instead of re-scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamIndex {
    /// Exact re-openable source and full decoded-byte identity certified when
    /// this index was built. Seek re-certifies and compares this whole value.
    pub source: CertifiedSourceIdentity,
    /// The game id these offsets were folded under (offsets are byte positions,
    /// independent of the id, but the id is recorded so a seek re-folds
    /// identically).
    pub game_id: String,
    /// Explicit protocol used to certify the stream. This is source identity,
    /// not a decoding hint: seek must reopen under this exact mode.
    pub identity_mode: StreamIdentityMode,
    /// Emitted hours in stream order.
    pub hours: Vec<HourOffset>,
}

/// Full identity of the decoded H4CR source covered by an authoritative index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSourceIdentity {
    /// `"stream_file"` or `"container"`.
    pub kind: String,
    /// Canonical filesystem path of the source opened for certification.
    pub canonical_path: String,
    /// Exact decoded H4CR byte length through certified physical EOF.
    pub decoded_len: u64,
    /// Lower-case SHA-256 of all `decoded_len` decoded H4CR bytes.
    pub decoded_sha256: String,
}

impl StreamIndex {
    fn validate_structure(&self) -> Result<(), String> {
        if !matches!(self.source.kind.as_str(), "stream_file" | "container") {
            return Err(format!("unsupported source kind {:?}", self.source.kind));
        }
        if self.source.canonical_path.is_empty() {
            return Err("source canonical_path is empty".to_string());
        }
        if self.source.decoded_len == 0 {
            return Err("certified decoded length is zero".to_string());
        }
        if self.source.decoded_sha256.len() != 64
            || !self
                .source
                .decoded_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("decoded_sha256 is not 64 lower-case hexadecimal characters".to_string());
        }
        let first = self
            .hours
            .first()
            .ok_or_else(|| "certified index contains no emitted hours".to_string())?;
        if first.start_offset != 0 || !first.cheap_seek_point {
            return Err(
                "first hour must be the sole cheap seek point at decoded offset zero".to_string(),
            );
        }
        let mut previous_offset = None;
        for (position, hour) in self.hours.iter().enumerate() {
            if hour.start_offset >= self.source.decoded_len {
                return Err(format!(
                    "hour at position {position} has out-of-range start_offset {} for decoded length {}",
                    hour.start_offset, self.source.decoded_len
                ));
            }
            if position > 0 && hour.cheap_seek_point {
                return Err(format!(
                    "hour at position {position} forges an unsupported non-initial cheap seek point"
                ));
            }
            if let Some(previous) = previous_offset {
                if hour.start_offset <= previous {
                    return Err(format!(
                        "hour offsets are not strictly increasing at position {position}"
                    ));
                }
            }
            previous_offset = Some(hour.start_offset);
        }
        Ok(())
    }

    /// The `[first, last]` emitted hour indices, or `None` if empty.
    pub fn hour_range(&self) -> Option<(i64, i64)> {
        match (self.hours.first(), self.hours.last()) {
            (Some(a), Some(b)) => Some((a.hour_index, b.hour_index)),
            _ => None,
        }
    }

    /// The hours that are cheap seek points (see [`HourOffset::cheap_seek_point`]).
    pub fn cheap_seek_points(&self) -> Vec<i64> {
        self.hours
            .iter()
            .filter(|h| h.cheap_seek_point)
            .map(|h| h.hour_index)
            .collect()
    }

    /// Serialize the index to a compact JSON value (for persistence).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "index_schema": CERTIFIED_INDEX_SCHEMA,
            "source": {
                "kind": self.source.kind,
                "canonical_path": self.source.canonical_path,
                "decoded_len": self.source.decoded_len,
                "decoded_sha256": self.source.decoded_sha256,
            },
            "game_id": self.game_id,
            "identity_mode": match self.identity_mode {
                StreamIdentityMode::Legacy => "legacy",
                StreamIdentityMode::IdentityV1 => "identity_v1",
            },
            "hours": self.hours.iter().map(|h| serde_json::json!({
                "hour_index": h.hour_index,
                "start_offset": h.start_offset,
                "cheap_seek_point": h.cheap_seek_point,
            })).collect::<Vec<_>>(),
        })
    }

    /// Parse an index previously produced by [`StreamIndex::to_json`].
    pub fn from_json(v: &serde_json::Value) -> Option<StreamIndex> {
        let index_schema = v.get("index_schema")?.as_str()?;
        if index_schema != CERTIFIED_INDEX_SCHEMA && index_schema != LEGACY_CERTIFIED_INDEX_SCHEMA {
            return None;
        }
        let source_json = v.get("source")?;
        let source = CertifiedSourceIdentity {
            kind: source_json.get("kind")?.as_str()?.to_string(),
            canonical_path: source_json.get("canonical_path")?.as_str()?.to_string(),
            decoded_len: source_json.get("decoded_len")?.as_u64()?,
            decoded_sha256: source_json.get("decoded_sha256")?.as_str()?.to_string(),
        };
        let game_id = v.get("game_id")?.as_str()?.to_string();
        let identity_mode = match (
            index_schema,
            v.get("identity_mode").and_then(|mode| mode.as_str()),
        ) {
            // The v1 grammar had no mode and every v1 decoder was Legacy.
            (LEGACY_CERTIFIED_INDEX_SCHEMA, None) => StreamIdentityMode::Legacy,
            (CERTIFIED_INDEX_SCHEMA, Some("legacy")) => StreamIdentityMode::Legacy,
            (CERTIFIED_INDEX_SCHEMA, Some("identity_v1")) => StreamIdentityMode::IdentityV1,
            _ => return None,
        };
        let mut hours = Vec::new();
        for h in v.get("hours")?.as_array()? {
            hours.push(HourOffset {
                hour_index: h.get("hour_index")?.as_i64()?,
                start_offset: h.get("start_offset")?.as_u64()?,
                cheap_seek_point: h.get("cheap_seek_point")?.as_bool()?,
            });
        }
        let index = StreamIndex {
            source,
            game_id,
            identity_mode,
            hours,
        };
        index.validate_structure().ok()?;
        Some(index)
    }
}

/// Build a [`StreamIndex`] for a source in one decode pass, discarding each
/// hour's snapshot bytes (bounded memory). Use [`HourCursor::take_index`]
/// instead when you are already iterating hours — the index is built for free.
pub fn build_index(source: &Source, opts: &OpenOptions) -> Result<StreamIndex, OpenError> {
    build_index_with_identity_mode(source, opts, StreamIdentityMode::Legacy)
}

/// [`build_index`] under an explicit stream identity mode.
pub fn build_index_with_identity_mode(
    source: &Source,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<StreamIndex, OpenError> {
    let mut cursor = source.open_cursor_with_identity_mode(opts, identity_mode)?;
    // Drain to completion; drop each hour immediately (bounded memory).
    while let Some(h) = cursor.next_hour() {
        h.map_err(|e| match e {
            DecodeError::Io(io) => OpenError::Io(io),
        })?;
    }
    cursor
        .take_index()
        .map_err(|e| OpenError::Certification(e.to_string()))
}

/// Reconstruct one hour from an authoritative persisted index.
///
/// The target is retained while decoding continues through physical EOF. Only
/// after full certification and exact comparison of source identity, digest,
/// length, offsets, and flags may it be returned. Cost is therefore `O(full
/// stream bytes)` until an authenticated keyframe/index format exists.
pub fn seek_to_hour(
    source: &Source,
    index: &StreamIndex,
    target: i64,
    opts: &OpenOptions,
) -> Result<WorldHour, SeekError> {
    index
        .validate_structure()
        .map_err(SeekError::InvalidIndex)?;
    // Retain the target while continuing through physical EOF. Certification
    // and index comparison happen before the retained hour can be returned.
    let mut cursor = source
        .open_cursor_with_identity_mode(opts, index.identity_mode)
        .map_err(SeekError::Open)?;
    let mut found = None;
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.map_err(SeekError::Decode)?;
        if found.is_none() && hour.hour_index == target {
            found = Some(hour);
        }
    }
    let certified = cursor
        .take_index()
        .map_err(|e| SeekError::Open(OpenError::Certification(e.to_string())))?;
    if &certified != index {
        return Err(SeekError::InvalidIndex(
            "persisted index does not match the freshly certified source, options, offsets, or flags"
                .to_string(),
        ));
    }
    found.ok_or(SeekError::HourNotFound(target))
}

#[cfg(test)]
mod tests;
