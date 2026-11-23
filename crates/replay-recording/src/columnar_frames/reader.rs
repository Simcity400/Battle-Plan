use crate::schema::{columns_at, schema, strings_frame, withheld_at};

use super::{
    DerivedCoverage, REQUIRED_DERIVED_VERSIONS, Segment, decode_derived_from, derived_withheld,
    input::{Held, Source},
    invalid,
};
use replay_data::rows::{TABLE_COUNT, TableId};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DamageScope {
    Unlocated,
    Run(u32),
    Page { run: u32, block: u32 },
    Extent(Extent),
    Table { run: u32, block: u32, table: u32 },
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

pub fn table_name(table: u32) -> String {
    TableId::ALL.get(table as usize).map_or_else(|| format!("Table {table}"), |id| format!("{id:?}"))
}

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
    pub fn wrap(scope: DamageScope, cause: io::Error) -> io::Error {
        if cause.kind() != io::ErrorKind::InvalidData || Self::of(&cause).is_some() {
            return cause;
        }
        io::Error::new(io::ErrorKind::InvalidData, Self { scope, cause })
    }

    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref()?.downcast_ref()
    }
}

pub struct Run {
    pub branch: u64,
    pub epoch: u8,
    pub first_hour: i32,
    pub ordinals: u32,
    pub chain: [u8; 32],
    directory: RunDirectory,
}

impl Run {
    pub fn pages(&self) -> usize {
        self.directory.blocks() as usize
    }

    pub fn block(&self, at: usize) -> (u32, u32) {
        let first = at as u32 * UNIT_ORDINALS;
        (first, self.ordinals.saturating_sub(first).min(UNIT_ORDINALS))
    }

    pub fn block_first_hour(&self, at: u32) -> i32 {
        self.first_hour + (at * UNIT_ORDINALS) as i32
    }

    pub fn place(&self, ordinal: u32) -> (u32, u16) {
        (ordinal / UNIT_ORDINALS, (ordinal % UNIT_ORDINALS) as u16)
    }

    pub fn ordinal(&self, hour: i32) -> Option<u32> {
        u32::try_from(i64::from(hour) - i64::from(self.first_hour)).ok().filter(|&ordinal| ordinal < self.ordinals)
    }

    pub fn last_hour(&self) -> i32 {
        self.first_hour + self.ordinals as i32 - 1
    }

    pub fn directory(&self) -> &RunDirectory {
        &self.directory
    }
}

struct SpanHeld {
    directory: SpanDirectory,
    index: Extent,
    rows: Mutex<Vec<Option<Arc<SpanRow>>>>,
}

