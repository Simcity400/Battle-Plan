//! A columnar recording read from its file or memory: every run's directory and restarts
//! held from open, its hours walked a page at a time by [`Walker`]s, each decoding a page's
//! selected units into one buffer it keeps, or, for a unit larger than its frames' windows,
//! as the walk reads it.

use super::{
    Segment, columns_at, withheld_at,
    input::{Decoder, Frame, Held, Source, TRAILING},
    invalid, schema,
};
use crate::rows::{TABLE_COUNT, TableId};
use replay_columnar::{
    envelope::{self, Extent, IndexPage, RestartDirectory, Root, RunDirectory},
    framing,
    leaf::Reader,
    manifest::Manifest,
    restart::WeakRestored,
    walk::{Chunks, Dictionary, Packed, State},
};
use std::{
    collections::HashMap,
    fmt,
    fs::File,
    io,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex, PoisonError, Weak},
    time::Instant,
};
use zstd::zstd_safe::{self, DCtx, InBuffer, OutBuffer, ResetDirective};

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
    capture: impl FnOnce() -> io::Result<Option<crate::columnar_file::Capture>>,
) -> io::Result<(Vec<usize>, Vec<(usize, String)>)> {
    let schema = schema()?;
    if root.identity.schema == envelope::schema_digest(&schema) {
        return Ok((
            engine_allocation::collect_vec(schema.iter().map(Vec::len))?,
            Vec::new(),
        ));
    }
    let differs = |what: &str| invalid(format!("The recording's schema differs from {what}"));
    let format = match capture() {
        Ok(Some(capture)) => capture.format,
        Ok(None) => return Err(differs("this build's, and it names no capture")),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return Err(differs(&format!(
                "this build's, and its capture cannot be read: {error}"
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

/// A columnar recording (`H4COLUMN` version 4), read from its file or memory: its root, every
/// run's directory and the restarts of every page, held from open. A run directory or page
/// that cannot be read refuses only what it covers ([`Self::damage`]); the rest stays
/// readable.
pub struct Recording {
    source: Arc<Source>,
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
    segments: Box<[Segment]>,
    damage: Box<[Damage]>,
    /// Whether walks check every hour against the pages' hour digest chain.
    verify: bool,
    /// The tables walks restore and apply.
    selected: [bool; TABLE_COUNT],
    restarts: Restarts,
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
        let (source, root) = Source::open(held).map_err(unlocated)?;
        let source = Arc::new(source);
        let mut decoder = Decoder::new()?;
        let mut raw = Vec::new();
        decoder
            .append(&source, &root, &mut raw)
            .map_err(unlocated)?;
        let root = Root::read(&raw).map_err(unlocated)?;
        let (columns, withheld) = recorded_columns(&root, || {
            crate::columnar_file::read_capture(&root.controls, |extent| {
                let mut raw = Vec::new();
                decoder.append(&source, extent, &mut raw)?;
                Ok(raw)
            })
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
            campaign: root.identity.campaign.clone(),
            columns: engine_allocation::boxed_slice(columns)?,
            withheld: engine_allocation::boxed_slice(withheld)?,
            root_identity: root.identity,
            source,
            runs: engine_allocation::boxed_slice(runs)?,
            segments: engine_allocation::boxed_slice(segments)?,
            damage: engine_allocation::boxed_slice(damage)?,
            verify: false,
            selected: [true; TABLE_COUNT],
            restarts: Restarts::default(),
        })
    }

    /// The recording as read from the same file, its runs and pages shared with this one, whose
    /// walks read only `tables` ([`Self::select`]): for walks of a few tables beside the walks of
    /// this one.
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
            identity: engine_allocation::copy_vec(&self.identity)?,
            root_identity: self.root_identity.clone(),
            controls: engine_allocation::boxed_slice(engine_allocation::copy_vec(&self.controls)?)?,
            campaign: engine_allocation::copy_vec(&self.campaign)?,
            columns: engine_allocation::boxed_slice(engine_allocation::copy_vec(&self.columns)?)?,
            withheld: engine_allocation::boxed_slice(withheld)?,
            runs: engine_allocation::boxed_slice(engine_allocation::collect_vec(
                self.runs.iter().cloned(),
            )?)?,
            segments: engine_allocation::boxed_slice(engine_allocation::copy_vec(&self.segments)?)?,
            damage: engine_allocation::boxed_slice(damage)?,
            verify: self.verify,
            selected: [true; TABLE_COUNT],
            restarts: Restarts::default(),
        };
        recording.select(tables);
        Ok(recording)
    }

    /// Walks restore, read and apply only `tables`, the tables their readers read; the others
    /// keep their statuses and no rows. A walk that verifies still reads every unit.
    pub fn select(&mut self, tables: &[TableId]) {
        self.restarts = Restarts::default();
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

    /// Whether walks decode `extent`, a page's unit: the manifests, the selected tables', or
    /// every one when they verify.
    fn decodes(&self, extent: &Extent) -> bool {
        extent.stream == framing::MANIFESTS
            || self.verify
            || self.selects(extent.stream as usize - 1)
    }

    /// How walks decode `page`'s units ([`Plan`]), but the units of the tables `lost` names:
    /// each piece's frame as `frame` finds it from its place among the page's units, a piece
    /// whose frame is damage going to `damaged`, counted as none. A stream's unit is decoded as
    /// it is read when `streams` allows it and that holds less than decoding it whole, a zstd
    /// context taking `context` bytes. The plan's stored bytes are the most one piece takes.
    fn plan(
        &self,
        page: &IndexPage,
        lost: &[(usize, io::Error)],
        damaged: &mut Vec<(Extent, io::Error)>,
        streams: bool,
        context: usize,
        frame: impl Fn(usize, &Extent) -> io::Result<Frame>,
    ) -> io::Result<Plan> {
        let mut plan = Plan::default();
        // The stream of the unit being gathered, its pieces, raw bytes and largest window.
        let mut open: Option<(u32, Range<usize>, usize, usize)> = None;
        let close = |plan: &mut Plan, unit: Option<(u32, Range<usize>, usize, usize)>| {
            let Some((_, slots, raw, window)) = unit else {
                return Ok(());
            };
            let streaming = streaming_bytes(window, context);
            let streamed = streams && streaming < raw;
            if streamed {
                plan.streams = plan.streams.saturating_add(streaming);
            } else {
                plan.raw = plan.raw.checked_add(raw).ok_or_else(engine_allocation::oom)?;
            }
            engine_allocation::reserve(&mut plan.units, 1)?;
            plan.units.push((slots, streamed));
            Ok::<(), io::Error>(())
        };
        for (slot, extent) in page.units.iter().enumerate() {
            if !self.decodes(extent) || gone(lost, extent) {
                continue;
            }
            let held = match frame(slot, extent) {
                Ok(held) => held,
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
            plan.stored =
                plan.stored.max(usize::try_from(extent.stored).map_err(|_| engine_allocation::oom())?);
            if let Some((_, slots, raw, window)) =
                open.as_mut().filter(|unit| unit.0 == extent.stream)
            {
                slots.end = slot + 1;
                *raw = raw.checked_add(held.raw).ok_or_else(engine_allocation::oom)?;
                *window = (*window).max(held.window);
                continue;
            }
            let unit = open.replace((extent.stream, slot..slot + 1, held.raw, held.window));
            close(&mut plan, unit)?;
        }
        close(&mut plan, open)?;
        Ok(plan)
    }

    /// The stored bytes of `page`'s units that walks decode and that lie within the
    /// recording's extents, together: what a walker gathers of the page from a file.
    fn gathered_bytes(&self, page: &IndexPage) -> io::Result<usize> {
        let mut total = 0_usize;
        for extent in page.units.iter().filter(|extent| self.decodes(extent)) {
            if let Ok(length) = self.source.place(extent) {
                total = total.checked_add(length).ok_or_else(engine_allocation::oom)?;
            }
        }
        Ok(total)
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

    /// Its identity block, every run's chain starts from: its schema, the campaign, the
    /// content identity and the capture mode.
    pub fn identity(&self) -> &[u8] {
        &self.identity
    }

    /// The capture the file's runs belong to, as the root's control records hold it; none
    /// when the root holds none.
    pub fn capture(&self) -> io::Result<Option<crate::columnar_file::Capture>> {
        let mut decoder = Decoder::new()?;
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

    /// The hour digest chain of run `run` through ordinal `ordinal`: after every hour from the
    /// run's start to it, that one too. A recording grown from this one holds the same there; a
    /// recording whose hours to it differ does not. The page holding it is read whole unless it
    /// ends there.
    pub fn chain_through(&self, run: u32, ordinal: u32) -> io::Result<[u8; 32]> {
        let pages = &self.run(run)?.pages;
        let at = pages
            .partition_point(|page| page.first <= ordinal)
            .checked_sub(1)
            .filter(|&at| ordinal - pages[at].first < pages[at].hours)
            .ok_or_else(|| invalid("No page of the run holds the ordinal"))?;
        if ordinal - pages[at].first + 1 == pages[at].hours {
            return Ok(self.page(run, at)?.chain);
        }
        let mut chain = match at.checked_sub(1) {
            Some(before) => self.page(run, before)?.chain,
            None => envelope::chain_start(&self.identity, run),
        };
        self.page_hours(run, at, ordinal, |_, manifest, _, sections| {
            chain = envelope::chain_hour(&chain, manifest, sections.iter().copied());
            Ok(true)
        })?;
        Ok(chain)
    }

    /// Each hour of run `run`'s page `at` from its first through ordinal `through`, as it was
    /// written: its ordinal, its manifest's bytes and its manifest, and its sections' bytes in
    /// its manifest's order, every unit of the page decoded whichever tables walks select.
    /// `visit` stops it by returning false.
    pub fn page_hours(
        &self,
        run: u32,
        at: usize,
        through: u32,
        mut visit: impl FnMut(u32, &[u8], &Manifest<'_>, &[&[u8]]) -> io::Result<bool>,
    ) -> io::Result<()> {
        let held = self.run(run)?;
        let page = self.page(run, at)?;
        let scope = DamageScope::Page {
            run,
            first: page.first,
            hours: page.hours,
        };
        let damaged = |error: io::Error| Damage::wrap(scope.clone(), error);
        let last = page
            .first
            .checked_add(page.hours)
            .and_then(|end| end.checked_sub(1))
            .ok_or_else(|| damaged(invalid("names no hours")))?;
        let mut decoder = Decoder::new()?;
        let mut units: Vec<Vec<u8>> = engine_allocation::vec(TABLE_COUNT + 1)?;
        units.resize_with(TABLE_COUNT + 1, Vec::new);
        for extent in &page.units {
            let unit = units
                .get_mut(extent.stream as usize)
                .ok_or_else(|| damaged(invalid("holds a unit of no stream")))?;
            decoder
                .append(&self.source, extent, unit)
                .map_err(|error| Damage::wrap(DamageScope::Extent(*extent), error))?;
        }
        let columns = &self.columns;
        let (manifests, units) = units.split_first().expect("a manifest stream");
        let mut manifest_unit = Reader::new(manifests);
        let mut tables =
            engine_allocation::collect_vec(units.iter().map(|unit| Reader::new(unit)))?;
        let mut sections: Vec<&[u8]> = engine_allocation::vec(TABLE_COUNT)?;
        for ordinal in page.first..=through.min(last) {
            let bytes = framing::next_manifest(&mut manifest_unit, manifests.len())
                .map_err(damaged)?
                .ok_or_else(|| damaged(invalid("does not hold exactly its hours")))?;
            let manifest = Manifest::read(bytes, columns).map_err(damaged)?;
            if manifest.branch != held.branch
                || i64::from(manifest.hour) != i64::from(held.first_hour) + i64::from(ordinal)
            {
                return Err(damaged(invalid(
                    "holds an hour off its run's branch and hours",
                )));
            }
            sections.clear();
            for section in &manifest.sections {
                let unit = tables
                    .get_mut(section.table)
                    .ok_or_else(|| damaged(invalid("holds a section of a table with no unit")))?;
                sections
                    .push(framing::next_section(unit, section.bytes as usize).map_err(damaged)?);
            }
            if !visit(ordinal, bytes, &manifest, &sections)? {
                break;
            }
        }
        Ok(())
    }

    /// `extent`, one of a page's units or its restart's, decoded.
    pub fn extent_bytes(&self, extent: &Extent) -> io::Result<Vec<u8>> {
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

/// The most a walker's buffers hold for one segment's walk: bytes decoded whole, stored bytes
/// read from a file, and what its units decoded as they are read hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    raw: usize,
    stored: usize,
    streams: usize,
}

impl Reads {
    /// What a walk reading both `self` and `other`, one after the other, holds at most.
    pub(crate) fn max(self, other: Self) -> Self {
        Self {
            raw: self.raw.max(other.raw),
            stored: self.stored.max(other.stored),
            streams: self.streams.max(other.streams),
        }
    }
}

/// How a page's units the walk decodes are decoded: each stream's pieces, among the page's
/// units, and whether they are decoded as they are read; and what that holds.
#[derive(Default)]
struct Plan {
    units: Vec<(Range<usize>, bool)>,
    raw: usize,
    stored: usize,
    streams: usize,
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

/// zstd's largest block: what a unit decoded as it is read holds beyond its window, coming in
/// and going out, and what it decodes ahead of the walk.
const BLOCK_MAX: usize = 128 << 10;

/// What decoding a unit as it is read holds, its largest frame's window `window`: a zstd
/// context of `context` bytes with its window and a block in and out, and a block ahead.
fn streaming_bytes(window: usize, context: usize) -> usize {
    context
        .saturating_add(window)
        .saturating_add(window.min(BLOCK_MAX).saturating_mul(3))
}

/// One piece of a stream's unit: the ordinals it covers, where its raw bytes sit in the unit,
/// its extent, and where its stored bytes begin in what the walk decodes them from.
struct Piece {
    first: u64,
    end: u64,
    start: usize,
    stop: usize,
    extent: Extent,
    input: usize,
}

/// A stream's unit of the page a walker decoded: its bytes in the arena, or the walker's
/// stream `at`, which decodes it as it is read; and its pieces.
#[derive(Clone)]
enum Unit {
    Whole {
        bytes: Range<usize>,
        pieces: Range<usize>,
    },
    Streamed {
        at: usize,
        pieces: Range<usize>,
    },
}

/// A unit decoded as the walk reads it, through its frames' windows: the bytes decoded that the
/// walk has not passed, from a multiple of eight, so a reader over them aligns as one over the
/// whole unit would.
struct Stream {
    context: DCtx<'static>,
    buffer: Vec<u8>,
    /// The unit's offsets of the buffer's first byte and of the walk, and its length.
    base: usize,
    at: usize,
    length: usize,
    /// The piece being decoded, its stored bytes taken in and its raw bytes given out.
    piece: usize,
    used: usize,
    decoded: usize,
}

impl Stream {
    fn new() -> io::Result<Self> {
        Ok(Self {
            context: DCtx::try_create().ok_or_else(engine_allocation::oom)?,
            buffer: Vec::new(),
            base: 0,
            at: 0,
            length: 0,
            piece: 0,
            used: 0,
            decoded: 0,
        })
    }

    /// Starts a unit of `length` raw bytes.
    fn start(&mut self, length: usize) -> io::Result<()> {
        self.context
            .reset(ResetDirective::SessionOnly)
            .map_err(|code| invalid(zstd_safe::get_error_name(code)))?;
        self.buffer.clear();
        (self.base, self.at, self.length) = (0, 0, length);
        (self.piece, self.used, self.decoded) = (0, 0, 0);
        Ok(())
    }

    /// Whether the piece being decoded has given out all its raw bytes but is not yet checked
    /// to end its frame there with all its stored bytes taken in.
    fn closing(&self, pieces: &[Piece]) -> bool {
        pieces
            .get(self.piece)
            .is_some_and(|piece| self.decoded == piece.extent.raw as usize)
    }

    /// Decodes until it holds the `count` bytes from the walk on, or through the unit's end;
    /// a piece is taken only once its one frame ends exactly at its raw length and its last
    /// stored byte, as a whole unit's decode takes it.
    fn fill(&mut self, count: usize, pieces: &[Piece], input: &[u8]) -> io::Result<()> {
        let want = self.at.saturating_add(count).min(self.length);
        if self.base + self.buffer.len() >= want && !self.closing(pieces) {
            return Ok(());
        }
        let passed = (self.at - self.base) & !7;
        if passed > 0 {
            self.buffer.drain(..passed);
            self.base += passed;
        }
        let ahead = want.saturating_add(BLOCK_MAX).min(self.length) - self.base;
        if self.buffer.capacity() < ahead {
            let room = ahead - self.buffer.len();
            engine_allocation::reserve_exact(&mut self.buffer, room)?;
        }
        while self.base + self.buffer.len() < want || self.closing(pieces) {
            let piece = pieces
                .get(self.piece)
                .ok_or_else(|| invalid("decompresses to another length"))?;
            let damaged = |error| Damage::wrap(DamageScope::Extent(piece.extent), error);
            let stored = &input[piece.input..][..piece.extent.stored as usize];
            let before = (self.buffer.len(), self.used);
            let mut source = InBuffer::around(stored);
            source.set_pos(self.used);
            let ended = {
                let mut out = OutBuffer::around_pos(&mut self.buffer, before.0);
                self.context
                    .decompress_stream(&mut out, &mut source)
                    .map_err(|code| damaged(invalid(zstd_safe::get_error_name(code))))?
                    == 0
            };
            let made = self.buffer.len() - before.0;
            self.used = source.pos();
            self.decoded += made;
            let raw = piece.extent.raw as usize;
            if self.decoded > raw {
                return Err(damaged(invalid("decompresses to another length")));
            }
            if ended {
                if self.used != stored.len() {
                    return Err(damaged(invalid(TRAILING)));
                }
                if self.decoded != raw {
                    return Err(damaged(invalid("decompresses to another length")));
                }
                (self.piece, self.used, self.decoded) = (self.piece + 1, 0, 0);
            } else if made == 0 && self.used == before.1 {
                return Err(damaged(invalid("decompresses to another length")));
            }
        }
        Ok(())
    }

    /// A reader over what it holds, at the walk.
    fn reader(&self) -> io::Result<Reader<'_>> {
        let mut reader = Reader::new(&self.buffer);
        reader.raw(self.at - self.base)?;
        Ok(reader)
    }
}

/// Where a record read from a unit lies in it: from its start, its own bytes from `body`, to
/// its end.
struct Read {
    start: usize,
    body: usize,
    stop: usize,
}

/// A stream's unit as the walk reads it, hour after hour: from the arena, or as its stream
/// decodes it from `input`.
enum Cursor<'a> {
    Whole {
        bytes: &'a [u8],
        reader: Reader<'a>,
        pieces: &'a [Piece],
    },
    Streamed {
        stream: &'a mut Stream,
        input: &'a [u8],
        pieces: &'a [Piece],
    },
}

impl<'a> Cursor<'a> {
    /// `unit`'s, from `arena`, or decoded by `stream` from `input`.
    fn new(
        arena: &'a [u8],
        pieces: &'a [Piece],
        stream: Option<&'a mut Stream>,
        input: &'a [u8],
        unit: &Unit,
    ) -> Option<Self> {
        Some(match unit {
            Unit::Whole { bytes, pieces: held } => Self::Whole {
                bytes: &arena[bytes.clone()],
                reader: Reader::new(&arena[bytes.clone()]),
                pieces: &pieces[held.clone()],
            },
            Unit::Streamed { pieces: held, .. } => Self::Streamed {
                stream: stream?,
                input,
                pieces: &pieces[held.clone()],
            },
        })
    }

    fn pieces(&self) -> &[Piece] {
        match self {
            Self::Whole { pieces, .. } | Self::Streamed { pieces, .. } => pieces,
        }
    }

    /// Whether `start..stop` of the unit lies within the piece covering `ordinal`.
    fn holds(&self, ordinal: u64, start: usize, stop: usize) -> bool {
        self.pieces().iter().any(|piece| {
            (piece.first..piece.end).contains(&ordinal)
                && piece.start <= start
                && stop <= piece.stop
        })
    }

    /// The unit's bytes `start..stop`, which it read last.
    fn slice(&self, start: usize, stop: usize) -> &[u8] {
        match self {
            Self::Whole { bytes, .. } => &bytes[start..stop],
            Self::Streamed { stream, .. } => &stream.buffer[start - stream.base..stop - stream.base],
        }
    }

    /// The next manifest of a manifest unit; none at its end.
    fn next_manifest(&mut self) -> io::Result<Option<Read>> {
        match self {
            Self::Whole { bytes, reader, .. } => {
                let start = reader.at();
                Ok(framing::next_manifest(reader, bytes.len())?.map(|manifest| Read {
                    start,
                    body: reader.at() - manifest.len(),
                    stop: reader.at(),
                }))
            }
            Self::Streamed {
                stream,
                input,
                pieces,
            } => {
                if stream.at == stream.length {
                    return Ok(None);
                }
                // A varint takes ten bytes at most.
                stream.fill(10, pieces, input)?;
                let (count, prefix) = {
                    let mut length = stream.reader()?;
                    let count = length.count()?;
                    (count, length.at() - (stream.at - stream.base))
                };
                stream.fill(prefix.saturating_add(count), pieces, input)?;
                let (manifest, read) = {
                    let mut reader = stream.reader()?;
                    let manifest =
                        framing::next_manifest(&mut reader, stream.length - stream.base)?
                            .ok_or_else(|| invalid("holds no manifest where one begins"))?;
                    (manifest.len(), reader.at())
                };
                let start = stream.at;
                stream.at = stream.base + read;
                Ok(Some(Read {
                    start,
                    body: stream.at - manifest,
                    stop: stream.at,
                }))
            }
        }
    }

    /// The next section of a table unit, `length` bytes: where its bytes begin and end.
    fn next_section(&mut self, length: usize) -> io::Result<(usize, usize)> {
        match self {
            Self::Whole { reader, .. } => {
                framing::next_section(reader, length)?;
                Ok((reader.at() - length, reader.at()))
            }
            Self::Streamed {
                stream,
                input,
                pieces,
            } => {
                let padding = stream.at.next_multiple_of(8) - stream.at;
                stream.fill(padding.saturating_add(length), pieces, input)?;
                let read = {
                    let mut reader = stream.reader()?;
                    framing::next_section(&mut reader, length)?;
                    reader.at()
                };
                stream.at = stream.base + read;
                Ok((stream.at - length, stream.at))
            }
        }
    }

    /// Refuses the unit's bytes after its last section but the padding it may end with.
    fn finish(&mut self) -> io::Result<()> {
        match self {
            Self::Whole { reader, .. } => framing::finish_sections(reader),
            Self::Streamed {
                stream,
                input,
                pieces,
            } => {
                stream.fill(stream.length - stream.at, pieces, input)?;
                framing::finish_sections(&stream.reader()?)
            }
        }
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
/// segment: its decoder and its buffers, which it gives back when it parks ([`Self::park`]).
pub struct Walker<'r> {
    recording: &'r Recording,
    decoder: Decoder,
    /// The page read last.
    raw: Vec<u8>,
    /// The page's units it gathers from a file, as (offset, stored length, place among the
    /// page's units), and where each one's stored bytes begin in its decoder's buffer.
    spans: Vec<(u64, usize, usize)>,
    starts: Vec<usize>,
    /// The page's units the walk reads whole, decoded, stream after stream; and those it
    /// decodes as it reads them, each through a stream of its own.
    arena: Vec<u8>,
    pieces: Vec<Piece>,
    streams: Vec<Stream>,
    /// Each stream's unit.
    units: Vec<Option<Unit>>,
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
            spans: Vec::new(),
            starts: Vec::new(),
            arena: Vec::new(),
            pieces: Vec::new(),
            streams: Vec::new(),
            units: engine_allocation::collect_vec((0..=TABLE_COUNT).map(|_| None))?,
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
        let bytes = [reads.stored, reads.streams, self.decoder.context()]
            .into_iter()
            .fold(reads.raw, usize::saturating_add);
        self.count(self.counted.max(bytes));
        true
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
        self.spans = Vec::new();
        self.starts = Vec::new();
        self.arena = Vec::new();
        self.pieces = Vec::new();
        self.streams = Vec::new();
        self.units.fill(None);
        self.lost.clear();
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
            stored: index_bytes(recording.run(segment.run)?, at)?,
            ..Reads::default()
        };
        match recording.read_page(segment.run, at, &mut self.decoder, &mut self.raw) {
            Ok(page) => {
                let restart = page
                    .restart
                    .as_ref()
                    .ok_or_else(|| invalid("The page holds no restart"))?;
                match recording.table_lengths(segment.run, restart) {
                    Ok((raw, stored)) => {
                        reads = reads.max(Reads {
                            raw,
                            stored,
                            streams: 0,
                        })
                    }
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
                stored: index_bytes(run, at)?,
                ..Reads::default()
            });
            let page = match recording.read_page(segment.run, at, &mut self.decoder, &mut self.raw)
            {
                Ok(page) => page,
                Err(error) if Damage::of(&error).is_some() => continue,
                Err(error) => return Err(error),
            };
            let plan = recording.plan(
                &page,
                &[],
                &mut damaged,
                true,
                self.decoder.context(),
                |_, extent| recording.source.frame(extent),
            )?;
            damaged.clear();
            let stored = recording.gathered_bytes(&page)?;
            reads = reads.max(Reads {
                raw: plan.raw,
                stored,
                streams: plan.streams,
            });
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

    /// Gives back its buffers, and the walk it was sized for.
    pub fn park(&mut self) {
        self.arena = Vec::new();
        self.streams = Vec::new();
        self.decoder.release();
        self.reads = None;
        self.count(0);
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
        if !isolate || extent.stream == framing::MANIFESTS || Damage::of(&error).is_none() {
            return Err(error);
        }
        engine_allocation::reserve(&mut self.lost, 1)?;
        self.lost.push((extent.stream as usize - 1, error));
        Ok(())
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
        engine_allocation::resize(&mut self.starts, page.units.len(), usize::MAX)?;
        for (slot, extent) in page.units.iter().enumerate() {
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

    /// Decodes `page`'s units of the streams the walk reads: into the arena, sized by their
    /// frames' lengths, or each set to be decoded as it is read through a stream of its own when
    /// that holds less ([`Recording::plan`]); a walk that `isolate`s damage leaves out a table
    /// whose unit it cannot read, and every later unit of a table it lost.
    fn load(&mut self, page: &IndexPage, isolate: bool) -> io::Result<()> {
        let recording = self.recording;
        let source = &recording.source;
        let mut damaged = Vec::new();
        // A gather that cannot be read leaves each unit to be read alone, as it refuses.
        let gathered = source.reads_file() && self.gather(page).is_ok();
        // A unit decoded as it is read keeps its stored bytes where the walk found them.
        let streams = gathered || !source.reads_file();
        let plan = {
            let (decoder, starts) = (&self.decoder, &self.starts);
            recording.plan(
                page,
                &self.lost,
                &mut damaged,
                streams,
                decoder.context(),
                |slot, extent| match starts.get(slot) {
                    Some(&start) if gathered && start != usize::MAX => {
                        decoder.gathered_frame(extent, start)
                    }
                    _ => source.frame(extent),
                },
            )?
        };
        for (extent, error) in damaged {
            self.lose(&extent, error, isolate)?;
        }
        self.admit(plan.raw, if gathered { 0 } else { plan.stored })?;
        self.arena.clear();
        self.pieces.clear();
        self.units.fill(None);
        let mut streamed = 0;
        for (slots, stream) in plan.units {
            if gone(&self.lost, &page.units[slots.start]) {
                continue;
            }
            if stream {
                if self.stream(page, slots, gathered, streamed, isolate)? {
                    streamed += 1;
                }
                continue;
            }
            for slot in slots {
                let extent = &page.units[slot];
                if gone(&self.lost, extent) {
                    continue;
                }
                let start = self.arena.len();
                let decoded = match self.starts.get(slot) {
                    Some(&at) if gathered && at != usize::MAX => {
                        self.decoder.append_gathered(extent, at, &mut self.arena)
                    }
                    _ => self.decoder.append(source, extent, &mut self.arena),
                };
                if let Err(error) = decoded {
                    // A stream's earlier pieces go with its damaged one.
                    self.arena.truncate(start);
                    self.units[extent.stream as usize] = None;
                    let error = Damage::wrap(DamageScope::Extent(*extent), error);
                    self.lose(extent, error, isolate)?;
                    continue;
                }
                let unit = self.units[extent.stream as usize].get_or_insert(Unit::Whole {
                    bytes: start..start,
                    pieces: self.pieces.len()..self.pieces.len(),
                });
                let Unit::Whole { bytes, pieces } = unit else {
                    return Err(invalid("A stream's unit is decoded two ways"));
                };
                engine_allocation::reserve(&mut self.pieces, 1)?;
                self.pieces.push(Piece {
                    first: u64::from(extent.first),
                    end: u64::from(extent.first) + u64::from(extent.hours),
                    start: start - bytes.start,
                    stop: self.arena.len() - bytes.start,
                    extent: *extent,
                    input: 0,
                });
                bytes.end = self.arena.len();
                pieces.end = self.pieces.len();
            }
        }
        Ok(())
    }

    /// Sets `page`'s unit of pieces `slots` to be decoded as it is read, by its stream `at`,
    /// their stored bytes `gathered` or in the recording's memory: whether it is, as its
    /// pieces' digests hold. One that does not is lost as a unit decoded whole would be.
    fn stream(
        &mut self,
        page: &IndexPage,
        slots: Range<usize>,
        gathered: bool,
        at: usize,
        isolate: bool,
    ) -> io::Result<bool> {
        let stream = page.units[slots.start].stream as usize;
        let first = self.pieces.len();
        let (mut length, mut failed) = (0_usize, None);
        {
            let input = match self.recording.source.memory() {
                Some(memory) if !gathered => memory,
                _ => self.decoder.gathered(),
            };
            for slot in slots {
                let extent = page.units[slot];
                let offset = match (gathered, self.starts.get(slot)) {
                    (true, Some(&start)) => start,
                    _ => usize::try_from(extent.offset).map_err(|_| engine_allocation::oom())?,
                };
                let stored = offset
                    .checked_add(extent.stored as usize)
                    .and_then(|end| input.get(offset..end));
                let checked = stored
                    .ok_or_else(|| invalid("lies outside the recording's extents"))
                    .and_then(|stored| super::input::check(&extent, stored));
                if let Err(error) = checked {
                    failed = Some((extent, error));
                    break;
                }
                let raw = usize::try_from(extent.raw).map_err(|_| engine_allocation::oom())?;
                engine_allocation::reserve(&mut self.pieces, 1)?;
                self.pieces.push(Piece {
                    first: u64::from(extent.first),
                    end: u64::from(extent.first) + u64::from(extent.hours),
                    start: length,
                    stop: length + raw,
                    extent,
                    input: offset,
                });
                length += raw;
            }
        }
        if let Some((extent, error)) = failed {
            self.pieces.truncate(first);
            let error = Damage::wrap(DamageScope::Extent(extent), error);
            self.lose(&extent, error, isolate)?;
            return Ok(false);
        }
        if self.streams.len() <= at {
            engine_allocation::reserve(&mut self.streams, 1)?;
            self.streams.push(Stream::new()?);
        }
        self.streams[at].start(length)?;
        self.units[stream] = Some(Unit::Streamed {
            at,
            pieces: first..self.pieces.len(),
        });
        Ok(true)
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
        // One walk decodes a restart while the others starting there wait to share it.
        let entry = recording.restarts.entry(run, at)?;
        let mut decoded = locked(&entry);
        let mut declared = Vec::new();
        for table in tables() {
            declared.clear();
            for &ordinal in &table.chunks {
                let at = held.chunk_at(ordinal)?;
                state.pin_strings(held.load(at)?)?;
                engine_allocation::reserve(&mut declared, 1)?;
                declared.push(held.restarts[at].ids);
            }
            let index = table.table as usize;
            let damaged = |error: io::Error| Damage::wrap(scope(table.table), error);
            let restoring =
                engine_allocation::labels::kind(engine_allocation::labels::Kind::WalkerTables);
            let shared = match decoded.iter().find(|(held, _)| *held == index) {
                Some((_, restored)) => restored.upgrade()?,
                None => None,
            };
            if let Some(restored) = shared {
                state.restore_restored(index, restored).map_err(damaged)?;
                continue;
            }
            self.arena.clear();
            self.decoder
                .append(&recording.source, &table.extent, &mut self.arena)
                .map_err(damaged)?;
            let restored = state.decode(index, &self.arena).map_err(damaged)?;
            let weak = restored.downgrade()?;
            state.restore_restored(index, restored).map_err(damaged)?;
            drop(restoring);
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
            let input = recording.source.memory().unwrap_or(self.decoder.gathered());
            let mut streams =
                engine_allocation::collect_vec(self.streams.iter_mut().map(Some))?;
            let units = &self.units;
            let mut cursor = |unit: Option<&Unit>| {
                let unit = unit?;
                let stream = match unit {
                    Unit::Whole { .. } => None,
                    Unit::Streamed { at, .. } => Some(streams.get_mut(*at)?.take()?),
                };
                Cursor::new(arena, pieces, stream, input, unit)
            };
            let mut manifests = cursor(units[0].as_ref())
                .ok_or_else(|| damaged(invalid("holds no manifests")))?;
            let mut tables: [Option<Cursor<'_>>; TABLE_COUNT] =
                std::array::from_fn(|table| cursor(units[table + 1].as_ref()));
            let mut ordinal = u64::from(page.first);
            loop {
                let Some(read) = manifests.next_manifest().map_err(damaged)? else {
                    break;
                };
                if ordinal >= end || !manifests.holds(ordinal, read.start, read.stop) {
                    return Err(damaged(invalid("holds a manifest off its ordinals")));
                }
                let bytes = manifests.slice(read.body, read.stop);
                let manifest = Manifest::read(bytes, &columns).map_err(damaged)?;
                if manifest.branch != run.branch || i64::from(manifest.hour) != hour(ordinal) {
                    return Err(damaged(invalid(
                        "holds an hour off its run's branch and hours",
                    )));
                }
                // A manifest names each table once, so its sections are no more than the tables.
                let mut found: [Option<(usize, usize)>; TABLE_COUNT] = [None; TABLE_COUNT];
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
                    let read = cursor
                        .next_section(section.bytes as usize)
                        .and_then(|(start, stop)| {
                            if cursor.holds(ordinal, start, stop) {
                                Ok((start, stop))
                            } else {
                                Err(invalid("holds a section outside its hour's piece"))
                            }
                        });
                    match read {
                        Ok(held) => found[index] = Some(held),
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
                let mut sections: [&[u8]; TABLE_COUNT] = [&[][..]; TABLE_COUNT];
                for (index, section) in manifest.sections.iter().enumerate() {
                    if let (Some((start, stop)), Some(Some(cursor))) =
                        (found[index], tables.get(section.table))
                    {
                        sections[index] = cursor.slice(start, stop);
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
                let applying = engine_allocation::labels::kind(
                    engine_allocation::labels::Kind::WalkerTables,
                );
                state
                    .apply(&manifest, |index| Ok(sections[index]))
                    .map_err(damaged)?;
                drop(applying);
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
            for (table, cursor) in tables.iter_mut().enumerate() {
                let finished = cursor.as_mut().map_or(Ok(()), |cursor| cursor.finish());
                match finished {
                    Err(error) if error.kind() != io::ErrorKind::InvalidData => return Err(error),
                    Err(_) => {}
                    Ok(()) => continue,
                }
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
