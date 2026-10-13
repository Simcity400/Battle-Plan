//! Independently compressed blocks, an indexed trailer, and atomic publication.

use std::{
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
    spool::{ControlRecord, Record, Recovery, SegmentState},
};

const HEADER_MAGIC: &[u8; 4] = b"H4RP";
const TRAILER_MAGIC: &[u8; 4] = b"H4TR";
const FOOTER_MAGIC: &[u8; 4] = b"H4FT";
const FOOTER_SIZE: usize = 24;
// A decoder allocation bound, not a campaign-size or frame-count limit.
// V1E branch 6 held 720 valid frames (~5.95 GB raw) between keyframes;
// writers must close smaller independently decodable blocks before this bound.
// A grouped block keeps each table's plain updates, its plain frame headers
// and its stored sections within it; an interleaved block its plain frames.
pub const MAX_BLOCK_RAW_BYTES: usize = 1 << 30;

type FrameObserver<'a> =
    dyn FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()> + 'a;
type FrameReader<'a> = dyn FnMut(usize, &Frame) -> io::Result<Frame> + 'a;
type TrailerSummary = (Option<ControlRecord>, Option<(Hour, Hour)>);

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
    /// The header version, validated before decoding any frames.
    pub format: FormatVersion,
    pub campaign_id: Vec<u8>,
    pub identity: Identity,
    pub keyframe_interval: u32,
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
    trailer_start: usize,
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

/// The interleaved block layout older recordings use: one zstd frame of a
/// frame count and length-prefixed plain frames. Writers emit grouped blocks;
/// tests build interleaved ones to check the reading path.
#[cfg(test)]
struct RawBlock {
    branch: BranchId,
    first_hour: Hour,
    last_hour: Hour,
    frames: Vec<u8>,
    count: u32,
}