#[derive(Default)]
struct Pages {
    spans: Mutex<HashMap<(u32, u32), Arc<SpanHeld>, foldhash::fast::RandomState>>,
    open: Mutex<HashMap<(u32, u32), Arc<Block>, foldhash::fast::RandomState>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Framing {
    columns: Vec<usize>,
    withheld: Vec<(usize, String)>,
    tables: usize,
    format: u16,
}

fn recorded_columns(root: &Root, format: impl FnOnce() -> io::Result<Option<u16>>) -> io::Result<Framing> {
    let schema = schema()?;
    let ours = envelope::schema_digest(&schema);
    let hex = |digest: &[u8; 32]| digest[..8].iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let differs = |also: &str| invalid(format!("This recording was written with another schema (its digest {}, this build's {}){also}", hex(&root.identity.schema), hex(&ours)));
    let current = replay_data::format::FormatVersion::CURRENT.version();
    let format = match format() {
        Ok(Some(format)) => replay_data::format::FormatVersion::read(format)?.version(),
        Ok(None) if root.identity.schema == ours => current,
        Ok(None) => return Err(differs("")),
        Err(error) if error.kind() == io::ErrorKind::InvalidData && root.identity.schema == ours => current,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Err(differs(&format!("; its records cannot be read: {error}"))),
        Err(error) => return Err(error),
    };
    if format == current {
        if root.identity.schema != ours {
            return Err(differs(""));
        }
        let mut withheld = Vec::new();
        derived_withheld(root, TABLE_COUNT, &mut withheld);
        return Ok(Framing { columns: schema.iter().map(Vec::len).collect(), withheld, tables: TABLE_COUNT, format });
    }
    let mut columns = columns_at(format)?;
    let tables = columns.len();
    let historical = crate::schema::schema_at(format)?;
    if envelope::schema_digest_prefix(&historical[..tables], &columns) != Some(root.identity.schema) {
        return Err(differs(""));
    }
    columns.resize(TABLE_COUNT, 0);
    let mut withheld = withheld_at(format)?;
    derived_withheld(root, tables, &mut withheld);
    Ok(Framing { columns, withheld, tables, format })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableCoverage<'a> {
    Valid,
    WithheldAll(&'a str),
    WithheldBefore { ordinal: u32, reason: &'a str },
}

pub struct Recording {
    source: Arc<Source>,
    root: Root,
    revision: Revision,
    saved: Option<Revision>,
    records: Box<[Extent]>,
    facts: Box<[Extent]>,
    directories: Box<[Extent]>,
    columns: Box<[usize]>,
    tables: usize,
    format: u16,
    withheld: Box<[(usize, String)]>,
    derived_coverage: Box<[DerivedCoverage]>,
    runs: Box<[Option<Arc<Run>>]>,
    segments: Box<[Segment]>,
    recorded: Box<[Segment]>,
    damage: Box<[Damage]>,
    selected: [bool; TABLE_COUNT],
    projected: Option<Arc<Vec<Option<Vec<usize>>>>>,
    pages: Arc<Pages>,
}

impl Recording {
    pub fn open(bytes: Vec<u8>) -> io::Result<Self> {
        Self::read(Held::Memory(bytes), Open::Saved)
    }

    pub fn open_path(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_path_as(path, Open::Saved)
    }

    pub fn open_path_as(path: impl AsRef<Path>, open: Open) -> io::Result<Self> {
        Self::read(Held::File(File::open(path)?), open)
    }

    pub fn open_file(file: File, open: Open) -> io::Result<Self> {
        Self::read(Held::File(file), open)
    }

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
            tables: TABLE_COUNT,
            format: replay_data::format::FormatVersion::CURRENT.version(),
            withheld: Box::default(),
            derived_coverage: Box::default(),
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
        let framing = recorded_columns(&recording.root, || {
            let Some(last) = recording.records.last() else { return Ok(None) };
            let raw = recording.raw(last)?;
            if let Some(format) = crate::records::Records::format_of(&raw) {
                return Ok(Some(format));
            }
            Ok(Some(crate::capture::Capture::read(&raw)?.format))
        })?;
        recording.columns = framing.columns.into();
        recording.tables = framing.tables;
        recording.format = framing.format;
        let mut withheld = framing.withheld;
        let mut damage = Vec::new();
        let mut runs = Vec::with_capacity(recording.directories.len());
        let mut segments = Vec::new();
        for (index, extent) in (0..).zip(&*recording.directories) {
            match recording.raw(extent).and_then(|raw| RunDirectory::read(&raw, extent, index)) {
                Ok(directory) => {
                    if directory.hours > 0 {
                        segments.push(Segment { branch: directory.branch, run: index, first: 0, last: directory.hours - 1 });
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
        let mut derived_coverage = Vec::new();
        for &(table, required, reason) in REQUIRED_DERIVED_VERSIONS {
            let index = table.index();
            if index >= recording.tables || withheld.iter().any(|(held, _)| *held == index) {
                continue;
            }
            let key = SectionKey::derived(index as u32);
            let Some(section) = recording.root.section(key) else { continue };
            if section.version < required || section.is_empty() {
                continue;
            }
            let read_boundary = || -> io::Result<(u32, u32)> {
                let extents = recording.section(key, section.version)?;
                let [extent] = extents.as_slice() else {
                    return Err(invalid(format!("{key} holds {} extents; expected 1", extents.len())));
                };
                let (from_run, from_ordinal) = decode_derived_from(&recording.raw(extent)?)?;
                let run_count = recording.runs.len() as u32;
                let valid = if from_run > run_count || (from_run == run_count && from_ordinal > 0) {
                    false
                } else if from_run < run_count {
                    recording.runs[from_run as usize].as_ref().is_some_and(|run| {
                        from_ordinal <= run.ordinals
                            && i32::try_from(from_ordinal).ok().and_then(|o| run.first_hour.checked_add(o)).is_some()
                    })
                } else {
                    true
                };
                if !valid {
                    return Err(invalid(format!("{key} names out-of-range start run {from_run} ordinal {from_ordinal}")));
                }
                Ok((from_run, from_ordinal))
            };
            match read_boundary() {
                Ok((0, 0)) => {}
                Ok((from_run, from_ordinal)) => {
                    derived_coverage.push(DerivedCoverage { table: index, from_run, from_ordinal, reason: reason.to_owned() });
                }
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    let text = format!("{}'s version marker is damaged: {error}", table_name(index as u32));
                    damage.push(Damage { scope: DamageScope::Withheld { table: index as u32 }, cause: error });
                    withheld.push((index, text));
                }
                Err(error) => return Err(error),
            }
        }
        recording.withheld = withheld.into();
        recording.derived_coverage = derived_coverage.into();
        recording.recorded = segments.clone().into();
        recording.segments = segments.into();
        recording.damage = damage.into();
        Ok(recording)
    }

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
            tables: self.tables,
            format: self.format,
            withheld: self.withheld.clone(),
            derived_coverage: self.derived_coverage.clone(),
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

    pub fn select(&mut self, tables: &[TableId]) {
        self.selected = [false; TABLE_COUNT];
        for table in tables {
            self.selected[table.index()] = true;
        }
    }

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

    pub fn selects(&self, table: usize) -> bool {
        self.selected.get(table).copied().unwrap_or(false)
    }

    pub fn read_columns(&self, table: usize) -> Vec<usize> {
        match self.projected.as_ref().and_then(|projected| projected.get(table)?.as_ref()) {
            Some(columns) => columns.clone(),
            None => (0..self.columns.get(table).copied().unwrap_or(0)).collect(),
        }
    }

    pub fn withheld(&self) -> &[(usize, String)] {
        &self.withheld
    }

    pub fn withheld_reason(&self, table: usize) -> Option<&str> {
        self.withheld.iter().find(|(held, _)| *held == table).map(|(_, reason)| reason.as_str())
    }

    pub fn table_coverage(&self, table: usize, run: u32) -> TableCoverage<'_> {
        if let Some(reason) = self.withheld_reason(table) {
            return TableCoverage::WithheldAll(reason);
        }
        let Some(coverage) = self.derived_coverage.iter().find(|held| held.table == table) else {
            return TableCoverage::Valid;
        };
        match run.cmp(&coverage.from_run) {
            std::cmp::Ordering::Less => TableCoverage::WithheldAll(coverage.reason.as_str()),
            std::cmp::Ordering::Equal if coverage.from_ordinal > 0 => TableCoverage::WithheldBefore {
                ordinal: coverage.from_ordinal,
                reason: coverage.reason.as_str(),
            },
            _ => TableCoverage::Valid,
        }
    }

    pub fn predates(&self, table: usize) -> Option<&str> {
        (table >= self.tables).then(|| self.withheld_reason(table)).flatten()
    }

    pub fn campaign(&self) -> &[u8] {
        &self.root.identity.campaign
    }

    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    pub fn identity(&self) -> io::Result<replay_data::identity::Identity> {
        crate::recording_identity::decode_identity(&self.root.identity.content)
    }

    pub fn root_identity(&self) -> &envelope::Identity {
        &self.root.identity
    }

    pub fn records(&self) -> io::Result<crate::records::Records> {
        let mut records = crate::records::Records::default();
        for extent in &*self.records {
            records.decode_into(&self.raw(extent)?)?;
        }
        Ok(records)
    }

    pub fn capture(&self) -> io::Result<Option<crate::capture::Capture>> {
        if let Some(last) = self.records.last() {
            if crate::records::Records::format_of(&self.raw(last)?).is_some() {
                return Ok(None);
            }
        }
        crate::capture::read_capture(&self.records, |extent| self.raw(extent))
    }

    pub fn damage(&self) -> &[Damage] {
        &self.damage
    }

    pub fn unlocated(&self) -> Option<io::Error> {
        self.damage
            .iter()
            .find(|damage| matches!(damage.scope, DamageScope::Unlocated | DamageScope::Run(_)))
            .map(|damage| Damage::wrap(damage.scope.clone(), invalid(damage.cause.to_string())))
    }

    pub fn hours(&self) -> u64 {
        self.runs.iter().flatten().map(|run| u64::from(run.ordinals)).sum()
    }

    pub fn run_count(&self) -> u32 {
        self.runs.len() as u32
    }

    pub fn run(&self, run: u32) -> io::Result<&Run> {
        match self.runs.get(run as usize) {
            Some(Some(held)) => Ok(held),
            Some(None) => Err(Damage::wrap(DamageScope::Run(run), invalid("cannot be read"))),
            None => Err(invalid(format!("The recording holds no run {run}"))),
        }
    }

    pub fn directories(&self) -> io::Result<Vec<&RunDirectory>> {
        (0..self.run_count()).map(|run| self.run(run).map(Run::directory)).collect()
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn recorded_segments(&self) -> &[Segment] {
        &self.recorded
    }

    pub fn with_segments(mut self, segments: Vec<Segment>) -> io::Result<Self> {
        for segment in &segments {
            let held = self.recorded.iter().find(|held| held.run == segment.run);
            if !held.is_some_and(|held| segment.first <= segment.last && segment.last <= held.last) {
                return Err(invalid("A segment the recording does not hold"));
            }
        }
        self.segments = segments.into();
        Ok(self)
    }

    pub fn root(&self) -> &Root {
        &self.root
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn saved(&self) -> Option<Revision> {
        self.saved
    }

    pub fn root_footer_digest(&self) -> io::Result<[u8; 16]> {
        Ok(self.revision.root)
    }

    pub fn records_extents(&self) -> &[Extent] {
        &self.records
    }

    pub fn facts_extents(&self) -> &[Extent] {
        &self.facts
    }

    pub fn run_directories(&self) -> &[Extent] {
        &self.directories
    }

    pub fn len(&self) -> u64 {
        self.source.len()
    }

    pub fn memory(&self) -> Option<&[u8]> {
        self.source.memory()
    }

    pub fn extents(&self) -> &(dyn Extents + 'static) {
        &*self.source
    }

    pub fn source(&self) -> Arc<Source> {
        Arc::clone(&self.source)
    }

    pub fn format(&self) -> u16 {
        self.format
    }

    pub fn tables(&self) -> usize {
        self.tables
    }

    fn holds(&self, table: usize) -> io::Result<()> {
        if table < self.tables {
            return Ok(());
        }
        let reason = self.withheld_reason(table).map_or_else(|| format!("{} is not in this recording's schema", table_name(table as u32)), str::to_owned);
        Err(Damage::wrap(DamageScope::Withheld { table: table as u32 }, invalid(reason)))
    }

    pub fn stored_at(&self, extent: &Extent, at: u64, out: &mut [u8]) -> io::Result<()> {
        self.source.stored_at(extent, at, out).map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))
    }

    pub fn stored_bytes(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        self.source.stored(extent).map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))
    }

    pub fn raw(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        let stored = self.stored_bytes(extent)?;
        match extent.kind {
            Kind::Summary | Kind::SpanIndex | Kind::Facts | Kind::Stored => Ok(stored),
            _ => container::decompress(&stored, Some(extent.raw)).map_err(|error| Damage::wrap(DamageScope::Extent(*extent), extent.damaged(&error.to_string()))),
        }
    }

    pub fn extent_bytes(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        self.raw(extent)
    }

    pub fn frame(&self, frame: &FrameEntry) -> io::Result<Vec<u8>> {
        let mut stored = Vec::new();
        self.source.read_at(frame.offset, frame.stored as usize, &mut stored)?;
        if envelope::digest(&stored) != frame.digest {
            return Err(invalid("A frame differs from its digest"));
        }
        container::decompress(&stored, Some(u64::from(frame.raw)))
    }

    fn span(&self, run: u32, span: u32) -> io::Result<Arc<SpanHeld>> {
        if let Some(held) = locked(&self.pages.spans).get(&(run, span)) {
            return Ok(Arc::clone(held));
        }
        let pages = *self.run(run)?.directory.spans.get(span as usize).ok_or_else(|| invalid(format!("Run {run} holds no span {span}")))?;
        let scope = DamageScope::Page { run, block: span * SPAN_BLOCKS };
        let read = || -> io::Result<SpanHeld> {
            let directory = SpanDirectory::read(&self.raw(&pages.directory)?, &pages.directory, self.tables)?;
            if directory.first_block != span * SPAN_BLOCKS || directory.blocks.len() != SPAN_BLOCKS as usize {
                return Err(invalid("is not its run's span"));
            }
            if pages.index.stored != pages.index.raw {
                return Err(invalid("A span index is stored compressed"));
            }
            Ok(SpanHeld { directory, index: pages.index, rows: Mutex::new(vec![None; self.tables]) })
        };
        let held = Arc::new(read().map_err(|error| Damage::wrap(scope, error))?);
        Ok(Arc::clone(locked(&self.pages.spans).entry((run, span)).or_insert(held)))
    }

    fn span_row(&self, run: u32, span: u32, held: &SpanHeld, table: usize) -> io::Result<Arc<SpanRow>> {
        self.holds(table)?;
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

    fn open_block(&self, run: u32, block: u32) -> io::Result<Arc<Block>> {
        if let Some(held) = locked(&self.pages.open).get(&(run, block)) {
            return Ok(Arc::clone(held));
        }
        let directory = &self.run(run)?.directory;
        let page = directory.open.get((block - directory.spans.len() as u32 * SPAN_BLOCKS) as usize).ok_or_else(|| invalid(format!("Run {run} holds no block {block}")))?;
        let read = Block::read(&self.raw(page)?, page, self.tables).map_err(|error| Damage::wrap(DamageScope::Page { run, block }, error))?;
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

    pub fn block(&self, run: u32, block: u32) -> io::Result<Block> {
        self.check_block(run, block)?;
        if !self.closed(run, block)? {
            return Ok((*self.open_block(run, block)?).clone());
        }
        let span = block / SPAN_BLOCKS;
        let held = self.span(run, span)?;
        let at = (block % SPAN_BLOCKS) as usize;
        let mut tables = Vec::with_capacity(self.tables);
        for table in 0..self.tables {
            tables.push(self.span_row(run, span, &held, table)?.blocks[at].clone());
        }
        Ok(Block { run, block, head: held.directory.blocks[at].clone(), tables })
    }

    pub fn head(&self, run: u32, block: u32) -> io::Result<BlockHead> {
        self.check_block(run, block)?;
        if !self.closed(run, block)? {
            return Ok(self.open_block(run, block)?.head.clone());
        }
        Ok(self.span(run, block / SPAN_BLOCKS)?.directory.blocks[(block % SPAN_BLOCKS) as usize].clone())
    }

    pub fn table_entry(&self, run: u32, block: u32, table: usize) -> io::Result<Entry> {
        self.holds(table)?;
        self.check_block(run, block)?;
        if !self.closed(run, block)? {
            return self.open_block(run, block)?.tables.get(table).cloned().ok_or_else(|| invalid(format!("A block names no table {table}")));
        }
        let span = block / SPAN_BLOCKS;
        let held = self.span(run, span)?;
        Ok(self.span_row(run, span, &held, table)?.blocks[(block % SPAN_BLOCKS) as usize].clone())
    }

    pub fn block_of(&self, run: u32, block: usize, table: usize) -> io::Result<Option<Extent>> {
        Ok(self.table_entry(run, block as u32, table)?.summary)
    }

    pub fn span_summaries(&self, run: u32, span: u32, table: usize) -> io::Result<Vec<(u8, Extent)>> {
        let held = self.span(run, span)?;
        Ok(self.span_row(run, span, &held, table)?.summaries.clone())
    }

    pub fn table_span(&self, run: u32, span: u32, table: usize) -> io::Result<Arc<SpanRow>> {
        let held = self.span(run, span)?;
        self.span_row(run, span, &held, table)
    }

    pub fn heads(&self, run: u32) -> io::Result<Vec<BlockHead>> {
        (0..self.run(run)?.directory.blocks()).map(|block| self.head(run, block)).collect()
    }

    pub fn clock(&self, run: u32, block: u32) -> io::Result<Vec<ClockHour>> {
        let head = self.head(run, block)?;
        let raw = self.frame(&head.clock).map_err(|error| Damage::wrap(DamageScope::Page { run, block }, error))?;
        Clock::read(&raw, u32::from(head.hours))
    }

    pub fn chain_through(&self, run: u32, ordinal: u32) -> io::Result<[u8; 32]> {
        if ordinal >= self.run(run)?.ordinals {
            return Err(invalid("No block of the run holds the ordinal"));
        }
        Ok(self.head(run, ordinal / UNIT_ORDINALS)?.chain)
    }

    pub fn unit_pages(&self, run: u32, from: u32, to: u32) -> io::Result<Vec<BlockPage>> {
        (from..to).map(|block| self.block(run, block).map(|block| block.unit_page())).collect()
    }

    pub fn table_pages(&self, run: u32, table: usize, from: u32, to: u32) -> io::Result<Vec<BlockPage>> {
        let held = self.run(run)?;
        let mut pages = Vec::with_capacity(to.saturating_sub(from) as usize);
        for block in from..to {
            let entry = self.table_entry(run, block, table)?.unit;
            let mut tables = vec![TableEntry::default(); self.tables];
            tables[table] = entry;
            pages.push(BlockPage { block, hours: held.block(block as usize).1 as u16, tables });
        }
        Ok(pages)
    }

    pub fn strings(&self, head: &BlockHead) -> io::Result<Vec<(u32, Vec<u8>)>> {
        let mut out = Vec::new();
        for chunk in &head.strings {
            out.extend(strings_of(&*self.source, &chunk.frame, chunk.first)?);
        }
        Ok(out)
    }

    pub fn pages_since_restores(&self, run: u32) -> io::Result<(u32, Vec<BlockPage>)> {
        let blocks = self.run(run)?.directory.blocks();
        let mut pages = Vec::new();
        let mut found = vec![false; self.tables];
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

pub fn strings_of(file: &dyn Extents, frame: &FrameEntry, first: u32) -> io::Result<Vec<(u32, Vec<u8>)>> {
    let (raw, places) = strings_frame(file, frame)?;
    Ok(places.iter().zip(first..).map(|(&(at, length), id)| (id, raw[at as usize..(at + length) as usize].to_vec())).collect())
}


impl fmt::Debug for Recording {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Recording").field("revision", &self.revision).field("runs", &self.runs.len()).finish_non_exhaustive()
    }
}
