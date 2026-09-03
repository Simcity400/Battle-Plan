//! # Portable ownership boundary

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use compact_contract::campaign_identity::{
    ValidatedCid1, COMPACT_DOMAIN_STREAM_HEADER, COMPACT_RECORD_TYPE_STREAM_HEADER,
};
use compact_contract::campaign_rollover::{RolloverProposeV1, RolloverReason};
pub use compact_contract::stream_validator::StreamIdentityMode;
use compact_contract::stream_validator::{AcceptedRecordKind, StatefulStreamValidator};
use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, CompactRecordHeader,
    RecordFramingError, ASSIGNED_COMPACT_DOMAINS, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_RECORD_TYPE_FRAME_REFUSED,
};
pub use recorder_host::stream_fold::{
    domain_data_payload_status_at, excluded_domain_data_json, ExcludedDomainDataRecord,
};
use recorder_host::stream_fold::{
    strict_stream_certification_verdict, validate_domain_data_payload_at,
    validate_excluded_domain_data_accounting, BoundaryTracker, FoldStreamStats,
};
use recorder_host::world_fold::{schema_version, FoldedSnapshotFile, WorldFold};
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::{to_hex, Sha256};

const CHUNK: usize = 8 * 1024 * 1024;

pub mod fold_cache;
/// H4CP v3 prototype driver (lanes DQ/EE/GE; restored by lane PR).
#[cfg(feature = "dq-v3-prototype")]
pub mod dq_v3_prototype;
/// H4CP v3 density census (lane EE; restored by lane PR).
#[cfg(feature = "dq-v3-prototype")]
pub mod dq_v3_census;

/// Stage timers for `build_certified_replay_artifacts`, enabled by the same
/// `FOLD_PROFILE` environment variable as `world_fold::foldprof` and
/// `snapshot::replay_cache::cacheprof`. Off, each site is one branch on a
/// `OnceLock<bool>`; nothing here can change a produced byte.
pub mod finprof {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    /// Fold thread: inside `HourCursor::next_hour` (apply + build + serialize).
    pub static NEXT_HOUR_NS: AtomicU64 = AtomicU64::new(0);
    /// Fold thread: blocked handing a finished hour to the encode worker.
    pub static SEND_WAIT_NS: AtomicU64 = AtomicU64::new(0);
    /// Encode worker: inside `emit_cache_hour` (parse + render blobs + encode).
    pub static EMIT_NS: AtomicU64 = AtomicU64::new(0);
    pub static RENDER_BLOB_NS: AtomicU64 = AtomicU64::new(0);
    pub static ENCODER_FINISH_NS: AtomicU64 = AtomicU64::new(0);
    pub static PARITY_OPEN_NS: AtomicU64 = AtomicU64::new(0);
    pub static PARITY_VERIFY_NS: AtomicU64 = AtomicU64::new(0);
    pub static TOTAL_NS: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn enabled() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("FOLD_PROFILE").is_some())
    }

    #[inline]
    pub fn timed<T>(counter: &AtomicU64, f: impl FnOnce() -> T) -> T {
        if enabled() {
            let t = Instant::now();
            let out = f();
            counter.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            out
        } else {
            f()
        }
    }

    pub fn report() -> Option<String> {
        if !enabled() {
            return None;
        }
        let ms = |c: &AtomicU64| c.load(Ordering::Relaxed) as f64 / 1e6;
        Some(format!(
            "finalize stage breakdown (ms): total={:.1} | fold thread: next_hour={:.1} send_wait={:.1} | encode worker: emit={:.1} render_blobs={:.1} | encoder_finish={:.1} parity_open={:.1} parity_verify={:.1}",
            ms(&TOTAL_NS),
            ms(&NEXT_HOUR_NS),
            ms(&SEND_WAIT_NS),
            ms(&EMIT_NS),
            ms(&RENDER_BLOB_NS),
            ms(&ENCODER_FINISH_NS),
            ms(&PARITY_OPEN_NS),
            ms(&PARITY_VERIFY_NS),
        ))
    }
}

const fn round_up_to_chunk(bytes: usize) -> usize {
    match bytes.checked_add(CHUNK - 1) {
        Some(raised) => (raised / CHUNK) * CHUNK,
        None => usize::MAX,
    }
}

const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";

const CERTIFIED_INDEX_SCHEMA: &str = "h4cr-certified-index-v3";

pub const STREAM_ROLE: &str = "collector_compact_stream";

