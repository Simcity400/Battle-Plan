//! A columnar recording read from its file or memory: every run's directory and restarts
//! held from open, its hours walked a page at a time by [`Walker`]s, each decoding a page's
//! selected units at once into one buffer it keeps.

use super::{
    Segment,
    input::{Budget, Decoder, Held, Limits, Permit, Source},
    invalid, schema,
};
use crate::rows::{TABLE_COUNT, TableId};
use replay_columnar::{
    envelope::{self, Extent, IndexPage, RestartDirectory, Root, RunDirectory},
    framing,
    leaf::Reader,
    manifest::Manifest,
    walk::{Chunks, Dictionary, Packed, State},
};
use std::{
    fmt,
    fs::File,
    io,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex, PoisonError, Weak, atomic::AtomicBool},
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
    /// A page's unit of one stream.
    Extent(Extent),
    /// A run's dictionary chunk, by its restart's ordinal.
    Dictionary { run: u32, chunk: u32 },
    /// One table's state at a run's restart after ordinal `after`.
    RestartTable { run: u32, after: u32, table: u32 },
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

/// A walk nobody stops.
static NEVER: AtomicBool = AtomicBool::new(false);

/// A columnar recording (`H4COLUMN` version 3), read from its file or memory: its root, every
/// run's directory and the restarts of every page, held from open. A run directory or page
/// that cannot be read refuses only what it covers ([`Self::damage`]); the rest stays
/// readable.
pub struct Recording {
    source: Arc<Source>,
    /// The identity block every run's chain starts from.
    identity: Vec<u8>,
    /// The root's control records' extents.
    controls: Box<[Extent]>,
    runs: Box<[Option<Arc<Run>>]>,
    segments: Box<[Segment]>,
    damage: Box<[Damage]>,
    budget: Arc<Budget>,
    dictionary_bytes: usize,
    /// Whether walks check every hour against the pages' hour digest chain.
    verify: bool,
    /// The tables walks restore and apply.
    selected: [bool; TABLE_COUNT],
}

impl Recording {
    pub fn open(bytes: Vec<u8>, limits: Limits) -> io::Result<Self> {
        Self::read(Held::Memory(bytes), limits)
    }

    pub fn open_path(path: impl AsRef<Path>, limits: Limits) -> io::Result<Self> {
        Self::read(Held::File(File::open(path)?), limits)
    }

