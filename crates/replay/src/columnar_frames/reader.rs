//! A recording of format 12 read from its file or memory: its root, records, facts and every
//! run's directory held from open; a span's directory and each table's row of it read when a
//! reader first needs them and kept; block pages, a table's entries, its summary pieces, a
//! block's clock and strings read on demand. The unit's reader primitives
//! ([`replay_columnar::unit::reader`]) read the frames these name through [`Recording::extents`].

use super::{
    Segment, columns_at,
    input::{Held, Source},
    invalid, schema, withheld_at,
};
use crate::rows::{TABLE_COUNT, TableId};
use replay_columnar::{
    container::{self, Open, PAGE_RAW_BYTES, Revision, Root, Saved, SectionKey, check_follows, check_owner, read_page, schema as sections_schema},
    envelope::{self, Extent, Kind, SPAN_BLOCKS, UNIT_ORDINALS},
    lane::{Clock, ClockHour},
    pages::{Block, BlockHead, Entry, RunDirectory, SpanDirectory, SpanRow},
    unit::{BlockPage, FrameEntry, TableEntry, reader::Extents},
};
use std::{
    collections::HashMap,
    fmt,
    fs::File,
    io,
    path::Path,
    sync::{Arc, Mutex, PoisonError},
};

/// What a piece of damage refuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DamageScope {
    /// The recording: no committed root can be read.
    Unlocated,
    /// A run's directory.
    Run(u32),
    /// A run's block page or span page, by its first block.
    Page { run: u32, block: u32 },
    /// A block's clock or strings, or a summary piece.
    Extent(Extent),
    /// One table's entries, parts or restore in a block.
    Table { run: u32, block: u32, table: u32 },
    /// A table the recording's format holds in a layout or meaning this build reads otherwise.
    Withheld { table: u32 },
}

impl fmt::Display for DamageScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unlocated => write!(f, "The recording"),
            Self::Run(run) => write!(f, "Run {run}'s directory"),
            Self::Page { run, block } => write!(f, "Run {run}'s page of block {block}"),
            Self::Extent(extent) => write!(f, "{:?} extent of run {} stream {} from ordinal {} ({} hours)", extent.kind, extent.run, extent.stream, extent.first, extent.hours),
            Self::Table { run, block, table } => write!(f, "{} of run {run} in block {block}", table_name(*table)),
            Self::Withheld { table } => write!(f, "{} of the recording", table_name(*table)),
        }
    }
}

/// A schema table's name, for damage naming it.
pub fn table_name(table: u32) -> String {
    TableId::ALL.get(table as usize).map_or_else(|| format!("Table {table}"), |id| format!("{id:?}"))
}

/// Recorded bytes that cannot be read, and what they refuse.
#[derive(Debug)]
pub struct Damage {
    pub scope: DamageScope,
    cause: io::Error,
}

impl fmt::Display for Damage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.scope, self.cause)
    }
}

impl std::error::Error for Damage {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

impl Damage {
    /// `cause` as damage refusing `scope`, unless it names its own scope already or is no
    /// damage: an I/O failure or a lack of memory goes on as it is.
    pub fn wrap(scope: DamageScope, cause: io::Error) -> io::Error {
        if cause.kind() != io::ErrorKind::InvalidData || Self::of(&cause).is_some() {
            return cause;
        }
        io::Error::new(io::ErrorKind::InvalidData, Self { scope, cause })
    }

    /// The damage `error` is.
    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref()?.downcast_ref()
    }
}

/// A run whose directory can be read: its place in the campaign and its pages.
pub struct Run {
    pub branch: u64,
    /// Why the run starts ([`envelope::epoch`]).
    pub epoch: u8,
    pub first_hour: i32,
    pub ordinals: u32,
    /// The chain after its last hour.
    pub chain: [u8; 32],
    directory: RunDirectory,
}

impl Run {
    /// Its blocks.
    pub fn pages(&self) -> usize {
        self.directory.blocks() as usize
    }

    /// Block `at`'s first ordinal and hours.
    pub fn block(&self, at: usize) -> (u32, u32) {
        let first = at as u32 * UNIT_ORDINALS;
        (first, self.ordinals.saturating_sub(first).min(UNIT_ORDINALS))
    }

    /// The game hour of block `at`'s first hour.
    pub fn block_first_hour(&self, at: u32) -> i32 {
        self.first_hour + (at * UNIT_ORDINALS) as i32
    }

