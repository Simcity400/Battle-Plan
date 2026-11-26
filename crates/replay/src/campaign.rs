use replay_recording::{recording_identity::decode_identity, records::{CaptureStop, Cursor, Records, RecoveryNote}};
use replay_data::{BranchId, Hour, SessionId, codec::{invalid, invalid_static}, format::FormatVersion, identity::{Identity, IdentityCodec, SessionStart, Verification}, scope::CaptureMode};
use replay_frame::{branches::{BranchReason, BranchStart, BranchTracker, Gap}, frame::DecodedFrame};
use replay_recording::{columnar_frames::Recording, control::ControlRecord};
use replay_columnar::envelope::{self, Extent};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

mod changes;
pub mod commands;
pub mod collect;
#[cfg(any(test, feature = "recording"))]
pub mod encode_report;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
#[cfg(any(test, feature = "recording"))]
pub mod live;
pub mod read;
pub mod timelines;
pub mod writer;

pub use changes::{BlockChange, QuietHours, TableFields};
pub use collect::records_of;
pub use read::HourCursor;

pub const SESSION_FACTS_MAGIC: &[u8; 4] = b"H4SF";
pub const SESSION_FACTS_FILE: &str = "facts.bin";

pub fn encode_session_facts(identity: &Identity, pack: &[u8]) -> io::Result<Vec<u8>> {
    if pack.is_empty() {
        return Err(invalid_static("A session's game facts pack is empty"));
    }
    let mut encoded = Vec::new();
    identity.encode(&mut encoded)?;
    let mut out = Vec::with_capacity(8 + encoded.len() + pack.len());
    out.extend_from_slice(SESSION_FACTS_MAGIC);
    replay_data::codec::put_bytes(&mut out, &encoded)?;
    out.extend_from_slice(pack);
    Ok(out)
}

fn session_facts_identity(head: &[u8], total: u64) -> Option<u64> {
    let length = head.strip_prefix(SESSION_FACTS_MAGIC)?.get(..4)?;
    let length = u64::from(u32::from_le_bytes(length.try_into().ok()?));
    (8 + length < total).then_some(length)
}

pub fn session_facts_for<'a>(bytes: &'a [u8], identity: &Identity) -> Option<&'a [u8]> {
    let length = usize::try_from(session_facts_identity(bytes.get(..8)?, bytes.len() as u64)?).ok()?;
    let (encoded, pack) = bytes[8..].split_at(length);
    let recorded = decode_identity(encoded).ok()?;
    recorded.same_content(identity).then_some(pack)
}

pub fn session_facts_place(file: &mut std::fs::File, identity: &Identity) -> io::Result<Option<(u64, u64)>> {
    let total = file.metadata()?.len();
    let mut head = [0; 8];
    file.seek(SeekFrom::Start(0))?;
    if total < 8 || file.read_exact(&mut head).is_err() {
        return Ok(None);
    }
    let Some(length) = session_facts_identity(&head, total) else { return Ok(None) };
    let bytes = usize::try_from(length).map_err(|_| engine_allocation::oom())?;
    let mut encoded = engine_allocation::vec(bytes)?;
    encoded.resize(bytes, 0);
    file.read_exact(&mut encoded)?;
    let recorded = match decode_identity(&encoded) {
        Ok(recorded) => recorded,
        Err(_) => return Ok(None),
    };
    let offset = 8 + length;
    Ok(recorded.same_content(identity).then_some((offset, total - offset)))
}

const OLDER_MAGIC: &[u8; 4] = b"H4RP";

#[derive(Debug)]
struct OlderLayout(String);

impl std::fmt::Display for OlderLayout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for OlderLayout {}

pub fn is_older_layout(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<OlderLayout>())
}

