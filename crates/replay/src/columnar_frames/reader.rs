//! A columnar recording read from its file or memory: every run's directory and restarts
//! held from open, its hours walked a block at a time by [`Walker`]s, each decoding a block's
//! hour records and selected tables at once into one buffer it keeps.

use super::{
    Segment, columns_at, withheld_at,
    input::{Decoder, Held, Source, check, chunk_places},
    invalid, schema,
};
use crate::rows::{TABLE_COUNT, TableId};
use replay_columnar::{
    block::TableDecoder,
    envelope::{
        self, Extent, IndexPage, KEYFRAME_RESTARTS, RestartDirectory, RestartTable, Root,
        RunDirectory,
    },
    section::Scratch,
    manifest::Manifest,
    restart::WeakRestored,
    records::RecordsDecoder,
    walk::{Chunks, Dictionary, Packed, Part, SpanPart, State},
};
use std::{
    collections::HashMap,
    fmt,
    fs::File,
    io,
    mem::MaybeUninit,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex, PoisonError, Weak},
    time::Instant,
};

/// What a piece of damage refuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DamageScope {
    /// The recording: no committed root can be read.
    Unlocated,
    /// A run's directory, or a dictionary id no readable chunk of the run holds.
    Run(u32),
    /// A run's page, by its first ordinal and hours.
    Page { run: u32, first: u32, hours: u32 },
    /// A block's hour records, or one table's block.
    Extent(Extent),
    /// A run's dictionary chunk, by its restart's ordinal.
    Dictionary { run: u32, chunk: u32 },
    /// One table's state at a run's restart after ordinal `after`.
    RestartTable { run: u32, after: u32, table: u32 },
    /// A table the recording's format holds in a layout or meaning this build reads otherwise.
    Withheld { table: u32 },
}

