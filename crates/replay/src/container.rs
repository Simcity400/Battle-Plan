//! Independently compressed blocks, an indexed trailer, and atomic publication.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::{
    BranchId, FORMAT_VERSION, Hour, KEYFRAME_INTERVAL, SessionId,
    branches::{BranchReason, BranchStart, BranchTracker, Gap},
    codec::{Decoder, invalid, invalid_static, put_bytes, put_string},
    format::FormatVersion,
    frame::{DecodedFrame, Frame, FrameDecoder, TABLE_COUNT, TableView},
    identity::{Identity, SessionStart, Verification},
    rows::{TableId, TableRows},
    spool::{ControlRecord, Record, Recovery, SegmentState},
};

const HEADER_MAGIC: &[u8; 4] = b"H4RP";
const TRAILER_MAGIC: &[u8; 4] = b"H4TR";
const FOOTER_MAGIC: &[u8; 4] = b"H4FT";
const FOOTER_SIZE: usize = 24;
pub const MAX_BLOCK_RAW_BYTES: usize = 1 << 30;

type FrameObserver<'a> =
    dyn FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()> + 'a;

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
    pub index: Vec<BlockIndex>,
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

struct RawBlock {
    branch: BranchId,
    first_hour: Hour,
    last_hour: Hour,
    frames: Vec<u8>,
    count: u32,
}

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
    fn push(&mut self, frame: &Frame) -> io::Result<[usize; TABLE_COUNT]> {
        let mut payload = Vec::new();
        let sizes = frame.encode_with_sizes(&mut payload)?;
        put_bytes(&mut self.frames, &payload)?;
        if self.frames.len() > MAX_BLOCK_RAW_BYTES - 4 {
            return Err(invalid("replay block exceeds raw size limit"));
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| invalid("too many frames"))?;
        self.last_hour = frame.hour;
        Ok(sizes)
    }
    fn raw(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity(self.frames.len() + 4);
        raw.extend_from_slice(&self.count.to_le_bytes());
        raw.extend_from_slice(&self.frames);
        raw
    }
}

struct Pending {
    sessions: Vec<SessionStart>,
    branches: Vec<BranchStart>,
    gaps: Vec<Gap>,
    controls: Vec<ControlRecord>,
    blocks: Vec<RawBlock>,
}

impl Replay {
    pub fn partial(&self) -> Option<&ControlRecord> {
        self.partial_control
            .and_then(|index| self.controls.get(index))
    }

    pub fn is_partial(&self) -> bool {
        self.partial().is_some()
    }