fn refuse_older(header: &[u8]) -> io::Result<()> {
    if header.starts_with(OLDER_MAGIC) {
        let format = header
            .get(4..6)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
        let text = match format {
            Some(format) => format!(
                "This recording was saved by an older BattlePlan (row format {format}), with its \
                 native files in a folder beside it. This build reads one-file recordings only \
                 and has no upgrader for older ones."
            ),
            None => "This recording was saved by an older BattlePlan, with its native files in a \
                     folder beside it. This build reads one-file recordings only and has no \
                     upgrader for older ones."
                .to_owned(),
        };
        return Err(io::Error::new(io::ErrorKind::InvalidData, OlderLayout(text)));
    }
    if header.len() >= envelope::HEADER_BYTES && &header[..8] == envelope::MAGIC {
        let version = u32::from_le_bytes(header[8..12].try_into().expect("four bytes"));
        if version < envelope::VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                OlderLayout(format!(
                    "This recording is of format {version}; this build reads and writes format {} \
                     only and has no upgrader for it.",
                    envelope::VERSION
                )),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockIndex {
    pub branch: BranchId,
    pub first_hour: Hour,
    pub last_hour: Hour,
    pub run: u32,
    pub page: u32,
}

impl BlockIndex {
    pub fn key(&self) -> u64 {
        (u64::from(self.run) << 32) | u64::from(self.page)
    }
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

use replay_recording::columnar_frames::Open;
pub use replay_columnar::container::Revision;

type RowLifetimes = Arc<Vec<(u128, i64, i64)>>;
type LifetimeSlot = Arc<OnceLock<RowLifetimes>>;

pub struct Replay {
    recording: Recording,
    lifetimes: Mutex<HashMap<(u32, replay_data::rows::TableId), LifetimeSlot>>,
    path: Option<PathBuf>,
    pub format: FormatVersion,
    pub campaign_id: Vec<u8>,
    pub identity: Identity,
    pub capture_mode: CaptureMode,
    pub sessions: Vec<SessionStart>,
    pub branches: Vec<BranchStart>,
    pub gaps: Vec<Gap>,
    pub controls: Vec<ControlRecord>,
    pub recovery_notes: Vec<RecoveryNote>,
    pub cursors: Vec<Cursor>,
    pub stops: Vec<CaptureStop>,
    pub warnings: Vec<String>,
    pub damaged_parts: usize,
    pub damaged_range: Option<(Hour, Hour)>,
    pub index: Vec<BlockIndex>,
    pub fact_packs: Vec<Extent>,
    segments: Vec<u32>,
    revision: Revision,
    saved: Option<Revision>,
    partial_control: Option<usize>,
    physical: Option<Box<timelines::Physical>>,
    pool: Option<std::sync::Arc<replay_recording::columnar_frames::Pool>>,
}

impl Replay {
    pub fn open(bytes: Vec<u8>) -> io::Result<Self> {
        Self::open_as(bytes, Open::Saved)
    }

    pub fn open_as(bytes: Vec<u8>, open: Open) -> io::Result<Self> {
        refuse_older(&bytes[..bytes.len().min(envelope::HEADER_BYTES)])?;
        Self::read(Recording::open_as(bytes, open)?, None)
    }

    pub fn open_path(path: &Path) -> io::Result<Self> {
        Self::open_path_as(path, Open::Saved)
    }

    pub fn open_newest(path: &Path) -> io::Result<Self> {
        Self::open_path_as(path, Open::Newest)
    }

    pub fn open_path_as(path: &Path, open: Open) -> io::Result<Self> {
        let mut header = [0; envelope::HEADER_BYTES];
        let mut file = File::open(path)?;
        let held = read_prefix(&mut file, &mut header)?;
        refuse_older(&header[..held])?;
        Self::read(Recording::open_path_as(path, open)?, Some(path.to_path_buf()))
    }

    fn read(recording: Recording, path: Option<PathBuf>) -> io::Result<Self> {
        let root = recording.root();
        let mut records = Records::default();
        for extent in recording.records_extents() {
            records.decode_into(&recording.extent_bytes(extent)?)?;
        }
        let identity = decode_identity(&root.identity.content)?;
        let capture_mode = CaptureMode::from_byte(root.identity.capture_mode)?;
        let campaign_id = root.identity.campaign.clone();
        if campaign_id.is_empty() {
            return Err(invalid_static("empty campaign identity"));
        }
        let revision = Revision {
            number: root.revision,
            length: recording.len(),
            root: root_digest(&recording)?,
        };
        let saved = match root.saved {
            replay_columnar::container::Saved::Yes => Some(revision),
            replay_columnar::container::Saved::No(before) => before,
        };
        let fact_packs = engine_allocation::copy_vec(recording.facts_extents())?;
        let Records {
            sessions,
            branches,
            mut gaps,
            mut controls,
            notes,
            cursors,
            stops,
        } = records;
        if sessions.iter().any(|session| session.capture_mode != capture_mode) {
            return Err(invalid_static("replay sessions name different capture modes"));
        }
        let mut warnings = Vec::new();
        let mut damaged_parts = 0;
        validate_records(
            &identity,
            &campaign_id,
            &sessions,
            &branches,
            &mut gaps,
            &mut controls,
            &mut warnings,
            &mut damaged_parts,
        )?;
        let mut index = Vec::new();
        let mut segments = Vec::new();
        let mut damaged_range: Option<(Hour, Hour)> = None;
        let known: HashSet<BranchId> = branches.iter().map(|branch| branch.id).collect();
        let held_segments = recording.segments();
        for damage in recording.damage() {
            engine_allocation::reserve(&mut warnings, 1)?;
            warnings.push(replay_data::codec::try_format(format_args!("{damage}"))?);
            damaged_parts += 1;
        }
        for run in 0..recording.run_count() {
            let Ok(held) = recording.run(run) else {
                continue;
            };
            if !known.contains(&held.branch) {
                engine_allocation::reserve(&mut warnings, 1)?;
                warnings.push(replay_data::codec::try_format(format_args!(
                    "Run {run} records branch {}, which no branch start names",
                    held.branch
                ))?);
                damaged_parts += 1;
                let last = i64::from(held.first_hour) + i64::from(held.ordinals) - 1;
                let last = Hour::try_from(last).map_err(|_| invalid_static("a run passes 2^31 hours"))?;
                let (first, end) = damaged_range.unwrap_or((held.first_hour, last));
                damaged_range = Some((first.min(held.first_hour), end.max(last)));
                continue;
            }
            for at in 0..held.pages() {
                let (first, hours) = held.block(at);
                let first_hour = i64::from(held.first_hour) + i64::from(first);
                let last_hour = first_hour + i64::from(hours) - 1;
                let first_hour =
                    Hour::try_from(first_hour).map_err(|_| invalid_static("a page passes 2^31 hours"))?;
                let last_hour =
                    Hour::try_from(last_hour).map_err(|_| invalid_static("a page passes 2^31 hours"))?;
                let segment = held_segments
                    .iter()
                    .position(|segment| {
                        segment.run == run && segment.first <= first && first <= segment.last
                    })
                    .ok_or_else(|| invalid_static("a page lies in no segment of its run"))?;
                engine_allocation::reserve(&mut index, 1)?;
                index.push(BlockIndex {
                    branch: held.branch,
                    first_hour,
                    last_hour,
                    run,
                    page: u32::try_from(at).map_err(|_| invalid_static("a run passes 2^32 pages"))?,
                });
                engine_allocation::reserve(&mut segments, 1)?;
                segments.push(segment as u32);
            }
        }
        let partial_control = controls
            .iter()
            .position(|control| matches!(control, ControlRecord::Partial { .. }));
        let format = FormatVersion::read(recording.format())?;
        Ok(Self {
            recording,
            lifetimes: Mutex::default(),
            path,
            format,
            campaign_id,
            identity,
            capture_mode,
            sessions,
            branches,
            gaps,
            controls,
            recovery_notes: notes,
            cursors,
            stops,
            warnings,
            damaged_parts,
            damaged_range,
            index,
            fact_packs,
            segments,
            revision,
            saved,
            partial_control,
            physical: None,
            pool: None,
        })
    }

    pub fn set_pool(&mut self, pool: Option<std::sync::Arc<replay_recording::columnar_frames::Pool>>) {
        self.pool = pool;
    }

    pub fn recording(&self) -> &Recording {
        &self.recording
    }

    pub fn row_lifetimes<G>(&self, run: u32, table: replay_data::rows::TableId, on_read: impl FnOnce() -> G) -> io::Result<RowLifetimes> {
        let slot = Arc::clone(self.lifetimes.lock().unwrap_or_else(PoisonError::into_inner).entry((run, table)).or_default());
        if let Some(rows) = slot.get() {
            return Ok(Arc::clone(rows));
        }
        let _read = on_read();
        let mut rows = replay_lanes::lanes::lifetimes(&self.recording, run, table)?;
        rows.shrink_to_fit();
        Ok(Arc::clone(slot.get_or_init(|| Arc::new(rows))))
    }

    pub fn forget_row_lifetimes(&self) {
        self.lifetimes.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    pub fn cursor(&self, source: &str, capture: u32) -> Option<&Cursor> {
        self.cursors
            .iter()
            .rev()
            .find(|cursor| cursor.source == source && cursor.capture == capture)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_file_backed(&self) -> bool {
        self.path.is_some()
    }

    pub fn bytes(&self) -> &[u8] {
        self.recording
            .memory()
            .expect("a recording read from its file holds no bytes in memory")
    }

    pub(crate) fn segment_of(&self, block: &BlockIndex) -> io::Result<usize> {
        let (pages, segments) = match &self.physical {
            Some(physical) => (&physical.index, &physical.segments),
            None => (&self.index, &self.segments),
        };
        let position = pages
            .binary_search_by_key(&(block.run, block.page), |held| (held.run, held.page))
            .map_err(|_| invalid_static("the block is none of the recording's"))?;
        Ok(segments[position] as usize)
    }

    pub fn same_segment(&self, a: &BlockIndex, b: &BlockIndex) -> bool {
        matches!((self.segment_of(a), self.segment_of(b)), (Ok(a), Ok(b)) if a == b)
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn saved_revision(&self) -> Option<Revision> {
        self.saved
    }

    pub fn unsaved(&self) -> bool {
        self.saved != Some(self.revision)
    }

    pub fn same_revision(&self, other: &Self) -> bool {
        self.campaign_id == other.campaign_id && self.revision == other.revision
    }

    pub fn contains_revision(&self, revision: Revision) -> io::Result<bool> {
        if revision == self.revision {
            return Ok(true);
        }
        match &self.path {
            Some(path) if revision.number < self.revision.number => writer::holds_revision(path, revision),
            _ => Ok(false),
        }
    }

    pub fn saved_length(&self) -> u64 {
        self.revision.length
    }

    pub fn partial(&self) -> Option<&ControlRecord> {
        self.partial_control
            .and_then(|index| self.controls.get(index))
    }

    pub fn is_partial(&self) -> bool {
        self.partial().is_some()
    }

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

    pub fn needed_recovery(&self) -> bool {
        !self.warnings.is_empty()
    }

    pub fn require_undamaged(&self) -> io::Result<()> {
        if self.needed_recovery() {
            return Err(invalid_static(
                "This recording is damaged, so it can't be added to; start a new file.",
            ));
        }
        Ok(())
    }

    pub fn content_verified(&self) -> bool {
        matches!(self.identity.verification, Verification::Verified { .. })
            && self.sessions.iter().all(|session| {
                matches!(session.identity.verification, Verification::Verified { .. })
            })
    }

    pub fn compatibility_gaps(&self) -> Vec<String> {
        self.recording.withheld().iter().map(|(_, reason)| reason.clone()).collect()
    }

    pub fn facts(&self, at: usize) -> io::Result<Vec<u8>> {
        let extent = self
            .fact_packs
            .get(at)
            .ok_or_else(|| invalid_static("the recording holds no such game facts pack"))?;
        self.recording.stored_bytes(extent)
    }

    pub fn verify_facts(&self, at: usize) -> io::Result<()> {
        self.facts(at).map(drop)
    }

    pub fn read_facts(&self, at: usize, offset: u64, out: &mut [u8]) -> io::Result<()> {
        let extent = self
            .fact_packs
            .get(at)
            .ok_or_else(|| invalid_static("the recording holds no such game facts pack"))?;
        self.recording.stored_at(extent, offset, out)
    }

    pub fn newest_facts(&self) -> io::Result<Option<Vec<u8>>> {
        match self.fact_packs.len() {
            0 => Ok(None),
            count => self.facts(count - 1).map(Some),
        }
    }

    pub fn gap_reason(&self, branch: BranchId, hour: Hour) -> io::Result<String> {
        let mut reason = String::new();
        let stopped = self.stops.iter().filter(|stop| {
            stop.branch == branch && stop.last_hour.is_none_or(|last| last < hour)
        });
        let gaps = self
            .gaps
            .iter()
            .filter(|gap| gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour)
            .map(|gap| gap.reason.as_str());
        for text in gaps.chain(stopped.map(|stop| stop.reason.as_str())) {
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

    pub fn recorded_hours(&self, branch: BranchId) -> io::Result<Vec<Hour>> {
        let mut hours = Vec::new();
        for block in self.index.iter().filter(|block| block.branch == branch) {
            let count = usize::try_from(i64::from(block.last_hour) - i64::from(block.first_hour) + 1)
                .map_err(|_| invalid_static("a block's hours do not rise"))?;
            engine_allocation::reserve(&mut hours, count)?;
            hours.extend(block.first_hour..=block.last_hour);
        }
        Ok(hours)
    }

    pub fn hour_span(&self) -> Option<(Hour, Hour)> {
        let first = self.index.iter().map(|block| block.first_hour).min()?;
        let last = self.index.iter().map(|block| block.last_hour).max()?;
        Some((first, last))
    }

    pub fn into_history(self) -> io::Result<CampaignHistory> {
        if self.physical.is_some() {
            return Err(invalid_static("a recording read as timelines holds no recorder history"));
        }
        self.require_undamaged()?;
        let mut signals = vec![None; self.index.len()];
        let blocks: Vec<_> = self.index.iter().collect();
        let mut at = 0;
        self.visit_blocks(
            &blocks,
            &[replay_data::rows::TableId::Clock],
            |block, _, frame| {
                while blocks[at] != block {
                    at += 1;
                }
                signals[at] = clock_signal(frame);
                Ok(true)
            },
            || Ok(()),
        )?;
        let spans: Vec<_> = self
            .index
            .iter()
            .zip(signals)
            .map(|(block, signal)| (block.branch, block.first_hour, block.last_hour, signal))
            .collect();
        build_history(
            self.campaign_id,
            self.identity,
            self.sessions,
            &self.branches,
            &self.gaps,
            &self.controls,
            &spans,
        )
    }

    pub fn read_partial(path: &Path) -> io::Result<Option<ControlRecord>> {
        Ok(Summary::read(path)?.partial)
    }

    pub fn read_campaign_uuid(path: &Path) -> io::Result<Option<Vec<u8>>> {
        let id = Self::read_campaign_id(path)?;
        Ok(replay_data::identity::is_campaign_uuid(&id).then_some(id))
    }

    pub fn read_campaign_id(path: &Path) -> io::Result<Vec<u8>> {
        Ok(Summary::read_as(path, Open::Newest)?.campaign_id)
    }

    pub fn read_hour_span(path: &Path) -> io::Result<Option<(Hour, Hour)>> {
        Ok(Summary::read(path)?.span)
    }

    pub fn read_capture_mode(path: &Path) -> io::Result<CaptureMode> {
        Ok(Summary::read_as(path, Open::Newest)?.capture_mode)
    }

    pub fn read_session_ids(path: &Path) -> io::Result<Vec<SessionId>> {
        Ok(Summary::read(path)?
            .sessions
            .iter()
            .map(|session| session.id)
            .collect())
    }
}

pub struct Summary {
    pub campaign_id: Vec<u8>,
    pub capture_mode: CaptureMode,
    pub sessions: Vec<SessionStart>,
    pub partial: Option<ControlRecord>,
    pub span: Option<(Hour, Hour)>,
    pub revision: Revision,
}

impl Summary {
    pub fn read(path: &Path) -> io::Result<Self> {
        Self::read_as(path, Open::Saved)
    }

    pub fn read_as(path: &Path, open: Open) -> io::Result<Self> {
        Self::read_from_as(File::open(path)?, open)
    }

    pub fn read_from(file: File) -> io::Result<Self> {
        Self::read_from_as(file, Open::Saved)
    }

    pub fn read_from_as(mut file: File, open: Open) -> io::Result<Self> {
        let mut header = [0; envelope::HEADER_BYTES];
        file.seek(SeekFrom::Start(0))?;
        let held = read_prefix(&mut file, &mut header)?;
        refuse_older(&header[..held])?;
        let recording = Recording::open_file(file, open)?;
        let root = recording.root().clone();
        let records = recording.records()?;
        let mut span: Option<(Hour, Hour)> = None;
        for run in 0..recording.run_count() {
            let Ok(held) = recording.run(run) else { continue };
            if held.ordinals == 0 {
                continue;
            }
            let (first, last) = (held.first_hour, held.last_hour());
            span = Some(span.map_or((first, last), |(low, high)| (low.min(first), high.max(last))));
        }
        let capture_mode = CaptureMode::from_byte(root.identity.capture_mode)?;
        let readable = validate_partial_controls(&records.controls).is_ok()
            && records
                .controls
                .iter()
                .all(|control| records.sessions.iter().any(|session| session.id == control.session()));
        let partial = if readable {
            records
                .controls
                .iter()
                .find(|control| matches!(control, ControlRecord::Partial { .. }))
                .cloned()
        } else {
            None
        };
        Ok(Self {
            revision: recording.revision(),
            campaign_id: root.identity.campaign,
            capture_mode,
            sessions: records.sessions,
            partial,
            span,
        })
    }
}

fn read_prefix(input: &mut impl Read, header: &mut [u8]) -> io::Result<usize> {
    let mut held = 0;
    while held < header.len() {
        match input.read(&mut header[held..])? {
            0 => break,
            count => held += count,
        }
    }
    Ok(held)
}


fn root_digest(recording: &Recording) -> io::Result<[u8; 16]> {
    recording.root_footer_digest()
}

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
    pub hours: Vec<Hour>,
    pub last_missing_hour: Option<Hour>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionTail {
    pub branch: BranchId,
    pub hour: Hour,
    pub load_signal: Option<i64>,
    pub recorded: bool,
}

impl SessionTail {
    pub fn resume(
        self,
        tracker: &mut BranchTracker,
        session: SessionId,
        last_missing_hour: Option<Hour>,
    ) -> io::Result<()> {
        if let Some(hour) = last_missing_hour.filter(|hour| *hour >= self.hour) {
            tracker.resume_observation(session, self.branch, hour, self.load_signal)
        } else if self.recorded {
            tracker.resume_session(session, self.branch, self.hour, self.load_signal)
        } else {
            tracker.resume_commands(session, self.branch, self.hour, self.load_signal)
        }
    }
}

impl CampaignHistory {
    pub fn session_tail(&self, session: SessionId) -> Option<SessionTail> {
        self.tails
            .iter()
            .find(|(id, _)| *id == session)
            .map(|(_, tail)| *tail)
    }

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
                invalid_static(
                    "existing process session has no witnessed frame or command phase to resume",
                )
            })?;
            let last_missing_hour = self
                .branches
                .iter()
                .find(|branch| branch.start.id == tail.branch)
                .and_then(|branch| branch.last_missing_hour);
            tail.resume(&mut tracker, session.id, last_missing_hour)?;
        } else {
            tracker.begin_session(session.id)?;
        }
        Ok(tracker)
    }
}

fn clock_signal(frame: &DecodedFrame) -> Option<i64> {
    match &*frame.tables[replay_data::rows::TableId::Clock.index()] {
        replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Clock(rows)) => {
            rows.first().map(|clock| clock.play_session_started_at)
        }
        _ => None,
    }
}

impl CampaignHistory {
    pub fn from_spool(
        campaign_id: &[u8],
        segments: &[(u32, replay_capture_spool::spool::Recovery<replay_capture_spool::spool::Recovered>)],
    ) -> io::Result<Self> {
        use replay_capture_spool::spool::Recovered;
        let clock_decoder = || replay_frame::frame::FrameDecoder::for_tables(&[replay_data::rows::TableId::Clock]);
        let mut sorted: Vec<_> = segments.iter().collect();
        sorted.sort_unstable_by_key(|(number, _)| *number);
        let mut sessions: Vec<SessionStart> = Vec::new();
        let mut branches: Vec<BranchStart> = Vec::new();
        let mut gaps = Vec::new();
        let mut controls = Vec::new();
        let mut spans = Vec::new();
        for (_, recovery) in sorted {
            let mut clock = clock_decoder();
            for record in &recovery.records {
                match record {
                    Recovered::SessionStart(session) => {
                        if !sessions.contains(session) {
                            sessions.push(session.clone());
                        }
                        clock = clock_decoder();
                    }
                    Recovered::BranchStart(branch) => {
                        if !branches.contains(branch) {
                            branches.push(branch.clone());
                        }
                        clock = clock_decoder();
                    }
                    Recovered::Gap(gap) => gaps.push(gap.try_clone()?),
                    Recovered::Control(control) => controls.push(control.clone()),
                    Recovered::Frame(metadata) => {
                        let signal = clock_signal(clock.advance(&metadata.frame()?)?);
                        spans.push((metadata.branch, metadata.hour, metadata.hour, signal));
                    }
                }
            }
        }
        let identity = sessions
            .first()
            .map(|session| session.identity.clone())
            .ok_or_else(|| invalid_static("the spool names no session"))?;
        build_history(
            campaign_id.to_vec(),
            identity,
            sessions,
            &branches,
            &gaps,
            &controls,
            &spans,
        )
    }
}

fn build_history(
    campaign_id: Vec<u8>,
    identity: Identity,
    sessions: Vec<SessionStart>,
    starts: &[BranchStart],
    gaps: &[Gap],
    controls: &[ControlRecord],
    spans: &[(BranchId, Hour, Hour, Option<i64>)],
) -> io::Result<CampaignHistory> {
    let positions: BTreeMap<_, _> = starts
        .iter()
        .enumerate()
        .map(|(index, branch)| (branch.id, index))
        .collect();
    let mut branches: Vec<_> = starts
        .iter()
        .map(|start| RecordedBranchHistory {
            start: start.clone(),
            hours: Vec::new(),
            last_missing_hour: None,
        })
        .collect();
    for gap in gaps {
        let index = *positions
            .get(&gap.branch)
            .ok_or_else(|| invalid_static("gap refers to an unknown branch"))?;
        let last = &mut branches[index].last_missing_hour;
        *last = Some(last.map_or(gap.last_hour, |hour| hour.max(gap.last_hour)));
    }
    let mut tails: Vec<(SessionId, SessionTail)> = Vec::new();
    for &(branch, first, last, load_signal) in spans {
        let index = *positions
            .get(&branch)
            .ok_or_else(|| invalid_static("block refers to an unknown branch"))?;
        let history = &mut branches[index];
        if history.hours.last().is_some_and(|hour| *hour >= first) {
            return Err(invalid_static(
                "recorded history hours do not increase within branch",
            ));
        }
        let count = usize::try_from(i64::from(last) - i64::from(first) + 1)
            .map_err(|_| invalid_static("a block's hours do not rise"))?;
        engine_allocation::reserve(&mut history.hours, count)?;
        history.hours.extend(first..=last);
        let tail = SessionTail {
            branch,
            hour: last,
            load_signal,
            recorded: true,
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
    for control in controls.iter().rev() {
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
    let mut observed: BTreeMap<SessionId, u64> = BTreeMap::new();
    for control in controls {
        if let ControlRecord::ClockWitness {
            session,
            qpc,
            hour,
            load_signal,
        } = control
            && tails.iter().any(|(id, tail)| {
                id == session && tail.hour == *hour && tail.load_signal == *load_signal
            })
        {
            observed
                .entry(*session)
                .and_modify(|time| *time = (*time).max(*qpc))
                .or_insert(*qpc);
        }
    }
    for control in controls {
        let ControlRecord::PlayerCommands { session, packet } = control else {
            continue;
        };
        let Some(phase) = &packet.phase else {
            continue;
        };
        let previous = tails.iter().position(|(id, _)| id == session);
        if previous.is_some() && !observed.contains_key(session) {
            return Err(invalid_static(
                "frame and command phase order has no clock witness",
            ));
        }
        if observed.get(session).is_some_and(|time| *time >= phase.qpc) {
            continue;
        }
        let tail = SessionTail {
            branch: packet.branch,
            hour: phase.hour,
            load_signal: phase.load_marker(),
            recorded: false,
        };
        if let Some(index) = previous {
            tails[index].1 = tail;
        } else {
            engine_allocation::reserve(&mut tails, 1)?;
            tails.push((*session, tail));
        }
        observed.insert(*session, phase.qpc);
    }
    Ok(CampaignHistory {
        campaign_id,
        identity,
        sessions,
        branches,
        tails,
    })
}

fn validate_records(
    identity: &Identity,
    campaign_id: &[u8],
    sessions: &[SessionStart],
    branches: &[BranchStart],
    gaps: &mut Vec<Gap>,
    controls: &mut Vec<ControlRecord>,
    warnings: &mut Vec<String>,
    damaged_parts: &mut usize,
) -> io::Result<()> {
    if sessions.is_empty() {
        return Err(invalid_static("replay has no sessions"));
    }
    let mut session_ids = HashSet::new();
    engine_allocation::reserve_set(&mut session_ids, sessions.len())?;
    for session in sessions {
        if !session_ids.insert(session.id) || !session.identity.same_content(identity) {
            return Err(invalid_static("duplicate session or conflicting content"));
        }
    }
    let mut branch_ids = HashMap::new();
    engine_allocation::reserve_map(&mut branch_ids, branches.len())?;
    let mut sessions_with_start_branch = HashSet::new();
    engine_allocation::reserve_set(&mut sessions_with_start_branch, sessions.len())?;
    for branch in branches {
        branch.validate()?;
        if !session_ids.contains(&branch.session) || branch_ids.contains_key(&branch.id) {
            return Err(invalid_static("unknown session or duplicate branch"));
        }
        if branch.reason == BranchReason::SessionStart
            && !sessions_with_start_branch.insert(branch.session)
        {
            return Err(invalid_static("session has multiple process-start branches"));
        }
        branch_ids.insert(branch.id, branch.session);
    }
    if gaps.iter().any(|gap| !branch_ids.contains_key(&gap.branch)) {
        engine_allocation::reserve(warnings, 1)?;
        warnings.push("Unreadable gap metadata: gap refers to an unknown branch".into());
        *damaged_parts += 1;
        gaps.retain(|gap| branch_ids.contains_key(&gap.branch));
    }
    let checked = (|| -> io::Result<()> {
        for control in controls.iter() {
            if !session_ids.contains(&control.session()) {
                return Err(invalid_static("replay control refers to an unknown session"));
            }
            if let ControlRecord::PlayerCommands { session, packet } = control {
                if branch_ids.get(&packet.branch) != Some(session) {
                    return Err(invalid_static("player commands refer to another session's branch"));
                }
                if let Some(phase) = &packet.phase {
                    phase.validate()?;
                    if !phase.campaign.eq_ignore_ascii_case(campaign_id)
                        || phase.start.id != packet.branch
                        || phase.start.session != *session
                        || branches.iter().find(|branch| branch.id == packet.branch)
                            != Some(&phase.start)
                    {
                        return Err(invalid_static(
                            "player command phase conflicts with recording metadata",
                        ));
                    }
                }
            }
            if let ControlRecord::EventLoss {
                session, branch, ..
            } = control
            {
                if branch_ids.get(branch) != Some(session) {
                    return Err(invalid_static("event loss refers to another session's branch"));
                }
                if controls.iter().filter(|other| *other == control).count() > 1 {
                    return Err(invalid_static("event loss is recorded twice"));
                }
            }
        }
        validate_partial_controls(controls)
    })();
    if let Err(error) = checked {
        engine_allocation::reserve(warnings, 1)?;
        warnings.push(replay_data::codec::try_format(format_args!(
            "Unreadable controls: {error}"
        ))?);
        controls.clear();
        *damaged_parts += 1;
    }
    Ok(())
}

pub(crate) fn validate_partial_controls(controls: &[ControlRecord]) -> io::Result<()> {
    for (position, control) in controls.iter().enumerate() {
        if let ControlRecord::Partial { session, reason, .. } = control
            && (reason.trim().is_empty()
                || controls[..position].iter().any(|previous| {
                    matches!(previous, ControlRecord::Partial { session: id, .. } if id == session)
                }))
        {
            return Err(invalid("duplicate or invalid partial capture"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod spool_tests;
