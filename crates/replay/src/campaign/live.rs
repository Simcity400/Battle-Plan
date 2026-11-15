//! Writes a campaign's recording while it is recorded: one capture's records ([`Source`]), in
//! order, into the campaign's own file. Each hour is applied to the recorder's store a section at
//! a time ([`replay_columnar::store`]), its derived tables written by their writers
//! ([`recorder_derive`]) and its summaries built, and the store's encoder writes the hour's
//! changes into the file as its parts close ([`replay_columnar::unit`]); each block's end writes
//! its page ([`crate::columnar_file`]); each segment's end commits one revision holding its
//! blocks, the records that came with them and the capture's cursor, after which the source lets
//! go of the segment. Nothing is written anywhere else first, and nothing is copied afterwards.
//!
//! A branch is decided before anything of it is written ([`Host::admit`]): its records are
//! kept in memory until then, and the first hour of a branch that waits is not taken, so its
//! hours stay with their source. A recording is created at its first hour written, which is its
//! run's first restore.
//!
//! A revision is committed with the writer's resume state in its state slot: the run under way
//! (its cursor, the encoder's, the journal adapter's and the writers' state, the summaries'
//! ladders; no table) and the records it still keeps to itself. A writer taking the recording up
//! after a crash rebuilds the run's tables from the file, restores the rest and reads on from its
//! cursor's segment, so no hour is left out and none is written twice.

use super::records::{CaptureStop, Cursor, Records, RecoveryNote};
use crate::{
    BranchId, Hour,
    branches::{BranchStart, Gap},
    codec::Decoder,
    columnar_file::{BlockEnd, Capture, FileStrings, FileWriter, OpenRun, Resumed, Settings, Stop},
    columnar_frames::{Recording, keeps_last, schema},
    columnar_journal::{Record, Sections, Source, Witness},
    format::FormatVersion,
    identity::{Identity, IdentityCodec, SessionStart},
    scope::CaptureMode,
    spool::{self, ControlRecord},
};
use replay_columnar::{
    container::{self, ContainerWriter, Open, Revision, SectionKey, SectionKind, StateEntry, StateFile, Tags, schema as sections, state_file},
    envelope::{self, Kind, SPAN_BLOCKS, UNIT_ORDINALS},
    lane::{Clock, ClockHour},
    leaf::{Reader, Writer as Bytes},
    manifest::{Manifest, Status},
    state::Kind as Column,
    store::{Store, TableState, journal::Journal as Adapter},
    summary::{self, FANOUT, Shape},
    unit::{self, Encoder, FrameEntry, reader},
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::Instant,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The one session a capture's start record (`Record::Begin`'s `capture`) begins with.
pub fn capture_session(capture: &[u8]) -> io::Result<SessionStart> {
    let capture = Capture::read(capture)?;
    if capture.format != FormatVersion::CURRENT.version() {
        return Err(invalid(format!(
            "A capture of row format {}; this build writes row format {} only",
            capture.format,
            FormatVersion::CURRENT.version()
        )));
    }
    let [session] = capture.sessions.as_slice() else {
        return Err(invalid("A capture begins with its one session"));
    };
    let mut input = Decoder::new(session);
    input.format = capture.format;
    let session = SessionStart::decode(&mut input)?;
    input.finish()?;
    Ok(session)
}

/// Labels this thread's allocations as the writer's part `name` until the scope drops, for a
/// build counting memory (`memory-labels`); nothing otherwise.
fn label(name: &str) -> engine_allocation::labels::Scope {
    use engine_allocation::labels;
    labels::enter(labels::owner(name), labels::Kind::Operator)
}

/// Whether a branch's hours and records are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    Write,
    /// Never: they are passed over.
    Skip,
    /// Not decided yet: its records are kept aside and its first hour is not taken.
    Wait,
}

/// A branch the writer asks about before it writes anything of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Asked {
    pub branch: BranchId,
    /// The load its first hour or command phase names; none while neither names one.
    pub load: Option<i64>,
    /// When that hour or phase was read, by its session's performance counter.
    pub qpc: Option<u64>,
    /// Asked at an hour of the branch: whatever names its load before that hour has been read.
    pub hour: bool,
    /// The capture ended: nothing more names a load, and a branch left waiting now is left out.
    pub last: bool,
}

/// An hour as the writer applied it: the store after it.
pub struct Applied<'a> {
    pub store: &'a Store,
    pub branch: BranchId,
    pub hour: Hour,
}

/// One revision the writer committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    pub revision: Revision,
    pub saved: bool,
    /// Hours it added.
    pub hours: u64,
    /// The capture ended with it.
    pub finished: bool,
}

/// A game facts pack at its place in a file: `length` bytes from `offset`.
pub struct FactsFile {
    pub file: File,
    pub offset: u64,
    pub length: u64,
}

/// A session's game facts pack as its host gives it.
enum GivenFacts {
    Held(Vec<u8>),
    Placed(FactsFile),
}

/// The game facts pack of `identity` that `host` gives, from a file where it can.
fn given_facts(host: &mut dyn Host, identity: &Identity) -> Option<GivenFacts> {
    host.facts_file(identity).map(GivenFacts::Placed).or_else(|| host.facts(identity).map(GivenFacts::Held))
}

/// Whoever runs the writer: what it decides and what it is told.
pub trait Host {
    /// Whether `asked`'s branch is written. Asked before the branch's first write and again
    /// while the answer is [`Admit::Wait`].
    fn admit(&mut self, asked: &Asked) -> Admit;

    /// Whether the revision about to be committed is saved.
    fn saves(&mut self) -> bool;

    /// The game facts pack of content `identity`, for a recording that carries none yet.
    fn facts(&mut self, identity: &Identity) -> Option<Vec<u8>> {
        let _ = identity;
        None
    }

    /// [`Self::facts`] where a file holds them, read from it a piece at a time and never whole;
    /// asked first.
    fn facts_file(&mut self, identity: &Identity) -> Option<FactsFile> {
        let _ = identity;
        None
    }

    /// An hour of a written branch was applied.
    fn hour(&mut self, hour: &Applied<'_>) {
        let _ = hour;
    }

    /// A run of a branch that is not written ended after `hours` hours.
    fn skipped(&mut self, asked: &Asked, hours: u64) {
        let _ = (asked, hours);
    }

    /// A revision was committed.
    fn committed(&mut self, committed: &Committed) {
        let _ = committed;
    }
}

/// Another owner's part of each revision the writer commits: a section of its own, written
/// from the same hours, with its own entry in the resume state.
pub trait Part: Send {
    /// Declares its section in `container`, and takes up from `state`, its entry of the resume
    /// state the recording was taken up with; none in a new recording or when it kept none.
    fn open(
        &mut self,
        container: &mut ContainerWriter<File>,
        state: Option<&StateEntry>,
    ) -> io::Result<()>;

    /// An hour of a written branch was applied.
    fn hour(&mut self, hour: &Applied<'_>) -> io::Result<()>;

    /// Puts and names its extents of the revision about to be committed; its resume state for
    /// that revision's slot.
    fn revise(&mut self, container: &mut ContainerWriter<File>) -> io::Result<Option<StateEntry>>;
}

/// What became of a record given to the writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taken {
    /// Taken: its reader moves past it.
    Moved,
    /// Taken, and every segment before it is settled: a committed revision holds what the
    /// recording keeps of them, so their source lets go of them.
    Settled,
    /// Not taken: the hour's branch waits to be decided.
    Waits(Asked),
    /// Taken: the capture ended, and the recording holds all it keeps of it.
    Finished,
}

/// What one call to [`LiveWriter::advance`] got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// It took every record made so far.
    Waiting,
    /// It stopped when asked, with records left.
    Paused,
    /// The next hour's branch waits to be decided.
    Undecided(Asked),
    /// The capture ended.
    Finished,
}

/// The schema version of the world log's resume state ([`LiveWriter::world_log_state`]).
const WORLD_LOG_STATE: u32 = 7;
/// The schema version of the records' resume state ([`LiveWriter::records_state`]).
const RECORDS_STATE: u32 = 1;

/// What the writer knows of one branch of its capture.
#[derive(Default)]
struct Branch {
    load: Option<i64>,
    /// A command phase named it before its first simulation update: its load is 0.
    bootstrap: bool,
    qpc: Option<u64>,
    admit: Option<bool>,
    /// Its records, until it is decided.
    held: Vec<spool::Record>,
}

impl Branch {
    fn load(&self) -> Option<i64> {
        if self.bootstrap { Some(0) } else { self.load }
    }
}

/// The run the capture's records are at.
#[derive(Clone, Copy)]
struct Run {
    branch: BranchId,
    epoch: u8,
    first_hour: Hour,
    /// Hours of it taken.
    hours: u32,
    /// Its branch is not written.
    skipped: bool,
    /// The file holds its start.
    started: bool,
}

