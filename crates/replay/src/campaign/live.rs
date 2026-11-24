use replay_journal::summaries::Summaries;

use replay_recording::records::{CaptureStop, Cursor, Records, RecoveryNote};
use replay_data::{BranchId, Hour, codec::Decoder, format::FormatVersion, identity::{Identity, IdentityCodec, SessionStart}, scope::CaptureMode};
use replay_frame::branches::{BranchStart, Gap};
use replay_journal::{columnar_file::{BlockEnd, FileStrings, FileWriter, OpenRun, Resumed}, columnar_journal::{Record, Sections, Source, Witness}};
use replay_recording::{capture::{Capture, Stop}, columnar_frames::Recording, schema::{keeps_last, schema}, control::ControlRecord};
use replay_settings::compression::Settings;
use replay_capture_spool::spool;
use recorder_derive_framework::framework::{Definitions, Facts, Writers};
use replay_columnar::{
    container::{self, ContainerWriter, Open, Revision, SectionKey, SectionKind, StateEntry, StateFile, Tags, schema as sections, state_file},
    envelope::{self, Kind, SPAN_BLOCKS, UNIT_ORDINALS},
    lane::{Clock, ClockHour},
    leaf::{Reader, Writer as Bytes},
    manifest::{Manifest, Status},
    state::Kind as Column,
    store::{Store, TableState, journal::Journal as Adapter},
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

fn label(name: &str) -> engine_allocation::labels::Scope {
    use engine_allocation::labels;
    labels::enter(labels::owner(name), labels::Kind::Operator)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    Write,
    Skip,
    Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Asked {
    pub branch: BranchId,
    pub load: Option<i64>,
    pub qpc: Option<u64>,
    pub hour: bool,
    pub last: bool,
}

pub struct Applied<'a> {
    pub store: &'a Store,
    pub branch: BranchId,
    pub hour: Hour,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    pub revision: Revision,
    pub saved: bool,
    pub hours: u64,
    pub finished: bool,
}

pub struct FactsFile {
    pub file: File,
    pub offset: u64,
    pub length: u64,
}

enum GivenFacts {
    Held(Vec<u8>),
    Placed(FactsFile),
}

fn given_facts(host: &mut dyn Host, identity: &Identity) -> Option<GivenFacts> {
    host.facts_file(identity).map(GivenFacts::Placed).or_else(|| host.facts(identity).map(GivenFacts::Held))
}

pub trait Host {
    fn admit(&mut self, asked: &Asked) -> Admit;

    fn saves(&mut self) -> bool;

    fn facts(&mut self, identity: &Identity) -> Option<Vec<u8>> {
        let _ = identity;
        None
    }

    fn facts_file(&mut self, identity: &Identity) -> Option<FactsFile> {
        let _ = identity;
        None
    }

    fn facts_packing(&mut self) -> bool {
        false
    }

    fn hour(&mut self, hour: &Applied<'_>) {
        let _ = hour;
    }

    fn skipped(&mut self, asked: &Asked, hours: u64) {
        let _ = (asked, hours);
    }

    fn committed(&mut self, committed: &Committed) {
        let _ = committed;
    }
}

pub trait Part: Send {
    fn open(
        &mut self,
        container: &mut ContainerWriter<File>,
        state: Option<&StateEntry>,
    ) -> io::Result<()>;

    fn hour(&mut self, hour: &Applied<'_>) -> io::Result<()>;

    fn revise(&mut self, container: &mut ContainerWriter<File>) -> io::Result<Option<StateEntry>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taken {
    Moved,
    Settled,
    Waits(Asked),
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Waiting,
    Paused,
    Undecided(Asked),
    Finished,
}

const WORLD_LOG_STATE: u32 = 7;
const RECORDS_STATE: u32 = 1;

#[derive(Default)]
struct Branch {
    load: Option<i64>,
    bootstrap: bool,
    qpc: Option<u64>,
    admit: Option<bool>,
    held: Vec<spool::Record>,
}

impl Branch {
    fn load(&self) -> Option<i64> {
        if self.bootstrap { Some(0) } else { self.load }
    }
}

#[derive(Clone, Copy)]
struct Run {
    branch: BranchId,
    epoch: u8,
    first_hour: Hour,
    hours: u32,
    skipped: bool,
    started: bool,
}

#[derive(Default)]
struct Known {
    sessions: Vec<SessionStart>,
    branches: BTreeMap<BranchId, BranchStart>,
    gaps: Vec<Gap>,
    losses: Vec<ControlRecord>,
    stops: Vec<(String, u32)>,
    facts: bool,
}

struct OpenBlock {
    hours: u32,
    clock: Clock,
    count: u64,
    strings: Vec<u8>,
    first_string: u32,
}

const COUNT_ROOM: usize = 10;

fn rebuilt(source: &dyn reader::Extents, pages: &[unit::BlockPage], kinds: &[Column], table: usize, run_first: i32) -> io::Result<TableState> {
    let log = reader::class_of(pages, table)?;
    if log != replay_data::rows::logs()[table] {
        let (file, build) = if log { ("a log", "state") } else { ("state", "a log") };
        return Err(invalid(format!("{:?} was recorded as {file} and this build holds it as {build}", replay_data::rows::TableId::ALL[table])));
    }
    if log {
        reader::log_state(source, pages, kinds, table, pages.len() - 1)
    } else {
        Ok(reader::rebuild(source, pages, kinds, table, pages.len() - 1, run_first)?.0)
    }
}

fn check_campaign(store: &Store, campaign: &[u8]) -> io::Result<()> {
    let table = replay_data::rows::TableId::Campaign.index();
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

fn reading(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};
    fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

type Frames = Arc<Mutex<Vec<(u32, FrameEntry)>>>;

#[derive(Clone, Copy, Debug, Default)]
pub struct Spent {
    pub journal: std::time::Duration,
    pub store: std::time::Duration,
    pub writers: std::time::Duration,
    pub summaries: std::time::Duration,
    pub ending: std::time::Duration,
    pub blocks: std::time::Duration,
}

pub struct LiveWriter {
    path: PathBuf,
    state_path: PathBuf,
    settings: Settings,
    source: String,
    capture: u32,
    known: Known,
    file: Option<FileWriter<File>>,
    begun: Option<(envelope::Identity, SessionStart)>,
    run: Option<Run>,
    context: Option<BranchId>,
    branches: BTreeMap<BranchId, Branch>,
    pending: Records,
    segments: u32,
    hours: u64,
    finished: bool,
    parts: Vec<Box<dyn Part>>,
    kinds: Vec<Vec<Column>>,
    columns: Vec<usize>,
    store: Store,
    adapter: Adapter,
    writers: Writers,
    writer_ns: u64,
    summaries: Summaries,
    block: Option<OpenBlock>,
    frames: Frames,
    spent: Spent,
    finished_written: Option<replay_journal::columnar_file::Written>,
    facts_put: bool,
    defined: bool,
}

impl LiveWriter {
    pub fn open_with(
        path: &Path,
        state_path: &Path,
        source: &str,
        capture: u32,
        settings: Settings,
        parts: Vec<Box<dyn Part>>,
        writers: Writers,
    ) -> io::Result<Self> {
        let kinds = schema()?;
        let columns = kinds.iter().map(Vec::len).collect();
        let derived = writers.tables();
        let mut store = {
            let _label = label("store");
            Store::new(kinds.clone(), keeps_last().to_vec())
        };
        store.declare_logs(&replay_data::rows::logs())?;
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
            adapter: Adapter::leaving(&derived, replay_recording::schema::names()),
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

    pub fn segments(&self) -> u32 {
        self.segments
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn moved(&mut self, path: &Path) {
        self.path = path.to_path_buf();
    }

    pub fn created(&self) -> bool {
        self.file.is_some() || self.finished && self.path.exists()
    }

    pub fn saved(&self) -> Option<Revision> {
        self.file.as_ref().and_then(FileWriter::saved)
    }

    pub fn derive_ns(&self) -> (u64, u64) {
        (self.writer_ns, self.summaries.ns)
    }

    pub fn writer_spent(&self) -> Vec<(&'static str, u64)> {
        self.writers.spent()
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn time(&mut self, timed: bool) {
        if let Some(encoder) = self.store.encoder_mut() {
            encoder.time(timed);
        }
    }

    pub fn spent(&self) -> Spent {
        self.spent
    }

    pub fn written(&self) -> Option<replay_journal::columnar_file::Written> {
        self.file.as_ref().map(FileWriter::written).or(self.finished_written)
    }

    pub fn summary_bytes(&self) -> Vec<(usize, usize, usize, usize)> {
        self.summaries.table_bytes()
    }

    pub fn writer_blobs(&self) -> io::Result<Vec<(&'static str, usize)>> {
        self.writers.blob_sizes()
    }

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

    pub fn capture_mode(&self) -> Option<CaptureMode> {
        self.begun.as_ref().map(|(_, session)| session.capture_mode)
    }

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
        if let Some(extent) = log.facts_extents().last() {
            if !self.writers.wanted().is_empty() {
                let _label = label("definitions");
                match (reading(&self.path), self.known.sessions.last()) {
                    (Ok(file), Some(session)) => self.writers.define_from(&Facts::stored(file, extent.offset, extent.stored), &session.identity),
                    (Ok(_), None) => self.writers.define(Definitions::absent("the recording's game facts name no session")),
                    (Err(error), _) => self.writers.define(Definitions::absent(format!("the recording's game facts cannot be read: {error}"))),
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
        writer.record_derived_versions(self.kinds.len(), &self.writers.table_versions())?;
        for part in &mut self.parts {
            let owner = |entry: &&StateEntry| !matches!(entry.owner.kind, SectionKind::WorldLog | SectionKind::Records);
            part.open(writer.container(), entries.iter().find(owner))?;
        }
        self.file = Some(writer);
        Ok(())
    }

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
                .map_err(|error| invalid(format!("{:?} rebuilt from the file: {error}", replay_data::rows::TableId::ALL[table])))?;
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
            writer.record_derived_versions(self.kinds.len(), &self.writers.table_versions())?;
            for part in &mut self.parts {
                part.open(writer.container(), None)?;
            }
            self.file = Some(writer);
        }
        Ok(self.file.as_mut().expect("created"))
    }

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
            self.summaries.written(&span)?;
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

    fn end_run(&mut self, host: &mut dyn Host) -> io::Result<()> {
        let Some(run) = self.run else {
            return Ok(());
        };
        if run.skipped {
            host.skipped(&self.asked(run.branch, false, false), u64::from(run.hours));
        } else if run.started && run.hours == 0 {
            self.block = None;
            self.file.as_mut().ok_or_else(|| invalid("A run needs its file"))?.finish_run()?;
        } else if run.started {
            self.finish_written_run()?;
        }
        self.run = None;
        Ok(())
    }

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

    fn settle(&mut self, host: &mut dyn Host, finish: Option<Option<Stop>>) -> io::Result<Taken> {
        if finish.is_some() {
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

    fn put_facts(&mut self, pack: GivenFacts, identity: &Identity) -> io::Result<()> {
        let extent = match pack {
            GivenFacts::Held(pack) => {
                let pack = Arc::new(pack);
                if !self.defined {
                    let _label = label("definitions");
                    self.writers.define_from(&Facts::held(Arc::clone(&pack)), identity);
                    self.defined = true;
                }
                self.file()?.container().put(Kind::Facts, Tags::default(), pack.len() as u64, &pack[..])?
            }
            GivenFacts::Placed(FactsFile { mut file, offset, length }) => {
                if !self.defined {
                    let _label = label("definitions");
                    self.writers.define_from(&Facts::stored(file.try_clone()?, offset, length), identity);
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

    pub fn carries_facts(&self) -> bool {
        self.known.facts
    }

    pub fn finish_stopped(&mut self, source: &mut dyn Source, host: &mut dyn Host, reason: &str) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        let damage = source.damage()?;
        self.end_run(host)?;
        if self.begun.is_none() {
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

    pub fn note(&mut self, note: RecoveryNote) -> io::Result<()> {
        if note.reason.trim().is_empty() {
            return Err(invalid("empty recovery reason"));
        }
        self.pending.notes.push(note);
        Ok(())
    }

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
