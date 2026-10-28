//! Independently compressed blocks, an indexed trailer, and atomic publication.

use self::check::Check;
use crate::identity::IdentityCodec;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    BranchId, FORMAT_VERSION, Hour, KEYFRAME_INTERVAL, SessionId,
    branches::{BranchReason, BranchStart, BranchTracker, Gap},
    codec::{Decoder, invalid, invalid_static, put_bytes, put_string},
    format::FormatVersion,
    frame::{DecodedFrame, Frame, FrameDecoder, Metrics, TABLE_COUNT, TableView},
    identity::{Identity, SessionStart, Verification},
    rows::{TableId, TableRows},
    scope::CaptureMode,
    spool::{
        ControlRecord, FrameMetadata, Record, RecordRef, Recovered, Recovery, SegmentState,
        SpoolRecord,
    },
};

const HEADER_MAGIC: &[u8; 4] = b"H4RP";
const TRAILER_MAGIC: &[u8; 4] = b"H4TR";
const FOOTER_MAGIC: &[u8; 4] = b"H4FT";
const FOOTER_SIZE: usize = 24;
mod aliases;
mod changes;
mod check;
mod commit;
pub use commit::Revision;
mod decoded_sections;
pub use decoded_sections::{SectionCacheCensus, SectionConsumer};
#[cfg(any(test, feature = "test-support"))]
pub use decoded_sections::{SectionDecode, WireDecode, WireRoute};
mod file_backed;
mod hour;
mod reader;
mod section_chunks;
mod stream;
pub use hour::{HourReader, PointResult};
pub use reader::{
    BlockReceipt, ReadStep, ReaderCensus, ReaderCheckpoint, ReaderExecution, ReaderSelection,
    SequentialReader,
};
pub use section_chunks::{SectionChunk, SectionChunks};
#[cfg(any(test, feature = "test-support"))]
pub mod test_work;
mod watched;
pub use changes::{BlockChange, QuietHours, TableFields};
pub use stream::{ReplayCursor, ReplayStream};
pub use watched::{BlockWatch, Fanout, Keep, Watched, Watching};
// A decoder allocation bound, not a campaign-size or frame-count limit.
// V1E branch 6 held 720 valid frames (~5.95 GB raw) between keyframes;
// writers must close smaller independently decodable blocks before this bound.
// A grouped block keeps each table's plain updates, its plain frame headers
// and its stored sections within it.
pub const MAX_BLOCK_RAW_BYTES: usize = replay_frame::envelope::MAX_RECORD_BYTES;

type FrameObserver<'a> =
    dyn FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()> + 'a;
type HourObserver<'a> = dyn FnMut(&Observed) -> io::Result<()> + 'a;

enum Observer<'a> {
    Nothing,
    Hours(&'a mut HourObserver<'a>),
    /// Only the sequential writer has whole frames to show.
    Frames(&'a mut FrameObserver<'a>),
}
type FrameReader<'a> = dyn FnMut(usize, &FrameMetadata) -> io::Result<Frame> + 'a;
type SharedReader<'a> = dyn Fn(usize, &FrameMetadata) -> io::Result<Frame> + Sync + 'a;