/// What the recording holds of the records a capture may give again.
#[derive(Default)]
struct Known {
    sessions: Vec<SessionStart>,
    branches: BTreeMap<BranchId, BranchStart>,
    gaps: Vec<Gap>,
    losses: Vec<ControlRecord>,
    stops: Vec<(String, u32)>,
    facts: bool,
}

/// The block under way: its first hour and hours so far, its clock, and the strings its hours
/// appended (`count | per string: length | bytes`) from the first one's id.
struct OpenBlock {
    hours: u32,
    clock: Clock,
    count: u64,
    /// The block's appends, after [`COUNT_ROOM`] bytes its count is written into at the end.
    strings: Vec<u8>,
    first_string: u32,
}

/// Room for a varint of a block's string count ahead of its strings, so the frame is written
/// from the buffer the hours filled.
const COUNT_ROOM: usize = 10;

/// A summarized table's pieces of the levels above a block's not folded yet, by level from 1: those
/// of the span or era under way, as the file holds them. Its level-0 pieces are the open span's
/// blocks', read back from the file when the span closes.
#[derive(Default)]
struct Ladder {
    upper: Vec<Vec<PieceAt>>,
}

/// A summary piece the file holds: where, and its digest.
#[derive(Clone, Copy)]
struct PieceAt {
    offset: u64,
    stored: u64,
    digest: [u8; 16],
}

/// The summarized tables' builders of the block under way and the run's ladders.
pub(crate) struct Summaries {
    registry: Vec<(usize, Shape)>,
    /// Each summarized table's memory label.
    owners: Vec<engine_allocation::labels::Owner>,
    builders: Vec<summary::Builder>,
    ladders: Vec<Ladder>,
    level: i32,
    ns: u64,
    /// In a build counting memory: each builder's most bytes read in the block under way (once a
    /// day), and in the block before.
    peaks: Vec<usize>,
    last_peaks: Vec<usize>,
}

impl Summaries {
    pub(crate) fn new(level: i32) -> io::Result<Self> {
        use engine_allocation::labels;
        let registry = crate::summaries::registry()?;
        let owners = registry
            .iter()
            .map(|&(table, _)| match labels::ENABLED {
                true => labels::owner(&format!("summaries: {:?}", crate::rows::TableId::ALL[table])),
                false => labels::Owner::RUN,
            })
            .collect();
        let ladders = registry.iter().map(|_| Ladder::default()).collect();
        let peaks = vec![0; registry.len()];
        Ok(Self { last_peaks: peaks.clone(), peaks, registry, owners, builders: Vec::new(), ladders, level, ns: 0 })
    }

    /// Charges what the thread allocates to the `at`th summarized table's label until the scope
    /// drops.
    fn label(&self, at: usize) -> engine_allocation::labels::Scope {
        engine_allocation::labels::enter(self.owners[at], engine_allocation::labels::Kind::Operator)
    }

    pub(crate) fn start_run(&mut self) {
        self.builders.clear();
        self.ladders = self.registry.iter().map(|_| Ladder::default()).collect();
    }

    /// The block at `hour` opens: each builder from its table as the hour before left it.
    pub(crate) fn open(&mut self, store: &mut Store, hour: i32) {
        let clock = Instant::now();
        for &(table, _) in &self.registry {
            store.index(table);
        }
        self.builders = (0..self.registry.len())
            .map(|at| {
                let _label = self.label(at);
                let (table, shape) = &self.registry[at];
                let view = store.view(*table);
                summary::Builder::open(shape, &view, view.status(), hour)
            })
            .collect();
        self.ns += clock.elapsed().as_nanos() as u64;
    }

    pub(crate) fn hour(&mut self, store: &Store, hour: i32) {
        let clock = Instant::now();
        for ((builder, (table, _)), &owner) in self.builders.iter_mut().zip(&self.registry).zip(&self.owners) {
            let _label = engine_allocation::labels::enter(owner, engine_allocation::labels::Kind::Operator);
            let view = store.view(*table);
            builder.hour(hour, &view, &store.log().tables[*table], view.status(), view.view());
        }
        self.ns += clock.elapsed().as_nanos() as u64;
        if engine_allocation::labels::ENABLED && hour.rem_euclid(24) == 0 {
            self.note_peaks();
        }
    }

    fn note_peaks(&mut self) {
        for (peak, builder) in self.peaks.iter_mut().zip(&self.builders) {
            *peak = (*peak).max(builder.held_bytes());
        }
    }

    /// By summarized table: its ladder's bytes (where the file holds its pieces), its builder's
    /// bytes now, and in a build counting memory the most its builder held in the block before
    /// (read once a day and as the block ends, before it finishes).
    pub(crate) fn table_bytes(&self) -> Vec<(usize, usize, usize, usize)> {
        self.registry
            .iter()
            .enumerate()
            .map(|(at, (table, _))| {
                let ladder = self.ladders[at].upper.iter().map(|level| level.capacity() * size_of::<PieceAt>()).sum();
                (*table, ladder, self.builders.get(at).map_or(0, summary::Builder::held_bytes), self.last_peaks[at])
            })
            .collect()
    }

    /// The block ending at `last_hour`, the run's `block`th from `first_hour`, finished: each
    /// table's level-0 piece, and when the block closes its span the pieces of the levels above
    /// it closes, as (table, level, piece), folded with the pieces `file` wrote before.
    #[allow(clippy::type_complexity, reason = "the block's pieces and the higher levels it closes, returned together")]
    pub(crate) fn finish<W: io::Write + Seek + io::Read>(&mut self, last_hour: i32, block: u32, first_hour: i32, file: &mut FileWriter<W>) -> io::Result<(Vec<(u32, Vec<u8>)>, Vec<(u32, u8, Vec<u8>)>)> {
        let clock = Instant::now();
        let closes = (block + 1) % SPAN_BLOCKS == 0;
        if engine_allocation::labels::ENABLED {
            self.note_peaks();
            self.last_peaks = std::mem::replace(&mut self.peaks, vec![0; self.registry.len()]);
        }
        let mut pieces = Vec::new();
        let mut above = Vec::new();
        let builders = std::mem::take(&mut self.builders);
        for (at, builder) in builders.into_iter().enumerate() {
            let _label = self.label(at);
            let table = self.registry[at].0 as u32;
            let piece = builder.finish(last_hour + 1)?.map(|raw| raw.assemble(self.level)).transpose()?;
            if !closes {
                pieces.extend(piece.map(|piece| (table, piece)));
                continue;
            }
            // Level `level + 1` closes when the run's blocks fill a whole number of its pieces.
            let mut held = file.span_summaries(table)?;
            held.extend(piece.iter().cloned());
            pieces.extend(piece.map(|piece| (table, piece)));
            let mut level = 0usize;
            let mut blocks = u64::from(SPAN_BLOCKS);
            while u64::from(block + 1) % blocks == 0 {
                let start = i64::from(first_hour) + (u64::from(block + 1) - blocks) as i64 * i64::from(UNIT_ORDINALS);
                let hours = (i64::from(last_hour) + 1 - start) as u32;
                let refs: Vec<&[u8]> = held.iter().map(Vec::as_slice).collect();
                let folded = match refs.is_empty() {
                    true => None,
                    false => summary::fold(&refs, start, hours)?.map(|raw| raw.assemble(self.level)).transpose()?,
                };
                level += 1;
                blocks *= FANOUT as u64;
                let ladder = &mut self.ladders[at];
                if ladder.upper.len() < level {
                    ladder.upper.push(Vec::new());
                }
                if u64::from(block + 1) % blocks == 0 {
                    held = std::mem::take(&mut ladder.upper[level - 1]).iter().map(|piece| file.summary(piece.offset, piece.stored, piece.digest)).collect::<io::Result<_>>()?;
                    held.extend(folded.iter().cloned());
                }
                above.extend(folded.map(|folded| (table, level as u8, folded)));
            }
        }
        self.ns += clock.elapsed().as_nanos() as u64;
        Ok((pieces, above))
    }

    /// The pieces of the levels above a block's the file wrote at a block's end, (table, level,
    /// where): of each table the highest, which the levels below it were folded into.
    pub(crate) fn written(&mut self, span: &[(u32, u8, envelope::Extent)]) {
        for (at, ladder) in self.ladders.iter_mut().enumerate() {
            let table = self.registry[at].0 as u32;
            let Some(&(_, level, extent)) = span.iter().filter(|(held, _, _)| *held == table).max_by_key(|(_, level, _)| *level) else { continue };
            let level = usize::from(level);
            while ladder.upper.len() < level {
                ladder.upper.push(Vec::new());
            }
            ladder.upper[level - 1].push(PieceAt { offset: extent.offset, stored: extent.stored, digest: extent.digest });
        }
    }