impl fmt::Display for DamageScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unlocated => write!(f, "The recording"),
            Self::Run(run) => write!(f, "Run {run}'s directory"),
            Self::Page { run, first, hours } => {
                write!(f, "Run {run} from ordinal {first} ({hours} hours)")
            }
            Self::Extent(extent) => write!(
                f,
                "{:?} extent of run {} stream {} from ordinal {} ({} hours)",
                extent.kind, extent.run, extent.stream, extent.first, extent.hours
            ),
            Self::Dictionary { run, chunk } => write!(f, "Run {run}'s dictionary chunk {chunk}"),
            Self::RestartTable { run, after, table } => {
                write!(
                    f,
                    "Run {run}'s restart after ordinal {after}, table {table}"
                )
            }
            Self::Withheld { table } => write!(f, "Table {table} of the recording"),
        }
    }
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
    pub(super) fn wrap(scope: DamageScope, cause: io::Error) -> io::Error {
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

/// Keeps `cause` as damage refusing `scope`; anything else stops the recording's open.
fn record(damage: &mut Vec<Damage>, scope: DamageScope, cause: io::Error) -> io::Result<()> {
    if cause.kind() != io::ErrorKind::InvalidData {
        return Err(cause);
    }
    engine_allocation::reserve(damage, 1)?;
    damage.push(Damage { scope, cause });
    Ok(())
}

/// What segments and dictionary chunks need of a readable page's restart.
struct Restart {
    page: usize,
    after: u32,
    ordinal: u32,
    /// The run's dictionary ids its chunk holds: its first, and how many.
    ids: (u32, u32),
    chunk: Extent,
}

/// A run whose directory can be read: its place in the campaign, its pages and the restarts
/// its readable pages hold.
pub struct Run {
    pub branch: u64,
    /// Why the run starts ([`envelope::epoch`]); every run starts from empty tables.
    pub epoch: u8,
    pub first_hour: i32,
    pub ordinals: u32,
    /// The hour digest chain after its last hour: every manifest and section it holds.
    pub chain: [u8; 32],
    index: u32,
    source: Arc<Source>,
    pages: Box<[Extent]>,
    restarts: Box<[Restart]>,
    /// How many of the restarts, from the first, hold the run's next dictionary ids.
    valid: usize,
    /// The chunks of the valid restarts that walkers hold: the run's one lock.
    loaded: Mutex<Box<[Weak<Packed>]>>,
}

impl Run {
    pub fn pages(&self) -> usize {
        self.pages.len()
    }

    /// Its pages' envelopes, in order.
    pub fn page_extents(&self) -> &[Extent] {
        &self.pages
    }

    /// Each valid restart's dictionary chunk's first id, by restart ordinal, and the run's ids
    /// those chunks hold.
    pub fn chunk_starts(&self) -> (Vec<u32>, u32) {
        let valid = &self.restarts[..self.valid];
        let starts = valid.iter().map(|restart| restart.ids.0).collect();
        let ids = valid.last().map_or(0, |restart| restart.ids.0 + restart.ids.1);
        (starts, ids)
    }

    /// The pages holding `segment`'s ordinals.
    fn pages_of(&self, segment: &Segment) -> Range<usize> {
        let first = self
            .pages
            .partition_point(|page| page.first < segment.first);
        first
            ..self
                .pages
                .partition_point(|page| page.first <= segment.last)
    }

    /// Run `index`'s directory at `extent` and its pages' restarts; None, with its damage
    /// kept, when the directory cannot be read.
    fn read(
        source: &Arc<Source>,
        decoder: &mut Decoder,
        raw: &mut Vec<u8>,
        extent: &Extent,
        index: u32,
        damage: &mut Vec<Damage>,
    ) -> io::Result<Option<Self>> {
        raw.clear();
        let directory = decoder
            .append(source, extent, raw)
            .and_then(|()| RunDirectory::read(raw, extent, index));
        let directory = match directory {
            Ok(directory) => directory,
            Err(error) => {
                record(damage, DamageScope::Run(index), error)?;
                return Ok(None);
            }
        };
        let mut restarts = Vec::new();
        for (page, extent) in directory.pages.iter().enumerate() {
            raw.clear();
            let read = decoder
                .append(source, extent, raw)
                .and_then(|()| IndexPage::read(raw, extent, TABLE_COUNT));
            match read {
                Ok(read) => {
                    if let Some(restart) = read.restart {
                        engine_allocation::reserve(&mut restarts, 1)?;
                        restarts.push(Restart {
                            page,
                            after: restart.after,
                            ordinal: restart.ordinal,
                            ids: restart.ids,
                            chunk: restart.chunk,
                        });
                    }
                }
                Err(error) => {
                    let scope = DamageScope::Page {
                        run: index,
                        first: extent.first,
                        hours: extent.hours,
                    };
                    record(damage, scope, error)?;
                }
            }
        }
        // Chunks follow one another in their ordinals' and ids' order; an unreadable page
        // may hide one, and an inconsistent one leaves no later chunk's ids certain.
        let (mut valid, mut last, mut next) = (0, None, 0_u64);
        for restart in &restarts {
            let first = u64::from(restart.ids.0);
            let follows = match last {
                Some(last) => {
                    restart.ordinal > last && (restart.ordinal != last + 1 || first == next)
                }
                None => restart.ordinal != 0 || first == 0,
            };
            if !follows || first < next {
                break;
            }
            valid += 1;
            (last, next) = (Some(restart.ordinal), first + u64::from(restart.ids.1));
        }
        Ok(Some(Self {
            branch: directory.branch,
            epoch: directory.epoch,
            first_hour: directory.first_hour,
            ordinals: directory.ordinals,
            chain: directory.chain,
            index,
            source: source.clone(),
            pages: engine_allocation::boxed_slice(directory.pages)?,
            restarts: engine_allocation::boxed_slice(restarts)?,
            valid,
            loaded: Mutex::new(engine_allocation::boxed_slice(
                engine_allocation::collect_vec((0..valid).map(|_| Weak::new()))?,
            )?),
        }))
    }

    /// The valid restart whose chunk is chunk `ordinal` of the run.
    fn chunk_at(&self, ordinal: u32) -> io::Result<usize> {
        self.restarts[..self.valid]
            .binary_search_by_key(&ordinal, |restart| restart.ordinal)
            .map_err(|_| {
                let held = self.restarts[self.valid..]
                    .iter()
                    .any(|restart| restart.ordinal == ordinal);
                let scope = DamageScope::Dictionary {
                    run: self.index,
                    chunk: ordinal,
                };
                Damage::wrap(
                    scope,
                    invalid(if held {
                        "does not hold the run's next dictionary ids"
                    } else {
                        "is on no readable page"
                    }),
                )
            })
    }

    /// Valid restart `at`'s chunk, shared with every walker holding it.
    fn load(&self, at: usize) -> io::Result<Arc<Packed>> {
        let restart = &self.restarts[at];
        let mut loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(chunk) = loaded[at].upgrade() {
            return Ok(chunk);
        }
        let read = || -> io::Result<Packed> {
            let mut raw = Vec::new();
            Decoder::new()?.append(&self.source, &restart.chunk, &mut raw)?;
            let chunk = Packed::read(restart.ids.0 as usize, raw)?;
            if chunk.len() != restart.ids.1 as usize {
                return Err(invalid("does not hold the run's next dictionary ids"));
            }
            Ok(chunk)
        };
        let chunk = Arc::new(read().map_err(|error| {
            let scope = DamageScope::Dictionary {
                run: self.index,
                chunk: restart.ordinal,
            };
            Damage::wrap(scope, error)
        })?);
        loaded[at] = Arc::downgrade(&chunk);
        Ok(chunk)
    }
}

impl Chunks for Run {
    fn chunk(&self, id: u32) -> io::Result<Arc<Packed>> {
        let valid = &self.restarts[..self.valid];
        let at = valid.partition_point(|restart| restart.ids.0 <= id);
        match at.checked_sub(1) {
            Some(at) if id - valid[at].ids.0 < valid[at].ids.1 => self.load(at),
            _ => Err(Damage::wrap(
                DamageScope::Run(self.index),
                invalid(format!(
                    "names no readable dictionary chunk holding id {id}"
                )),
            )),
        }
    }
}

/// One restart's tables as walks starting there decoded them, each held weakly, by table.
type Decoded = Mutex<Vec<(usize, WeakRestored)>>;

/// The restarts walks restored, by run and page, each decoded once while any walk still shares
/// its pages ([`Walker::restore`]): a walk starting there shares them copy-on-write, holding
/// of its own only what its hours write.
#[derive(Default)]
struct Restarts {
    held: Mutex<HashMap<(u32, usize), Arc<Decoded>, foldhash::fast::RandomState>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Restarts {
    /// Restart page `at` of `run`'s tables, made when no walk holds them, and the restarts no
    /// walk shares any page of let go.
    fn entry(&self, run: u32, at: usize) -> io::Result<Arc<Decoded>> {
        let mut held = locked(&self.held);
        if let Some(entry) = held.get(&(run, at)) {
            return Ok(entry.clone());
        }
        // An entry being decoded, or one a walk holds, stays.
        held.retain(|_, entry| {
            Arc::strong_count(entry) > 1
                || entry.try_lock().map_or(true, |tables| {
                    tables.iter().any(|(_, restored)| restored.alive())
                })
        });
        engine_allocation::reserve_map(&mut held, 1)?;
        let entry = Arc::new(Mutex::new(Vec::new()));
        held.insert((run, at), entry.clone());
        Ok(entry)
    }
}

/// What `root`'s recording holds of each schema table: its columns, and the tables this build
/// reads otherwise, each with why. Of this build's schema, every column and none; else of the
/// row format its `capture` names, when its identity names that format's schema as each table
/// was framed then: the columns appended since hold no recorded cell, and the tables whose
/// layout or meaning changed since are withheld by name.
fn recorded_columns(
    root: &Root,
    format: impl FnOnce() -> io::Result<Option<u16>>,
) -> io::Result<(Vec<usize>, Vec<(usize, String)>)> {
    let schema = schema()?;
    if root.identity.schema == envelope::schema_digest(&schema) {
        return Ok((
            engine_allocation::collect_vec(schema.iter().map(Vec::len))?,
            Vec::new(),
        ));
    }
    let differs = |what: &str| invalid(format!("The recording's schema differs from {what}"));
    let format = match format() {
        Ok(Some(format)) => format,
        Ok(None) => return Err(differs("this build's, and it names no format")),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return Err(differs(&format!(
                "this build's, and its records cannot be read: {error}"
            )));
        }
        Err(error) => return Err(error),
    };
    let columns = columns_at(format)?;
    if envelope::schema_digest_prefix(&schema, &columns) != Some(root.identity.schema) {
        return Err(differs(&format!("this build's and from its format {format}'s")));
    }
    Ok((columns, withheld_at(format)?))
}

/// A recording file's root alone: what its last footer whose root verifies names, the file's
/// length through that footer, and any extent the root names decoded on demand. Lists and
/// names a recording without reading its pages.
pub struct RootFile {
    source: Source,
    decoder: Decoder,
    root: Root,
    extent: Extent,
}

impl RootFile {
    pub fn open(file: File) -> io::Result<Self> {
        let (source, extent) = Source::open(Held::File(file))?;
        let mut decoder = Decoder::new()?;
        let mut raw = Vec::new();
        decoder.append(&source, &extent, &mut raw)?;
        let root = Root::read(&raw)?;
        Ok(Self {
            source,
            decoder,
            root,
            extent,
        })
    }

    pub fn root(&self) -> &Root {
        &self.root
    }

    /// The root's envelope, whose digest names the revision.
    pub fn root_extent(&self) -> &Extent {
        &self.extent
    }

    /// The file's length through the footer.
    pub fn len(&self) -> u64 {
        self.source.len()
    }

    /// `extent`, one the root names or one those name, decoded.
    pub fn raw(&mut self, extent: &Extent) -> io::Result<Vec<u8>> {
        let mut raw = Vec::new();
        self.decoder.append(&self.source, extent, &mut raw)?;
        Ok(raw)
    }
}

/// A columnar recording ([`replay_columnar::envelope`]), read from its file or memory: its root, every
/// run's directory and the restarts of every page, held from open. A run directory or page
/// that cannot be read refuses only what it covers ([`Self::damage`]); the rest stays
/// readable.
pub struct Recording {
    source: Arc<Source>,
    /// The root the footer names, and its envelope there.
    root: Root,
    root_extent: Extent,
    /// The identity block every run's chain starts from.
    identity: Vec<u8>,
    root_identity: envelope::Identity,
    /// The root's control records' extents.
    controls: Box<[Extent]>,
    /// The campaign its identity names.
    campaign: Vec<u8>,
    /// Each schema table's columns it holds: the first of this build's.
    columns: Box<[usize]>,
    /// The tables its format holds as this build reads them otherwise, each with why.
    withheld: Box<[(usize, String)]>,
    runs: Box<[Option<Arc<Run>>]>,
    /// The segments its walks take ([`Self::with_segments`]), and those it records.
    segments: Box<[Segment]>,
    recorded: Box<[Segment]>,
    damage: Box<[Damage]>,
    /// Whether walks check every hour against the pages' hour digest chain.
    verify: bool,
    /// The tables walks restore and apply.
    selected: [bool; TABLE_COUNT],
    /// Shared with the recordings selecting from it ([`Self::selecting`]).
    restarts: Arc<Restarts>,
    raws: Arc<Raws>,
}

/// Restart tables' raw bytes as walks decoded them, by run, restart ordinal and table: a
/// restart stored as a change from the one before is decoded from that one's, so walks
/// restoring a run's restarts each decode every restart once. The oldest go first past
/// [`RESTART_RAW_BYTES`].
#[derive(Default)]
struct Raws {
    held: Mutex<RawsHeld>,
}

#[derive(Default)]
struct RawsHeld {
    by_key: HashMap<(u32, u32, u32), Arc<Vec<u8>>, foldhash::fast::RandomState>,
    order: std::collections::VecDeque<(u32, u32, u32)>,
    bytes: usize,
}

/// The most restart bytes the cache holds: the restarts walks starting near one another share.
const RESTART_RAW_BYTES: usize = 64 << 20;

impl Raws {
    fn get(&self, key: (u32, u32, u32)) -> Option<Arc<Vec<u8>>> {
        locked(&self.held).by_key.get(&key).cloned()
    }

    fn put(&self, key: (u32, u32, u32), raw: Arc<Vec<u8>>) -> io::Result<()> {
        let mut held = locked(&self.held);
        if held.by_key.contains_key(&key) {
            return Ok(());
        }
        engine_allocation::reserve_map(&mut held.by_key, 1)?;
        held.order.try_reserve(1).map_err(|_| engine_allocation::oom())?;
        held.bytes += raw.len();
        held.by_key.insert(key, raw);
        held.order.push_back(key);
        while held.bytes > RESTART_RAW_BYTES {
            let Some(oldest) = held.order.pop_front() else {
                break;
            };
            if let Some(gone) = held.by_key.remove(&oldest) {
                held.bytes -= gone.len();
            }
        }
        Ok(())
    }
}

impl Recording {
    pub fn open(bytes: Vec<u8>) -> io::Result<Self> {
        Self::read(Held::Memory(bytes))
    }

    pub fn open_path(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::read(Held::File(File::open(path)?))
    }

    fn read(held: Held) -> io::Result<Self> {
        let unlocated = |error: io::Error| Damage::wrap(DamageScope::Unlocated, error);
        let (source, root_extent) = Source::open(held).map_err(unlocated)?;
        let source = Arc::new(source);
        let mut decoder = Decoder::new()?;
        let mut raw = Vec::new();
        decoder
            .append(&source, &root_extent, &mut raw)
            .map_err(unlocated)?;
        let root = Root::read(&raw).map_err(unlocated)?;
        let (columns, withheld) = recorded_columns(&root, || {
            let Some(first) = root.controls.first() else {
                return Ok(None);
            };
            let mut raw = Vec::new();
            decoder.append(&source, first, &mut raw)?;
            if let Some(format) = crate::campaign::Records::format_of(&raw) {
                return Ok(Some(format));
            }
            Ok(Some(crate::columnar_file::Capture::read(&raw)?.format))
        })?;
        let mut damage = Vec::new();
        let mut runs = engine_allocation::vec(root.runs.len())?;
        for (index, extent) in (0..).zip(&root.runs) {
            let run = Run::read(&source, &mut decoder, &mut raw, extent, index, &mut damage)?;
            runs.push(run.map(Arc::new));
        }
        let mut segments = Vec::new();
        for (index, run) in (0..).zip(&runs) {
            let Some(run) = run else {
                continue;
            };
            let mut start = (None, 0_u32);
            for restart in &run.restarts {
                if u64::from(restart.after) + 1 < u64::from(run.ordinals) {
                    engine_allocation::reserve(&mut segments, 1)?;
                    segments.push(Segment {
                        branch: run.branch,
                        run: index,
                        restart: start.0,
                        first: start.1,
                        last: restart.after,
                    });
                    start = (Some(restart.page), restart.after + 1);
                }
            }
            if start.1 < run.ordinals {
                engine_allocation::reserve(&mut segments, 1)?;
                segments.push(Segment {
                    branch: run.branch,
                    run: index,
                    restart: start.0,
                    first: start.1,
                    last: run.ordinals - 1,
                });
            }
        }
        Ok(Self {
            identity: root.identity.chain_bytes()?,
            controls: engine_allocation::boxed_slice(engine_allocation::copy_vec(&root.controls)?)?,
            campaign: root.identity.campaign.clone(),
            columns: engine_allocation::boxed_slice(columns)?,
            withheld: engine_allocation::boxed_slice(withheld)?,
            root_identity: root.identity.clone(),
            root,
            root_extent,
            source,
            runs: engine_allocation::boxed_slice(runs)?,
            recorded: engine_allocation::boxed_slice(engine_allocation::copy_vec(&segments)?)?,
            segments: engine_allocation::boxed_slice(segments)?,
            damage: engine_allocation::boxed_slice(damage)?,
            verify: false,
            selected: [true; TABLE_COUNT],
            restarts: Arc::default(),
            raws: Arc::default(),
        })
    }

    /// The recording as read from the same file, its runs, pages and the restarts its walks
    /// decoded shared with this one, whose walks read only `tables` ([`Self::select`]): for
    /// walks of a few tables beside the walks of this one.
    pub fn selecting(&self, tables: &[TableId]) -> io::Result<Self> {
        let mut damage = engine_allocation::vec(self.damage.len())?;
        for held in &*self.damage {
            damage.push(Damage {
                scope: held.scope.clone(),
                cause: io::Error::new(held.cause.kind(), held.cause.to_string()),
            });
        }
        let mut withheld = engine_allocation::vec(self.withheld.len())?;
        for (table, reason) in &*self.withheld {
            withheld.push((*table, engine_allocation::string(reason)?));
        }
        let mut recording = Self {
            source: Arc::clone(&self.source),
            root: self.root.clone(),
            root_extent: self.root_extent,
            columns: engine_allocation::boxed_slice(engine_allocation::copy_vec(&self.columns)?)?,
            withheld: engine_allocation::boxed_slice(withheld)?,
            identity: engine_allocation::copy_vec(&self.identity)?,
            root_identity: self.root_identity.clone(),
            controls: engine_allocation::boxed_slice(engine_allocation::copy_vec(&self.controls)?)?,
            campaign: engine_allocation::copy_vec(&self.campaign)?,
            runs: engine_allocation::boxed_slice(engine_allocation::collect_vec(
                self.runs.iter().cloned(),
            )?)?,
            segments: engine_allocation::boxed_slice(engine_allocation::copy_vec(&self.segments)?)?,
            recorded: engine_allocation::boxed_slice(engine_allocation::copy_vec(&self.recorded)?)?,
            damage: engine_allocation::boxed_slice(damage)?,
            verify: self.verify,
            selected: [false; TABLE_COUNT],
            restarts: Arc::clone(&self.restarts),
            raws: Arc::clone(&self.raws),
        };
        for table in tables {
            recording.selected[table.index()] = true;
        }
        Ok(recording)
    }

    /// Walks restore, read and apply only `tables`, the tables their readers read; the others
    /// keep their statuses and no rows. A walk that verifies still reads every unit.
    pub fn select(&mut self, tables: &[TableId]) {
        self.restarts = Arc::default();
        self.selected = [false; TABLE_COUNT];
        for table in tables {
            self.selected[table.index()] = true;
        }
    }

    fn selects(&self, table: usize) -> bool {
        self.selected.get(table).copied().unwrap_or(false) && !self.withholds(table)
    }

    fn withholds(&self, table: usize) -> bool {
        self.withheld.iter().any(|&(held, _)| held == table)
    }

    /// The tables it holds in a layout or meaning this build reads otherwise, each with why:
    /// walks never apply them, their states name them withheld ([`State::withheld`]) and
    /// isolating walks begin with them lost.
    pub fn withheld(&self) -> &[(usize, String)] {
        &self.withheld
    }

    /// Whether walks decode `extent`, a block's: its records, the selected tables', or every
    /// one when they verify.
    fn decodes(&self, extent: &Extent) -> bool {
        extent.kind == envelope::Kind::Records
            || self.verify
            || self.selects(extent.stream as usize)
    }

    /// The decoded and stored bytes of `page`'s units that walks decode together, but for the
    /// units of the tables `lost` names, each unit's raw length as its chunks' frames declare
    /// it, read from `gathered`'s bytes where it gathered them: one whose length is damage goes
    /// to `damaged`, counted as none.
    fn unit_lengths(
        &self,
        page: &IndexPage,
        lost: &[(usize, io::Error)],
        damaged: &mut Vec<(Extent, io::Error)>,
        gathered: Option<(&Decoder, &[usize])>,
    ) -> io::Result<(usize, usize)> {
        let (mut raw, mut stored) = (0_usize, 0_usize);
        for (slot, (extent, chunks)) in page.units().enumerate() {
            if !self.decodes(extent) || gone(lost, extent) {
                continue;
            }
            if let Ok(placed) = self.source.place(extent) {
                stored = stored.checked_add(placed).ok_or_else(engine_allocation::oom)?;
            }
            let held = match gathered.and_then(|(decoder, starts)| Some((decoder, *starts.get(slot)?))) {
                Some((decoder, at)) if at != usize::MAX => Some(decoder.gathered(extent, at)),
                _ => self.source.memory_stored(extent).ok(),
            };
            let length = match self.source.unit_length(extent, chunks, held) {
                Ok(length) => length,
                Err(error) => {
                    let error = Damage::wrap(DamageScope::Extent(*extent), error);
                    if Damage::of(&error).is_none() {
                        return Err(error);
                    }
                    engine_allocation::reserve(damaged, 1)?;
                    damaged.push((*extent, error));
                    continue;
                }
            };
            raw = raw.checked_add(length).ok_or_else(engine_allocation::oom)?;
        }
        Ok((raw, stored))
    }

    /// The most decoded and stored bytes one of the tables walks restore from `restart`, of
    /// run `run`, takes: a table stored as a change holds the restart it changes beside it.
    fn table_lengths(&self, run: u32, restart: &RestartDirectory) -> io::Result<(usize, usize)> {
        let (mut raw, mut stored) = (0_usize, 0_usize);
        for table in &restart.tables {
            if !self.selects(table.table as usize) {
                continue;
            }
            let length = self.source.raw_length(&table.extent).map_err(|error| {
                let scope = DamageScope::RestartTable {
                    run,
                    after: restart.after,
                    table: table.table,
                };
                Damage::wrap(scope, error)
            })?;
            raw = raw.max(if table.base { length.saturating_mul(2) } else { length });
            stored = stored
                .max(usize::try_from(table.extent.stored).map_err(|_| engine_allocation::oom())?);
        }
        Ok((raw, stored))
    }

    /// Applies the selection to a walk's `state`, clearing any an earlier walk left.
    fn select_in(&self, state: &mut State) -> io::Result<()> {
        let skipped: [bool; TABLE_COUNT] = std::array::from_fn(|table| !self.selects(table));
        state.skip(&skipped)?;
        state.keep_last(&super::keeps_last())
    }

    /// Makes every walk check each hour against the hour digest chain the pages hold.
    pub fn verify(&mut self, verify: bool) {
        self.verify = verify;
    }

    /// The campaign its identity names, as the row recording's header names it.
    pub fn campaign(&self) -> &[u8] {
        &self.campaign
    }

    /// Each schema table's columns it holds, as its manifests are read with: the first of
    /// this build's, the others unrecorded in every row it walks.
    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    /// Empty tables of this build's schema holding the columns it records, its withheld
    /// tables named so.
    pub fn state(&self) -> io::Result<State> {
        let mut state = State::recorded(schema()?, engine_allocation::copy_vec(&self.columns)?)?;
        for (table, reason) in &self.withheld {
            state.withhold(*table, Some(reason))?;
        }
        Ok(state)
    }

    /// Its identity block, every run's chain starts from: this build's schema, the campaign,
    /// the content identity and the capture mode.
    pub fn identity(&self) -> &[u8] {
        &self.identity
    }

    /// The capture the file's runs belong to, as the root's control records hold it; none
    /// when the root holds none.
    pub fn capture(&self) -> io::Result<Option<crate::columnar_file::Capture>> {
        let mut decoder = Decoder::new()?;
        let mut raw = Vec::new();
        if let Some(first) = self.controls.first() {
            decoder.append(&self.source, first, &mut raw)?;
            // A campaign's recording holds campaign records, not one capture's.
            if crate::campaign::Records::format_of(&raw).is_some() {
                return Ok(None);
            }
        }
        crate::columnar_file::read_capture(&self.controls, |extent| {
            let mut raw = Vec::new();
            decoder.append(&self.source, extent, &mut raw)?;
            Ok(raw)
        })
    }

    /// Why each run directory or page that cannot be read is refused.
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
        self.runs
            .iter()
            .flatten()
            .map(|run| u64::from(run.ordinals))
            .sum()
    }

    /// Runs in the recording, readable or not.
    pub fn run_count(&self) -> u32 {
        self.runs.len() as u32
    }

    /// Run `run`, when its directory can be read.
    pub fn run(&self, run: u32) -> io::Result<&Run> {
        self.shared(run).map(|run| &**run)
    }

    fn shared(&self, run: u32) -> io::Result<&Arc<Run>> {
        match self.runs.get(run as usize) {
            Some(Some(held)) => Ok(held),
            Some(None) => Err(Damage::wrap(
                DamageScope::Run(run),
                invalid("cannot be read"),
            )),
            None => Err(invalid("No such run")),
        }
    }

    /// The segments its walks take: those it records ([`Self::recorded_segments`]), unless
    /// given others ([`Self::with_segments`]).
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Every readable run's segments in order: from the run's start, then from each restart
    /// a readable page holds, each of its run's branch.
    pub fn recorded_segments(&self) -> &[Segment] {
        &self.recorded
    }

    /// The recording whose walks take `segments` in place of those it records, as a timeline
    /// reads them: each one a recorded segment, of any branch, its last ordinal kept or brought
    /// earlier. A segment may appear more than once.
    pub fn with_segments(mut self, segments: Vec<Segment>) -> io::Result<Self> {
        for segment in &segments {
            let held = self
                .recorded
                .binary_search_by_key(&(segment.run, segment.first), |held| (held.run, held.first))
                .map(|at| &self.recorded[at]);
            if !held.is_ok_and(|held| {
                held.restart == segment.restart
                    && segment.first <= segment.last
                    && segment.last <= held.last
            }) {
                return Err(invalid("A segment the recording does not hold"));
            }
        }
        self.segments = engine_allocation::boxed_slice(segments)?;
        Ok(self)
    }

    /// The page of `segment`'s run whose restart holds the segment's last hour, the run's
    /// last restart included; None when no readable page holds one.
    pub fn restart_after(&self, segment: &Segment) -> Option<usize> {
        let run = self.runs.get(segment.run as usize)?.as_ref()?;
        let at = run
            .restarts
            .partition_point(|restart| restart.after < segment.last);
        run.restarts
            .get(at)
            .filter(|restart| restart.after == segment.last)
            .map(|restart| restart.page)
    }

    /// The chain of run `run` through the block holding ordinal `ordinal`: after every block
    /// from the run's start to it, that one too. Blocks are written once, so a recording grown
    /// from this one holds the same there; a recording whose blocks to it differ does not.
    pub fn chain_through(&self, run: u32, ordinal: u32) -> io::Result<[u8; 32]> {
        let pages = &self.run(run)?.pages;
        let at = pages
            .partition_point(|page| page.first <= ordinal)
            .checked_sub(1)
            .filter(|&at| ordinal - pages[at].first < pages[at].hours)
            .ok_or_else(|| invalid("No page of the run holds the ordinal"))?;
        Ok(self.page(run, at)?.chain)
    }

    /// The root its footer names.
    pub fn root(&self) -> &Root {
        &self.root
    }

    /// The digest of the root its footer names, which names its revision.
    pub fn root_footer_digest(&self) -> io::Result<[u8; 16]> {
        Ok(self.root_extent.digest)
    }

    /// Run `run`'s page `at`'s units, raw: its hour records, then each table's block.
    pub fn page_units(&self, run: u32, at: usize) -> io::Result<Vec<(Extent, Vec<u8>)>> {
        let mut decoder = Decoder::new()?;
        let page = self.read_page(run, at, &mut decoder, &mut Vec::new())?;
        let mut units = engine_allocation::vec(1 + page.tables.len())?;
        for (extent, chunks) in page.units() {
            let mut raw = Vec::new();
            decoder
                .append_unit(&self.source, extent, chunks, &mut raw)
                .map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))?;
            units.push((*extent, raw));
        }
        Ok(units)
    }

    /// `out.len()` stored bytes of `extent` from `at` within it; its digest is the caller's to
    /// check, once, with [`Self::stored_bytes`].
    pub fn stored_at(&self, extent: &Extent, at: u64, out: &mut [u8]) -> io::Result<()> {
        self.source
            .stored_at(extent, at, out)
            .map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))
    }

    /// `extent`'s stored bytes, checked against its digest.
    pub fn stored_bytes(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        let mut buffer = Vec::new();
        let stored = self
            .source
            .stored_checked(extent, &mut buffer)
            .map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))?;
        engine_allocation::copy_vec(stored)
    }

    /// Every table's raw bytes at run `run`'s restart on page `at`, each read through the
    /// restarts it is a change from.
    pub fn restart_tables(&self, run: u32, at: usize) -> io::Result<Vec<Vec<u8>>> {
        let mut decoder = Decoder::new()?;
        let page = self.read_page(run, at, &mut decoder, &mut Vec::new())?;
        let restart = page.restart.as_ref().ok_or_else(|| invalid("The page holds no restart"))?;
        let chain = self.restart_chain(run, restart, &mut decoder)?;
        let mut tables = engine_allocation::vec(restart.tables.len())?;
        for table in &restart.tables {
            tables.push(engine_allocation::copy_vec(&self.restart_table(run, &chain, table.table, &mut decoder)?)?);
        }
        Ok(tables)
    }

    /// The restarts of run `run` from the one stored whole before `restart` through it.
    fn restart_chain(&self, run: u32, restart: &RestartDirectory, decoder: &mut Decoder) -> io::Result<Vec<RestartDirectory>> {
        let held = self.run(run)?;
        let keyframe = restart.ordinal - restart.ordinal % KEYFRAME_RESTARTS;
        let mut chain = engine_allocation::vec((restart.ordinal - keyframe + 1) as usize)?;
        for ordinal in keyframe..restart.ordinal {
            let scope = DamageScope::RestartTable {
                run,
                after: restart.after,
                table: 0,
            };
            let at = held
                .restarts
                .iter()
                .find(|candidate| candidate.ordinal == ordinal)
                .map(|candidate| candidate.page)
                .ok_or_else(|| Damage::wrap(scope, invalid("changes a restart on no readable page")))?;
            chain.push(
                self.read_page(run, at, decoder, &mut Vec::new())?
                    .restart
                    .ok_or_else(|| invalid("The page holds no restart"))?,
            );
        }
        chain.push(restart.clone());
        Ok(chain)
    }

    /// Table `table`'s raw bytes at the last of `chain`, run `run`'s restarts from one stored
    /// whole: each change applied in turn from the latest restart a walk decoded before.
    fn restart_table(&self, run: u32, chain: &[RestartDirectory], table: u32, decoder: &mut Decoder) -> io::Result<Arc<Vec<u8>>> {
        let key = |restart: &RestartDirectory| (run, restart.ordinal, table);
        // The cached bytes found are held from here on: another walk may evict them meanwhile.
        let found = chain
            .iter()
            .enumerate()
            .rev()
            .find_map(|(at, restart)| self.raws.get(key(restart)).map(|raw| (at, raw)));
        let start = found.as_ref().map_or(0, |(at, _)| at + 1);
        let mut raw = found.map(|(_, raw)| raw);
        if start == chain.len() {
            return raw.ok_or_else(|| invalid("A restart chain holds no restart"));
        }
        for (index, held_restart) in chain.iter().enumerate().skip(start) {
            let scope = DamageScope::RestartTable {
                run,
                after: held_restart.after,
                table,
            };
            let extent = held_restart
                .tables
                .get(table as usize)
                .filter(|entry| entry.table == table)
                .ok_or_else(|| invalid("A restart does not hold every table in order"))?;
            if extent.base != (index != 0) {
                return Err(Damage::wrap(scope, invalid("changes another restart than the one before it")));
            }
            let mut decoded = Vec::new();
            let base = raw.as_deref().map(Vec::as_slice).filter(|_| extent.base);
            decoder
                .append_with(&self.source, &extent.extent, base, &mut decoded)
                .map_err(|error| Damage::wrap(scope, error))?;
            let decoded = Arc::new(decoded);
            self.raws.put(key(held_restart), decoded.clone())?;
            raw = Some(decoded);
        }
        raw.ok_or_else(|| invalid("A restart chain holds no restart"))
    }

    /// `extent`, one stored as one frame, decoded: any but a page's units ([`Self::page_units`]).
    pub fn extent_bytes(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        if matches!(extent.kind, envelope::Kind::Records | envelope::Kind::Block) {
            return Err(invalid("A page's unit is read in its page's chunks"));
        }
        let mut raw = Vec::new();
        Decoder::new()?
            .append(&self.source, extent, &mut raw)
            .map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))?;
        Ok(raw)
    }

    /// The identity its root names.
    pub fn root_identity(&self) -> &envelope::Identity {
        &self.root_identity
    }

    /// The file's length through its last valid footer.
    pub fn len(&self) -> u64 {
        self.source.len()
    }

    /// Its bytes, when it was opened from memory.
    pub fn memory(&self) -> Option<&[u8]> {
        self.source.memory()
    }

    /// Run `run`'s page `at`, read again.
    pub fn page(&self, run: u32, at: usize) -> io::Result<IndexPage> {
        self.read_page(run, at, &mut Decoder::new()?, &mut Vec::new())
    }

    fn read_page(
        &self,
        run: u32,
        at: usize,
        decoder: &mut Decoder,
        raw: &mut Vec<u8>,
    ) -> io::Result<IndexPage> {
        let extent = self
            .run(run)?
            .pages
            .get(at)
            .ok_or_else(|| invalid("No such page"))?;
        raw.clear();
        decoder
            .append(&self.source, extent, raw)
            .and_then(|()| IndexPage::read(raw, extent, TABLE_COUNT))
            .map_err(|error| {
                let scope = DamageScope::Page {
                    run,
                    first: extent.first,
                    hours: extent.hours,
                };
                Damage::wrap(scope, error)
            })
    }

    /// Every readable run's hours in order, each run from empty tables: the walker's state
    /// once it applied the hour and that hour's manifest, with whether the hour starts its
    /// run. `visit` stops the walk by returning false; a page that cannot be read refuses
    /// the walk where it reaches it.
    pub fn walk(
        &self,
        state: &mut State,
        visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        Walker::new(self)?.walk_runs(state, visit)
    }

    /// `segment`'s hours from its restart restored into `state`, or from empty tables at its
    /// run's start, as [`Self::walk`] visits them; `starts` is the segment's first hour.
    pub fn walk_segment(
        &self,
        segment: &Segment,
        state: &mut State,
        visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let mut walker = Walker::new(self)?;
        walker.begin(segment, state)?;
        walker.walk(segment, state, visit).map(drop)
    }

    /// Replaces `state` with what restart page `at` of `run` holds, as [`Walker::restore`].
    pub fn restore(&self, run: u32, at: usize, state: &mut State) -> io::Result<()> {
        Walker::new(self)?.restore(run, at, state)
    }
}