enum Frames<'a> {
    Kept,
    Shared(&'a SharedReader<'a>, &'a mut dyn FnMut() -> io::Result<()>),
    Ordered(&'a mut FrameReader<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateKind {
    Snapshot,
    Delta,
    Unchanged,
    Truncated,
    Unavailable,
    Incompatible,
    /// The capture mode leaves the table out.
    NotRecorded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incomplete {
    Truncated { missing: u32 },
    Unavailable { reason: String },
}

/// A written hour as finalization's summary reads it, so writers on any thread can report it.
#[derive(Clone, Debug)]
pub struct Observed {
    pub branch: BranchId,
    pub hour: Hour,
    pub metrics: Metrics,
    pub updates: [UpdateKind; TABLE_COUNT],
    pub sizes: [usize; TABLE_COUNT],
    pub incomplete: Vec<(TableId, Incomplete)>,
    pub field_gaps: Vec<(TableId, &'static str, usize)>,
    /// Humans in the hour's recorded roster; zero when it was not recorded.
    pub players: usize,
}

impl Observed {
    /// `previous`, the hour observed before on this branch, lends its gaps to each table unchanged since.
    pub fn new(
        frame: &Frame,
        decoded: &DecodedFrame,
        sizes: [usize; TABLE_COUNT],
        previous: Option<&Observed>,
    ) -> io::Result<Self> {
        let mut incomplete = Vec::new();
        let mut field_gaps = Vec::new();
        for table in TableId::ALL {
            if FormatVersion::CURRENT.table_refusal(table).is_some() {
                continue;
            }
            // The decoder holds an unchanged table's baseline rows, so they have its gaps.
            let baseline = match (&frame.tables[table.index()], previous) {
                (crate::frame::TableUpdate::Unchanged { baseline }, Some(previous))
                    if previous.branch == frame.branch && previous.hour == *baseline =>
                {
                    Some(previous)
                }
                _ => None,
            };
            let rows = match &*decoded.tables[table.index()] {
                TableView::Complete(rows) => rows,
                TableView::Truncated { rows, missing } => {
                    engine_allocation::reserve(&mut incomplete, 1)?;
                    incomplete.push((table, Incomplete::Truncated { missing: *missing }));
                    rows
                }
                TableView::Unavailable { reason } => {
                    engine_allocation::reserve(&mut incomplete, 1)?;
                    incomplete.push((
                        table,
                        Incomplete::Unavailable {
                            reason: engine_allocation::string(reason)?,
                        },
                    ));
                    continue;
                }
                // Expected: not incomplete.
                TableView::NotRecorded { .. } => continue,
            };
            if let Some(previous) = baseline {
                for gap in previous.field_gaps.iter().filter(|(id, ..)| *id == table) {
                    engine_allocation::reserve(&mut field_gaps, 1)?;
                    field_gaps.push(*gap);
                }
                continue;
            }
            for (field, count) in rows.field_gaps() {
                engine_allocation::reserve(&mut field_gaps, 1)?;
                field_gaps.push((table, field, count));
            }
        }
        Ok(Self {
            branch: frame.branch,
            hour: frame.hour,
            metrics: frame.metrics.clone(),
            updates: std::array::from_fn(|index| match &frame.tables[index] {
                crate::frame::TableUpdate::Snapshot(_) => UpdateKind::Snapshot,
                crate::frame::TableUpdate::Delta { .. } => UpdateKind::Delta,
                crate::frame::TableUpdate::Unchanged { .. } => UpdateKind::Unchanged,
                crate::frame::TableUpdate::Truncated { .. } => UpdateKind::Truncated,
                crate::frame::TableUpdate::Unavailable { .. } => UpdateKind::Unavailable,
                crate::frame::TableUpdate::Incompatible { .. } => UpdateKind::Incompatible,
                crate::frame::TableUpdate::NotRecorded { .. } => UpdateKind::NotRecorded,
            }),
            sizes,
            incomplete,
            field_gaps,
            players: match &*decoded.tables[TableId::Players.index()] {
                TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
                TableView::Unavailable { .. } | TableView::NotRecorded { .. } => 0,
            },
        })
    }

    pub fn unavailable(&self, table: TableId) -> Option<&str> {
        self.incomplete
            .iter()
            .find_map(|(id, incomplete)| match incomplete {
                Incomplete::Unavailable { reason } if *id == table => Some(reason.as_str()),
                _ => None,
            })
    }
}
type TrailerSummary = (
    Option<ControlRecord>,
    Option<(Hour, Hour)>,
    CaptureMode,
    Vec<SessionId>,
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockIndex {
    pub branch: BranchId,
    pub first_hour: Hour,
    pub last_hour: Hour,
    pub offset: u64,
    pub length: u64,
    pub raw_length: u64,
}

#[derive(Debug)]
pub enum SeekResult {
    Frame(Box<DecodedFrame>),
    Gap {
        branch: BranchId,
        hour: Hour,
        reason: String,
    },
}

/// Damage may have no trustworthy hour; keep its segment and byte position instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryNote {
    pub session: Option<u64>,
    pub segment: u32,
    pub valid_bytes: u64,
    pub reason: String,
}

impl RecoveryNote {
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        if self.reason.trim().is_empty() {
            return Err(invalid("empty recovery reason"));
        }
        out.push(u8::from(self.session.is_some()));
        if let Some(session) = self.session {
            out.extend_from_slice(&session.to_le_bytes());
        }
        out.extend_from_slice(&self.segment.to_le_bytes());
        out.extend_from_slice(&self.valid_bytes.to_le_bytes());
        put_string(out, &self.reason)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let session = if input.bool()? {
            Some(input.u64()?)
        } else {
            None
        };
        let note = Self {
            session,
            segment: input.u32()?,
            valid_bytes: input.u64()?,
            reason: input.string()?,
        };
        if note.reason.trim().is_empty() {
            return Err(invalid("empty recovery reason"));
        }
        Ok(note)
    }
}

/// Metadata is parsed when opened; seeking inflates only the selected block.
/// The byte backing can later become a mapping without changing block semantics.
pub struct Replay {
    bytes: Vec<u8>,
    decoded_sections: decoded_sections::Cache,
    anchor_candidates: grouped::CandidateCache,
    file: Option<File>,
    held_length: u64,
    /// The header version, validated before decoding any frames.
    pub format: FormatVersion,
    pub campaign_id: Vec<u8>,
    pub identity: Identity,
    pub keyframe_interval: u32,
    /// Every session's capture mode: one per file.
    pub capture_mode: CaptureMode,
    pub sessions: Vec<SessionStart>,
    pub branches: Vec<BranchStart>,
    pub gaps: Vec<Gap>,
    pub controls: Vec<ControlRecord>,
    pub recovery_notes: Vec<RecoveryNote>,
    /// Read-time damage diagnostics; never written into the recording.
    pub warnings: Vec<String>,
    pub damaged_parts: usize,
    pub damaged_range: Option<(Hour, Hour)>,
    pub index: Vec<BlockIndex>,
    /// Each index entry's position among the file's entries, which opening
    /// may withhold; grouped keyframe anchors count the file's blocks.
    ordinals: Vec<usize>,
    /// Per block: every check of a whole read has passed, so a later read may stop
    /// partway through it.
    verified: Vec<AtomicBool>,
    header_end: usize,
    /// The newest trailer's first byte.
    trailer_start: usize,
    /// The commit slot's first byte, in formats that have one.
    slot: Option<usize>,
    revision: Revision,
    /// Each earlier trailer with its footer, which lie among the blocks.
    regions: Vec<(u64, u64)>,
    /// The trailer the newest one adds to.
    previous: Option<(u64, u64)>,
    /// The sessions, branches, gaps, controls and recovery notes earlier trailers hold.
    earlier: [usize; 5],
    /// Opened to be added to in place.
    appendable: bool,
    append_path: Option<PathBuf>,
    partial_control: Option<usize>,
}

/// Exact validated prior hours and provenance, without replay bytes or decoded rows.
/// Build this, and any tracker made from it, before starting a bounded capture queue.
#[derive(Clone, Debug)]
pub struct CampaignHistory {
    pub campaign_id: Vec<u8>,
    pub identity: Identity,
    pub sessions: Vec<SessionStart>,
    pub branches: Vec<RecordedBranchHistory>,
    tails: Vec<(SessionId, SessionTail)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedBranchHistory {
    pub start: BranchStart,
    /// Actual frame hours only. Holes in the block index are never filled.
    pub hours: Vec<Hour>,
}

/// The last recorded frame in that process's block write order, not its largest hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionTail {
    pub branch: BranchId,
    pub hour: Hour,
    pub load_signal: Option<i64>,
}

impl CampaignHistory {
    pub fn session_tail(&self, session: SessionId) -> Option<SessionTail> {
        self.tails
            .iter()
            .find(|(id, _)| *id == session)
            .map(|(_, tail)| *tail)
    }

    /// Seed exact history and select a new process session or resume the same process.
    /// Same-process reattachment requires its complete saved session metadata to match.
    /// A caller whose content verification is pending can preload against the saved
    /// SessionStart, then compare it with the final verified session before use.
    pub fn tracker_for(&self, session: &SessionStart) -> io::Result<BranchTracker> {
        if !self.identity.same_content(&session.identity) {
            return Err(invalid_static(
                "existing campaign replay has different content",
            ));
        }
        if self
            .sessions
            .iter()
            .any(|old| old.capture_mode != session.capture_mode)
        {
            return Err(invalid_static(
                "existing campaign replay has another capture mode; record into a new file",
            ));
        }
        let previous = self.sessions.iter().find(|old| old.id == session.id);
        if previous.is_some_and(|old| old != session) {
            return Err(invalid_static(
                "same process session has conflicting collector or content metadata",
            ));
        }
        let mut tracker = BranchTracker::new();
        for branch in &self.branches {
            tracker.seed_recorded_branch(branch.start.clone(), branch.hours.iter().copied())?;
        }
        if previous.is_some() {
            let tail = self.session_tail(session.id).ok_or_else(|| {
                invalid_static("existing process session has no recorded hour to resume")
            })?;
            tracker.resume_session(session.id, tail.branch, tail.hour, tail.load_signal)?;
        } else {
            tracker.begin_session(session.id)?;
        }
        Ok(tracker)
    }
}

#[derive(Clone)]
struct CompressedBlock {
    branch: BranchId,
    first_hour: Hour,
    last_hour: Hour,
    raw_length: u64,
    bytes: Vec<u8>,
}

struct Pending {
    sessions: Vec<SessionStart>,
    branches: Vec<BranchStart>,
    gaps: Vec<Gap>,
    controls: Vec<ControlRecord>,
    blocks: Vec<CompressedBlock>,
}

impl Replay {
    pub fn block_ordinal(&self, position: usize) -> io::Result<usize> {
        self.ordinals.get(position).copied().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Physical block position is absent",
            )
        })
    }

    pub fn partial(&self) -> Option<&ControlRecord> {
        self.partial_control
            .and_then(|index| self.controls.get(index))
    }

    pub fn is_partial(&self) -> bool {
        self.partial().is_some()
    }

    /// The recorded event losses on `branch`, as the reads each lies between
    /// (`ControlRecord::EventLoss`): `(after_hour, hour)`.
    pub fn event_losses(&self, branch: BranchId) -> impl Iterator<Item = (Hour, Hour)> + '_ {
        self.controls
            .iter()
            .filter_map(move |control| match *control {
                ControlRecord::EventLoss {
                    branch: lost,
                    after_hour,
                    hour,
                    ..
                } if lost == branch => Some((after_hour, hour)),
                _ => None,
            })
    }

    /// Opening withheld records or found structural/checksum damage.
    /// Recorded partial captures and spool recovery notes alone are not damage here.
    pub fn needed_recovery(&self) -> bool {
        !self.warnings.is_empty()
    }

    pub fn require_undamaged(&self) -> io::Result<()> {
        if self.needed_recovery() {
            return Err(invalid(
                "This recording is damaged, so it can't be added to; start a new file.",
            ));
        }
        Ok(())
    }

    /// File-backed replays lack resident bytes; reject copying before creating any output.
    fn require_resident(&self) -> io::Result<()> {
        if self.is_file_backed() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a held recording can't be added to or written; open it whole",
            ));
        }
        Ok(())
    }

    /// Recording-list metadata reads the checksummed trailer without inflating blocks.
    /// Opening the replay still performs the full header and metadata validation.
    pub fn read_partial(path: &Path) -> io::Result<Option<ControlRecord>> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        Ok(Self::read_trailer_summary(&mut file, length)?.0)
    }

    /// Read terminal metadata from a seekable recording, using its actual byte length.
    pub fn read_partial_from(
        file: &mut (impl Read + Seek),
        length: u64,
    ) -> io::Result<Option<ControlRecord>> {
        if file.seek(SeekFrom::End(0))? != length {
            return Err(invalid("replay length differs from stream"));
        }
        Ok(Self::read_trailer_summary(file, length)?.0)
    }

    /// The campaign UUID a recording's header names, from its first bytes; `None` for another identity.
    pub fn read_campaign_uuid(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
        let mut prefix = [0; 14];
        input.read_exact(&mut prefix)?;
        expect_magic(&mut Decoder::new(&prefix[..4]), HEADER_MAGIC)?;
        FormatVersion::read(u16::from_le_bytes([prefix[4], prefix[5]]))?;
        let mut id = [0; 36];
        if u32::from_le_bytes(prefix[10..].try_into().unwrap()) as usize != id.len() {
            return Ok(None);
        }
        input.read_exact(&mut id)?;
        Ok(crate::identity::is_campaign_uuid(&id).then(|| id.to_vec()))
    }

    /// The first and last recorded hours across every branch, from the
    /// checksummed trailer's block index; `None` for a recording with no blocks.
    pub fn read_hour_span(path: &Path) -> io::Result<Option<(Hour, Hour)>> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        Ok(Self::read_trailer_summary(&mut file, length)?.1)
    }

    pub fn read_capture_mode(path: &Path) -> io::Result<CaptureMode> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        Ok(Self::read_trailer_summary(&mut file, length)?.2)
    }

    /// The ids of the sessions a recording holds, from its trailer without inflating blocks.
    pub fn read_session_ids(path: &Path) -> io::Result<Vec<SessionId>> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        Ok(Self::read_trailer_summary(&mut file, length)?.3)
    }

    /// The partial notice, when readable, and the recorded hour span.
    fn read_trailer_summary(
        file: &mut (impl Read + Seek),
        _length: u64,
    ) -> io::Result<TrailerSummary> {
        let located = commit::locate(&mut commit::Seeking(&mut Budgeted::new(file)))?;
        let format =
            FormatVersion::read(u16::from_le_bytes(located.header[4..6].try_into().unwrap()))?;
        let mut sessions = Vec::new();
        let mut controls = Vec::new();
        let mut warnings = Vec::new();
        let mut damaged_parts = 0;
        let mut span: Option<(Hour, Hour)> = None;
        for (_, bytes) in &located.trailers {
            let mut trailer = Decoder::new(bytes);
            trailer.format = format.version();
            expect_magic(&mut trailer, TRAILER_MAGIC)?;
            sessions.append(&mut decode_list(&mut trailer, SessionStart::decode)?);
            let _branches = decode_list(&mut trailer, BranchStart::decode)?;
            let _gaps = decode_list_recovering(
                &mut trailer,
                Gap::decode,
                "gap",
                &mut warnings,
                &mut damaged_parts,
            )?;
            controls.append(&mut decode_list_recovering(
                &mut trailer,
                |input| ControlRecord::decode_version(input, format),
                "control",
                &mut warnings,
                &mut damaged_parts,
            )?);
            let _recovery = decode_list_recovering(
                &mut trailer,
                RecoveryNote::decode,
                "recovery note",
                &mut warnings,
                &mut damaged_parts,
            )?;
            if located.slot.is_some() {
                trailer.take(16)?;
            }
            let count = trailer.count(40)?;
            for _ in 0..count {
                let _branch = trailer.u64()?;
                let first = trailer.i32()?;
                let last = trailer.i32()?;
                trailer.take(24)?;
                span = Some(match span {
                    Some((low, high)) => (low.min(first), high.max(last)),
                    None => (first, last),
                });
            }
        }
        let mode = file_capture_mode(&sessions)?;
        let ids = sessions.iter().map(|session| session.id).collect();
        // A readable trailer prefix still supplies the terminal notice and the
        // hour span. Full opening reports checksum failures and trailing bytes
        // as warnings.
        if validate_partial_controls(&controls).is_err() {
            return Ok((None, span, mode, ids));
        }
        for control in &controls {
            if !sessions
                .iter()
                .any(|session| session.id == control.session())
            {
                return Ok((None, span, mode, ids));
            }
        }
        let partial = controls
            .into_iter()
            .find(|control| matches!(control, ControlRecord::Partial { .. }));
        Ok((partial, span, mode, ids))
    }

    pub fn from_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Self> {
        let segments: Vec<_> = segments.iter().map(Vec::as_slice).collect();
        Self::from_record_slices(
            campaign_id,
            &segments,
            Observer::Nothing,
            Frames::Kept,
            MAX_BLOCK_RAW_BYTES,
            None,
        )
    }

    fn from_record_slices<R: SpoolRecord>(
        campaign_id: Vec<u8>,
        segments: &[&[R]],
        observe: Observer<'_>,
        frames: Frames<'_>,
        block_limit: usize,
        ahead: Option<Ahead>,
    ) -> io::Result<Self> {
        if campaign_id.is_empty() {
            return Err(invalid("empty campaign identity"));
        }
        let identity = segments
            .iter()
            .flat_map(|segment| segment.iter())
            .find_map(|record| match record.view() {
                RecordRef::SessionStart(session) => Some(session.identity.clone()),
                _ => None,
            })
            .ok_or_else(|| invalid("replay has no session identity"))?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(HEADER_MAGIC);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&KEYFRAME_INTERVAL.to_le_bytes());
        put_bytes(&mut bytes, &campaign_id)?;
        let mut encoded_identity = Vec::new();
        identity.encode(&mut encoded_identity)?;
        put_bytes(&mut bytes, &encoded_identity)?;
        let slot = bytes.len();
        bytes.extend_from_slice(&[0; commit::SLOT_SIZE]);
        let pending = collect(
            segments,
            &identity,
            &campaign_id,
            None,
            KEYFRAME_INTERVAL,
            observe,
            frames,
            block_limit,
            ahead,
        )?;
        let mut index = Vec::new();
        append_owned_blocks(&mut bytes, &mut index, pending.blocks)?;
        finish_file(
            &mut bytes,
            &pending.sessions,
            &pending.branches,
            &pending.gaps,
            &pending.controls,
            &[],
            &index,
        )?;
        commit::stamp_slot(&mut bytes, Some(slot))?;
        Self::open(bytes)
    }

    /// Existing saves are copied verbatim, and a trailer adding the new blocks follows them.
    pub fn append(&self, segments: &[Vec<Record>]) -> io::Result<Self> {
        let segments: Vec<_> = segments.iter().map(Vec::as_slice).collect();
        self.append_record_slices(&segments, Observer::Nothing, Frames::Kept, None)
    }

    fn append_record_slices<R: SpoolRecord>(
        &self,
        segments: &[&[R]],
        observe: Observer<'_>,
        frames: Frames<'_>,
        ahead: Option<Ahead>,
    ) -> io::Result<Self> {
        self.require_resident()?;
        self.require_undamaged()?;
        // Existing blocks keep their original contract. Never mix current
        // writer rows under an older header or rewrite that header as migration.
        if self.format != FormatVersion::CURRENT {
            return Err(invalid(format!(
                "Replay append requires format {FORMAT_VERSION}; this recording is format {}; open it read-only and record into a new file",
                self.format.version()
            )));
        }
        let pending = collect(
            segments,
            &self.identity,
            &self.campaign_id,
            Some(self),
            self.keyframe_interval,
            observe,
            frames,
            MAX_BLOCK_RAW_BYTES,
            ahead,
        )?;
        if self.slot.is_none() {
            return Err(invalid(format!(
                "Replay append requires format {FORMAT_VERSION}; this recording is format {}; open it read-only and record into a new file",
                self.format.version()
            )));
        }
        let mut bytes = self.bytes().to_vec();
        let mut index = Vec::new();
        append_owned_blocks(&mut bytes, &mut index, pending.blocks)?;
        commit::encode_trailer(
            &mut bytes,
            0,
            &pending.sessions,
            &pending.branches,
            &pending.gaps,
            &pending.controls,
            &[],
            Some((self.revision.trailer, self.revision.length)),
            &index,
        )?;
        commit::stamp_slot(&mut bytes, self.slot)?;
        Self::open(bytes)
    }

    pub fn from_recovered<R: SpoolRecord>(
        campaign_id: Vec<u8>,
        segments: &[(u32, Recovery<R>)],
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        Self::from_record_slices(
            campaign_id,
            &records,
            Observer::Nothing,
            Frames::Kept,
            MAX_BLOCK_RAW_BYTES,
            None,
        )?
        .add_recovery_notes(segments)
    }

    /// Observation does not publish the file; report success only after writing it atomically.
    pub fn from_recovered_with<R: SpoolRecord>(
        campaign_id: Vec<u8>,
        segments: &[(u32, Recovery<R>)],
        mut observe: impl FnMut(&Observed) -> io::Result<()>,
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        Self::from_record_slices(
            campaign_id,
            &records,
            Observer::Hours(&mut observe),
            Frames::Kept,
            MAX_BLOCK_RAW_BYTES,
            None,
        )?
        .add_recovery_notes(segments)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn append_recovered<R: SpoolRecord>(
        &self,
        segments: &[(u32, Recovery<R>)],
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        self.append_record_slices(&records, Observer::Nothing, Frames::Kept, None)?
            .add_recovery_notes(segments)
    }

    /// Observe only the new frames; existing compressed blocks are copied intact.
    #[cfg(any(test, feature = "test-support"))]
    pub fn append_recovered_with<R: SpoolRecord>(
        &self,
        segments: &[(u32, Recovery<R>)],
        mut observe: impl FnMut(&Observed) -> io::Result<()>,
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        self.append_record_slices(&records, Observer::Hours(&mut observe), Frames::Kept, None)?
            .add_recovery_notes(segments)
    }

    /// `read` restores recovered frames on any writer thread; `progress` runs here once per frame.
    pub fn from_recovered_with_reader(
        campaign_id: Vec<u8>,
        segments: &[(u32, Recovery<Recovered>)],
        read: impl Fn(u32, &FrameMetadata) -> io::Result<Frame> + Sync,
        mut progress: impl FnMut() -> io::Result<()>,
        mut observe: impl FnMut(&Observed) -> io::Result<()>,
        ahead: Option<Ahead>,
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        let numbers = segment_numbers(segments);
        let read_frame = |position: usize, frame: &FrameMetadata| read(numbers[position], frame);
        Self::from_record_slices(
            campaign_id,
            &records,
            Observer::Hours(&mut observe),
            Frames::Shared(&read_frame, &mut progress),
            MAX_BLOCK_RAW_BYTES,
            ahead,
        )?
        .add_recovery_notes(segments)
    }

    /// Append using the same bounded frame restoration as initial finalization.
    pub fn append_recovered_with_reader(
        &self,
        segments: &[(u32, Recovery<Recovered>)],
        read: impl Fn(u32, &FrameMetadata) -> io::Result<Frame> + Sync,
        mut progress: impl FnMut() -> io::Result<()>,
        mut observe: impl FnMut(&Observed) -> io::Result<()>,
        ahead: Option<Ahead>,
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        let numbers = segment_numbers(segments);
        let read_frame = |position: usize, frame: &FrameMetadata| read(numbers[position], frame);
        self.append_record_slices(
            &records,
            Observer::Hours(&mut observe),
            Frames::Shared(&read_frame, &mut progress),
            ahead,
        )?
        .add_recovery_notes(segments)
    }

    fn add_recovery_notes<R: SpoolRecord>(
        mut self,
        segments: &[(u32, Recovery<R>)],
    ) -> io::Result<Self> {
        self.require_undamaged()?;
        let notes = commit::recovery_notes(segments)?;
        if notes.is_empty() {
            return Ok(self);
        }
        self.recovery_notes.extend(notes);
        // The newest trailer is written again with the notes.
        let [sessions, branches, gaps, controls, recovery] = self.earlier;
        let blocks = self.index.len() - self.newest_blocks();
        self.bytes.truncate(self.trailer_start);
        commit::encode_trailer(
            &mut self.bytes,
            0,
            &self.sessions[sessions..],
            &self.branches[branches..],
            &self.gaps[gaps..],
            &self.controls[controls..],
            &self.recovery_notes[recovery..],
            self.previous,
            &self.index[blocks..],
        )?;
        commit::stamp_slot(&mut self.bytes, self.slot)?;
        Self::open(self.bytes)
    }

    /// The index entries the newest trailer adds.
    fn newest_blocks(&self) -> usize {
        self.index
            .iter()
            .rev()
            .take_while(|block| {
                self.regions
                    .last()
                    .is_none_or(|(_, end)| block.offset >= *end)
            })
            .count()
    }

    pub fn content_verified(&self) -> bool {
        matches!(self.identity.verification, Verification::Verified { .. })
            && self.sessions.iter().all(|session| {
                matches!(session.identity.verification, Verification::Verified { .. })
            })
    }

    pub fn bytes(&self) -> &[u8] {
        assert!(
            self.file.is_none(),
            "file-backed replay has no in-memory byte slice"
        );
        &self.bytes[..to_usize(self.revision.end).unwrap_or(self.bytes.len())]
    }

    /// Exact recoverable frame hours, never the index's range with holes filled.
    pub fn recorded_hours(&self, branch: BranchId) -> io::Result<Vec<Hour>> {
        let mut hours = Vec::new();
        for block in self.index.iter().filter(|block| block.branch == branch) {
            hours.extend(self.read_block(block)?.into_iter().map(|frame| frame.hour));
        }
        Ok(hours)
    }

    /// Validate every block and replay all its table baselines once, retaining only
    /// compact exact history. Consuming self releases compressed bytes, control rows
    /// and block payloads before this method returns to a recorder/collector worker.
    pub fn into_history(self) -> io::Result<CampaignHistory> {
        self.require_undamaged()?;
        let positions: BTreeMap<_, _> = self
            .branches
            .iter()
            .enumerate()
            .map(|(index, branch)| (branch.id, index))
            .collect();
        let mut branches: Vec<_> = self
            .branches
            .iter()
            .map(|start| RecordedBranchHistory {
                start: start.clone(),
                hours: Vec::new(),
            })
            .collect();
        let mut tails: Vec<(SessionId, SessionTail)> = Vec::new();
        for block in &self.index {
            let index = *positions
                .get(&block.branch)
                .ok_or_else(|| invalid("block refers to an unknown branch"))?;
            let history = &mut branches[index];
            let mut decoder = FrameDecoder::new();
            let mut load_signal = None;
            for frame in self.read_block(block)? {
                let decoded = decoder.advance(&frame)?;
                validate_campaign(decoded, &self.campaign_id)?;
                if history.hours.last().is_some_and(|hour| *hour >= frame.hour) {
                    return Err(invalid(
                        "recorded history hours do not increase within branch",
                    ));
                }
                history.hours.push(frame.hour);
                load_signal = match &*decoded.tables[TableId::Clock.index()] {
                    TableView::Complete(TableRows::Clock(rows)) => {
                        rows.first().map(|clock| clock.play_session_started_at)
                    }
                    TableView::Complete(_) => None,
                    TableView::Truncated { .. }
                    | TableView::Unavailable { .. }
                    | TableView::NotRecorded { .. } => None,
                };
            }
            let tail = SessionTail {
                branch: block.branch,
                hour: block.last_hour,
                load_signal,
            };
            if let Some((_, old)) = tails
                .iter_mut()
                .find(|(id, _)| *id == history.start.session)
            {
                *old = tail;
            } else {
                engine_allocation::reserve(&mut tails, 1)?;
                tails.push((history.start.session, tail));
            }
        }
        // Match the existing reattach fallback: latest matching nonempty witness
        // is used only when the last recorded frame has no readable Clock table.
        for control in self.controls.iter().rev() {
            if let ControlRecord::ClockWitness {
                session,
                hour,
                load_signal: Some(signal),
                ..
            } = control
                && let Some((_, tail)) = tails.iter_mut().find(|(id, _)| id == session)
                && tail.hour == *hour
                && tail.load_signal.is_none()
            {
                tail.load_signal = Some(*signal);
            }
        }
        Ok(CampaignHistory {
            campaign_id: self.campaign_id,
            identity: self.identity,
            sessions: self.sessions,
            branches,
            tails,
        })
    }

    pub fn open(bytes: Vec<u8>) -> io::Result<Self> {
        let located = commit::locate(&mut commit::Resident(&bytes))?;
        let mut replay = Self::open_metadata(located)?;
        replay.bytes = bytes;
        Ok(replay)
    }

    /// Container metadata validation; `located` carries what finding the trailers found
    /// (a trailer checksum failure).
    fn open_metadata(located: commit::Located) -> io::Result<Self> {
        let commit::Located {
            header: header_bytes,
            slot,
            trailers,
            revision,
            mut warnings,
        } = located;
        let mut header = Decoder::new(&header_bytes);
        expect_magic(&mut header, HEADER_MAGIC)?;
        let format = FormatVersion::read(header.u16()?)?;
        let keyframe_interval = header.u32()?;
        if keyframe_interval != KEYFRAME_INTERVAL {
            return Err(invalid("unsupported keyframe interval"));
        }
        let campaign_id = header.bytes()?;
        if campaign_id.is_empty() {
            return Err(invalid("empty campaign identity"));
        }
        let identity_bytes = header.bytes()?;
        let mut identity_decoder = Decoder::new(&identity_bytes);
        let identity = Identity::decode(&mut identity_decoder)?;
        identity_decoder.finish()?;
        if slot.is_some() {
            header.take(commit::SLOT_SIZE)?;
        }
        header.finish()?;
        let header_end = header_bytes.len();
        let mut sessions = Vec::new();
        let mut branches = Vec::new();
        let mut damaged_parts = 0;
        let mut gaps = Vec::new();
        let mut controls = Vec::new();
        let mut recovery_notes = Vec::new();
        let mut index = Vec::new();
        let mut earlier = [0; 5];
        let mut previous = None;
        for (position, (_, bytes)) in trailers.iter().enumerate() {
            if position + 1 == trailers.len() {
                earlier = [
                    sessions.len(),
                    branches.len(),
                    gaps.len(),
                    controls.len(),
                    recovery_notes.len(),
                ];
            }
            let mut trailer = Decoder::new(bytes);
            trailer.format = format.version();
            expect_magic(&mut trailer, TRAILER_MAGIC)?;
            sessions.append(&mut decode_list(&mut trailer, SessionStart::decode)?);
            branches.append(&mut decode_list(&mut trailer, BranchStart::decode)?);
            gaps.append(&mut decode_list_recovering(
                &mut trailer,
                Gap::decode,
                "gap",
                &mut warnings,
                &mut damaged_parts,
            )?);
            controls.append(&mut decode_list_recovering(
                &mut trailer,
                |input| ControlRecord::decode_version(input, format),
                "control",
                &mut warnings,
                &mut damaged_parts,
            )?);
            recovery_notes.append(&mut decode_list_recovering(
                &mut trailer,
                RecoveryNote::decode,
                "recovery note",
                &mut warnings,
                &mut damaged_parts,
            )?);
            if slot.is_some() {
                let link = (trailer.u64()?, trailer.u64()?);
                previous = (link != (0, 0)).then_some(link);
            }
            let count = trailer.count(40)?;
            engine_allocation::reserve(&mut index, count)?;
            for _ in 0..count {
                index.push(BlockIndex {
                    branch: trailer.u64()?,
                    first_hour: trailer.i32()?,
                    last_hour: trailer.i32()?,
                    offset: trailer.u64()?,
                    length: trailer.u64()?,
                    raw_length: trailer.u64()?,
                });
            }
            if trailer.remaining() != 0 {
                engine_allocation::reserve(&mut warnings, 1)?;
                warnings.push("trailing bytes in replay trailer".into());
            }
        }
        let capture_mode = file_capture_mode(&sessions)?;
        let trailer_start = trailers.last().map_or(header_end, |(start, _)| *start);
        let regions = commit::trailer_regions(&trailers);
        let mut damaged_range = None;
        let mut ordinals = Vec::new();
        let control_check = validate_partial_controls(&controls).and_then(|()| {
            if controls.iter().any(|control| {
                !sessions
                    .iter()
                    .any(|session| session.id == control.session())
            }) {
                Err(invalid("replay control refers to an unknown session"))
            } else {
                Ok(())
            }
        });
        if let Err(error) = control_check {
            engine_allocation::reserve(&mut warnings, 1)?;
            warnings.push(crate::codec::try_format(format_args!(
                "Unreadable controls: {error}"
            ))?);
            controls.clear();
            damaged_parts += 1;
        }
        validate_metadata(
            &identity,
            &sessions,
            &branches,
            &mut gaps,
            &controls,
            &mut index,
            &mut ordinals,
            header_end,
            trailer_start,
            &regions,
            keyframe_interval,
            &mut warnings,
            &mut damaged_parts,
            &mut damaged_range,
        )?;
        let partial_control = controls
            .iter()
            .position(|control| matches!(control, ControlRecord::Partial { .. }));
        let mut verified = engine_allocation::vec(index.len())?;
        for _ in &index {
            verified.push(AtomicBool::new(false));
        }
        Ok(Self {
            bytes: Vec::new(),
            decoded_sections: decoded_sections::Cache::default(),
            anchor_candidates: grouped::CandidateCache::default(),
            file: None,
            held_length: 0,
            format,
            campaign_id,
            identity,
            keyframe_interval,
            capture_mode,
            sessions,
            branches,
            gaps,
            controls,
            recovery_notes,
            warnings,
            damaged_parts,
            damaged_range,
            verified,
            index,
            ordinals,
            header_end,
            trailer_start,
            slot,
            revision,
            regions,
            previous,
            earlier,
            appendable: false,
            append_path: None,
            partial_control,
        })
    }

    /// Persistent named layout, meaning and lifetime gaps shared by every open path.
    pub fn compatibility_gaps(&self) -> Vec<String> {
        TableId::ALL
            .into_iter()
            .flat_map(|table| {
                self.format
                    .table_refusal(table)
                    .into_iter()
                    .chain(self.format.field_gaps(table))
            })
            .chain(self.format.nested_field_gaps())
            .collect()
    }

    pub fn seek(&self, branch: BranchId, hour: Hour) -> io::Result<SeekResult> {
        self.seek_selected(branch, hour, None)
    }

    /// [`Replay::seek`] reconstructing only `tables` (and the campaign identity every
    /// frame is checked against), reading its block one frame at a time up to the hour.
    pub fn seek_selected(
        &self,
        branch: BranchId,
        hour: Hour,
        tables: Option<&[TableId]>,
    ) -> io::Result<SeekResult> {
        self.seek_selected_checked(branch, hour, tables, &mut Check::disabled())
    }

    /// Selected seek with caller cancellation, retaining allocation/cancellation
    /// errors separately from actual recording damage.
    pub fn seek_selected_with(
        &self,
        branch: BranchId,
        hour: Hour,
        tables: Option<&[TableId]>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<SeekResult> {
        self.seek_selected_checked(branch, hour, tables, &mut Check::new(&mut check))
    }

    fn seek_selected_checked(
        &self,
        branch: BranchId,
        hour: Hour,
        tables: Option<&[TableId]>,
        check: &mut Check<'_>,
    ) -> io::Result<SeekResult> {
        HourReader::new().seek(self, branch, hour, tables, check)
    }

    /// Why `hour` of `branch` has no frame, as a seek of it names the gap.
    pub fn gap_reason(&self, branch: BranchId, hour: Hour) -> io::Result<String> {
        let mut reason = String::new();
        for gap in self
            .gaps
            .iter()
            .filter(|gap| gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour)
        {
            let text = &gap.reason;
            if text.is_empty() {
                continue;
            }
            let separator = if reason.is_empty() { "" } else { "; " };
            engine_allocation::reserve_string(&mut reason, separator.len() + text.len())?;
            reason.push_str(separator);
            reason.push_str(text);
        }
        if reason.is_empty() {
            engine_allocation::string("hour was not recorded")
        } else {
            Ok(reason)
        }
    }

    /// Decode one block for sequential playback, inspection, or history recovery.
    pub fn read_block(&self, index: &BlockIndex) -> io::Result<Vec<Frame>> {
        self.read_block_inner(index, None)
    }

    /// [`Replay::read_block`], decoding table sections on `parallel`'s
    /// worker threads. The same frames and the
    /// same first error; only a caller that owns the machine's spare cores asks
    /// for it (the viewer's playback read-ahead), never the collector.
    pub fn read_block_parallel(
        &self,
        index: &BlockIndex,
        parallel: &Parallel<'_>,
    ) -> io::Result<Vec<Frame>> {
        self.read_block_inner(index, Some(parallel))
    }

    /// Preserve full wire validation but do not retain unrelated table payloads.
    pub fn read_block_selected(
        &self,
        index: &BlockIndex,
        tables: &[crate::rows::TableId],
    ) -> io::Result<Vec<Frame>> {
        self.read_block_selected_inner(index, tables, None)
    }

    /// [`Replay::read_block_selected`], decoding a grouped block's tables on
    /// `parallel`'s workers. The same frames and the same first error.
    #[cfg(any(test, feature = "test-support"))]
    pub fn read_block_selected_parallel(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        parallel: &Parallel<'_>,
    ) -> io::Result<Vec<Frame>> {
        self.read_block_selected_inner(index, tables, Some(parallel))
    }

    fn read_block_selected_inner(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<Frame>> {
        let mut frames = Vec::new();
        self.visit_block_inner(index, Some(tables), parallel, &mut |frame| {
            engine_allocation::reserve(&mut frames, 1)?;
            frames.push(frame);
            Ok(true)
        })?;
        Ok(frames)
    }

    fn read_block_inner(
        &self,
        index: &BlockIndex,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<Frame>> {
        let block = self.grouped_block(index)?;
        let frames =
            check_block_frames(index, grouped::read(self, index, &block, None, parallel)?)?;
        self.mark_verified(index);
        Ok(frames)
    }

    /// Each selected frame of an admitted block in order, until `visit` returns false.
    pub fn visit_block(
        &self,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        mut visit: impl FnMut(Frame) -> io::Result<bool>,
    ) -> io::Result<()> {
        self.visit_block_inner(index, selected, None, &mut visit)
    }

    fn visit_block_inner(
        &self,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        parallel: Option<&Parallel<'_>>,
        // One copy of the block walk serves every visitor, rather than one each.
        visit: &mut dyn FnMut(Frame) -> io::Result<bool>,
    ) -> io::Result<()> {
        let block = self.grouped_block(index)?;
        let frames = check_block_frames(
            index,
            grouped::read(self, index, &block, selected, parallel)?,
        )?;
        for frame in frames {
            if !visit(frame)? {
                break;
            }
        }
        if selected.is_some() && !self.is_verified(index) {
            grouped::verify(&block, index)?;
        }
        self.mark_verified(index);
        Ok(())
    }

    fn mark_verified(&self, index: &BlockIndex) {
        if let Some(position) = self.block_position(index) {
            self.verified[position].store(true, Ordering::Release);
        }
    }

    /// A current grouped block's stored bytes; other layouts are refused.
    fn grouped_block(&self, index: &BlockIndex) -> io::Result<Cow<'_, [u8]>> {
        let _cpu = crate::cpu::scope("decode", None);
        if index.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
            return Err(invalid("block exceeds raw size limit"));
        }
        let block = self.block_bytes(index)?;
        if !grouped::is_grouped(&block) {
            return Err(invalid("unsupported replay block magic"));
        }
        Ok(block)
    }

    /// A block's stored bytes.
    fn block_bytes(&self, index: &BlockIndex) -> io::Result<Cow<'_, [u8]>> {
        if self.file.is_some() {
            return self.held_block_bytes(index).map(Cow::Owned);
        }
        let start = to_usize(index.offset)?;
        let end = to_usize(
            index
                .offset
                .checked_add(index.length)
                .ok_or_else(|| invalid("block offset overflow"))?,
        )?;
        self.bytes
            .get(start..end)
            .map(Cow::Borrowed)
            .ok_or_else(|| invalid("block outside replay"))
    }

    /// The compressed sections of a current grouped block.
    pub fn block_sections(&self, index: &BlockIndex) -> io::Result<Option<Vec<SectionSize>>> {
        let block = self.block_bytes(index)?;
        if !grouped::is_grouped(&block) {
            return Err(invalid("unsupported replay block magic"));
        }
        grouped::sections(&block, index).map(Some)
    }

    /// The table sections split into their columns, each also compressed alone.
    pub fn block_columns(&self, index: &BlockIndex) -> io::Result<Option<Vec<ColumnSizes>>> {
        let block = self.block_bytes(index)?;
        if !grouped::is_grouped(&block) {
            return Err(invalid("unsupported replay block magic"));
        }
        grouped::section_columns(&block, index).map(Some)
    }

    /// Every section of a grouped block with its plain bytes compressed again
    /// at each of `levels` (zstd, windows within what readers accept).
    pub fn block_levels(
        &self,
        index: &BlockIndex,
        levels: &[i32],
    ) -> io::Result<Option<Vec<SectionLevels>>> {
        let block = self.block_bytes(index)?;
        if !grouped::is_grouped(&block) {
            return Err(invalid("unsupported replay block magic"));
        }
        grouped::section_levels(&block, index, levels).map(Some)
    }

    /// This recording's frames written again by the current block writer, in
    /// memory, keeping its sessions, branches, gaps, controls and recovery
    /// notes. Size attribution compares layouts with it; the frames, their
    /// format and their meaning are unchanged, so only current-format,
    /// undamaged recordings can be written again.
    pub fn reencoded(
        &self,
        mut observe: impl FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()>,
    ) -> io::Result<Self> {
        if self.format != FormatVersion::CURRENT {
            return Err(invalid(format!(
                "only format {FORMAT_VERSION} frames can be written again; this recording is format {}",
                self.format.version()
            )));
        }
        // Opening drops damaged blocks from the index; a rewrite would lose them for good.
        self.require_undamaged()?;
        // One segment per session with frames: its start, then each branch's
        // start and frames in block order. The frames are metadata here and
        // are read back from this recording block by block; the trailer is
        // copied, so controls and gaps need no place among the frames.
        let mut segments: Vec<Vec<Recovered>> = Vec::new();
        let mut session = None;
        let mut branch = None;
        for block in &self.index {
            let start = self
                .branches
                .iter()
                .find(|start| start.id == block.branch)
                .ok_or_else(|| invalid("block refers to an unknown branch"))?;
            if session != Some(start.session) {
                let started = self
                    .sessions
                    .iter()
                    .find(|session| session.id == start.session)
                    .ok_or_else(|| invalid("branch refers to an unknown session"))?;
                segments.push(vec![Recovered::SessionStart(started.clone())]);
                session = Some(start.session);
                branch = None;
            }
            let records = segments.last_mut().unwrap();
            if branch != Some(block.branch) {
                records.push(Recovered::BranchStart(start.clone()));
                branch = Some(block.branch);
            }
            for frame in self.read_block_selected(block, &[])? {
                let placeholder = crate::frame::TableUpdate::Unchanged {
                    baseline: frame.hour,
                };
                records.push(Recovered::Frame(FrameMetadata {
                    branch: frame.branch,
                    hour: frame.hour,
                    keyframe: frame.keyframe,
                    placement: Default::default(),
                    campaign: placeholder.clone(),
                    clock: placeholder,
                }));
            }
        }
        if segments.is_empty() {
            return Self::open(self.bytes().to_vec());
        }
        // Frames in block order, one block in memory.
        let mut blocks = self.index.iter();
        let mut frames: std::vec::IntoIter<Frame> = Vec::new().into_iter();
        let mut read = |_: usize, wanted: &FrameMetadata| -> io::Result<Frame> {
            loop {
                if let Some(frame) = frames.next() {
                    if (frame.branch, frame.hour) != (wanted.branch, wanted.hour) {
                        return Err(invalid("re-encoded frames left recorded order"));
                    }
                    return Ok(frame);
                }
                let block = blocks
                    .next()
                    .ok_or_else(|| invalid("re-encoded frames outran the recording"))?;
                frames = self.read_block(block)?.into_iter();
            }
        };
        let slices: Vec<&[Recovered]> = segments.iter().map(Vec::as_slice).collect();
        let mut replay = Self::from_record_slices(
            self.campaign_id.clone(),
            &slices,
            Observer::Frames(&mut observe),
            Frames::Ordered(&mut read),
            MAX_BLOCK_RAW_BYTES,
            None,
        )?;
        replay.bytes.truncate(replay.trailer_start);
        finish_file(
            &mut replay.bytes,
            &self.sessions,
            &self.branches,
            &self.gaps,
            &self.controls,
            &self.recovery_notes,
            &replay.index,
        )?;
        commit::stamp_slot(&mut replay.bytes, replay.slot)?;
        Self::open(replay.bytes)
    }

    /// The index position of this validated block. Extents have increasing
    /// offsets even across skipped bytes.
    pub fn block_position(&self, index: &BlockIndex) -> Option<usize> {
        self.index
            .binary_search_by_key(&index.offset, |entry| entry.offset)
            .ok()
            .filter(|&position| self.index[position] == *index)
    }

    fn is_verified(&self, index: &BlockIndex) -> bool {
        self.block_position(index)
            .is_some_and(|position| self.verified[position].load(Ordering::Acquire))
    }

    /// Publish a synced sibling atomically. On Windows, delete-sharing readers
    /// keep the old revision while new opens see the replacement. Unsupported
    /// volumes use classic replacement, which open readers may block.
    /// A flush error after POSIX rename reports failure with the new file visible.
    pub fn write_atomic(&self, destination: &Path) -> io::Result<()> {
        self.write_atomic_with(destination, |_| Ok(()))
    }

    /// Fault checkpoints happen before replacement; checkpoint failure leaves
    /// the destination untouched. Publication itself can fail after rename if
    /// the renamed file or parent directory cannot be flushed.
    pub fn write_atomic_with(
        &self,
        destination: &Path,
        mut checkpoint: impl FnMut(PublishStage) -> io::Result<()>,
    ) -> io::Result<()> {
        let _phase = Packing::phase(Phase::Write);
        self.publish_with(
            &mut FilePublisher {
                temporary: temporary_path(destination),
                destination: destination.into(),
            },
            &mut checkpoint,
        )
    }

    pub fn publish_with(
        &self,
        publisher: &mut impl AtomicPublisher,
        mut checkpoint: impl FnMut(PublishStage) -> io::Result<()>,
    ) -> io::Result<()> {
        self.require_resident()?;
        self.require_undamaged()?;
        let bytes = self.bytes();
        let mut file = publisher.create()?;
        checkpoint(PublishStage::TempCreated)?;
        file.write_all(&bytes[..self.header_end])?;
        checkpoint(PublishStage::HeaderWritten)?;
        // Earlier trailers between the blocks are written with the block after them.
        let mut written = self.header_end;
        for (position, block) in self.index.iter().enumerate() {
            let start = to_usize(block.offset)?;
            let end = to_usize(block.offset + block.length)?;
            let midpoint = start + (end - start) / 2;
            file.write_all(&bytes[written..midpoint])?;
            checkpoint(PublishStage::BlockPartWritten(position))?;
            file.write_all(&bytes[midpoint..end])?;
            checkpoint(PublishStage::BlockWritten(position))?;
            written = end;
        }
        let footer_start = bytes.len() - FOOTER_SIZE;
        file.write_all(&bytes[written..footer_start])?;
        checkpoint(PublishStage::TrailerWritten)?;
        file.write_all(&bytes[footer_start..])?;
        checkpoint(PublishStage::FooterWritten)?;
        publisher.sync(&mut file)?;
        checkpoint(PublishStage::Flushed)?;
        checkpoint(PublishStage::BeforeReplace)?;
        publisher.replace(file)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishStage {
    TempCreated,
    HeaderWritten,
    BlockPartWritten(usize),
    BlockWritten(usize),
    TrailerWritten,
    FooterWritten,
    Flushed,
    BeforeReplace,
}

/// The same publication sequence runs against files and pure failure-injection writers.
pub trait AtomicPublisher {
    type Writer: Write;
    fn create(&mut self) -> io::Result<Self::Writer>;
    fn sync(&mut self, writer: &mut Self::Writer) -> io::Result<()>;
    fn replace(&mut self, writer: Self::Writer) -> io::Result<()>;
}

struct FilePublisher {
    temporary: PathBuf,
    destination: PathBuf,
}
impl AtomicPublisher for FilePublisher {
    type Writer = Budgeted<File>;
    fn create(&mut self) -> io::Result<Self::Writer> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.temporary)
            .map(Budgeted::new)
    }
    fn sync(&mut self, writer: &mut Self::Writer) -> io::Result<()> {
        writer.get_ref().sync_all()
    }
    fn replace(&mut self, writer: Self::Writer) -> io::Result<()> {
        drop(writer);
        replace_file(&self.temporary, &self.destination)
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(windows)]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    Ok(crate::publication::replace_file(source, destination)?)
}

