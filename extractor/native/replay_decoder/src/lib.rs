//! `replay_decoder` — the shared, pull-based decoder for the game compact
//! record stream.
//!
//! # Why this crate exists
//!
//! The canonical full-detail record of a recording is the compact typed
//! **stream** (`collector_compact_stream`), not the per-hour folded snapshot
//! JSONs that `.battleplan` containers currently also ship. Today only the
//! recorder host knows how to turn that stream into world states; the renderer
//! (`replay_interface/native/replay_core`) can only read the pre-folded snapshot
//! entries. This crate is the thin, dependency-free-of-internals facade that
//! lets an external consumer decode the stream directly, so production
//! containers can eventually drop the redundant snapshot JSONs (~72% of bytes).
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
//! The world-state JSON bytes this crate yields are **byte-identical** to the
//! snapshot entries a container ships today, so `replay_core`'s `world.rs`
//! parsing keeps working unchanged.
//!
//! # Memory model
//!
//! Iteration is bounded: each finished game-hour is drained from the fold the
//! moment its `FRAME_END` closes ([`WorldFold::take_finished`]) and handed to
//! the caller; no previous hour is retained. Peak memory is
//! `O(current fold state + one hour's snapshot)`, independent of stream length.
//! (The container source is the one exception: it materializes the verified
//! stream entry in RAM first — see [`open`] — because the container reader has
//! no streaming-entry API yet. Use a standalone raw or zstd `collector_stream.bin`
//! for large fold passes.)
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
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, CompactRecordHeader,
    RecordFramingError, ASSIGNED_COMPACT_DOMAINS, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END,
};
use recorder_host::stream_fold::{
    strict_stream_certification_verdict, validate_domain_data_payload, BoundaryTracker,
};
use recorder_host::world_fold::{schema_version, WorldFold};
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::{to_hex, Sha256};

/// Read granularity for the streaming raw-stream path. Bounded, independent of
/// stream length (mirrors the production streaming fold's chunk size).
const CHUNK: usize = 8 * 1024 * 1024;

/// ZIP local-file header magic (`PK\x03\x04`) — a `.battleplan` container.
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";

/// Persisted format marker for a source-bound, fully certified stream index.
const CERTIFIED_INDEX_SCHEMA: &str = "h4cr-certified-index-v1";

/// Manifest role of the compact stream inside a `.battleplan` container.
pub const STREAM_ROLE: &str = "collector_compact_stream";

/// Default fold game id for a raw stream that carries no container manifest.
/// (Containers use their manifest `game_id` unless [`OpenOptions::game_id`]
/// overrides it.) It participates in the folded snapshot's `game_id` field and
/// therefore in the per-hour hash, so identical stream bytes folded under the
/// same id hash identically.
pub const DEFAULT_GAME_ID: &str = "replay";

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

/// Open a raw `collector_stream.bin` explicitly (skips kind auto-detection).
pub fn open_stream_file(
    path: impl AsRef<Path>,
    opts: &OpenOptions,
) -> Result<HourCursor, OpenError> {
    Source::StreamFile(path.as_ref().to_path_buf()).open_cursor(opts)
}

/// Open a `.battleplan` container explicitly (its `collector_compact_stream`
/// entry).
pub fn open_container(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<HourCursor, OpenError> {
    Source::Container(path.as_ref().to_path_buf()).open_cursor(opts)
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
    HourCursor::from_reader(Box::new(reader), total_len, game_id.into())
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
        1 => container
            .read_entry_bytes(&entries[0].path)
            .map_err(|e| OpenError::Container(format!("read {}: {e}", entries[0].path)))?,
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
    /// stricter: sequence starts at one and never wraps, every FRAME_END closes
    /// an open DATA hour, and the final record is FRAME_END.
    strict_previous_sequence: Option<u64>,
    strict_open_hour: Option<i32>,
    strict_last_record_type: Option<u16>,
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
    fn from_reader(reader: Box<dyn Read>, total_len: u64, game_id: String) -> HourCursor {
        Self::from_reader_compressed(reader, total_len, game_id, false)
    }

    fn from_reader_with_source(
        reader: Box<dyn Read>,
        total_len: u64,
        game_id: String,
        index_source: Option<IndexSourceDescriptor>,
    ) -> HourCursor {
        Self::from_reader_compressed_with_source(reader, total_len, game_id, false, index_source)
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
    ) -> HourCursor {
        Self::from_reader_compressed_with_source(reader, total_len, game_id, compressed, None)
    }

    fn from_reader_compressed_with_source(
        reader: Box<dyn Read>,
        total_len: u64,
        game_id: String,
        compressed: bool,
        index_source: Option<IndexSourceDescriptor>,
    ) -> HourCursor {
        HourCursor {
            reader,
            fold: WorldFold::new(game_id.clone()),
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
            strict_last_record_type: None,
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
            hours: self.index_hours,
        };
        index
            .validate_structure()
            .map_err(IndexCertificationError::InvalidIndex)?;
        Ok(index)
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

                self.stats.records_total += 1;
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
                        &reason,
                    );
                } else {
                    // The single production fold. PERF / probe records carry no
                    // folded state and are ignored inside `ingest_record`.
                    self.fold.ingest_record(
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
        if self.strict_validation_error.is_none() {
            self.strict_validation_error = shared_boundary_error.or_else(|| {
                (self.strict_last_record_type != Some(COMPACT_RECORD_TYPE_FRAME_END)).then(|| {
                    match self.strict_last_record_type {
                        Some(record_type) => {
                            format!("stream final record type {record_type} is not FRAME_END")
                        }
                        None => "stream contains no records".to_string(),
                    }
                })
            });
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
            _ => None,
        };
        self.strict_last_record_type = Some(header.record_type);

        if self.strict_validation_error.is_none() {
            self.strict_validation_error = sequence_error.or(boundary_error);
        }
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
            "hours": self.hours.iter().map(|h| serde_json::json!({
                "hour_index": h.hour_index,
                "start_offset": h.start_offset,
                "cheap_seek_point": h.cheap_seek_point,
            })).collect::<Vec<_>>(),
        })
    }

    /// Parse an index previously produced by [`StreamIndex::to_json`].
    pub fn from_json(v: &serde_json::Value) -> Option<StreamIndex> {
        if v.get("index_schema")?.as_str()? != CERTIFIED_INDEX_SCHEMA {
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
    let mut cursor = source.open_cursor(opts)?;
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
    let mut cursor = source.open_cursor(opts).map_err(SeekError::Open)?;
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