/// The most a walker's buffers hold for one segment's walk: decoded bytes, and stored bytes
/// read from a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    raw: usize,
    stored: usize,
}

impl Reads {
    /// What a walk reading both `self` and `other`, one after the other, holds at most.
    pub(crate) fn max(self, other: Self) -> Self {
        Self {
            raw: self.raw.max(other.raw),
            stored: self.stored.max(other.stored),
        }
    }
}

/// The stored bytes of `run`'s page `at`'s index.
fn index_bytes(run: &Run, at: usize) -> io::Result<usize> {
    usize::try_from(run.pages[at].stored).map_err(|_| engine_allocation::oom())
}

/// The bytes the walkers counted by it hold at once, and the most they held: a statistic,
/// which nothing limits.
#[derive(Debug, Default)]
pub struct Tally {
    held: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
}

impl Tally {
    /// The most its walkers held at once.
    pub fn peak(&self) -> usize {
        self.peak.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn change(&self, from: usize, to: usize) {
        use std::sync::atomic::Ordering;
        if to >= from {
            let held = self.held.fetch_add(to - from, Ordering::AcqRel) + (to - from);
            self.peak.fetch_max(held, Ordering::AcqRel);
        } else {
            self.held.fetch_sub(from - to, Ordering::AcqRel);
        }
    }
}

/// Whether `extent` is the block of a table `lost` names.
fn gone(lost: &[(usize, io::Error)], extent: &Extent) -> bool {
    extent.kind == envelope::Kind::Block
        && lost
            .iter()
            .any(|&(table, _)| table == extent.stream as usize)
}

/// What the walk of `recording` skips: the tables it does not select, and those `lost`.
fn skipped(recording: &Recording, lost: &[(usize, io::Error)]) -> [bool; TABLE_COUNT] {
    std::array::from_fn(|table| {
        !recording.selects(table) || lost.iter().any(|&(held, _)| held == table)
    })
}

/// Runs `job` for each index below `count`, on any threads, every one before it returns.
pub(crate) type Spread<'s> = dyn Fn(usize, &(dyn Fn(usize) + Sync)) + 's;

thread_local! {
    /// The zstd context a thread decodes restart tables and chunks apart with, kept between them.
    static APART: std::cell::RefCell<Option<Decoder>> = const { std::cell::RefCell::new(None) };
}

/// `decode` with this thread's own decoder.
fn apart<T>(decode: impl FnOnce(&mut Decoder) -> io::Result<T>) -> io::Result<T> {
    APART.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() {
            *cell = Some(Decoder::new()?);
        }
        decode(cell.as_mut().ok_or_else(|| invalid("No decoder apart"))?)
    })
}