    /// Run ordinal `ordinal`'s block and its hour in the block.
    pub fn place(&self, ordinal: u32) -> (u32, u16) {
        (ordinal / UNIT_ORDINALS, (ordinal % UNIT_ORDINALS) as u16)
    }

    /// The ordinal of game hour `hour`; none outside the run.
    pub fn ordinal(&self, hour: i32) -> Option<u32> {
        u32::try_from(i64::from(hour) - i64::from(self.first_hour)).ok().filter(|&ordinal| ordinal < self.ordinals)
    }

    pub fn last_hour(&self) -> i32 {
        self.first_hour + self.ordinals as i32 - 1
    }

    /// Its directory as read.
    pub fn directory(&self) -> &RunDirectory {
        &self.directory
    }
}

/// One closed span's directory, where its index lies, and each table's row of it as parsed: a
/// row is read alone from the index, which is stored as it is.
struct SpanHeld {
    directory: SpanDirectory,
    index: Extent,
    rows: Mutex<Vec<Option<Arc<SpanRow>>>>,
}

/// The pages read so far, shared by the recordings selecting from one another.
#[derive(Default)]
struct Pages {
    spans: Mutex<HashMap<(u32, u32), Arc<SpanHeld>, foldhash::fast::RandomState>>,
    open: Mutex<HashMap<(u32, u32), Arc<Block>, foldhash::fast::RandomState>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What `root`'s recording holds of each schema table: its columns, and the tables this build
/// reads otherwise, each with why. Of this build's schema, every column and none; else of the
/// row format its records name, when its identity names that format's schema as each table was
/// framed then: the columns appended since hold no recorded cell, and the tables whose layout or
/// meaning changed since are withheld by name.
fn recorded_columns(root: &Root, format: impl FnOnce() -> io::Result<Option<u16>>) -> io::Result<(Vec<usize>, Vec<(usize, String)>)> {
    let schema = schema()?;
    let ours = envelope::schema_digest(&schema);
    if root.identity.schema == ours {
        return Ok((schema.iter().map(Vec::len).collect(), Vec::new()));
    }
    let hex = |digest: &[u8; 32]| digest[..8].iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let differs = |also: &str| invalid(format!("This recording was written with another schema (its digest {}, this build's {}){also}", hex(&root.identity.schema), hex(&ours)));
    let format = match format() {
        Ok(Some(format)) => format,
        Ok(None) => return Err(differs("")),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Err(differs(&format!("; its records cannot be read: {error}"))),
        Err(error) => return Err(error),
    };
    let columns = columns_at(format)?;
    if envelope::schema_digest_prefix(&schema, &columns) != Some(root.identity.schema) {
        return Err(differs(""));
    }
    Ok((columns, withheld_at(format)?))
}

/// A recording of format 12 ([`replay_columnar::pages`]): its root and every run's directory,
/// held from open; its pages read as readers need them. A run directory that cannot be read
/// refuses only what it covers ([`Self::damage`]); the rest stays readable.
pub struct Recording {
    source: Arc<Source>,
    root: Root,
    revision: Revision,
    saved: Option<Revision>,
    records: Box<[Extent]>,
    facts: Box<[Extent]>,
    directories: Box<[Extent]>,
    /// Each schema table's columns it holds: the first of this build's.
    columns: Box<[usize]>,
    /// The tables its format holds as this build reads them otherwise, each with why.
    withheld: Box<[(usize, String)]>,
    runs: Box<[Option<Arc<Run>>]>,
    /// The segments its readers take ([`Self::with_segments`]), and those it records: one a run.
    segments: Box<[Segment]>,
    recorded: Box<[Segment]>,
    damage: Box<[Damage]>,
    /// The tables its readers read.
    selected: [bool; TABLE_COUNT],
    /// The columns of each table its readers read; every one when none.
    projected: Option<Arc<Vec<Option<Vec<usize>>>>>,
    pages: Arc<Pages>,
}

impl Recording {
    /// The newest saved revision of the recording `bytes` hold.
    pub fn open(bytes: Vec<u8>) -> io::Result<Self> {
        Self::read(Held::Memory(bytes), Open::Saved)
    }

    /// The newest saved revision of the recording at `path`.
    pub fn open_path(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_path_as(path, Open::Saved)
    }