    /// The pieces of the level above a block's covering the last span of a run of `hours` hours
    /// from `first_hour`, ended, when that span is not whole: folded from its blocks' pieces
    /// `file` wrote.
    pub(crate) fn tail<W: io::Write + Seek + io::Read>(&mut self, first_hour: i32, hours: u32, file: &mut FileWriter<W>) -> io::Result<Vec<(u32, Vec<u8>)>> {
        let clock = Instant::now();
        let blocks = hours.div_ceil(UNIT_ORDINALS);
        let start = i64::from(first_hour) + i64::from(blocks - blocks % SPAN_BLOCKS) * i64::from(UNIT_ORDINALS);
        let span_hours = (i64::from(first_hour) + i64::from(hours) - start) as u32;
        let mut tail = Vec::new();
        for at in 0..self.registry.len() {
            let _label = self.label(at);
            let table = self.registry[at].0 as u32;
            let held = file.span_summaries(table)?;
            if held.is_empty() {
                continue;
            }
            let refs: Vec<&[u8]> = held.iter().map(Vec::as_slice).collect();
            if let Some(folded) = summary::fold(&refs, start, span_hours)? {
                tail.push((table, folded.assemble(self.level)?));
            }
        }
        self.ns += clock.elapsed().as_nanos() as u64;
        Ok(tail)
    }

    fn write(&self, out: &mut Bytes) -> io::Result<()> {
        out.varint(self.ladders.len() as u64)?;
        for ladder in &self.ladders {
            out.varint(ladder.upper.len() as u64)?;
            for level in &ladder.upper {
                out.varint(level.len() as u64)?;
                for piece in level {
                    out.varint(piece.offset)?;
                    out.varint(piece.stored)?;
                    out.raw(&piece.digest)?;
                }
            }
        }
        Ok(())
    }

    fn read(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut reader = Reader::new(bytes);
        if reader.count()? != self.ladders.len() {
            return Err(invalid("A resume state's summaries are not this build's"));
        }
        for ladder in &mut self.ladders {
            let levels = reader.count()?;
            if levels > 16 {
                return Err(invalid("A resume state's summaries are malformed"));
            }
            ladder.upper = Vec::with_capacity(levels);
            for _ in 0..levels {
                let count = reader.count()?;
                if count > FANOUT as usize {
                    return Err(invalid("A resume state's summaries are malformed"));
                }
                let mut held = Vec::with_capacity(count);
                for _ in 0..count {
                    let offset = reader.varint()?;
                    let stored = reader.varint()?;
                    let digest = reader.raw(16)?.try_into().expect("sixteen bytes");
                    held.push(PieceAt { offset, stored, digest });
                }
                ladder.upper.push(held);
            }
        }
        reader.finish()
    }

    /// What its builders, and its ladders, hold, in bytes.
    fn held_bytes(&self) -> (usize, usize) {
        let builders = self.builders.iter().map(summary::Builder::held_bytes).sum();
        let ladders = self.ladders.iter().map(|ladder| ladder.upper.iter().map(|level| level.capacity() * size_of::<PieceAt>()).sum::<usize>()).sum();
        (builders, ladders)
    }
}

/// Table `table` as the last of `pages` left it, read from the file: a log's count and live set,
/// a state table's rows. Its class in the file must be this build's.
fn rebuilt(source: &dyn reader::Extents, pages: &[unit::BlockPage], kinds: &[Column], table: usize, run_first: i32) -> io::Result<TableState> {
    let log = reader::class_of(pages, table)?;
    if log != crate::rows::logs()[table] {
        let (file, build) = if log { ("a log", "state") } else { ("state", "a log") };
        return Err(invalid(format!("{:?} was recorded as {file} and this build holds it as {build}", crate::rows::TableId::ALL[table])));
    }
    if log {
        reader::log_state(source, pages, kinds, table, pages.len() - 1)
    } else {
        Ok(reader::rebuild(source, pages, kinds, table, pages.len() - 1, run_first)?.0)
    }
}

/// Refuses an hour that changed a complete Campaign table to hold anything but `campaign`: so a
/// reader of a recording's tables never checks it.
fn check_campaign(store: &Store, campaign: &[u8]) -> io::Result<()> {
    let table = crate::rows::TableId::Campaign.index();
    let log = &store.log().tables[table];
    let held = store.view(table);
    let changed = if log.whole { held.rows().next().is_some() } else { !log.inserted.is_empty() || !log.written.is_empty() || !log.deleted.is_empty() };
    if !changed || held.status() != Status::Complete {
        return Ok(());
    }
    let mut live = held.rows();
    let named = match (live.next(), live.next()) {
        (Some(slot), None) => held.text(0, slot).and_then(|id| held.string(id)),
        _ => None,
    };
    if named != Some(campaign) {
        return Err(invalid("An hour's campaign identity differs from its recording's"));
    }
    Ok(())
}

/// How a recording's one writer opens it: readers go on and it can be renamed, and nothing
/// else can open it to write, so no other revision is added under the writer.
fn held() -> fs::OpenOptions {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
    options
}

/// A read handle on a recording its writer holds open.
fn reading(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};
    fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

/// Every string chunk of the run under way: its first string id and its frame, as written.
type Frames = Arc<Mutex<Vec<(u32, FrameEntry)>>>;

/// Where the writer's own thread spent its time, by stage.
#[derive(Clone, Copy, Debug, Default)]
pub struct Spent {
    /// The journal's records and sections read.
    pub journal: std::time::Duration,
    /// Sections applied to the store, events appended to pending.
    pub store: std::time::Duration,
    pub writers: std::time::Duration,
    pub summaries: std::time::Duration,
    /// The store's hour ends: parts closed, restores made, frames handed to the file, waits for
    /// the compression thread.
    pub ending: std::time::Duration,
    /// Blocks' pages, clocks and strings written.
    pub blocks: std::time::Duration,
}

/// Writes one capture's records into its campaign's recording.
pub struct LiveWriter {
    path: PathBuf,
    state_path: PathBuf,
    settings: Settings,
    source: String,
    capture: u32,
    known: Known,
    file: Option<FileWriter<File>>,
    /// The identity and session its capture began with.
    begun: Option<(envelope::Identity, SessionStart)>,
    run: Option<Run>,
    /// The branch the records being given belong to.
    context: Option<BranchId>,
    branches: BTreeMap<BranchId, Branch>,
    /// The records the next revision adds: every one given since the last, in the order they
    /// were given, each hour's clock witness among them.
    pending: Records,
    /// Segments of the capture taken whole.
    segments: u32,
    /// Hours written since the last revision.
    hours: u64,
    finished: bool,
    parts: Vec<Box<dyn Part>>,
    kinds: Vec<Vec<Column>>,
    columns: Vec<usize>,
    store: Store,
    adapter: Adapter,
    writers: recorder_derive::Writers,
    writer_ns: u64,
    summaries: Summaries,
    block: Option<OpenBlock>,
    frames: Frames,
    spent: Spent,
    /// What its file writer wrote, kept once the recording is finished.
    finished_written: Option<crate::columnar_file::Written>,
    /// The session's game facts pack is in the file, for the next revision to name.
    facts_put: bool,
    /// The writers were handed the definitions of the recording's or the session's facts.
    defined: bool,
}

impl LiveWriter {
    /// The writer of capture `capture` made in `source` into the recording at `path`, which it
    /// takes up after its newest revision when it exists and creates at its first hour when it
    /// does not. Its resume state is kept at `state_path`. `parts` are the other owners of each
    /// revision. Its source reads on from [`Self::segments`].
    pub fn open(
        path: &Path,
        state_path: &Path,
        source: &str,
        capture: u32,
        settings: Settings,
        parts: Vec<Box<dyn Part>>,
    ) -> io::Result<Self> {
        Self::open_with(path, state_path, source, capture, settings, parts, recorder_derive::writers())
    }