pub const DEFAULT_GAME_ID: &str = "replay";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverStreamDescriptor {
    pub logical_stream_id: u64,
    pub path: PathBuf,
    pub expected_last_complete_sequence: u64,
    pub expected_campaign_id: [u8; 32],
    pub expected_cid1_payload_sha256: [u8; 32],
    pub expected_file_sha256: [u8; 32],
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverSetDescriptor {
    pub ordered_streams: Vec<RolloverStreamDescriptor>,
    pub transitions: Vec<RolloverTransitionDescriptor>,
}

#[derive(Debug, Clone)]
pub struct AdmittedRolloverIdentity {
    pub logical_stream_id: u64,
    pub identity: ValidatedCid1,
}

pub struct RolloverStreamSet {
    cursors: VecDeque<HourCursor>,
    identities: Vec<AdmittedRolloverIdentity>,
    transitions: Vec<RolloverTransitionDescriptor>,
    completed_streams: usize,
    terminal: bool,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainInfo {
    pub id: u16,
    pub canonical_name: String,
    pub wire_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamMeta {
    pub schema_version: String,
    pub game_id: String,
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

    pub fn domain_name(&self, id: u16) -> Option<&str> {
        self.domains
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.wire_name.as_str())
    }
}

#[derive(Debug)]
pub struct WorldHour {
    pub hour_index: i64,
    pub date: String,
    pub snapshot: Vec<u8>,
    /// The hour as a canonical tree, when the fold emitted one (production
    /// finalize): `snapshot` is then empty and the encoder thread takes the
    /// digest, so the text length is accounted there rather than here.
    canonical: Option<snapshot::UnhashedWorldHour>,
    pub domain_data_counts: BTreeMap<u16, u64>,
    snapshot_charge: recorder_host::mem_account::Charge,
}

impl Clone for WorldHour {
    fn clone(&self) -> Self {
        let snapshot = self.snapshot.clone();
        let canonical = self.canonical.clone();
        let snapshot_charge = recorder_host::mem_account::Charge::new(
            recorder_host::mem_account::Component::InflightSnapshots,
            canonical
                .as_ref()
                .map_or(snapshot.capacity() as u64, |prepared| prepared.value_heap_bytes()),
        );
        Self {
            hour_index: self.hour_index,
            date: self.date.clone(),
            snapshot,
            canonical,
            domain_data_counts: self.domain_data_counts.clone(),
            snapshot_charge,
        }
    }
}

impl PartialEq for WorldHour {
    fn eq(&self, other: &Self) -> bool {
        self.hour_index == other.hour_index
            && self.date == other.date
            && self.snapshot == other.snapshot
            && self.canonical == other.canonical
            && self.domain_data_counts == other.domain_data_counts
    }
}

impl Eq for WorldHour {}

impl WorldHour {
    fn into_charged_body(self) -> (HourBody, recorder_host::mem_account::Charge) {
        match self.canonical {
            Some(prepared) => (HourBody::Canonical(prepared), self.snapshot_charge),
            None => (HourBody::Json(self.snapshot), self.snapshot_charge),
        }
    }

    /// The world JSON text, when the fold emitted text; empty for an hour the
    /// fold emitted as a canonical tree (see [`Self::canonical`]).
    pub fn snapshot_bytes(&self) -> &[u8] {
        &self.snapshot
    }

    pub fn canonical(&self) -> Option<&snapshot::UnhashedWorldHour> {
        self.canonical.as_ref()
    }

    /// Length of the world JSON text this hour carries; zero for a canonical
    /// hour, whose length the encoder thread accounts when it digests it.
    pub fn json_len(&self) -> u64 {
        self.snapshot.len() as u64
    }

    pub fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        match &self.canonical {
            Some(prepared) => serde_json::from_slice(&prepared.value().to_json_bytes()),
            None => serde_json::from_slice(&self.snapshot),
        }
    }

    pub fn snapshot_sha256(&self) -> String {
        match &self.canonical {
            Some(prepared) => prepared.value().json_sha256_hex().0,
            None => to_hex(&Sha256::digest(&self.snapshot)),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct DecodeStats {
    pub stream_bytes: u64,
    pub bytes_folded: u64,
    pub truncated_tail_bytes: u64,
    pub records_total: u64,
    pub data_records: u64,
    pub malformed_payload_records: u64,
    pub excluded_domain_data_records: Vec<ExcludedDomainDataRecord>,
    pub frame_end_records: u64,
    pub perf_records: u64,
    pub freeze_census_records: u64,
    pub other_records: u64,
    pub hours_emitted: u64,
    pub per_domain_data_records: BTreeMap<u16, u64>,
    pub stop_reason: Option<String>,
    pub boundary_error: Option<String>,
    pub ends_at_frame_end: bool,
    pub hour_coverage: recorder_host::hour_coverage::HourCoverageMap,
}

impl DecodeStats {
    pub fn certification_verdict(&self) -> Result<(), String> {
        validate_excluded_domain_data_accounting(
            self.malformed_payload_records,
            &self.excluded_domain_data_records,
        )?;
        strict_stream_certification_verdict(
            self.stop_reason.as_deref(),
            self.truncated_tail_bytes,
            self.ends_at_frame_end,
            self.boundary_error.as_deref(),
        )
    }
}

#[derive(Debug)]
pub enum OpenError {
    Io(io::Error),
    UnrecognizedInput(String),
    Container(String),
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

#[derive(Debug)]
pub enum SeekError {
    Open(OpenError),
    Decode(DecodeError),
    HourNotFound(i64),
    HourExcluded { hour_index: i64, reason: String },
    InvalidIndex(String),
}

impl std::fmt::Display for SeekError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeekError::Open(e) => write!(f, "open: {e}"),
            SeekError::Decode(e) => write!(f, "decode: {e}"),
            SeekError::HourNotFound(h) => write!(f, "hour {h} not found in stream"),
            SeekError::HourExcluded { hour_index, reason } => write!(
                f,
                "hour {hour_index} is excluded from this recording's verified coverage: {reason}"
            ),
            SeekError::InvalidIndex(s) => write!(f, "invalid stream index: {s}"),
        }
    }
}
impl std::error::Error for SeekError {}

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

#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    pub game_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Source {
    StreamFile(PathBuf),
    Container(PathBuf),
}

impl Source {
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

    pub fn open_cursor(&self, opts: &OpenOptions) -> Result<HourCursor, OpenError> {
        self.open_cursor_with_identity_mode(opts, StreamIdentityMode::Legacy)
    }

    pub fn open_cursor_with_identity_mode(
        &self,
        opts: &OpenOptions,
        identity_mode: StreamIdentityMode,
    ) -> Result<HourCursor, OpenError> {
        match self {
            Source::StreamFile(path) => {
                let canonical_path = fs::canonicalize(path)?;
                // where a spool whose storage prefix was released is refused.
                recorder_host::spool_trim::require_whole_spool(&canonical_path, "replay decoder")
                    .map_err(OpenError::Certification)?;
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

pub fn open(path: impl AsRef<Path>) -> Result<HourCursor, OpenError> {
    open_with(path, &OpenOptions::default())
}

pub fn open_with(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<HourCursor, OpenError> {
    Source::detect(path)?.open_cursor(opts)
}

pub fn open_with_identity_mode(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<HourCursor, OpenError> {
    Source::detect(path)?.open_cursor_with_identity_mode(opts, identity_mode)
}

pub fn open_stream_file(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
) -> Result<HourCursor, OpenError> {
    Source::StreamFile(path.as_ref().to_path_buf()).open_cursor(opts)
}

pub fn open_stream_file_with_identity_mode(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<HourCursor, OpenError> {
    Source::StreamFile(path.as_ref().to_path_buf())
        .open_cursor_with_identity_mode(opts, identity_mode)
}

pub fn open_container(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<HourCursor, OpenError> {
    Source::Container(path.as_ref().to_path_buf()).open_cursor(opts)
}

pub fn open_container_with_identity_mode(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<HourCursor, OpenError> {
    Source::Container(path.as_ref().to_path_buf())
        .open_cursor_with_identity_mode(opts, identity_mode)
}

pub fn open_reader<R: Read + 'static>(
    reader: R,
    total_len: u64,
    game_id: impl Into<String>,
) -> HourCursor {
    open_reader_with_identity_mode(reader, total_len, game_id, StreamIdentityMode::Legacy)
}

pub fn open_reader_with_identity_mode<R: Read + 'static>(
    reader: R,
    total_len: u64,
    game_id: impl Into<String>,
    identity_mode: StreamIdentityMode,
) -> HourCursor {
    HourCursor::from_reader(Box::new(reader), total_len, game_id.into(), identity_mode)
}

pub fn fold_raw_h4cr_prefolded(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
) -> Result<snapshot::PrefoldedReplay, OpenError> {
    fold_raw_h4cr_prefolded_with_identity_mode(path, opts, StreamIdentityMode::Legacy)
}

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
    pub excluded_domain_data_records: Vec<ExcludedDomainDataRecord>,
    pub hour_coverage: recorder_host::hour_coverage::HourCoverageMap,
    pub campaign_identity_manifest: Option<serde_json::Value>,
    pub durable_decoded_end_offset: u64,
    pub published_coverage_path: PathBuf,
}

type ReplayCacheEncoderFile = snapshot::ReplayCacheEncoder<fs::File>;

/// What an hour arrives at the encoder as: the text the fold serialized, or
/// the canonical tree it built directly.
enum HourBody {
    Json(Vec<u8>),
    Canonical(snapshot::UnhashedWorldHour),
}

struct CacheEmitHour {
    hour_index: i64,
    date: String,
    start_offset: u64,
    end_offset: u64,
    body: HourBody,
    snapshot_charge: recorder_host::mem_account::Charge,
}

enum CacheEmitJob {
    Hour(CacheEmitHour),
    Hole(snapshot::ReplayExcludedHour),
}

/// The two are one object because their lifetimes are: the encoder's delta base
struct ChargedCacheEncoder {
    encoder: ReplayCacheEncoderFile,
    delta_base: recorder_host::mem_account::Charge,
    hour_entries: recorder_host::mem_account::Charge,
    /// Canonical text bytes of the hours that arrived as trees, accounted
    /// here because their length is only known once digested.
    canonical_json_bytes: u64,
}

impl ChargedCacheEncoder {
    fn new(encoder: ReplayCacheEncoderFile) -> Self {
        Self {
            encoder,
            delta_base: recorder_host::mem_account::Charge::new(
                recorder_host::mem_account::Component::CacheDeltaBase,
                0,
            ),
            hour_entries: recorder_host::mem_account::Charge::new(
                recorder_host::mem_account::Component::CacheHourEntries,
                0,
            ),
            canonical_json_bytes: 0,
        }
    }

    fn reprice(&mut self) {
        self.delta_base.set(self.encoder.delta_base_heap_bytes());
        self.hour_entries
            .set(self.encoder.hour_entries_heap_bytes());
    }
}

impl CacheEmitJob {
    fn from_hour(hour: WorldHour, start_offset: u64, end_offset: u64) -> Self {
        let hour_index = hour.hour_index;
        let date = hour.date.clone();
        let (body, snapshot_charge) = hour.into_charged_body();
        Self::Hour(CacheEmitHour {
            hour_index,
            date,
            start_offset,
            end_offset,
            body,
            snapshot_charge,
        })
    }
}

fn hole_for(
    coverage: &recorder_host::hour_coverage::HourCoverageMap,
    hour: &WorldHour,
    start_offset: u64,
    end_offset: u64,
) -> Option<snapshot::ReplayExcludedHour> {
    let raw = i32::try_from(hour.hour_index)
        .ok()
        .and_then(|index| index.checked_add(compact_contract::calendar::DATE_EPOCH_CONST));
    if raw.is_some_and(|raw| coverage.is_hour_verified(raw)) {
        return None;
    }
    let reason = raw
        .and_then(|raw| coverage.row(raw))
        .and_then(|row| match &row.coverage {
            recorder_host::hour_coverage::HourCoverage::Excluded { reason } => Some(reason.clone()),
            recorder_host::hour_coverage::HourCoverage::Verified { .. } => None,
        })
        .unwrap_or_else(|| {
            format!(
                "this recording's verified-coverage map names game hour {} nowhere at all, and \
                 absence is not a claim of coverage",
                hour.hour_index
            )
        });
    Some(snapshot::ReplayExcludedHour::new(
        hour.hour_index,
        start_offset,
        end_offset,
        reason,
    ))
}

const FINALIZE_PIPELINE_DEPTH: usize = 2;

fn lower_current_thread_priority() {
    #[cfg(windows)]
    {
        const THREAD_PRIORITY_LOWEST: i32 = -2;
        extern "system" {
            fn GetCurrentThread() -> isize;
            fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        }
        // SAFETY: both calls take no pointers and cannot fail destructively;
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        }
    }
}

fn emit_cache_hour(
    output_dir: &Path,
    charged: &mut ChargedCacheEncoder,
    job: CacheEmitJob,
) -> Result<(), OpenError> {
    let cache_encoder = &mut charged.encoder;
    let hour = match job {
        CacheEmitJob::Hole(hole) => {
            return cache_encoder
                .exclude_hour(hole)
                .map_err(|error| OpenError::Certification(error.to_string()))
        }
        CacheEmitJob::Hour(hour) => hour,
    };
    // and releases it once it has the digest, so a guard that lived to the end
    let CacheEmitHour {
        hour_index,
        date,
        start_offset,
        end_offset,
        body,
        snapshot_charge,
    } = hour;
    let prepared = match body {
        HourBody::Json(world_json) => snapshot::CanonicalWorldHour::parse(world_json, hour_index)
            .map_err(|error| OpenError::Certification(error.to_string()))?,
        HourBody::Canonical(unhashed) => {
            let prepared = unhashed.hashed();
            recorder_host::world_fold::foldprof::add_bytes(prepared.json_len() as usize);
            charged.canonical_json_bytes = charged
                .canonical_json_bytes
                .checked_add(prepared.json_len())
                .ok_or_else(|| {
                    OpenError::Certification(
                        "canonical JSON byte accounting overflowed u64".to_string(),
                    )
                })?;
            prepared
        }
    };
    drop(snapshot_charge);
    finprof::timed(&finprof::RENDER_BLOB_NS, || {
        recorder_host::world_fold::preserve_production_render_blobs_from_canon(
            output_dir,
            prepared.value(),
        )
    })?;
    cache_encoder
        .push_prepared(
            snapshot::CacheBuildHourMeta {
                hour_index,
                date,
                h4cr_start_offset: start_offset,
                h4cr_end_offset: end_offset,
            },
            prepared,
        )
        .map_err(|error| OpenError::Certification(error.to_string()))
}

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
    if !source_meta.file_type().is_file() || source_meta.file_type().is_symlink() {
        return Err(OpenError::Certification(format!(
            "H4CR source is not a regular file: {}",
            raw_h4cr_path.display()
        )));
    }
    recorder_host::spool_trim::require_whole_spool(&raw_h4cr_path, "certified artifact build")
        .map_err(OpenError::Certification)?;
    let source_compression = compact_stream::detect(&raw_h4cr_path)?;
    match source_compression {
        compact_stream::StreamCompression::None if source_meta.len() != expected_source_len => {
            return Err(OpenError::Certification(format!(
                "raw H4CR source metadata disagrees with its receipt: {}",
                raw_h4cr_path.display()
            )));
        }
        compact_stream::StreamCompression::Zstd if source_meta.len() == 0 => {
            return Err(OpenError::Certification(format!(
                "compressed H4CR source is empty: {}",
                raw_h4cr_path.display()
            )));
        }
        _ => {}
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

    let source_file = compact_stream::open_with_compression(&raw_h4cr_path, source_compression)?;
    let mut cursor = HourCursor::from_reader_compressed_with_source(
        Box::new(source_file),
        expected_source_len,
        game_id.clone(),
        source_compression == compact_stream::StreamCompression::Zstd,
        identity_mode,
        Some(IndexSourceDescriptor {
            kind: "stream_file".to_string(),
            canonical_path: raw_h4cr_path.to_string_lossy().into_owned(),
        }),
    );
    cursor.set_emit_canonical(true);

    recorder_host::mem_account::mark("finalize.cursor_open");
    let finalize_started = std::time::Instant::now();

    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<CacheEmitJob>(FINALIZE_PIPELINE_DEPTH);
    let mut pending: Option<(WorldHour, u64)> = None;
    let mut legacy_json_bytes_avoided = 0u64;

    let pipeline_result = thread::scope(|scope| {
        let consumer_dir = output_dir.clone();
        let consumer = scope.spawn(move || -> Result<ChargedCacheEncoder, OpenError> {
            lower_current_thread_priority();
            let mut charged = ChargedCacheEncoder::new(cache_encoder);
            for job in jobs_rx {
                finprof::timed(&finprof::EMIT_NS, || {
                    emit_cache_hour(&consumer_dir, &mut charged, job)
                })?;
                charged.reprice();
                recorder_host::finalize_progress::tick(1);
            }
            Ok(charged)
        });

        let producer = (|| -> Result<(), OpenError> {
            while let Some(hour) = finprof::timed(&finprof::NEXT_HOUR_NS, || cursor.next_hour()) {
                recorder_host::finalize_progress::tick(1);
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
                    legacy_json_bytes_avoided = legacy_json_bytes_avoided
                        .checked_add(previous.json_len())
                        .ok_or_else(|| {
                            OpenError::Certification(
                                "legacy JSON byte accounting overflowed u64".to_string(),
                            )
                        })?;
                    let job = match hole_for(
                        cursor.live_hour_coverage(),
                        &previous,
                        previous_start,
                        start_offset,
                    ) {
                        Some(hole) => CacheEmitJob::Hole(hole),
                        None => CacheEmitJob::from_hour(previous, previous_start, start_offset),
                    };
                    if finprof::timed(&finprof::SEND_WAIT_NS, || jobs_tx.send(job)).is_err() {
                        return Ok(());
                    }
                }
                pending = Some((hour, start_offset));
            }
            Ok(())
        })();

        let producer_result = producer.and_then(|()| {
            let excluded = cursor.stats().excluded_domain_data_records.clone();
            let hour_coverage = cursor.stats().hour_coverage.clone();
            let identity = cursor.campaign_identity().map(|identity| {
                recorder_host::campaign_identity::to_manifest_json(
                    &recorder_host::campaign_identity::resolve_validated_cid1(identity),
                )
            });
            let published_hour_coverage_end = cursor.published_hour_coverage_end();
            recorder_host::mem_account::mark("finalize.before_fold_release");
            let index_rows_charge = cursor.take_index_rows_charge();
            let stream_index = cursor
                .take_index()
                .map_err(|error| OpenError::Certification(error.to_string()))?;
            recorder_host::mem_account::mark("finalize.after_fold_release");
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
            let (last, last_start) = pending.take().ok_or_else(|| {
                OpenError::Certification("certified H4CR stream emitted no hours".to_string())
            })?;
            legacy_json_bytes_avoided = legacy_json_bytes_avoided
                .checked_add(last.json_len())
                .ok_or_else(|| {
                    OpenError::Certification(
                        "legacy JSON byte accounting overflowed u64".to_string(),
                    )
                })?;
            let last_job = if stream_index
                .hours
                .iter()
                .any(|row| row.hour_index == last.hour_index)
            {
                Some(CacheEmitJob::from_hour(
                    last,
                    last_start,
                    expected_source_len,
                ))
            } else {
                stream_index
                    .excluded
                    .iter()
                    .find(|hole| hole.hour_index == last.hour_index)
                    .map(|hole| {
                        CacheEmitJob::Hole(snapshot::ReplayExcludedHour::new(
                            hole.hour_index,
                            hole.start_offset,
                            hole.end_offset,
                            hole.reason.clone(),
                        ))
                    })
            };
            if let Some(job) = last_job {
                let _ = jobs_tx.send(job);
            }
            Ok((
                excluded,
                hour_coverage,
                identity,
                stream_index,
                index_rows_charge,
                published_hour_coverage_end,
            ))
        });
        drop(jobs_tx);

        let consumed = consumer.join().unwrap_or_else(|_| {
            Err(OpenError::Certification(
                "finalize cache-encode worker panicked".to_string(),
            ))
        });
        let cache_encoder = consumed?;
        let (
            excluded,
            hour_coverage,
            identity,
            stream_index,
            index_rows_charge,
            published_hour_coverage_end,
        ) = producer_result?;
        Ok::<_, OpenError>((
            cache_encoder,
            excluded,
            hour_coverage,
            identity,
            stream_index,
            index_rows_charge,
            published_hour_coverage_end,
        ))
    });
    let (
        charged_encoder,
        excluded_domain_data_records,
        hour_coverage,
        campaign_identity_manifest,
        stream_index,
        _stream_index_rows_charge,
        published_hour_coverage_end,
    ) = pipeline_result?;

    // all still alive here and are released over the next few statements; a
    recorder_host::mem_account::mark("finalize.pipeline_joined");

    let cache_identity_mode = match identity_mode {
        StreamIdentityMode::Legacy => snapshot::CacheIdentityMode::Legacy,
        StreamIdentityMode::IdentityV1 => snapshot::CacheIdentityMode::IdentityV1,
    };
    let schema = schema_version().to_string();
    let ChargedCacheEncoder {
        encoder: cache_encoder,
        delta_base: delta_base_charge,
        hour_entries: hour_entries_charge,
        canonical_json_bytes,
    } = charged_encoder;
    legacy_json_bytes_avoided = legacy_json_bytes_avoided
        .checked_add(canonical_json_bytes)
        .ok_or_else(|| {
            OpenError::Certification("legacy JSON byte accounting overflowed u64".to_string())
        })?;
    let (mut cache_file, replay_index) = finprof::timed(&finprof::ENCODER_FINISH_NS, || {
        cache_encoder.finish(game_id, schema, cache_identity_mode, expected_source_len)
    })
    .map_err(|error| OpenError::Certification(error.to_string()))?;
    // dropped, and only released when the index is.
    drop(delta_base_charge);
    let _replay_index_rows_charge = hour_entries_charge;
    finprof::timed(&finprof::ENCODER_FINISH_NS, || -> io::Result<()> {
        cache_file.flush()?;
        cache_file.sync_all()
    })?;
    drop(cache_file);

    let stream_end_offsets = stream_index.served_end_offsets();
    if replay_index.hours().len() != stream_index.hours.len()
        || replay_index
            .hours()
            .iter()
            .zip(stream_index.hours.iter().zip(&stream_end_offsets))
            .any(|(artifact, (stream, stream_end))| {
                artifact.hour_index() != stream.hour_index
                    || artifact.h4cr_start_offset() != stream.start_offset
                    || artifact.h4cr_end_offset() != *stream_end
            })
    {
        return Err(OpenError::Certification(
            "binary replay hour index disagrees with the certified H4CR fold index".to_string(),
        ));
    }
    if replay_index.excluded_hours().len() != stream_index.excluded.len()
        || replay_index
            .excluded_hours()
            .iter()
            .zip(&stream_index.excluded)
            .any(|(artifact, stream)| {
                artifact.hour_index() != stream.hour_index
                    || artifact.h4cr_start_offset() != stream.start_offset
                    || artifact.h4cr_end_offset() != stream.end_offset
                    || artifact.reason() != stream.reason
            })
    {
        return Err(OpenError::Certification(
            "binary replay hour index names different excluded hours from the certified H4CR \
             fold index"
                .to_string(),
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

    recorder_host::mem_account::mark("finalize.before_parity_verify");
    let cache_file = fs::File::open(&cache_path)?;
    let parity_reader = finprof::timed(&finprof::PARITY_OPEN_NS, || {
        snapshot::CertifiedReplayCache::open_file(
            &index_bytes,
            cache_file,
            expected_source_len,
            expected_source_sha256,
        )
    })
    .map_err(|error| OpenError::Certification(error.to_string()))?;
    recorder_host::finalize_progress::stage(recorder_host::finalize_progress::STAGE_PARITY_VERIFY);
    let max_seek_deltas = finprof::timed(&finprof::PARITY_VERIFY_NS, || {
        parity_reader.verify_every_hour_observed(&mut || recorder_host::finalize_progress::tick(1))
    })
    .map_err(|error| {
        OpenError::Certification(format!("binary reconstruction parity failed: {error}"))
    })?;
    finprof::TOTAL_NS.fetch_add(
        finalize_started.elapsed().as_nanos() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
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

    let published_coverage_path = output_dir.join(snapshot::PUBLISHED_COVERAGE_FILE);
    let published_coverage = serde_json::json!({
        "schema": snapshot::PUBLISHED_COVERAGE_SCHEMA,
        "source_len": expected_source_len,
        "source_sha256": expected_source_sha256,
        "checkpoint_cache_sha256": replay_index.cache_sha256(),
        "hour_index_sha256": Sha256::hex(&index_bytes),
        "hour_count": replay_index.hours().len() as u64,
        "published_decoded_end_offset": published_hour_coverage_end,
        "note": "published_decoded_end_offset is where the fold's PUBLISHED hours stop. \
                 It is SHORTER than source_len whenever the stream ended in bytes that \
                 reached no published hour - a terminal FRAME_REFUSED, or a trailing \
                 FRAME_END that closed nothing. The hour index deliberately stretches its \
                 last hour to source_len so a seek can replay it; that is not a statement \
                 that the cache holds those bytes.",
    });
    let published_coverage_bytes = serde_json::to_vec_pretty(&published_coverage)
        .map_err(|error| OpenError::Certification(error.to_string()))?;
    let mut coverage_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&published_coverage_path)?;
    coverage_file.write_all(&published_coverage_bytes)?;
    coverage_file.flush()?;
    coverage_file.sync_all()?;
    drop(coverage_file);

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
        excluded_domain_data_records,
        hour_coverage,
        campaign_identity_manifest,
        durable_decoded_end_offset: published_hour_coverage_end,
        published_coverage_path,
    })
}

struct SpooledFoldPass {
    files: Vec<FoldedSnapshotFile>,
    stats: FoldStreamStats,
    boundary_error: Option<String>,
    campaign_identity: Option<ValidatedCid1>,
}

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

            let sequence_error =
                strict_sequence_error(previous_ordinary_sequence, header.sequence, record_at);
            previous_ordinary_sequence = Some(header.sequence);
            let contract_error: Option<String> = sequence_error.clone().or_else(|| {
                validate_compact_record_envelope(&header, payload)
                    .err()
                    .map(|reason| {
                        format!("record contract violation at byte {record_at}: {reason}")
                    })
            });
            // A SEMANTICALLY MALFORMED DOMAIN_DATA_REPEAT IS A LOST RECORD,
            // NOT A DEAD CERTIFICATION (Codex review round 15, finding 3): it
            // is excluded and named below instead of poisoning
            // `boundary_error`. Sequence-order violations stay fatal — they
            // say the STREAM is damaged, not one record.
            let tolerated_repeat_error = contract_error.is_some()
                && sequence_error.is_none()
                && header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT;
            if !tolerated_repeat_error && validation_error.is_none() {
                validation_error = contract_error.clone();
            }
            stats.records_total += 1;

            match compact_contract::KnownRecordType::try_from(header.record_type) {
                Ok(
                    compact_contract::KnownRecordType::DomainData
                    | compact_contract::KnownRecordType::DomainDataRepeat,
                ) => stats.data_records += 1,
                Ok(compact_contract::KnownRecordType::FrameEnd) => stats.frame_end_records += 1,
                Ok(compact_contract::KnownRecordType::DomainPerf) => stats.perf_records += 1,
                Ok(compact_contract::KnownRecordType::FreezeCensus) => {
                    stats.freeze_census_records += 1
                }
                Ok(
                    compact_contract::KnownRecordType::ContentIdentityCensus
                    | compact_contract::KnownRecordType::StateTopologyCensus
                    | compact_contract::KnownRecordType::StreamHeader
                    | compact_contract::KnownRecordType::DomainGap
                    | compact_contract::KnownRecordType::FrameRefused
                    | compact_contract::KnownRecordType::FrameDegraded
                    | compact_contract::KnownRecordType::CaptureTerminated
                    | compact_contract::KnownRecordType::ClockUnreadableInterval
                    | compact_contract::KnownRecordType::UntrustedClockRecovery,
                )
                | Err(_) => stats.other_records += 1,
            }
            if tolerated_repeat_error {
                stats.malformed_payload_records += 1;
                stats
                    .excluded_domain_data_records
                    .push(ExcludedDomainDataRecord {
                        stream_offset: record_at,
                        game_hour_raw: header.game_hour_raw,
                        domain_id: header.domain_id,
                        validation_reason: contract_error.clone().unwrap_or_default(),
                    });
                fold.refuse_domain_data_repeat(
                    header.game_hour_raw,
                    header.domain_id,
                    contract_error.as_deref().unwrap_or("malformed"),
                );
                // The tracker is told the record is TOLERATED so its rules
                // do not latch a violation the walk has already forgiven
                // (Codex review round 16, finding 4).
                tracker.observe_tolerated(&header, payload, record_at);
                off += record_len;
                continue;
            }
            tracker.observe(&header, payload, record_at);


            // RECONSTRUCT A `PSD1` ENVELOPE BEFORE VALIDATING IT, and clear the
            // flag for everything downstream. This walk validates the payload as
            // the domain's own grammar and SKIPS `ingest_record` when that
            // fails, so an envelope reaching the validator un-reconstructed is
            // not merely mis-validated: the record is marked malformed and the
            // one call that would have reconstructed it never runs. Every
            // wrapped d43/d53 record would be excluded from the certified replay
            // product while `stream_fold`'s own artefacts looked clean, because
            // that is a different walker (Codex review round 7, finding 1).
            //
            // The reconstruction uses the FOLD's retained baselines, exactly as
            // `stream_fold::drive_fold_compressed_with_identity_mode` does, so
            // this walker and that one hold the same bytes for the same records.
            let mut downstream_flags = header.record_flags;
            let mut reconstructed: Option<Vec<u8>> = None;
            let mut reconstruction_failure: Option<String> = None;
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                // PROVE, DON'T ADVANCE (Codex review round 16, finding 2):
                // validation uses the READ-ONLY probe; `ingest_record` below
                // receives RAW ON-WIRE bytes plus the ORIGINAL flag word and
                // performs the one true advance itself.
                match fold.probe_domain_data(
                    header.sequence,
                    header.game_hour_raw,
                    header.domain_id,
                    header.record_flags,
                    payload,
                ) {
                    Ok(value) => {
                        if value.is_some() {
                            downstream_flags &=
                                !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA;
                        }
                        reconstructed = value;
                    }
                    Err(reason) => reconstruction_failure = Some(reason),
                }
            } else if header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
            {
                // A refused repeat reaches THIS walker's ledger with its exact
                // reason, not only the fold's malformed-hour note (Codex
                // review round 10, finding 4): probe every check
                // `ingest_record` would apply, read-only — and keep the
                // RESOLVED payload so its grammar is validated exactly like a
                // full record's (Codex review round 21).
                match fold.verify_domain_data_repeat(
                    header.domain_id,
                    header.record_flags,
                    payload,
                ) {
                    Ok(resolved) => {
                        reconstructed = Some(resolved);
                        downstream_flags &=
                            !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA;
                    }
                    Err(reason) => {
                        reconstruction_failure =
                            Some(format!("DOMAIN_DATA_REPEAT composition refused: {reason}"))
                    }
                }
            }
            let folded_payload = reconstructed.as_deref().unwrap_or(payload);
            let is_repeat = header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT;
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA || is_repeat {
                let outcome = match reconstruction_failure {
                    Some(reason) => Err(reason),
                    None => validate_domain_data_payload_at(
                        header.domain_id,
                        folded_payload,
                        downstream_flags,
                        header.sequence,
                        header.game_hour_raw,
                        record_at,
                    ),
                };
                if let Err(reason) = outcome {
                    stats.malformed_payload_records += 1;
                    stats
                        .excluded_domain_data_records
                        .push(ExcludedDomainDataRecord {
                            stream_offset: record_at,
                            game_hour_raw: header.game_hour_raw,
                            domain_id: header.domain_id,
                            validation_reason: reason.clone(),
                        });
                    if is_repeat {
                        // A refused repeat keeps the REPEAT arm's named
                        // provenance, not the generic malformed-payload one.
                        fold.refuse_domain_data_repeat(
                            header.game_hour_raw,
                            header.domain_id,
                            &reason,
                        );
                    } else {
                        // The producer published this record and moved BOTH
                        // baselines with it; the reader must too (Codex
                        // review round 17, finding 1).
                        fold.note_rejected_domain_data(
                            header.sequence,
                            header.game_hour_raw,
                            header.domain_id,
                            header.record_flags,
                            payload,
                        );
                        fold.record_malformed_domain_payload(
                            header.game_hour_raw,
                            header.domain_id,
                            payload,
                            &reason,
                        );
                    }
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

pub fn fold_raw_h4cr_file_spooled(
    request: snapshot::CertifiedPrefoldSpoolRequest,
) -> Result<snapshot::PrefoldedSpool, OpenError> {
    let identity_mode = compact_identity_mode(request.identity_mode());
    fold_raw_h4cr_file_spooled_with_identity_mode(request, identity_mode)
}

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
    recorder_host::spool_trim::require_whole_spool(&raw_path, "spooled prefold")
        .map_err(OpenError::Certification)?;
    let source = fs::File::open(&raw_path)?;
    fold_raw_h4cr_open_file_spooled_with_identity_mode(request, source, identity_mode)
}

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
    validate_excluded_domain_data_accounting(
        stats.malformed_payload_records,
        &stats.excluded_domain_data_records,
    )
    .map_err(OpenError::Certification)?;
    strict_stream_certification_verdict(
        stats.stop_reason.as_deref(),
        stats.truncated_tail_bytes,
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

pub fn fold_raw_h4cr_bytes_prefolded(
    raw_h4cr: Vec<u8>,
    game_id: impl Into<String>,
) -> Result<snapshot::PrefoldedReplay, OpenError> {
    fold_raw_h4cr_bytes_prefolded_with_identity_mode(raw_h4cr, game_id, StreamIdentityMode::Legacy)
}

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

fn extract_container_stream(path: &Path) -> Result<(Vec<u8>, String), OpenError> {
    let container = Container::open(path, &ExpectedIdentity::default())
        .map_err(|e| OpenError::Container(format!("open {}: {e}", path.display())))?;
    let game_id = container.game_id().to_string();
    let entries = container.entries_by_role(STREAM_ROLE);
    let bytes = match entries.len() {
        0 => {
            return Err(OpenError::Container(
                match snapshot::container_raw_stream_packaging(&container) {
                    Ok(snapshot::RawStreamPackaging::Omitted) => format!(
                        "{}: {}",
                        path.display(),
                        snapshot::raw_stream_unavailable_message("decode the raw H4CR stream"),
                    ),
                    _ => format!("{} has no entry with role {STREAM_ROLE:?}", path.display()),
                },
            ))
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

pub struct HourCursor {
    reader: Box<dyn Read>,
    fold: WorldFold,
    identity_mode: StreamIdentityMode,
    stream_validator: StatefulStreamValidator,
    campaign_identity: Option<ValidatedCid1>,
    carry: Vec<u8>,
    pos: usize,
    processed: u64,
    decoded_read: u64,
    total_len: u64,
    ready: VecDeque<WorldHour>,
    cur_counts: BTreeMap<u16, u64>,
    stats: DecodeStats,
    meta: StreamMeta,
    done: bool,
    compressed: bool,
    cur_hour_start: u64,
    published_hour_coverage_end: u64,
    index_hours: Vec<HourOffset>,
    first_hour_seen: bool,
    boundary_tracker: BoundaryTracker,
    strict_previous_sequence: Option<u64>,
    strict_open_hour: Option<i32>,
    strict_validation_error: Option<String>,
    source_hasher: Sha256,
    index_source: Option<IndexSourceDescriptor>,
    framing_charge: recorder_host::mem_account::Charge,
    hour_index_charge: recorder_host::mem_account::Charge,
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
        let framing_charge = recorder_host::mem_account::Charge::new(
            recorder_host::mem_account::Component::DecoderFraming,
            (CHUNK * 2) as u64,
        );
        let hour_index_charge = recorder_host::mem_account::Charge::new(
            recorder_host::mem_account::Component::DecoderHourIndex,
            0,
        );
        HourCursor {
            reader,
            fold: WorldFold::new(game_id.clone()),
            identity_mode,
            stream_validator: stream_validator(identity_mode),
            campaign_identity: None,
            carry: Vec::with_capacity(CHUNK * 2),
            pos: 0,
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
            published_hour_coverage_end: 0,
            index_hours: Vec::new(),
            first_hour_seen: false,
            boundary_tracker: BoundaryTracker::default(),
            strict_previous_sequence: None,
            strict_open_hour: None,
            strict_validation_error: None,
            source_hasher: Sha256::new(),
            index_source,
            framing_charge,
            hour_index_charge,
        }
    }

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

    pub fn metadata(&self) -> &StreamMeta {
        &self.meta
    }

    pub fn identity_mode(&self) -> StreamIdentityMode {
        self.identity_mode
    }

    pub fn campaign_identity(&self) -> Option<&ValidatedCid1> {
        self.campaign_identity.as_ref()
    }

    pub fn published_hour_coverage_end(&self) -> u64 {
        self.published_hour_coverage_end
    }

    pub fn stats(&self) -> &DecodeStats {
        &self.stats
    }

    pub fn live_hour_coverage(&self) -> &recorder_host::hour_coverage::HourCoverageMap {
        self.boundary_tracker.coverage()
    }

    pub fn buffered_hours(&self) -> usize {
        self.ready.len()
    }

    /// Have the fold hand hours over as canonical trees instead of JSON text.
    pub fn set_emit_canonical(&mut self, on: bool) {
        self.fold.set_emit_canonical(on);
    }

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

    pub fn take_index(mut self) -> Result<StreamIndex, IndexCertificationError> {
        self.validate_complete()?;
        let coverage = self.boundary_tracker.coverage().clone();
        let decoded_len = self.decoded_read;
        let mut excluded = Vec::new();
        let mut served = Vec::new();
        for (position, row) in self.index_hours.iter().enumerate() {
            let end_offset = self
                .index_hours
                .get(position + 1)
                .map_or(decoded_len, |next| next.start_offset);
            let raw = i32::try_from(row.hour_index)
                .ok()
                .and_then(|hour| hour.checked_add(compact_contract::calendar::DATE_EPOCH_CONST));
            match raw {
                Some(raw) if coverage.is_hour_verified(raw) => served.push(row.clone()),
                _ => {
                    let reason = raw
                        .and_then(|raw| coverage.row(raw))
                        .and_then(|row| match &row.coverage {
                            recorder_host::hour_coverage::HourCoverage::Excluded { reason } => {
                                Some(reason.clone())
                            }
                            recorder_host::hour_coverage::HourCoverage::Verified { .. } => None,
                        })
                        .unwrap_or_else(|| {
                            format!(
                                "this recording's verified-coverage map names game hour {} \
                                 nowhere at all, and absence is not a claim of coverage",
                                row.hour_index
                            )
                        });
                    excluded.push(ExcludedHour {
                        hour_index: row.hour_index,
                        start_offset: row.start_offset,
                        end_offset,
                        reason,
                    });
                }
            }
        }
        if served.is_empty() {
            return Err(IndexCertificationError::Stream(format!(
                "no game hour in this recording is verified, so no certified index can be \
                 minted ({})",
                self.stats.hour_coverage.summary()
            )));
        }
        self.index_hours = served;
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
            excluded,
        };
        index
            .validate_structure()
            .map_err(IndexCertificationError::InvalidIndex)?;
        Ok(index)
    }

    /// guard leaves the cursor's remaining lifetime honestly charging nothing,
    fn take_index_rows_charge(&mut self) -> recorder_host::mem_account::Charge {
        std::mem::replace(
            &mut self.hour_index_charge,
            recorder_host::mem_account::Charge::new(
                recorder_host::mem_account::Component::DecoderHourIndex,
                0,
            ),
        )
    }

    pub fn certify_complete(self) -> Result<(), IndexCertificationError> {
        self.validate_complete()
    }

    fn validate_complete(&self) -> Result<(), IndexCertificationError> {
        if !self.done {
            return Err(IndexCertificationError::NotExhausted);
        }
        validate_excluded_domain_data_accounting(
            self.stats.malformed_payload_records,
            &self.stats.excluded_domain_data_records,
        )
        .map_err(IndexCertificationError::Stream)?;
        recorder_host::hour_coverage::per_hour_seal_verdict(&self.stats.hour_coverage).map_err(
            |error| {
                IndexCertificationError::Stream(match self.stats.boundary_error.as_deref() {
                    Some(strict) => format!("{error}; {strict}"),
                    None => error,
                })
            },
        )?;
        if self.total_len != 0 && self.total_len != self.decoded_read {
            return Err(IndexCertificationError::Length(format!(
                "declared decoded length {} does not match {} byte(s) read",
                self.total_len, self.decoded_read
            )));
        }
        Ok(())
    }

    fn pump(&mut self) -> Result<(), DecodeError> {
        loop {
            while self.carry.len() - self.pos >= COMPACT_RECORD_HEADER_LEN {
                let remaining = self.carry.len() - self.pos;
                let record_start_abs = self.processed + self.pos as u64;
                let header = match parse_compact_record_header(&self.carry[self.pos..]) {
                    Ok(h) => h,
                    Err(e) => {
                        let message = framing_stop_message(e, record_start_abs);
                        if self.stats.stop_reason.is_none() {
                            self.stats.stop_reason = Some(message.clone());
                        }
                        match if self.compressed {
                            recorder_host::stream_fold::FramingResync::Exhausted
                        } else {
                            recorder_host::stream_fold::find_frame_resync_offset(
                                &self.carry,
                                self.pos,
                            )
                        } {
                            recorder_host::stream_fold::FramingResync::At(next) => {
                                let skipped = (next - self.pos) as u64;
                                let before = recorder_host::stream_fold::FoldDamageWatch::before(
                                    &self.boundary_tracker,
                                );
                                self.boundary_tracker.note_framing_damage(
                                    record_start_abs,
                                    skipped,
                                    &message,
                                );
                                recorder_host::stream_fold::apply_fold_damage_step(
                                    before.after(&self.boundary_tracker, 0),
                                    &mut self.fold,
                                );
                                self.pos = next;
                                continue;
                            }
                            recorder_host::stream_fold::FramingResync::NeedMoreData
                                if self.carry.len() - self.pos
                                    <= recorder_host::stream_fold::MAX_RESYNC_WINDOW_BYTES =>
                            {
                                break;
                            }
                            recorder_host::stream_fold::FramingResync::NeedMoreData
                            | recorder_host::stream_fold::FramingResync::Exhausted
                                if !self.compressed =>
                            {
                                let keep = COMPACT_RECORD_HEADER_LEN.saturating_sub(1);
                                let discard = (self.carry.len() - self.pos).saturating_sub(keep);
                                if discard > 0 {
                                    let before =
                                        recorder_host::stream_fold::FoldDamageWatch::before(
                                            &self.boundary_tracker,
                                        );
                                    self.boundary_tracker.note_framing_damage(
                                        record_start_abs,
                                        discard as u64,
                                        &message,
                                    );
                                    recorder_host::stream_fold::apply_fold_damage_step(
                                        before.after(&self.boundary_tracker, 0),
                                        &mut self.fold,
                                    );
                                    self.pos += discard;
                                    continue;
                                }
                                break;
                            }
                            recorder_host::stream_fold::FramingResync::NeedMoreData
                            | recorder_host::stream_fold::FramingResync::Exhausted => {}
                        }
                        if self.compressed {
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
                    break;
                }
                if self.identity_mode == StreamIdentityMode::Legacy
                    && (self.stream_validator.is_terminal()
                        || (header.sequence == 0
                            && header.domain_id == COMPACT_DOMAIN_STREAM_HEADER
                            && header.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER))
                {
                    if self.strict_validation_error.is_none() {
                        let message = format!(
                            "Legacy stream contains an IdentityV1 sequence-zero CID1 header at byte {record_start_abs}"
                        );
                        self.strict_validation_error = Some(message.clone());
                        let payload = &self.carry
                            [self.pos + COMPACT_RECORD_HEADER_LEN..self.pos + record_len];
                        let _ = self.stream_validator.validate_record(&header, payload);
                        let before = recorder_host::stream_fold::FoldDamageWatch::before(
                            &self.boundary_tracker,
                        );
                        self.boundary_tracker.note_framing_damage(
                            record_start_abs,
                            record_len as u64,
                            &message,
                        );
                        recorder_host::stream_fold::apply_fold_damage_step(
                            before.after(&self.boundary_tracker, header.game_hour_raw),
                            &mut self.fold,
                        );
                    } else {
                        self.boundary_tracker.note_framing_damage(
                            record_start_abs,
                            record_len as u64,
                            "records after a terminal stream-identity failure",
                        );
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
                // A malformed DOMAIN_DATA_REPEAT is a TOLERATED loss, not a
                // certification boundary violation: the strict error must
                // not latch for it (Codex review round 16, finding 4). Full
                // DATA keeps the stricter rule.
                let tolerated_repeat_error = envelope_error.is_some()
                    && header.record_type
                        == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT;
                if !tolerated_repeat_error && self.strict_validation_error.is_none() {
                    self.strict_validation_error = envelope_error.clone();
                }
                // SAME RECONSTRUCTION AS THE SPOOLED FOLD ABOVE, through the same
                // `WorldFold` baselines (Codex review round 7, finding 1) — but
                // PROVE, DON'T ADVANCE: the cursor validates through the fold's
                // READ-ONLY probe and passes RAW ON-WIRE bytes plus the ORIGINAL
                // flag word to `ingest_record`, whose internal apply performs
                // the ONE true advance (Codex review round 16, finding 2). An
                // advancing probe here would store grammar in the repeat
                // baseline and refuse every honest Delta-resolution repeat.
                let mut downstream_flags = header.record_flags;
                let mut reconstructed: Option<Vec<u8>> = None;
                let mut reconstruction_failure: Option<String> = None;
                if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
                    && envelope_error.is_none()
                {
                    match self.fold.probe_domain_data(
                        header.sequence,
                        header.game_hour_raw,
                        header.domain_id,
                        header.record_flags,
                        payload,
                    ) {
                        Ok(value) => {
                            if value.is_some() {
                                downstream_flags &=
                                    !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA;
                            }
                            reconstructed = value;
                        }
                        Err(reason) => {
                            reconstruction_failure = Some(format!(
                                "PSD1 reconstruction failed at byte {record_start_abs}: {reason}"
                            ))
                        }
                    }
                } else if header.record_type
                    == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
                    && envelope_error.is_none()
                {
                    // A refused repeat must reach this cursor's exclusion
                    // ledger, not only the fold's malformed-hour note (Codex
                    // review round 10, finding 4): probe every check
                    // `ingest_record` would apply, read-only — and keep the
                    // RESOLVED payload so its grammar is validated exactly
                    // like a full record's (Codex review round 21).
                    match self.fold.verify_domain_data_repeat(
                        header.domain_id,
                        header.record_flags,
                        payload,
                    ) {
                        Ok(resolved) => {
                            reconstructed = Some(resolved);
                            downstream_flags &=
                                !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA;
                        }
                        Err(reason) => {
                            reconstruction_failure = Some(format!(
                                "DOMAIN_DATA_REPEAT composition refused at byte \
                                 {record_start_abs}: {reason}"
                            ))
                        }
                    }
                }
                let folded_payload = reconstructed.as_deref().unwrap_or(payload);
                let exact_payload_error = match (header.record_type, envelope_error.as_ref()) {
                    (COMPACT_RECORD_TYPE_DOMAIN_DATA, None) => {
                        reconstruction_failure.clone().or_else(|| {
                            validate_domain_data_payload_at(
                                header.domain_id,
                                folded_payload,
                                downstream_flags,
                                header.sequence,
                                header.game_hour_raw,
                                record_start_abs,
                            )
                            .err()
                        })
                    }
                    // A repeat's payload is a volatile prefix plus a tail
                    // digest, never grammar; its resolved payload, however,
                    // IS domain grammar and validates exactly like a full
                    // record's (Codex review round 21).
                    (
                        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT,
                        None,
                    ) => reconstruction_failure.clone().or_else(|| {
                        validate_domain_data_payload_at(
                            header.domain_id,
                            folded_payload,
                            downstream_flags,
                            header.sequence,
                            header.game_hour_raw,
                            record_start_abs,
                        )
                        .err()
                    }),
                    _ => None,
                };
                let malformed_reason = matches!(
                    header.record_type,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA | compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
                )
                .then(|| {
                    envelope_error
                        .clone()
                        .or_else(|| exact_payload_error.clone())
                })
                .flatten();

                match compact_contract::KnownRecordType::try_from(header.record_type) {
                    Ok(
                        compact_contract::KnownRecordType::DomainData
                        | compact_contract::KnownRecordType::DomainDataRepeat,
                    ) => {
                        self.stats.data_records += 1;
                        *self
                            .stats
                            .per_domain_data_records
                            .entry(header.domain_id)
                            .or_default() += 1;
                        *self.cur_counts.entry(header.domain_id).or_default() += 1;
                    }
                    Ok(compact_contract::KnownRecordType::FrameEnd) => {
                        self.stats.frame_end_records += 1
                    }
                    Ok(compact_contract::KnownRecordType::DomainPerf) => {
                        self.stats.perf_records += 1
                    }
                    Ok(compact_contract::KnownRecordType::FreezeCensus) => {
                        self.stats.freeze_census_records += 1
                    }
                    Ok(
                        compact_contract::KnownRecordType::ContentIdentityCensus
                        | compact_contract::KnownRecordType::StateTopologyCensus
                        | compact_contract::KnownRecordType::StreamHeader
                        | compact_contract::KnownRecordType::DomainGap
                        | compact_contract::KnownRecordType::FrameRefused
                        | compact_contract::KnownRecordType::FrameDegraded
                        | compact_contract::KnownRecordType::CaptureTerminated
                        | compact_contract::KnownRecordType::ClockUnreadableInterval
                        | compact_contract::KnownRecordType::UntrustedClockRecovery,
                    )
                    | Err(_) => self.stats.other_records += 1,
                }
                let damage_before =
                    recorder_host::stream_fold::FoldDamageWatch::before(&self.boundary_tracker);
                if tolerated_repeat_error {
                    // A repeat that cannot ride the wire at all refuses here:
                    // for type 14 ANY malformed reason is ledger-worthy
                    // (Codex review round 11, finding 3), and the boundary
                    // tracker is told the record is TOLERATED so its rules do
                    // not latch a violation the walk has already forgiven
                    // (Codex review round 16, finding 4).
                    let reason = envelope_error.clone().unwrap_or_default();
                    self.stats.malformed_payload_records += 1;
                    self.stats
                        .excluded_domain_data_records
                        .push(ExcludedDomainDataRecord {
                            stream_offset: record_start_abs,
                            game_hour_raw: header.game_hour_raw,
                            domain_id: header.domain_id,
                            validation_reason: reason.clone(),
                        });
                    self.boundary_tracker
                        .observe_tolerated(&header, payload, record_start_abs);
                    recorder_host::stream_fold::apply_fold_damage_step(
                        damage_before.after(&self.boundary_tracker, header.game_hour_raw),
                        &mut self.fold,
                    );
                    // A refused repeat keeps the REPEAT arm's named
                    // provenance, not the generic malformed-payload one.
                    self.fold.refuse_domain_data_repeat(
                        header.game_hour_raw,
                        header.domain_id,
                        &reason,
                    );
                    self.pos += record_len;
                    continue;
                }
                self.boundary_tracker
                    .observe(&header, payload, record_start_abs);
                recorder_host::stream_fold::apply_fold_damage_step(
                    damage_before.after(&self.boundary_tracker, header.game_hour_raw),
                    &mut self.fold,
                );

                if let Some(reason) = malformed_reason {
                    if header.record_type
                        == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
                    {
                        // A composition-refused repeat (framing was fine) is
                        // still ledger-worthy here (Codex review round 11,
                        // finding 3) and keeps the REPEAT arm's named
                        // provenance.
                        self.stats.malformed_payload_records += 1;
                        self.stats
                            .excluded_domain_data_records
                            .push(ExcludedDomainDataRecord {
                                stream_offset: record_start_abs,
                                game_hour_raw: header.game_hour_raw,
                                domain_id: header.domain_id,
                                validation_reason: reason.clone(),
                            });
                        self.fold
                            .refuse_domain_data_repeat(header.game_hour_raw, header.domain_id, &reason);
                    } else if exact_payload_error.is_some() {
                        self.stats.malformed_payload_records += 1;
                        self.stats
                            .excluded_domain_data_records
                            .push(ExcludedDomainDataRecord {
                                stream_offset: record_start_abs,
                                game_hour_raw: header.game_hour_raw,
                                domain_id: header.domain_id,
                                validation_reason: reason.clone(),
                            });
                        // The producer published this record and moved BOTH
                        // baselines with it; the reader must too (Codex
                        // review round 17, finding 1).
                        self.fold.note_rejected_domain_data(
                            header.sequence,
                            header.game_hour_raw,
                            header.domain_id,
                            header.record_flags,
                            payload,
                        );
                        self.fold.record_malformed_domain_payload(
                            header.game_hour_raw,
                            header.domain_id,
                            payload,
                            &reason,
                        );
                    }
                } else {
                    // RAW ON-WIRE bytes plus the ORIGINAL flag word: the
                    // fold's internal apply performs the one true advance and
                    // stores the raw baseline (Codex review round 16,
                    // finding 2).
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
                        return Ok(());
                    }
                } else if header.record_type == COMPACT_RECORD_TYPE_FRAME_REFUSED {
                    self.close_refused_hour();
                }
            }

            self.compact_carry();
            let framed = self.carry.len();
            let needed = framed.saturating_add(CHUNK);
            if self.carry.capacity() < needed {
                let capacity = self.carry.capacity();
                let geometric = capacity.saturating_add(capacity / 2);
                let wanted = round_up_to_chunk(needed.max(geometric));
                self.carry.reserve_exact(wanted - framed);
            }
            self.framing_charge.set(self.carry.capacity() as u64);
            self.carry.resize(framed + CHUNK, 0);
            let read = self.reader.read(&mut self.carry[framed..]);
            let n = match read {
                Ok(n) => n,
                Err(error) => {
                    self.carry.truncate(framed);
                    return Err(DecodeError::Io(error));
                }
            };
            self.carry.truncate(framed + n);
            if n == 0 {
                self.finalize_done();
                return Ok(());
            }
            self.decoded_read = self.decoded_read.saturating_add(n as u64);
            self.source_hasher.update(&self.carry[framed..]);
            self.framing_charge.set(self.carry.capacity() as u64);
        }
    }

    fn compact_carry(&mut self) {
        if self.pos > 0 {
            self.processed += self.pos as u64;
            self.carry.drain(0..self.pos);
            self.pos = 0;
        }
    }

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
            self.strict_validation_error = shared_boundary_error;
        }
        self.stats.boundary_error = self.strict_validation_error.take();
        self.stats.ends_at_frame_end = self.stats.boundary_error.is_none();
        self.stats.hour_coverage = self.boundary_tracker.seal_coverage(&self.carry);
        self.done = true;
    }

    fn observe_strict_header(&mut self, header: &CompactRecordHeader, at: u64) {
        let sequence_error =
            strict_sequence_error(self.strict_previous_sequence, header.sequence, at);
        self.strict_previous_sequence = Some(header.sequence);

        let boundary_error = match compact_contract::KnownRecordType::try_from(header.record_type) {
            Err(unknown) => Some(format!(
                "unknown compact record type {} at byte {at}",
                unknown.0
            )),
            Ok(record_type) => match record_type {
            compact_contract::KnownRecordType::DomainData
            | compact_contract::KnownRecordType::DomainDataRepeat => match self.strict_open_hour {
                Some(open) if open != header.game_hour_raw => Some(format!(
                    "hour interleaving at byte {at}: data record for hour {} while hour {open} is open",
                    header.game_hour_raw
                )),
                None => {
                    self.strict_open_hour = Some(header.game_hour_raw);
                    None
                }
                Some(_) => None,
            },
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::DomainPerf => {
                match self.strict_open_hour {
                    Some(open) if open != header.game_hour_raw => Some(format!(
                        "hour interleaving at byte {at}: terminal domain outcome for hour {} while hour {open} is open",
                        header.game_hour_raw
                    )),
                    None => {
                        self.strict_open_hour = Some(header.game_hour_raw);
                        None
                    }
                    Some(_) => None,
                }
            }
            compact_contract::KnownRecordType::FrameEnd => match self.strict_open_hour {
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
            compact_contract::KnownRecordType::FrameRefused => {
                self.strict_open_hour = None;
                None
            }
            compact_contract::KnownRecordType::FreezeCensus
            | compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StateTopologyCensus
            | compact_contract::KnownRecordType::StreamHeader
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => None,
            },
        };
        if self.strict_validation_error.is_none() {
            self.strict_validation_error = sequence_error.or(boundary_error);
        }
    }

    fn close_refused_hour(&mut self) {
        self.cur_counts.clear();
    }

    fn drain_finished_hours(&mut self, next_hour_start: u64) {
        let counts = std::mem::take(&mut self.cur_counts);
        let finished = self.fold.take_finished();
        let finished_canonical = self.fold.take_finished_canonical();
        let published_any = !finished.is_empty() || !finished_canonical.is_empty();
        for (hour_index, date, bytes) in finished {
            let cheap = !self.first_hour_seen;
            self.first_hour_seen = true;
            self.index_hours.push(HourOffset {
                hour_index,
                start_offset: self.cur_hour_start,
                cheap_seek_point: cheap,
            });
            self.stats.hours_emitted += 1;
            // guard rides with the hour and releases wherever it is dropped.
            let snapshot_charge = recorder_host::mem_account::Charge::new(
                recorder_host::mem_account::Component::InflightSnapshots,
                bytes.capacity() as u64,
            );
            self.ready.push_back(WorldHour {
                hour_index,
                date,
                snapshot: bytes,
                canonical: None,
                domain_data_counts: counts.clone(),
                snapshot_charge,
            });
        }
        for (hour_index, date, prepared) in finished_canonical {
            let cheap = !self.first_hour_seen;
            self.first_hour_seen = true;
            self.index_hours.push(HourOffset {
                hour_index,
                start_offset: self.cur_hour_start,
                cheap_seek_point: cheap,
            });
            self.stats.hours_emitted += 1;
            let snapshot_charge = recorder_host::mem_account::Charge::new(
                recorder_host::mem_account::Component::InflightSnapshots,
                prepared.value_heap_bytes(),
            );
            self.ready.push_back(WorldHour {
                hour_index,
                date,
                snapshot: Vec::new(),
                canonical: Some(prepared),
                domain_data_counts: counts.clone(),
                snapshot_charge,
            });
        }
        self.hour_index_charge
            .set((self.index_hours.capacity() * std::mem::size_of::<HourOffset>()) as u64);
        self.cur_hour_start = next_hour_start;
        if published_any {
            self.published_hour_coverage_end = next_hour_start;
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourOffset {
    pub hour_index: i64,
    pub start_offset: u64,
    pub cheap_seek_point: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamIndex {
    pub source: CertifiedSourceIdentity,
    pub game_id: String,
    pub identity_mode: StreamIdentityMode,
    pub hours: Vec<HourOffset>,
    pub excluded: Vec<ExcludedHour>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludedHour {
    pub hour_index: i64,
    pub start_offset: u64,
    pub end_offset: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSourceIdentity {
    pub kind: String,
    pub canonical_path: String,
    pub decoded_len: u64,
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
        if self.hours.is_empty() {
            return Err("certified index contains no emitted hours".to_string());
        }
        let mut previous_offset = None;
        for (position, hour) in self.hours.iter().enumerate() {
            if hour.start_offset >= self.source.decoded_len {
                return Err(format!(
                    "hour at position {position} has out-of-range start_offset {} for decoded length {}",
                    hour.start_offset, self.source.decoded_len
                ));
            }
            if hour.cheap_seek_point != (hour.start_offset == 0) {
                return Err(format!(
                    "hour at position {position} claims cheap_seek_point={} at decoded offset {}; \
                     a cheap seek point is exactly an hour whose records start at offset zero",
                    hour.cheap_seek_point, hour.start_offset
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
        self.validate_hour_account()
    }

    fn validate_hour_account(&self) -> Result<(), String> {
        #[derive(Clone, Copy)]
        enum Kind {
            Served,
            Hole { end: u64 },
        }
        let mut merged: Vec<(i64, u64, Kind)> =
            Vec::with_capacity(self.hours.len() + self.excluded.len());
        merged.extend(
            self.hours
                .iter()
                .map(|hour| (hour.hour_index, hour.start_offset, Kind::Served)),
        );
        for hole in &self.excluded {
            if hole.reason.trim().is_empty() {
                return Err(format!(
                    "excluded hour {} names no reason; an injured hour is never guessed, only \
                     named",
                    hole.hour_index
                ));
            }
            if hole.start_offset > hole.end_offset || hole.end_offset > self.source.decoded_len {
                return Err(format!(
                    "excluded hour {} declares the out-of-range byte range [{}, {})",
                    hole.hour_index, hole.start_offset, hole.end_offset
                ));
            }
            merged.push((
                hole.hour_index,
                hole.start_offset,
                Kind::Hole {
                    end: hole.end_offset,
                },
            ));
        }
        merged.sort_by_key(|(hour_index, _, _)| *hour_index);

        let mut expected_start = 0u64;
        let mut previous_hour: Option<i64> = None;
        for (position, (hour_index, start, kind)) in merged.iter().enumerate() {
            if previous_hour.is_some_and(|previous| *hour_index <= previous) {
                return Err(format!(
                    "certified index names game hour {hour_index} twice, or out of order"
                ));
            }
            if *start != expected_start {
                return Err(format!(
                    "game hour {hour_index} starts at decoded offset {start} but the hours before \
                     it account for the source through {expected_start}"
                ));
            }
            expected_start = match kind {
                Kind::Hole { end } => *end,
                Kind::Served => match merged.get(position + 1) {
                    Some((next_hour, next_start, _)) => {
                        if next_start <= start {
                            return Err(format!(
                                "served game hour {hour_index} is empty: game hour {next_hour} \
                                 after it starts at the same decoded offset {next_start}"
                            ));
                        }
                        *next_start
                    }
                    None => self.source.decoded_len,
                },
            };
            previous_hour = Some(*hour_index);
        }
        if expected_start != self.source.decoded_len {
            return Err(format!(
                "certified index accounts for the source through {expected_start} of {} decoded \
                 byte(s)",
                self.source.decoded_len
            ));
        }
        Ok(())
    }

    pub fn served_end_offsets(&self) -> Vec<u64> {
        let mut boundaries: Vec<u64> = self
            .hours
            .iter()
            .map(|hour| hour.start_offset)
            .chain(self.excluded.iter().map(|hole| hole.start_offset))
            .collect();
        boundaries.sort_unstable();
        let mut ends = Vec::with_capacity(self.hours.len());
        let mut next = 0usize;
        for hour in &self.hours {
            while next < boundaries.len() && boundaries[next] <= hour.start_offset {
                next += 1;
            }
            ends.push(
                boundaries
                    .get(next)
                    .copied()
                    .unwrap_or(self.source.decoded_len),
            );
        }
        ends
    }

    pub fn hour_range(&self) -> Option<(i64, i64)> {
        match (self.hours.first(), self.hours.last()) {
            (Some(a), Some(b)) => Some((a.hour_index, b.hour_index)),
            _ => None,
        }
    }

    pub fn cheap_seek_points(&self) -> Vec<i64> {
        self.hours
            .iter()
            .filter(|h| h.cheap_seek_point)
            .map(|h| h.hour_index)
            .collect()
    }

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
            "excluded_hours": self.excluded.iter().map(|h| serde_json::json!({
                "hour_index": h.hour_index,
                "start_offset": h.start_offset,
                "end_offset": h.end_offset,
                "reason": h.reason,
            })).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(v: &serde_json::Value) -> Option<StreamIndex> {
        let index_schema = v.get("index_schema")?.as_str()?;
        if index_schema != CERTIFIED_INDEX_SCHEMA {
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
        let identity_mode = match v.get("identity_mode").and_then(|mode| mode.as_str()) {
            Some("legacy") => StreamIdentityMode::Legacy,
            Some("identity_v1") => StreamIdentityMode::IdentityV1,
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
        let mut excluded = Vec::new();
        for hole in v.get("excluded_hours")?.as_array()? {
            excluded.push(ExcludedHour {
                hour_index: hole.get("hour_index")?.as_i64()?,
                start_offset: hole.get("start_offset")?.as_u64()?,
                end_offset: hole.get("end_offset")?.as_u64()?,
                reason: hole.get("reason")?.as_str()?.to_string(),
            });
        }
        let index = StreamIndex {
            source,
            game_id,
            identity_mode,
            hours,
            excluded,
        };
        index.validate_structure().ok()?;
        Some(index)
    }
}

pub fn build_index(source: &Source, opts: &OpenOptions) -> Result<StreamIndex, OpenError> {
    build_index_with_identity_mode(source, opts, StreamIdentityMode::Legacy)
}

pub fn build_index_with_identity_mode(
    source: &Source,
    opts: &OpenOptions,
    identity_mode: StreamIdentityMode,
) -> Result<StreamIndex, OpenError> {
    let mut cursor = source.open_cursor_with_identity_mode(opts, identity_mode)?;
    while let Some(h) = cursor.next_hour() {
        h.map_err(|e| match e {
            DecodeError::Io(io) => OpenError::Io(io),
        })?;
    }
    cursor
        .take_index()
        .map_err(|e| OpenError::Certification(e.to_string()))
}

pub fn seek_to_hour(
    source: &Source,
    index: &StreamIndex,
    target: i64,
    opts: &OpenOptions,
) -> Result<WorldHour, SeekError> {
    index
        .validate_structure()
        .map_err(SeekError::InvalidIndex)?;
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
    if let Some(hole) = certified
        .excluded
        .iter()
        .find(|hole| hole.hour_index == target)
    {
        return Err(SeekError::HourExcluded {
            hour_index: target,
            reason: hole.reason.clone(),
        });
    }
    if !certified.hours.iter().any(|row| row.hour_index == target) {
        return Err(SeekError::HourNotFound(target));
    }
    found.ok_or(SeekError::HourNotFound(target))
}

#[cfg(test)]
mod tests;