    /// Recording-list metadata reads the checksummed trailer without inflating blocks.
    /// Opening the replay still performs the full header and metadata validation.
    pub fn read_partial(path: &Path) -> io::Result<Option<ControlRecord>> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        Self::read_partial_from(&mut file, length)
    }

    fn read_partial_from(
        file: &mut (impl Read + Seek),
        length: u64,
    ) -> io::Result<Option<ControlRecord>> {
        if length < (FOOTER_SIZE + 6) as u64 {
            return Err(invalid("missing replay footer"));
        }
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
        let checksum = footer.u32()?;
        if start < 6 || start.checked_add(trailer_length) != Some(length - FOOTER_SIZE as u64) {
            return Err(invalid("invalid trailer extent"));
        }
        let mut bytes = engine::allocation::filled(to_usize(trailer_length)?, 0)?;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut bytes)?;
        if crc32fast::hash(&bytes) != checksum {
            return Err(invalid("trailer CRC mismatch"));
        }
        let mut trailer = Decoder::new(&bytes);
        expect_magic(&mut trailer, TRAILER_MAGIC)?;
        let sessions = decode_list(&mut trailer, SessionStart::decode)?;
        let _branches = decode_list(&mut trailer, BranchStart::decode)?;
        let _gaps = decode_list(&mut trailer, Gap::decode)?;
        let controls = decode_list(&mut trailer, |input| {
            ControlRecord::decode_version(input, format)
        })?;
        let _recovery = decode_list(&mut trailer, RecoveryNote::decode)?;
        let count = trailer.count(40)?;
        trailer.take(count * 40)?;
        trailer.finish()?;
        validate_partial_controls(&controls)?;
        for control in &controls {
            if !sessions
                .iter()
                .any(|session| session.id == control.session())
            {
                return Err(invalid("replay control refers to an unknown session"));
            }
        }
        Ok(controls
            .into_iter()
            .find(|control| matches!(control, ControlRecord::Partial { .. })))
    }

    pub fn from_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Self> {
        let segments: Vec<_> = segments.iter().map(Vec::as_slice).collect();
        Self::from_record_slices(campaign_id, &segments, &mut |_, _, _| Ok(()))
    }

    fn from_record_slices(
        campaign_id: Vec<u8>,
        segments: &[&[Record]],
        observe: &mut FrameObserver<'_>,
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
        self.append_record_slices(&segments, &mut |_, _, _| Ok(()))
    }

    fn append_record_slices(
        &self,
        segments: &[&[Record]],
        observe: &mut FrameObserver<'_>,
    ) -> io::Result<Self> {
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
        Self::from_record_slices(campaign_id, &records, &mut observe)?.add_recovery_notes(segments)
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
        self.append_record_slices(&records, &mut observe)?
            .add_recovery_notes(segments)
    }

    fn add_recovery_notes(mut self, segments: &[(u32, Recovery)]) -> io::Result<Self> {
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
        if crc32fast::hash(trailer_bytes) != trailer_crc {
            return Err(invalid("trailer CRC mismatch"));
        }
        let mut header = Decoder::new(&bytes[..trailer_start]);
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
        let header_end = trailer_start - header.remaining();
        let mut trailer = Decoder::new(trailer_bytes);
        expect_magic(&mut trailer, TRAILER_MAGIC)?;
        let sessions = decode_list(&mut trailer, SessionStart::decode)?;
        let branches = decode_list(&mut trailer, BranchStart::decode)?;
        let gaps = decode_list(&mut trailer, Gap::decode)?;
        let controls = decode_list(&mut trailer, |input| {
            ControlRecord::decode_version(input, format)
        })?;
        let recovery_notes = decode_list(&mut trailer, RecoveryNote::decode)?;
        let count = trailer.count(40)?;
        let mut index = Vec::with_capacity(count);
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
        trailer.finish()?;
        validate_metadata(
            &identity,
            &sessions,
            &branches,
            &gaps,
            &controls,
            &index,
            header_end,
            trailer_start,
            keyframe_interval,
        )?;
        let partial_control = controls
            .iter()
            .position(|control| matches!(control, ControlRecord::Partial { .. }));
        Ok(Self {
            bytes,
            format,
            campaign_id,
            identity,
            keyframe_interval,
            sessions,
            branches,
            gaps,
            controls,
            recovery_notes,
            index,
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
            .collect()
    }

    pub fn seek(&self, branch: BranchId, hour: Hour) -> io::Result<SeekResult> {
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
            return Ok(gap(self.gap_reason(branch, hour)));
        };
        let frames = match self.read_block(index) {
            Ok(frames) => frames,
            Err(error) => return Ok(gap(format!("unreadable block: {error}"))),
        };
        let mut decoder = FrameDecoder::new();
        for frame in frames {
            if frame.hour > hour {
                break;
            }
            let decoded = match decoder.advance(&frame) {
                Ok(decoded) => decoded,
                Err(error) => {
                    return Ok(gap(format!(
                        "invalid frame baseline at hour {}: {error}",
                        frame.hour
                    )));
                }
            };
            if let Err(error) = validate_campaign(decoded, &self.campaign_id) {
                return Ok(gap(error.to_string()));
            }
            if frame.hour == hour {
                return Ok(SeekResult::Frame(Box::new(decoded.clone())));
            }
        }
        Ok(gap(self.gap_reason(branch, hour)))
    }

    fn gap_reason(&self, branch: BranchId, hour: Hour) -> String {
        let reason = self
            .gaps
            .iter()
            .filter(|gap| gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour)
            .map(|gap| gap.reason.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        if reason.is_empty() {
            "hour was not recorded".into()
        } else {
            reason
        }
    }

    /// Decode one block for sequential playback, inspection, or history recovery.
    pub fn read_block(&self, index: &BlockIndex) -> io::Result<Vec<Frame>> {
        self.read_block_inner(index, None, None)
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
        self.read_block_inner(index, None, Some(parallel))
    }

    /// Preserve full wire validation but do not retain unrelated table payloads.
    pub fn read_block_selected(
        &self,
        index: &BlockIndex,
        tables: &[crate::rows::TableId],
    ) -> io::Result<Vec<Frame>> {
        self.read_block_inner(index, Some(tables), None)
    }

    fn read_block_inner(
        &self,
        index: &BlockIndex,
        selected: Option<&[crate::rows::TableId]>,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<Frame>> {
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
        let mut decompressor = zstd::stream::read::Decoder::with_buffer(compressed)?.single_frame();
        decompressor.window_log_max(26)?;
        let mut raw = Vec::with_capacity(to_usize(index.raw_length)?);
        decompressor
            .take(index.raw_length + 1)
            .read_to_end(&mut raw)?;
        if raw.len() as u64 != index.raw_length {
            return Err(invalid("block raw length mismatch"));
        }
        let mut decoder = Decoder::new(&raw);
        let count = decoder.count(4)?;
        if count == 0 {
            return Err(invalid("empty block"));
        }
        // The payload boundaries first, then the payloads decoded (in parallel
        // when asked), then every check in position order: the first error
        // reported is the one a frame-by-frame reader would have met first.
        let mut payloads = Vec::with_capacity(count);
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
        let decoded = decode_payloads(&payloads, selected, parallel, self.format);
        let mut frames = Vec::with_capacity(count);
        let mut last = None;
        for (position, frame) in decoded.into_iter().enumerate() {
            let frame = frame?;
            if frame.branch != index.branch
                || frame.keyframe != (position == 0)
                || last.is_some_and(|hour| frame.hour <= hour)
            {
                return Err(invalid(
                    "block frame branch, keyframe, or hour ordering mismatch",
                ));
            }
            last = Some(frame.hour);
            frames.push(frame);
        }
        if let Some(error) = boundary_error {
            return Err(error);
        }
        decoder.finish()?;
        if frames[0].hour != index.first_hour || last != Some(index.last_hour) {
            return Err(invalid("block index hour range mismatch"));
        }
        Ok(frames)
    }

    pub fn write_atomic(&self, destination: &Path) -> io::Result<()> {
        self.write_atomic_with(destination, |_| Ok(()))
    }

    /// Fault checkpoints happen before replacement; failure leaves the destination untouched.
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
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // The source is a closed, fully synced sibling file on the same volume.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)?;
    if let Some(parent) = destination.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
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

fn append_blocks(
    bytes: &mut Vec<u8>,
    index: &mut Vec<BlockIndex>,
    blocks: &[RawBlock],
) -> io::Result<()> {
    if blocks.is_empty() {
        return Ok(());
    }
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(blocks.len());
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                scope.spawn(move || -> io::Result<Vec<(usize, Vec<u8>)>> {
                    let mut output = Vec::new();
                    for position in (worker..blocks.len()).step_by(workers) {
                        let raw = blocks[position].raw();
                        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)?;
                        encoder.include_checksum(true)?;
                        encoder.write_all(&raw)?;
                        output.push((position, encoder.finish()?));
                    }
                    Ok(output)
                })
            })
            .collect();
        let mut output = Vec::with_capacity(blocks.len());
        for handle in handles {
            output.extend(
                handle
                    .join()
                    .map_err(|_| invalid("block compressor panicked"))??,
            );
        }
        Ok::<_, io::Error>(output)
    })?;
    let mut results = results;
    results.sort_unstable_by_key(|(position, _)| *position);
    for (position, compressed) in results {
        let block = &blocks[position];
        index.push(BlockIndex {
            branch: block.branch,
            first_hour: block.first_hour,
            last_hour: block.last_hour,
            offset: bytes.len() as u64,
            length: compressed.len() as u64,
            raw_length: (block.frames.len() + 4) as u64,
        });
        bytes.extend_from_slice(&compressed);
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

fn collect(
    segments: &[&[Record]],
    identity: &Identity,
    campaign_id: &[u8],
    existing: Option<&Replay>,
    interval: u32,
    observe: &mut FrameObserver<'_>,
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
    let mut block: Option<RawBlock> = None;
    for segment in segments {
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
                        pending.blocks.push(previous);
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
                        pending.blocks.push(previous);
                    }
                    current_session = Some(branch.session);
                    current_branch = Some(branch.id);
                    needs_keyframe = true;
                    frame_decoder = FrameDecoder::new();
                }
                Record::Frame(frame) => {
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
                            pending.blocks.push(previous);
                        }
                        block = Some(RawBlock::new(frame));
                    }
                    let block = block
                        .as_mut()
                        .ok_or_else(|| invalid("frame has no block keyframe"))?;
                    if i64::from(frame.hour) - i64::from(block.first_hour) >= i64::from(interval) {
                        return Err(invalid("block exceeds its keyframe hour interval"));
                    }
                    let decoded = frame_decoder.advance(frame)?;
                    validate_campaign(decoded, campaign_id)?;
                    let sizes = block.push(frame)?;
                    observe(frame, decoded, sizes)?;
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
                    pending.controls.push(control.clone());
                }
            }
        }
        if let Some(previous) = block.take() {
            pending.blocks.push(previous);
        }
    }
    // A complete CRC-valid record can survive a failed flush. Preserve its frame
    // and the reported write-loss diagnostic; seeking gives the recovered frame priority.
    Ok(pending)
}