    fn read(held: Held, limits: Limits) -> io::Result<Self> {
        let unlocated = |error: io::Error| Damage::wrap(DamageScope::Unlocated, error);
        let (source, root) = Source::open(held).map_err(unlocated)?;
        let source = Arc::new(source);
        let mut decoder = Decoder::new()?;
        let mut raw = Vec::new();
        decoder
            .append(&source, &root, &mut raw)
            .map_err(unlocated)?;
        let root = Root::read(&raw).map_err(unlocated)?;
        if root.identity.schema != envelope::schema_digest(&schema()?) {
            return Err(invalid("The recording's schema differs from this build's"));
        }
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
            source,
            runs: engine_allocation::boxed_slice(runs)?,
            segments: engine_allocation::boxed_slice(segments)?,
            damage: engine_allocation::boxed_slice(damage)?,
            budget: limits.budget,
            dictionary_bytes: limits.dictionary_bytes,
            verify: false,
            selected: [true; TABLE_COUNT],
        })
    }

    /// Walks restore, read and apply only `tables`, the tables their readers read; the others
    /// keep their statuses and no rows. A walk that verifies still reads every unit.
    pub fn select(&mut self, tables: &[TableId]) {
        self.selected = [false; TABLE_COUNT];
        for table in tables {
            self.selected[table.index()] = true;
        }
    }

    fn selects(&self, table: usize) -> bool {
        self.selected.get(table).copied().unwrap_or(false)
    }

    /// Whether walks decode `extent`, a page's unit: the manifests, the selected tables', or
    /// every one when they verify.
    fn decodes(&self, extent: &Extent) -> bool {
        extent.stream == framing::MANIFESTS
            || self.verify
            || self.selects(extent.stream as usize - 1)
    }

    /// The decoded bytes of `page`'s units that walks decode, together, and the most stored
    /// bytes one of them takes, but for the units of the tables `lost` names; a unit whose
    /// length is damage goes to `damaged`, counted as none.
    fn unit_lengths(
        &self,
        page: &IndexPage,
        lost: &[(usize, io::Error)],
        damaged: &mut Vec<(Extent, io::Error)>,
    ) -> io::Result<(usize, usize)> {
        let (mut raw, mut stored) = (0_usize, 0_usize);
        for extent in page.units.iter().filter(|extent| self.decodes(extent)) {
            if gone(lost, extent) {
                continue;
            }
            let length = match self.source.raw_length(extent) {
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
            stored =
                stored.max(usize::try_from(extent.stored).map_err(|_| engine_allocation::oom())?);
        }
        Ok((raw, stored))
    }

    /// The most decoded and stored bytes one of the tables walks restore from `restart`, of
    /// run `run`, takes.
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
            raw = raw.max(length);
            stored = stored
                .max(usize::try_from(table.extent.stored).map_err(|_| engine_allocation::oom())?);
        }
        Ok((raw, stored))
    }

    /// Applies the selection to a walk's `state`, clearing any an earlier walk left.
    fn select_in(&self, state: &mut State) -> io::Result<()> {
        let skipped: [bool; TABLE_COUNT] = std::array::from_fn(|table| !self.selects(table));
        state.skip(&skipped)
    }

    /// Makes every walk check each hour against the hour digest chain the pages hold.
    pub fn verify(&mut self, verify: bool) {
        self.verify = verify;
    }

    /// Why each run directory or page that cannot be read is refused.
    /// The capture the file's runs belong to, as the root's control records hold it; none
    /// when the root holds none.
    pub fn capture(&self) -> io::Result<Option<crate::columnar_file::Capture>> {
        let Some(extent) = self.controls.iter().find(|extent| extent.stream == 0) else {
            return Ok(None);
        };
        let mut decoder = Decoder::new()?;
        let mut raw = Vec::new();
        decoder.append(&self.source, extent, &mut raw)?;
        crate::columnar_file::Capture::read(&raw).map(Some)
    }

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

    /// Every readable run's segments in order: from the run's start, then from each restart
    /// a readable page holds.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
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

    /// The file's length through its last valid footer.
    pub fn len(&self) -> u64 {
        self.source.len()
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
        Walker::new(self, &NEVER)?.walk_runs(state, visit)
    }

    /// `segment`'s hours from its restart restored into `state`, or from empty tables at its
    /// run's start, as [`Self::walk`] visits them; `starts` is the segment's first hour.
    pub fn walk_segment(
        &self,
        segment: &Segment,
        state: &mut State,
        visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let mut walker = Walker::new(self, &NEVER)?;
        walker.begin(segment, state)?;
        walker.walk(segment, state, visit).map(drop)
    }

    /// Replaces `state` with what restart page `at` of `run` holds, as [`Walker::restore`].
    pub fn restore(&self, run: u32, at: usize, state: &mut State) -> io::Result<()> {
        Walker::new(self, &NEVER)?.restore(run, at, state)
    }

    /// Wakes the walkers waiting for the budget, for those whose walk stopped to leave.
    #[cfg(test)]
    pub(crate) fn wake(&self) {
        self.budget.wake();
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

/// Whether `extent` is a unit of a table `lost` names.
fn gone(lost: &[(usize, io::Error)], extent: &Extent) -> bool {
    extent.stream != framing::MANIFESTS
        && lost
            .iter()
            .any(|&(table, _)| table + 1 == extent.stream as usize)
}

/// What the walk of `recording` skips: the tables it does not select, and those `lost`.
fn skipped(recording: &Recording, lost: &[(usize, io::Error)]) -> [bool; TABLE_COUNT] {
    std::array::from_fn(|table| {
        !recording.selects(table) || lost.iter().any(|&(held, _)| held == table)
    })
}

/// One piece of a stream's unit: the ordinals it covers and where its raw bytes sit in the
/// unit.
struct Piece {
    first: u64,
    end: u64,
    start: usize,
    stop: usize,
}

/// A stream's unit of the page a walker decoded: its bytes in the arena, and its pieces.
#[derive(Clone)]
struct Unit {
    bytes: Range<usize>,
    pieces: Range<usize>,
}

/// A stream's unit as the walk reads it, hour after hour.
struct Cursor<'a> {
    reader: Reader<'a>,
    length: usize,
    pieces: &'a [Piece],
}