#[cfg(not(windows))]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)?;
    if let Some(parent) = destination.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Every block's frames: one branch, a leading keyframe, increasing hours
/// spanning its index range.
fn check_block_frames(index: &BlockIndex, frames: Vec<Frame>) -> io::Result<Vec<Frame>> {
    let mut last = None;
    for (position, frame) in frames.iter().enumerate() {
        if frame.branch != index.branch
            || frame.keyframe != (position == 0)
            || last.is_some_and(|hour| frame.hour <= hour)
        {
            return Err(invalid(
                "block frame branch, keyframe, or hour ordering mismatch",
            ));
        }
        last = Some(frame.hour);
    }
    if frames.first().map(|frame| frame.hour) != Some(index.first_hour)
        || last != Some(index.last_hour)
    {
        return Err(invalid("block index hour range mismatch"));
    }
    Ok(frames)
}

fn expect_magic(decoder: &mut Decoder<'_>, expected: &[u8; 4]) -> io::Result<()> {
    if decoder.take(4)? == expected {
        Ok(())
    } else {
        Err(invalid("invalid replay magic"))
    }
}
fn to_usize(value: u64) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid("file extent exceeds platform address space"))
}
fn put_count(out: &mut Vec<u8>, count: usize) -> io::Result<()> {
    engine_allocation::reserve(out, 4)?;
    out.extend_from_slice(
        &u32::try_from(count)
            .map_err(|_| invalid_static("too many metadata entries"))?
            .to_le_bytes(),
    );
    Ok(())
}
fn encode_list<T>(
    out: &mut Vec<u8>,
    values: &[T],
    encode: impl Fn(&T, &mut Vec<u8>) -> io::Result<()>,
) -> io::Result<()> {
    put_count(out, values.len())?;
    for value in values {
        let mut payload = Vec::new();
        encode(value, &mut payload)?;
        put_bytes(out, &payload)?;
    }
    Ok(())
}
fn decode_list<T>(
    decoder: &mut Decoder<'_>,
    decode: impl Fn(&mut Decoder<'_>) -> io::Result<T>,
) -> io::Result<Vec<T>> {
    let count = decoder.count(4)?;
    let mut values = engine_allocation::vec(count)?;
    for _ in 0..count {
        let payload = decoder.bytes()?;
        let mut item = Decoder::new(&payload);
        item.format = decoder.format;
        values.push(decode(&mut item)?);
        item.finish()?;
    }
    Ok(values)
}

fn decode_list_recovering<T>(
    decoder: &mut Decoder<'_>,
    decode: impl Fn(&mut Decoder<'_>) -> io::Result<T>,
    name: &str,
    warnings: &mut Vec<String>,
    damaged_parts: &mut usize,
) -> io::Result<Vec<T>> {
    let count = decoder.count(4)?;
    let mut values = engine_allocation::vec(count)?;
    for _ in 0..count {
        // The outer length must be readable; errors within that boundary can
        // withhold this record without losing subsequent trailer lists/index.
        let payload = decoder.bytes()?;
        let mut item = Decoder::new(&payload);
        item.format = decoder.format;
        match decode(&mut item).and_then(|value| {
            item.finish()?;
            Ok(value)
        }) {
            Ok(value) => values.push(value),
            Err(error)
                if engine_allocation::is_allocation_error(&error)
                    || crate::format::is_layout_refusal(&error) =>
            {
                return Err(error);
            }
            Err(error) => {
                engine_allocation::reserve(warnings, 1)?;
                warnings.push(crate::codec::try_format(format_args!(
                    "Unreadable {name} record: {}",
                    crate::codec::diagnostic_text(&error.to_string())
                ))?);
                *damaged_parts += 1;
            }
        }
    }
    Ok(values)
}

/// Releases each block once copied, so the file is held twice one block at a time.
fn append_owned_blocks(
    bytes: &mut Vec<u8>,
    index: &mut Vec<BlockIndex>,
    blocks: Vec<CompressedBlock>,
) -> io::Result<()> {
    engine_allocation::reserve(bytes, blocks.iter().map(|block| block.bytes.len()).sum())?;
    for block in blocks {
        append_blocks(bytes, index, std::slice::from_ref(&block))?;
    }
    Ok(())
}

fn append_blocks(
    bytes: &mut Vec<u8>,
    index: &mut Vec<BlockIndex>,
    blocks: &[CompressedBlock],
) -> io::Result<()> {
    engine_allocation::reserve(index, blocks.len())?;
    for block in blocks {
        engine_allocation::reserve(bytes, block.bytes.len())?;
        index.push(BlockIndex {
            branch: block.branch,
            first_hour: block.first_hour,
            last_hour: block.last_hour,
            offset: bytes.len() as u64,
            length: block.bytes.len() as u64,
            raw_length: block.raw_length,
        });
        bytes.extend_from_slice(&block.bytes);
    }
    Ok(())
}

/// End a resident recording with its first trailer and footer.
#[allow(clippy::too_many_arguments)]
fn finish_file(
    bytes: &mut Vec<u8>,
    sessions: &[SessionStart],
    branches: &[BranchStart],
    gaps: &[Gap],
    controls: &[ControlRecord],
    recovery_notes: &[RecoveryNote],
    index: &[BlockIndex],
) -> io::Result<()> {
    commit::encode_trailer(
        bytes,
        0,
        sessions,
        branches,
        gaps,
        controls,
        recovery_notes,
        None,
        index,
    )
    .map(|_| ())
}

/// A current recording's header and trailer sessions in `format`'s layout; blocks are the caller's.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn relabelled(bytes: &[u8], format: u16) -> io::Result<Vec<u8>> {
    let replay = Replay::open(bytes.to_vec())?;
    if replay.previous.is_some() {
        return Err(invalid("only a recording saved once can be relabelled"));
    }
    let trailer = &bytes[replay.trailer_start..replay.bytes().len() - FOOTER_SIZE];
    let mut input = Decoder::new(trailer);
    expect_magic(&mut input, TRAILER_MAGIC)?;
    let count = input.count(4)?;
    for _ in 0..count {
        input.bytes()?;
    }
    let mut rest = trailer[trailer.len() - input.remaining()..].to_vec();
    let (mut out, shift) = match replay.slot {
        // Earlier headers have no commit slot, so every block starts that much sooner.
        Some(slot) if format < crate::rows::compatibility::CHECKPOINT_FORMAT => {
            let mut lists = Decoder::new(&rest);
            for _ in 0..4 {
                let count = lists.count(4)?;
                for _ in 0..count {
                    lists.bytes()?;
                }
            }
            let link = rest.len() - lists.remaining();
            rest.drain(link..link + 16);
            let count = Decoder::new(&rest[link..]).count(40)?;
            for entry in 0..count {
                let at = link + 4 + entry * 40 + 16;
                let offset = u64::from_le_bytes(rest[at..at + 8].try_into().unwrap());
                rest[at..at + 8]
                    .copy_from_slice(&(offset - commit::SLOT_SIZE as u64).to_le_bytes());
            }
            let mut out = bytes[..slot].to_vec();
            out.extend_from_slice(&bytes[slot + commit::SLOT_SIZE..replay.trailer_start]);
            (out, None)
        }
        slot => (bytes[..replay.trailer_start].to_vec(), slot),
    };
    out[4..6].copy_from_slice(&format.to_le_bytes());
    let trailer_start = out.len();
    out.extend_from_slice(TRAILER_MAGIC);
    encode_list(&mut out, &replay.sessions, |session, out| {
        let start = out.len();
        session.encode(out)?;
        if format < crate::rows::compatibility::CAPTURE_MODE_FORMAT {
            // Earlier sessions have no capture mode byte.
            out.remove(start + 29);
        }
        Ok(())
    })?;
    out.extend_from_slice(&rest);
    let trailer_length = out.len() - trailer_start;
    let crc = crc32fast::hash(&out[trailer_start..]);
    out.extend_from_slice(FOOTER_MAGIC);
    out.extend_from_slice(&(trailer_start as u64).to_le_bytes());
    out.extend_from_slice(&(trailer_length as u64).to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    commit::stamp_slot(&mut out, shift)?;
    Ok(out)
}

fn validate_campaign(frame: &DecodedFrame, campaign_id: &[u8]) -> io::Result<()> {
    validate_campaign_view(&frame.tables[TableId::Campaign.index()], campaign_id)
}

/// [`validate_campaign`] of an hour's Campaign view.
fn validate_campaign_view(view: &TableView, campaign_id: &[u8]) -> io::Result<()> {
    if let TableView::Complete(rows) = view
        && (rows.len() != 1
            || !matches!(rows, TableRows::Campaign(values) if values[0].id == campaign_id))
    {
        return Err(invalid(
            "frame campaign identity differs from replay header",
        ));
    }
    Ok(())
}

fn segment_numbers<R>(segments: &[(u32, Recovery<R>)]) -> Vec<u32> {
    let mut numbers: Vec<_> = segments.iter().map(|(number, _)| *number).collect();
    numbers.sort_unstable();
    numbers
}

fn ordered_records<R>(segments: &[(u32, Recovery<R>)]) -> io::Result<Vec<&[R]>> {
    let mut sorted: Vec<_> = segments.iter().collect();
    sorted.sort_unstable_by_key(|(number, _)| *number);
    if sorted.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("duplicate spool segment number"));
    }
    Ok(sorted
        .into_iter()
        .map(|(_, recovery)| recovery.records.as_slice())
        .collect())
}