#[allow(clippy::too_many_arguments)]
fn validate_metadata(
    identity: &Identity,
    sessions: &[SessionStart],
    branches: &[BranchStart],
    gaps: &[Gap],
    controls: &[ControlRecord],
    index: &[BlockIndex],
    header_end: usize,
    trailer_start: usize,
    interval: u32,
) -> io::Result<()> {
    if sessions.is_empty() {
        return Err(invalid("replay has no sessions"));
    }
    let mut session_ids = BTreeSet::new();
    for session in sessions {
        if !session_ids.insert(session.id) || !session.identity.same_content(identity) {
            return Err(invalid("duplicate session or conflicting content"));
        }
    }
    let mut branch_ids = BTreeMap::new();
    let mut sessions_with_start_branch = BTreeSet::new();
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
    for gap in gaps {
        gap.validate()?;
        if !branch_ids.contains_key(&gap.branch) {
            return Err(invalid("gap refers to an unknown branch"));
        }
    }
    for control in controls {
        if !session_ids.contains(&control.session()) {
            return Err(invalid("replay control refers to an unknown session"));
        }
    }
    validate_partial_controls(controls)?;
    let mut expected_offset = header_end as u64;
    let mut last_hours = BTreeMap::new();
    let mut closed_branches = BTreeSet::new();
    let mut closed_sessions = BTreeSet::new();
    let mut previous_branch = None;
    let mut previous_session = None;
    for block in index {
        let session = *branch_ids
            .get(&block.branch)
            .ok_or_else(|| invalid("block refers to an unknown branch"))?;
        if previous_branch != Some(block.branch) {
            if let Some(previous) = previous_branch {
                closed_branches.insert(previous);
            }
            if closed_branches.contains(&block.branch) {
                return Err(invalid("branch blocks are not contiguous"));
            }
        }
        if previous_session != Some(session) {
            if let Some(previous) = previous_session {
                closed_sessions.insert(previous);
            }
            if closed_sessions.contains(&session) {
                return Err(invalid("session blocks are not contiguous"));
            }
        }
        previous_branch = Some(block.branch);
        previous_session = Some(session);
        if block.first_hour > block.last_hour
            || i64::from(block.last_hour) - i64::from(block.first_hour) >= i64::from(interval)
            || last_hours
                .get(&block.branch)
                .is_some_and(|hour| block.first_hour <= *hour)
        {
            return Err(invalid("invalid or overlapping block hour ranges"));
        }
        if block.offset != expected_offset
            || block.length == 0
            || block.raw_length < 4
            || block.raw_length > MAX_BLOCK_RAW_BYTES as u64
        {
            return Err(invalid("invalid block extent or raw length"));
        }
        expected_offset = block
            .offset
            .checked_add(block.length)
            .ok_or_else(|| invalid("block offset overflow"))?;
        if expected_offset > trailer_start as u64 {
            return Err(invalid("block overlaps replay trailer"));
        }
        last_hours.insert(block.branch, block.last_hour);
    }
    if expected_offset != trailer_start as u64 {
        return Err(invalid("unindexed bytes before replay trailer"));
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
    selected: Option<&[crate::rows::TableId]>,
    parallel: Option<&Parallel<'_>>,
    format: FormatVersion,
) -> Vec<io::Result<Frame>> {
    let decode = |payload: &&[u8]| -> io::Result<Frame> {
        let mut frame_decoder = Decoder::new(payload);
        let frame = Frame::decode_version(&mut frame_decoder, selected, format)?;
        frame_decoder.finish()?;
        Ok(frame)
    };
    match parallel {
        Some(parallel) => parallel_map(payloads, parallel, decode),
        None => payloads.iter().map(decode).collect(),
    }
}

/// `work` over `items` in order, on up to `parallel.workers` threads.
fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    parallel: &Parallel<'_>,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let workers = parallel
        .workers
        .min(items.len().div_ceil(PARALLEL_FRAMES))
        .max(1);
    if workers <= 1 {
        return items.iter().map(&work).collect();
    }
    let run = items.len().div_ceil(workers);
    let work = &work;
    let on_worker = parallel.on_worker;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(run)
            .map(|chunk| {
                let spawned = std::thread::Builder::new().spawn_scoped(scope, move || {
                    on_worker();
                    chunk.iter().map(work).collect::<Vec<R>>()
                });
                (chunk, spawned.ok())
            })
            .collect();
        let mut out = Vec::with_capacity(items.len());
        for (chunk, handle) in handles {
            match handle {
                Some(handle) => match handle.join() {
                    Ok(results) => out.extend(results),
                    Err(panic) => std::panic::resume_unwind(panic),
                },
                // A worker the OS refused leaves its run to this thread.
                None => out.extend(chunk.iter().map(work)),
            }
        }
        out
    })
}

#[cfg(test)]
mod tests;