impl<'a> Cursor<'a> {
    fn new(arena: &'a [u8], pieces: &'a [Piece], unit: &Unit) -> Self {
        Self {
            reader: Reader::new(&arena[unit.bytes.clone()]),
            length: unit.bytes.len(),
            pieces: &pieces[unit.pieces.clone()],
        }
    }

    /// Whether `start..stop` of the unit lies within the piece covering `ordinal`.
    fn holds(&self, ordinal: u64, start: usize, stop: usize) -> bool {
        self.pieces.iter().any(|piece| {
            (piece.first..piece.end).contains(&ordinal)
                && piece.start <= start
                && stop <= piece.stop
        })
    }
}

/// A timed walk's time, by part.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    /// Reading and decompressing the pages' units.
    pub decompress_ns: u64,
    /// Reading the manifests and finding their sections.
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
/// segment: its decoder, its buffers, and the share of the budget they hold, which it gives
/// back when it parks ([`Self::park`]) and, when it waits for its share, before it waits for
/// a larger one.
pub struct Walker<'r> {
    recording: &'r Recording,
    budget: &'r Arc<Budget>,
    /// What ends its wait for its share; none when it never waits, but takes a segment's
    /// whole share before walking it ([`Self::hold`]).
    stop: Option<&'r AtomicBool>,
    decoder: Decoder,
    /// The page read last.
    raw: Vec<u8>,
    /// The page's units the walk reads, decoded, stream after stream.
    arena: Vec<u8>,
    pieces: Vec<Piece>,
    /// Each stream's unit in the arena.
    units: Vec<Option<Unit>>,
    /// Tables an isolating walk lost to damage of their own since it began, with the damage.
    lost: Vec<(usize, io::Error)>,
    permit: Option<Permit>,
    /// What the walk its share was taken for reads.
    reads: Option<Reads>,
    timed: bool,
    timings: Timings,
}

impl<'r> Walker<'r> {
    /// A walker of `recording`, whose wait for the budget ends once `stop` is set.
    pub fn new(recording: &'r Recording, stop: &'r AtomicBool) -> io::Result<Self> {
        Self::with(recording, &recording.budget, Some(stop))
    }

    /// A walker of `recording` under `budget` that never waits for its share:
    /// [`Self::hold`] takes a walk's whole share before it starts, or cannot then.
    pub(crate) fn without_waiting(
        recording: &'r Recording,
        budget: &'r Arc<Budget>,
    ) -> io::Result<Self> {
        Self::with(recording, budget, None)
    }