/// Where the parallel writer stops, the sequential one repeats it, reporting only unreported hours.
#[allow(clippy::too_many_arguments)]
fn collect<R: SpoolRecord>(
    segments: &[&[R]],
    identity: &Identity,
    campaign_id: &[u8],
    existing: Option<&Replay>,
    interval: u32,
    mut observe: Observer<'_>,
    mut frames: Frames<'_>,
    block_limit: usize,
    ahead: Option<Ahead>,
) -> io::Result<Pending> {
    let _phase = Packing::phase(Phase::Pack);
    if let Some(mut ahead) = ahead {
        ahead.renumber();
        let kept: Vec<_> = segments
            .iter()
            .enumerate()
            .map(|(at, records)| {
                (
                    at as u32,
                    records
                        .iter()
                        .map(|record| match record.view() {
                            RecordRef::Metadata(frame) => Ok(Recovered::Frame(frame.clone())),
                            RecordRef::SessionStart(value) => {
                                Ok(Recovered::SessionStart(value.clone()))
                            }
                            RecordRef::BranchStart(value) => {
                                Ok(Recovered::BranchStart(value.clone()))
                            }
                            RecordRef::Gap(value) => Ok(Recovered::Gap(value.clone())),
                            RecordRef::Control(value) => Ok(Recovered::Control(value.clone())),
                            RecordRef::Frame(_) => Err(invalid_static(
                                "retained writer requires recovered metadata",
                            )),
                        })
                        .collect::<io::Result<Vec<_>>>(),
                )
            })
            .map(|(number, records)| records.map(|records| (number, records)))
            .collect::<io::Result<Vec<_>>>()?;
        let unwritten = kept
            .iter()
            .flat_map(|(_, records)| records)
            .filter(|record| matches!(record, Recovered::Frame(_)))
            .count()
            .saturating_sub(ahead.written());
        let records: Vec<_> = kept
            .iter()
            .map(|(number, records)| (*number, records.as_slice()))
            .collect();
        let shared = match &frames {
            Frames::Shared(read, _) => Some(*read),
            _ => None,
        };
        let read = |number, metadata: &FrameMetadata| match shared {
            Some(read) => read(number as usize, metadata),
            None => metadata.frame(),
        };
        let pending = ahead.finish(campaign_id, &records, read, |hour| match &mut observe {
            Observer::Hours(observe) => observe(hour),
            Observer::Nothing => Ok(()),
            Observer::Frames(_) => Err(invalid_static("retained writer reports observed hours")),
        })?;
        if let Frames::Shared(_, progress) = &mut frames {
            for _ in 0..unwritten {
                progress()?;
            }
        }
        return Ok(pending);
    }
    let ahead: Option<Ahead> = None;
    let threads = pack::workers();
    let (mut hours, mut whole) = match observe {
        Observer::Nothing => (None, None),
        Observer::Hours(hours) => (Some(hours), None),
        Observer::Frames(whole) => (None, Some(whole)),
    };
    let (mut reported, mut progressed) = (ahead.as_ref().map_or(0, Ahead::hours), 0);
    let mut ahead = ahead;
    // Legacy lanes wait on one another; budgeted saves lend only finite work to their pool.
    if !Packing::active() && threads > 1 && whole.is_none() {
        let mut quiet = || Ok(());
        let parallel = match &mut frames {
            Frames::Kept => Some((None, &mut quiet as &mut dyn FnMut() -> io::Result<()>)),
            Frames::Shared(read, progress) => Some((Some(*read), &mut **progress)),
            Frames::Ordered(_) => None,
        };
        if let Some((read, progress)) = parallel {
            match units::write(
                segments,
                identity,
                campaign_id,
                existing,
                interval,
                hours.as_deref_mut(),
                progress,
                read,
                block_limit,
                threads,
                ahead.take(),
            )? {
                units::Written::Pending(pending) => return Ok(pending),
                units::Written::Stopped { hours, progress } => {
                    (reported, progressed) = (hours, progress)
                }
            }
        }
    }
    let mut previous: Option<Observed> = None;
    let mut observe = |frame: &Frame, decoded: &DecodedFrame, sizes: [usize; TABLE_COUNT]| {
        if let Some(whole) = whole.as_deref_mut() {
            return whole(frame, decoded, sizes);
        }
        let Some(hours) = hours.as_deref_mut() else {
            return Ok(());
        };
        if reported > 0 {
            reported -= 1;
            previous = None;
            return Ok(());
        }
        let hour = Observed::new(frame, decoded, sizes, previous.as_ref())?;
        hours(&hour)?;
        previous = Some(hour);
        Ok(())
    };
    let (mut pending, blocks) = pack::blocks(
        threads,
        |block: grouped::BlockWriter, cores: &pack::Cores| block.compress(cores),
        |packer| {
            write_blocks(
                packer,
                segments,
                identity,
                campaign_id,
                existing,
                interval,
                &mut observe,
                frames,
                progressed,
                block_limit,
            )
        },
    )?;
    pending.blocks = blocks;
    Ok(pending)
}