    /// [`Self::open`] with `writers` as the derived tables' writers.
    pub(crate) fn open_with(
        path: &Path,
        state_path: &Path,
        source: &str,
        capture: u32,
        settings: Settings,
        parts: Vec<Box<dyn Part>>,
        writers: recorder_derive::Writers,
    ) -> io::Result<Self> {
        let kinds = schema()?;
        let columns = kinds.iter().map(Vec::len).collect();
        let derived = writers.tables();
        let mut store = {
            let _label = label("store");
            Store::new(kinds.clone(), keeps_last().to_vec())
        };
        store.declare_logs(&crate::rows::logs())?;
        store.attach(Encoder::new(&kinds, unit::Settings::at(settings.level))?);
        let mut writer = Self {
            path: path.to_path_buf(),
            state_path: state_path.to_path_buf(),
            settings,
            source: source.to_owned(),
            capture,
            known: Known::default(),
            file: None,
            begun: None,
            run: None,
            context: None,
            branches: BTreeMap::new(),
            pending: Records::default(),
            segments: 0,
            hours: 0,
            finished: false,
            parts,
            kinds,
            columns,
            store,
            adapter: Adapter::leaving(&derived, crate::columnar_frames::names()),
            writers,
            writer_ns: 0,
            summaries: Summaries::new(settings.level)?,
            block: None,
            frames: Frames::default(),
            spent: Spent::default(),
            finished_written: None,
            facts_put: false,
            defined: false,
        };
        match fs::metadata(path) {
            Ok(_) => writer.take_up()?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(writer)
    }

    /// The segments of its capture the recording holds: where its source reads on.
    pub fn segments(&self) -> u32 {
        self.segments
    }

    /// Whether its capture ended and the recording holds all it keeps of it.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Where the recording is, or will be created.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The recording was moved to `path`.
    pub fn moved(&mut self, path: &Path) {
        self.path = path.to_path_buf();
    }

    /// Whether the recording exists yet.
    pub fn created(&self) -> bool {
        self.file.is_some() || self.finished && self.path.exists()
    }

    /// The newest saved revision of the recording it writes; none while it holds none or does
    /// not exist.
    pub fn saved(&self) -> Option<Revision> {
        self.file.as_ref().and_then(FileWriter::saved)
    }

    /// The thread time its derived-table writers and its summaries took so far.
    pub fn derive_ns(&self) -> (u64, u64) {
        (self.writer_ns, self.summaries.ns)
    }

    /// Each writer's name and time so far.
    pub fn writer_spent(&self) -> Vec<(&'static str, u64)> {
        self.writers.spent()
    }

    /// The store after the last hour applied.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Times where the store's encoder spends its work ([`Encoder::time`]).
    pub fn time(&mut self, timed: bool) {
        if let Some(encoder) = self.store.encoder_mut() {
            encoder.time(timed);
        }
    }

    /// Where its own thread spent its time so far.
    pub fn spent(&self) -> Spent {
        self.spent
    }

    /// The stored bytes its file writer wrote so far; none before the file exists.
    pub fn written(&self) -> Option<crate::columnar_file::Written> {
        self.file.as_ref().map(FileWriter::written).or(self.finished_written)
    }

    /// By summarized table: its ladder's bytes, its builder's bytes now and its builder's most in
    /// the block before ([`Summaries::table_bytes`]).
    pub fn summary_bytes(&self) -> Vec<(usize, usize, usize, usize)> {
        self.summaries.table_bytes()
    }

    /// Each derived-table writer's resume state, in bytes.
    pub fn writer_blobs(&self) -> io::Result<Vec<(&'static str, usize)>> {
        self.writers.blob_sizes()
    }

    /// What it holds besides the store and the encoder, by part, in bytes.
    pub fn held_parts(&self) -> Vec<(String, u64)> {
        let (store, _) = self.store.bytes();
        let mut out = vec![
            ("store".to_owned(), store.total() as u64),
            ("of the store, strings".to_owned(), store.strings as u64),
            ("summary builders".to_owned(), self.summaries.held_bytes().0 as u64),
            ("summary ladders".to_owned(), self.summaries.held_bytes().1 as u64),
            ("block strings".to_owned(), self.block.as_ref().map_or(0, |block| block.strings.capacity() as u64)),
            ("writers' definitions".to_owned(), self.writers.definitions().bytes() as u64),
        ];
        if let Some(file) = &self.file {
            out.push(("file: open span".to_owned(), file.held_bytes() as u64));
        }
        out
    }

    /// How its capture's session records; none before the capture began.
    pub fn capture_mode(&self) -> Option<CaptureMode> {
        self.begun.as_ref().map(|(_, session)| session.capture_mode)
    }

    /// Takes the recording up after its newest revision: what follows it is cut, the records it
    /// holds are noted, and when its resume state is this capture's, the run under way and the
    /// records kept aside are restored.
    fn take_up(&mut self) -> io::Result<()> {
        let file = held().open(&self.path)?;
        let (root_extent, length) = container::locate(&file, Open::Newest)?
            .ok_or_else(|| invalid("The recording has no footer whose root verifies"))?;
        file.set_len(length)?;
        let log = Recording::open_file(reading(&self.path)?, Open::Newest)?;
        let root = log.root().clone();
        let at = Revision {
            number: root.revision,
            length,
            root: root_extent.digest,
        };
        let records = log.records()?;
        let cursor = records
            .cursors
            .iter()
            .rev()
            .find(|cursor| cursor.source == self.source && cursor.capture == self.capture)
            .cloned();
        self.known = Known {
            sessions: records.sessions.clone(),
            branches: records.branches.iter().map(|branch| (branch.id, branch.clone())).collect(),
            gaps: records.gaps.iter().map(Gap::try_clone).collect::<io::Result<_>>()?,
            losses: records.controls.iter().filter(|control| matches!(control, ControlRecord::EventLoss { .. })).cloned().collect(),
            stops: records.stops.iter().map(|stop| (stop.spool.clone(), stop.capture)).collect(),
            facts: !log.facts_extents().is_empty(),
        };
        // The resume state is taken up only by the capture it was written for: any other
        // leaves the run that was under way where its last block ended.
        let mut entries = Vec::new();
        if let (Some(slot), Some(cursor)) = (root.state, cursor.as_ref().filter(|cursor| !cursor.finished))
            && self.state_path.exists()
            && let Some(stored) = StateFile::open(&self.state_path)?.read(slot)?
        {
            entries = container::read_entries(&container::decompress(&stored, None)?)?;
            let records = entries
                .iter()
                .find(|entry| entry.owner == SectionKey::RECORDS)
                .ok_or_else(|| invalid("The recording's resume state names no capture"))?;
            records.expect(RECORDS_STATE)?;
            self.restore_records(&records.bytes)?;
            let session = self
                .known
                .sessions
                .iter()
                .find(|session| session.id == cursor.session)
                .cloned()
                .ok_or_else(|| invalid("The recording's cursor names a session it does not hold"))?;
            self.begun = Some((root.identity.clone(), session));
        }
        if let Some(cursor) = &cursor {
            self.segments = cursor.segments;
            self.finished = cursor.finished;
        }
        if cursor.as_ref().is_some_and(|cursor| !cursor.finished && cursor.segments > 0) && self.begun.is_none() {
            return Err(invalid(format!(
                "{}: capture {} of {} is under way in it, and its resume state is gone, so its hours cannot go on",
                self.path.display(),
                self.capture,
                self.source
            )));
        }
        // The writers take their definitions up from the recording's own facts, never the game's,
        // read a piece at a time from the file.
        if let Some(extent) = log.facts_extents().last() {
            if !self.writers.wanted().is_empty() {
                let _label = label("definitions");
                match (reading(&self.path), self.known.sessions.last()) {
                    (Ok(file), Some(session)) => self.writers.define_from(&recorder_derive::Facts::stored(file, extent.offset, extent.stored), &session.identity),
                    (Ok(_), None) => self.writers.define(recorder_derive::Definitions::absent("the recording's game facts name no session")),
                    (Err(error), _) => self.writers.define(recorder_derive::Definitions::absent(format!("the recording's game facts cannot be read: {error}"))),
                }
            }
            self.defined = true;
        }
        let open = match entries.iter().find(|entry| entry.owner == SectionKey::WORLD_LOG) {
            Some(entry) => {
                entry.expect(WORLD_LOG_STATE)?;
                self.restore_run(&log, &entry.bytes)?
            }
            None => None,
        };
        let last = log
            .directories()?
            .into_iter()
            .rev()
            .find(|run| run.hours > 0)
            .map(|run| (run.branch, run.first_hour + (run.hours - 1) as i32));
        // A branch the file holds a run of was admitted; the resume state keeps only the others.
        for run in log.directories()? {
            self.branches.entry(run.branch).or_default().admit = Some(true);
        }
        drop(log);
        if self.finished {
            return Ok(());
        }
        let state = StateFile::open(&self.state_path)?;
        let resumed = Resumed { root, at, last, open };
        let mut writer = FileWriter::resume(file, resumed, self.kinds.len(), self.settings, Some(state))?;
        if let Some(encoder) = self.store.encoder() {
            writer.compress_on(encoder.frames());
        }
        writer.container().section(SectionKey::FACTS, sections::FACTS)?;
        for part in &mut self.parts {
            let owner = |entry: &&StateEntry| !matches!(entry.owner.kind, SectionKind::WorldLog | SectionKind::Records);
            part.open(writer.container(), entries.iter().find(owner))?;
        }
        self.file = Some(writer);
        Ok(())
    }

    /// The run under way as its resume state `bytes` hold it ([`Self::world_log_state`]): its
    /// tables rebuilt from `log`, the encoder, adapter, writers and summaries taken up.
    fn restore_run(&mut self, log: &Recording, bytes: &[u8]) -> io::Result<Option<OpenRun>> {
        let mut reader = Reader::new(bytes);
        let index = reader.varint_u32()?;
        let hours = reader.varint_u32()?;
        let next_string = reader.varint_u32()?;
        let blob = |reader: &mut Reader<'_>| -> io::Result<Vec<u8>> {
            let length = reader.count()?;
            Ok(reader.raw(length)?.to_vec())
        };
        let encoder = blob(&mut reader)?;
        let adapter = blob(&mut reader)?;
        let writers = blob(&mut reader)?;
        let ladders = blob(&mut reader)?;
        reader.finish()?;
        let run = log.run(index)?.directory().clone();
        if index + 1 != log.run_count() || run.hours != hours || hours % UNIT_ORDINALS != 0 {
            return Err(invalid("The world log's resume state names another run than the file's last"));
        }
        let (first, pages) = log.pages_since_restores(index)?;
        let _ = first;
        let mut tables = Vec::with_capacity(self.kinds.len());
        for (table, kinds) in self.kinds.iter().enumerate() {
            let cells = rebuilt(log.extents(), &pages, kinds, table, run.first_hour)
                .map_err(|error| invalid(format!("{:?} rebuilt from the file: {error}", crate::rows::TableId::ALL[table])))?;
            tables.push(cells);
        }
        drop(pages);
        {
            let _label = label("store");
            self.store.begin_run(run.first_hour);
            self.store.encoder_mut().ok_or_else(|| invalid("The store writes no unit"))?.load(&encoder)?;
            let heads = log.heads(index)?;
            *self.frames.lock().unwrap_or_else(PoisonError::into_inner) = heads.iter().flat_map(|head| head.strings.iter().map(|chunk| (chunk.first, chunk.frame.clone()))).collect();
            self.store.read_strings_from(Box::new(FileStrings::new(reading(&self.path)?, self.frames.clone())));
            self.store.resume(tables, next_string)?;
        }
        self.adapter.load(&adapter)?;
        self.writers.start_run();
        self.writers.load(&writers, &mut self.store)?;
        self.writers.prime(&self.store, run.branch, run.first_hour + hours as i32 - 1);
        self.summaries.start_run();
        self.summaries.read(&ladders)?;
        self.run = Some(Run {
            branch: run.branch,
            epoch: run.epoch,
            first_hour: run.first_hour,
            hours,
            skipped: false,
            started: true,
        });
        let open_first = run.spans.len() as u32 * SPAN_BLOCKS;
        let mut blocks = Vec::with_capacity(run.open.len());
        for block in open_first..run.blocks() {
            blocks.push(log.block(index, block)?);
        }
        Ok(Some(OpenRun { run: index, directory: run, blocks }))
    }