    /// The revision of the recording at `path` that `open` names.
    pub fn open_path_as(path: impl AsRef<Path>, open: Open) -> io::Result<Self> {
        Self::read(Held::File(File::open(path)?), open)
    }

    /// The revision of the recording in `file` that `open` names.
    pub fn open_file(file: File, open: Open) -> io::Result<Self> {
        Self::read(Held::File(file), open)
    }

    /// The revision of the recording `bytes` hold that `open` names.
    pub fn open_as(bytes: Vec<u8>, open: Open) -> io::Result<Self> {
        Self::read(Held::Memory(bytes), open)
    }

    fn read(held: Held, open: Open) -> io::Result<Self> {
        let unlocated = |error: io::Error| Damage::wrap(DamageScope::Unlocated, error);
        let (source, root_extent) = Source::open(held, open).map_err(unlocated)?;
        let root = Root::read(&container::decompress(&source.stored(&root_extent).map_err(unlocated)?, None).map_err(unlocated)?).map_err(unlocated)?;
        let revision = Revision { number: root.revision, length: source.len(), root: root_extent.digest };
        let saved = match root.saved {
            Saved::Yes => Some(revision),
            Saved::No(before) => before,
        };
        let mut recording = Self {
            source: Arc::new(source),
            root,
            revision,
            saved,
            records: Box::default(),
            facts: Box::default(),
            directories: Box::default(),
            columns: Box::default(),
            withheld: Box::default(),
            runs: Box::default(),
            segments: Box::default(),
            recorded: Box::default(),
            damage: Box::default(),
            selected: [true; TABLE_COUNT],
            projected: None,
            pages: Arc::default(),
        };
        recording.records = recording.section(SectionKey::RECORDS, sections_schema::RECORDS).map_err(unlocated)?.into();
        recording.facts = recording.section(SectionKey::FACTS, sections_schema::FACTS).map_err(unlocated)?.into();
        recording.directories = recording.section(SectionKey::WORLD_LOG, sections_schema::WORLD_LOG).map_err(unlocated)?.into();
        let (columns, withheld) = recorded_columns(&recording.root, || {
            // Every revision's records name the row format: a campaign's in each extent, a
            // capture's in its record, the last.
            let Some(last) = recording.records.last() else { return Ok(None) };
            let raw = recording.raw(last)?;
            if let Some(format) = crate::campaign::Records::format_of(&raw) {
                return Ok(Some(format));
            }
            Ok(Some(crate::columnar_file::Capture::read(&raw)?.format))
        })?;
        recording.columns = columns.into();
        recording.withheld = withheld.into();
        let mut damage = Vec::new();
        let mut runs = Vec::with_capacity(recording.directories.len());
        let mut segments = Vec::new();
        for (index, extent) in (0..).zip(&*recording.directories) {
            match recording.raw(extent).and_then(|raw| RunDirectory::read(&raw, extent, index)) {
                Ok(directory) => {
                    if directory.hours > 0 {
                        segments.push(Segment { branch: directory.branch, run: index, restart: None, first: 0, last: directory.hours - 1 });
                    }
                    runs.push(Some(Arc::new(Run {
                        branch: directory.branch,
                        epoch: directory.epoch,
                        first_hour: directory.first_hour,
                        ordinals: directory.hours,
                        chain: directory.chain,
                        directory,
                    })));
                }
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    damage.push(Damage { scope: DamageScope::Run(index), cause: error });
                    runs.push(None);
                }
                Err(error) => return Err(error),
            }
        }
        recording.runs = runs.into();
        recording.recorded = segments.clone().into();
        recording.segments = segments.into();
        recording.damage = damage.into();
        Ok(recording)
    }

    /// Every extent of section `key`, refused unless it is of `version`: its frozen ones read
    /// down through their index pages, then its open ones.
    fn section(&self, key: SectionKey, version: u32) -> io::Result<Vec<Extent>> {
        let Some(section) = self.root.open(key, version)? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for (level, held) in section.frozen.levels().iter().enumerate().rev() {
            for extent in held {
                if level == 0 {
                    check_follows(key, out.last(), extent, self.len())?;
                    out.push(*extent);
                } else {
                    self.expand(key, extent, &mut out)?;
                }
            }
        }
        out.extend_from_slice(&section.open);
        for (at, extent) in (0_u64..).zip(&out) {
            check_owner(key, at, extent)?;
        }
        Ok(out)
    }