/// [`collect`]'s records in order, each finished block handed to `packer`.
#[allow(clippy::too_many_arguments)]
fn write_blocks<R: SpoolRecord>(
    packer: &mut pack::Packer<'_, grouped::BlockWriter, CompressedBlock>,
    segments: &[&[R]],
    identity: &Identity,
    campaign_id: &[u8],
    existing: Option<&Replay>,
    interval: u32,
    observe: &mut FrameObserver<'_>,
    mut frames: Frames<'_>,
    mut progressed: usize,
    block_limit: usize,
) -> io::Result<Pending> {
    identity.validate()?;
    let mut sessions: BTreeMap<_, _> = existing
        .into_iter()
        .flat_map(|replay| &replay.sessions)
        .map(|session| (session.id, session.clone()))
        .collect();
    let mut branches: BTreeMap<_, _> = existing
        .into_iter()
        .flat_map(|replay| &replay.branches)
        .map(|branch| (branch.id, branch.clone()))
        .collect();
    let mut last_hours: BTreeMap<_, _> = existing
        .into_iter()
        .flat_map(|replay| &replay.index)
        .map(|block| (block.branch, block.last_hour))
        .collect();
    let mut pending = Pending {
        sessions: Vec::new(),
        branches: Vec::new(),
        gaps: Vec::new(),
        controls: Vec::new(),
        blocks: Vec::new(),
    };
    let mut block: Option<Writing> = None;
    // Appended blocks anchor to the file's earlier keyframes too.
    let mut anchors = match existing {
        Some(replay) => grouped::Anchors::seed(replay)?,
        None => grouped::Anchors::default(),
    };
    for (position, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            continue;
        }
        if !matches!(
            segment[0].view(),
            RecordRef::SessionStart(_) | RecordRef::BranchStart(_)
        ) {
            return Err(invalid(
                "segment does not begin with session or branch context",
            ));
        }
        let mut current_session = None;
        let mut current_branch = None;
        let mut needs_keyframe = true;
        let mut segment_state = SegmentState::default();
        for record in *segment {
            pack::refresh();
            let record = record.view();
            segment_state.accept_view(&record)?;
            match record {
                RecordRef::SessionStart(session) => {
                    if !session.identity.same_content(identity) {
                        return Err(invalid("session content differs from replay content"));
                    }
                    if sessions
                        .values()
                        .any(|old| old.capture_mode != session.capture_mode)
                    {
                        return Err(invalid(
                            "session capture mode differs from the replay's; record into a new file",
                        ));
                    }
                    if let Some(previous) = sessions.get(&session.id) {
                        if previous != session {
                            return Err(invalid("conflicting session context"));
                        }
                    } else {
                        session.validate()?;
                        sessions.insert(session.id, session.clone());
                        pending.sessions.push(session.clone());
                    }
                    if let Some(previous) = block.take() {
                        packer.submit(previous.finish())?;
                    }
                    current_session = Some(session.id);
                    current_branch = None;
                    needs_keyframe = true;
                }
                RecordRef::BranchStart(branch) => {
                    branch.validate()?;
                    if !sessions.contains_key(&branch.session) {
                        return Err(invalid("branch refers to an unknown session"));
                    }
                    if current_session.is_some_and(|session| session != branch.session) {
                        return Err(invalid("branch changed session without session context"));
                    }
                    if let Some(previous) = branches.get(&branch.id) {
                        if previous != branch {
                            return Err(invalid("conflicting branch context"));
                        }
                    } else {
                        branches.insert(branch.id, branch.clone());
                        pending.branches.push(branch.clone());
                    }
                    if let Some(previous) = block.take() {
                        packer.submit(previous.finish())?;
                    }
                    current_session = Some(branch.session);
                    current_branch = Some(branch.id);
                    needs_keyframe = true;
                }
                RecordRef::Frame(_) | RecordRef::Metadata(_) => {
                    let restored;
                    let frame = match record {
                        RecordRef::Frame(frame) => frame,
                        RecordRef::Metadata(metadata) => {
                            restored = match &mut frames {
                                Frames::Kept => metadata.frame()?,
                                Frames::Shared(read, progress) => {
                                    // The parallel writer reported these before it stopped.
                                    match progressed.checked_sub(1) {
                                        Some(left) => progressed = left,
                                        None => progress()?,
                                    }
                                    read(position, metadata)?
                                }
                                Frames::Ordered(read) => read(position, metadata)?,
                            };
                            if (restored.branch, restored.hour, restored.keyframe)
                                != (metadata.branch, metadata.hour, metadata.keyframe)
                            {
                                return Err(invalid(
                                    "restored spool frame differs from recovered metadata",
                                ));
                            }
                            &restored
                        }
                        _ => unreachable!("a frame record"),
                    };
                    if current_branch != Some(frame.branch) {
                        return Err(invalid("frame has no matching branch context"));
                    }
                    let session_mode = current_session
                        .and_then(|id| sessions.get(&id))
                        .map(|session| session.capture_mode)
                        .ok_or_else(|| invalid("frame has no session context"))?;
                    if session_mode != frame.capture_mode()? {
                        return Err(invalid("frame capture mode differs from its session's"));
                    }
                    if needs_keyframe && !frame.keyframe {
                        return Err(invalid("segment or branch starts without keyframe"));
                    }
                    if last_hours
                        .get(&frame.branch)
                        .is_some_and(|hour| frame.hour <= *hour)
                    {
                        return Err(invalid("branch frame hour did not increase"));
                    }
                    if frame.hour < branches[&frame.branch].first_hour {
                        return Err(invalid("frame precedes its branch start"));
                    }
                    if frame.keyframe {
                        if let Some(previous) = block.take() {
                            packer.submit(previous.finish())?;
                        }
                        block = Some(Writing::new(frame, session_mode)?);
                    }
                    let current = block
                        .as_mut()
                        .ok_or_else(|| invalid("frame has no block keyframe"))?;
                    if i64::from(frame.hour) - i64::from(current.block.first_hour)
                        >= i64::from(interval)
                    {
                        return Err(invalid("block exceeds its keyframe hour interval"));
                    }
                    let cores = packer.cores();
                    let written = current.frame(
                        frame,
                        campaign_id,
                        block_limit,
                        &mut anchors,
                        cores,
                        observe,
                        Some(&mut |full: grouped::BlockWriter| packer.submit(full.written())),
                    )?;
                    assert!(written, "a writer holding the anchors splits blocks");
                    last_hours.insert(frame.branch, frame.hour);
                    needs_keyframe = false;
                }
                RecordRef::Gap(gap) => {
                    gap.validate()?;
                    if current_branch != Some(gap.branch) {
                        return Err(invalid("gap has no matching branch context"));
                    }
                    if !pending.gaps.contains(gap)
                        && !existing.is_some_and(|replay| replay.gaps.contains(gap))
                    {
                        pending.gaps.push(gap.clone());
                    }
                }
                RecordRef::Control(control) => {
                    if current_session != Some(control.session()) {
                        return Err(invalid("control has no matching session context"));
                    }
                    // A writer retries an event loss whose segment failed after
                    // writing it; the same loss is kept once.
                    if !(matches!(control, ControlRecord::EventLoss { .. })
                        && pending.controls.contains(control))
                    {
                        pending.controls.push(control.clone());
                    }
                }
            }
        }
        if let Some(previous) = block.take() {
            packer.submit(previous.finish())?;
        }
    }
    // A complete CRC-valid record can survive a failed flush. Preserve its frame
    // and the reported write-loss diagnostic; seeking gives the recovered frame priority.
    Ok(pending)
}