/// `job` of each index below `count`, run on `spread`, each in its place.
fn restore_apart<T: Send>(
    spread: &Spread<'_>,
    count: usize,
    job: impl Fn(usize) -> io::Result<T> + Sync,
) -> io::Result<Vec<io::Result<T>>> {
    let cells = engine_allocation::collect_vec((0..count).map(|_| Mutex::new(None)))?;
    spread(count, &|at| {
        *cells[at].lock().unwrap_or_else(PoisonError::into_inner) = Some(job(at));
    });
    Ok(cells
        .into_iter()
        .map(|cell| {
            cell.into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap_or_else(|| Err(invalid("A restart table was never decoded")))
        })
        .collect())
}

/// [`State::apply_block`], each of `parts` applied by its table's decoder of `decoders` through
/// `spread`; what fails is met in the parts' order, as one after another would meet it.
fn apply_spread(
    parts: &mut [Part<'_>],
    decoders: &mut [Option<TableDecoder<'_>>],
    lost: Option<&mut Vec<(usize, io::Error)>>,
    spread: &Spread<'_>,
) -> io::Result<()> {
    use engine_allocation::labels::{self, Kind, Owner};
    use std::sync::{Mutex, PoisonError};
    let isolate = lost.is_some();
    let mut held = engine_allocation::collect_vec(decoders.iter_mut().map(Option::as_mut))?;
    let mut jobs = engine_allocation::vec(parts.len())?;
    for part in parts.iter_mut() {
        let decoder = held.get_mut(part.table()).and_then(Option::take);
        jobs.push(Mutex::new((part, decoder, None)));
    }
    spread(jobs.len(), &|at| {
        let _memory = labels::enter(Owner::RUN, Kind::WalkerTables);
        let mut job = jobs[at].lock().unwrap_or_else(PoisonError::into_inner);
        let (part, decoder, outcome) = &mut *job;
        // Damage of the table's own block loses it once the hour is applied.
        let mut damage = None;
        let applied = part.apply(|entry, _, truncated, table, changes| {
            let decoder = decoder
                .as_mut()
                .ok_or_else(|| invalid("An hour names a table its block does not hold"))?;
            match decoder.hour(table, changes, entry, truncated) {
                Err(error) if isolate && error.kind() == io::ErrorKind::InvalidData => {
                    damage = Some(error);
                    Ok(())
                }
                applied => applied,
            }
        });
        *outcome = Some(applied.map(|()| damage));
    });
    let mut damaged = engine_allocation::vec(jobs.len())?;
    for job in jobs {
        let (part, _, outcome) = job.into_inner().unwrap_or_else(PoisonError::into_inner);
        match outcome {
            Some(Ok(None)) => {}
            Some(Ok(Some(error))) => damaged.push((part.table(), error)),
            Some(Err(error)) => return Err(error),
            None => return Err(io::Error::other("A part of an hour never ran")),
        }
    }
    drop(held);
    if let Some(lost) = lost {
        for (table, error) in damaged {
            decoders[table] = None;
            engine_allocation::reserve(lost, 1)?;
            lost.push((table, error));
        }
    }
    Ok(())
}

/// A timed walk's time, by part.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    /// Reading and decompressing the blocks.
    pub decompress_ns: u64,
    /// Reading the hour records.
    pub manifest_ns: u64,
    pub apply_ns: u64,
}

/// The time since `clock` read last, restarting it; none for an untimed walk.
fn lap(clock: &mut Option<Instant>) -> u64 {
    let Some(since) = clock else {
        return 0;
    };
    let now = Instant::now();
    let nanoseconds = now.duration_since(*since).as_nanos() as u64;
    *since = now;
    nanoseconds
}

/// What a worker walks a recording's segments with, kept from page to page and segment to
/// segment: its decoder and its buffers, which it gives back when it parks ([`Self::park`]).
pub struct Walker<'r> {
    recording: &'r Recording,
    decoder: Decoder,
    /// The page read last.
    raw: Vec<u8>,
    /// The block's records and tables the walk reads, decoded, one after another.
    arena: Vec<u8>,
    /// Where the records (0) and each table's block (`1 + table`) sit in the arena.
    units: Vec<Option<Range<usize>>>,
    /// The page's units it gathers from a file, as (offset, stored length, place among the
    /// page's units), and where each one's stored bytes begin in its decoder's buffer.
    spans: Vec<(u64, usize, usize)>,
    starts: Vec<usize>,
    /// Where restores decode their tables, and walks with no spread of their own a page's
    /// chunks, apart; none decodes them in turn.
    spread: Option<fn(usize, &(dyn Fn(usize) + Sync))>,
    /// Tables an isolating walk lost to damage of their own since it began, with the damage.
    lost: Vec<(usize, io::Error)>,
    /// The walk its buffers are sized for ([`Self::hold`]), and what it counts of them.
    reads: Option<Reads>,
    tally: Option<&'r Tally>,
    counted: usize,
    timed: bool,
    timings: Timings,
}

impl Drop for Walker<'_> {
    fn drop(&mut self) {
        self.count(0);
    }
}

impl<'r> Walker<'r> {
    /// A walker of `recording`.
    pub fn new(recording: &'r Recording) -> io::Result<Self> {
        Ok(Self {
            recording,
            decoder: Decoder::new()?,
            raw: Vec::new(),
            arena: Vec::new(),
            units: engine_allocation::collect_vec((0..=TABLE_COUNT).map(|_| None))?,
            spans: Vec::new(),
            starts: Vec::new(),
            spread: None,
            lost: Vec::new(),
            reads: None,
            tally: None,
            counted: 0,
            timed: false,
            timings: Timings::default(),
        })
    }