    fn expand(&self, key: SectionKey, page: &Extent, out: &mut Vec<Extent>) -> io::Result<()> {
        if page.raw > PAGE_RAW_BYTES {
            return Err(page.damaged(&format!("is larger than any index page of {key}")));
        }
        for entry in read_page(&self.raw(page)?, page, key)? {
            if page.stream == 1 {
                check_follows(key, out.last(), &entry, self.len())?;
                out.push(entry);
            } else {
                self.expand(key, &entry, out)?;
            }
        }
        Ok(())
    }

    /// The recording as read from the same file, its pages shared with this one, whose readers
    /// read only `tables` ([`Self::select`]).
    pub fn selecting(&self, tables: &[TableId]) -> io::Result<Self> {
        let damage = self.damage.iter().map(|held| Damage { scope: held.scope.clone(), cause: io::Error::new(held.cause.kind(), held.cause.to_string()) }).collect();
        let mut recording = Self {
            source: Arc::clone(&self.source),
            root: self.root.clone(),
            revision: self.revision,
            saved: self.saved,
            records: self.records.clone(),
            facts: self.facts.clone(),
            directories: self.directories.clone(),
            columns: self.columns.clone(),
            withheld: self.withheld.clone(),
            runs: self.runs.clone(),
            segments: self.segments.clone(),
            recorded: self.recorded.clone(),
            damage,
            selected: [false; TABLE_COUNT],
            projected: None,
            pages: Arc::clone(&self.pages),
        };
        recording.select(tables);
        Ok(recording)
    }

    /// Its readers read only `tables`.
    pub fn select(&mut self, tables: &[TableId]) {
        self.selected = [false; TABLE_COUNT];
        for table in tables {
            self.selected[table.index()] = true;
        }
    }

    /// Its readers read only `columns` of `table`: the others' cells read as not recorded.
    pub fn project(&mut self, table: TableId, columns: &[usize]) -> io::Result<()> {
        let mut projected = match self.projected.take() {
            Some(projected) => Arc::unwrap_or_clone(projected),
            None => vec![None; TABLE_COUNT],
        };
        let count = self.columns[table.index()];
        let mut wanted: Vec<usize> = columns.iter().copied().filter(|&column| column < count).collect();
        wanted.sort_unstable();
        wanted.dedup();
        projected[table.index()] = Some(wanted);
        self.projected = Some(Arc::new(projected));
        Ok(())
    }

    /// Whether its readers read `table`.
    pub fn selects(&self, table: usize) -> bool {
        self.selected.get(table).copied().unwrap_or(false)
    }

    /// The columns of `table` its readers read: those it projects to, else every one it records.
    pub fn read_columns(&self, table: usize) -> Vec<usize> {
        match self.projected.as_ref().and_then(|projected| projected.get(table)?.as_ref()) {
            Some(columns) => columns.clone(),
            None => (0..self.columns.get(table).copied().unwrap_or(0)).collect(),
        }
    }

    /// The tables it holds in a layout or meaning this build reads otherwise, each with why.
    pub fn withheld(&self) -> &[(usize, String)] {
        &self.withheld
    }

    /// Why `table` is withheld, when it is.
    pub fn withheld_reason(&self, table: usize) -> Option<&str> {
        self.withheld.iter().find(|(held, _)| *held == table).map(|(_, reason)| reason.as_str())
    }

    /// The campaign its identity names.
    pub fn campaign(&self) -> &[u8] {
        &self.root.identity.campaign
    }

    /// Each schema table's columns it holds: the first of this build's.
    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    /// The content identity its root names, decoded.
    pub fn identity(&self) -> io::Result<crate::identity::Identity> {
        crate::campaign::decode_identity(&self.root.identity.content)
    }

    /// The identity its root names.
    pub fn root_identity(&self) -> &envelope::Identity {
        &self.root.identity
    }

    /// Its campaign records, every revision's in order.
    pub fn records(&self) -> io::Result<crate::campaign::Records> {
        let mut records = crate::campaign::Records::default();
        for extent in &*self.records {
            records.decode_into(&self.raw(extent)?)?;
        }
        Ok(records)
    }

