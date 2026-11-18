//! Rows at the playback head for every reader that takes a frame of rows (a tooltip, a panel, a
//! selection observer, a script evaluator). A reader declares the tables it reads and how fresh it
//! needs them ([`Cadence`]); it gets a frame holding those tables' rows as the recording's row
//! codecs make them from the playhead's cells, and every other table as [`not_declared`].
//!
//! One worker per playhead is its own reader of it: it declares every column of the union of the
//! row readers' tables, takes each step the head reaches into whole-table mirrors of its own
//! ([`Mirror`], the map's), and makes a table's rows only when a reader of it is due and the
//! table changed since they were made, and then only the rows the steps touched ([`view::patch`]),
//! on an earlier view of it no frame holds any more where it can, unless the table's own costs say
//! making every row is cheaper. A reader that feeds a picture is made rows only when it says it is
//! ready for them ([`Cadence::Ready`]), at the worker's next wake after a step changed a table it
//! reads, never ahead of it; the map's frame wakes the worker when it moves the head ([`wake`]).
//! Readers of one cadence are due at the same wake, and readers of a slower one on the ticks of the
//! fastest, so a table they share is made once for all of them.
//! Unchanged rows are the same allocation from frame to frame, so a reader keyed on a table's
//! identity recomputes only when that table changed. Nothing here runs on the frame thread, and
//! the frame never waits for rows.

pub mod counts;
#[cfg(any(test, feature = "test-support"))]
pub mod tests;
pub mod touched;
pub mod view;

use super::{Entry, lock, mirror::Mirror};
use crate::layers::rebuilt::panicked;
use touched::Touched;
use playhead::{
    Declared, Step,
    changes::{Changes, Lives, Written},
};
use replay_columnar::state::Kind;
use replay_data::{
    BranchId, Hour,
    rows::{TABLE_COUNT, TableId},
};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView, not_declared};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    panic::{AssertUnwindSafe, Location, catch_unwind},
    sync::{Arc, Condvar, Mutex, Weak},
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// How often the worker looks for steps when nothing wakes it: a frame at 60 Hz.
const TICK: Duration = Duration::from_micros(16_667);

/// The times a join passes over each table whole: the lane sought, the reset it ships, the mirror
/// rebuilt from it.
const JOIN_PASSES: usize = 3;

/// How fresh a reader needs its rows while playing; paused, after a seek or a newly declared table
/// it gets them at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cadence {
    /// Every step's rows, the head never landed at past a step (a proof, a script evaluator).
    EveryStep,
    /// A reader that feeds a picture of the map at the head: once it is ready for rows
    /// ([`RowReader::ready`]: it has finished with the frame it took), it is made one frame at the
    /// worker's next wake after a step changed a table it reads, from the newest step applied; not
    /// ready, it is made nothing however many steps pass. Its frame stands at every later hour no
    /// step changed it by ([`RowReader::through`]).
    Ready,
    /// At most `n` frames a second: a reader that feeds no picture of the map (a tooltip, a panel).
    PerSecond(u32),
}

impl Cadence {
    fn interval(self) -> Option<Duration> {
        match self {
            Self::EveryStep | Self::Ready => None,
            Self::PerSecond(times) => Some(Duration::from_secs(1) / times.max(1)),
        }
    }

    /// The tick of its clock `elapsed` falls in, every reader's clock starting with the worker's:
    /// a reader is due when the tick moved on since its last frame, so readers of one cadence are
    /// due at the same wake whenever each was opened. None every step.
    fn tick(self, elapsed: Duration) -> Option<u128> {
        self.interval().map(|interval| elapsed.as_nanos() / interval.as_nanos().max(1))
    }
}

/// `elapsed` on the ticks of the fastest of `cadences`: a reader of a slower one is due at the
/// first of them its own tick falls in, as often a second as its cadence asks, so the rows made
/// for the fastest are the rows it takes whenever it is due with them.
fn on_fastest(elapsed: Duration, cadences: impl IntoIterator<Item = Cadence>) -> Duration {
    match cadences.into_iter().filter_map(Cadence::interval).min() {
        Some(fastest) => {
            let fastest = fastest.as_nanos().max(1);
            Duration::from_nanos(u64::try_from(elapsed.as_nanos() / fastest * fastest).unwrap_or(u64::MAX))
        }
        None => elapsed,
    }
}

/// The recording fact a frame of rows carries that the playhead's cells do not: its branch. (The
/// capture mode and an unavailable table's reason come with each mirror.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recording {
    pub branch: BranchId,
}

/// How a table's view was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Make {
    /// Patched in place: an earlier view of it that no frame held any more.
    InPlace,
    /// Patched on a copy of the newest view, which a frame holds.
    Copied,
    Whole(Whole),
    /// The rows of one reader's keys alone ([`RowReader::declare_keyed`]).
    Keyed,
}

/// Why a view was made whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whole {
    /// Its first view since its rows arrived.
    First,
    /// A seek's reset, or the rows of a declaration of its own arriving.
    Reset,
    Gap,
    /// Its status moved.
    Status,
    Log,
    /// Rows without keys.
    Unkeyed,
    /// A text cell whose string was not named.
    Untold,
    /// Patching the rows touched would cost more.
    Cost,
    /// The patch was refused.
    Refused,
}

impl Whole {
    pub const ALL: [Self; 9] = [Self::First, Self::Reset, Self::Gap, Self::Status, Self::Log, Self::Unkeyed, Self::Untold, Self::Cost, Self::Refused];

    pub fn name(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Reset => "reset",
            Self::Gap => "gap",
            Self::Status => "status",
            Self::Log => "log",
            Self::Unkeyed => "unkeyed",
            Self::Untold => "untold",
            Self::Cost => "cost",
            Self::Refused => "refused",
        }
    }
}

/// What the worker measured, while timing is on.
#[derive(Clone, Debug, Default)]
pub struct Timings {
    /// Per table index: each build's nanoseconds and the rows it made (every row when made whole,
    /// those made again when patched).
    pub builds: BTreeMap<usize, Vec<(u64, usize)>>,
    /// Per table index: how each build in `builds` was made.
    pub makes: BTreeMap<usize, Vec<Make>>,
    /// Per table index: steps that changed it.
    pub changed: BTreeMap<usize, u64>,
    /// Per table index: change sets applied to its mirror.
    pub applies: BTreeMap<usize, u64>,
    /// Per table index: steps that renumbered its slots.
    pub renumbered: BTreeMap<usize, u64>,
    /// Each step's apply to the worker's mirrors, nanoseconds.
    pub apply: Vec<u64>,
    /// Steps taken, applied or passed over by a landing.
    pub steps: u64,
    /// Times the worker landed at the head instead of applying the steps it was behind by.
    pub landed: u64,
    pub resets: u64,
    pub published: u64,
    pub untold: u64,
    /// The thread rows were made on.
    pub built_on: Option<String>,
}

/// What the worker holds, in bytes: its mirrors, the rows made from them, and the earlier rows it
/// keeps that no frame holds any more, by table.
#[derive(Clone, Debug, Default)]
pub struct Held {
    pub mirrors: Vec<(TableId, usize)>,
    pub rows: Vec<(TableId, usize)>,
    pub spares: Vec<(TableId, usize)>,
}

/// A table a reader reads and the columns of it: every column, those of a mask (the bits of
/// [`replay_data::rows::mask`]), or none, its live keys alone. The rows it gets hold every live row
/// ascending by key; a field of a column no reader of the table named holds its kind's zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Need {
    pub table: TableId,
    pub columns: u64,
}

impl Need {
    pub const fn every(table: TableId) -> Self {
        Self { table, columns: u64::MAX }
    }

    pub const fn columns(table: TableId, mask: u64) -> Self {
        Self { table, columns: mask }
    }

    pub const fn keys(table: TableId) -> Self {
        Self { table, columns: 0 }
    }
}

/// The rows of each table made anew since a frame: keys with the columns written ([`view::EVERY`]
/// for a row inserted or deleted), or none for a table made whole, whose every row may differ. A
/// table not named had no rows made anew.
pub type Told = BTreeMap<TableId, Option<BTreeMap<u128, u64>>>;

/// Adds to `into` that `rows` of `table` were made anew (none: any row may have).
fn tell<'a>(into: &mut Told, table: TableId, rows: Option<impl IntoIterator<Item = (&'a u128, &'a u64)>>) {
    let held = into.entry(table).or_insert_with(|| Some(BTreeMap::new()));
    match (held, rows) {
        (Some(held), Some(rows)) => {
            for (&key, &columns) in rows {
                *held.entry(key).or_default() |= columns;
            }
        }
        (held, _) => *held = None,
    }
}

/// What a reader opened with [`Rows::telling`] is told changed: since its last frame published, and
/// since the frame it last took.
#[derive(Default)]
struct Telling {
    pending: Told,
    unread: Told,
}

/// A table a reader reads by key: the rows whose keys' top 64 bits are among `keys`, and the
/// count of its declarations of them.
#[derive(Clone)]
struct Keyed {
    table: usize,
    keys: Arc<BTreeSet<u64>>,
    version: u64,
}

/// The keys each table a reader reads by key holds in one of its frames.
#[derive(Clone, Debug, Default)]
pub struct HeldKeys(Vec<(usize, Arc<BTreeSet<u64>>)>);

impl HeldKeys {
    /// The keys `table`'s rows were made for; none for a table not read by key.
    pub fn of(&self, table: TableId) -> Option<&Arc<BTreeSet<u64>>> {
        self.0.iter().find(|(held, _)| *held == table.index()).map(|(_, keys)| keys)
    }
}

/// The top 64 bits of a 96-bit key: the division of a division's equipment row.
fn prefix(key: u128) -> u64 {
    (key >> 32) as u64
}

struct Wanted {
    /// Table indexes ascending, each with the columns read.
    tables: Vec<(usize, u64)>,
    /// Bumped at each change of `tables`: a frame is published only for the declaration it was
    /// made for.
    declared: u64,
    cadence: Cadence,
    /// Where the code opens it: its name in the counts.
    opened: &'static Location<'static>,
    frame: Option<Arc<DecodedFrame>>,
    /// The last hour of the newest step the worker applied that changed no table of `frame`: the
    /// hour its rows stand at.
    through: Hour,
    /// It has finished with `frame` ([`Cadence::Ready`]).
    ready: bool,
    /// Tables it reads by key ([`RowReader::declare_keyed`]), ascending, and the keys `frame`'s
    /// rows of each were made for.
    keyed: Vec<Keyed>,
    frame_keys: Vec<(usize, Arc<BTreeSet<u64>>)>,
    /// Frames published to it, and the playhead's resets taken when its frame was made.
    published: u64,
    resets: u64,
    /// The seek ([`playhead::Playhead::epoch`]) of the newest step its frame's rows stand through.
    seek: u64,
    told: Option<Telling>,
}

/// A reader's newest frame, the count of frames published to it, and the playhead's resets (a
/// seek, a piece's first hour) taken before it was made.
#[derive(Clone, Debug)]
pub struct Newest {
    pub frame: Arc<DecodedFrame>,
    pub published: u64,
    pub resets: u64,
}