#[cfg(test)]
impl RawBlock {
    fn new(frame: &Frame) -> Self {
        Self {
            branch: frame.branch,
            first_hour: frame.hour,
            last_hour: frame.hour,
            frames: Vec::new(),
            count: 0,
        }
    }
    fn fits(&self, payload_bytes: usize, limit: usize) -> bool {
        self.frames
            .len()
            .checked_add(payload_bytes)
            .and_then(|size| size.checked_add(8))
            .is_some_and(|size| size <= limit)
    }
    fn push_payload(&mut self, frame: &Frame, payload: &[u8], limit: usize) -> io::Result<()> {
        if !self.fits(payload.len(), limit) {
            return Err(invalid("single replay frame exceeds raw block size limit"));
        }
        put_bytes(&mut self.frames, payload)?;
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| invalid("too many frames"))?;
        self.last_hour = frame.hour;
        Ok(())
    }
    fn push(&mut self, frame: &Frame) -> io::Result<[usize; TABLE_COUNT]> {
        let mut payload = Vec::new();
        let sizes = frame.encode_with_sizes(&mut payload)?;
        self.push_payload(frame, &payload, MAX_BLOCK_RAW_BYTES)?;
        Ok(sizes)
    }
    fn compress(self) -> io::Result<CompressedBlock> {
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)?;
        encoder.include_checksum(true)?;
        encoder.write_all(&self.count.to_le_bytes())?;
        encoder.write_all(&self.frames)?;
        Ok(CompressedBlock {
            branch: self.branch,
            first_hour: self.first_hour,
            last_hour: self.last_hour,
            raw_length: (self.frames.len() + 4) as u64,
            bytes: encoder.finish()?,
        })
    }
}

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

    /// Recording-list metadata reads the checksummed trailer without inflating blocks.
    /// Opening the replay still performs the full header and metadata validation.
    pub fn read_partial(path: &Path) -> io::Result<Option<ControlRecord>> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        Self::read_partial_from(&mut file, length)
    }

    /// Read terminal metadata from a seekable recording, using its actual byte length.
    pub fn read_partial_from(
        file: &mut (impl Read + Seek),
        length: u64,
    ) -> io::Result<Option<ControlRecord>> {
        Ok(Self::read_trailer_summary(file, length)?.0)
    }

    /// The first and last recorded hours across every branch, from the
    /// checksummed trailer's block index; `None` for a recording with no blocks.
    pub fn read_hour_span(path: &Path) -> io::Result<Option<(Hour, Hour)>> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        Ok(Self::read_trailer_summary(&mut file, length)?.1)
    }

    /// The partial notice, when readable, and the recorded hour span.
    fn read_trailer_summary(
        file: &mut (impl Read + Seek),
        length: u64,
    ) -> io::Result<TrailerSummary> {
        if length < (FOOTER_SIZE + 6) as u64 {
            return Err(invalid("missing replay footer"));
        }
        if file.seek(SeekFrom::End(0))? != length {
            return Err(invalid("replay length differs from stream"));
        }
        file.seek(SeekFrom::Start(0))?;
        let mut prefix = [0; 6];
        file.read_exact(&mut prefix)?;
        if &prefix[..4] != HEADER_MAGIC {
            return Err(invalid("invalid replay header magic"));
        }
        let format = FormatVersion::read(u16::from_le_bytes(prefix[4..].try_into().unwrap()))?;
        file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
        let mut footer_bytes = [0; FOOTER_SIZE];
        file.read_exact(&mut footer_bytes)?;
        let mut footer = Decoder::new(&footer_bytes);
        expect_magic(&mut footer, FOOTER_MAGIC)?;
        let start = footer.u64()?;
        let trailer_length = footer.u64()?;
        let _checksum = footer.u32()?;
        if start < 6 || start.checked_add(trailer_length) != Some(length - FOOTER_SIZE as u64) {
            return Err(invalid("invalid trailer extent"));
        }
        let mut bytes = engine::allocation::vec(to_usize(trailer_length)?)?;
        file.seek(SeekFrom::Start(start))?;
        file.take(trailer_length)
            .read_to_end(&mut bytes)
            .map_err(engine::allocation::allocation_error)?;
        if bytes.len() as u64 != trailer_length {
            return Err(invalid("truncated replay trailer"));
        }
        let mut trailer = Decoder::new(&bytes);
        expect_magic(&mut trailer, TRAILER_MAGIC)?;
        let sessions = decode_list(&mut trailer, SessionStart::decode)?;
        let _branches = decode_list(&mut trailer, BranchStart::decode)?;
        let mut warnings = Vec::new();
        let mut damaged_parts = 0;
        let _gaps = decode_list_recovering(
            &mut trailer,
            Gap::decode,
            "gap",
            &mut warnings,
            &mut damaged_parts,
        )?;
        let controls = decode_list_recovering(
            &mut trailer,
            |input| ControlRecord::decode_version(input, format),
            "control",
            &mut warnings,
            &mut damaged_parts,
        )?;
        let _recovery = decode_list_recovering(
            &mut trailer,
            RecoveryNote::decode,
            "recovery note",
            &mut warnings,
            &mut damaged_parts,
        )?;
        let count = trailer.count(40)?;
        let mut span: Option<(Hour, Hour)> = None;
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
        // A readable trailer prefix still supplies the terminal notice and the
        // hour span. Full opening reports checksum failures and trailing bytes
        // as warnings.
        if validate_partial_controls(&controls).is_err() {
            return Ok((None, span));
        }
        for control in &controls {
            if !sessions
                .iter()
                .any(|session| session.id == control.session())
            {
                return Ok((None, span));
            }
        }
        let partial = controls
            .into_iter()
            .find(|control| matches!(control, ControlRecord::Partial { .. }));
        Ok((partial, span))
    }

    pub fn from_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Self> {
        let segments: Vec<_> = segments.iter().map(Vec::as_slice).collect();
        Self::from_record_slices(
            campaign_id,
            &segments,
            &mut |_, _, _| Ok(()),
            None,
            MAX_BLOCK_RAW_BYTES,
        )
    }

    fn from_record_slices(
        campaign_id: Vec<u8>,
        segments: &[&[Record]],
        observe: &mut FrameObserver<'_>,
        read_frame: Option<&mut FrameReader<'_>>,
        block_limit: usize,
    ) -> io::Result<Self> {
        if campaign_id.is_empty() {
            return Err(invalid("empty campaign identity"));
        }
        let identity = segments
            .iter()
            .flat_map(|segment| segment.iter())
            .find_map(|record| match record {
                Record::SessionStart(session) => Some(session.identity.clone()),
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
        let pending = collect(
            segments,
            &identity,
            &campaign_id,
            None,
            KEYFRAME_INTERVAL,
            observe,
            read_frame,
            block_limit,
        )?;
        let mut index = Vec::new();
        append_blocks(&mut bytes, &mut index, &pending.blocks)?;
        finish_file(
            &mut bytes,
            &pending.sessions,
            &pending.branches,
            &pending.gaps,
            &pending.controls,
            &[],
            &index,
        )?;
        Self::open(bytes)
    }

    /// Existing compressed blocks and their original offsets are copied verbatim.
    pub fn append(&self, segments: &[Vec<Record>]) -> io::Result<Self> {
        let segments: Vec<_> = segments.iter().map(Vec::as_slice).collect();
        self.append_record_slices(&segments, &mut |_, _, _| Ok(()), None)
    }

    fn append_record_slices(
        &self,
        segments: &[&[Record]],
        observe: &mut FrameObserver<'_>,
        read_frame: Option<&mut FrameReader<'_>>,
    ) -> io::Result<Self> {
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
            read_frame,
            MAX_BLOCK_RAW_BYTES,
        )?;
        let mut bytes = self.bytes[..self.trailer_start].to_vec();
        let mut index = self.index.clone();
        append_blocks(&mut bytes, &mut index, &pending.blocks)?;
        let mut sessions = self.sessions.clone();
        sessions.extend(pending.sessions);
        let mut branches = self.branches.clone();
        branches.extend(pending.branches);
        let mut gaps = self.gaps.clone();
        gaps.extend(pending.gaps);
        let mut controls = self.controls.clone();
        controls.extend(pending.controls);
        finish_file(
            &mut bytes,
            &sessions,
            &branches,
            &gaps,
            &controls,
            &self.recovery_notes,
            &index,
        )?;
        Self::open(bytes)
    }

    pub fn from_recovered(campaign_id: Vec<u8>, segments: &[(u32, Recovery)]) -> io::Result<Self> {
        Self::from_recovered_with(campaign_id, segments, |_, _, _| Ok(()))
    }

    /// Report each frame using its validated decoded view and the sizes produced
    /// by the block encoder. Observation does not publish the file; callers must
    /// report success only after the returned replay has been written atomically.
    pub fn from_recovered_with(
        campaign_id: Vec<u8>,
        segments: &[(u32, Recovery)],
        mut observe: impl FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()>,
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        Self::from_record_slices(
            campaign_id,
            &records,
            &mut observe,
            None,
            MAX_BLOCK_RAW_BYTES,
        )?
        .add_recovery_notes(segments)
    }

    pub fn append_recovered(&self, segments: &[(u32, Recovery)]) -> io::Result<Self> {
        self.append_recovered_with(segments, |_, _, _| Ok(()))
    }

    /// Observe only the new frames; existing compressed blocks are copied intact.
    pub fn append_recovered_with(
        &self,
        segments: &[(u32, Recovery)],
        mut observe: impl FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()>,
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        self.append_record_slices(&records, &mut observe, None)?
            .add_recovery_notes(segments)
    }

    /// Recovered records may keep only frame metadata. Restore each frame on
    /// demand from its validated segment prefix, without retaining its rows.
    pub fn from_recovered_with_reader(
        campaign_id: Vec<u8>,
        segments: &[(u32, Recovery)],
        mut read: impl FnMut(u32, &Frame) -> io::Result<Frame>,
        mut observe: impl FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()>,
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        let mut numbers: Vec<_> = segments.iter().map(|(number, _)| *number).collect();
        numbers.sort_unstable();
        let mut read_frame = |position: usize, frame: &Frame| read(numbers[position], frame);
        Self::from_record_slices(
            campaign_id,
            &records,
            &mut observe,
            Some(&mut read_frame),
            MAX_BLOCK_RAW_BYTES,
        )?
        .add_recovery_notes(segments)
    }

    /// Append using the same bounded frame restoration as initial finalization.
    pub fn append_recovered_with_reader(
        &self,
        segments: &[(u32, Recovery)],
        mut read: impl FnMut(u32, &Frame) -> io::Result<Frame>,
        mut observe: impl FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()>,
    ) -> io::Result<Self> {
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        let mut numbers: Vec<_> = segments.iter().map(|(number, _)| *number).collect();
        numbers.sort_unstable();
        let mut read_frame = |position: usize, frame: &Frame| read(numbers[position], frame);
        self.append_record_slices(&records, &mut observe, Some(&mut read_frame))?
            .add_recovery_notes(segments)
    }

    fn add_recovery_notes(mut self, segments: &[(u32, Recovery)]) -> io::Result<Self> {
        self.require_undamaged()?;
        let mut sorted: Vec<_> = segments.iter().collect();
        sorted.sort_unstable_by_key(|(number, _)| *number);
        for (number, recovery) in sorted {
            let session = recovery.records.first().and_then(|record| match record {
                Record::SessionStart(session) => Some(session.id),
                Record::BranchStart(branch) => Some(branch.session),
                _ => None,
            });
            if let Some(reason) = &recovery.damage {
                self.recovery_notes.push(RecoveryNote {
                    session,
                    segment: *number,
                    valid_bytes: recovery.valid_bytes as u64,
                    reason: reason.clone(),
                });
            }
        }
        self.bytes.truncate(self.trailer_start);
        finish_file(
            &mut self.bytes,
            &self.sessions,
            &self.branches,
            &self.gaps,
            &self.controls,
            &self.recovery_notes,
            &self.index,
        )?;
        Self::open(self.bytes)
    }

    pub fn content_verified(&self) -> bool {
        matches!(self.identity.verification, Verification::Verified { .. })
            && self.sessions.iter().all(|session| {
                matches!(session.identity.verification, Verification::Verified { .. })
            })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
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
                    TableView::Truncated { .. } | TableView::Unavailable { .. } => None,
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
                engine::allocation::reserve(&mut tails, 1)?;
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
        if bytes.len() < FOOTER_SIZE {
            return Err(invalid("missing replay footer"));
        }
        let footer_start = bytes.len() - FOOTER_SIZE;
        let mut footer = Decoder::new(&bytes[footer_start..]);
        expect_magic(&mut footer, FOOTER_MAGIC)?;
        let trailer_start = to_usize(footer.u64()?)?;
        let trailer_length = to_usize(footer.u64()?)?;
        let trailer_crc = footer.u32()?;
        footer.finish()?;
        if trailer_start.checked_add(trailer_length) != Some(footer_start) {
            return Err(invalid("invalid trailer extent"));
        }
        let trailer_bytes = bytes
            .get(trailer_start..footer_start)
            .ok_or_else(|| invalid("trailer outside replay"))?;
        let mut warnings = Vec::new();
        if crc32fast::hash(trailer_bytes) != trailer_crc {
            engine::allocation::reserve(&mut warnings, 1)?;
            warnings.push("trailer CRC mismatch".into());
        }
        let mut replay = Self::open_metadata(
            &bytes[..trailer_start],
            trailer_bytes,
            trailer_start,
            warnings,
        )?;
        replay.bytes = bytes;
        Ok(replay)
    }

    /// Container metadata validation;
    /// `warnings` carries what the caller already found (a trailer checksum failure).
    fn open_metadata(
        header_bytes: &[u8],
        trailer_bytes: &[u8],
        trailer_start: usize,
        mut warnings: Vec<String>,
    ) -> io::Result<Self> {
        let mut header = Decoder::new(header_bytes);
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
        let header_end = header_bytes.len() - header.remaining();
        let mut trailer = Decoder::new(trailer_bytes);
        expect_magic(&mut trailer, TRAILER_MAGIC)?;
        let sessions = decode_list(&mut trailer, SessionStart::decode)?;
        let branches = decode_list(&mut trailer, BranchStart::decode)?;
        let mut damaged_parts = 0;
        let mut gaps = decode_list_recovering(
            &mut trailer,
            Gap::decode,
            "gap",
            &mut warnings,
            &mut damaged_parts,
        )?;
        let mut controls = decode_list_recovering(
            &mut trailer,
            |input| ControlRecord::decode_version(input, format),
            "control",
            &mut warnings,
            &mut damaged_parts,
        )?;
        let recovery_notes = decode_list_recovering(
            &mut trailer,
            RecoveryNote::decode,
            "recovery note",
            &mut warnings,
            &mut damaged_parts,
        )?;
        let count = trailer.count(40)?;
        let mut index = engine::allocation::vec(count)?;
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
            engine::allocation::reserve(&mut warnings, 1)?;
            warnings.push("trailing bytes in replay trailer".into());
        }
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
            engine::allocation::reserve(&mut warnings, 1)?;
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
            keyframe_interval,
            &mut warnings,
            &mut damaged_parts,
            &mut damaged_range,
        )?;
        let partial_control = controls
            .iter()
            .position(|control| matches!(control, ControlRecord::Partial { .. }));
        let mut verified = engine::allocation::vec(index.len())?;
        for _ in &index {
            verified.push(AtomicBool::new(false));
        }
        Ok(Self {
            bytes: Vec::new(),
            format,
            campaign_id,
            identity,
            keyframe_interval,
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
        self.seek_selected_with(branch, hour, tables, || Ok(()))
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
        check()?;
        let gap = |reason| SeekResult::Gap {
            branch,
            hour,
            reason,
        };
        if !self.branches.iter().any(|entry| entry.id == branch) {
            return Ok(gap("unknown branch".into()));
        }
        let Some(index) = self.index.iter().find(|index| {
            index.branch == branch && index.first_hour <= hour && hour <= index.last_hour
        }) else {
            return Ok(gap(self.gap_reason(branch, hour)?));
        };
        let selected = tables
            .map(|tables| -> io::Result<_> {
                let include_campaign = !tables.contains(&TableId::Campaign);
                let capacity =
                    engine::allocation::add(tables.len(), usize::from(include_campaign))?;
                let mut selected = engine::allocation::vec(capacity)?;
                selected.extend_from_slice(tables);
                if include_campaign {
                    selected.push(TableId::Campaign);
                }
                Ok(selected)
            })
            .transpose()?;
        let mut decoder = selected
            .as_deref()
            .map_or_else(FrameDecoder::new, FrameDecoder::for_tables);
        let mut found = None;
        let read = self.visit_block(index, selected.as_deref(), |frame| {
            check()?;
            if frame.hour > hour {
                return Ok(false);
            }
            let decoded = match decoder.advance(&frame) {
                Ok(decoded) => decoded,
                Err(error) => {
                    if !recording_damage(&error) {
                        return Err(error);
                    }
                    found = Some(gap(crate::codec::try_format(format_args!(
                        "invalid frame baseline at hour {}: {}",
                        frame.hour,
                        crate::codec::diagnostic_text(&error.to_string())
                    ))?));
                    return Ok(false);
                }
            };
            if let Err(error) = validate_campaign(decoded, &self.campaign_id) {
                found = Some(gap(engine::allocation::string(
                    crate::codec::diagnostic_text(&error.to_string()),
                )?));
                return Ok(false);
            }
            if frame.hour == hour {
                found = Some(SeekResult::Frame(engine::allocation::boxed(
                    decoded.clone(),
                )?));
                return Ok(false);
            }
            Ok(true)
        });
        if let Err(error) = read {
            if !recording_damage(&error) {
                return Err(error);
            }
            return Ok(gap(crate::codec::try_format(format_args!(
                "unreadable block: {}",
                crate::codec::diagnostic_text(&error.to_string())
            ))?));
        }
        match found {
            Some(found) => Ok(found),
            None => Ok(gap(self.gap_reason(branch, hour)?)),
        }
    }

    fn gap_reason(&self, branch: BranchId, hour: Hour) -> io::Result<String> {
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
            engine::allocation::reserve_string(&mut reason, separator.len() + text.len())?;
            reason.push_str(separator);
            reason.push_str(text);
        }
        if reason.is_empty() {
            engine::allocation::string("hour was not recorded")
        } else {
            Ok(reason)
        }
    }

    /// Decode one block for sequential playback, inspection, or history recovery.
    pub fn read_block(&self, index: &BlockIndex) -> io::Result<Vec<Frame>> {
        self.read_block_inner(index, None)
    }

    /// [`Replay::read_block`], decoding the frame payloads on `parallel`'s
    /// worker threads once their boundaries are known. The same frames and the
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
    /// The block is streamed, so its raw bytes are never held whole.
    pub fn read_block_selected(
        &self,
        index: &BlockIndex,
        tables: &[crate::rows::TableId],
    ) -> io::Result<Vec<Frame>> {
        let mut frames = Vec::new();
        self.visit_block(index, Some(tables), |frame| {
            engine::allocation::reserve(&mut frames, 1)?;
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
        if let Some(block) = self.grouped_block(index)? {
            let frames =
                check_block_frames(index, grouped::read(self, index, block, None, parallel)?)?;
            self.mark_verified(index);
            return Ok(frames);
        }
        let mut inflated = self.inflate(index)?;
        let mut raw = engine::allocation::vec(to_usize(index.raw_length)?)?;
        inflated
            .by_ref()
            .take(index.raw_length)
            .read_to_end(&mut raw)
            .map_err(engine::allocation::allocation_error)?;
        if raw.len() as u64 != index.raw_length || inflated.read(&mut [0])? != 0 {
            return Err(invalid("block raw length mismatch"));
        }
        let mut decoder = Decoder::new(&raw);
        let count = decoder.count(4)?;
        if count == 0 {
            return Err(invalid("empty block"));
        }
        validate_frame_count(index, count as u64)?;
        // The payload boundaries first, then the payloads decoded (in parallel
        // when asked), then every check in position order: the first error
        // reported is the one a frame-by-frame reader would have met first.
        let mut payloads = engine::allocation::vec(count)?;
        let mut boundary_error = None;
        for _ in 0..count {
            let payload = decoder
                .u32()
                .and_then(|length| decoder.take(length as usize));
            match payload {
                Ok(payload) => payloads.push(payload),
                Err(error) => {
                    boundary_error = Some(error);
                    break;
                }
            }
        }
        let decoded = decode_payloads(&payloads, parallel, self.format)?;
        let mut frames = engine::allocation::vec(count)?;
        let mut order = BlockOrder::new(index);
        for frame in decoded {
            let frame = frame?;
            order.next(&frame)?;
            frames.push(frame);
        }
        if let Some(error) = boundary_error {
            return Err(error);
        }
        decoder.finish()?;
        order.finish()?;
        if let Some(position) = self.block_position(index) {
            self.verified[position].store(true, Ordering::Release);
        }
        Ok(frames)
    }

    /// Each frame of a block in order, inflated and decoded one at a time with only
    /// `selected` tables kept, until `visit` returns false: one payload is held at a
    /// time however large the block. Every read of a block not yet verified decodes
    /// and checks all of it, as [`Replay::read_block`] does, even after `visit` stops:
    /// only a block that has passed every check may be left partway through.
    pub fn visit_block(
        &self,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        mut visit: impl FnMut(Frame) -> io::Result<bool>,
    ) -> io::Result<()> {
        if let Some(block) = self.grouped_block(index)? {
            let frames =
                check_block_frames(index, grouped::read(self, index, block, selected, None)?)?;
            for frame in frames {
                if !visit(frame)? {
                    break;
                }
            }
            // A grouped read inflates only the selected tables' sections; as for
            // an interleaved block, a first read checks all of it, here every
            // section's checksum, even after `visit` stops.
            if selected.is_some() && !self.is_verified(index) {
                grouped::verify(block, index)?;
            }
            self.mark_verified(index);
            return Ok(());
        }
        let mut raw = Inflated {
            stream: self.inflate(index)?,
            read: 0,
        };
        let result = self.visit_inflated(&mut raw, index, selected, visit);
        // A whole read checks the length first, so a block of the wrong length says so.
        result.map_err(|error| {
            if engine::allocation::is_allocation_error(&error)
                || error.kind() == io::ErrorKind::Interrupted
            {
                return error;
            }
            match io::copy(&mut raw.stream, &mut io::sink()) {
                Err(inflate) => inflate,
                Ok(rest) if raw.read + rest != index.raw_length => {
                    invalid("block raw length mismatch")
                }
                Ok(_) => error,
            }
        })
    }

    fn visit_inflated(
        &self,
        raw: &mut Inflated<impl Read>,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        mut visit: impl FnMut(Frame) -> io::Result<bool>,
    ) -> io::Result<()> {
        let mut word = [0; 4];
        raw.fill(&mut word)?;
        let count = u64::from(u32::from_le_bytes(word));
        let mut remaining = index.raw_length.saturating_sub(4);
        if count > remaining / 4 {
            return Err(invalid_static(
                "collection length exceeds remaining payload",
            ));
        }
        if count == 0 {
            return Err(invalid("empty block"));
        }
        validate_frame_count(index, count)?;
        let position = self.block_position(index);
        let verified =
            position.is_some_and(|position| self.verified[position].load(Ordering::Acquire));
        let mut order = BlockOrder::new(index);
        let mut payload = Vec::new();
        let mut visiting = true;
        for _ in 0..count {
            raw.fill(&mut word)?;
            let length = u64::from(u32::from_le_bytes(word));
            remaining = remaining
                .checked_sub(4 + length)
                .ok_or_else(|| invalid_static("truncated field"))?;
            let length = to_usize(length)?;
            let additional = length.saturating_sub(payload.len());
            engine::allocation::reserve(&mut payload, additional)?;
            payload.resize(length, 0);
            raw.fill(&mut payload)?;
            let frame = decode_payload(&payload, selected, self.format)?;
            order.next(&frame)?;
            if visiting && !visit(frame)? {
                if verified {
                    return Ok(());
                }
                visiting = false;
            }
        }
        if remaining != 0 {
            return Err(invalid_static("trailing bytes"));
        }
        if raw.stream.read(&mut [0])? != 0 {
            return Err(invalid("block raw length mismatch"));
        }
        order.finish()?;
        if let Some(position) = position {
            self.verified[position].store(true, Ordering::Release);
        }
        Ok(())
    }

    /// The block's zstd stream, at most one byte past its recorded raw length.
    fn inflate(&self, index: &BlockIndex) -> io::Result<impl Read + '_> {
        if index.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
            return Err(invalid("block exceeds raw size limit"));
        }
        let start = to_usize(index.offset)?;
        let end = to_usize(
            index
                .offset
                .checked_add(index.length)
                .ok_or_else(|| invalid("block offset overflow"))?,
        )?;
        let compressed = self
            .bytes
            .get(start..end)
            .ok_or_else(|| invalid("block outside replay"))?;
        if compressed.get(..4) != Some(&[0x28, 0xb5, 0x2f, 0xfd]) {
            return Err(invalid("block is not a zstd frame"));
        }
        let frame_size = zstd::zstd_safe::find_frame_compressed_size(compressed)
            .map_err(|_| invalid("invalid zstd frame"))?;
        if frame_size != compressed.len() {
            return Err(invalid(
                "block contains trailing or concatenated zstd frames",
            ));
        }
        let mut decompressor = zstd::stream::read::Decoder::with_buffer(compressed)
            .map_err(zstd_error)?
            .single_frame();
        decompressor.window_log_max(26).map_err(zstd_error)?;
        Ok(CheckedInflation(decompressor.take(index.raw_length + 1)))
    }

    fn mark_verified(&self, index: &BlockIndex) {
        if let Some(position) = self.block_position(index) {
            self.verified[position].store(true, Ordering::Release);
        }
    }

    /// A grouped block's stored bytes; `None` for an interleaved block.
    fn grouped_block(&self, index: &BlockIndex) -> io::Result<Option<&[u8]>> {
        if index.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
            return Err(invalid("block exceeds raw size limit"));
        }
        let block = self.block_bytes(index)?;
        Ok(grouped::is_grouped(block).then_some(block))
    }

    /// A block's stored bytes.
    fn block_bytes(&self, index: &BlockIndex) -> io::Result<&[u8]> {
        let start = to_usize(index.offset)?;
        let end = to_usize(
            index
                .offset
                .checked_add(index.length)
                .ok_or_else(|| invalid("block offset overflow"))?,
        )?;
        self.bytes
            .get(start..end)
            .ok_or_else(|| invalid("block outside replay"))
    }

    /// The compressed sections of a grouped block; `None` for an interleaved
    /// block, whose single zstd frame has no sections.
    pub fn block_sections(&self, index: &BlockIndex) -> io::Result<Option<Vec<SectionSize>>> {
        let block = self.block_bytes(index)?;
        if !grouped::is_grouped(block) {
            return Ok(None);
        }
        grouped::sections(block, index).map(Some)
    }

    /// The table sections of a grouped block split into their columns, each
    /// also compressed alone; `None` for an interleaved block.
    pub fn block_columns(&self, index: &BlockIndex) -> io::Result<Option<Vec<ColumnSizes>>> {
        let block = self.block_bytes(index)?;
        if !grouped::is_grouped(block) {
            return Ok(None);
        }
        grouped::section_columns(block, index).map(Some)
    }

    /// Every section of a grouped block with its plain bytes compressed again
    /// at each of `levels` (zstd, windows within what readers accept);
    /// `None` for an interleaved block.
    pub fn block_levels(
        &self,
        index: &BlockIndex,
        levels: &[i32],
    ) -> io::Result<Option<Vec<SectionLevels>>> {
        let block = self.block_bytes(index)?;
        if !grouped::is_grouped(block) {
            return Ok(None);
        }
        grouped::section_levels(block, index, levels).map(Some)
    }

    /// This recording's frames written again by the current block writer, in
    /// memory, keeping its sessions, branches, gaps, controls and recovery
    /// notes. Size attribution compares layouts with it; the frames, their
    /// format and their meaning are unchanged, so only current-format
    /// recordings can be written again.
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
        // One segment per session with frames: its start, then each branch's
        // start and frames in block order. The frames are metadata here and
        // are read back from this recording block by block; the trailer is
        // copied, so controls and gaps need no place among the frames.
        let mut segments: Vec<Vec<Record>> = Vec::new();
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
                segments.push(vec![Record::SessionStart(started.clone())]);
                session = Some(start.session);
                branch = None;
            }
            let records = segments.last_mut().unwrap();
            if branch != Some(block.branch) {
                records.push(Record::BranchStart(start.clone()));
                branch = Some(block.branch);
            }
            for frame in self.read_block_selected(block, &[])? {
                records.push(Record::Frame(Frame {
                    branch: frame.branch,
                    hour: frame.hour,
                    keyframe: frame.keyframe,
                    metrics: Metrics::default(),
                    tables: std::array::from_fn(|_| crate::frame::TableUpdate::Unavailable {
                        reason: "re-encoding metadata only".into(),
                    }),
                }));
            }
        }
        if segments.is_empty() {
            return Self::open(self.bytes.clone());
        }
        // Frames in block order, one block in memory.
        let mut blocks = self.index.iter();
        let mut frames: std::vec::IntoIter<Frame> = Vec::new().into_iter();
        let mut read = |_: usize, wanted: &Frame| -> io::Result<Frame> {
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
        let slices: Vec<&[Record]> = segments.iter().map(Vec::as_slice).collect();
        let mut replay = Self::from_record_slices(
            self.campaign_id.clone(),
            &slices,
            &mut observe,
            Some(&mut read),
            MAX_BLOCK_RAW_BYTES,
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
        Self::open(replay.bytes)
    }

    /// Validated block extents have increasing offsets even across skipped bytes.
    fn block_position(&self, index: &BlockIndex) -> Option<usize> {
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
        self.require_undamaged()?;
        let mut file = publisher.create()?;
        checkpoint(PublishStage::TempCreated)?;
        file.write_all(&self.bytes[..self.header_end])?;
        checkpoint(PublishStage::HeaderWritten)?;
        for (position, block) in self.index.iter().enumerate() {
            let start = to_usize(block.offset)?;
            let end = to_usize(block.offset + block.length)?;
            let midpoint = start + (end - start) / 2;
            file.write_all(&self.bytes[start..midpoint])?;
            checkpoint(PublishStage::BlockPartWritten(position))?;
            file.write_all(&self.bytes[midpoint..end])?;
            checkpoint(PublishStage::BlockWritten(position))?;
        }
        let footer_start = self.bytes.len() - FOOTER_SIZE;
        file.write_all(&self.bytes[self.trailer_start..footer_start])?;
        checkpoint(PublishStage::TrailerWritten)?;
        file.write_all(&self.bytes[footer_start..])?;
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
    type Writer = File;
    fn create(&mut self) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.temporary)
    }
    fn sync(&mut self, writer: &mut File) -> io::Result<()> {
        writer.sync_all()
    }
    fn replace(&mut self, writer: File) -> io::Result<()> {
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
    crate::publication::replace_file(source, destination)
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
    engine::allocation::reserve(out, 4)?;
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
    let mut values = engine::allocation::vec(count)?;
    for _ in 0..count {
        let payload = decoder.bytes()?;
        let mut item = Decoder::new(&payload);
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
    let mut values = engine::allocation::vec(count)?;
    for _ in 0..count {
        // The outer length must be readable; errors within that boundary can
        // withhold this record without losing subsequent trailer lists/index.
        let payload = decoder.bytes()?;
        let mut item = Decoder::new(&payload);
        match decode(&mut item).and_then(|value| {
            item.finish()?;
            Ok(value)
        }) {
            Ok(value) => values.push(value),
            Err(error)
                if engine::allocation::is_allocation_error(&error)
                    || crate::format::is_layout_refusal(&error) =>
            {
                return Err(error);
            }
            Err(error) => {
                engine::allocation::reserve(warnings, 1)?;
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

fn append_blocks(
    bytes: &mut Vec<u8>,
    index: &mut Vec<BlockIndex>,
    blocks: &[CompressedBlock],
) -> io::Result<()> {
    engine::allocation::reserve(index, blocks.len())?;
    for block in blocks {
        engine::allocation::reserve(bytes, block.bytes.len())?;
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
    let trailer_start = bytes.len();
    bytes.extend_from_slice(TRAILER_MAGIC);
    encode_list(bytes, sessions, |session, out| session.encode(out))?;
    encode_list(bytes, branches, BranchStart::encode)?;
    encode_list(bytes, gaps, Gap::encode)?;
    encode_list(bytes, controls, ControlRecord::encode)?;
    encode_list(bytes, recovery_notes, RecoveryNote::encode)?;
    put_count(bytes, index.len())?;
    for block in index {
        bytes.extend_from_slice(&block.branch.to_le_bytes());
        bytes.extend_from_slice(&block.first_hour.to_le_bytes());
        bytes.extend_from_slice(&block.last_hour.to_le_bytes());
        bytes.extend_from_slice(&block.offset.to_le_bytes());
        bytes.extend_from_slice(&block.length.to_le_bytes());
        bytes.extend_from_slice(&block.raw_length.to_le_bytes());
    }
    let trailer_length = bytes.len() - trailer_start;
    let crc = crc32fast::hash(&bytes[trailer_start..]);
    bytes.extend_from_slice(FOOTER_MAGIC);
    bytes.extend_from_slice(&(trailer_start as u64).to_le_bytes());
    bytes.extend_from_slice(&(trailer_length as u64).to_le_bytes());
    bytes.extend_from_slice(&crc.to_le_bytes());
    Ok(())
}

fn validate_campaign(frame: &DecodedFrame, campaign_id: &[u8]) -> io::Result<()> {
    if let TableView::Complete(rows) = &*frame.tables[TableId::Campaign.index()]
        && (rows.len() != 1
            || !matches!(rows, TableRows::Campaign(values) if values[0].id == campaign_id))
    {
        return Err(invalid(
            "frame campaign identity differs from replay header",
        ));
    }
    Ok(())
}

fn ordered_records(segments: &[(u32, Recovery)]) -> io::Result<Vec<&[Record]>> {
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

/// The sessions, branches, gaps, controls and new blocks of `segments`. The
/// blocks are written on this thread in recorded order and packed on
/// [`pack::workers`] threads, this one included ([`pack::blocks`]).
#[allow(clippy::too_many_arguments)]
fn collect(
    segments: &[&[Record]],
    identity: &Identity,
    campaign_id: &[u8],
    existing: Option<&Replay>,
    interval: u32,
    observe: &mut FrameObserver<'_>,
    read_frame: Option<&mut FrameReader<'_>>,
    block_limit: usize,
) -> io::Result<Pending> {
    let (mut pending, blocks) = pack::blocks(
        pack::workers(),
        |block: grouped::BlockWriter, cores: &pack::Cores| block.compress(cores),
        |packer| {
            write_blocks(
                packer,
                segments,
                identity,
                campaign_id,
                existing,
                interval,
                observe,
                read_frame,
                block_limit,
            )
        },
    )?;
    pending.blocks = blocks;
    Ok(pending)
}

/// [`collect`]'s records in order, each finished block handed to `packer`.
#[allow(clippy::too_many_arguments)]
fn write_blocks(
    packer: &mut pack::Packer<'_, grouped::BlockWriter, CompressedBlock>,
    segments: &[&[Record]],
    identity: &Identity,
    campaign_id: &[u8],
    existing: Option<&Replay>,
    interval: u32,
    observe: &mut FrameObserver<'_>,
    mut read_frame: Option<&mut FrameReader<'_>>,
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
    let mut block: Option<grouped::BlockWriter> = None;
    // Appended blocks anchor to the file's earlier keyframes too.
    let mut anchors = match existing {
        Some(replay) => grouped::Anchors::seed(replay)?,
        None => grouped::Anchors::default(),
    };
    for (position, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            continue;
        }
        if !matches!(segment[0], Record::SessionStart(_) | Record::BranchStart(_)) {
            return Err(invalid(
                "segment does not begin with session or branch context",
            ));
        }
        let mut current_session = None;
        let mut current_branch = None;
        let mut needs_keyframe = true;
        let mut frame_decoder = FrameDecoder::new();
        let mut segment_state = SegmentState::default();
        for record in *segment {
            segment_state.accept(record)?;
            match record {
                Record::SessionStart(session) => {
                    if !session.identity.same_content(identity) {
                        return Err(invalid("session content differs from replay content"));
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
                        packer.submit(previous.written())?;
                    }
                    current_session = Some(session.id);
                    current_branch = None;
                    needs_keyframe = true;
                }
                Record::BranchStart(branch) => {
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
                        packer.submit(previous.written())?;
                    }
                    current_session = Some(branch.session);
                    current_branch = Some(branch.id);
                    needs_keyframe = true;
                    frame_decoder = FrameDecoder::new();
                }
                Record::Frame(frame) => {
                    let restored;
                    let frame = if let Some(read) = read_frame.as_mut() {
                        restored = read(position, frame)?;
                        if (restored.branch, restored.hour, restored.keyframe)
                            != (frame.branch, frame.hour, frame.keyframe)
                        {
                            return Err(invalid(
                                "restored spool frame differs from recovered metadata",
                            ));
                        }
                        &restored
                    } else {
                        frame
                    };
                    if current_branch != Some(frame.branch) {
                        return Err(invalid("frame has no matching branch context"));
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
                            packer.submit(previous.written())?;
                        }
                        block = Some(grouped::BlockWriter::new(frame));
                    }
                    let current = block
                        .as_mut()
                        .ok_or_else(|| invalid("frame has no block keyframe"))?;
                    if i64::from(frame.hour) - i64::from(current.first_hour) >= i64::from(interval)
                    {
                        return Err(invalid("block exceeds its keyframe hour interval"));
                    }
                    let decoded = frame_decoder.advance(frame)?;
                    validate_campaign(decoded, campaign_id)?;
                    let mut payload = Vec::new();
                    let mut sizes = frame.encode_with_sizes(&mut payload)?;
                    let promoted;
                    let emitted = if !current.fits(payload.len(), &sizes, block_limit) {
                        if current.frames == 0 {
                            return Err(invalid(
                                "single replay frame exceeds raw block size limit",
                            ));
                        }
                        // Block boundaries need an independent baseline. Snapshot
                        // the already decoded hour, retaining sample origins and
                        // native values; keep decoding later spool deltas normally.
                        promoted = block_keyframe(decoded)?;
                        payload.clear();
                        sizes = promoted.encode_with_sizes(&mut payload)?;
                        let next = grouped::BlockWriter::new(&promoted);
                        if !next.fits(payload.len(), &sizes, block_limit) {
                            return Err(invalid(
                                "single replay keyframe exceeds raw block size limit",
                            ));
                        }
                        packer.submit(block.replace(next).unwrap().written())?;
                        &promoted
                    } else {
                        frame
                    };
                    block.as_mut().unwrap().push(
                        emitted,
                        decoded,
                        &payload,
                        &sizes,
                        &mut anchors,
                        block_limit,
                        packer.cores(),
                    )?;
                    observe(emitted, decoded, sizes)?;
                    last_hours.insert(frame.branch, frame.hour);
                    needs_keyframe = false;
                }
                Record::Gap(gap) => {
                    gap.validate()?;
                    if current_branch != Some(gap.branch) {
                        return Err(invalid("gap has no matching branch context"));
                    }
                    pending.gaps.push(gap.clone());
                }
                Record::Control(control) => {
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
            packer.submit(previous.written())?;
        }
    }
    // A complete CRC-valid record can survive a failed flush. Preserve its frame
    // and the reported write-loss diagnostic; seeking gives the recovered frame priority.
    Ok(pending)
}

fn block_keyframe(decoded: &DecodedFrame) -> io::Result<Frame> {
    let mut tables = engine::allocation::vec(TABLE_COUNT)?;
    for table in TableId::ALL {
        tables.push(match &*decoded.tables[table.index()] {
            TableView::Complete(rows) => crate::frame::TableUpdate::Snapshot(rows.try_clone()?),
            TableView::Unavailable { reason } => crate::frame::TableUpdate::Unavailable {
                reason: engine::allocation::string(reason)?,
            },
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
    interval: u32,
    warnings: &mut Vec<String>,
    damaged_parts: &mut usize,
    damaged_range: &mut Option<(Hour, Hour)>,
) -> io::Result<()> {
    if sessions.is_empty() {
        return Err(invalid("replay has no sessions"));
    }
    let mut session_ids = HashSet::new();
    engine::allocation::reserve_set(&mut session_ids, sessions.len())?;
    for session in sessions {
        if !session_ids.insert(session.id) || !session.identity.same_content(identity) {
            return Err(invalid("duplicate session or conflicting content"));
        }
    }
    let mut branch_ids = HashMap::new();
    engine::allocation::reserve_map(&mut branch_ids, branches.len())?;
    let mut sessions_with_start_branch = HashSet::new();
    engine::allocation::reserve_set(&mut sessions_with_start_branch, sessions.len())?;
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
        engine::allocation::reserve(warnings, 1)?;
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
    engine::allocation::reserve_map(&mut last_hours, branches.len())?;
    let mut closed_sessions = HashSet::new();
    engine::allocation::reserve_set(&mut closed_sessions, sessions.len())?;
    let mut previous_branch = None;
    let mut previous_session = None;
    let mut kept = 0;
    for position in 0..index.len() {
        let block = &index[position];
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
            engine::allocation::reserve(warnings, 1)?;
            warnings.push(reason.clone());
            *damaged_parts += 1;
            if known_branch && valid_hours {
                engine::allocation::reserve(gaps, 1)?;
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
            engine::allocation::reserve(warnings, 1)?;
            warnings.push("unindexed bytes between replay blocks".into());
            *damaged_parts += 1;
        }
        if previous_branch != Some(block.branch) && last_hours.contains_key(&block.branch) {
            engine::allocation::reserve(warnings, 1)?;
            warnings.push("branch blocks are not contiguous".into());
        }
        let session = branch_ids[&block.branch];
        if previous_session != Some(session) {
            if let Some(previous) = previous_session {
                closed_sessions.insert(previous);
            }
            if closed_sessions.contains(&session) {
                engine::allocation::reserve(warnings, 1)?;
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
        engine::allocation::reserve(ordinals, 1)?;
        ordinals.push(position);
        kept += 1;
    }
    index.truncate(kept);
    if expected_offset != trailer_start as u64 {
        engine::allocation::reserve(warnings, 1)?;
        warnings.push("unindexed bytes before replay trailer".into());
        *damaged_parts += 1;
    }
    Ok(())
}

fn require_current_spools(segments: &[(u32, Recovery)]) -> io::Result<()> {
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

/// Each payload's frame, in payload order.
fn decode_payloads(
    payloads: &[&[u8]],
    parallel: Option<&Parallel<'_>>,
    format: FormatVersion,
) -> io::Result<Vec<io::Result<Frame>>> {
    let decode = |payload: &&[u8]| decode_payload(payload, None, format);
    match parallel {
        Some(parallel) => parallel_map(payloads, parallel, decode),
        None => engine::allocation::collect_vec(payloads.iter().map(decode)),
    }
}

fn decode_payload(
    payload: &[u8],
    selected: Option<&[crate::rows::TableId]>,
    format: FormatVersion,
) -> io::Result<Frame> {
    let mut decoder = Decoder::new(payload);
    let frame = Frame::decode_version(&mut decoder, selected, format)?;
    decoder.finish()?;
    Ok(frame)
}

/// A block's inflated stream and how much of it has been read.
struct Inflated<R> {
    stream: R,
    read: u64,
}

impl<R: Read> Inflated<R> {
    /// A stream ending early is a block shorter than its index says.
    fn fill(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        self.stream
            .read_exact(bytes)
            .map_err(|error| match error.kind() {
                io::ErrorKind::UnexpectedEof => invalid("block raw length mismatch"),
                _ => error,
            })?;
        self.read += bytes.len() as u64;
        Ok(())
    }
}

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
    if engine::allocation::is_allocation_error(&error)
        || error.to_string().contains("Allocation error")
    {
        engine::allocation::oom()
    } else if error.kind() == io::ErrorKind::Interrupted {
        error
    } else {
        invalid(error.to_string())
    }
}
struct CheckedInflation<R>(R);
impl<R: Read> Read for CheckedInflation<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.0.read(output).map_err(zstd_error)
    }
}

fn recording_damage(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
    )
}

fn validate_frame_count(index: &BlockIndex, count: u64) -> io::Result<()> {
    let hours = i64::from(index.last_hour) - i64::from(index.first_hour) + 1;
    if hours <= 0 || count > hours as u64 {
        return Err(invalid("block frame count exceeds hour range"));
    }
    Ok(())
}

impl<'a> BlockOrder<'a> {
    fn new(index: &'a BlockIndex) -> Self {
        Self {
            index,
            first: None,
            last: None,
        }
    }

    fn next(&mut self, frame: &Frame) -> io::Result<()> {
        if frame.branch != self.index.branch
            || frame.hour < self.index.first_hour
            || frame.hour > self.index.last_hour
            || frame.keyframe != self.last.is_none()
            || self.last.is_some_and(|hour| frame.hour <= hour)
        {
            return Err(invalid(
                "block frame branch, keyframe, or hour ordering mismatch",
            ));
        }
        self.first.get_or_insert(frame.hour);
        self.last = Some(frame.hour);
        Ok(())
    }

    fn finish(self) -> io::Result<()> {
        if self.first != Some(self.index.first_hour) || self.last != Some(self.index.last_hour) {
            return Err(invalid("block index hour range mismatch"));
        }
        Ok(())
    }
}

/// `work` over `items` in order, on up to `parallel.workers` threads.
fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    parallel: &Parallel<'_>,
    work: impl Fn(&T) -> R + Sync,
) -> io::Result<Vec<R>> {
    let workers = parallel
        .workers
        .min(items.len().div_ceil(PARALLEL_FRAMES))
        .max(1);
    if workers <= 1 {
        return engine::allocation::collect_vec(items.iter().map(&work));
    }
    let run = items.len().div_ceil(workers);
    let work = &work;
    let on_worker = parallel.on_worker;
    std::thread::scope(|scope| {
        let mut handles = engine::allocation::vec(workers)?;
        let mut out = engine::allocation::vec(items.len())?;
        for chunk in items.chunks(run) {
            let spawned = std::thread::Builder::new().spawn_scoped(scope, move || {
                on_worker();
                engine::allocation::collect_vec(chunk.iter().map(work))
            });
            handles.push((chunk, spawned.ok()));
        }
        for (chunk, handle) in handles {
            match handle {
                Some(handle) => match handle.join() {
                    Ok(results) => out.extend(results?),
                    Err(panic) => std::panic::resume_unwind(panic),
                },
                // A worker the OS refused leaves its run to this thread.
                None => out.extend(chunk.iter().map(work)),
            }
        }
        Ok(out)
    })
}

mod grouped;
mod pack;
mod patch;
pub use grouped::{ColumnSize, ColumnSizes, STORED_LEVEL, Section, SectionLevels, SectionSize};
#[cfg(test)]
pub(crate) use {
    grouped::{Layout, WRITTEN_LAYOUT},
    pack::WORKERS as PACK_WORKERS,
};

#[cfg(test)]
mod tests;