    /// The capture the file's runs belong to, as its records hold it; none when they hold none.
    pub fn capture(&self) -> io::Result<Option<crate::columnar_file::Capture>> {
        if let Some(last) = self.records.last() {
            // A campaign's recording holds campaign records, not one capture's.
            if crate::campaign::Records::format_of(&self.raw(last)?).is_some() {
                return Ok(None);
            }
        }
        crate::columnar_file::read_capture(&self.records, |extent| self.raw(extent))
    }

    /// Why each run directory that cannot be read is refused.
    pub fn damage(&self) -> &[Damage] {
        &self.damage
    }

    /// Damage that leaves recorded hours where no branch places them: a run whose directory
    /// cannot be read, as an error naming it.
    pub fn unlocated(&self) -> Option<io::Error> {
        self.damage
            .iter()
            .find(|damage| matches!(damage.scope, DamageScope::Unlocated | DamageScope::Run(_)))
            .map(|damage| Damage::wrap(damage.scope.clone(), invalid(damage.cause.to_string())))
    }

    /// Recorded hours over every run that can be read.
    pub fn hours(&self) -> u64 {
        self.runs.iter().flatten().map(|run| u64::from(run.ordinals)).sum()
    }

    /// Runs in the recording, readable or not.
    pub fn run_count(&self) -> u32 {
        self.runs.len() as u32
    }

    /// Run `run`, when its directory can be read.
    pub fn run(&self, run: u32) -> io::Result<&Run> {
        match self.runs.get(run as usize) {
            Some(Some(held)) => Ok(held),
            Some(None) => Err(Damage::wrap(DamageScope::Run(run), invalid("cannot be read"))),
            None => Err(invalid(format!("The recording holds no run {run}"))),
        }
    }

    /// Every run's directory, in run order; refused when one cannot be read.
    pub fn directories(&self) -> io::Result<Vec<&RunDirectory>> {
        (0..self.run_count()).map(|run| self.run(run).map(Run::directory)).collect()
    }

    /// The segments its readers take: one a readable run ([`Self::recorded_segments`]), unless
    /// given others ([`Self::with_segments`]).
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn recorded_segments(&self) -> &[Segment] {
        &self.recorded
    }

    /// The recording whose readers take `segments` in place of those it records, as a timeline
    /// reads them: each one a run's hours, of any branch, its first and last ordinals within the
    /// run's. A segment may appear more than once.
    pub fn with_segments(mut self, segments: Vec<Segment>) -> io::Result<Self> {
        for segment in &segments {
            let held = self.recorded.iter().find(|held| held.run == segment.run);
            if !held.is_some_and(|held| segment.restart.is_none() && segment.first <= segment.last && segment.last <= held.last) {
                return Err(invalid("A segment the recording does not hold"));
            }
        }
        self.segments = segments.into();
        Ok(self)
    }

    /// The root its footer names.
    pub fn root(&self) -> &Root {
        &self.root
    }

    /// The revision opened.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The newest saved revision: this one, or the one its root names; none while none is saved.
    pub fn saved(&self) -> Option<Revision> {
        self.saved
    }

    /// The digest of the root its footer names, which names its revision.
    pub fn root_footer_digest(&self) -> io::Result<[u8; 16]> {
        Ok(self.revision.root)
    }

    /// Its campaign records' extents, in order.
    pub fn records_extents(&self) -> &[Extent] {
        &self.records
    }

    /// Its game facts packs' extents, oldest first.
    pub fn facts_extents(&self) -> &[Extent] {
        &self.facts
    }

    /// Its run directories' extents, in run order.
    pub fn run_directories(&self) -> &[Extent] {
        &self.directories
    }

    /// The file's length through the footer of the revision opened.
    pub fn len(&self) -> u64 {
        self.source.len()
    }

    /// Its bytes, when it was opened from memory.
    pub fn memory(&self) -> Option<&[u8]> {
        self.source.memory()
    }

    /// Where the unit's reader primitives read its frames from.
    pub fn extents(&self) -> &(dyn Extents + 'static) {
        &*self.source
    }

    /// Its source, shared: for a reader that outlives the borrow.
    pub fn source(&self) -> Arc<Source> {
        Arc::clone(&self.source)
    }

    /// The number of schema tables its pages name.
    pub fn tables(&self) -> usize {
        TABLE_COUNT
    }