/// Decoder and alias encoder keep nothing across a keyframe, so a block writes the same on any thread.
struct Writing {
    block: grouped::BlockWriter,
    decoder: FrameDecoder,
    alias: aliases::Encoder,
}

impl Writing {
    fn new(keyframe: &Frame, mode: CaptureMode) -> io::Result<Self> {
        if keyframe.capture_mode()? != mode {
            return Err(invalid("frame capture mode differs from its session's"));
        }
        Ok(Self {
            block: grouped::BlockWriter::new(keyframe)?,
            decoder: FrameDecoder::new(),
            alias: aliases::Encoder::default(),
        })
    }

    /// The written block, its decoded hour released before it is compressed.
    fn finish(self) -> grouped::BlockWriter {
        self.block.written()
    }

    /// False, writing nothing, where the block would split without a `split` (which needs the anchors).
    #[allow(clippy::too_many_arguments)]
    fn frame(
        &mut self,
        frame: &Frame,
        campaign_id: &[u8],
        block_limit: usize,
        anchors: &mut grouped::Anchors,
        cores: &pack::Cores,
        observe: &mut FrameObserver<'_>,
        split: Option<&mut dyn FnMut(grouped::BlockWriter) -> io::Result<()>>,
    ) -> io::Result<bool> {
        pack::refresh();
        let decoded = self.decoder.advance(frame)?;
        validate_campaign(decoded, campaign_id)?;
        let mut payload = Vec::new();
        let mut sizes = frame.encode_with_sizes(&mut payload)?;
        let mut aliased = self.alias.prepare(frame, decoded, &payload, &sizes)?;
        let promoted;
        let emitted = if !self.block.fits(payload.len(), &sizes, block_limit)
            || !self
                .block
                .fits(aliased.stored_len, &aliased.stored_sizes, block_limit)
        {
            if self.block.frames == 0 {
                return Err(invalid("single replay frame exceeds raw block size limit"));
            }
            let Some(split) = split else {
                return Ok(false);
            };
            // Block boundaries need an independent baseline. Snapshot the
            // already decoded hour, retaining sample origins and native
            // values; keep decoding later spool deltas normally.
            promoted = block_keyframe(decoded)?;
            payload.clear();
            sizes = promoted.encode_with_sizes(&mut payload)?;
            let next = grouped::BlockWriter::new(&promoted)?;
            aliased = self.alias.prepare(&promoted, decoded, &payload, &sizes)?;
            if !next.fits(payload.len(), &sizes, block_limit)
                || !next.fits(aliased.stored_len, &aliased.stored_sizes, block_limit)
            {
                return Err(invalid(
                    "single replay keyframe exceeds raw block size limit",
                ));
            }
            split(std::mem::replace(&mut self.block, next))?;
            &promoted
        } else {
            frame
        };
        self.block.push_marked(
            emitted,
            &aliased.views,
            &aliased.payload,
            &aliased.sizes,
            anchors,
            block_limit,
            cores,
            &aliased.private,
        )?;
        self.block
            .charge_source(payload.len(), &sizes, &aliased.stored_sizes);
        observe(emitted, decoded, aliased.sizes)?;
        self.alias.commit(aliased);
        Ok(true)
    }
}