    /// A walker of `recording` whose buffers `tally` counts.
    pub(crate) fn counted(recording: &'r Recording, tally: &'r Tally) -> io::Result<Self> {
        let mut walker = Self::new(recording)?;
        walker.tally = Some(tally);
        Ok(walker)
    }

    fn count(&mut self, bytes: usize) {
        if let Some(tally) = self.tally {
            tally.change(self.counted, bytes);
        }
        self.counted = bytes;
    }

    /// Sizes it for a walk reading `reads`: its buffers are made to fit as the walk starts.
    pub(crate) fn hold(&mut self, reads: Reads) -> bool {
        self.reads = Some(reads);
        let bytes = [reads.raw, reads.stored, self.decoder.context()]
            .into_iter()
            .fold(0, usize::saturating_add);
        self.count(self.counted.max(bytes));
        true
    }

    /// Gives back its buffers, and the walk it was sized for.
    pub fn park(&mut self) {
        self.arena = Vec::new();
        self.decoder.release();
        self.reads = None;
        self.count(0);
    }

    /// Whether it is sized for a walk.
    pub(crate) fn holds(&self) -> bool {
        self.reads.is_some()
    }

    /// Starts afresh but for the walk it is sized for: what a panic unwound through is not
    /// used again.
    pub(crate) fn renew(&mut self) -> io::Result<()> {
        self.decoder = Decoder::new()?;
        self.raw = Vec::new();
        self.arena = Vec::new();
        self.units.fill(None);
        self.lost.clear();
        Ok(())
    }

    /// The tables its isolating walks lost since it last began a segment, each with its
    /// damage, in the order they were lost ([`Self::walk_isolated`]).
    pub fn lost(&self) -> &[(usize, io::Error)] {
        &self.lost
    }

    /// Takes [`Self::lost`]: the tables stay skipped until it next begins a segment.
    pub fn take_lost(&mut self) -> Vec<(usize, io::Error)> {
        std::mem::take(&mut self.lost)
    }

    /// Forgets the tables lost so far, but those the recording withholds: each lost from the
    /// start, as damage naming the table and why.
    fn lose_withheld(&mut self) -> io::Result<()> {
        self.lost.clear();
        engine_allocation::reserve(&mut self.lost, self.recording.withheld.len())?;
        for (table, reason) in self.recording.withheld.iter() {
            let scope = DamageScope::Withheld {
                table: u32::try_from(*table).map_err(|_| invalid("A table passes 2^32"))?,
            };
            let error = Damage::wrap(scope, invalid(reason.as_str()));
            self.lost.push((*table, error));
        }
        Ok(())
    }

    /// Keeps `error`, met reading `extent`, as its table's loss when the walk isolates damage
    /// and the damage is that table's unit's alone; anything else refuses the walk.
    fn lose(&mut self, extent: &Extent, error: io::Error, isolate: bool) -> io::Result<()> {
        if !isolate || extent.kind == envelope::Kind::Records || Damage::of(&error).is_none() {
            return Err(error);
        }
        engine_allocation::reserve(&mut self.lost, 1)?;
        self.lost.push((extent.stream as usize, error));
        Ok(())
    }

    /// What restoring `segment`'s restart holds in its buffers at most, from its page's index:
    /// its largest table the walk reads, and the index itself; nothing for a segment starting
    /// its run. Damage counts as nothing read: a walk beginning there goes back before it.
    pub(crate) fn measure_restart(&mut self, segment: &Segment) -> io::Result<Reads> {
        let Some(at) = segment.restart else {
            return Ok(Reads::default());
        };
        let recording = self.recording;
        let kept = self.decoder.stored();
        let mut reads = Reads {
            raw: 0,
            stored: index_bytes(recording.run(segment.run)?, at)?,
        };
        match recording.read_page(segment.run, at, &mut self.decoder, &mut self.raw) {
            Ok(page) => {
                let restart = page
                    .restart
                    .as_ref()
                    .ok_or_else(|| invalid("The page holds no restart"))?;
                match recording.table_lengths(segment.run, restart) {
                    Ok((raw, stored)) => reads = reads.max(Reads { raw, stored }),
                    Err(error) if Damage::of(&error).is_some() => {}
                    Err(error) => return Err(error),
                }
            }
            Err(error) if Damage::of(&error).is_some() => {}
            Err(error) => return Err(error),
        }
        Ok(self.measured(kept, reads))
    }

    /// What walking `segment`'s own pages holds in its buffers at most, from their indexes:
    /// each page's units the walk reads, and the indexes themselves. Damage counts as nothing
    /// read: a walk reaching it refuses what it covers there, as an isolating walk does.
    pub(crate) fn measure_pages(&mut self, segment: &Segment) -> io::Result<Reads> {
        let recording = self.recording;
        let run = recording.run(segment.run)?;
        let kept = self.decoder.stored();
        let mut reads = Reads::default();
        let mut damaged = Vec::new();
        for at in run.pages_of(segment) {
            reads = reads.max(Reads {
                raw: 0,
                stored: index_bytes(run, at)?,
            });
            let page = match recording.read_page(segment.run, at, &mut self.decoder, &mut self.raw)
            {
                Ok(page) => page,
                Err(error) if Damage::of(&error).is_some() => continue,
                Err(error) => return Err(error),
            };
            let (raw, stored) = recording.unit_lengths(&page, &[], &mut damaged, None)?;
            damaged.clear();
            reads = reads.max(Reads { raw, stored });
        }
        Ok(self.measured(kept, reads))
    }

    /// `reads`, once the indexes read for them are given back: stored bytes count only for a
    /// recording read from a file.
    fn measured(&mut self, kept: usize, reads: Reads) -> Reads {
        // The indexes read here are no part of what it is sized for.
        if self.decoder.stored() > kept {
            self.decoder.release();
        }
        if self.recording.source.reads_file() {
            reads
        } else {
            Reads { stored: 0, ..reads }
        }
    }

    /// Sizes its buffers for every read of the walk it is sized for, before the walk starts.
    fn fit(&mut self) -> io::Result<()> {
        let Some(reads) = self.reads else {
            return Ok(());
        };
        if self.arena.capacity() < reads.raw {
            self.arena = Vec::new();
            self.arena = engine_allocation::vec(reads.raw)?;
        }
        self.decoder.reserve(reads.stored)
    }

    /// Times its walks from now, from zero.
    pub fn time(&mut self) {
        self.timed = true;
        self.timings = Timings::default();
    }

    /// What its walks took since [`Self::time`].
    pub fn timings(&self) -> Timings {
        self.timings
    }

    /// Makes its buffers hold `raw` decoded bytes and, for a recording read from a file,
    /// `stored` stored bytes.
    fn admit(&mut self, raw: usize, stored: usize) -> io::Result<()> {
        if raw > self.arena.capacity() {
            self.arena = Vec::new();
            self.arena = engine_allocation::vec(raw)?;
        }
        if self.recording.source.reads_file() && stored > self.decoder.stored() {
            self.decoder.reserve(stored)?;
        }
        Ok(())
    }

    /// Reads from a file the stored bytes of `page`'s units the walk decodes, of no table it
    /// lost and within the recording's extents, a run of them lying end to end at a time.
    fn gather(&mut self, page: &IndexPage) -> io::Result<()> {
        let recording = self.recording;
        self.spans.clear();
        self.starts.clear();
        engine_allocation::resize(&mut self.starts, 1 + page.tables.len(), usize::MAX)?;
        for (slot, (extent, _)) in page.units().enumerate() {
            if !recording.decodes(extent) || gone(&self.lost, extent) {
                continue;
            }
            if let Ok(length) = recording.source.place(extent) {
                engine_allocation::reserve(&mut self.spans, 1)?;
                self.spans.push((extent.offset, length, slot));
            }
        }
        self.decoder
            .gather(&recording.source, &mut self.spans, &mut self.starts)
    }

    /// Decodes `page`'s records and the blocks of the tables the walk reads into the arena,
    /// sized by their declared lengths, their chunks apart on `spread` when given; a walk that
    /// `isolate`s damage leaves out a table whose block it cannot read, and every later block of
    /// a table it lost.
    fn load(&mut self, page: &IndexPage, isolate: bool, spread: Option<&Spread<'_>>) -> io::Result<()> {
        let recording = self.recording;
        let source = &recording.source;
        // A gather that cannot be read leaves each unit to be read alone, as it refuses.
        let gathered = source.reads_file() && self.gather(page).is_ok();
        let mut damaged = Vec::new();
        let (raw, stored) = recording.unit_lengths(
            page,
            &self.lost,
            &mut damaged,
            gathered.then_some((&self.decoder, &self.starts[..])),
        )?;
        for (extent, error) in damaged {
            self.lose(&extent, error, isolate)?;
        }
        self.admit(raw, if gathered { 0 } else { stored })?;
        self.arena.clear();
        self.units.fill(None);
        match spread {
            Some(spread) if gathered || !source.reads_file() => {
                self.load_apart(page, isolate, gathered, spread)
            }
            _ => self.load_in_turn(page, isolate, gathered),
        }
    }

    /// [`Self::load`]'s units decoded one after another on its own decoder.
    fn load_in_turn(&mut self, page: &IndexPage, isolate: bool, gathered: bool) -> io::Result<()> {
        let recording = self.recording;
        for (slot, (extent, chunks)) in page.units().enumerate() {
            if !recording.decodes(extent) || gone(&self.lost, extent) {
                continue;
            }
            let start = self.arena.len();
            let decoded = match self.starts.get(slot) {
                Some(&at) if gathered && at != usize::MAX => {
                    self.decoder.append_unit_gathered(extent, chunks, at, &mut self.arena)
                }
                _ => self.decoder.append_unit(&recording.source, extent, chunks, &mut self.arena),
            };
            if let Err(error) = decoded {
                self.arena.truncate(start);
                let error = Damage::wrap(DamageScope::Extent(*extent), error);
                self.lose(extent, error, isolate)?;
                continue;
            }
            self.units[unit_place(extent)] = Some(start..self.arena.len());
        }
        Ok(())
    }