#[derive(Default)]
struct Control {
    readers: BTreeMap<u64, Wanted>,
    next: u64,
    /// Bumped when a declaration changes.
    version: u64,
    /// The version the worker has declared to the playhead.
    seen: u64,
    /// The head set by a caller that drives it ([`Rows::drive`]), with the playhead's seek epoch
    /// then; none follows the clock.
    driven: Option<(Hour, u64)>,
    stop: bool,
    error: Option<String>,
    timing: Option<Timings>,
    held: Held,
    /// The tables declared whose rows have not arrived, by index.
    arriving: Vec<usize>,
    /// Holds the worker once between making frames and publishing them.
    #[cfg(any(test, feature = "test-support"))]
    pause: Option<Arc<Pause>>,
    /// Holds the worker once before it takes steps, after the given count of wakes that took some.
    #[cfg(any(test, feature = "test-support"))]
    hold: Option<(u32, Arc<Pause>)>,
    /// The worker's wakes done to their end, announced on `published`.
    #[cfg(any(test, feature = "test-support"))]
    wakes: u64,
    /// No reader is due at its cadence's tick, only at once ([`Rows::hold_cadence`]).
    #[cfg(any(test, feature = "test-support"))]
    cadence_held: bool,
}

struct Shared {
    entry: Arc<Entry>,
    recording: Recording,
    control: Mutex<Control>,
    /// Shared with [`OPEN`], so the map's frame wakes the worker ([`wake`]).
    wake: Arc<Condvar>,
    /// Signalled when a frame is published.
    published: Condvar,
}

/// One playhead's rows at the head, shared by its row readers.
pub struct Rows {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

static OPEN: Mutex<Vec<(Weak<Entry>, Weak<Rows>, Weak<Condvar>)>> = Mutex::new(Vec::new());

/// Wakes the rows workers of `entry`'s playhead: the map's frame moved the head they follow, so
/// the steps to it are taken and a reader due at them has its rows within the frame.
pub fn wake(entry: &Entry) {
    for (_, _, wake) in lock(&OPEN).iter().filter(|(held, ..)| std::ptr::eq(held.as_ptr(), entry)) {
        if let Some(wake) = wake.upgrade() {
            wake.notify_all();
        }
    }
}

impl Rows {
    /// The rows at the head of `entry`'s playhead stamped as `recording`: the one already open and
    /// running, or a new worker. The stamp is the branch the reader asked for, which names every hour of its
    /// timeline as the recording read as timelines names it; a reader asking another has its own.
    pub fn of(entry: &Arc<Entry>, recording: Recording) -> io::Result<Arc<Self>> {
        let mut open = lock(&OPEN);
        open.retain(|(entry, rows, _)| entry.strong_count() > 0 && rows.strong_count() > 0);
        if let Some(rows) = open.iter().filter(|(held, ..)| std::ptr::eq(held.as_ptr(), Arc::as_ptr(entry))).filter_map(|(_, rows, _)| rows.upgrade()).find(|rows| rows.shared.recording == recording && rows.error().is_none()) {
            return Ok(rows);
        }
        let shared = Arc::new(Shared { entry: Arc::clone(entry), recording, control: Mutex::default(), wake: Arc::new(Condvar::new()), published: Condvar::new() });
        let working = Arc::clone(&shared);
        let worker = std::thread::Builder::new().name("rows at the head".into()).spawn(move || {
            // At the head's priority: it is a reader of the playhead that must keep up with every step.
            viewer_foundation::worker_priority::enter(viewer_foundation::worker_priority::Class::Kept);
            let stopped = match catch_unwind(AssertUnwindSafe(|| Worker::new(&working).and_then(|worker| worker.run()))) {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error.to_string()),
                Err(payload) => Some(format!("its worker panicked ({})", panicked(&*payload))),
            };
            if let Some(error) = stopped {
                lock(&working.control).error = Some(error);
                working.published.notify_all();
            }
        })?;
        let rows = Arc::new(Self { shared, worker: Some(worker) });
        open.push((Arc::downgrade(entry), Arc::downgrade(&rows), Arc::downgrade(&rows.shared.wake)));
        Ok(rows)
    }

    /// The playhead it is the rows of.
    pub fn entry(&self) -> &Arc<Entry> {
        &self.shared.entry
    }

    /// A reader of every column of `tables` at `cadence`; it holds nothing until every table's
    /// reset is taken.
    #[track_caller]
    pub fn reader(self: &Arc<Self>, tables: &[TableId], cadence: Cadence) -> RowReader {
        self.reader_opened(&every(tables), cadence, Location::caller(), false)
    }

    /// A reader of every column of `tables` at `cadence` that takes, with each frame, the rows of
    /// them made anew since the frame it took before ([`RowReader::take`]): what changed is told,
    /// never found by comparing tables.
    #[track_caller]
    pub fn telling(self: &Arc<Self>, tables: &[TableId], cadence: Cadence) -> RowReader {
        self.reader_opened(&every(tables), cadence, Location::caller(), true)
    }

    /// A reader of `needs` (tables with the columns read) at `cadence`.
    #[track_caller]
    pub fn reader_of(self: &Arc<Self>, needs: &[Need], cadence: Cadence) -> RowReader {
        self.reader_opened(needs, cadence, Location::caller(), false)
    }

    fn reader_opened(self: &Arc<Self>, needs: &[Need], cadence: Cadence, opened: &'static Location<'static>, told: bool) -> RowReader {
        let mut control = lock(&self.shared.control);
        let id = control.next;
        control.next += 1;
        control.readers.insert(id, Wanted { tables: indexes(needs), declared: 0, cadence, opened, frame: None, through: 0, ready: true, keyed: Vec::new(), frame_keys: Vec::new(), published: 0, resets: 0, seek: 0, told: told.then(Telling::default) });
        control.version += 1;
        drop(control);
        self.shared.wake.notify_all();
        RowReader { rows: Arc::clone(self), id }
    }

    /// The head is `hour` from now on, not the clock's: for a caller that plays the playhead itself.
    /// A seek after it supersedes it until the next drive: the worker never advances the playhead
    /// to a head driven before the seek.
    pub fn drive(&self, hour: Hour) {
        let epoch = self.shared.entry.playhead().epoch();
        lock(&self.shared.control).driven = Some((hour, epoch));
        self.shared.wake.notify_all();
    }

    /// Times each build and apply from now on ([`Self::timings`]).
    pub fn time(&self) {
        lock(&self.shared.control).timing.get_or_insert_with(Timings::default);
    }

    pub fn timings(&self) -> Option<Timings> {
        lock(&self.shared.control).timing.clone()
    }

    pub fn held(&self) -> Held {
        lock(&self.shared.control).held.clone()
    }