fn block_keyframe(decoded: &DecodedFrame) -> io::Result<Frame> {
    let mut tables = engine_allocation::vec(TABLE_COUNT)?;
    for table in TableId::ALL {
        tables.push(match &*decoded.tables[table.index()] {
            TableView::Complete(rows) => crate::frame::TableUpdate::Snapshot(rows.try_clone()?),
            TableView::Unavailable { reason } => crate::frame::TableUpdate::Unavailable {
                reason: engine_allocation::string(reason)?,
            },
            TableView::NotRecorded { mode } => {
                crate::frame::TableUpdate::NotRecorded { mode: *mode }
            }
            TableView::Truncated { rows, missing } => {
                return Err(invalid(format!(
                    "cannot split replay block at hour {}: {table:?} has {} retained rows and {missing} missing rows; keyframes cannot represent truncated tables",
                    decoded.hour, rows.len()
                )));
            }
        });
    }
    Ok(Frame {
        branch: decoded.branch,
        hour: decoded.hour,
        keyframe: true,
        metrics: decoded.metrics.clone(),
        tables: tables.try_into().unwrap(),
    })
}

fn file_capture_mode(sessions: &[SessionStart]) -> io::Result<CaptureMode> {
    let mode = sessions
        .first()
        .ok_or_else(|| invalid("replay has no sessions"))?
        .capture_mode;
    if sessions.iter().any(|session| session.capture_mode != mode) {
        return Err(invalid("replay sessions name different capture modes"));
    }
    Ok(mode)
}