    /// `out.len()` stored bytes of `extent` from `at` within it; its digest is the caller's to
    /// check, once, with [`Self::stored_bytes`].
    pub fn stored_at(&self, extent: &Extent, at: u64, out: &mut [u8]) -> io::Result<()> {
        self.source.stored_at(extent, at, out).map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))
    }

    /// `extent`'s stored bytes, checked against its digest.
    pub fn stored_bytes(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        self.source.stored(extent).map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))
    }

    /// `extent`'s bytes: as stored for one stored as it is (a summary piece, a span index, a
    /// facts pack), else decoded from its frame.
    pub fn raw(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        let stored = self.stored_bytes(extent)?;
        match extent.kind {
            Kind::Summary | Kind::SpanIndex | Kind::Facts | Kind::Stored => Ok(stored),
            _ => container::decompress(&stored, Some(extent.raw)).map_err(|error| Damage::wrap(DamageScope::Extent(*extent), extent.damaged(&error.to_string()))),
        }
    }

    /// `extent`, decoded: the bytes of a record, facts pack or other extent its root names.
    pub fn extent_bytes(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        self.raw(extent)
    }

    /// A unit frame's raw bytes: its stored bytes checked against its digest and decompressed.
    pub fn frame(&self, frame: &FrameEntry) -> io::Result<Vec<u8>> {
        let mut stored = Vec::new();
        self.source.read_at(frame.offset, frame.stored as usize, &mut stored)?;
        if envelope::digest(&stored) != frame.digest {
            return Err(invalid("A frame differs from its digest"));
        }
        container::decompress(&stored, Some(u64::from(frame.raw)))
    }

    /// Span `span` of run `run`: its directory and index, read once.
    fn span(&self, run: u32, span: u32) -> io::Result<Arc<SpanHeld>> {
        if let Some(held) = locked(&self.pages.spans).get(&(run, span)) {
            return Ok(Arc::clone(held));
        }
        let pages = *self.run(run)?.directory.spans.get(span as usize).ok_or_else(|| invalid(format!("Run {run} holds no span {span}")))?;
        let scope = DamageScope::Page { run, block: span * SPAN_BLOCKS };
        let read = || -> io::Result<SpanHeld> {
            let directory = SpanDirectory::read(&self.raw(&pages.directory)?, &pages.directory, TABLE_COUNT)?;
            if directory.first_block != span * SPAN_BLOCKS || directory.blocks.len() != SPAN_BLOCKS as usize {
                return Err(invalid("is not its run's span"));
            }
            if pages.index.stored != pages.index.raw {
                return Err(invalid("A span index is stored compressed"));
            }
            Ok(SpanHeld { directory, index: pages.index, rows: Mutex::new(vec![None; TABLE_COUNT]) })
        };
        let held = Arc::new(read().map_err(|error| Damage::wrap(scope, error))?);
        Ok(Arc::clone(locked(&self.pages.spans).entry((run, span)).or_insert(held)))
    }

    fn span_row(&self, run: u32, span: u32, held: &SpanHeld, table: usize) -> io::Result<Arc<SpanRow>> {
        if let Some(row) = locked(&held.rows).get(table).and_then(Option::as_ref) {
            return Ok(Arc::clone(row));
        }
        let parse = || -> io::Result<SpanRow> {
            let place = held.directory.rows.get(table).ok_or_else(|| invalid(format!("A span names no table {table}")))?;
            let mut bytes = engine_allocation::filled(place.length as usize, 0)?;
            self.source.stored_at(&held.index, place.offset, &mut bytes)?;
            if envelope::digest(&bytes) != place.digest {
                return Err(invalid("A span row differs from its digest"));
            }
            SpanRow::read(&bytes, run, table as u32, SPAN_BLOCKS)
        };
        let row = Arc::new(parse().map_err(|error| Damage::wrap(DamageScope::Table { run, block: span * SPAN_BLOCKS, table: table as u32 }, error))?);
        let mut rows = locked(&held.rows);
        Ok(Arc::clone(rows[table].get_or_insert(row)))
    }

    /// Open block page `block` of run `run`, read once.
    fn open_block(&self, run: u32, block: u32) -> io::Result<Arc<Block>> {
        if let Some(held) = locked(&self.pages.open).get(&(run, block)) {
            return Ok(Arc::clone(held));
        }
        let directory = &self.run(run)?.directory;
        let page = directory.open.get((block - directory.spans.len() as u32 * SPAN_BLOCKS) as usize).ok_or_else(|| invalid(format!("Run {run} holds no block {block}")))?;
        let read = Block::read(&self.raw(page)?, page, TABLE_COUNT).map_err(|error| Damage::wrap(DamageScope::Page { run, block }, error))?;
        let held = Arc::new(read);
        Ok(Arc::clone(locked(&self.pages.open).entry((run, block)).or_insert(held)))
    }

    fn check_block(&self, run: u32, block: u32) -> io::Result<()> {
        if block >= self.run(run)?.directory.blocks() {
            return Err(invalid(format!("Run {run} holds no block {block}")));
        }
        Ok(())
    }

    fn closed(&self, run: u32, block: u32) -> io::Result<bool> {
        Ok(block / SPAN_BLOCKS < self.run(run)?.directory.spans.len() as u32)
    }

    /// Block `block` of run `run`: its head and every table's entry.
    pub fn block(&self, run: u32, block: u32) -> io::Result<Block> {
        self.check_block(run, block)?;
        if !self.closed(run, block)? {
            return Ok((*self.open_block(run, block)?).clone());
        }
        let span = block / SPAN_BLOCKS;
        let held = self.span(run, span)?;
        let at = (block % SPAN_BLOCKS) as usize;
        let mut tables = Vec::with_capacity(TABLE_COUNT);
        for table in 0..TABLE_COUNT {
            tables.push(self.span_row(run, span, &held, table)?.blocks[at].clone());
        }
        Ok(Block { run, block, head: held.directory.blocks[at].clone(), tables })
    }

    /// The head of block `block` of run `run`.
    pub fn head(&self, run: u32, block: u32) -> io::Result<BlockHead> {
        self.check_block(run, block)?;
        if !self.closed(run, block)? {
            return Ok(self.open_block(run, block)?.head.clone());
        }
        Ok(self.span(run, block / SPAN_BLOCKS)?.directory.blocks[(block % SPAN_BLOCKS) as usize].clone())
    }

    /// Table `table`'s entry of block `block` of run `run`: what the unit's reader primitives
    /// read the table's parts, restore and statuses of that block through.
    pub fn table_entry(&self, run: u32, block: u32, table: usize) -> io::Result<Entry> {
        self.check_block(run, block)?;
        if !self.closed(run, block)? {
            return self.open_block(run, block)?.tables.get(table).cloned().ok_or_else(|| invalid(format!("A block names no table {table}")));
        }
        let span = block / SPAN_BLOCKS;
        let held = self.span(run, span)?;
        Ok(self.span_row(run, span, &held, table)?.blocks[(block % SPAN_BLOCKS) as usize].clone())
    }

    /// Table `table`'s summary piece of block `block` of run `run`, when it has one.
    pub fn block_of(&self, run: u32, block: usize, table: usize) -> io::Result<Option<Extent>> {
        Ok(self.table_entry(run, block as u32, table)?.summary)
    }

    /// Table `table`'s summary pieces of the levels above a block's that closed span `span` of
    /// run `run` holds, as (level, extent).
    pub fn span_summaries(&self, run: u32, span: u32, table: usize) -> io::Result<Vec<(u8, Extent)>> {
        let held = self.span(run, span)?;
        Ok(self.span_row(run, span, &held, table)?.summaries.clone())
    }

    /// Table `table`'s row of span `span` of run `run`: its entries of the span's blocks and its
    /// summary pieces of the levels above a block's.
    pub fn table_span(&self, run: u32, span: u32, table: usize) -> io::Result<Arc<SpanRow>> {
        let held = self.span(run, span)?;
        self.span_row(run, span, &held, table)
    }

    /// The head of every block of run `run`, in order.
    pub fn heads(&self, run: u32) -> io::Result<Vec<BlockHead>> {
        (0..self.run(run)?.directory.blocks()).map(|block| self.head(run, block)).collect()
    }

    /// Every hour's flags and metrics of block `block` of run `run`.
    pub fn clock(&self, run: u32, block: u32) -> io::Result<Vec<ClockHour>> {
        let head = self.head(run, block)?;
        let raw = self.frame(&head.clock).map_err(|error| Damage::wrap(DamageScope::Page { run, block }, error))?;
        Clock::read(&raw, u32::from(head.hours))
    }

    /// The chain of run `run` through the block holding ordinal `ordinal`. Blocks are written
    /// once, so a recording grown from this one holds the same there.
    pub fn chain_through(&self, run: u32, ordinal: u32) -> io::Result<[u8; 32]> {
        if ordinal >= self.run(run)?.ordinals {
            return Err(invalid("No block of the run holds the ordinal"));
        }
        Ok(self.head(run, ordinal / UNIT_ORDINALS)?.chain)
    }

    /// Blocks `from..to` of run `run` as the unit's reader primitives take a run's pages.
    pub fn unit_pages(&self, run: u32, from: u32, to: u32) -> io::Result<Vec<BlockPage>> {
        (from..to).map(|block| self.block(run, block).map(|block| block.unit_page())).collect()
    }

    /// `table`'s entries as the reader primitives read a run's pages through, for a reader of one
    /// table: blocks `from..to` of run `run`, each its block's page holding that table's entry
    /// alone at its index.
    pub fn table_pages(&self, run: u32, table: usize, from: u32, to: u32) -> io::Result<Vec<BlockPage>> {
        let held = self.run(run)?;
        let mut pages = Vec::with_capacity(to.saturating_sub(from) as usize);
        for block in from..to {
            let mut tables = vec![TableEntry::default(); TABLE_COUNT];
            tables[table] = self.table_entry(run, block, table)?.unit;
            pages.push(BlockPage { block, hours: held.block(block as usize).1 as u16, tables });
        }
        Ok(pages)
    }

    /// The strings block `head` appended, by id.
    pub fn strings(&self, head: &BlockHead) -> io::Result<Vec<(u32, Vec<u8>)>> {
        let mut out = Vec::new();
        for chunk in &head.strings {
            out.extend(strings_of(&*self.source, &chunk.frame, chunk.first)?);
        }
        Ok(out)
    }

    /// Run `run`'s pages from the earliest block every table's last restore at or before its
    /// last block lies in: what taking the run up rebuilds its tables from. The first block's
    /// place in the run, and the pages.
    pub fn pages_since_restores(&self, run: u32) -> io::Result<(u32, Vec<BlockPage>)> {
        let blocks = self.run(run)?.directory.blocks();
        let mut pages = Vec::new();
        let mut found = vec![false; TABLE_COUNT];
        let mut first = blocks;
        while first > 0 && found.iter().any(|found| !found) {
            first -= 1;
            let page = self.block(run, first)?.unit_page();
            for (table, entry) in page.tables.iter().enumerate() {
                found[table] |= entry.restore.is_some() || entry.first.is_some();
            }
            pages.push(page);
        }
        pages.reverse();
        if found.iter().any(|found| !found) {
            return Err(invalid(format!("Run {run} holds a table with no restore")));
        }
        Ok((first, pages))
    }
}

