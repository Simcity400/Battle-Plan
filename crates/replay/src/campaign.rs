//! A campaign's recording: one `<Campaign name>.battleplan` file holding every session of it
//! ([`replay_columnar::container`]). Its world log's runs hold every
//! table's hours as columns ([`crate::columnar_frames`]); its records section holds its sessions,
//! branches, gaps, control records, recovery notes, where each run came from and where a
//! capture's writer stopped ([`records`]); its facts section holds the game facts it carries.
//!
//! [`Replay`] opens one held on disk or in memory, reads its records and names each run's pages
//! as [`BlockIndex`] entries; its hours are read through [`read`]. It opens the newest saved
//! revision: hours appended since, which wait to be saved or discarded, are opened only on
//! purpose ([`Open::Newest`]), by the recorder and by whoever goes on from its last hour.
//! [`writer`] creates one and adds to it.

use crate::{
    BranchId, Hour, SessionId,
    branches::{BranchReason, BranchStart, BranchTracker, Gap},
    codec::{Decoder, invalid, invalid_static},
    columnar_frames::Recording,
    format::FormatVersion,
    frame::DecodedFrame,
    identity::{Identity, IdentityCodec, SessionStart, Verification},
    scope::CaptureMode,
    spool::ControlRecord,
};
use replay_columnar::envelope::{self, Extent};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

mod changes;
pub mod collect;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
#[cfg(any(test, feature = "recording"))]
pub mod live;
pub mod read;
pub mod records;
pub mod timelines;
pub mod writer;

pub use changes::{BlockChange, QuietHours, TableFields};
pub use collect::records_of;
pub use read::{HourCursor, Parallel};
pub use records::{CaptureStop, Cursor, Records, RecoveryNote};

/// Starts a session's facts file ([`encode_session_facts`]).
pub const SESSION_FACTS_MAGIC: &[u8; 4] = b"H4SF";
/// A session folder's facts file, which its recorder writes once the facts are gathered.
pub const SESSION_FACTS_FILE: &str = "facts.bin";

/// A session's game facts as its recorder gathered them, for the recordings its spools become:
/// the identity they are the facts of, then the pack.
pub fn encode_session_facts(identity: &Identity, pack: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoded = Vec::new();
    identity.encode(&mut encoded)?;
    let mut out = Vec::with_capacity(8 + encoded.len() + pack.len());
    out.extend_from_slice(SESSION_FACTS_MAGIC);
    crate::codec::put_bytes(&mut out, &encoded)?;
    out.extend_from_slice(pack);
    Ok(out)
}

/// The pack of a session's facts file, when they are `identity`'s facts.
pub fn session_facts_for<'a>(bytes: &'a [u8], identity: &Identity) -> Option<&'a [u8]> {
    let mut input = Decoder::new(bytes.strip_prefix(SESSION_FACTS_MAGIC)?);
    let encoded = input.bytes().ok()?;
    let mut decoder = Decoder::new(&encoded);
    let recorded = Identity::decode(&mut decoder).ok()?;
    let pack = &bytes[bytes.len() - input.remaining()..];
    (recorded.same_content(identity) && !pack.is_empty()).then_some(pack)
}

/// Where the pack of the session facts file `file` lies, its offset and length, when they are
/// `identity`'s facts: only the file's head is read, never the pack.
pub fn session_facts_place(file: &mut std::fs::File, identity: &Identity) -> io::Result<Option<(u64, u64)>> {
    use std::io::{Read, Seek, SeekFrom};
    let total = file.metadata()?.len();
    let mut head = [0; 8];
    file.seek(SeekFrom::Start(0))?;
    if total < 8 || file.read_exact(&mut head).is_err() || &head[..4] != SESSION_FACTS_MAGIC {
        return Ok(None);
    }
    let length = u64::from(u32::from_le_bytes(head[4..].try_into().expect("four bytes")));
    let offset = 8 + length;
    // An identity is a few hundred bytes; a longer head is not a facts file.
    if length > 1 << 20 || offset >= total {
        return Ok(None);
    }
    let mut encoded = vec![0; length as usize];
    file.read_exact(&mut encoded)?;
    let recorded = match Identity::decode(&mut Decoder::new(&encoded)) {
        Ok(recorded) => recorded,
        Err(_) => return Ok(None),
    };
    Ok(recorded.same_content(identity).then_some((offset, total - offset)))
}