    /// The world log's resume state after the last block written: the run under way, the next
    /// string id, the encoder's, the adapter's and the writers' state, and the summaries'
    /// ladders. No table: they are rebuilt from the file. None without a run under way in the
    /// file.
    fn world_log_state(&self) -> io::Result<Option<StateEntry>> {
        let Some(run) = self.file.as_ref().and_then(FileWriter::open_run) else { return Ok(None) };
        if self.block.is_some() {
            return Err(invalid("A revision is committed at a block's end"));
        }
        let (index, hours) = (run.run, run.hours);
        let _label = label("resume state");
        let mut out = Bytes::new();
        out.varint(u64::from(index))?;
        out.varint(u64::from(hours))?;
        out.varint(u64::from(self.store.strings().next()))?;
        let encoder = self.store.encoder().ok_or_else(|| invalid("The store writes no unit"))?.save();
        let mut ladders = Bytes::new();
        self.summaries.write(&mut ladders)?;
        for blob in [encoder, self.adapter.save(), self.writers.save()?, ladders.into_bytes()] {
            out.varint(blob.len() as u64)?;
            out.raw(&blob)?;
        }
        Ok(Some(StateEntry {
            owner: SectionKey::WORLD_LOG,
            version: WORLD_LOG_STATE,
            bytes: out.into_bytes(),
        }))
    }

    /// The records' resume state: the capture it is, the branch its records are at, a run
    /// being passed over, and each branch not written yet with what is known of it and the
    /// records kept aside for it.
    fn records_state(&self) -> io::Result<StateEntry> {
        let mut out = Bytes::new();
        out.varint(self.source.len() as u64)?;
        out.raw(self.source.as_bytes())?;
        out.varint(u64::from(self.capture))?;
        match self.context {
            None => out.u8(0)?,
            Some(branch) => {
                out.u8(1)?;
                out.varint(branch)?;
            }
        }
        match self.run.filter(|run| run.skipped) {
            None => out.u8(0)?,
            Some(run) => {
                out.u8(1)?;
                out.varint(run.branch)?;
                out.u8(run.epoch)?;
                out.varint(u64::from(run.first_hour as u32))?;
                out.varint(u64::from(run.hours))?;
            }
        }
        let unwritten: Vec<_> = self.branches.iter().filter(|(_, branch)| branch.admit != Some(true)).collect();
        out.varint(unwritten.len() as u64)?;
        for (id, branch) in unwritten {
            out.varint(*id)?;
            out.u8(u8::from(branch.load.is_some())
                | u8::from(branch.bootstrap) << 1
                | u8::from(branch.qpc.is_some()) << 2
                | u8::from(branch.admit == Some(false)) << 3)?;
            out.raw(&branch.load.unwrap_or(0).to_le_bytes())?;
            out.raw(&branch.qpc.unwrap_or(0).to_le_bytes())?;
            out.varint(branch.held.len() as u64)?;
            for record in &branch.held {
                let bytes = spool::encode_record(record)?;
                out.varint(bytes.len() as u64)?;
                out.raw(&bytes)?;
            }
        }
        Ok(StateEntry {
            owner: SectionKey::RECORDS,
            version: RECORDS_STATE,
            bytes: out.into_bytes(),
        })
    }

    fn restore_records(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut reader = Reader::new(bytes);
        let length = reader.count()?;
        let source = reader.raw(length)?;
        let capture = reader.varint_u32()?;
        if source != self.source.as_bytes() || capture != self.capture {
            return Err(invalid(format!(
                "{}: its resume state is of capture {capture} of {}, which is under way in it; finish that session first",
                self.path.display(),
                String::from_utf8_lossy(source),
            )));
        }
        self.context = match reader.u8()? {
            0 => None,
            _ => Some(reader.varint()?),
        };
        if reader.u8()? != 0 {
            self.run = Some(Run {
                branch: reader.varint()?,
                epoch: reader.u8()?,
                first_hour: reader.varint_u32()? as i32,
                hours: reader.varint_u32()?,
                skipped: true,
                started: false,
            });
        }
        let count = reader.count()?;
        if count > bytes.len() {
            return Err(invalid("The records' resume state counts more branches than it holds"));
        }
        for _ in 0..count {
            let id = reader.varint()?;
            let flags = reader.u8()?;
            let load = i64::from_le_bytes(reader.raw(8)?.try_into().expect("eight bytes"));
            let qpc = u64::from_le_bytes(reader.raw(8)?.try_into().expect("eight bytes"));
            let records = reader.count()?;
            if records > bytes.len() {
                return Err(invalid("The records' resume state counts more records than it holds"));
            }
            let mut held = Vec::with_capacity(records);
            for _ in 0..records {
                let length = reader.count()?;
                held.push(spool::decode_record(reader.raw(length)?)?);
            }
            self.branches.insert(
                id,
                Branch {
                    load: (flags & 1 != 0).then_some(load),
                    bootstrap: flags & 2 != 0,
                    qpc: (flags & 4 != 0).then_some(qpc),
                    admit: (flags & 8 != 0).then_some(false),
                    held,
                },
            );
        }
        reader.finish()
    }

    /// Takes each record `source` has made, in order, until it has made no more, `stop` asks, a
    /// branch waits to be decided or the capture ends. The source lets go of each segment once
    /// a committed revision holds what the recording keeps of it.
    pub fn advance(&mut self, source: &mut dyn Source, host: &mut dyn Host, stop: &dyn Fn() -> bool) -> io::Result<Progress> {
        loop {
            if self.finished {
                return Ok(Progress::Finished);
            }
            let reading = Instant::now();
            let record = source.record()?;
            self.spent.journal += reading.elapsed();
            let taken = match record {
                None => return Ok(Progress::Waiting),
                Some(record) => self.take(record, host)?,
            };
            match taken {
                Taken::Waits(asked) => return Ok(Progress::Undecided(asked)),
                Taken::Moved => source.advance(),
                Taken::Settled => {
                    source.advance();
                    source.retire()?;
                }
                Taken::Finished => {
                    source.advance();
                    source.retire()?;
                    return Ok(Progress::Finished);
                }
            }
            if stop() {
                return Ok(Progress::Paused);
            }
        }
    }