/// The strings frame `frame` of a block chunk whose first string is `first`, read from `file`:
/// each string with its id.
pub fn strings_of(file: &dyn Extents, frame: &FrameEntry, first: u32) -> io::Result<Vec<(u32, Vec<u8>)>> {
    let (raw, places) = strings_frame(file, frame)?;
    Ok(places.iter().zip(first..).map(|(&(at, length), id)| (id, raw[at as usize..(at + length) as usize].to_vec())).collect())
}

/// A block's strings chunk, decoded: the frame, and each string's place in it, by id from the
/// chunk's first.
pub fn strings_frame(file: &dyn Extents, frame: &FrameEntry) -> io::Result<(Vec<u8>, Vec<(u32, u32)>)> {
    let mut stored = Vec::new();
    file.read_at(frame.offset, frame.stored as usize, &mut stored)?;
    if envelope::digest(&stored) != frame.digest {
        return Err(invalid("A block's strings differ from their digest"));
    }
    let raw = container::decompress(&stored, Some(u64::from(frame.raw)))?;
    drop(stored);
    let mut reader = replay_columnar::leaf::Reader::new(&raw);
    let count = reader.count()?;
    if count > raw.len() {
        return Err(invalid("A block's strings count more than they hold"));
    }
    let mut places = Vec::with_capacity(count);
    for _ in 0..count {
        let length = reader.count()?;
        let at = raw.len() - reader.rest().len();
        reader.raw(length)?;
        places.push((at as u32, length as u32));
    }
    reader.finish()?;
    Ok((raw, places))
}

impl fmt::Debug for Recording {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Recording").field("revision", &self.revision).field("runs", &self.runs.len()).finish_non_exhaustive()
    }
}