/// The magic an older recording's file starts with: rows beside native files, which this build
/// refuses by name.
const OLDER_MAGIC: &[u8; 4] = b"H4RP";

/// An older layout, refused whole by name: readers show its text as the reason.
#[derive(Debug)]
struct OlderLayout(String);

impl std::fmt::Display for OlderLayout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for OlderLayout {}

/// Whether `error` refuses a recording of an older layout ([`refuse_older`]).
pub fn is_older_layout(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<OlderLayout>())
}

/// Refuses a file starting with `header` when it is not a recording this build reads, naming
/// what it is.
fn refuse_older(header: &[u8]) -> io::Result<()> {
    if header.starts_with(OLDER_MAGIC) {
        let format = header
            .get(4..6)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
        let text = match format {
            Some(format) => format!(
                "This recording was saved by an older Battle Plan (row format {format}), with its \
                 native files in a folder beside it. This build reads one-file recordings only \
                 and has no upgrader for older ones."
            ),
            None => "This recording was saved by an older Battle Plan, with its native files in a \
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

/// One page of one run: a block of up to [`envelope::UNIT_ORDINALS`] consecutive hours of one
/// branch, the unit hours are read by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockIndex {
    pub branch: BranchId,
    pub first_hour: Hour,
    pub last_hour: Hour,
    /// The run and its page.
    pub run: u32,
    pub page: u32,
}

impl BlockIndex {
    /// Its place in the recording, run and page, as one number: what names the block among
    /// every revision's blocks.
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

pub use crate::columnar_frames::{NotSaved, Open, is_not_saved};
pub use replay_columnar::container::Revision;

/// A campaign's recording, opened: its records, its runs' pages and its revision.
pub struct Replay {
    recording: Recording,
    path: Option<PathBuf>,
    /// The row format its hours are recorded in: this build's or an earlier proven one.
    pub format: FormatVersion,
    pub campaign_id: Vec<u8>,
    pub identity: Identity,
    /// Every session's capture mode: one per file.
    pub capture_mode: CaptureMode,
    pub sessions: Vec<SessionStart>,
    pub branches: Vec<BranchStart>,
    pub gaps: Vec<Gap>,
    pub controls: Vec<ControlRecord>,
    pub recovery_notes: Vec<RecoveryNote>,
    /// How much of each capture's records it holds, as each revision noted it: a capture's
    /// last entry stands ([`Self::cursor`]).
    pub cursors: Vec<Cursor>,
    pub stops: Vec<CaptureStop>,
    /// Read-time damage diagnostics; never written into the recording.
    pub warnings: Vec<String>,
    pub damaged_parts: usize,
    pub damaged_range: Option<(Hour, Hour)>,
    /// Every readable run's pages, run by run.
    pub index: Vec<BlockIndex>,
    /// The game facts packs it carries, oldest first.
    pub fact_packs: Vec<Extent>,
    /// Each entry of `index`'s segment in the recording's.
    segments: Vec<u32>,
    revision: Revision,
    /// The newest saved revision: its own, or the one its root names.
    saved: Option<Revision>,
    partial_control: Option<usize>,
    /// Its segments, when read as timelines ([`Self::into_timelines`]).
    physical: Option<Box<timelines::Physical>>,
    /// The threads its frames are built on; rayon's own without one ([`Self::set_pool`]).
    pool: Option<std::sync::Arc<crate::columnar_frames::Pool>>,
}

impl Replay {
    /// The recording held in memory, at its newest saved revision.
    pub fn open(bytes: Vec<u8>) -> io::Result<Self> {
        Self::open_as(bytes, Open::Saved)
    }

    /// The recording held in memory, at the revision `open` names.
    pub fn open_as(bytes: Vec<u8>, open: Open) -> io::Result<Self> {
        refuse_older(&bytes[..bytes.len().min(envelope::HEADER_BYTES)])?;
        Self::read(Recording::open_as(bytes, open)?, None)
    }

    /// The recording at `path` at its newest saved revision, read from the file as it is
    /// needed. One with no saved revision is refused ([`is_not_saved`]).
    pub fn open_path(path: &Path) -> io::Result<Self> {
        Self::open_path_as(path, Open::Saved)
    }

    /// The recording at `path` at its newest revision, saved or not: what its recorder goes on
    /// from.
    pub fn open_newest(path: &Path) -> io::Result<Self> {
        Self::open_path_as(path, Open::Newest)
    }

    /// The recording at `path` at the revision `open` names.
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
            warnings.push(crate::codec::try_format(format_args!("{damage}"))?);
            damaged_parts += 1;
        }
        for run in 0..recording.run_count() {
            let Ok(held) = recording.run(run) else {
                continue;
            };
            if !known.contains(&held.branch) {
                engine_allocation::reserve(&mut warnings, 1)?;
                warnings.push(crate::codec::try_format(format_args!(
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

    /// Builds its frames on `pool`'s threads from now on, or on rayon's own without one.
    pub fn set_pool(&mut self, pool: Option<std::sync::Arc<crate::columnar_frames::Pool>>) {
        self.pool = pool;
    }

    /// The columnar recording its hours are read from.
    pub fn recording(&self) -> &Recording {
        &self.recording
    }

    /// How much of capture `capture` made in `source` it holds; none when it holds none of it.
    pub fn cursor(&self, source: &str, capture: u32) -> Option<&Cursor> {
        self.cursors
            .iter()
            .rev()
            .find(|cursor| cursor.source == source && cursor.capture == capture)
    }

    /// The file it was opened from; none when held in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_file_backed(&self) -> bool {
        self.path.is_some()
    }

    /// The bytes of a recording opened from memory.
    ///
    /// # Panics
    /// When it was opened from its file.
    pub fn bytes(&self) -> &[u8] {
        self.recording
            .memory()
            .expect("a recording read from its file holds no bytes in memory")
    }

    /// `block`'s segment in [`Recording::segments`].
    pub(crate) fn segment_of(&self, block: &BlockIndex) -> io::Result<usize> {
        // The recording's pages list runs in order, each run's pages in order.
        let (pages, segments) = match &self.physical {
            Some(physical) => (&physical.index, &physical.segments),
            None => (&self.index, &self.segments),
        };
        let position = pages
            .binary_search_by_key(&(block.run, block.page), |held| (held.run, held.page))
            .map_err(|_| invalid_static("the block is none of the recording's"))?;
        Ok(segments[position] as usize)
    }

    /// Whether blocks `a` and `b` lie in one segment: a walk from its restart reads both.
    pub fn same_segment(&self, a: &BlockIndex, b: &BlockIndex) -> bool {
        matches!((self.segment_of(a), self.segment_of(b)), (Ok(a), Ok(b)) if a == b)
    }

    /// The revision opened.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The newest saved revision: the one opened, or the one before it that it names. None
    /// while no revision of the recording is saved.
    pub fn saved_revision(&self) -> Option<Revision> {
        self.saved
    }

    /// Whether the revision opened is not saved: hours wait to be saved or discarded.
    pub fn unsaved(&self) -> bool {
        self.saved != Some(self.revision)
    }

    /// Whether `other` is this recording at this same revision.
    pub fn same_revision(&self, other: &Self) -> bool {
        self.campaign_id == other.campaign_id && self.revision == other.revision
    }

    /// Whether the recording holds `revision`: this one, or for one read from its file, a
    /// revision the file still ends one of its footers at.
    pub fn contains_revision(&self, revision: Revision) -> io::Result<bool> {
        if revision == self.revision {
            return Ok(true);
        }
        match &self.path {
            Some(path) if revision.number < self.revision.number => writer::holds_revision(path, revision),
            _ => Ok(false),
        }
    }

    /// The file's length through the revision opened.
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

    /// Opening withheld records or found structural or digest damage. Recorded partial
    /// captures and spool recovery notes alone are not damage here.
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

    /// Why each table is withheld in this recording's format: added since, or read otherwise.
    pub fn compatibility_gaps(&self) -> Vec<String> {
        self.recording.withheld().iter().map(|(_, reason)| reason.clone()).collect()
    }

    /// Game facts pack `at` ([`Self::fact_packs`]), checked against its digest.
    pub fn facts(&self, at: usize) -> io::Result<Vec<u8>> {
        let extent = self
            .fact_packs
            .get(at)
            .ok_or_else(|| invalid_static("the recording holds no such game facts pack"))?;
        self.recording.stored_bytes(extent)
    }

    /// Checks game facts pack `at` against its digest, read whole once, before its bytes are
    /// read in parts ([`Self::read_facts`]).
    pub fn verify_facts(&self, at: usize) -> io::Result<()> {
        self.facts(at).map(drop)
    }

    /// `out.len()` bytes of game facts pack `at` from `offset` within it.
    pub fn read_facts(&self, at: usize, offset: u64, out: &mut [u8]) -> io::Result<()> {
        let extent = self
            .fact_packs
            .get(at)
            .ok_or_else(|| invalid_static("the recording holds no such game facts pack"))?;
        self.recording.stored_at(extent, offset, out)
    }

    /// The newest game facts pack, when it carries one.
    pub fn newest_facts(&self) -> io::Result<Option<Vec<u8>>> {
        match self.fact_packs.len() {
            0 => Ok(None),
            count => self.facts(count - 1).map(Some),
        }
    }

    /// Why `hour` of `branch` has no hour, as a seek of it names the gap.
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

    /// Every hour recorded of `branch`, in order: each run's hours are consecutive.
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

    /// The first and last recorded hours across every branch; none when it records none.
    pub fn hour_span(&self) -> Option<(Hour, Hour)> {
        let first = self.index.iter().map(|block| block.first_hour).min()?;
        let last = self.index.iter().map(|block| block.last_hour).max()?;
        Some((first, last))
    }

    /// Exact prior history, without recording bytes or decoded rows: every branch's hours and
    /// each session's tail, for a collector or recorder to go on from.
    pub fn into_history(self) -> io::Result<CampaignHistory> {
        if self.physical.is_some() {
            return Err(invalid_static("a recording read as timelines holds no recorder history"));
        }
        self.require_undamaged()?;
        // Each block's last Clock row, which names its load.
        let mut signals = vec![None; self.index.len()];
        let blocks: Vec<_> = self.index.iter().collect();
        let mut at = 0;
        self.visit_blocks(
            &blocks,
            &[crate::rows::TableId::Clock],
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

    /// The partial notice of the recording at `path`, without walking it.
    pub fn read_partial(path: &Path) -> io::Result<Option<ControlRecord>> {
        Ok(Summary::read(path)?.partial)
    }

    /// The campaign UUID the recording at `path` names; none for another identity.
    pub fn read_campaign_uuid(path: &Path) -> io::Result<Option<Vec<u8>>> {
        let id = Self::read_campaign_id(path)?;
        Ok(crate::identity::is_campaign_uuid(&id).then_some(id))
    }

    /// The campaign id the recording at `path` names, of whatever identity: every revision's,
    /// so a recording not saved yet names it too.
    pub fn read_campaign_id(path: &Path) -> io::Result<Vec<u8>> {
        Ok(Summary::read_as(path, Open::Newest)?.campaign_id)
    }

    /// The first and last recorded hours across every branch; none for a recording of none.
    pub fn read_hour_span(path: &Path) -> io::Result<Option<(Hour, Hour)>> {
        Ok(Summary::read(path)?.span)
    }

    /// The capture mode of the recording at `path`: every revision's.
    pub fn read_capture_mode(path: &Path) -> io::Result<CaptureMode> {
        Ok(Summary::read_as(path, Open::Newest)?.capture_mode)
    }

    /// The ids of the sessions the recording at `path` holds, without walking it.
    pub fn read_session_ids(path: &Path) -> io::Result<Vec<SessionId>> {
        Ok(Summary::read(path)?
            .sessions
            .iter()
            .map(|session| session.id)
            .collect())
    }
}

/// What a recording's root and records say, read without its pages: what lists and names it.
pub struct Summary {
    pub campaign_id: Vec<u8>,
    pub capture_mode: CaptureMode,
    pub sessions: Vec<SessionStart>,
    pub partial: Option<ControlRecord>,
    pub span: Option<(Hour, Hour)>,
    pub revision: Revision,
}

impl Summary {
    /// Of the newest saved revision of the recording at `path`.
    pub fn read(path: &Path) -> io::Result<Self> {
        Self::read_as(path, Open::Saved)
    }

    /// Of the revision of the recording at `path` that `open` names.
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

/// Reads as much of `header` as the file holds from its start.
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

pub(crate) fn decode_identity(content: &[u8]) -> io::Result<Identity> {
    let mut decoder = Decoder::new(content);
    let identity = Identity::decode(&mut decoder)?;
    decoder.finish()?;
    Ok(identity)
}

/// The digest the footer of `recording`'s last revision names.
fn root_digest(recording: &Recording) -> io::Result<[u8; 16]> {
    recording.root_footer_digest()
}

/// Exact validated prior hours and provenance, without recording bytes or decoded rows.
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
    /// Actual recorded hours only. Holes between runs are never filled.
    pub hours: Vec<Hour>,
    pub last_missing_hour: Option<Hour>,
}

/// A process's latest hour or explicitly witnessed command-only phase.
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

    /// Seed exact history and select a new process session or resume the same process.
    /// Same-process reattachment requires its complete saved session metadata to match.
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

/// The load the Clock row of `frame` names, when it holds one.
fn clock_signal(frame: &DecodedFrame) -> Option<i64> {
    match &*frame.tables[crate::rows::TableId::Clock.index()] {
        crate::frame::TableView::Complete(crate::rows::TableRows::Clock(rows)) => {
            rows.first().map(|clock| clock.play_session_started_at)
        }
        _ => None,
    }
}

impl CampaignHistory {
    /// The history spool `segments` of campaign `campaign_id` hold: their sessions and branches,
    /// every recorded hour of each branch, and each session's tail, its last hour's load from
    /// its Clock row.
    pub fn from_spool(
        campaign_id: &[u8],
        segments: &[(u32, crate::spool::Recovery<crate::spool::Recovered>)],
    ) -> io::Result<Self> {
        use crate::spool::Recovered;
        let clock_decoder = || crate::frame::FrameDecoder::for_tables(&[crate::rows::TableId::Clock]);
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

/// Every branch's hours and each session's tail from `spans` (branch, first hour, last hour, and
/// the last hour's load), in recording order, and the records they go with.
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
    // The latest matching witness stands only where the last recorded hour has no Clock row.
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

/// Checks the records against one another: a damaged gap or control list is withheld with a
/// warning, any other inconsistency refuses the recording.
#[allow(clippy::too_many_arguments)]
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
        warnings.push(crate::codec::try_format(format_args!(
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