    /// One record of the capture, from whoever makes them.
    pub fn take(&mut self, record: Record<'_>, host: &mut dyn Host) -> io::Result<Taken> {
        if self.finished {
            return Err(invalid("A capture's records end with its finish"));
        }
        match record {
            Record::Begin { identity, capture } => {
                if self.begun.is_some() {
                    return Err(invalid("A capture begins once"));
                }
                let identity = envelope::Identity::read_capture(identity)?;
                let session = capture_session(capture)?;
                self.begin(identity, session)?;
                Ok(Taken::Moved)
            }
            Record::Controls(records) => {
                for bytes in records {
                    self.given(spool::decode_record(bytes)?, host)?;
                }
                Ok(Taken::Moved)
            }
            Record::Run { branch, epoch, first_hour } => {
                self.end_run(host)?;
                self.run = Some(Run {
                    branch,
                    epoch,
                    first_hour,
                    hours: 0,
                    skipped: false,
                    started: false,
                });
                Ok(Taken::Moved)
            }
            Record::Hour { witness, manifest, sections } => self.hour(witness, manifest, sections, host),
            Record::Next => {
                // A segment ends at a block's end or its run's: a short block is the run's last.
                if self.block.is_some() {
                    self.end_run(host)?;
                }
                self.segments = self.segments.checked_add(1).ok_or_else(|| invalid("A capture's segments pass 2^32"))?;
                self.settle(host, None)
            }
            Record::Finish { stop } => {
                self.end_run(host)?;
                self.segments = self.segments.saturating_add(1);
                self.settle(host, Some(stop))
            }
        }
    }