    /// Waits until the worker has declared every reader's tables to the playhead; for a caller that
    /// seeks right after declaring (the proof tool, tests), never the window.
    pub fn settled(&self) {
        let mut control = lock(&self.shared.control);
        while control.seen != control.version && control.error.is_none() {
            control = self.shared.published.wait(control).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Waits until one reader declares every one of `tables` and the worker has declared it to the
    /// playhead; false at `until` or when the worker stopped. A reader opened on another thread
    /// has declared nothing yet when its owner returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn declared(&self, tables: &[TableId], until: Instant) -> bool {
        let mut control = lock(&self.shared.control);
        loop {
            let declared = control.readers.values().any(|wanted| tables.iter().all(|table| wanted.tables.iter().any(|&(held, _)| held == table.index())));
            if declared && control.seen == control.version {
                return true;
            }
            let now = Instant::now();
            if now >= until || control.error.is_some() {
                return false;
            }
            control = self.shared.published.wait_timeout(control, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Waits until no reader declares `table` and the worker has told the playhead; false at
    /// `until` or when the worker stopped. A reader closed on another thread is still declared when
    /// its owner returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn released(&self, table: TableId, until: Instant) -> bool {
        let mut control = lock(&self.shared.control);
        loop {
            let declared = control.readers.values().any(|wanted| wanted.tables.iter().any(|&(held, _)| held == table.index()));
            if !declared && control.seen == control.version {
                return true;
            }
            let now = Instant::now();
            if now >= until || control.error.is_some() {
                return false;
            }
            control = self.shared.published.wait_timeout(control, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Holds the worker the next time it has made frames, before it publishes them, until the
    /// returned pause is released.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pause_before_publish(&self) -> Arc<Pause> {
        let pause = Arc::new(Pause::default());
        lock(&self.shared.control).pause = Some(Arc::clone(&pause));
        pause
    }

    /// From now on no reader is due at its cadence's tick, only at once (a reset, a head held): a
    /// frame a test then waits for can only be one due at once.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_cadence(&self) {
        lock(&self.shared.control).cadence_held = true;
    }

    /// Holds the worker the next time it is about to take steps, until the returned pause is
    /// released: the head runs on without it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_before_take(&self) -> Arc<Pause> {
        self.hold_after_taking(0)
    }

    /// [`Self::hold_before_take`] once `wakes` more wakes have taken steps.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_after_taking(&self, wakes: u32) -> Arc<Pause> {
        let pause = Arc::new(Pause::default());
        lock(&self.shared.control).hold = Some((wakes, Arc::clone(&pause)));
        self.shared.wake.notify_all();
        pause
    }

    /// The worker's wakes done to their end so far.
    #[cfg(any(test, feature = "test-support"))]
    pub fn wakes(&self) -> u64 {
        lock(&self.shared.control).wakes
    }

    /// Waits until the worker has done a wake to its end past the `seen`th; false at `until` (the
    /// hang guard) or when the worker stopped.
    #[cfg(any(test, feature = "test-support"))]
    pub fn woke_past(&self, seen: u64, until: Instant) -> bool {
        let mut control = lock(&self.shared.control);
        while control.wakes <= seen {
            let now = Instant::now();
            if now >= until || control.error.is_some() {
                return false;
            }
            control = self.shared.published.wait_timeout(control, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
        true
    }

    /// The error that stopped the worker, by name.
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.control).error.clone()
    }
}

impl Drop for Rows {
    fn drop(&mut self) {
        lock(&self.shared.control).stop = true;
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Where a test holds the rows worker: made frames in hand, not yet published.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct Pause {
    /// Whether the worker is held, and whether the test released it.
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

#[cfg(any(test, feature = "test-support"))]
impl Pause {
    /// The worker's side: held until released, or a minute, after which it goes on and the
    /// test's waits fail by name.
    fn hold(&self) {
        let until = Instant::now() + Duration::from_secs(60);
        let mut state = lock(&self.state);
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            let Some(left) = until.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()) else { break };
            state = self.changed.wait_timeout(state, left).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Waits until the worker is held; false at `until`.
    pub fn holding(&self, until: Instant) -> bool {
        let mut state = lock(&self.state);
        while !state.0 {
            let Some(left) = until.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()) else { return false };
            state = self.changed.wait_timeout(state, left).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
        true
    }

    pub fn release(&self) {
        lock(&self.state).1 = true;
        self.changed.notify_all();
    }
}

/// One reader's rows at the head; its tables are retired when it is dropped.
pub struct RowReader {
    rows: Arc<Rows>,
    id: u64,
}

impl RowReader {
    /// The newest frame of its tables; none while a table it declared is loading. Never waits.
    pub fn frame(&self) -> Option<Arc<DecodedFrame>> {
        lock(&self.rows.shared.control).readers.get(&self.id).and_then(|wanted| wanted.frame.clone())
    }

    /// Why its rows stopped coming, by name, with the playhead's newest hour; none while they come.
    pub fn stopped(&self) -> Option<(Hour, String)> {
        let error = lock(&self.rows.shared.control).error.clone()?;
        Some((self.rows.shared.entry.playhead().newest()?, format!("Rows at the head stopped: {error}")))
    }

    /// The tables it declared whose rows have not arrived yet: what its next frame waits for.
    pub fn arriving(&self) -> Vec<TableId> {
        let control = lock(&self.rows.shared.control);
        let Some(wanted) = control.readers.get(&self.id) else { return Vec::new() };
        wanted.tables.iter().filter(|(table, _)| control.arriving.binary_search(table).is_ok()).map(|&(table, _)| TableId::ALL[table]).collect()
    }

    /// [`Self::take_keyed`] without the keys.
    #[cfg(any(test, feature = "test-support"))]
    pub fn take(&self) -> Option<(Arc<DecodedFrame>, Told)> {
        self.take_keyed().map(|(frame, told, _)| (frame, told))
    }

    /// [`Self::frame`] with the keys each table it reads by key holds in it, taken together.
    pub fn frame_keyed(&self) -> Option<(Arc<DecodedFrame>, HeldKeys)> {
        let control = lock(&self.rows.shared.control);
        let wanted = control.readers.get(&self.id)?;
        Some((wanted.frame.clone()?, HeldKeys(wanted.frame_keys.clone())))
    }

    /// Its newest frame, the rows of its tables made anew since the frame it last took (for a
    /// reader opened with [`Rows::telling`]; another is told nothing) and the keys each table it
    /// reads by key holds in the frame, taken together; none while it has no frame.
    pub fn take_keyed(&self) -> Option<(Arc<DecodedFrame>, Told, HeldKeys)> {
        let mut control = lock(&self.rows.shared.control);
        let wanted = control.readers.get_mut(&self.id)?;
        let frame = wanted.frame.clone()?;
        let told = wanted.told.as_mut().map(|telling| std::mem::take(&mut telling.unread)).unwrap_or_default();
        Some((frame, told, HeldKeys(wanted.frame_keys.clone())))
    }

    /// The hour the rows of `frame` stand at while it is this reader's newest frame: the newest
    /// step applied since it was made changed none of its tables. None once a newer frame replaced
    /// it.
    pub fn through(&self, frame: *const DecodedFrame) -> Option<Hour> {
        let control = lock(&self.rows.shared.control);
        let wanted = control.readers.get(&self.id)?;
        wanted.frame.as_ref().filter(|newest| std::ptr::eq(Arc::as_ptr(newest), frame)).map(|_| wanted.through)
    }

    /// [`Self::through`] with the seek ([`playhead::Playhead::epoch`]) of the newest step those rows
    /// stand through: rows of an older seek than the reader's may stand past an hour it sought back to.
    pub fn through_seek(&self, frame: *const DecodedFrame) -> Option<(Hour, u64)> {
        let control = lock(&self.rows.shared.control);
        let wanted = control.readers.get(&self.id)?;
        wanted.frame.as_ref().filter(|newest| std::ptr::eq(Arc::as_ptr(newest), frame)).map(|_| (wanted.through, wanted.seek))
    }

    /// It has finished with the frame it took: a reader [`Cadence::Ready`] is made its next frame
    /// at the worker's next wake after a step changed a table it reads, never before this. A maker
    /// says it when its last product is answered, a reader on the frame thread each frame it looks.
    pub fn ready(&self) {
        let mut control = lock(&self.rows.shared.control);
        let Some(wanted) = control.readers.get_mut(&self.id).filter(|wanted| wanted.cadence == Cadence::Ready && !wanted.ready) else { return };
        wanted.ready = true;
        if counts::enabled() {
            counts::asked(opened_at(wanted.opened));
        }
        drop(control);
        self.rows.shared.wake.notify_all();
    }

    /// Reads `table` by key from now on: the rows whose keys' top 64 bits (for a division's
    /// equipment, the division) are among `keys`, and no other. They come in its frame as that
    /// table's view, made from the worker's whole mirror of it, so a key newly asked is served at
    /// its next frame with no join; the frame's view holds those keys' rows alone, so its reader
    /// must ask no other ([`Self::frame_keyed`]). A table read so is not among its whole tables.
    pub fn declare_keyed(&self, table: TableId, keys: Arc<BTreeSet<u64>>) {
        let mut control = lock(&self.rows.shared.control);
        let Some(wanted) = control.readers.get_mut(&self.id) else { return };
        let joins = match wanted.keyed.iter_mut().find(|keyed| keyed.table == table.index()) {
            Some(keyed) if Arc::ptr_eq(&keyed.keys, &keys) => return,
            Some(keyed) => {
                (keyed.keys, keyed.version) = (keys, keyed.version + 1);
                false
            }
            None => {
                wanted.keyed.push(Keyed { table: table.index(), keys, version: 0 });
                wanted.keyed.sort_unstable_by_key(|keyed| keyed.table);
                true
            }
        };
        if joins {
            control.version += 1;
        }
        drop(control);
        self.rows.shared.wake.notify_all();
    }

    /// [`Self::frame`] with its publish count and the resets before it.
    pub fn newest(&self) -> Option<Newest> {
        lock(&self.rows.shared.control).readers.get(&self.id).and_then(|wanted| wanted.frame.clone().map(|frame| Newest { frame, published: wanted.published, resets: wanted.resets }))
    }

    /// Waits up to `limit` for a frame published after the `seen`th, or the worker to stop.
    pub fn wait_newer(&self, seen: u64, limit: Duration) -> Option<Newest> {
        let shared = &self.rows.shared;
        let until = Instant::now() + limit;
        let mut control = lock(&shared.control);
        loop {
            if let Some(wanted) = control.readers.get(&self.id)
                && wanted.published > seen
                && let Some(frame) = &wanted.frame
            {
                return Some(Newest { frame: Arc::clone(frame), published: wanted.published, resets: wanted.resets });
            }
            let now = Instant::now();
            if now >= until || control.error.is_some() {
                return None;
            }
            control = shared.published.wait_timeout(control, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Reads `tables` from now on: a table new to it arrives as a reset off the frame, and it has no
    /// frame until then.
    pub fn declare(&self, tables: &[TableId]) {
        self.declare_needs(&every(tables));
    }

    /// Reads `needs` from now on: a table or column new to it arrives as a reset off the frame.
    pub fn declare_needs(&self, needs: &[Need]) {
        let tables = indexes(needs);
        let mut control = lock(&self.rows.shared.control);
        if let Some(wanted) = control.readers.get_mut(&self.id)
            && wanted.tables != tables
        {
            wanted.tables = tables;
            wanted.declared += 1;
            (wanted.frame, wanted.ready) = (None, true);
            control.version += 1;
            drop(control);
            self.rows.shared.wake.notify_all();
        }
    }

    /// Waits until its frame satisfies `ready`, or the worker stops; for a caller that drives the
    /// head and must see a given hour (the proof tool, tests), never the window.
    pub fn wait(&self, mut ready: impl FnMut(&DecodedFrame) -> bool, limit: Duration) -> io::Result<Arc<DecodedFrame>> {
        let shared = &self.rows.shared;
        let until = Instant::now() + limit;
        let mut control = lock(&shared.control);
        loop {
            if let Some(error) = &control.error {
                return Err(io::Error::other(format!("Rows at the head stopped: {error}")));
            }
            if let Some(frame) = control.readers.get(&self.id).and_then(|wanted| wanted.frame.as_ref())
                && ready(frame)
            {
                return Ok(Arc::clone(frame));
            }
            let now = Instant::now();
            if now >= until {
                let newest = control.readers.get(&self.id).and_then(|wanted| wanted.frame.as_ref()).map(|frame| frame.hour);
                return Err(io::Error::other(format!("Rows at the head did not reach the hour asked in time: its newest frame is at {newest:?}, the head driven to {:?}", control.driven.map(|(hour, _)| hour))));
            }
            control = shared.published.wait_timeout(control, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Its newest frame once its rows stand at `hour` ([`Self::through`]): a frame made at `hour`
    /// or later, or one made before it that no step since changed a table of. A reader
    /// [`Cadence::Ready`] whose rows stand is made no new frame while the clock plays, so a wait on
    /// the frame's own hour would never be answered. Waited for within `limit`.
    pub fn wait_through(&self, hour: Hour, limit: Duration) -> io::Result<Arc<DecodedFrame>> {
        let shared = &self.rows.shared;
        let until = Instant::now() + limit;
        let mut control = lock(&shared.control);
        loop {
            if let Some(error) = &control.error {
                return Err(io::Error::other(format!("Rows at the head stopped: {error}")));
            }
            if let Some(wanted) = control.readers.get(&self.id)
                && let Some(frame) = &wanted.frame
                && wanted.through.max(frame.hour) >= hour
            {
                return Ok(Arc::clone(frame));
            }
            // Waiting for rows at an hour no frame of its stands at, it has finished with every frame
            // it holds: one the worker publishes while it waits (an earlier hour's, made before the
            // head reached this one) takes no readiness from it, or no frame of the hour would come.
            if let Some(wanted) = control.readers.get_mut(&self.id)
                && wanted.cadence == Cadence::Ready
                && !wanted.ready
            {
                wanted.ready = true;
                shared.wake.notify_all();
            }
            let now = Instant::now();
            if now >= until {
                let newest = control.readers.get(&self.id).and_then(|wanted| wanted.frame.as_ref().map(|frame| (frame.hour, wanted.through)));
                return Err(io::Error::other(format!("Rows at the head did not stand at hour {hour} in time: its newest frame is at and stands through {newest:?}, the tables arriving {:?}", control.arriving.iter().map(|&table| TableId::ALL[table]).collect::<Vec<_>>())));
            }
            control = shared.published.wait_timeout(control, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }
}

impl Drop for RowReader {
    fn drop(&mut self) {
        let mut control = lock(&self.rows.shared.control);
        if control.readers.remove(&self.id).is_some() {
            control.version += 1;
        }
        drop(control);
        self.rows.shared.wake.notify_all();
    }
}

impl std::fmt::Debug for RowReader {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("RowReader").field("id", &self.id).finish_non_exhaustive()
    }
}

/// A reader of rows at the head as a window part holds one: opened on the playhead the first time
/// it is asked, its tables those given and those [`Self::discover`] finds it reads, closed by
/// [`Self::end_frame`] after a frame that did not ask (a hover that ended, a panel closed). The
/// tables it learned are kept, so it opens with all of them next time.
#[derive(Debug, Default)]
pub struct AtHead {
    open: Option<(Weak<Entry>, RowReader)>,
    needs: Vec<Need>,
    cadence: Option<Cadence>,
    /// Where the code made it; none for a default one, named where it is first asked.
    made: Option<&'static Location<'static>>,
    asked: bool,
    /// Ready for its next rows only when its owner says so ([`Self::ready`]), not each time it is
    /// asked.
    by_owner: bool,
}

impl AtHead {
    #[track_caller]
    pub fn new(tables: &[TableId], cadence: Cadence) -> Self {
        Self::with(&every(tables), cadence)
    }

    /// A reader of `needs`: tables with the columns it reads ([`Need`]); a table it reads that it
    /// did not name is learned whole.
    #[track_caller]
    pub fn with(needs: &[Need], cadence: Cadence) -> Self {
        Self { open: None, needs: needs.to_vec(), cadence: Some(cadence), made: Some(Location::caller()), asked: false, by_owner: false }
    }

    /// The newest rows at the head of `entry`; none while a table is loading or the worker is
    /// refused (printed once by name).
    #[track_caller]
    pub fn frame(&mut self, entry: &Arc<Entry>, recording: Recording) -> Option<Arc<DecodedFrame>> {
        self.asked = true;
        if !self.open.as_ref().is_some_and(|(held, reader)| std::ptr::eq(held.as_ptr(), Arc::as_ptr(entry)) && reader.rows.shared.recording == recording) {
            let opened = *self.made.get_or_insert_with(Location::caller);
            self.open = match Rows::of(entry, recording) {
                Ok(rows) => Some((Arc::downgrade(entry), rows.reader_opened(&self.needs, self.cadence.unwrap_or(Cadence::PerSecond(4)), opened, false))),
                Err(error) => {
                    eprintln!("Rows at the head are refused: {error}");
                    None
                }
            };
        }
        // Asked, it has finished with the frame it gave before (unless its owner says when): its
        // next is made at the next step.
        let by_owner = self.by_owner;
        self.open.as_ref().and_then(|(_, reader)| {
            if !by_owner {
                reader.ready();
            }
            reader.frame()
        })
    }

    /// Ready for its next rows only when [`Self::ready`] says so.
    pub fn ready_by_owner(mut self) -> Self {
        self.by_owner = true;
        self
    }

    /// Its owner has finished with the frame it took ([`RowReader::ready`]).
    pub fn ready(&self) {
        if let Some((_, reader)) = &self.open {
            reader.ready();
        }
    }

    /// `read` over a frame this gave; none when it read a table not declared yet, which is declared
    /// and arrives off the frame.
    pub fn discover<T>(&mut self, read: impl FnOnce() -> T) -> Option<T> {
        let (value, missed) = replay_frame::frame::discovering_fields(read);
        if missed.is_empty() {
            return Some(value);
        }
        self.needs.extend(missed.into_iter().map(|(table, fields)| Need::columns(table, fields)));
        if let Some((_, reader)) = &self.open {
            reader.declare_needs(&self.needs);
        }
        None
    }

    /// Declares `tables`, which a read made on another thread found undeclared (`discovering` is
    /// per thread).
    pub fn learn(&mut self, tables: &[TableId]) {
        // Whole: a table held with some columns only is widened, not passed over.
        let new: Vec<TableId> = tables.iter().copied().filter(|table| !self.needs.iter().any(|need| need.table == *table && need.columns == u64::MAX)).collect();
        if new.is_empty() {
            return;
        }
        self.needs.extend(new.into_iter().map(Need::every));
        if let Some((_, reader)) = &self.open {
            reader.declare_needs(&self.needs);
        }
    }

    /// Closes the reader unless it was asked since the last call.
    pub fn end_frame(&mut self) {
        if !std::mem::take(&mut self.asked) {
            self.open = None;
        }
    }

    /// [`RowReader::through`] of its open reader.
    pub fn through(&self, frame: *const DecodedFrame) -> Option<Hour> {
        self.open.as_ref().and_then(|(_, reader)| reader.through(frame))
    }

    /// The tables its open reader waits for ([`RowReader::arriving`]); none while it is closed.
    pub fn arriving(&self) -> Vec<TableId> {
        self.open.as_ref().map_or_else(Vec::new, |(_, reader)| reader.arriving())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Its reader's newest frame that satisfies `ready`, waited for within `limit`; refused by
    /// name while it is not open.
    pub fn wait(&self, ready: impl FnMut(&DecodedFrame) -> bool, limit: Duration) -> io::Result<Arc<DecodedFrame>> {
        self.open.as_ref().ok_or_else(|| io::Error::other("The reader at the head is not open"))?.1.wait(ready, limit)
    }
}

/// The hour a product made from a frame of rows stands for: the frame's own, then each later hour
/// its reader says no step changed those rows by ([`RowReader::through`]), kept once the frame is
/// replaced. Held weakly: the frame's rows are not kept for it.
#[derive(Clone, Debug)]
pub struct Stands {
    frame: Weak<DecodedFrame>,
    hour: Hour,
}

impl Stands {
    /// Made from `frame`, at `hour`: its own unless its maker found it current at later rows.
    pub fn at(frame: Weak<DecodedFrame>, hour: Hour) -> Self {
        Self { frame, hour }
    }

    /// Asks `through` (a reader's [`RowReader::through`]) again; the hour it stands for.
    pub fn look(&mut self, through: impl FnOnce(*const DecodedFrame) -> Option<Hour>) -> Hour {
        if let Some(hour) = through(self.frame.as_ptr()) {
            self.hour = self.hour.max(hour);
        }
        self.hour
    }

    pub fn hour(&self) -> Hour {
        self.hour
    }

    pub fn frame(&self) -> &Weak<DecodedFrame> {
        &self.frame
    }
}

/// `view` marked with the columns it holds when they are not every one of the table's `columns`:
/// a read of another field is refused by name, never served.
fn held_fields(view: SharedView, held: u64, columns: usize) -> SharedView {
    let every = if columns >= 64 { u64::MAX } else { (1 << columns) - 1 };
    if held & every == every { view } else { view.with_fields(held) }
}

fn every(tables: &[TableId]) -> Vec<Need> {
    tables.iter().map(|&table| Need::every(table)).collect()
}

/// Each table's index and the union of the columns its needs name, ascending.
fn indexes(needs: &[Need]) -> Vec<(usize, u64)> {
    let mut set: BTreeMap<usize, u64> = BTreeMap::new();
    for need in needs {
        *set.entry(need.table.index()).or_default() |= need.columns;
    }
    set.into_iter().collect()
}

/// Fewer rows than this time the call more than the rows: not measured.
const MEASURED_FROM: usize = 32;

/// A copied row's cost, a row made whole costing one, until a copy is measured.
const COPY_UNMEASURED: f64 = 0.125;

/// What a table's makes cost it, in nanoseconds a row, as its own makes measured them.
#[derive(Clone, Copy, Debug, Default)]
struct Costs {
    whole: Option<f64>,
    patch: Option<f64>,
    copy: Option<f64>,
}

impl Costs {
    fn learn(held: &mut Option<f64>, took: Duration, rows: usize) {
        if rows >= MEASURED_FROM {
            let each = took.as_nanos() as f64 / rows as f64;
            *held = Some(held.map_or(each, |held| held * 0.75 + each * 0.25));
        }
    }

    /// A patched row's cost and a copied row's, a row made whole costing one: as measured, a
    /// patched row one and a copied row [`COPY_UNMEASURED`] until they are.
    fn relative(self) -> (f64, f64) {
        match self.whole.filter(|whole| *whole > 0.0) {
            Some(whole) => (self.patch.map_or(1.0, |patch| patch / whole), self.copy.map_or(COPY_UNMEASURED, |copy| copy / whole)),
            None => (1.0, COPY_UNMEASURED),
        }
    }

    /// What patching `touched` rows costs, `copied` rows copied first, a row made whole costing one.
    fn patching(self, touched: usize, copied: usize) -> f64 {
        let (patch, copy) = self.relative();
        touched as f64 * patch + copied as f64 * copy
    }

    /// The rows noted past which no patch of a view of `rows` pays, a row noted twice over (its
    /// insert and its cells) counted twice.
    fn bound(self, rows: usize) -> usize {
        (2.0 * rows.max(1) as f64 / self.relative().0.max(f64::EPSILON)).ceil() as usize
    }
}

/// One table the worker holds.
struct Kept {
    mirror: Option<Box<Mirror>>,
    kinds: Vec<Kind>,
    /// The columns declared: the union of its readers'.
    columns: u64,
    /// Bumped by a step that changed the table, a reset and a gap.
    stamp: u64,
    gap: bool,
    built: Option<(u64, SharedView)>,
    /// The rows `built` holds.
    rows: usize,
    /// The rows the steps touched since `built` was made; why it is made whole next instead.
    touched: Result<Touched, Whole>,
    /// The rows noted past which it is made whole next ([`Costs::bound`]).
    bound: usize,
    /// Views made before `built` that frames still held at its make, oldest first, each with the
    /// rows made anew from it to the next newer (the last, to `built`): the newest of them no frame
    /// holds any more is patched in place at the next make ([`Self::prune`]).
    spares: Vec<(SharedView, Vec<(u128, u64)>)>,
    costs: Costs,
    /// Whether `built` holds a text cell whose string was not named: made whole next.
    untold: bool,
    /// A reader is told which of its rows each make makes anew ([`Rows::telling`]).
    told: bool,
    /// Why its rows can no longer be made (its mirror failed a step), until its next reset.
    failed: Option<String>,
}

impl Kept {
    fn new(kinds: Vec<Kind>, columns: u64) -> Self {
        Self { mirror: None, kinds, columns, stamp: 0, gap: false, built: None, rows: 0, touched: Err(Whole::First), bound: 0, spares: Vec::new(), costs: Costs::default(), untold: false, told: false, failed: None }
    }

    /// Its mirror, views and touched rows let go: the table arrives again.
    fn arriving(&mut self) {
        (self.mirror, self.built, self.touched, self.failed) = (None, None, Err(Whole::First), None);
        self.spares.clear();
        self.stamp += 1;
    }

    /// Whether a reader of it can be answered: its mirror is held, or it failed and says why.
    fn answers(&self) -> bool {
        self.mirror.is_some() || self.failed.is_some()
    }

    /// The table's rows as the steps applied left them, made again only when a step touched it
    /// (its mirror is held); the build recorded in `made`.
    fn view(&mut self, table: usize, order: &mut Vec<(u128, u32)>, made: &mut Made) -> SharedView {
        if self.mirror.is_none()
            && let Some(reason) = &self.failed
        {
            return SharedView::from(TableView::Unavailable { reason: reason.clone() });
        }
        let view = match &self.built {
            Some((stamp, view)) if *stamp == self.stamp => view.clone(),
            _ => match catch_unwind(AssertUnwindSafe(|| self.make(table, order, made))) {
                Ok(view) => view,
                Err(payload) => self.failed_make(table, &*payload),
            },
        };
        let sampled = if matches!(view.held(), TableView::Complete(_)) && !self.gap { self.mirror.as_deref().and_then(Mirror::sampled) } else { None };
        view.with_sampled_at(sampled)
    }

    fn make(&mut self, table: usize, order: &mut Vec<(u128, u32)>, made: &mut Made) -> SharedView {
        let started = Instant::now();
        let previous = self.built.take().map(|(_, view)| view);
        let since = self.since();
        let (view, built, how) = self.remake(TableId::ALL[table], previous, since.as_deref().map_err(|why| *why), order);
        let took = started.elapsed();
        made.builds.push((table, took.as_nanos() as u64, built.rows, how));
        if let Make::Whole(_) = how {
            Costs::learn(&mut self.costs.whole, took, built.rows);
        }
        if self.told {
            made.told.push((table, since.ok()));
        }
        made.untold += built.untold;
        self.untold = built.untold > 0;
        let view = held_fields(SharedView::from(view), self.columns, self.kinds.len());
        self.rows = match view.held() {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
            _ => 0,
        };
        self.bound = self.costs.bound(self.rows);
        self.prune();
        self.built = Some((self.stamp, view.clone()));
        view
    }

    /// Lets go each earlier view no frame holds any more, its rows made anew passed to the view
    /// before it, and every view so old that patching the rows made anew since it would cost more
    /// than making them all. A view a frame holds costs nothing to keep; one released after this
    /// is kept until the next make, which patches the newest of them in place.
    fn prune(&mut self) {
        let mut at = 0;
        while at < self.spares.len() {
            if self.spares[at].0.is_shared() {
                at += 1;
                continue;
            }
            let (_, rows) = self.spares.remove(at);
            if let Some(before) = at.checked_sub(1) {
                let held = &mut self.spares[before].1;
                *held = touched::merge(held.iter().chain(&rows).copied().collect());
            }
        }
        let mut anew = Vec::new();
        for at in (0..self.spares.len()).rev() {
            anew = touched::merge(anew.into_iter().chain(self.spares[at].1.iter().copied()).collect());
            if self.costs.patching(anew.len(), 0) >= self.rows as f64 {
                self.spares.drain(..=at);
                break;
            }
        }
    }

    /// The rows of `keys` alone ([`RowReader::declare_keyed`]), made from the mirror: what they
    /// cost is those rows, never the table. The rows touched since the last such make are told to
    /// every telling reader of the table (each takes those of its keys); while a reader reads the
    /// table whole, any row may have changed.
    fn keyed(&mut self, table: usize, keys: &BTreeSet<u64>, order: &mut Vec<(u128, u32)>, made: &mut Made) -> SharedView {
        if self.mirror.is_none()
            && let Some(reason) = &self.failed
        {
            return SharedView::from(TableView::Unavailable { reason: reason.clone() });
        }
        match catch_unwind(AssertUnwindSafe(|| self.keyed_made(table, keys, order, made))) {
            Ok(view) => view,
            Err(payload) => self.failed_make(table, &*payload),
        }
    }

    /// A make of `table` that panicked: its rows unavailable by name until a step changes it, and
    /// its views and touched rows let go, so the next make is whole.
    fn failed_make(&mut self, table: usize, payload: &(dyn std::any::Any + Send)) -> SharedView {
        let view = SharedView::from(TableView::Unavailable { reason: format!("{:?}'s rows at the head could not be made ({})", TableId::ALL[table], panicked(payload)) });
        (self.touched, self.untold) = (Err(Whole::First), false);
        self.spares.clear();
        self.built = Some((self.stamp, view.clone()));
        view
    }

    fn keyed_made(&mut self, table: usize, keys: &BTreeSet<u64>, order: &mut Vec<(u128, u32)>, made: &mut Made) -> SharedView {
        let started = Instant::now();
        if self.told {
            let since = if self.built.is_none() { self.since().ok() } else { None };
            made.told.push((table, since));
        }
        let mirror = self.mirror.as_deref().expect("a view is made of a table held");
        let (view, built) = view::make_of(TableId::ALL[table], mirror, (&self.kinds, self.columns), self.gap, |key| keys.contains(&prefix(key)), order);
        made.builds.push((table, started.elapsed().as_nanos() as u64, built.rows, Make::Keyed));
        made.untold += built.untold;
        let sampled = if matches!(view, TableView::Complete(_)) && !self.gap { mirror.sampled() } else { None };
        held_fields(SharedView::from(view), self.columns, self.kinds.len()).with_sampled_at(sampled)
    }

    /// The rows made anew since `built`, ascending by key, the noting begun again; why it is made
    /// whole instead.
    fn since(&mut self) -> Result<Vec<(u128, u64)>, Whole> {
        let mirror = self.mirror.as_deref().expect("a view is made of a table held");
        let since = match &mut self.touched {
            Ok(touched) => touched.take(mirror).ok_or(Whole::Unkeyed),
            Err(why) => Err(*why),
        };
        if self.touched.is_err() {
            self.touched = Ok(Touched::default());
        }
        since.and_then(|rows| match (self.gap, self.untold) {
            (true, _) => Err(Whole::Gap),
            (_, true) => Err(Whole::Untold),
            _ => Ok(rows),
        })
    }

    /// The table's view made again from `previous` and the rows made anew since it (`since`), the
    /// cheapest way by what each costs this table: an earlier view no frame holds any more patched
    /// in place for every row made anew since it; `previous` patched, in place when no frame holds
    /// it and on a copy of its rows when one does; or every row made. How it was made.
    fn remake(&mut self, table: TableId, previous: Option<SharedView>, since: Result<&[(u128, u64)], Whole>, order: &mut Vec<(u128, u32)>) -> (TableView, view::Made, Make) {
        let (previous, touched) = match (previous, since) {
            (Some(previous), Ok(touched)) if holds_rows(&previous) => (previous, touched),
            (previous, since) => {
                self.spares.clear();
                let why = match (previous, since) {
                    (_, Err(why)) => why,
                    (None, _) => Whole::First,
                    (Some(_), Ok(_)) => Whole::Status,
                };
                return self.whole(table, why, order);
            }
        };
        let rows = self.rows;
        let shared = previous.is_shared();
        let on_newest = self.costs.patching(touched.len(), if shared { rows } else { 0 });
        let mut made = view::Made::default();
        if let Some(at) = self.spares.iter().rposition(|(spare, _)| !spare.is_shared()) {
            let anew = touched::merge(self.spares[at..].iter().flat_map(|(_, rows)| rows.iter().copied()).chain(touched.iter().copied()).collect());
            let on_spare = self.costs.patching(anew.len(), 0);
            if on_spare < on_newest && on_spare < rows as f64 {
                let (spare, _) = self.spares.drain(..=at).next_back().expect("the spare found");
                let started = Instant::now();
                let patched = spare.try_unwrap().map_err(|_| io::Error::other(format!("{table:?}'s spare is held"))).and_then(|held| self.patch(table, held, &anew, &mut made));
                if let Ok(view) = patched {
                    Costs::learn(&mut self.costs.patch, started.elapsed(), made.rows);
                    self.keep(previous, touched);
                    return (view, made, Make::InPlace);
                }
                made = view::Made::default();
            }
        }
        if on_newest >= rows as f64 {
            self.keep(previous, touched);
            return self.whole(table, Whole::Cost, order);
        }
        let (held, previous) = if shared {
            let started = Instant::now();
            let copied = copy(table, &previous);
            Costs::learn(&mut self.costs.copy, started.elapsed(), rows);
            (copied, Some(previous))
        } else {
            (previous.try_unwrap().map_err(|_| io::Error::other(format!("{table:?}'s view is held"))), None)
        };
        let started = Instant::now();
        let patched = held.and_then(|held| self.patch(table, held, touched, &mut made));
        match previous {
            Some(previous) => self.keep(previous, touched),
            None => self.extend(touched),
        }
        match patched {
            Ok(view) => {
                Costs::learn(&mut self.costs.patch, started.elapsed(), made.rows);
                (view, made, if shared { Make::Copied } else { Make::InPlace })
            }
            Err(_) => self.whole(table, Whole::Refused, order),
        }
    }

    /// `held` with the rows of `anew` made again from the mirror ([`view::patch`]).
    fn patch(&self, table: TableId, held: TableView, anew: &[(u128, u64)], made: &mut view::Made) -> io::Result<TableView> {
        let mirror = self.mirror.as_deref().expect("a view is made of a table held");
        view::patch(table, mirror, (&self.kinds, self.columns), held, anew, made)
    }

    /// Every row made from the mirror.
    fn whole(&self, table: TableId, why: Whole, order: &mut Vec<(u128, u32)>) -> (TableView, view::Made, Make) {
        let mirror = self.mirror.as_deref().expect("a view is made of a table held");
        let (view, made) = view::make(table, mirror, (&self.kinds, self.columns), self.gap, order);
        (view, made, Make::Whole(why))
    }

    /// `view`, the newest made, kept as a spare with the rows made anew since it.
    fn keep(&mut self, view: SharedView, since: &[(u128, u64)]) {
        self.spares.push((view, since.to_vec()));
    }

    /// The newest view was patched in place: the newest spare's rows made anew run to the view
    /// made from it.
    fn extend(&mut self, since: &[(u128, u64)]) {
        if let Some((_, rows)) = self.spares.last_mut() {
            *rows = touched::merge(rows.iter().chain(since).copied().collect());
        }
    }
}

/// What steps did to the tables held, by table index.
#[derive(Default)]
struct Applied {
    /// A row or the status changed.
    changed: Vec<usize>,
    /// A change set was applied to the mirror.
    applies: Vec<usize>,
    /// The slots were renumbered.
    renumbered: Vec<usize>,
}

/// One step into `kept`'s mirrors, a reset's states taken through `consumer`; what it did added to
/// `applied`, and each change set applied to `spent` (table, nanoseconds, bytes) when given. True
/// for a reset.
fn apply_step(kept: &mut [Option<Kept>], consumer: &playhead::Consumer, capture: Option<replay_data::scope::CaptureMode>, step: &Step, applied: &mut Applied, mut spent: Option<&mut Vec<(usize, u64, u64)>>) -> bool {
    if step.gap() {
        for (table, kept) in kept.iter_mut().enumerate() {
            if let Some(kept) = kept
                && !kept.gap
            {
                kept.gap = true;
                kept.stamp += 1;
                kept.touched = Err(Whole::Gap);
                applied.changed.push(table);
            }
        }
        return step.reset();
    }
    let mut reset = step.reset();
    for changes in step.tables() {
        let table = changes.table();
        let Some(Some(kept)) = kept.get_mut(table) else { continue };
        if kept.mirror.is_some() && !kept.gap && quiet(changes) {
            continue;
        }
        let started = spent.is_some().then(Instant::now);
        if changes.reset() {
            match consumer.state(step, table).map(|state| state.downcast::<Mirror>()) {
                Some(Ok(mut mirror)) => {
                    mirror.set_capture(capture);
                    kept.mirror = Some(mirror);
                    kept.failed = None;
                    reset = true;
                }
                // Another reader's join: a mirror given no state stands at the reset's head.
                _ if !step.reset() && kept.mirror.is_some() && !kept.gap => continue,
                _ => {}
            }
        }
        let Some(mirror) = &mut kept.mirror else { continue };
        let before = (mirror.status(), kept.gap);
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| mirror.apply(changes))) {
            kept.failed = Some(format!("{:?}'s rows at the head stopped at hour {}: a step could not be applied ({})", TableId::ALL[table], step.last(), panicked(&*payload)));
            (kept.mirror, kept.built, kept.touched) = (None, None, Err(Whole::First));
            kept.spares.clear();
            kept.stamp += 1;
            applied.changed.push(table);
            continue;
        }
        applied.applies.push(table);
        kept.gap = false;
        let moved = before != (mirror.status(), false);
        let rows = changes.reset() || !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty()) || moved;
        if rows {
            kept.stamp += 1;
            applied.changed.push(table);
        }
        if changes.renumber().is_some() {
            applied.renumbered.push(table);
        }
        if rows || changes.renumber().is_some() {
            note(&mut kept.touched, changes, moved, kept.bound);
        }
        if let (Some(spent), Some(started)) = (spent.as_deref_mut(), started) {
            spent.push((table, started.elapsed().as_nanos() as u64, changes.payload() as u64));
        }
    }
    reset
}

/// Rows of a few tables that their owner steps through, in order, to the hours it asks for: one
/// reader of the playhead of its own, so the rows at the end of any step are there however many
/// steps the map took in one frame (the map text's events' context). A table the recording holds
/// nothing of is not declared ([`Self::absent`]): its owner refuses it by name.
pub struct Stepped {
    entry: Arc<Entry>,
    reader: playhead::Reader,
    consumer: playhead::Consumer,
    kept: Vec<Option<Kept>>,
    tables: Vec<usize>,
    absent: Vec<TableId>,
    /// The last hour of the last step applied, and its seek.
    hour: Option<(Hour, u64)>,
    order: Vec<(u128, u32)>,
    /// Taken and not applied: a newer seek's, until a request of that seek.
    steps: Vec<Arc<Step>>,
}

impl Stepped {
    /// Every column of `tables`, keyed, declared to `entry`'s playhead now: before a seek they
    /// come with its reset, after one with a later step.
    pub fn new(entry: &Arc<Entry>, tables: &[TableId]) -> io::Result<Self> {
        let (held, absent): (Vec<TableId>, Vec<TableId>) = tables.iter().partition(|&&table| super::details::held_at_all(entry, table));
        let kinds = entry.kinds();
        let mut kept: Vec<Option<Kept>> = (0..TABLE_COUNT).map(|_| None).collect();
        let mut declared = Vec::with_capacity(held.len());
        for table in &held {
            let columns = kinds.get(table.index()).ok_or_else(|| io::Error::other(format!("No table {table:?} in the schema")))?;
            declared.push(Declared { table: table.index(), columns: (0..columns.len()).collect(), keyed: true, passing: false });
            kept[table.index()] = Some(Kept::new(columns.clone(), u64::MAX));
        }
        let playhead = entry.playhead();
        let reader = playhead.declare(declared, Some(super::mirror::rebuild(Arc::clone(&entry.pages))))?;
        let consumer = playhead.consumer(reader);
        Ok(Self { entry: Arc::clone(entry), reader, consumer, kept, tables: held.iter().map(|table| table.index()).collect(), absent, hour: None, order: Vec::new(), steps: Vec::new() })
    }

    /// The tables asked for that the recording holds nothing of.
    pub fn absent(&self) -> &[TableId] {
        &self.absent
    }

    /// The newest seek ([`playhead::Playhead::epoch`]).
    pub fn epoch(&self) -> u64 {
        self.entry.playhead().epoch()
    }

    /// Takes every step built through `hour` and applies, in order, those of seek `epoch`: a newer
    /// seek's step waits for a request of that seek, an older one's is dropped, and a request of a
    /// superseded seek takes nothing. The last hour applied, if of `epoch`.
    pub fn take_through(&mut self, hour: Hour, epoch: u64) -> Option<Hour> {
        if epoch < self.epoch() {
            return None;
        }
        self.consumer.take(hour, &mut self.steps);
        self.steps.retain(|step| step.epoch() >= epoch);
        let ready = self.steps.iter().take_while(|step| step.epoch() == epoch && step.last() <= hour).count();
        let capture = self.entry.capture.get().copied();
        let mut applied = Applied::default();
        for step in self.steps.drain(..ready) {
            self.hour = Some((step.last(), epoch));
            apply_step(&mut self.kept, &self.consumer, capture, &step, &mut applied, None);
        }
        self.hour.filter(|&(_, of)| of == epoch).map(|(hour, _)| hour)
    }

    /// The rows of its tables as the steps taken left them, stamped `branch` and `hour`; none
    /// while a table has not arrived.
    pub fn frame(&mut self, branch: BranchId, hour: Hour) -> Option<DecodedFrame> {
        if !self.tables.iter().all(|&table| self.kept[table].as_ref().is_some_and(Kept::answers)) {
            return None;
        }
        let mut frame = DecodedFrame { branch, hour, tables: std::array::from_fn(|index| not_declared(TableId::ALL[index])), metrics: Metrics::default() };
        let mut made = Made::default();
        for &table in &self.tables {
            let kept = self.kept[table].as_mut().expect("held");
            frame.tables[table] = kept.view(table, &mut self.order, &mut made);
        }
        Some(frame)
    }
}

impl Drop for Stepped {
    fn drop(&mut self) {
        self.entry.playhead().retire(self.reader);
    }
}

/// The views one wake built: (table, nanoseconds, rows, how) each, and the untold text cells; for
/// a table a reader is told of, the rows each build made anew (none: any row may have).
#[derive(Default)]
struct Made {
    builds: Vec<(usize, u64, usize, Make)>,
    untold: u64,
    told: Vec<(usize, Option<Vec<(u128, u64)>>)>,
    /// Rows new to one reader of a table by key: those of the keys it newly asked.
    told_reader: Vec<(u64, usize, Vec<(u128, u64)>)>,
}

/// What the worker reads of a reader at a wake.
struct Reading {
    id: u64,
    declared: u64,
    tables: Vec<usize>,
    keyed: Vec<Keyed>,
    cadence: Cadence,
    has_frame: bool,
    ready: bool,
}

/// What a wake gives a reader: a new frame; none, its frame standing at the hour applied; or none.
enum Due {
    /// With the keys of each table read by key it holds.
    Frame(Arc<DecodedFrame>, Vec<(usize, Arc<BTreeSet<u64>>)>),
    Stands(Hour),
    Not,
}

/// What the worker last published for a reader.
#[derive(Default)]
struct Shown {
    tick: Option<u128>,
    hour: Option<Hour>,
    stamps: Vec<u64>,
    /// Per table read by key: the table's stamp and the declaration's count it was made at, the
    /// keys, and the view.
    keyed: Vec<(usize, u64, u64, Arc<BTreeSet<u64>>, SharedView)>,
}

struct Worker<'a> {
    shared: &'a Shared,
    reader: playhead::Reader,
    consumer: playhead::Consumer,
    kept: Vec<Option<Kept>>,
    shown: HashMap<u64, Shown>,
    version: u64,
    /// The last hour of the last step applied.
    hour: Option<Hour>,
    /// Steps taken that reset every table.
    resets: u64,
    /// The seek of the newest step taken.
    seek: u64,
    order: Vec<(u128, u32)>,
    /// Where every reader's cadence clock starts.
    epoch: Instant,
    /// The skip to the head under way: since when, the steps it left and the bytes they carried.
    skipping: Option<(Instant, u64, u64)>,
    /// What applying has cost it: nanoseconds and the bytes of the change sets applied to tables
    /// it holds; and what its last landing cost, the applies that brought its tables back, with
    /// those of the landing under way.
    applied: (u64, u64),
    landing: Option<u64>,
    arriving_ns: u64,
}

/// However the worker ends, a panic included, its reader goes from the playhead.
impl Drop for Worker<'_> {
    fn drop(&mut self) {
        self.shared.entry.playhead().retire(self.reader);
    }
}

impl<'a> Worker<'a> {
    fn new(shared: &'a Shared) -> io::Result<Self> {
        let playhead = shared.entry.playhead();
        let reader = playhead.declare(Vec::new(), Some(super::mirror::rebuild(std::sync::Arc::clone(&shared.entry.pages))))?;
        let consumer = playhead.consumer(reader);
        // Far behind, it lands itself by what its own mirrors cost to rebuild ([`Self::lands`]).
        consumer.lands_itself();
        Ok(Self { shared, reader, consumer, kept: (0..TABLE_COUNT).map(|_| None).collect(), shown: HashMap::new(), version: u64::MAX, hour: None, resets: 0, seek: 0, order: Vec::new(), epoch: Instant::now(), skipping: None, applied: (0, 0), landing: None, arriving_ns: 0 })
    }

    fn run(mut self) -> io::Result<()> {
        let mut steps: Vec<Arc<Step>> = Vec::new();
        let mut idle = 0_u32;
        loop {
            let (mut lock_ns, mut wait_ns) = (0, 0);
            let (readers, driven, timing, redeclared) = {
                let asked = Instant::now();
                let mut control = lock(&self.shared.control);
                lock_ns += asked.elapsed().as_nanos() as u64;
                if control.stop {
                    break;
                }
                if control.version == self.version {
                    let slept = Instant::now();
                    control = self.shared.wake.wait_timeout(control, TICK).unwrap_or_else(std::sync::PoisonError::into_inner).0;
                    wait_ns += slept.elapsed().as_nanos() as u64;
                    if control.stop {
                        break;
                    }
                }
                let redeclared = control.version != self.version;
                if redeclared {
                    self.version = control.version;
                    self.redeclare(&control)?;
                    control.seen = self.version;
                    self.shared.published.notify_all();
                }
                let readers: Vec<Reading> = control.readers.iter().map(|(&id, wanted)| Reading { id, declared: wanted.declared, tables: wanted.tables.iter().map(|&(table, _)| table).collect(), keyed: wanted.keyed.clone(), cadence: wanted.cadence, has_frame: wanted.frame.is_some(), ready: wanted.ready }).collect();
                (readers, control.driven, control.timing.is_some(), redeclared)
            };
            #[cfg(any(test, feature = "test-support"))]
            {
                let hold = lock(&self.shared.control).hold.take_if(|(wakes, _)| *wakes == 0);
                if let Some((_, hold)) = hold {
                    hold.hold();
                    continue;
                }
            }
            // While a map draws, the rows follow the head its frame side took (a held clock's hour
            // may be another).
            let counting = counts::enabled();
            let phase = || counting.then(counts::Phase::begin);
            let spent = |phase: Option<counts::Phase>| phase.map(counts::Phase::end).unwrap_or_default();
            let taking = phase();
            let playhead = self.shared.entry.playhead();
            // A head driven before the newest seek is stale: advanced to, it would make the step
            // after the seek's reset run past every hour driven after it.
            let driven = driven.filter(|&(_, epoch)| epoch == playhead.epoch()).map(|(hour, _)| hour);
            let map = self.shared.entry.map_shown().filter(|_| driven.is_none());
            // A map that has not taken the newest seek's reset gives no head: the rows take nothing
            // of that seek until it does. The clock is asked only with no map and no driver.
            let head = match (driven, map) {
                (Some(driven), _) => Some(driven),
                (None, Some(shown)) => shown,
                (None, None) => Some(playhead.head(Instant::now())),
            };
            steps.clear();
            if let Some(head) = head {
                self.consumer.advance(head);
                self.consumer.take(head, &mut steps);
            }
            let mut wake = counts::Wake { taken: steps.len() as u64, take: spent(taking), wait_ns, ..counts::Wake::default() };
            let mut reset = false;
            let mut apply_ns = Vec::new();
            let mut applied = Applied::default();
            // A reader at a cadence wants the head, not the steps to it: unless one takes every
            // step, a worker far enough behind lands at the head.
            let skip = if readers.iter().any(|reader| reader.cadence == Cadence::EveryStep) { None } else { self.lands(&steps) };
            let landed = skip.is_some();
            if let Some(bytes) = skip {
                self.land();
                (self.skipping, self.arriving_ns) = (Some((Instant::now(), steps.len() as u64, bytes)), 0);
                wake.skipped = Some((steps.len() as u64, bytes));
            } else {
                let applying = phase();
                let held = if counting { self.mirror_bytes() } else { 0 };
                let began = Instant::now();
                for step in &steps {
                    self.resets += u64::from(step.reset());
                    // A step that brings a landed table back is the landing's cost, not an apply's.
                    let arriving = self.skipping.is_some() && self.kept.iter().flatten().any(|kept| kept.mirror.is_none());
                    let bytes = if arriving { 0 } else { self.payload(std::slice::from_ref(step)) };
                    let started = Instant::now();
                    reset |= self.apply(step, &mut applied, counting.then_some(&mut wake.tables));
                    let ns = started.elapsed().as_nanos() as u64;
                    if arriving {
                        self.arriving_ns += ns;
                    } else {
                        self.applied = (self.applied.0 + ns, self.applied.1 + bytes);
                    }
                    if timing {
                        apply_ns.push(ns);
                    }
                }
                wake.applied = steps.len() as u64;
                wake.apply_ns = began.elapsed().as_nanos() as u64;
                wake.apply = spent(applying);
                if counting {
                    wake.apply.grew = self.mirror_bytes() as i64 - held as i64;
                }
                wake.apply_cpu_ns = wake.apply.cpu_ns;
            }
            if let Some(step) = steps.last() {
                self.seek = step.epoch();
            }
            idle = if steps.is_empty() { idle.saturating_add(1) } else { 0 };
            // Two wakes without a step mean a held head only when the clock is not playing: a
            // clock playing slower than the wakes leaves wakes without a step every step.
            let playing = driven.is_none() && self.shared.entry.anchored() && !self.shared.entry.paused();
            let now = reset || (idle >= 2 && !playing);
            let clock = on_fastest(self.epoch.elapsed(), readers.iter().map(|reader| reader.cadence));
            let mut made = Made::default();
            let mut frames = Vec::new();
            let mut standing = Vec::new();
            let making = phase();
            self.shown.retain(|id, _| readers.iter().any(|reader| reader.id == *id));
            for reader in &readers {
                match self.due(reader, (now, clock), &mut made) {
                    Due::Frame(frame, keys) => frames.push((reader.id, reader.declared, frame, keys)),
                    Due::Stands(hour) => standing.push((reader.id, reader.declared, hour)),
                    Due::Not => {}
                }
            }
            wake.make = spent(making);
            wake.make.views = made.builds.len() as u64;
            wake.make.copies = made.builds.iter().filter(|&&(_, _, _, how)| how == Make::Copied).count() as u64;
            wake.make.rows = made.builds.iter().map(|&(_, _, rows, _)| rows as u64).sum();
            #[cfg(any(test, feature = "test-support"))]
            if !frames.is_empty() {
                let pause = lock(&self.shared.control).pause.take();
                if let Some(pause) = pause {
                    pause.hold();
                }
            }
            let asked = Instant::now();
            let mut control = lock(&self.shared.control);
            lock_ns += asked.elapsed().as_nanos() as u64;
            let publishing = phase();
            #[cfg(any(test, feature = "test-support"))]
            if let Some((wakes, _)) = &mut control.hold
                && !steps.is_empty()
            {
                *wakes = wakes.saturating_sub(1);
            }
            // A reader that declared again since its frame was made has none until one made for it.
            frames.retain(|(id, declared, ..)| control.readers.get(id).is_some_and(|wanted| wanted.declared == *declared));
            // Every reader told of a table hears of each make of it, whichever reader it was made
            // for; one that reads it by key, of the rows of its keys.
            for (table, rows) in &made.told {
                for wanted in control.readers.values_mut() {
                    let Some(telling) = &mut wanted.told else { continue };
                    if wanted.tables.binary_search_by_key(table, |&(held, _)| held).is_ok() {
                        tell(&mut telling.pending, TableId::ALL[*table], rows.as_ref().map(|rows| rows.iter().map(|(key, columns)| (key, columns))));
                    } else if let Some(keyed) = wanted.keyed.iter().find(|keyed| keyed.table == *table) {
                        tell(&mut telling.pending, TableId::ALL[*table], rows.as_ref().map(|rows| rows.iter().filter(|(key, _)| keyed.keys.contains(&prefix(*key))).map(|(key, columns)| (key, columns))));
                    }
                }
            }
            for (id, table, rows) in &made.told_reader {
                if let Some(telling) = control.readers.get_mut(id).and_then(|wanted| wanted.told.as_mut()) {
                    tell(&mut telling.pending, TableId::ALL[*table], Some(rows.iter().map(|(key, columns)| (key, columns))));
                }
            }
            if let Some(timings) = &mut control.timing {
                timings.apply.extend(apply_ns);
                timings.steps += steps.len() as u64;
                timings.landed += u64::from(landed);
                timings.resets += u64::from(reset);
                timings.untold += made.untold;
                for (tables, into) in [(&applied.changed, &mut timings.changed), (&applied.applies, &mut timings.applies), (&applied.renumbered, &mut timings.renumbered)] {
                    for &table in tables {
                        *into.entry(table).or_default() += 1;
                    }
                }
                for &(table, ns, rows, how) in &made.builds {
                    timings.builds.entry(table).or_default().push((ns, rows));
                    timings.makes.entry(table).or_default().push(how);
                }
                timings.published += frames.len() as u64;
                if !made.builds.is_empty() {
                    timings.built_on = std::thread::current().name().map(str::to_owned);
                }
            }
            let mut stood = false;
            for (id, declared, hour) in standing {
                // A seek's reset that changed none of its tables stands its frame at the seek's hour.
                if let Some(wanted) = control.readers.get_mut(&id).filter(|wanted| wanted.declared == declared && wanted.frame.is_some() && (wanted.through < hour || wanted.seek != self.seek)) {
                    (wanted.through, wanted.seek) = (hour, self.seek);
                    stood = true;
                }
            }
            let published = !frames.is_empty();
            for (id, _, frame, keys) in frames {
                if let Some(wanted) = control.readers.get_mut(&id) {
                    wanted.through = frame.hour;
                    wanted.frame = Some(frame);
                    wanted.frame_keys = keys;
                    wanted.ready = wanted.cadence != Cadence::Ready;
                    wanted.published += 1;
                    (wanted.resets, wanted.seek) = (self.resets, self.seek);
                    if let Some(telling) = &mut wanted.told {
                        for (table, rows) in std::mem::take(&mut telling.pending) {
                            tell(&mut telling.unread, table, rows.as_ref());
                        }
                    }
                    if counting {
                        wake.frames.push(opened_at(wanted.opened));
                    }
                }
            }
            if published || redeclared || landed || !steps.is_empty() {
                control.held = self.held();
                control.arriving = self.kept.iter().enumerate().filter(|(_, kept)| kept.as_ref().is_some_and(|kept| kept.mirror.is_none())).map(|(table, _)| table).collect();
            }
            drop(control);
            // A frame whose rows stood to a later hour answers a wait for that hour too.
            if published || stood {
                self.shared.published.notify_all();
            }
            wake.publish = spent(publishing);
            wake.lock_ns = lock_ns;
            if let Some((since, left, bytes)) = self.skipping
                && self.kept.iter().flatten().all(|kept| kept.mirror.is_some())
            {
                (self.skipping, self.landing) = (None, Some(self.arriving_ns));
                wake.skip_done = Some(counts::Skip { steps: left, bytes, ns: since.elapsed().as_nanos() as u64 });
            }
            if counting {
                wake.made = made.builds;
                let now = self.shared.entry.map_head().or(head).unwrap_or(Hour::MIN);
                steps.clear();
                self.consumer.ahead(&mut steps);
                wake.behind = Some((steps.iter().filter(|step| step.last() <= now).count() as u64, self.hour));
                steps.clear();
                counts::record(wake);
            }
            #[cfg(any(test, feature = "test-support"))]
            {
                lock(&self.shared.control).wakes += 1;
                self.shared.published.notify_all();
            }
        }
        Ok(())
    }

    /// Declares to the playhead, keyed, the union of the columns the row readers read of each
    /// table; a table no reader reads any more is let go. A table whose columns grew waits for its
    /// reset; one whose columns shrank keeps the extra columns until its next.
    fn redeclare(&mut self, control: &Control) -> io::Result<()> {
        let mut union: BTreeMap<usize, u64> = BTreeMap::new();
        for (table, columns) in control.readers.values().flat_map(|wanted| wanted.tables.iter().copied().chain(wanted.keyed.iter().map(|keyed| (keyed.table, u64::MAX)))) {
            *union.entry(table).or_default() |= columns;
        }
        let told: Vec<usize> = control.readers.values().filter(|wanted| wanted.told.is_some()).flat_map(|wanted| wanted.tables.iter().map(|&(table, _)| table).chain(wanted.keyed.iter().map(|keyed| keyed.table))).collect();
        // A table no reader reads whole notes every row touched: a view of a reader's keys is told
        // the rows of its keys among them, however many.
        let whole: Vec<usize> = control.readers.values().flat_map(|wanted| wanted.tables.iter().map(|&(table, _)| table)).collect();
        let kinds = self.shared.entry.kinds();
        let mut declared = Vec::with_capacity(union.len());
        for (&table, &mask) in &union {
            let columns = kinds.get(table).ok_or_else(|| io::Error::other(format!("No table {table} in the schema")))?;
            // Over the columns the table has: a mask naming no column of it more is no new column.
            let mask = mask & if columns.len() >= 64 { u64::MAX } else { (1 << columns.len()) - 1 };
            declared.push(Declared { table, columns: (0..columns.len()).filter(|&column| column >= 64 || mask & (1 << column) != 0).collect(), keyed: true, passing: false });
            match &mut self.kept[table] {
                Some(kept) if mask & !kept.columns != 0 => {
                    kept.columns |= mask;
                    kept.arriving();
                }
                Some(_) => {}
                None => self.kept[table] = Some(Kept::new(columns.clone(), mask)),
            }
            if let Some(kept) = &mut self.kept[table] {
                kept.told = told.contains(&table);
                if !whole.contains(&table) {
                    kept.bound = usize::MAX;
                }
            }
        }
        for (table, kept) in self.kept.iter_mut().enumerate() {
            if !union.contains_key(&table) {
                *kept = None;
            }
        }
        self.shared.entry.playhead().redeclare(self.reader, declared)
    }

    /// The bytes `steps`, taken and not applied, write into the tables held, when landing at the
    /// head costs less than applying them: applying at what its applies have cost a byte, against
    /// what its last landing cost (the applies that brought its tables back) or, before it landed,
    /// [`JOIN_PASSES`] times its tables whole. Every time it pays: a worker that falls behind again
    /// lands again. Never across a reset, which brings every table whole anyway, nor while a table
    /// waits for its reset.
    fn lands(&self, steps: &[Arc<Step>]) -> Option<u64> {
        if steps.len() < 2 || steps.iter().any(|step| step.reset()) {
            return None;
        }
        let mut whole = 0_u64;
        for kept in self.kept.iter().flatten() {
            whole += kept.mirror.as_ref()?.bytes() as u64;
        }
        let missed = self.payload(steps);
        let pays = match (self.applied, self.landing) {
            ((ns, bytes), Some(landing)) if bytes > 0 => missed as f64 * ns as f64 / bytes as f64 > landing as f64,
            _ => missed > JOIN_PASSES as u64 * whole,
        };
        (whole > 0 && pays).then_some(missed)
    }

    /// The bytes its mirrors hold.
    fn mirror_bytes(&self) -> usize {
        self.kept.iter().flatten().filter_map(|kept| kept.mirror.as_ref()).map(|mirror| mirror.bytes()).sum()
    }

    /// The bytes `steps` write into the tables it holds.
    fn payload(&self, steps: &[Arc<Step>]) -> u64 {
        steps.iter().flat_map(|step| step.tables()).filter(|changes| self.kept.get(changes.table()).is_some_and(Option::is_some)).map(|changes| changes.payload() as u64).sum()
    }

    /// Lands at the head instead of applying the steps it is behind by ([`playhead::Consumer::land`]):
    /// the steps not taken are passed over, and each table comes whole at the newest step with this
    /// worker's mirror rebuilt there from the playhead's lanes in memory. Every reader keeps its frame
    /// until its tables arrive.
    fn land(&mut self) {
        self.consumer.land();
        self.kept.iter_mut().flatten().for_each(Kept::arriving);
    }

    /// One step into the mirrors; what it did added to `applied`, each change set's apply to `spent`
    /// when given. True for a reset.
    fn apply(&mut self, step: &Step, applied: &mut Applied, spent: Option<&mut Vec<(usize, u64, u64)>>) -> bool {
        self.hour = Some(step.last());
        apply_step(&mut self.kept, &self.consumer, self.shared.entry.capture.get().copied(), step, applied, spent)
    }

    /// The reader's frame when something it reads moved since it was last shown and it is due: at
    /// `clock` ([`on_fastest`]) for a reader of a cadence, once ready for one that waits until it
    /// is ([`Cadence::Ready`]); `now` (a reset, or a head held) a cadence's reader is due whatever
    /// its tick, and a ready one at a new hour. Not due, the hour its frame stands at when no step
    /// since it changed a table it reads.
    fn due(&mut self, reading: &Reading, (now, clock): (bool, Duration), made: &mut Made) -> Due {
        let Reading { id, tables, keyed, cadence, has_frame, ready, .. } = reading;
        let (id, cadence, has_frame, ready) = (*id, *cadence, *has_frame, *ready);
        let cadence_held = self.cadence_held();
        let Some(hour) = self.hour else { return Due::Not };
        if !tables.iter().copied().chain(keyed.iter().map(|keyed| keyed.table)).all(|table| self.kept[table].as_ref().is_some_and(Kept::answers)) {
            return Due::Not;
        }
        let stamp = |kept: &[Option<Kept>], table: usize| kept[table].as_ref().map_or(0, |kept| kept.stamp);
        let mut stamps: Vec<u64> = tables.iter().map(|&table| stamp(&self.kept, table)).collect();
        stamps.extend(keyed.iter().flat_map(|keyed| [stamp(&self.kept, keyed.table), keyed.version]));
        let shown = self.shown.entry(id).or_default();
        // A reader of the Clock reads the hour itself: its rows never stand past their own hour.
        let stands = has_frame && shown.stamps == stamps && (shown.hour == Some(hour) || !tables.contains(&TableId::Clock.index()));
        if stands && shown.hour == Some(hour) {
            return Due::Stands(hour);
        }
        let tick = cadence.tick(clock);
        let due = match cadence {
            Cadence::Ready => ready && (!stands || now),
            _ => {
                now || !cadence_held
                    && match (tick, shown.tick) {
                        (None, _) | (_, None) => true,
                        (Some(tick), Some(last)) => tick > last,
                    }
            }
        };
        if has_frame && !due {
            return if stands { Due::Stands(hour) } else { Due::Not };
        }
        let before = std::mem::take(&mut shown.keyed);
        *shown = Shown { tick, hour: Some(hour), stamps, keyed: Vec::new() };
        let mut held: Vec<Option<SharedView>> = vec![None; TABLE_COUNT];
        for &table in tables {
            let kept = self.kept[table].as_mut().expect("checked above");
            held[table] = Some(kept.view(table, &mut self.order, made));
        }
        let mut views = Vec::with_capacity(keyed.len());
        let mut keys = Vec::with_capacity(keyed.len());
        for keyed in keyed {
            let kept = self.kept[keyed.table].as_mut().expect("checked above");
            let was = before.iter().find(|(table, ..)| *table == keyed.table);
            let view = match was {
                Some((_, stamp, version, _, view)) if *stamp == kept.stamp && *version == keyed.version => view.clone(),
                _ => {
                    let view = kept.keyed(keyed.table, &keyed.keys, &mut self.order, made);
                    // The rows of keys it did not hold before are new to it.
                    let asked: Vec<(u128, u64)> = self.order.iter().map(|&(key, _)| key).filter(|&key| was.is_none_or(|(.., keys, _)| !keys.contains(&prefix(key)))).map(|key| (key, view::EVERY)).collect();
                    if kept.told && !asked.is_empty() {
                        made.told_reader.push((id, keyed.table, asked));
                    }
                    view
                }
            };
            held[keyed.table] = Some(view.clone());
            keys.push((keyed.table, Arc::clone(&keyed.keys)));
            views.push((keyed.table, kept.stamp, keyed.version, Arc::clone(&keyed.keys), view));
        }
        if let Some(shown) = self.shown.get_mut(&id) {
            shown.keyed = views;
        }
        let frame = DecodedFrame { branch: self.shared.recording.branch, hour, tables: std::array::from_fn(|index| held[index].take().unwrap_or_else(|| not_declared(TableId::ALL[index]))), metrics: Metrics::default() };
        Due::Frame(Arc::new(frame), keys)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn cadence_held(&self) -> bool {
        lock(&self.shared.control).cadence_held
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn cadence_held(&self) -> bool {
        false
    }

    fn held(&self) -> Held {
        let mut held = Held::default();
        for (table, kept) in self.kept.iter().enumerate() {
            let Some(kept) = kept else { continue };
            let id = TableId::ALL[table];
            if let Some(mirror) = &kept.mirror {
                held.mirrors.push((id, mirror.bytes()));
            }
            if let Some((_, view)) = &kept.built {
                held.rows.push((id, view_bytes(view)));
            }
            let spare: usize = kept.spares.iter().filter(|(view, _)| !view.is_shared()).map(|(view, _)| view_bytes(view)).sum();
            if spare > 0 {
                held.spares.push((id, spare));
            }
        }
        held
    }
}

/// Adds to `touched` the rows `changes` touched, and moves them by its renumber; why the next make
/// must be whole instead: a reset, a log's rows, a status that `moved`, rows not keyed, or more
/// rows noted than `bound`, past which no patch pays.
fn note(touched: &mut Result<Touched, Whole>, changes: &Changes, moved: bool, bound: usize) {
    let Ok(rows) = touched else { return };
    let why = match (changes.reset(), changes.log().is_some(), moved) {
        (true, ..) => Some(Whole::Reset),
        (_, true, _) => Some(Whole::Log),
        (.., true) => Some(Whole::Status),
        _ => None,
    };
    let (Some(gone), Some(born), None) = (keyed(changes.deleted()), keyed(changes.inserted()), why) else {
        *touched = Err(why.unwrap_or(Whole::Unkeyed));
        return;
    };
    for &key in gone.iter().chain(born) {
        rows.key(key);
    }
    for written in changes.columns() {
        let column = if written.column() < 64 { 1 << written.column() } else { view::EVERY };
        for &slot in written.slots() {
            rows.write(slot, column);
        }
    }
    if let Some(old) = changes.renumber() {
        rows.renumber(old);
    }
    if rows.rows() > bound {
        *touched = Err(Whole::Cost);
    }
}

/// Whether `changes` changes nothing a mirror holds: no row, cell, status, string or slot.
fn quiet(changes: &Changes) -> bool {
    !changes.reset()
        && changes.status().is_none()
        && changes.renumber().is_none()
        && changes.deleted().is_empty()
        && changes.inserted().is_empty()
        && changes.texts().is_empty()
        && changes.log().is_none()
        && changes.columns().iter().all(Written::is_empty)
}

/// Each row's key, when every row of `lives` has one.
fn keyed(lives: &Lives) -> Option<&[u128]> {
    if lives.is_empty() {
        return Some(&[]);
    }
    lives.keys().filter(|keys| keys.len() == lives.len())
}

/// Whether `view` holds rows that can be patched.
fn holds_rows(view: &SharedView) -> bool {
    matches!(view.held(), TableView::Complete(_) | TableView::Truncated { .. })
}

/// The rows `shared` holds, copied to be patched while a frame holds them.
fn copy(table: TableId, shared: &SharedView) -> io::Result<TableView> {
    match shared.held() {
        TableView::Complete(rows) => rows.try_clone().map(TableView::Complete),
        TableView::Truncated { rows, missing } => rows.try_clone().map(|rows| TableView::Truncated { rows, missing: *missing }),
        _ => Err(io::Error::other(format!("{table:?} holds no rows to patch"))),
    }
}

/// A reader's name in the counts: the source file and line that opened it.
fn opened_at(opened: &Location<'_>) -> String {
    let file = opened.file().replace('\\', "/");
    let file = file.rsplit_once("src/").map_or(file.as_str(), |(_, tail)| tail);
    format!("{file}:{}", opened.line())
}

/// The bytes a view's rows or reason hold.
fn view_bytes(view: &SharedView) -> usize {
    if view.is_not_declared() {
        return 0;
    }
    match view.held() {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.heap_bytes(),
        TableView::Unavailable { reason } => reason.capacity(),
        TableView::NotRecorded { .. } => 0,
    }
}