    fn with(
        recording: &'r Recording,
        budget: &'r Arc<Budget>,
        stop: Option<&'r AtomicBool>,
    ) -> io::Result<Self> {
        Ok(Self {
            recording,
            budget,
            stop,
            decoder: Decoder::new()?,
            raw: Vec::new(),
            arena: Vec::new(),
            pieces: Vec::new(),
            units: engine_allocation::collect_vec((0..=TABLE_COUNT).map(|_| None))?,
            lost: Vec::new(),
            permit: None,
            reads: None,
            timed: false,
            timings: Timings::default(),
        })
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

    /// Gives back its buffers, then its share of the budget.
    pub fn park(&mut self) {
        self.arena = Vec::new();
        self.decoder.release();
        self.permit = None;
        self.reads = None;
    }

    /// Whether it holds a share of the budget.
    pub(crate) fn holds(&self) -> bool {
        self.permit.is_some()
    }

    /// Starts afresh but for its share and what it was taken for: what a panic unwound
    /// through is not used again.
    pub(crate) fn renew(&mut self) -> io::Result<()> {
        self.decoder = Decoder::new()?;
        self.raw = Vec::new();
        self.arena = Vec::new();
        self.pieces = Vec::new();
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

    /// Keeps `error`, met reading `extent`, as its table's loss when the walk isolates damage
    /// and the damage is that table's unit's alone; anything else refuses the walk.
    fn lose(&mut self, extent: &Extent, error: io::Error, isolate: bool) -> io::Result<()> {
        if !isolate || extent.stream == framing::MANIFESTS || Damage::of(&error).is_none() {
            return Err(error);
        }
        engine_allocation::reserve(&mut self.lost, 1)?;
        self.lost.push((extent.stream as usize - 1, error));
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
            let (raw, stored) = recording.unit_lengths(&page, &[], &mut damaged)?;
            damaged.clear();
            reads = reads.max(Reads { raw, stored });
        }
        Ok(self.measured(kept, reads))
    }

    /// `reads`, once the indexes read for them are given back: stored bytes count only for a
    /// recording read from a file.
    fn measured(&mut self, kept: usize, reads: Reads) -> Reads {
        // The indexes read here are no part of the share it holds.
        if self.decoder.stored() > kept {
            self.decoder.release();
        }
        if self.recording.source.reads_file() {
            reads
        } else {
            Reads { stored: 0, ..reads }
        }
    }

    /// Takes the share of the budget a walk reading `reads` needs, its dictionary's too,
    /// for a walker that never waits: false when it cannot now. A share it holds already
    /// grows; its buffers fit within it as the walk starts.
    pub(crate) fn hold(&mut self, reads: Reads) -> bool {
        let share = [
            reads.stored,
            self.decoder.context(),
            self.recording.dictionary_bytes,
        ]
        .into_iter()
        .fold(reads.raw, usize::saturating_add);
        let held = if let Some(permit) = &mut self.permit {
            permit.bytes() >= share || permit.try_grow(share)
        } else if let Some(permit) = self.budget.try_enter(share) {
            self.permit = Some(permit);
            true
        } else {
            false
        };
        if held {
            self.reads = Some(reads);
        }
        held
    }

    /// Sizes its buffers for every read of the walk its share was taken for, within that
    /// share, for a walker that never waits; one that waits sizes them read by read.
    fn fit(&mut self) -> io::Result<()> {
        if self.stop.is_some() {
            return Ok(());
        }
        let (Some(permit), Some(reads)) = (&self.permit, self.reads) else {
            return Err(io::Error::other(
                "A walk starts without its share of the walkers' budget",
            ));
        };
        if reads.raw <= self.arena.capacity() && reads.stored <= self.decoder.stored() {
            return Ok(());
        }
        let kept = [
            self.decoder.stored().max(reads.stored),
            self.decoder.context(),
            self.recording.dictionary_bytes,
        ]
        .into_iter()
        .fold(self.arena.capacity().max(reads.raw), usize::saturating_add);
        if kept > permit.bytes() {
            self.arena = Vec::new();
            self.decoder.release();
        }
        if self.arena.capacity() < reads.raw {
            self.arena = Vec::new();
            self.arena = engine_allocation::vec(reads.raw)?;
        }
        self.decoder.reserve(reads.stored)
    }

    /// Makes its buffers hold `raw` decoded and `stored` stored bytes under a share of the
    /// budget covering them, its dictionary's budget too. A walker needing a larger share
    /// gives back what it holds before it waits for it; one that never waits holds it
    /// already.
    fn admit(&mut self, raw: usize, stored: usize) -> io::Result<()> {
        let stored = if self.recording.source.reads_file() {
            stored
        } else {
            0
        };
        if self.permit.is_some() && raw <= self.arena.capacity() && stored <= self.decoder.stored()
        {
            return Ok(());
        }
        let Some(stop) = self.stop else {
            return Err(io::Error::other(
                "A walk reads more than the share of the walkers' budget its segment was \
                 measured to need",
            ));
        };
        let raw = raw.max(self.arena.capacity());
        let stored = stored.max(self.decoder.stored());
        self.park();
        let bytes = [stored, self.decoder.held(), self.recording.dictionary_bytes]
            .into_iter()
            .try_fold(raw, usize::checked_add)
            .ok_or_else(engine_allocation::oom)?;
        let permit = self.budget.enter(bytes, stop)?;
        self.arena = engine_allocation::vec(raw)?;
        self.decoder.reserve(stored)?;
        self.permit = Some(permit);
        Ok(())
    }

    /// Decodes `page`'s units of the streams the walk reads into the arena, sized by their
    /// frames' lengths; a walk that `isolate`s damage leaves out a table whose unit it cannot
    /// read, and every later unit of a table it lost.
    fn load(&mut self, page: &IndexPage, isolate: bool) -> io::Result<()> {
        let recording = self.recording;
        let mut damaged = Vec::new();
        let (raw, stored) = recording.unit_lengths(page, &self.lost, &mut damaged)?;
        for (extent, error) in damaged {
            self.lose(&extent, error, isolate)?;
        }
        self.admit(raw, stored)?;
        self.arena.clear();
        self.pieces.clear();
        self.units.fill(None);
        for extent in page.units.iter().filter(|extent| recording.decodes(extent)) {
            if gone(&self.lost, extent) {
                continue;
            }
            let start = self.arena.len();
            if let Err(error) = self
                .decoder
                .append(&recording.source, extent, &mut self.arena)
            {
                // A stream's earlier pieces go with its damaged one.
                self.arena.truncate(start);
                self.units[extent.stream as usize] = None;
                let error = Damage::wrap(DamageScope::Extent(*extent), error);
                self.lose(extent, error, isolate)?;
                continue;
            }
            let unit = self.units[extent.stream as usize].get_or_insert(Unit {
                bytes: start..start,
                pieces: self.pieces.len()..self.pieces.len(),
            });
            engine_allocation::reserve(&mut self.pieces, 1)?;
            self.pieces.push(Piece {
                first: u64::from(extent.first),
                end: u64::from(extent.first) + u64::from(extent.hours),
                start: start - unit.bytes.start,
                stop: self.arena.len() - unit.bytes.start,
            });
            unit.bytes.end = self.arena.len();
            unit.pieces.end = self.pieces.len();
        }
        Ok(())
    }

    /// Puts into `state` what `segment`'s first hour applies to: its restart's tables, or
    /// empty tables at its run's start.
    pub fn begin(&mut self, segment: &Segment, state: &mut State) -> io::Result<()> {
        self.fit()?;
        self.lost.clear();
        let Some(at) = segment.restart else {
            let recording = self.recording;
            let run = recording.shared(segment.run)?;
            *state = State::new(schema()?)?;
            state.set_dictionary(Dictionary::run(run.clone(), recording.dictionary_bytes, 0));
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
        self.lost.clear();
        *state = State::new(schema()?)?;
        recording.select_in(state)?;
        state.set_dictionary(Dictionary::run(
            held.clone(),
            recording.dictionary_bytes,
            len,
        ));
        let mut declared = Vec::new();
        for table in tables() {
            declared.clear();
            for &ordinal in &table.chunks {
                let at = held.chunk_at(ordinal)?;
                state.pin_strings(held.load(at)?)?;
                engine_allocation::reserve(&mut declared, 1)?;
                declared.push(held.restarts[at].ids);
            }
            let damaged = |error: io::Error| Damage::wrap(scope(table.table), error);
            self.arena.clear();
            self.decoder
                .append(&recording.source, &table.extent, &mut self.arena)
                .map_err(damaged)?;
            state
                .restore(table.table as usize, &self.arena)
                .map_err(damaged)?;
            state
                .text_ids(table.table as usize, |id| {
                    let at = declared.partition_point(|&(first, _)| first <= id);
                    match at.checked_sub(1) {
                        Some(at) if id - declared[at].0 < declared[at].1 => Ok(()),
                        _ => Err(invalid("names a dictionary chunk outside its dependencies")),
                    }
                })
                .map_err(damaged)?;
        }
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

    /// The hours of `segment`'s pages from `state` as [`Self::begin`] left it: one manifest
    /// per ordinal at its run's branch and hour, every manifest and section within the piece
    /// covering its ordinal, every unit read to its end, and the chain checked when asked.
    /// `visit` sees each hour once it is applied, with whether it is the walk's first; it
    /// stops the walk by returning false, and the walk then returns false.
    pub fn walk(
        &mut self,
        segment: &Segment,
        state: &mut State,
        mut visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<bool> {
        self.walk_with(segment, state, false, |state, manifest, first, _| {
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
        self.walk_with(segment, state, true, visit)
    }

    fn walk_with<V>(
        &mut self,
        segment: &Segment,
        state: &mut State,
        isolate: bool,
        mut visit: V,
    ) -> io::Result<bool>
    where
        V: FnMut(&State, &Manifest<'_>, bool, &[(usize, io::Error)]) -> io::Result<bool>,
    {
        self.fit()?;
        let recording = self.recording;
        let isolate = isolate && !recording.verify;
        let run = recording.run(segment.run)?;
        let columns = engine_allocation::copy_vec(state.columns())?;
        let mut chain = match (recording.verify, segment.restart) {
            (false, _) => [0; 32],
            (true, Some(at)) => {
                recording
                    .read_page(segment.run, at, &mut self.decoder, &mut self.raw)?
                    .chain
            }
            (true, None) => envelope::chain_start(&recording.identity, segment.run),
        };
        let hour = |ordinal: u64| i64::from(run.first_hour) + ordinal as i64;
        let mut first = true;
        for at in run.pages_of(segment) {
            let page = recording.read_page(segment.run, at, &mut self.decoder, &mut self.raw)?;
            let scope = DamageScope::Page {
                run: page.run,
                first: page.first,
                hours: page.hours,
            };
            let damaged = |error: io::Error| Damage::wrap(scope.clone(), error);
            // A table's own damage names its unit.
            let unit = |table: usize| {
                page.units
                    .iter()
                    .find(|extent| extent.stream as usize == table + 1)
                    .map_or_else(|| scope.clone(), |extent| DamageScope::Extent(*extent))
            };
            let end = u64::from(page.first) + u64::from(page.hours);
            if i64::from(page.first_hour) != hour(page.first.into())
                || i64::from(page.last_hour) != hour(end - 1)
            {
                return Err(damaged(invalid("names hours off its ordinals")));
            }
            let mut clock = self.timed.then(Instant::now);
            let losing = self.lost.len();
            self.load(&page, isolate)?;
            if self.lost.len() != losing {
                state.skip(&skipped(recording, &self.lost))?;
            }
            self.timings.decompress_ns += lap(&mut clock);
            let (arena, pieces) = (&self.arena[..], &self.pieces[..]);
            let mut manifests = match &self.units[0] {
                Some(unit) => Cursor::new(arena, pieces, unit),
                None => return Err(damaged(invalid("holds no manifests"))),
            };
            let mut tables: [Option<Cursor<'_>>; TABLE_COUNT] = std::array::from_fn(|table| {
                self.units[table + 1]
                    .as_ref()
                    .map(|unit| Cursor::new(arena, pieces, unit))
            });
            let mut ordinal = u64::from(page.first);
            loop {
                let start = manifests.reader.at();
                let Some(bytes) = framing::next_manifest(&mut manifests.reader, manifests.length)
                    .map_err(damaged)?
                else {
                    break;
                };
                if ordinal >= end || !manifests.holds(ordinal, start, manifests.reader.at()) {
                    return Err(damaged(invalid("holds a manifest off its ordinals")));
                }
                let manifest = Manifest::read(bytes, &columns).map_err(damaged)?;
                if manifest.branch != run.branch || i64::from(manifest.hour) != hour(ordinal) {
                    return Err(damaged(invalid(
                        "holds an hour off its run's branch and hours",
                    )));
                }
                // A manifest names each table once, so its sections are no more than the tables.
                let mut sections: [&[u8]; TABLE_COUNT] = [&[][..]; TABLE_COUNT];
                for (index, section) in manifest.sections.iter().enumerate() {
                    if !recording.verify && state.skips(section.table) {
                        continue;
                    }
                    let cursor = tables
                        .get_mut(section.table)
                        .and_then(Option::as_mut)
                        .ok_or_else(|| {
                            damaged(invalid("holds a section of a table with no unit"))
                        })?;
                    let read = framing::next_section(&mut cursor.reader, section.bytes as usize)
                        .and_then(|held| {
                            let at = cursor.reader.at();
                            if cursor.holds(ordinal, at - held.len(), at) {
                                Ok(held)
                            } else {
                                Err(invalid("holds a section outside its hour's piece"))
                            }
                        });
                    match read {
                        Ok(held) => sections[index] = held,
                        Err(error) if !isolate => return Err(damaged(error)),
                        Err(error) => {
                            let error = Damage::wrap(unit(section.table), error);
                            if Damage::of(&error).is_none() {
                                return Err(error);
                            }
                            tables[section.table] = None;
                            engine_allocation::reserve(&mut self.lost, 1)?;
                            self.lost.push((section.table, error));
                            state.skip(&skipped(recording, &self.lost))?;
                        }
                    }
                }
                if recording.verify {
                    chain = envelope::chain_hour(
                        &chain,
                        bytes,
                        sections[..manifest.sections.len()].iter().copied(),
                    );
                }
                self.timings.manifest_ns += lap(&mut clock);
                state
                    .apply(&manifest, |index| Ok(sections[index]))
                    .map_err(damaged)?;
                self.timings.apply_ns += lap(&mut clock);
                if !visit(state, &manifest, first, &self.lost)? {
                    return Ok(false);
                }
                // The visit's time is no part of the walk's.
                lap(&mut clock);
                first = false;
                ordinal += 1;
            }
            if ordinal != end {
                return Err(damaged(invalid("does not hold exactly its hours")));
            }
            let losing = self.lost.len();
            for (table, cursor) in tables.iter().enumerate() {
                if cursor
                    .as_ref()
                    .is_some_and(|cursor| cursor.reader.finish().is_err())
                {
                    let error = invalid("does not hold exactly its hours");
                    if !isolate {
                        return Err(damaged(error));
                    }
                    // What the walk applied of a unit holding more than its hours is not to be
                    // trusted.
                    engine_allocation::reserve(&mut self.lost, 1)?;
                    self.lost.push((table, Damage::wrap(unit(table), error)));
                }
            }
            if self.lost.len() != losing {
                state.skip(&skipped(recording, &self.lost))?;
            }
            if recording.verify && chain != page.chain {
                return Err(damaged(invalid("differs from its hour digest chain")));
            }
            if page.restart.is_some() {
                state.freeze_dictionary()?;
            }
            state.trim_dictionary().map_err(damaged)?;
        }
        Ok(true)
    }
}