    /// The capture begins: `identity` is the recording's, or the recording is `identity`'s, and
    /// `session` is one it holds or a new one of its content and capture mode.
    fn begin(&mut self, identity: envelope::Identity, session: SessionStart) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            let held = file.container().identity();
            if held.schema != identity.schema || !held.campaign.eq_ignore_ascii_case(&identity.campaign) || held.capture_mode != identity.capture_mode {
                return Err(invalid("A capture of another campaign, capture mode or schema than its recording's; record into a new file"));
            }
        }
        session.identity.validate()?;
        if let Some(held) = self.known.sessions.first()
            && (!held.identity.same_content(&session.identity) || held.capture_mode != session.capture_mode)
        {
            return Err(invalid("A capture of other content or another capture mode than its recording's; record into a new file"));
        }
        if self.known.sessions.iter().any(|held| held.id == session.id && *held != session) {
            return Err(invalid("conflicting session context"));
        }
        self.begun = Some((identity, session));
        Ok(())
    }

    fn session(&self) -> io::Result<&SessionStart> {
        self.begun.as_ref().map(|(_, session)| session).ok_or_else(|| invalid("A capture's records precede its start"))
    }

    /// What the host is asked of `branch`.
    fn asked(&self, branch: BranchId, hour: bool, last: bool) -> Asked {
        let known = self.branches.get(&branch);
        Asked {
            branch,
            load: known.and_then(Branch::load),
            qpc: known.and_then(|known| known.qpc),
            hour,
            last,
        }
    }

    /// Whether `branch` is written: as decided before, because the recording names it already,
    /// or as the host says now. A decision keeps or drops the records kept aside for it.
    fn decide(&mut self, branch: BranchId, host: &mut dyn Host, hour: bool, last: bool) -> io::Result<Admit> {
        let decided = match self.branches.get(&branch).and_then(|known| known.admit) {
            Some(admit) => return Ok(if admit { Admit::Write } else { Admit::Skip }),
            None if self.known.branches.contains_key(&branch) => Admit::Write,
            None => match host.admit(&self.asked(branch, hour, last)) {
                Admit::Wait if last => Admit::Skip,
                admit => admit,
            },
        };
        if decided == Admit::Wait {
            return Ok(Admit::Wait);
        }
        let known = self.branches.entry(branch).or_default();
        known.admit = Some(decided == Admit::Write);
        let held = std::mem::take(&mut known.held);
        if decided == Admit::Write {
            for record in held {
                self.keep(record)?;
            }
        }
        Ok(decided)
    }

    /// One record the capture gives beside its hours: kept, dropped or kept aside as the
    /// branch it belongs to is decided.
    fn given(&mut self, record: spool::Record, host: &mut dyn Host) -> io::Result<()> {
        let branch = match &record {
            spool::Record::SessionStart(_) => return Ok(()),
            spool::Record::Frame(_) => return Err(invalid("A capture's records hold no frame")),
            spool::Record::BranchStart(start) => {
                self.context = Some(start.id);
                Some(start.id)
            }
            spool::Record::Gap(gap) => Some(gap.branch),
            spool::Record::Control(ControlRecord::PlayerCommands { packet, .. }) => {
                if let Some(phase) = &packet.phase {
                    let known = self.branches.entry(packet.branch).or_default();
                    known.qpc.get_or_insert(phase.qpc);
                    if phase.load_signal == 0 {
                        known.bootstrap = true;
                    } else {
                        known.load.get_or_insert(phase.load_signal);
                    }
                }
                Some(packet.branch)
            }
            spool::Record::Control(ControlRecord::EventLoss { branch, .. }) => Some(*branch),
            spool::Record::Control(_) => self.context,
        };
        let Some(branch) = branch else {
            return self.keep(record);
        };
        match self.decide(branch, host, false, false)? {
            Admit::Write => self.keep(record),
            Admit::Skip => Ok(()),
            Admit::Wait => {
                self.branches.entry(branch).or_default().held.push(record);
                Ok(())
            }
        }
    }

    /// The capture's session, added to the recording's with the first record kept of it.
    fn keep_session(&mut self) -> io::Result<()> {
        let session = self.session()?;
        if self.known.sessions.iter().any(|held| held.id == session.id) {
            return Ok(());
        }
        session.validate()?;
        let session = session.clone();
        self.known.sessions.push(session.clone());
        self.pending.sessions.push(session);
        Ok(())
    }

    fn keep_branch(&mut self, start: BranchStart) -> io::Result<()> {
        start.validate()?;
        if !self.known.sessions.iter().any(|session| session.id == start.session) {
            return Err(invalid("branch refers to an unknown session"));
        }
        match self.known.branches.get(&start.id) {
            Some(held) if *held != start => Err(invalid("conflicting branch context")),
            Some(_) => Ok(()),
            None => {
                self.known.branches.insert(start.id, start.clone());
                self.pending.branches.push(start);
                Ok(())
            }
        }
    }

    /// Adds `record` to the next revision's records, once: a capture opened after a failure
    /// gives its session's branch starts, gaps and event losses again.
    fn keep(&mut self, record: spool::Record) -> io::Result<()> {
        self.keep_session()?;
        match record {
            spool::Record::SessionStart(_) | spool::Record::Frame(_) => Ok(()),
            spool::Record::BranchStart(start) => self.keep_branch(start),
            spool::Record::Gap(gap) => {
                gap.validate()?;
                if !self.known.gaps.contains(&gap) {
                    self.known.gaps.push(gap.try_clone()?);
                    self.pending.gaps.push(gap);
                }
                Ok(())
            }
            spool::Record::Control(control) => {
                if !self.known.sessions.iter().any(|session| session.id == control.session()) {
                    return Err(invalid("control has no matching session context"));
                }
                if let ControlRecord::PlayerCommands { session, packet } = &control
                    && let Some(phase) = &packet.phase
                {
                    phase.validate()?;
                    let campaign = &self.begun.as_ref().expect("a session was kept").0.campaign;
                    if phase.start.session != *session || phase.start.id != packet.branch || !phase.campaign.eq_ignore_ascii_case(campaign) {
                        return Err(invalid("command phase has conflicting campaign or branch context"));
                    }
                    self.keep_branch(phase.start.clone())?;
                }
                if matches!(control, ControlRecord::EventLoss { .. }) {
                    if self.known.losses.contains(&control) {
                        return Ok(());
                    }
                    self.known.losses.push(control.clone());
                }
                self.pending.controls.push(control);
                Ok(())
            }
        }
    }

    /// The run's next hour: applied, and written with its block once that fills. An hour of a
    /// branch that waits is not taken; one of a branch not written is passed over.
    fn hour(&mut self, witness: Option<Witness>, manifest: &[u8], mut sections: Sections<'_>, host: &mut dyn Host) -> io::Result<Taken> {
        let run = self.run.ok_or_else(|| invalid("An hour needs a run under way"))?;
        if let Some(witness) = witness {
            let known = self.branches.entry(run.branch).or_default();
            known.qpc.get_or_insert(witness.qpc);
            if let Some(signal) = witness.load_signal {
                known.load.get_or_insert(signal);
            }
        }
        let skipped = run.skipped
            || !run.started
                && run.hours == 0
                && match self.decide(run.branch, host, true, false)? {
                    Admit::Write => false,
                    Admit::Skip => true,
                    Admit::Wait => return Ok(Taken::Waits(self.asked(run.branch, true, false))),
                };
        let hour = run.first_hour.checked_add_unsigned(run.hours).ok_or_else(|| invalid("A run's hours pass 2^31"))?;
        let branch = run.branch;
        if skipped {
            let run = self.run.as_mut().expect("a run under way");
            run.skipped = true;
            run.hours += 1;
            return Ok(Taken::Moved);
        }
        if !run.started {
            self.start_run()?;
        }
        let manifest = Manifest::read(manifest, &self.columns)?;
        if manifest.branch != branch || manifest.hour != hour {
            return Err(invalid("A run's hours are continuous from its first, on its branch"));
        }
        let first = run.hours == 0;
        if self.block.is_none() {
            let _label = label("summaries");
            self.summaries.open(&mut self.store, hour);
            self.block = Some(OpenBlock {
                hours: 0,
                clock: Clock::default(),
                count: 0,
                strings: vec![0; COUNT_ROOM],
                first_string: self.store.strings().next(),
            });
        }
        {
            let _label = label("strings");
            let block = self.block.as_mut().expect("a block under way");
            for text in &manifest.appends {
                unit::put_varint(&mut block.strings, text.len() as u64);
                block.strings.extend_from_slice(text);
            }
            block.count += manifest.appends.len() as u64;
        }
        {
            let _label = label("store");
            let applying = Instant::now();
            self.adapter.begin_hour(&mut self.store, &manifest)?;
            for entry in &manifest.sections {
                let reading = Instant::now();
                let bytes = sections.next(entry.bytes)?;
                self.spent.journal += reading.elapsed();
                self.adapter.section(&mut self.store, entry, bytes)?;
            }
            if sections.left() != 0 {
                return Err(invalid("An hour holds bytes past its manifest's sections"));
            }
            self.adapter.applied(&mut self.store);
            self.spent.store += applying.elapsed();
        }
        let clock = ClockHour {
            flags: manifest.flags,
            total_ns: manifest.total_ns,
            pool_bytes: manifest.pool_bytes,
            dropped_frames: manifest.dropped_frames,
        };
        self.finish_hour(branch, hour, first, clock, witness, host)?;
        Ok(Taken::Moved)
    }

    /// An hour applied to the store ends: the derived tables' writers, the campaign check, the
    /// summaries and the clock; the store's hour end into the file; the host and the parts told;
    /// the block's end when it fills.
    fn finish_hour(&mut self, branch: BranchId, hour: Hour, first: bool, clock: ClockHour, witness: Option<Witness>, host: &mut dyn Host) -> io::Result<()> {
        if !self.defined && !self.known.facts {
            self.take_facts(host)?;
        }
        {
            let _label = label("writers");
            let clock = Instant::now();
            self.writers.hour(branch, hour, first, &mut self.store)?;
            let spent = clock.elapsed();
            self.writer_ns += spent.as_nanos() as u64;
            self.spent.writers += spent;
        }
        let campaign = &self.begun.as_ref().ok_or_else(|| invalid("A capture's records precede its start"))?.0.campaign;
        check_campaign(&self.store, campaign)?;
        {
            let _label = label("summaries");
            let clock = Instant::now();
            self.summaries.hour(&self.store, hour);
            self.spent.summaries += clock.elapsed();
        }
        let block = self.block.as_mut().expect("a block under way");
        block.clock.hour(&clock);
        block.hours += 1;
        let ends = block.hours == UNIT_ORDINALS;
        let ending = Instant::now();
        self.store.end_hour(self.file.as_mut().expect("a run under way writes its file"))?;
        self.adapter.moved(&mut self.store);
        self.spent.ending += ending.elapsed();
        self.run.as_mut().expect("a run under way").hours += 1;
        if let Some(witness) = witness {
            self.keep_session()?;
            let session = self.session()?.id;
            self.pending.controls.push(ControlRecord::ClockWitness {
                session,
                qpc: witness.qpc,
                hour,
                load_signal: witness.load_signal,
            });
        }
        let applied = Applied { store: &self.store, branch, hour };
        host.hour(&applied);
        for part in &mut self.parts {
            part.hour(&applied)?;
        }
        if first {
            self.adapter.shed(&mut self.store);
        }
        if ends {
            let clock = Instant::now();
            self.end_block()?;
            self.spent.blocks += clock.elapsed();
        }
        Ok(())
    }

    /// The run under way's first hour is taken: the file holds its start, and the store, the
    /// writers and the summaries start from empty tables.
    fn start_run(&mut self) -> io::Result<()> {
        let run = self.run.ok_or_else(|| invalid("A run needs its start"))?;
        self.keep_session()?;
        let start = self
            .known
            .branches
            .get(&run.branch)
            .ok_or_else(|| invalid("A run's branch has no start among its capture's records"))?;
        if run.first_hour < start.first_hour {
            return Err(invalid("frame precedes its branch start"));
        }
        self.file()?;
        let file = self.file.as_mut().expect("created");
        file.start_run(run.branch, run.epoch, run.first_hour)?;
        {
            let _label = label("store");
            self.store.begin_run(run.first_hour);
        }
        *self.frames.lock().unwrap_or_else(PoisonError::into_inner) = Vec::new();
        self.store.read_strings_from(Box::new(FileStrings::new(reading(&self.path)?, self.frames.clone())));
        self.writers.start_run();
        self.summaries.start_run();
        self.run.as_mut().expect("a run under way").started = true;
        Ok(())
    }

    /// The recording's file, created now when the capture's first hour or its last records
    /// need it.
    fn file(&mut self) -> io::Result<&mut FileWriter<File>> {
        if self.file.is_none() {
            let (identity, _) = self.begun.as_ref().ok_or_else(|| invalid("A capture's records precede its start"))?;
            let file = held().create_new(true).open(&self.path)?;
            state_file::remove_at(&self.state_path)?;
            let state = StateFile::open(&self.state_path)?;
            let mut writer = FileWriter::create(file, identity.clone(), self.kinds.len(), self.settings, Some(state))?;
            if let Some(encoder) = self.store.encoder() {
                writer.compress_on(encoder.frames());
            }
            writer.container().section(SectionKey::FACTS, sections::FACTS)?;
            for part in &mut self.parts {
                part.open(writer.container(), None)?;
            }
            self.file = Some(writer);
        }
        Ok(self.file.as_mut().expect("created"))
    }

    /// The block under way ends: its summaries, clock and strings, and its page, into the file;
    /// then the store lets go of what no live cell names.
    fn end_block(&mut self) -> io::Result<()> {
        let block = self.block.take().ok_or_else(|| invalid("No block is under way"))?;
        let run = self.run.ok_or_else(|| invalid("A block needs a run under way"))?;
        let ordinal = run.hours - block.hours;
        let last_hour = run.first_hour + (run.hours - 1) as i32;
        let (pieces, span) = {
            let _label = label("summaries");
            let file = self.file.as_mut().ok_or_else(|| invalid("A block needs its file"))?;
            self.summaries.finish(last_hour, ordinal / UNIT_ORDINALS, run.first_hour, file)?
        };
        let mut count = Vec::with_capacity(COUNT_ROOM);
        unit::put_varint(&mut count, block.count);
        let mut strings = block.strings;
        let from = COUNT_ROOM - count.len();
        strings[from..COUNT_ROOM].copy_from_slice(&count);
        let strings = &strings[from..];
        let clock = block.clock.finish();
        let file = self.file.as_mut().ok_or_else(|| invalid("A block needs its file"))?;
        let (head, span) = {
            let _label = label("file");
            file.end_block(BlockEnd { clock: &clock, strings: (strings, block.first_string), summaries: pieces, span })?
        };
        {
            let _label = label("summaries");
            self.summaries.written(&span);
        }
        self.frames.lock().unwrap_or_else(PoisonError::into_inner).extend(head.strings.into_iter().map(|chunk| (chunk.first, chunk.frame)));
        {
            let _label = label("store");
            self.store.end_block();
            self.adapter.shed(&mut self.store);
        }
        engine_allocation::labels::phase("block");
        self.hours += u64::from(block.hours);
        Ok(())
    }

    /// Ends the run under way: its last block, short, when one is under way, then its
    /// directory. A run passed over is reported.
    fn end_run(&mut self, host: &mut dyn Host) -> io::Result<()> {
        let Some(run) = self.run else {
            return Ok(());
        };
        if run.skipped {
            host.skipped(&self.asked(run.branch, false, false), u64::from(run.hours));
        } else if run.started && run.hours == 0 {
            // A run given up before its first hour ended wrote nothing.
            self.block = None;
            self.file.as_mut().ok_or_else(|| invalid("A run needs its file"))?.finish_run()?;
        } else if run.started {
            self.finish_written_run()?;
        }
        self.run = None;
        Ok(())
    }

    /// The run under way, which wrote an hour, ends: its last block, short, when one is under way,
    /// its summaries' tail and its directory.
    fn finish_written_run(&mut self) -> io::Result<()> {
        let run = self.run.ok_or_else(|| invalid("No run is under way"))?;
        let file = self.file.as_mut().ok_or_else(|| invalid("A run needs its file"))?;
        self.store.end_run(file)?;
        if self.block.is_some() {
            self.end_block()?;
        }
        let file = self.file.as_mut().expect("a run's file");
        let tail = {
            let _label = label("summaries");
            self.summaries.tail(run.first_hour, run.hours, file)?
        };
        file.tail(&tail)?;
        file.finish_run()
    }

    /// A segment ended, or with `finish` the capture: commits a revision holding everything
    /// taken so far when the recording exists or the capture ends with something kept.
    fn settle(&mut self, host: &mut dyn Host, finish: Option<Option<Stop>>) -> io::Result<Taken> {
        if finish.is_some() {
            // Nothing more names a load: every branch still waiting is decided now.
            let waiting: Vec<BranchId> = self
                .branches
                .iter()
                .filter(|(_, known)| known.admit.is_none() && !known.held.is_empty())
                .map(|(id, _)| *id)
                .collect();
            for branch in waiting {
                self.decide(branch, host, false, true)?;
            }
        }
        let held = self.branches.values().any(|known| !known.held.is_empty());
        if self.file.is_none() && (self.pending.is_empty() || finish.is_none()) {
            // No recording yet: its first hour creates it. Segments nothing is kept of go;
            // those with records kept or kept aside stay with their source.
            return Ok(match (&finish, self.pending.is_empty() && !held) {
                (Some(_), _) => {
                    self.finished = true;
                    Taken::Finished
                }
                (None, true) => Taken::Settled,
                (None, false) => Taken::Moved,
            });
        }
        self.commit(host, finish.as_ref())?;
        Ok(match finish {
            Some(_) => Taken::Finished,
            None => Taken::Settled,
        })
    }

    /// Commits the next revision: the blocks written since the last, the records kept since,
    /// the capture's cursor, the session's game facts when the recording carries none, and
    /// each part's extents, with the resume state of them all; with `finish`, the capture's
    /// last, which names no resume state.
    fn commit(&mut self, host: &mut dyn Host, finish: Option<&Option<Stop>>) -> io::Result<Revision> {
        self.keep_session()?;
        let (_, session) = self.begun.clone().expect("a session was kept");
        if let Some(Some(stop)) = finish
            && self.known.branches.contains_key(&stop.branch)
            && !self.known.stops.contains(&(self.source.clone(), self.capture))
        {
            self.known.stops.push((self.source.clone(), self.capture));
            self.pending.stops.push(CaptureStop {
                spool: self.source.clone(),
                capture: self.capture,
                branch: stop.branch,
                last_hour: stop.last_hour,
                reason: stop.reason.clone(),
            });
        }
        self.pending.cursors.push(Cursor {
            source: self.source.clone(),
            capture: self.capture,
            session: session.id,
            segments: self.segments,
            finished: finish.is_some(),
        });
        let saved = host.saves();
        let level = self.settings.level;
        if !self.known.facts
            && !self.facts_put
            && let Some(pack) = given_facts(host, &session.identity)
        {
            self.put_facts(pack, &session.identity)?;
        }
        let records = self.pending.encode(FormatVersion::CURRENT)?;
        let world = match finish {
            None => self.world_log_state()?,
            Some(_) => None,
        };
        let kept = self.records_state()?;
        let file = self.file()?;
        let container = file.container();
        let extent = container.put_zstd(Kind::Controls, Tags::default(), &records, level)?;
        container.append(SectionKey::RECORDS, extent)?;
        let mut entries = Vec::new();
        entries.extend(world);
        entries.push(kept);
        for part in &mut self.parts {
            entries.extend(part.revise(self.file.as_mut().expect("created").container())?);
        }
        let revision = match finish {
            None => self.file.as_mut().expect("created").revise_runs(Some(&entries), saved, |file| file.sync_data())?,
            Some(_) => {
                entries.retain(|entry| entry.owner.kind == SectionKind::FamilyAnswers);
                let file = self.file.take().expect("created");
                let (file, revision, written) = file.finish_runs(&entries, saved, |file| file.sync_data())?;
                file.sync_all()?;
                self.finished_written = Some(written);
                self.finished = true;
                revision
            }
        };
        self.known.facts |= std::mem::take(&mut self.facts_put);
        self.pending = Records::default();
        let committed = Committed {
            revision,
            saved,
            hours: std::mem::take(&mut self.hours),
            finished: finish.is_some(),
        };
        host.committed(&committed);
        Ok(revision)
    }

    /// Asks the host for the session's game facts, before an hour's writers run, while no
    /// definitions were handed: once they come, the writers are handed theirs from that hour on.
    /// A host is not asked when no writer declares any.
    fn take_facts(&mut self, host: &mut dyn Host) -> io::Result<()> {
        if self.writers.wanted().is_empty() {
            self.defined = true;
            return Ok(());
        }
        let identity = self.session()?.identity.clone();
        if let Some(pack) = given_facts(host, &identity) {
            self.put_facts(pack, &identity)?;
        }
        Ok(())
    }

    /// Puts the session's game facts pack into the file for the next revision to name, and hands
    /// the writers their definitions read from it when none were handed yet: the pack is let go
    /// of here, never held until a commit.
    /// A pack a host gives at its place in a file is read and copied a piece at a time.
    fn put_facts(&mut self, pack: GivenFacts, identity: &Identity) -> io::Result<()> {
        // Stored as it is: a pack is compressed within, and read at its offsets.
        let extent = match pack {
            GivenFacts::Held(pack) => {
                let pack = Arc::new(pack);
                if !self.defined {
                    let _label = label("definitions");
                    self.writers.define_from(&recorder_derive::Facts::held(Arc::clone(&pack)), identity);
                    self.defined = true;
                }
                self.file()?.container().put(Kind::Facts, Tags::default(), pack.len() as u64, &pack[..])?
            }
            GivenFacts::Placed(FactsFile { mut file, offset, length }) => {
                if !self.defined {
                    let _label = label("definitions");
                    self.writers.define_from(&recorder_derive::Facts::stored(file.try_clone()?, offset, length), identity);
                    self.defined = true;
                }
                file.seek(SeekFrom::Start(offset))?;
                let extent = self.file()?.container().put_from(Kind::Facts, Tags::default(), length, &mut file, length)?;
                if extent.stored != length {
                    return Err(invalid("A game facts pack was not copied whole"));
                }
                extent
            }
        };
        self.file()?.container().append(SectionKey::FACTS, extent)?;
        self.facts_put = true;
        Ok(())
    }

    /// Whether the recording it writes carries game facts.
    pub fn carries_facts(&self) -> bool {
        self.known.facts
    }

    /// Ends the capture after what its source holds, its producer having stopped without
    /// finishing it: the recording says its hours stop after its last, for `reason`, and where
    /// its source's records ended short of its bytes. The source lets go of the capture.
    pub fn finish_stopped(&mut self, source: &mut dyn Source, host: &mut dyn Host, reason: &str) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        let damage = source.damage()?;
        // What the open segment applied stands: its blocks are written whole, its last short.
        self.end_run(host)?;
        if self.begun.is_none() {
            // Nothing of it was ever read: there is no capture to end.
            self.finished = true;
            return Ok(());
        }
        if let Some((segment, valid_bytes)) = damage {
            self.keep_session()?;
            self.pending.notes.push(RecoveryNote {
                session: Some(self.session()?.id),
                segment,
                valid_bytes,
                reason: "the capture's records end before its journal does".to_owned(),
            });
        }
        let last = self.file.as_ref().and_then(FileWriter::last);
        let stop = Stop {
            branch: last.map_or(0, |(branch, _)| branch),
            last_hour: last.map(|(_, hour)| hour),
            reason: reason.to_owned(),
        };
        self.segments = self.segments.saturating_add(1);
        self.settle(host, Some(Some(stop)))?;
        source.finish()
    }

    /// Adds `note` to the records the next committed revision holds, once; a recording not
    /// created yet holds it from its first revision.
    pub fn note(&mut self, note: RecoveryNote) -> io::Result<()> {
        if note.reason.trim().is_empty() {
            return Err(invalid("empty recovery reason"));
        }
        self.pending.notes.push(note);
        Ok(())
    }

    /// Commits a revision that saves everything the recording holds; none when it holds no
    /// revision that is not saved, or does not exist.
    pub fn save(&mut self) -> io::Result<Option<Revision>> {
        let Some(file) = &mut self.file else {
            return Ok(None);
        };
        let container = file.container();
        if container.revision() == 0 || container.saved().is_some_and(|saved| saved.number == container.revision()) {
            return Ok(None);
        }
        let world = self.world_log_state()?;
        let mut entries = Vec::new();
        entries.extend(world);
        entries.push(self.records_state()?);
        let file = self.file.as_mut().expect("held");
        file.revise_runs(Some(&entries), true, |file| file.sync_data()).map(Some)
    }
}

#[cfg(test)]
mod tests;