#[allow(clippy::too_many_arguments)]
fn validate_metadata(
    identity: &Identity,
    sessions: &[SessionStart],
    branches: &[BranchStart],
    gaps: &mut Vec<Gap>,
    controls: &[ControlRecord],
    index: &mut Vec<BlockIndex>,
    ordinals: &mut Vec<usize>,
    header_end: usize,
    trailer_start: usize,
    regions: &[(u64, u64)],
    interval: u32,
    warnings: &mut Vec<String>,
    damaged_parts: &mut usize,
    damaged_range: &mut Option<(Hour, Hour)>,
) -> io::Result<()> {
    if sessions.is_empty() {
        return Err(invalid("replay has no sessions"));
    }
    let mut session_ids = HashSet::new();
    engine_allocation::reserve_set(&mut session_ids, sessions.len())?;
    for session in sessions {
        if !session_ids.insert(session.id) || !session.identity.same_content(identity) {
            return Err(invalid("duplicate session or conflicting content"));
        }
    }
    let mut branch_ids = HashMap::new();
    engine_allocation::reserve_map(&mut branch_ids, branches.len())?;
    let mut sessions_with_start_branch = HashSet::new();
    engine_allocation::reserve_set(&mut sessions_with_start_branch, sessions.len())?;
    for branch in branches {
        branch.validate()?;
        if !session_ids.contains(&branch.session) || branch_ids.contains_key(&branch.id) {
            return Err(invalid("unknown session or duplicate branch"));
        }
        if branch.reason == BranchReason::SessionStart
            && !sessions_with_start_branch.insert(branch.session)
        {
            return Err(invalid("session has multiple process-start branches"));
        }
        // Parentage is the collector's evidence at branch creation. Recovery may
        // have lost the supporting hour or its entire segment; do not rewrite it.
        branch_ids.insert(branch.id, branch.session);
    }
    if gaps.iter().any(|gap| !branch_ids.contains_key(&gap.branch)) {
        engine_allocation::reserve(warnings, 1)?;
        warnings.push("Unreadable gap metadata: gap refers to an unknown branch".into());
        *damaged_parts += 1;
        gaps.retain(|gap| branch_ids.contains_key(&gap.branch));
    }
    for control in controls {
        if !session_ids.contains(&control.session()) {
            return Err(invalid("replay control refers to an unknown session"));
        }
        if let ControlRecord::EventLoss {
            session, branch, ..
        } = control
        {
            if branch_ids.get(branch) != Some(session) {
                return Err(invalid("event loss refers to another session's branch"));
            }
            if controls.iter().filter(|other| *other == control).count() > 1 {
                return Err(invalid("event loss is recorded twice"));
            }
        }
    }
    validate_partial_controls(controls)?;
    let mut expected_offset = header_end as u64;
    let mut last_hours = HashMap::new();
    engine_allocation::reserve_map(&mut last_hours, branches.len())?;
    let mut closed_sessions = HashSet::new();
    engine_allocation::reserve_set(&mut closed_sessions, sessions.len())?;
    let mut previous_branch = None;
    let mut previous_session = None;
    let mut kept = 0;
    // Earlier trailers lie between the blocks the saves before them wrote; one
    // after bytes no one names is passed with them.
    let mut regions = regions.iter().peekable();
    for position in 0..index.len() {
        let block = &index[position];
        while let Some(&&(start, end)) = regions.peek() {
            if start == expected_offset {
                expected_offset = end;
            } else if end > block.offset {
                break;
            }
            regions.next();
        }
        let known_branch = branch_ids.contains_key(&block.branch);
        let valid_hours = block.first_hour <= block.last_hour
            && i64::from(block.last_hour) - i64::from(block.first_hour) < i64::from(interval)
            && last_hours
                .get(&block.branch)
                .is_none_or(|hour| block.first_hour > *hour);
        let end = block.offset.checked_add(block.length);
        let reason = if !known_branch {
            Some("block refers to an unknown branch")
        } else if !valid_hours {
            Some("invalid or overlapping block hour ranges")
        } else if block.offset < expected_offset
            || block.length == 0
            || end.is_none_or(|end| end > trailer_start as u64)
            || regions
                .peek()
                .is_some_and(|(start, _)| end.is_none_or(|end| end > *start))
            || block.raw_length < 4
            || block.raw_length > MAX_BLOCK_RAW_BYTES as u64
        {
            Some("invalid block extent or raw length")
        } else {
            None
        };
        if let Some(reason) = reason {
            let reason = crate::codec::try_format(format_args!(
                "Skipped block {} at byte {}: {reason}",
                block.branch, block.offset
            ))?;
            engine_allocation::reserve(warnings, 1)?;
            warnings.push(reason.clone());
            *damaged_parts += 1;
            if known_branch && valid_hours {
                engine_allocation::reserve(gaps, 1)?;
                gaps.push(Gap {
                    branch: block.branch,
                    first_hour: block.first_hour,
                    last_hour: block.last_hour,
                    reason,
                });
                let (first, last) = damaged_range.unwrap_or((block.first_hour, block.last_hour));
                *damaged_range = Some((first.min(block.first_hour), last.max(block.last_hour)));
            }
            continue;
        }
        if block.offset != expected_offset {
            engine_allocation::reserve(warnings, 1)?;
            warnings.push("unindexed bytes between replay blocks".into());
            *damaged_parts += 1;
        }
        if previous_branch != Some(block.branch) && last_hours.contains_key(&block.branch) {
            engine_allocation::reserve(warnings, 1)?;
            warnings.push("branch blocks are not contiguous".into());
        }
        let session = branch_ids[&block.branch];
        if previous_session != Some(session) {
            if let Some(previous) = previous_session {
                closed_sessions.insert(previous);
            }
            if closed_sessions.contains(&session) {
                engine_allocation::reserve(warnings, 1)?;
                warnings.push("session blocks are not contiguous".into());
            }
        }
        previous_branch = Some(block.branch);
        previous_session = Some(session);
        expected_offset = end.unwrap();
        last_hours.insert(block.branch, block.last_hour);
        if kept != position {
            index[kept] = block.clone();
        }
        engine_allocation::reserve(ordinals, 1)?;
        ordinals.push(position);
        kept += 1;
    }
    index.truncate(kept);
    while let Some((_, end)) = regions.next_if(|(start, _)| *start == expected_offset) {
        expected_offset = *end;
    }
    if expected_offset != trailer_start as u64 || regions.next().is_some() {
        engine_allocation::reserve(warnings, 1)?;
        warnings.push("unindexed bytes before replay trailer".into());
        *damaged_parts += 1;
    }
    Ok(())
}

fn require_current_spools<R>(segments: &[(u32, Recovery<R>)]) -> io::Result<()> {
    for (segment, recovery) in segments {
        if recovery.format.is_none() && !recovery.records.is_empty() {
            return Err(invalid(format!(
                "Spool segment {segment} has no proven recording format"
            )));
        }
        if let Some(format) = recovery.format
            && format != FormatVersion::CURRENT
        {
            return Err(invalid(format!(
                "Spool finalization requires format {FORMAT_VERSION}; segment {segment} is format {}; historical rows are never rewritten",
                format.version()
            )));
        }
    }
    Ok(())
}

fn validate_partial_controls(controls: &[ControlRecord]) -> io::Result<()> {
    for (position, control) in controls.iter().enumerate() {
        if let ControlRecord::Partial { session, reason, .. } = control
            && (reason.trim().is_empty() || controls[..position].iter().any(|previous| matches!(previous, ControlRecord::Partial { session: id, .. } if id == session)))
        {
            return Err(invalid("duplicate or invalid partial capture"));
        }
    }
    Ok(())
}

/// How [`Replay::read_block_parallel`] spreads a block's payload decoding.
pub struct Parallel<'a> {
    /// Worker threads at most; one decodes on the calling thread.
    pub workers: usize,
    /// Runs first on each worker, for the caller's thread setup such as a
    /// lower scheduling priority, which a spawned thread does not inherit.
    pub on_worker: &'a (dyn Fn() + Sync),
}

/// Frames a worker takes at least, so a small block is not split at all.
const PARALLEL_FRAMES: usize = 32;

/// The order a block's frames keep: its branch, a keyframe first and only first,
/// and rising hours from the index's first hour to its last.
struct BlockOrder<'a> {
    index: &'a BlockIndex,
    first: Option<Hour>,
    last: Option<Hour>,
}

/// Compressed input is memory-backed: decoder failures are recording damage,
/// except resource exhaustion and caller cancellation.
fn zstd_error(error: io::Error) -> io::Error {
    if engine_allocation::is_allocation_error(&error)
        || error.to_string().contains("Allocation error")
    {
        engine_allocation::oom()
    } else if error.kind() == io::ErrorKind::Interrupted {
        error
    } else {
        invalid(error.to_string())
    }
}
fn recording_damage(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
    )
}

impl<'a> BlockOrder<'a> {
    fn new(index: &'a BlockIndex) -> Self {
        Self {
            index,
            first: None,
            last: None,
        }
    }

    fn next(&mut self, frame: &(impl crate::frame::FrameSource + ?Sized)) -> io::Result<()> {
        if frame.branch() != self.index.branch
            || frame.hour() < self.index.first_hour
            || frame.hour() > self.index.last_hour
            || frame.keyframe() != self.last.is_none()
            || self.last.is_some_and(|hour| frame.hour() <= hour)
        {
            return Err(invalid(
                "block frame branch, keyframe, or hour ordering mismatch",
            ));
        }
        self.first.get_or_insert(frame.hour());
        self.last = Some(frame.hour());
        Ok(())
    }

    fn finish(self) -> io::Result<()> {
        if self.first != Some(self.index.first_hour) || self.last != Some(self.index.last_hour) {
            return Err(invalid("block index hour range mismatch"));
        }
        Ok(())
    }
}

/// Take work in queue order but return results in registry order.
fn parallel_tables<T: Sync, R: Send>(
    items: &[T],
    order: &[usize],
    parallel: &Parallel<'_>,
    work: impl Fn(&T) -> R + Sync,
) -> io::Result<Vec<R>> {
    assert_eq!(order.len(), items.len(), "an order names every item once");
    let workers = parallel
        .workers
        .min(items.len().div_ceil(PARALLEL_FRAMES))
        .max(1);
    if workers <= 1 {
        return engine_allocation::collect_vec(items.iter().map(|item| {
            pack::readback_table();
            work(item)
        }));
    }
    // One slot per item, filled by whichever thread takes it.
    let mut slots =
        engine_allocation::collect_vec(items.iter().map(|_| std::sync::Mutex::new(None)))?;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let take = || {
        while let Some(&at) = order.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)) {
            pack::readback_table();
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&items[at])));
            *slots[at]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
        }
    };
    let take = &take;
    let on_worker = parallel.on_worker;
    // Measurement only: the workers' CPU is the caller's family's.
    let cpu_family = crate::cpu::family();
    let cpu_block = crate::cpu::current_block();
    let worker_panic = if Packing::active() {
        Packing::scope(|scope| {
            for _ in 1..workers {
                scope.spawn(move |_| {
                    pack::refresh();
                    on_worker();
                    let _cpu = crate::cpu::scope("decode", cpu_family);
                    let _cpu_block = crate::cpu::block(cpu_block);
                    take();
                });
            }
            pack::refresh();
            on_worker();
            take();
        });
        None
    } else {
        let progress = crate::progress::Progress::current();
        std::thread::scope(|scope| {
            let mut handles = engine_allocation::vec(workers)?;
            for _ in 0..workers {
                let priority = job_priority::JobPriority::current();
                let progress = progress.clone();
                let spawned = if refused_worker() {
                    None
                } else {
                    std::thread::Builder::new()
                        .name("replay parallel decode".into())
                        .spawn_scoped(scope, move || {
                            let _progress = progress.map(crate::progress::Progress::install);
                            priority.enter(job_priority::Level::Normal);
                            on_worker();
                            let _cpu = crate::cpu::scope("decode", cpu_family);
                            let _cpu_block = crate::cpu::block(cpu_block);
                            take();
                        })
                        .ok()
                };
                handles.push(spawned);
            }
            // A worker the OS refused leaves its share of the queue to this thread.
            if handles.iter().any(Option::is_none) {
                take();
            }
            let mut worker_panic = None;
            for handle in handles.into_iter().flatten() {
                if let Err(panic) = handle.join()
                    && worker_panic.is_none()
                {
                    worker_panic = Some(panic);
                }
            }
            Ok::<_, io::Error>(worker_panic)
        })?
    };
    // Item panics rank first in registry order; setup panics follow spawn order.
    for slot in &mut slots {
        let outcome = slot
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match outcome.take() {
            Some(Err(panic)) => std::panic::resume_unwind(panic),
            result => *outcome = result,
        }
    }
    if let Some(panic) = worker_panic {
        std::panic::resume_unwind(panic);
    }
    engine_allocation::collect_vec(slots.into_iter().map(|slot| {
        slot.into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .expect("an order names every item once")
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }))
}

fn refused_worker() -> bool {
    #[cfg(test)]
    {
        let refused = REFUSED_WORKERS.get();
        if refused != 0 {
            REFUSED_WORKERS.set(refused - 1);
            return true;
        }
    }
    false
}

#[cfg(test)]
thread_local! {
    /// Decode workers this thread's next scheduled reads take as refused.
    static REFUSED_WORKERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

mod grouped;
pub(crate) mod pack;
pub use pack::{Budgeted, Packing, Phase};
mod units;
pub use units::{Ahead, Outstanding};
pub mod patch;
pub use grouped::{
    ColumnSize, ColumnSizes, LATER_CHUNK_HOURS, STORED_LEVEL, Section, SectionLevels, SectionSize,
};
#[cfg(test)]
pub(crate) use pack::WORKERS as PACK_WORKERS;

#[cfg(test)]
mod tests;