    /// [`Self::load`]'s units, each gathered or in memory: every one's digest checked and every
    /// chunk decoded into its place in the arena, all apart on `spread`.
    fn load_apart(
        &mut self,
        page: &IndexPage,
        isolate: bool,
        gathered: bool,
        spread: &Spread<'_>,
    ) -> io::Result<()> {
        let recording = self.recording;
        let (decoder, starts, arena) = (&self.decoder, &self.starts, &mut self.arena);
        // Each unit: its envelope, chunks, stored bytes and place in the arena, and its failure.
        let mut wanted = Vec::new();
        let mut total = 0_usize;
        for (slot, (extent, chunks)) in page.units().enumerate() {
            if !recording.decodes(extent) || gone(&self.lost, extent) {
                continue;
            }
            let stored = match starts.get(slot) {
                Some(&at) if gathered && at != usize::MAX => Ok(decoder.gathered(extent, at)),
                _ => recording.source.memory_stored(extent),
            };
            let held = chunks.iter().map(|&chunk| chunk as usize).sum::<usize>();
            let (stored, failure) = match stored {
                Ok(stored) if chunks.len() as u64 == envelope::chunk_count(extent.raw) && held == stored.len() => {
                    (stored, None)
                }
                Ok(_) => (&[][..], Some(invalid("differs from its chunks"))),
                Err(error) => (&[][..], Some(error)),
            };
            let raw = usize::try_from(extent.raw).map_err(|_| engine_allocation::oom())?;
            let end = total.checked_add(raw).ok_or_else(engine_allocation::oom)?;
            engine_allocation::reserve(&mut wanted, 1)?;
            wanted.push((extent, chunks, stored, total..end, failure));
            total = end;
        }
        engine_allocation::reserve(arena, total)?;
        let mut spare = &mut arena.spare_capacity_mut()[..total];
        // A digest check (no place) or a chunk: its unit, stored bytes, place and outcome.
        let mut jobs = Vec::new();
        for (unit, (extent, chunks, stored, place, failure)) in wanted.iter().enumerate() {
            let stored: &[u8] = stored;
            let (mut into, rest) = std::mem::take(&mut spare).split_at_mut(place.len());
            spare = rest;
            if failure.is_some() {
                into.fill(MaybeUninit::new(0));
                continue;
            }
            engine_allocation::reserve(&mut jobs, 1 + chunks.len())?;
            jobs.push(Mutex::new((unit, stored, None, None)));
            for (stored_at, raw_at) in chunk_places(extent.raw, chunks) {
                let (chunk, rest) = std::mem::take(&mut into).split_at_mut(raw_at.len());
                into = rest;
                jobs.push(Mutex::new((unit, &stored[stored_at], Some(chunk), None)));
            }
        }
        spread(jobs.len(), &|at| {
            let mut job = jobs[at].lock().unwrap_or_else(PoisonError::into_inner);
            let (unit, stored, into, outcome) = &mut *job;
            *outcome = Some(match into {
                Some(into) => apart(|decoder| decoder.decode_chunk(stored, into)),
                None => check(wanted[*unit].0, stored),
            });
        });
        let mut decoded = engine_allocation::vec(jobs.len())?;
        for job in jobs {
            let (unit, _, into, outcome) = job.into_inner().unwrap_or_else(PoisonError::into_inner);
            let outcome = outcome.unwrap_or_else(|| Err(io::Error::other("A chunk was never decoded")));
            if let Err(error) = outcome
                && wanted[unit].4.is_none()
            {
                wanted[unit].4 = Some(error);
            }
            decoded.push((unit, into));
        }
        // A unit that failed holds bytes no frame wrote: they are made defined, then let go.
        for (unit, into) in decoded {
            if let Some(into) = into
                && wanted[unit].4.is_some()
            {
                into.fill(MaybeUninit::new(0));
            }
        }
        // SAFETY: every unit's place is written whole: by its every chunk, else filled above.
        unsafe { arena.set_len(total) };
        let mut outcomes = engine_allocation::vec(wanted.len())?;
        for (extent, _, _, place, failure) in wanted {
            outcomes.push((*extent, place, failure));
        }
        for (extent, place, failure) in outcomes {
            match failure {
                None => self.units[unit_place(&extent)] = Some(place),
                Some(error) => {
                    let error = Damage::wrap(DamageScope::Extent(extent), error);
                    self.lose(&extent, error, isolate)?;
                }
            }
        }
        Ok(())
    }

    /// The bytes of the unit at `place` ([`unit_place`]) of the page it loaded last.
    #[cfg(test)]
    pub(crate) fn loaded(&self, place: usize) -> Option<&[u8]> {
        self.units.get(place)?.clone().map(|range| &self.arena[range])
    }

    /// Decodes page `at` of run `run`'s units the walk reads into its buffers alone, as a walk
    /// opening the page does, their chunks apart where restores decode apart
    /// ([`Self::decode_apart`]): no other page is read. The bytes decoded.
    pub fn load_page(&mut self, run: u32, at: usize) -> io::Result<usize> {
        let page = self.recording.read_page(run, at, &mut self.decoder, &mut self.raw)?;
        let own = self.spread;
        self.load(&page, false, own.as_ref().map(|spread| spread as &Spread<'_>))?;
        Ok(self.arena.len())
    }

    /// Puts into `state` what `segment`'s first hour applies to: its restart's tables, or
    /// empty tables at its run's start.
    pub fn begin(&mut self, segment: &Segment, state: &mut State) -> io::Result<()> {
        self.fit()?;
        self.lose_withheld()?;
        let Some(at) = segment.restart else {
            let recording = self.recording;
            let run = recording.shared(segment.run)?;
            *state = recording.state()?;
            state.set_dictionary(Dictionary::run(run.clone(), 0));
            return recording.select_in(state);
        };
        self.restore(segment.run, at, state)
    }

    /// Replaces `state` with what restart page `at` of `run` holds: each table the recording
    /// selects, and every dictionary chunk they declare, each live text cell and reason in
    /// one of them.
    pub fn restore(&mut self, run: u32, at: usize, state: &mut State) -> io::Result<()> {
        let recording = self.recording;
        let held = recording.shared(run)?;
        let page = recording.read_page(run, at, &mut self.decoder, &mut self.raw)?;
        let restart = page
            .restart
            .as_ref()
            .ok_or_else(|| invalid("The page holds no restart"))?;
        if i64::from(restart.hour) != i64::from(held.first_hour) + i64::from(restart.after) {
            let scope = DamageScope::Page {
                run,
                first: page.first,
                hours: page.hours,
            };
            return Err(Damage::wrap(
                scope,
                invalid("holds a restart off its run's hours"),
            ));
        }
        if held.restarts[..held.valid]
            .binary_search_by_key(&at, |restart| restart.page)
            .is_err()
        {
            let scope = DamageScope::Dictionary {
                run,
                chunk: restart.ordinal,
            };
            return Err(Damage::wrap(
                scope,
                invalid("does not hold the run's next dictionary ids"),
            ));
        }
        let len = usize::try_from(u64::from(restart.ids.0) + u64::from(restart.ids.1))
            .map_err(|_| engine_allocation::oom())?;
        let tables = || {
            restart
                .tables
                .iter()
                .filter(move |table| recording.selects(table.table as usize))
        };
        let scope = |table: u32| DamageScope::RestartTable {
            run,
            after: restart.after,
            table,
        };
        let (raw, stored) = recording.table_lengths(run, restart)?;
        self.admit(raw, stored)?;
        self.lose_withheld()?;
        *state = recording.state()?;
        recording.select_in(state)?;
        state.set_dictionary(Dictionary::run(held.clone(), len));
        let chain = recording.restart_chain(run, restart, &mut self.decoder)?;
        // One walk decodes a restart while the others starting there wait to share it.
        let entry = recording.restarts.entry(run, at)?;
        let mut decoded = locked(&entry);
        // Each table's chunks pinned, and the tables no walk shares decoded after.
        let mut wanted: Vec<(&RestartTable, Vec<(u32, u32)>)> = Vec::new();
        for table in tables() {
            let mut declared = Vec::new();
            for &ordinal in &table.chunks {
                let at = held.chunk_at(ordinal)?;
                state.pin_strings(held.load(at)?)?;
                engine_allocation::reserve(&mut declared, 1)?;
                declared.push(held.restarts[at].ids);
            }
            let index = table.table as usize;
            let shared = match decoded.iter().find(|(held, _)| *held == index) {
                Some((_, restored)) => restored.upgrade()?,
                None => None,
            };
            match shared {
                Some(restored) => {
                    let damaged = |error: io::Error| Damage::wrap(scope(table.table), error);
                    state.restore_restored(index, restored).map_err(damaged)?;
                }
                None => {
                    engine_allocation::reserve(&mut wanted, 1)?;
                    wanted.push((table, declared));
                }
            }
        }
        let made = match self.spread {
            Some(spread) => {
                let state = &*state;
                restore_apart(&spread, wanted.len(), |at| {
                    let _restoring = engine_allocation::labels::kind(
                        engine_allocation::labels::Kind::WalkerTables,
                    );
                    let table = wanted[at].0;
                    apart(|decoder| {
                        let raw = recording.restart_table(run, &chain, table.table, decoder)?;
                        state.decode_with(table.table as usize, &raw, &mut Scratch::default())
                    })
                })?
            }
            None => {
                let mut made = engine_allocation::vec(wanted.len())?;
                for (table, _) in &wanted {
                    let _restoring = engine_allocation::labels::kind(
                        engine_allocation::labels::Kind::WalkerTables,
                    );
                    made.push(
                        recording
                            .restart_table(run, &chain, table.table, &mut self.decoder)
                            .and_then(|raw| state.decode(table.table as usize, &raw)),
                    );
                }
                made
            }
        };
        for ((table, declared), restored) in wanted.iter().zip(made) {
            let index = table.table as usize;
            let damaged = |error: io::Error| Damage::wrap(scope(table.table), error);
            let restored = restored.map_err(damaged)?;
            let weak = restored.downgrade()?;
            state.restore_restored(index, restored).map_err(damaged)?;
            state
                .text_ids(index, |id| {
                    let at = declared.partition_point(|&(first, _)| first <= id);
                    match at.checked_sub(1) {
                        Some(at) if id - declared[at].0 < declared[at].1 => Ok(()),
                        _ => Err(invalid("names a dictionary chunk outside its dependencies")),
                    }
                })
                .map_err(damaged)?;
            decoded.retain(|(held, _)| *held != index);
            engine_allocation::reserve(&mut *decoded, 1)?;
            decoded.push((index, weak));
        }
        drop(decoded);
        state.trim_dictionary()
    }

    /// Every readable run's hours in order, each from empty tables at its start, as
    /// [`Recording::walk`] visits them.
    pub fn walk_runs(
        &mut self,
        state: &mut State,
        mut visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let recording = self.recording;
        for (index, run) in (0..).zip(recording.runs.iter()) {
            let Some(run) = run.as_ref().filter(|run| run.ordinals != 0) else {
                continue;
            };
            let segment = Segment {
                branch: run.branch,
                run: index,
                restart: None,
                first: 0,
                last: run.ordinals - 1,
            };
            self.begin(&segment, state)?;
            if !self.walk(&segment, state, &mut visit)? {
                break;
            }
        }
        Ok(())
    }

    /// The hours of `segment` from `state` as [`Self::begin`] left it: one manifest per ordinal
    /// through its last, at its branch and hour, every manifest and section within the piece
    /// covering its ordinal, every unit of a page walked whole read to its end, and the chain
    /// checked when asked.
    /// `visit` sees each hour once it is applied, with whether it is the walk's first; it
    /// stops the walk by returning false, and the walk then returns false.
    pub fn walk(
        &mut self,
        segment: &Segment,
        state: &mut State,
        mut visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<bool> {
        self.walk_with(segment, state, false, None, |state, manifest, first, _| {
            visit(state, manifest, first)
        })
    }

    /// [`Self::walk`], but damage to a table's own unit, or to its sections' framing, refuses
    /// that table alone: its sections are left unapplied from there on, [`Self::lost`] names
    /// it, and the walk goes on with the rest. `visit` also sees the tables lost so far. A
    /// unit holding more than its hours is found at its page's end, once its hours were
    /// visited. A walk checking the hour digest chain needs every section: it refuses itself
    /// as [`Self::walk`] does.
    pub fn walk_isolated(
        &mut self,
        segment: &Segment,
        state: &mut State,
        visit: impl FnMut(&State, &Manifest<'_>, bool, &[(usize, io::Error)]) -> io::Result<bool>,
    ) -> io::Result<bool> {
        self.walk_with(segment, state, true, None, visit)
    }

    /// [`Self::walk_isolated`], each hour's tables applied apart by `spread`, which runs `job`
    /// for each index below `count`, on any threads, every one before it returns.
    pub(crate) fn walk_spread(
        &mut self,
        segment: &Segment,
        state: &mut State,
        spread: &Spread<'_>,
        visit: impl FnMut(&State, &Manifest<'_>, bool, &[(usize, io::Error)]) -> io::Result<bool>,
    ) -> io::Result<bool> {
        self.walk_with(segment, state, true, Some(spread), visit)
    }

    fn walk_with<V>(
        &mut self,
        segment: &Segment,
        state: &mut State,
        isolate: bool,
        spread: Option<&Spread<'_>>,
        mut visit: V,
    ) -> io::Result<bool>
    where
        V: FnMut(&State, &Manifest<'_>, bool, &[(usize, io::Error)]) -> io::Result<bool>,
    {
        self.fit()?;
        let recording = self.recording;
        let isolate = isolate && !recording.verify;
        let run = recording.run(segment.run)?;
        let mut chain = self.chain_before(segment)?;
        let mut first = true;
        for at in run.pages_of(segment) {
            let own = self.spread;
            let decoding = spread.or(own.as_ref().map(|own| own as &Spread<'_>));
            let mut page = self.open_page(
                segment.run,
                at,
                segment.branch,
                segment.last,
                &mut chain,
                state,
                isolate,
                decoding,
            )?;
            while !page.done() {
                self.page_hour(&mut page, state, isolate, spread)?;
                if page.done() && page.whole() {
                    self.check_page(&page, state, isolate)?;
                }
                let go = page.manifest(|manifest| visit(state, manifest, first, &self.lost))?;
                // The visit's time is no part of the walk's.
                lap(&mut page.clock);
                if !go {
                    self.leave(page);
                    return Ok(false);
                }
                first = false;
            }
            self.close_page(page, state)?;
        }
        Ok(true)
    }

    /// The hour digest chain a walk of `segment` starts from: its restart page's, or none at
    /// its run's start.
    fn chain_before(&mut self, segment: &Segment) -> io::Result<[u8; 32]> {
        Ok(match segment.restart {
            Some(at) => {
                self.recording
                    .read_page(segment.run, at, &mut self.decoder, &mut self.raw)?
                    .chain
            }
            None => [0; 32],
        })
    }

    /// Opens page `at` of run `run` once the hours before it are applied to `state`: its hours
    /// checked against its ordinals and its chain against `chain`, the chain through the page
    /// before, which then holds its own; its units decoded into the arena, which the page
    /// holds until it is closed ([`Self::close_page`]) or left ([`Self::leave`]). Its hours
    /// read as `branch`'s, through ordinal `last` at most; its chunks decoded apart on `spread`
    /// when given.
    #[allow(clippy::too_many_arguments)]
    fn open_page(
        &mut self,
        run: u32,
        at: usize,
        branch: u64,
        last: u32,
        chain: &mut [u8; 32],
        state: &mut State,
        isolate: bool,
        spread: Option<&Spread<'_>>,
    ) -> io::Result<Page> {
        let recording = self.recording;
        let held = recording.run(run)?;
        let page = recording.read_page(run, at, &mut self.decoder, &mut self.raw)?;
        let scope = page_scope(&page);
        let damaged = |error: io::Error| Damage::wrap(scope.clone(), error);
        let hour = |ordinal: u64| i64::from(held.first_hour) + ordinal as i64;
        let end = u64::from(page.first) + u64::from(page.hours);
        if i64::from(page.first_hour) != hour(page.first.into())
            || i64::from(page.last_hour) != hour(end - 1)
        {
            return Err(damaged(invalid("names hours off its ordinals")));
        }
        let next = envelope::chain_block(chain, &page.records, &page.tables);
        if next != page.chain {
            return Err(damaged(invalid("differs from its chain")));
        }
        *chain = next;
        let mut clock = self.timed.then(Instant::now);
        let losing = self.lost.len();
        self.load(&page, isolate, spread)?;
        if self.lost.len() != losing {
            state.skip(&skipped(recording, &self.lost))?;
        }
        self.timings.decompress_ns += lap(&mut clock);
        let columns = engine_allocation::copy_vec(state.columns())?;
        let kinds = schema()?;
        let records = self.units[0]
            .clone()
            .ok_or_else(|| damaged(invalid("holds no hour records")))?;
        let units = Units {
            arena: std::mem::take(&mut self.arena),
            at: engine_allocation::collect_vec(self.units.iter().cloned())?,
        };
        let lost = &mut self.lost;
        let decoding = Decoding::try_new::<io::Error>(Arc::new(units), |units| {
            let records = RecordsDecoder::new(&columns, &units.arena[records]).map_err(damaged)?;
            if records.hours() != page.hours {
                return Err(damaged(invalid("holds hour records of other hours")));
            }
            if page.hours == 0 {
                return Err(damaged(invalid("holds no hours")));
            }
            let mut tables: Vec<Option<TableDecoder<'_>>> =
                engine_allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
            for table in 0..TABLE_COUNT {
                let Some(range) = units.at[table + 1].clone() else {
                    continue;
                };
                match TableDecoder::new(&kinds[table][..columns[table]], &units.arena[range]) {
                    Ok(decoder) => tables[table] = Some(decoder),
                    Err(error) if !isolate => {
                        return Err(Damage::wrap(unit_scope(&page, table), error));
                    }
                    Err(error) => {
                        engine_allocation::reserve(lost, 1)?;
                        lost.push((table, Damage::wrap(unit_scope(&page, table), error)));
                        state.skip(&skipped(recording, lost))?;
                    }
                }
            }
            Ok(Decoders {
                records,
                manifest: Manifest {
                    branch,
                    hour: 0,
                    flags: 0,
                    total_ns: 0,
                    pool_bytes: 0,
                    dropped_frames: 0,
                    appends: Vec::new(),
                    statuses: Vec::new(),
                    read: Vec::new(),
                    samples: Vec::new(),
                    sections: Vec::new(),
                },
                tables,
            })
        })?;
        Ok(Page {
            next: u64::from(page.first),
            end,
            // A segment brought earlier ends inside its last page.
            through: end.min(u64::from(last) + 1),
            branch,
            index: page,
            decoding,
            failed: Vec::new(),
            clock,
        })
    }

    /// Applies `page`'s next hour to `state`, its tables apart through `spread` when given; the
    /// page's manifest is then that hour's.
    fn page_hour(
        &mut self,
        page: &mut Page,
        state: &mut State,
        isolate: bool,
        spread: Option<&Spread<'_>>,
    ) -> io::Result<()> {
        let recording = self.recording;
        let held = recording.run(page.index.run)?;
        let Page {
            index,
            decoding,
            next,
            failed,
            clock,
            branch,
            ..
        } = page;
        let scope = page_scope(index);
        let damaged = |error: io::Error| Damage::wrap(scope.clone(), error);
        let at = i32::try_from(i64::from(held.first_hour) + *next as i64)
            .map_err(|_| damaged(invalid("names an hour past 2^31")))?;
        let (lost, timings) = (&mut self.lost, &mut self.timings);
        decoding.with_dependent_mut(|_, decoders| -> io::Result<()> {
            let Decoders {
                records,
                manifest,
                tables,
            } = decoders;
            records.hour(*branch, at, manifest).map_err(damaged)?;
            for section in &manifest.sections {
                if (recording.verify || !state.skips(section.table))
                    && tables[section.table].is_none()
                {
                    return Err(damaged(invalid("names a table its block does not hold")));
                }
            }
            timings.manifest_ns += lap(clock);
            let applying =
                engine_allocation::labels::kind(engine_allocation::labels::Kind::WalkerTables);
            let ordered = manifest
                .sections
                .windows(2)
                .all(|pair| pair[0].table < pair[1].table);
            let failing = isolate.then_some(&mut *failed);
            match spread.filter(|_| ordered) {
                Some(spread) => state.apply_parts(manifest, |parts| {
                    apply_spread(parts, tables, failing, spread)
                }),
                None => state.apply_block(manifest, tables, failing),
            }
            .map_err(damaged)?;
            drop(applying);
            if !failed.is_empty() {
                engine_allocation::reserve(lost, failed.len())?;
                for (table, error) in failed.drain(..) {
                    lost.push((table, Damage::wrap(unit_scope(index, table), error)));
                }
                state.skip(&skipped(recording, lost))?;
            }
            timings.apply_ns += lap(clock);
            Ok(())
        })?;
        *next += 1;
        Ok(())
    }

    /// Checks `page` whole once its last hour is applied, before that hour is handed on, so a
    /// reader that stops there has had every check of what it read: its units hold exactly its
    /// hours.
    fn check_page(&mut self, page: &Page, state: &mut State, isolate: bool) -> io::Result<()> {
        let recording = self.recording;
        let index = &page.index;
        let scope = page_scope(index);
        let damaged = |error: io::Error| Damage::wrap(scope.clone(), error);
        let lost = &mut self.lost;
        let checked = page.decoding.with_dependent(|_, decoders| -> io::Result<bool> {
            decoders.records.finish().map_err(damaged)?;
            let losing = lost.len();
            for (table, decoder) in decoders.tables.iter().enumerate() {
                if decoder.as_ref().is_some_and(|decoder| decoder.finish().is_err()) {
                    let error = invalid("does not hold exactly its hours");
                    if !isolate {
                        return Err(damaged(error));
                    }
                    // What the walk applied of a block holding more than its hours is not
                    // to be trusted.
                    engine_allocation::reserve(lost, 1)?;
                    lost.push((table, Damage::wrap(unit_scope(index, table), error)));
                }
            }
            Ok(lost.len() != losing)
        })?;
        if checked {
            state.skip(&skipped(recording, &self.lost))?;
        }
        Ok(())
    }

    /// Closes `page` once its every hour is applied and checked ([`Self::check_page`]): its
    /// arena goes back to the walker for the page after it.
    fn close_page(&mut self, page: Page, state: &mut State) -> io::Result<()> {
        let Page {
            index, decoding, ..
        } = page;
        self.hand_back(decoding.into_owner());
        if index.restart.is_some() {
            state.freeze_dictionary()?;
        }
        state
            .trim_dictionary()
            .map_err(|error| Damage::wrap(page_scope(&index), error))
    }

    /// Gives back the arena of `page`, left before its every hour was applied.
    fn leave(&mut self, page: Page) {
        self.hand_back(page.decoding.into_owner());
    }

    /// Takes back a closed page's units as its arena and their places, unless a checkpoint
    /// shares them.
    fn hand_back(&mut self, units: Arc<Units>) {
        if let Ok(units) = Arc::try_unwrap(units) {
            // A checkpoint's page may not be the page the walker loaded last.
            self.units = units.at;
            self.arena = units.arena;
        }
    }

    /// Steps through `segment`'s run from the segment's first hour, once [`Self::begin`] put
    /// what that hour applies to into `state`.
    pub(crate) fn steps(&mut self, segment: &Segment) -> io::Result<Steps> {
        Ok(Steps {
            run: segment.run,
            branch: segment.branch,
            next: segment.first,
            chain: self.chain_before(segment)?,
            page: None,
        })
    }

    /// Restores decode their tables, and walks given no spread of their own a page's chunks,
    /// apart on `spread`, which runs `job` for each index below `count`, on any threads, every
    /// one before it returns: for a walker that has the threads to itself.
    pub fn decode_apart(&mut self, spread: fn(usize, &(dyn Fn(usize) + Sync))) {
        self.spread = Some(spread);
    }

    /// Readies it for steps forked from a checkpoint ([`Steps::fork`]), its state shared from
    /// the checkpoint's.
    pub(crate) fn take_up(&mut self) -> io::Result<()> {
        self.fit()?;
        self.lose_withheld()
    }

    /// Applies the hours of `steps` up to ordinal `until` (past the next) within its page to
    /// `state`, each table's hours one after another apart from the others' on `spread`, each
    /// hour of a table given to its `notes` entry by `note` once applied; then reads the last
    /// hour by `visit`. How many hours it applied and whether every note took, none past its
    /// run's last hour: a note failing leaves the walk whole. A page is closed before the next
    /// page's first hour, as [`Self::walk`] closes it.
    pub(crate) fn step_span<N: Send, T>(
        &mut self,
        steps: &mut Steps,
        state: &mut State,
        until: u32,
        spread: &Spread<'_>,
        notes: &mut [Option<N>],
        note: &(dyn Fn(&mut N, &SpanPart<'_>, usize) -> io::Result<()> + Sync),
        visit: impl FnOnce(&State, &Manifest<'_>) -> T,
    ) -> io::Result<Option<(u32, io::Result<()>, T)>> {
        if steps.page.as_ref().is_some_and(Page::done)
            && let Some(page) = steps.page.take()
        {
            self.close_page(page, state)?;
        }
        let run = self.recording.run(steps.run)?;
        if steps.next >= run.ordinals {
            return Ok(None);
        }
        let mut page = match steps.page.take() {
            Some(page) => page,
            None => {
                let at = run
                    .pages
                    .partition_point(|page| page.first <= steps.next)
                    .checked_sub(1)
                    .filter(|&at| run.pages[at].first == steps.next)
                    .ok_or_else(|| invalid("A step starts inside a page it did not walk"))?;
                // Steps walk on through their run's later segments; a walker that decodes apart
                // decodes a page's chunks on the hours' spread.
                let decoding = self.spread.is_some().then_some(spread);
                self.open_page(steps.run, at, steps.branch, u32::MAX, &mut steps.chain, state, false, decoding)?
            }
        };
        let hours = (page.through - page.next).min(u64::from(until.saturating_sub(steps.next)).max(1));
        let noted = match self
            .page_span(&mut page, state, hours as usize, spread, notes, note)
            .and_then(|noted| {
                if page.done() {
                    self.check_page(&page, state, false)?;
                }
                Ok(noted)
            }) {
            Ok(noted) => noted,
            Err(error) => {
                self.leave(page);
                return Err(error);
            }
        };
        let visited = page.manifest(|manifest| visit(state, manifest));
        steps.page = Some(page);
        steps.next += hours as u32;
        Ok(Some((hours as u32, noted, visited)))
    }

    /// Applies `page`'s next `hours` hours to `state` as [`Self::step_span`] does; the page's
    /// manifest is then the last hour's. Whether every note took.
    fn page_span<N: Send>(
        &mut self,
        page: &mut Page,
        state: &mut State,
        hours: usize,
        spread: &Spread<'_>,
        notes: &mut [Option<N>],
        note: &(dyn Fn(&mut N, &SpanPart<'_>, usize) -> io::Result<()> + Sync),
    ) -> io::Result<io::Result<()>> {
        use std::sync::{Mutex, PoisonError};
        let recording = self.recording;
        let held = recording.run(page.index.run)?;
        let Page {
            index,
            decoding,
            next,
            clock,
            branch,
            ..
        } = page;
        let scope = page_scope(index);
        let damaged = |error: io::Error| Damage::wrap(scope.clone(), error);
        let timings = &mut self.timings;
        let mut noted = Ok(());
        decoding.with_dependent_mut(|_, decoders| -> io::Result<()> {
            let Decoders {
                records,
                manifest,
                tables,
            } = decoders;
            let mut manifests = engine_allocation::vec(hours)?;
            for hour in 0..hours {
                let at = i32::try_from(i64::from(held.first_hour) + (*next + hour as u64) as i64)
                    .map_err(|_| damaged(invalid("names an hour past 2^31")))?;
                let mut read = manifest.clone();
                records.hour(*branch, at, &mut read).map_err(damaged)?;
                for section in &read.sections {
                    if (recording.verify || !state.skips(section.table))
                        && tables[section.table].is_none()
                    {
                        return Err(damaged(invalid("names a table its block does not hold")));
                    }
                }
                manifests.push(read);
            }
            timings.manifest_ns += lap(clock);
            let applying =
                engine_allocation::labels::kind(engine_allocation::labels::Kind::WalkerTables);
            state
                .apply_span(&manifests, |parts| {
                    let mut held =
                        engine_allocation::collect_vec(tables.iter_mut().map(Option::as_mut))?;
                    let mut noters =
                        engine_allocation::collect_vec(notes.iter_mut().map(Option::as_mut))?;
                    let mut jobs = engine_allocation::vec(parts.len())?;
                    for part in parts.iter_mut() {
                        let table = part.table();
                        let decoder = held.get_mut(table).and_then(Option::take);
                        let noter = noters.get_mut(table).and_then(Option::take);
                        jobs.push(Mutex::new((part, decoder, noter, None)));
                    }
                    spread(jobs.len(), &|at| {
                        use engine_allocation::labels::{self, Kind, Owner};
                        let _memory = labels::enter(Owner::RUN, Kind::WalkerTables);
                        let mut job = jobs[at].lock().unwrap_or_else(PoisonError::into_inner);
                        let (part, decoder, noter, outcome) = &mut *job;
                        let mut noting = Ok(());
                        let mut walk = || -> io::Result<()> {
                            for hour in 0..part.hours() {
                                part.apply_hour(hour, |entry, _, truncated, table, changes| {
                                    decoder
                                        .as_mut()
                                        .ok_or_else(|| {
                                            invalid("An hour names a table its block does not hold")
                                        })?
                                        .hour(table, changes, entry, truncated)
                                })?;
                                if let Some(noter) = noter.as_mut()
                                    && noting.is_ok()
                                {
                                    noting = note(noter, &**part, hour);
                                }
                            }
                            Ok(())
                        };
                        let applied = walk();
                        *outcome = Some((applied, noting));
                    });
                    for job in jobs {
                        let (_, _, _, outcome) = job.into_inner().unwrap_or_else(PoisonError::into_inner);
                        match outcome {
                            Some((Ok(()), noting)) => {
                                if noted.is_ok() {
                                    noted = noting;
                                }
                            }
                            Some((Err(error), _)) => return Err(error),
                            None => return Err(io::Error::other("A part of a span never ran")),
                        }
                    }
                    Ok(())
                })
                .map_err(damaged)?;
            drop(applying);
            if let Some(last) = manifests.pop() {
                *manifest = last;
            }
            timings.apply_ns += lap(clock);
            Ok(())
        })?;
        *next += hours as u64;
        Ok(noted)
    }

    /// Ends `steps`, giving back the arena of the page they hold.
    pub(crate) fn stop(&mut self, steps: Steps) {
        if let Some(page) = steps.page {
            self.leave(page);
        }
    }
}

/// The place among a walker's units of `extent`'s: the records first, then each table's block.
fn unit_place(extent: &Extent) -> usize {
    match extent.kind {
        envelope::Kind::Records => 0,
        _ => extent.stream as usize + 1,
    }
}

/// The damage scope of `page`.
fn page_scope(page: &IndexPage) -> DamageScope {
    DamageScope::Page {
        run: page.run,
        first: page.first,
        hours: page.hours,
    }
}

/// The damage scope of `table`'s own block on `page`: its extent, else the page.
fn unit_scope(page: &IndexPage, table: usize) -> DamageScope {
    page.table(table as u32)
        .map_or_else(|| page_scope(page), |extent| DamageScope::Extent(*extent))
}

/// A page's decoded units: the walker's arena and where each sits in it, shared by the
/// checkpoints taken inside the page.
struct Units {
    arena: Vec<u8>,
    at: Vec<Option<Range<usize>>>,
}

/// A page's decoders, each at its next hour, and the manifest of the hour applied last.
struct Decoders<'a> {
    records: RecordsDecoder<'a>,
    manifest: Manifest<'a>,
    tables: Vec<Option<TableDecoder<'a>>>,
}

impl Decoders<'_> {
    /// The decoders where they stand, over `to`, a copy of `from`, which they read.
    fn rebased<'b>(&self, from: &[u8], to: &'b [u8]) -> io::Result<Decoders<'b>> {
        let mut tables = engine_allocation::vec(self.tables.len())?;
        for decoder in &self.tables {
            tables.push(decoder.as_ref().map(|decoder| decoder.rebased(from, to)).transpose()?);
        }
        Ok(Decoders {
            records: self.records.rebased(from, to)?,
            manifest: Manifest {
                branch: self.manifest.branch,
                hour: 0,
                flags: 0,
                total_ns: 0,
                pool_bytes: 0,
                dropped_frames: 0,
                appends: Vec::new(),
                statuses: Vec::new(),
                read: Vec::new(),
                samples: Vec::new(),
                sections: Vec::new(),
            },
            tables,
        })
    }
}

self_cell::self_cell!(
    struct Decoding {
        owner: Arc<Units>,
        #[not_covariant]
        dependent: Decoders,
    }
);

/// A page under way: its decoders over its units, and the ordinal they apply next.
struct Page {
    index: IndexPage,
    decoding: Decoding,
    next: u64,
    end: u64,
    /// Past the last ordinal its walk applies: `end`, unless its segment ends inside it.
    through: u64,
    /// The branch its hours are read as.
    branch: u64,
    /// The tables whose block failed inside an hour, which an isolating walk loses.
    failed: Vec<(usize, io::Error)>,
    clock: Option<Instant>,
}

impl Page {
    fn done(&self) -> bool {
        self.next == self.through
    }

    /// Whether its walk applies its every hour.
    fn whole(&self) -> bool {
        self.through == self.end
    }

    /// `read` of the manifest of the hour applied last.
    fn manifest<T>(&self, read: impl FnOnce(&Manifest<'_>) -> T) -> T {
        self.decoding
            .with_dependent(|_, decoders| read(&decoders.manifest))
    }

    /// A copy where it stands, its decoders over the same units.
    fn fork(&self) -> io::Result<Self> {
        let units = Arc::clone(self.decoding.borrow_owner());
        let decoding = self.decoding.with_dependent(|from, decoders| {
            Decoding::try_new::<io::Error>(units, |to| decoders.rebased(&from.arena, &to.arena))
        })?;
        Ok(Self {
            index: self.index.clone(),
            decoding,
            next: self.next,
            end: self.end,
            through: self.through,
            branch: self.branch,
            failed: Vec::new(),
            clock: None,
        })
    }
}

/// A walk of one run going on hour by hour as it is asked ([`Walker::step`]): the ordinal it
/// applies next, the hour digest chain through its pages so far, and the page under way.
pub(crate) struct Steps {
    run: u32,
    /// The branch its hours are read as.
    branch: u64,
    next: u32,
    chain: [u8; 32],
    page: Option<Page>,
}

impl Steps {
    /// The ordinal applied next.
    pub(crate) fn next(&self) -> u32 {
        self.next
    }

    /// A copy where it stands, sharing the units of the page under way, for a walk to go on
    /// from later ([`Walker::take_up`]).
    pub(crate) fn fork(&self) -> io::Result<Self> {
        Ok(Self {
            run: self.run,
            branch: self.branch,
            next: self.next,
            chain: self.chain,
            page: self.page.as_ref().map(Page::fork).transpose()?,
        })
    }

    /// The bytes its decoders hold of their own, the units they read aside.
    pub(crate) fn decoder_bytes(&self) -> usize {
        self.page.as_ref().map_or(0, |page| {
            page.decoding.with_dependent(|_, decoders| {
                decoders.records.held_bytes()
                    + decoders
                        .tables
                        .iter()
                        .flatten()
                        .map(TableDecoder::held_bytes)
                        .sum::<usize>()
            })
        })
    }

    /// The units of the page under way, by their place in memory, and the bytes they hold.
    pub(crate) fn units(&self) -> Option<(usize, usize)> {
        let units = self.page.as_ref()?.decoding.borrow_owner();
        Some((Arc::as_ptr(units) as usize, units.arena.capacity()))
    }
}
