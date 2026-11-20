//! The rows the map side's makers build from, made from the side's own mirrors inside its step.
//!
//! A maker's product is global by construction, or its builder reads the content's scripts over
//! any table through the trigger evaluator, whose read surface is typed rows in key order. So rows
//! stay a maker's input — but they are made once, here, from the mirrors the side already holds,
//! instead of from a second copy of them kept at a cadence of its own.
//!
//! A maker declares the tables and columns it reads ([`Need`]); that declaration is part of the
//! same union the side's mirrors are built from, so a mirror holds every column a maker named. At
//! each step's apply the rows the change sets touched are noted into every declared set of columns
//! ([`Noting`], [`touched`]), the sets a maker reads whose rows moved are posted as jobs to this
//! store's make threads ([`Store::post`]), and a maker is given a frame the moment every set it
//! reads is made, while the layers that join no maker settle on the side's own thread. A view's
//! hour is the hour the side applied: nothing here chases an hour, and no maker waits for a wake.
//!
//! A set whose mirror is not home, or does not hold every column the maker declared, is withheld
//! as not declared rather than made: a column the mirror lacks would read as its kind's zero, and
//! a field not recorded is never a zero.

pub mod apart;
#[cfg(any(test, feature = "test-support"))]
pub mod tests;
pub mod touched;
pub mod view;

use super::{
    Entry, lock,
    mirror::{Mirror, Mirrors},
};
use crate::layers::rebuilt::panicked;
use playhead::changes::{Changes, Lives};
use replay_columnar::state::Kind;
use replay_data::{
    BranchId, Hour,
    rows::{TABLE_COUNT, TableId},
};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView, not_declared};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard, atomic::{AtomicU64, Ordering}},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use touched::Touched;

/// The recording fact a frame of rows carries that the playhead's cells do not: its branch. (The
/// capture mode and an unavailable table's reason come with each mirror.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recording {
    pub branch: BranchId,
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
pub fn tell<'a>(into: &mut Told, table: TableId, rows: Option<impl IntoIterator<Item = (&'a u128, &'a u64)>>) {
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

/// A table a reader reads by key: the rows whose keys' top 64 bits are among `keys`, and the
/// count of its declarations of them.
#[derive(Clone)]
pub struct Keyed {
    pub table: usize,
    pub keys: Arc<BTreeSet<u64>>,
    pub version: u64,
}

/// The keys each table a reader reads by key holds in one of its frames.
#[derive(Clone, Debug, Default)]
pub struct HeldKeys(Vec<(usize, Arc<BTreeSet<u64>>)>);

impl HeldKeys {
    /// The keys each table of `held` was made for, by table index.
    pub fn of_tables(held: Vec<(usize, Arc<BTreeSet<u64>>)>) -> Self {
        Self(held)
    }

    /// The keys `table`'s rows were made for; none for a table not read by key.
    pub fn of(&self, table: TableId) -> Option<&Arc<BTreeSet<u64>>> {
        self.0.iter().find(|(held, _)| *held == table.index()).map(|(_, keys)| keys)
    }
}

/// The top 64 bits of a 96-bit key: the division of a division's equipment row.
pub fn prefix(key: u128) -> u64 {
    (key >> 32) as u64
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


/// What the first make of a set of columns is taken to cost before one is measured: made whole,
/// it is the dearest the table has.
pub const FIRST_MAKE_NS: u64 = 500_000;


/// How often a reader's sets are made. There is no wake here: a step that moves a set a reader
/// reads owes it a view, so a reader of the picture asks for every step's and one that feeds no
/// picture (a tooltip, a panel, a dump) asks for as many a second as it draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// Every step that moved one of its sets.
    Step,
    /// Every step, once its reader has finished with the frame it was given ([`Reader::ready`],
    /// [`Reader::wait_newer`]): a reader whose own work is slower than the step is made one frame
    /// for each it takes, never one a step it will never read.
    Ready,
    /// At most `n` steps a second, and at once while it holds no frame of this seek (it was just
    /// opened, it declared a table, or the side sought).
    PerSecond(u32),
}

impl Pace {
    fn interval(self) -> Option<Duration> {
        match self {
            Self::Step | Self::Ready => None,
            Self::PerSecond(times) => Some(Duration::from_secs(1) / times.max(1)),
        }
    }
}


/// Fewer rows than this time the call more than the rows: not measured.
const MEASURED_FROM: usize = 32;

/// A copied row's cost, a row made whole costing one, until a copy is measured.
const COPY_UNMEASURED: f64 = 0.125;

/// What a table's makes cost it, in nanoseconds a row, as its own makes measured them.
#[derive(Clone, Copy, Debug, Default)]
pub struct Costs {
    pub whole: Option<f64>,
    pub patch: Option<f64>,
    pub copy: Option<f64>,
}

impl Costs {
    pub fn learn(held: &mut Option<f64>, took: Duration, rows: usize) {
        if rows >= MEASURED_FROM {
            let each = took.as_nanos() as f64 / rows as f64;
            *held = Some(held.map_or(each, |held| held * 0.75 + each * 0.25));
        }
    }

    /// A patched row's cost and a copied row's, a row made whole costing one: as measured, a
    /// patched row one and a copied row [`COPY_UNMEASURED`] until they are.
    pub fn relative(self) -> (f64, f64) {
        match self.whole.filter(|whole| *whole > 0.0) {
            Some(whole) => (self.patch.map_or(1.0, |patch| patch / whole), self.copy.map_or(COPY_UNMEASURED, |copy| copy / whole)),
            None => (1.0, COPY_UNMEASURED),
        }
    }

    /// What patching `touched` rows costs, `copied` rows copied first, a row made whole costing one.
    pub fn patching(self, touched: usize, copied: usize) -> f64 {
        let (patch, copy) = self.relative();
        touched as f64 * patch + copied as f64 * copy
    }

    /// The rows noted past which no patch of a view of `rows` pays, a row noted twice over (its
    /// insert and its cells) counted twice.
    pub fn bound(self, rows: usize) -> usize {
        (2.0 * rows.max(1) as f64 / self.relative().0.max(f64::EPSILON)).ceil() as usize
    }
}


/// One set of a table's columns, and the rows made of them behind the lock the job making them
/// holds: the columns are read without taking it, so a reader of one set never waits for the job
/// making another's.
pub struct Set {
    pub columns: u64,
    pub made: Mutex<Columns>,
}

/// The rows made of one set of a table's columns.
pub struct Columns {
    /// Bumped by a step that changed a row of these columns, a reset and a gap.
    pub stamp: u64,
    pub built: Option<(u64, SharedView)>,
    /// The rows `built` holds.
    pub rows: usize,
    /// The rows the steps touched since `built` was made; why it is made whole next instead.
    pub touched: Result<Touched, Whole>,
    /// The rows noted past which it is made whole next ([`Costs::bound`]); never, while no reader
    /// reads these columns whole (a view of a reader's keys is made from the mirror).
    pub bound: usize,
    /// Whether a reader reads these columns whole, and not by key alone.
    pub whole_read: bool,
    /// Views made before `built` that frames still held at its make, oldest first, each with the
    /// rows made anew from it to the next newer (the last, to `built`): the newest of them no frame
    /// holds any more is patched in place at the next make ([`Self::prune`]).
    pub spares: Vec<(SharedView, Vec<(u128, u64)>)>,
    pub costs: Costs,
    /// Whether `built` holds a text cell whose string was not named: made whole next.
    pub untold: bool,
    /// A reader is told which of its rows each make makes anew ([`Rows::telling`]).
    pub told: bool,
    /// What its last make took: what the next is expected to cost ([`Columns::costing`]).
    pub last_ns: u64,
}


/// A set's rows with no lock taken: its owner holds the table alone (the steps applied, a
/// declaration changed), so no job is making them.
pub fn mine(made: &mut Mutex<Columns>) -> &mut Columns {
    made.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The table one view is made from: its mirror, its columns' kinds, and whether the recording has
/// a gap here.
pub type Source<'a> = (&'a Mirror, &'a [Kind], bool);

impl Columns {
    pub fn new() -> Self {
        Self { stamp: 0, built: None, rows: 0, touched: Err(Whole::First), bound: 0, whole_read: false, spares: Vec::new(), costs: Costs::default(), untold: false, told: false, last_ns: 0 }
    }

    pub fn arriving(&mut self) {
        (self.built, self.touched) = (None, Err(Whole::First));
        self.spares.clear();
        self.stamp += 1;
    }

    pub fn make(&mut self, table: usize, columns: u64, from: Source<'_>, order: &mut Vec<(u128, u32)>, made: &mut Made) -> SharedView {
        let at = made.at();
        made.made_on();
        let started = Instant::now();
        let previous = self.built.take().map(|(_, view)| view);
        let since = self.since(from);
        // No row of these columns was touched: the rows stand as they were made, the allocation
        // included, so a reader keyed on their identity has nothing to do.
        if let (Some(previous), Ok(touched)) = (&previous, &since)
            && touched.is_empty()
            && holds_rows(previous)
        {
            self.built = Some((self.stamp, previous.clone()));
            return previous.clone();
        }
        let (view, built, how) = self.remake(TableId::ALL[table], columns, previous, since.as_deref().map_err(|why| *why), from, order);
        let took = started.elapsed();
        self.last_ns = took.as_nanos() as u64;
        made.builds.push((table, took.as_nanos() as u64, built.rows, how, at));
        if let Make::Whole(_) = how {
            Costs::learn(&mut self.costs.whole, took, built.rows);
        }
        if self.told {
            made.told.push((table, columns, since.ok()));
        }
        made.untold += built.untold;
        self.untold = built.untold > 0;
        let view = held_fields(SharedView::from(view), columns, from.1.len());
        self.rows = match view.held() {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
            _ => 0,
        };
        self.bound = if self.whole_read { self.costs.bound(self.rows) } else { usize::MAX };
        self.prune();
        self.built = Some((self.stamp, view.clone()));
        view
    }

    /// Lets go each earlier view no frame holds any more, its rows made anew passed to the view
    /// before it, and every view so old that patching the rows made anew since it would cost more
    /// than making them all. A view a frame holds costs nothing to keep; one released after this
    /// is kept until the next make, which patches the newest of them in place.
    pub fn prune(&mut self) {
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

    /// A make of `table` that panicked: its rows unavailable by name until a step changes it, and
    /// its views and touched rows let go, so the next make is whole.
    pub fn failed_make(&mut self, table: usize, payload: &(dyn std::any::Any + Send)) -> SharedView {
        let view = SharedView::from(TableView::Unavailable { reason: format!("{:?}'s rows at the head could not be made ({})", TableId::ALL[table], panicked(payload)) });
        (self.touched, self.untold) = (Err(Whole::First), false);
        self.spares.clear();
        self.built = Some((self.stamp, view.clone()));
        view
    }

    /// The rows of `keys` alone ([`RowReader::declare_keyed`]), made from the mirror: what they
    /// cost is those rows, never the table. The rows touched since the last such make are told to
    /// every telling reader of the table (each takes those of its keys); while a reader reads the
    /// table whole, any row may have changed.
    pub fn keyed_made(&mut self, table: usize, columns: u64, from: Source<'_>, keys: &BTreeSet<u64>, order: &mut Vec<(u128, u32)>, made: &mut Made) -> SharedView {
        let (mirror, kinds, gap) = from;
        let at = made.at();
        made.made_on();
        let started = Instant::now();
        if self.told {
            let since = if self.built.is_none() { self.since(from).ok() } else { None };
            made.told.push((table, columns, since));
        }
        let (view, built) = view::make_of(TableId::ALL[table], mirror, (kinds, columns), gap, |key| keys.contains(&prefix(key)), order);
        made.builds.push((table, started.elapsed().as_nanos() as u64, built.rows, Make::Keyed, at));
        made.untold += built.untold;
        let sampled = if matches!(view, TableView::Complete(_)) && !gap { mirror.sampled() } else { None };
        held_fields(SharedView::from(view), columns, kinds.len()).with_sampled_at(sampled)
    }

    /// The rows made anew since `built`, ascending by key, the noting begun again; why it is made
    /// whole instead.
    pub fn since(&mut self, (mirror, _, gap): Source<'_>) -> Result<Vec<(u128, u64)>, Whole> {
        if gap {
            self.touched = Ok(Touched::default());
            return Err(Whole::Gap);
        }
        let since = match &mut self.touched {
            Ok(touched) => touched.take(mirror).ok_or(Whole::Unkeyed),
            Err(why) => Err(*why),
        };
        if self.touched.is_err() {
            self.touched = Ok(Touched::default());
        }
        since.and_then(|rows| if self.untold { Err(Whole::Untold) } else { Ok(rows) })
    }

    /// The table's view made again from `previous` and the rows made anew since it (`since`), the
    /// cheapest way by what each costs this table: an earlier view no frame holds any more patched
    /// in place for every row made anew since it; `previous` patched, in place when no frame holds
    /// it and on a copy of its rows when one does; or every row made. How it was made.
    pub fn remake(&mut self, table: TableId, columns: u64, previous: Option<SharedView>, since: Result<&[(u128, u64)], Whole>, from: Source<'_>, order: &mut Vec<(u128, u32)>) -> (TableView, view::Made, Make) {
        let (previous, touched) = match (previous, since) {
            (Some(previous), Ok(touched)) if holds_rows(&previous) => (previous, touched),
            (previous, since) => {
                self.spares.clear();
                let why = match (previous, since) {
                    (_, Err(why)) => why,
                    (None, _) => Whole::First,
                    (Some(_), Ok(_)) => Whole::Status,
                };
                return self.whole(table, columns, why, from, order);
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
                let patched = spare.try_unwrap().map_err(|_| io::Error::other(format!("{table:?}'s spare is held"))).and_then(|held| self.patch(table, columns, held, &anew, from, &mut made));
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
            return self.whole(table, columns, Whole::Cost, from, order);
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
        let patched = held.and_then(|held| self.patch(table, columns, held, touched, from, &mut made));
        match previous {
            Some(previous) => self.keep(previous, touched),
            None => self.extend(touched),
        }
        match patched {
            Ok(view) => {
                Costs::learn(&mut self.costs.patch, started.elapsed(), made.rows);
                (view, made, if shared { Make::Copied } else { Make::InPlace })
            }
            Err(_) => self.whole(table, columns, Whole::Refused, from, order),
        }
    }

    /// `held` with the rows of `anew` made again from the mirror ([`view::patch`]).
    pub fn patch(&self, table: TableId, columns: u64, held: TableView, anew: &[(u128, u64)], (mirror, kinds, _): Source<'_>, made: &mut view::Made) -> io::Result<TableView> {
        view::patch(table, mirror, (kinds, columns), held, anew, made)
    }

    /// Every row made from the mirror.
    pub fn whole(&self, table: TableId, columns: u64, why: Whole, (mirror, kinds, gap): Source<'_>, order: &mut Vec<(u128, u32)>) -> (TableView, view::Made, Make) {
        let (view, made) = view::make(table, mirror, (kinds, columns), gap, order);
        (view, made, Make::Whole(why))
    }

    /// `view`, the newest made, kept as a spare with the rows made anew since it.
    pub fn keep(&mut self, view: SharedView, since: &[(u128, u64)]) {
        self.spares.push((view, since.to_vec()));
    }

    /// The newest view was patched in place: the newest spare's rows made anew run to the view
    /// made from it.
    pub fn extend(&mut self, since: &[(u128, u64)]) {
        if let Some((_, rows)) = self.spares.last_mut() {
            *rows = touched::merge(rows.iter().chain(since).copied().collect());
        }
    }
}


/// The views one wake built: (table, nanoseconds, rows, how) each, and the untold text cells; for
/// a table a reader is told of, the rows each build made anew (none: any row may have).
#[derive(Default)]
pub struct Made {
    /// Per view made: its table, what it took, the rows it made, how, and when its make began
    /// from the wake's start ([`Made::origin`]).
    pub builds: Vec<(usize, u64, usize, Make, u64)>,
    pub untold: u64,
    /// Per make a reader is told of: its table and columns, and the rows it made anew.
    pub told: Vec<(usize, u64, Option<Vec<(u128, u64)>>)>,
    /// Rows new to one reader of a table by key: those of the keys it newly asked.
    pub told_reader: Vec<(u64, usize, Vec<(u128, u64)>)>,
    /// What of each has been told to the readers ([`told`]): a frame is published as its views are
    /// made, so the makes are told as they come.
    pub told_from: usize,
    pub told_reader_from: usize,
    /// Where the wake these views are made at began; none while nothing is counted.
    pub origin: Option<Instant>,
    /// The thread the first of these views was made on.
    pub on: Option<String>,
}

impl Made {
    pub fn since(origin: Option<Instant>) -> Self {
        Self { origin, ..Self::default() }
    }

    /// Now, from the wake's start; zero while nothing is counted.
    pub fn at(&self) -> u64 {
        self.origin.map_or(0, |origin| origin.elapsed().as_nanos() as u64)
    }

    /// What another thread's job made added to this.
    pub fn merge(&mut self, mut other: Self) {
        self.builds.append(&mut other.builds);
        self.told.append(&mut other.told);
        self.told_reader.append(&mut other.told_reader);
        self.untold += other.untold;
        self.on = self.on.take().or(other.on);
    }

    /// This thread made a view.
    pub fn made_on(&mut self) {
        if self.on.is_none() {
            self.on = std::thread::current().name().map(str::to_owned);
        }
    }
}


/// Adds to `touched` the rows `changes` touched in the columns of `held`, and moves them by its
/// renumber; why the next make must be whole instead: a reset, a log's rows, a status that `moved`,
/// rows not keyed, or more rows noted than `bound`, past which no patch pays. A cell written
/// outside `held` touches no row of a view holding those columns: it reads the column's zero.
pub fn note(touched: &mut Result<Touched, Whole>, changes: &Changes, moved: bool, held: u64, bound: usize) {
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
        if column & held == 0 {
            continue;
        }
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


/// Each row's key, when every row of `lives` has one.
pub fn keyed(lives: &Lives) -> Option<&[u128]> {
    if lives.is_empty() {
        return Some(&[]);
    }
    lives.keys().filter(|keys| keys.len() == lives.len())
}

/// Whether `view` holds rows that can be patched.
pub fn holds_rows(view: &SharedView) -> bool {
    matches!(view.held(), TableView::Complete(_) | TableView::Truncated { .. })
}

/// The rows `shared` holds, copied to be patched while a frame holds them.
pub fn copy(table: TableId, shared: &SharedView) -> io::Result<TableView> {
    match shared.held() {
        TableView::Complete(rows) => rows.try_clone().map(TableView::Complete),
        TableView::Truncated { rows, missing } => rows.try_clone().map(|rows| TableView::Truncated { rows, missing: *missing }),
        _ => Err(io::Error::other(format!("{table:?} holds no rows to patch"))),
    }
}


/// `view` marked with the columns it holds when they are not every one of the table's `columns`:
/// a read of another field is refused by name, never served.
pub fn held_fields(view: SharedView, held: u64, columns: usize) -> SharedView {
    let every = if columns >= 64 { u64::MAX } else { (1 << columns) - 1 };
    if held & every == every { view } else { view.with_fields(held) }
}

/// `mask` over the `columns` a table has: a bit naming no column of it names nothing.
pub fn over(mask: u64, columns: usize) -> u64 {
    if columns >= 64 { mask } else { mask & ((1 << columns) - 1) }
}


/// Threads the store makes its views on: enough that a step's sets are made while the side
/// settles the layers that join no maker, few enough that the frame, the side and the makers'
/// builds keep their own. They are the store's own, spawned as a step first owes a view and
/// parked between steps, so no step pays to start one.
const MAKE_THREADS: usize = 4;

/// What one of those threads is called.
const MAKE_THREAD: &str = "map views: making";

/// One table of the store: the sets of columns the makers declared of it, and the rows made of
/// each from the side's mirror of it. A maker that reads two columns of a wide table pays for
/// those two, whatever another maker of the same table reads.
struct Kept {
    kinds: Vec<Kind>,
    /// The union of its sets' columns: what the side's mirror of it must hold.
    columns: u64,
    /// Whether the last step left it in a gap.
    gap: bool,
    /// One per set of columns declared, in no order; a set is always within `columns`.
    views: Vec<Set>,
}

impl Kept {
    fn new(kinds: Vec<Kind>) -> Self {
        Self { kinds, columns: 0, gap: false, views: Vec::new() }
    }

    /// Where the rows of `columns` are kept; none for a set no maker declared.
    fn set(&self, columns: u64) -> Option<usize> {
        let columns = over(columns, self.kinds.len());
        self.views.iter().position(|held| held.columns == columns)
    }

    /// Where the rows of `columns` are kept, a view of its own made from the mirror if the set is
    /// new.
    fn at(&mut self, columns: u64) -> usize {
        let columns = over(columns, self.kinds.len());
        match self.set(columns) {
            Some(at) => at,
            None => {
                self.views.push(Set { columns, made: Mutex::new(Columns::new()) });
                self.columns |= columns;
                self.views.len() - 1
            }
        }
    }

    /// Whether the rows of `columns` must be made again before a maker reading them is answered.
    fn stale(&self, columns: u64) -> bool {
        self.set(columns).is_none_or(|at| {
            let held = lock(&self.views[at].made);
            held.built.as_ref().is_none_or(|(stamp, _)| *stamp != held.stamp)
        })
    }

    /// What making the rows of `columns` is expected to cost, in nanoseconds: what its last make
    /// took, or [`FIRST_MAKE_NS`] for a set not made since its table arrived. Zero for a set
    /// already made at this step.
    fn costing(&self, columns: u64) -> u64 {
        let Some(at) = self.set(columns) else { return FIRST_MAKE_NS };
        let held = lock(&self.views[at].made);
        if held.built.as_ref().is_some_and(|(stamp, _)| *stamp == held.stamp) {
            return 0;
        }
        if held.last_ns > 0 { held.last_ns } else { FIRST_MAKE_NS }
    }

    /// Its views let go: its mirror is replaced or a column is newly declared, so the next make
    /// of every set is whole.
    fn arriving(&mut self) {
        self.views.iter_mut().for_each(|held| mine(&mut held.made).arriving());
    }

    /// What the rows of `columns` stand at: bumped by every step that changed a row of them, a
    /// reset and a gap. Zero for a set no maker declared.
    fn stamp(&self, columns: u64) -> u64 {
        self.set(columns).map_or(0, |at| lock(&self.views[at].made).stamp)
    }

    /// Whether a maker reading `columns` is told the rows each make of them made anew.
    fn tells(&self, columns: u64) -> bool {
        self.set(columns).is_some_and(|at| lock(&self.views[at].made).told)
    }

    /// The rows of `keys` alone, made from the mirror: a table read by key ([`Columns::keyed_made`]).
    /// Withheld as not declared while its mirror is not home.
    fn keyed(&self, table: usize, mirror: Option<&Mirror>, keys: &BTreeSet<u64>, order: &mut Vec<(u128, u32)>, made: &mut Made) -> SharedView {
        let columns = over(u64::MAX, self.kinds.len());
        let Some(at) = self.set(columns) else { return not_declared(TableId::ALL[table]) };
        let Some(mirror) = mirror else {
            viewer_foundation::view_record::add("map views", "sets_withheld", 1);
            return not_declared(TableId::ALL[table]);
        };
        let mut held = lock(&self.views[at].made);
        match catch_unwind(AssertUnwindSafe(|| held.keyed_made(table, columns, (mirror, &self.kinds, self.gap || mirror.in_gap()), keys, order, made))) {
            Ok(view) => view,
            Err(payload) => held.failed_make(table, &*payload),
        }
    }

    /// Whether a maker reading `columns` can be answered at `hour`: the side's mirror of the table
    /// is home, holds every column the maker declared, and stands at `hour` (or in a gap). Until
    /// then it is given no frame, never one with the table missing from it, nor one stamped `hour`
    /// holding the rows of an hour before it, as a table stalled for a part not read yet is.
    fn answers(&self, columns: u64, mirror: Option<&Mirror>, hour: Hour) -> bool {
        self.home(columns, mirror) && mirror.is_some_and(|mirror| stands_at(mirror, hour))
    }

    /// [`Self::answers`] at any hour: the mirror is home and holds every column declared.
    fn home(&self, columns: u64, mirror: Option<&Mirror>) -> bool {
        self.set(columns).is_some() && mirror.is_some_and(|mirror| holds_every(mirror, self.columns, self.kinds.len()))
    }

    /// The rows of `columns` as the steps left `mirror`, made again only when a step touched
    /// those columns. A set whose mirror is not home, or does not hold every column the maker
    /// declared, is withheld as not declared: the maker keeps the product it has until the mirror
    /// is home, and never reads a column's zero for a value.
    fn view(&self, table: usize, columns: u64, mirror: Option<&Mirror>, order: &mut Vec<(u128, u32)>, made: &mut Made) -> SharedView {
        let Some(at) = self.set(columns) else { return not_declared(TableId::ALL[table]) };
        let set = &self.views[at];
        let Some(mirror) = mirror.filter(|mirror| holds_every(mirror, set.columns, self.kinds.len())) else {
            viewer_foundation::view_record::add("map views", "sets_withheld", 1);
            return not_declared(TableId::ALL[table]);
        };
        let mut held = lock(&set.made);
        let view = match &held.built {
            Some((stamp, view)) if *stamp == held.stamp => view.clone(),
            _ => match catch_unwind(AssertUnwindSafe(|| held.make(table, set.columns, (mirror, &self.kinds, self.gap || mirror.in_gap()), order, made))) {
                Ok(view) => view,
                Err(payload) => held.failed_make(table, &*payload),
            },
        };
        let sampled = if matches!(view.held(), TableView::Complete(_)) && !self.gap && !mirror.in_gap() { mirror.sampled() } else { None };
        view.with_sampled_at(sampled)
    }

    /// The bytes its views hold.
    fn bytes(&self) -> usize {
        self.views.iter().map(|held| lock(&held.made).built.as_ref().map_or(0, |(_, view)| view_bytes(view))).sum()
    }
}

/// Whether `mirror` holds every column `columns` names of a table of `width` columns; a column
/// past the 64th is named by every set, as a view's reads take it. A column the recording's own
/// lane does not have ([`Mirror::width`]) is held by no mirror and read as not recorded, never as
/// a zero, so it is not one the mirror must hold.
/// Whether `mirror` stands at `hour`, or in a gap, where no hour stands: the mirror's own state,
/// which a set declared during the gap sees too.
fn stands_at(mirror: &Mirror, hour: Hour) -> bool {
    mirror.in_gap() || mirror.head() == Some(hour)
}

fn holds_every(mirror: &Mirror, columns: u64, width: usize) -> bool {
    (0..width.min(mirror.width())).filter(|&column| column >= 64 || columns & (1 << column) != 0).all(|column| mirror.holds(column))
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

/// What a maker opened as telling ([`Views::telling`]) is told changed: since its last frame was
/// made, and since the frame it last took.
#[derive(Default)]
struct Telling {
    pending: Told,
    unread: Told,
}

/// One maker's place in the store: the sets it reads, and the frame last made for it.
struct Reading {
    id: u64,
    /// The frames made for it before its newest, newest first with the seek each was made at, kept
    /// for a reader of the display frame's own ([`Reader::frame_now_through`]): the side prepares
    /// its rows ahead of the hour the display shows.
    older: Option<VecDeque<(Arc<DecodedFrame>, u64)>>,
    /// The maker that opened it, for the harness.
    name: &'static str,
    /// When the step that owes it views posted them, and when its frame was made: what the step
    /// waits for before this maker can start, measured.
    posted: Option<Instant>,
    published_at: Option<Instant>,
    /// Whether this step's frame has already been counted against `cost_ns`.
    costed: bool,
    /// What the step waits for after this maker's rows land: its own work, as its last builds
    /// measured it, smoothed. The step's sets are made in this order, dearest first.
    cost_ns: u64,
    /// Steps that owed it a view, the nanoseconds from the post to its frame, the nanoseconds from
    /// its frame to the maker's next wait (its build), the builds counted, and the steps it was
    /// the last maker given its frame.
    marks: Marks,
    /// Its sets read whole, ascending by table index.
    tables: Vec<(usize, u64)>,
    /// Its sets as the maker named them, to tell a declaration that changes nothing and to
    /// declare them again unchanged when it reads a table by key.
    declared: Vec<Need>,
    /// The tables it reads by key ([`Reader::declare_keyed`]), ascending: the rows of its own keys
    /// alone, made from the mirror as its frame is made. Not among `tables`.
    keyed: Vec<Keyed>,
    /// Per table it reads by key: the keys its frame's rows of that table were made for.
    frame_keys: Vec<(usize, Arc<BTreeSet<u64>>)>,
    /// Per table it reads by key: the set's stamp and the declaration's count the view it holds
    /// was made at, the keys, and the view: made again only when one of them moved.
    keyed_views: Vec<(usize, u64, u64, Arc<BTreeSet<u64>>, SharedView)>,
    frame: Option<Arc<DecodedFrame>>,
    /// The hour and the seek its frame stands at.
    stands: Option<(Hour, u64)>,
    /// Frames made for it since it opened.
    published: u64,
    /// The rows of its tables made anew, for a maker that takes them with its frame.
    told: Option<Telling>,
    /// The tables it declared that the side's mirrors do not hold yet: what its next frame waits
    /// for ([`Kept::answers`]).
    arriving: Vec<TableId>,
    open: bool,
    /// How often its sets are made, and the earliest step that may make them again.
    pace: Pace,
    due_at: Option<Instant>,
    /// Whether its reader has finished with the frame it was given ([`Pace::Ready`]).
    ready: bool,
}

/// What one maker's place cost the step, summed over the run.
#[derive(Default, Clone, Copy)]
struct Marks {
    owed: u64,
    wait_ns: u64,
    build_ns: u64,
    builds: u64,
    last: u64,
}

impl Reading {
    /// Whether a step posting now owes it views: every step for a reader of the picture, and for
    /// one on a cadence the first step of each of its ticks, or any step while it holds no frame
    /// of this seek.
    fn due(&self, now: Instant, epoch: u64) -> bool {
        let anew = self.frame.is_none() || self.stands.is_none_or(|(_, at)| at != epoch);
        match (self.pace, self.due_at) {
            (Pace::Ready, _) => self.ready || anew,
            (_, Some(due)) => now >= due || anew,
            (_, None) => true,
        }
    }

    /// Whether it asked for other keys of a table it reads by key since its frame was made: its
    /// next frame holds their rows, however little the step applied — a maker that hands on an
    /// entity while the clock is held is answered at the hour the side stands at.
    fn keys_moved(&self) -> bool {
        self.keyed.iter().any(|keyed| {
            !self
                .keyed_views
                .iter()
                .any(|(table, _, version, keys, _)| *table == keyed.table && *version == keyed.version && Arc::ptr_eq(keys, &keyed.keys))
        })
    }
}

/// One view a step owes: a set of a table's columns, and where the places waiting for it are kept
/// ([`Jobs::waiting`]).
struct Job {
    table: usize,
    columns: u64,
    waits: std::ops::Range<usize>,
}

/// The mirrors a step's views are made from, and what they stand at. Held until the side takes
/// them back before its next apply ([`Store::settled`]), so a maker that asks for the rows of
/// other keys while the side settles is answered from the mirrors its step left.
struct Posted {
    mirrors: Arc<Mirrors>,
    hour: Hour,
    branch: BranchId,
    epoch: u64,
}

/// What a step owes its makers, as the side and the make threads share it.
#[derive(Default)]
struct Jobs {
    /// The views to make, the ones the earliest maker reads first.
    queue: Vec<Job>,
    /// The places waiting, in the ranges the jobs name.
    waiting: Vec<usize>,
    /// The first job in `queue` no thread has taken.
    next: usize,
    /// Jobs being made now.
    running: usize,
    /// Per place: the views it still waits for.
    left: Vec<usize>,
    from: Option<Posted>,
    /// What the jobs made, taken by the side as the step's views end.
    made: Made,
    /// A job that panicked outside the guard a make takes of its own, raised on the side's thread.
    failed: Option<Box<dyn std::any::Any + Send>>,
    /// The store is dropped: every make thread returns.
    stop: bool,
}

impl Jobs {
    /// The job at `at` is made: what it built, and one view fewer for every place waiting for it.
    /// The places that now wait for none, whose frames are made at once. It still counts as
    /// running until its maker let the mirrors go.
    fn built(&mut self, at: usize, made: Made, failed: Option<Box<dyn std::any::Any + Send>>) -> Vec<usize> {
        let waits = self.queue[at].waits.clone();
        self.made.merge(made);
        let mut ready = Vec::new();
        for held in waits {
            let place = self.waiting[held];
            self.left[place] -= 1;
            if self.left[place] == 0 {
                ready.push(place);
            }
        }
        if self.failed.is_none() {
            self.failed = failed;
        }
        ready
    }
}

struct Shared {
    /// Per table index: the sets declared of it and the rows made of each. Written by the side
    /// between steps (the steps noted, a declaration changed) and read while views are made.
    kept: RwLock<Vec<Option<Kept>>>,
    jobs: Mutex<Jobs>,
    /// Announced when a step posts its jobs.
    work: Condvar,
    /// Announced as each job ends.
    done: Condvar,
    readers: Mutex<Vec<Reading>>,
    /// Announced when a maker's frame is made, with `readers` held.
    made: Condvar,
    /// The seek ([`playhead::Playhead::epoch`]) the side last posted its views at.
    epoch: AtomicU64,
    /// The hour the display shows, as [`Views::show_through`] set it; [`NOT_SHOWN`] for none.
    display: std::sync::atomic::AtomicI64,
    next: AtomicU64,
    /// The recording's schema: a table's column kinds by index.
    kinds: Vec<Vec<Kind>>,
    /// Per table index: whether the recording writes it, so a maker that declares one it never
    /// wrote is answered with the table not recorded rather than waiting for it.
    written: Vec<bool>,
    /// Per table the recording never wrote, its one not-recorded view, made when first given.
    unwritten: Vec<std::sync::OnceLock<SharedView>>,
    /// The playhead the side reads: what a maker asks of the recording itself (the tables it holds
    /// nothing of).
    entry: Arc<Entry>,
    /// Each table a maker learned while it read, by maker and table: the hour it first learned it
    /// and how many times it did.
    learned: Mutex<BTreeMap<(&'static str, usize), (Option<Hour>, u64)>>,
    /// What the display frame's own readers asked of the store, in order: applied by the side at
    /// its next post, so the frame never waits for the store's locks ([`Views::reader_later`]).
    asked: Mutex<Vec<Asked>>,
    /// A test's step inside a post, after the readers to publish are chosen.
    #[cfg(test)]
    chosen: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// One thing a frame's reader asked of the store ([`Shared::asked`]).
enum Asked {
    Open { id: u64, name: &'static str, needs: Vec<Need>, pace: Pace },
    Declare { id: u64, needs: Vec<Need> },
    Close { id: u64 },
}

fn reading(shared: &Shared) -> RwLockReadGuard<'_, Vec<Option<Kept>>> {
    shared.kept.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn writing(shared: &Shared) -> RwLockWriteGuard<'_, Vec<Option<Kept>>> {
    shared.kept.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the store's makes cost, for the harness.
#[derive(Default)]
struct Record {
    /// Steps that posted a view.
    steps: u64,
    jobs: u64,
    /// Per step that posted one: milliseconds from the post to the last of its views.
    made_ms: Vec<f64>,
    /// Per table index: makes, nanoseconds and rows made.
    tables: BTreeMap<usize, (u64, u64, u64)>,
    /// Per reason a view was made whole: how many.
    whole: BTreeMap<&'static str, u64>,
    /// Views patched in place, on a copy, and made whole.
    how: (u64, u64, u64),
    /// The threads the views were made on.
    on: BTreeSet<String>,
}

/// The rows the map side's makers build from: one store, on the side, made from its mirrors.
pub struct Store {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    /// The side's own scratch, for a view it makes itself while a step's jobs are outstanding.
    order: Vec<(u128, u32)>,
    record: Record,
}

impl Store {
    pub fn new(entry: &Arc<Entry>) -> Self {
        let shared = Shared {
            kept: RwLock::new((0..TABLE_COUNT).map(|_| None).collect()),
            jobs: Mutex::default(),
            work: Condvar::new(),
            done: Condvar::new(),
            readers: Mutex::default(),
            made: Condvar::new(),
            epoch: AtomicU64::new(0),
            display: std::sync::atomic::AtomicI64::new(NOT_SHOWN),
            next: AtomicU64::new(0),
            kinds: entry.kinds().to_vec(),
            written: (0..TABLE_COUNT).map(|table| super::written(entry, table)).collect(),
            unwritten: (0..TABLE_COUNT).map(|_| std::sync::OnceLock::new()).collect(),
            entry: Arc::clone(entry),
            learned: Mutex::default(),
            asked: Mutex::default(),
            #[cfg(test)]
            chosen: Mutex::default(),
        };
        Self { shared: Arc::new(shared), threads: Vec::new(), order: Vec::new(), record: Record::default() }
    }

    /// A maker's reader of `needs`. The side declares its mirrors again before the next step, so
    /// every column named is held before a view of it is made.
    pub fn reader(&self, name: &'static str, needs: &[Need]) -> Reader {
        open(&self.shared, name, needs, false, Pace::Step, false)
    }

    /// The store as a maker that opens its reader later holds it: a layer is given this once, as
    /// the side is made, and its worker opens its reader when it starts.
    pub fn handle(&self) -> Views {
        Views { shared: Arc::clone(&self.shared) }
    }

    /// A step's change sets noted into every set the makers read, as the mirrors apply them;
    /// `layers` names, per table index, the tables the drawn layers read.
    pub fn noting<'a>(&'a self, layers: &'a [bool]) -> Noting<'a> {
        Noting { kept: writing(&self.shared), layers }
    }

    /// The sets a maker reads whose rows the steps moved, posted to the make threads; the side
    /// does not wait here. A maker whose every set stands at this step is given its frame now.
    /// `whole`: every open reader is owed its rows, its pace aside, for a picture that is whole.
    pub fn post(&mut self, mirrors: &Arc<Mirrors>, hour: Hour, branch: BranchId, epoch: u64, whole: bool) {
        self.settled();
        // The mirrors a maker that asks for other keys is answered from stand before the readers to
        // publish are chosen: one that asks between the two is answered at once, never left out.
        lock(&self.shared.jobs).from = Some(Posted { mirrors: Arc::clone(mirrors), hour, branch, epoch });
        let asked = std::mem::take(&mut *lock(&self.shared.asked));
        for asked in asked {
            match asked {
                Asked::Open { id, name, needs, pace } => {
                    register(&self.shared, id, name, false, pace, true);
                    declare(&self.shared, id, &needs, false);
                }
                Asked::Declare { id, needs } => declare(&self.shared, id, &needs, false),
                Asked::Close { id } => close(&self.shared, id),
            }
        }
        let kept = reading(&self.shared);
        let mut readers = lock(&self.shared.readers);
        // Stored under the readers' lock, so a reader holding it sees the seek and the frames of
        // that seek together.
        self.shared.epoch.store(epoch, Ordering::Release);
        // A reader closed since the step before is dropped here, where no job holds a place: a
        // window part opens and closes one every time a tooltip or a click comes and goes.
        readers.retain(|reading| reading.open);
        // The step before: the maker whose frame was made last is the one it waited for.
        if let Some(at) = (0..readers.len()).filter(|&at| readers[at].posted.is_some() && readers[at].published_at.is_some()).max_by_key(|&at| readers[at].published_at) {
            readers[at].marks.last += 1;
        }
        let now = Instant::now();
        for reading in readers.iter_mut() {
            (reading.posted, reading.published_at, reading.costed) = (None, None, false);
        }
        // The maker the step waits for longest after its rows is given them first: its own work
        // is the step's tail, so its views start while the others' are still being made.
        // A reader that declared nothing at all learns the tables it reads by reading, so it is
        // given the step's frame with every table not declared to read; one whose declared sets
        // are not resolvable yet waits for them.
        let mut places: Vec<usize> =
            (0..readers.len()).filter(|&at| readers[at].open && (!readers[at].tables.is_empty() || readers[at].declared.is_empty()) && (whole || readers[at].due(now, epoch))).collect();
        places.sort_by_key(|&at| (std::cmp::Reverse(readers[at].cost_ns), at));
        // One job a set of a table's columns, the sets the maker the step waits for longest first
        // and, within one maker's, the dearest first, so the maker whose own work is the step's
        // tail has its frame first and its longest make starts first.
        let mut wanted: Vec<(usize, usize, u64, u64)> = Vec::new();
        for (order, &place) in places.iter().enumerate() {
            for &(table, columns) in &readers[place].tables {
                // A table the recording never wrote has nothing to make: its answer is the same
                // not-recorded view at every hour.
                if self.shared.written[table] && kept[table].as_ref().is_none_or(|held| held.stale(columns)) && !wanted.iter().any(|&(_, held, set, _)| (held, set) == (table, columns)) {
                    let cost = kept[table].as_ref().map_or(FIRST_MAKE_NS, |held| held.costing(columns));
                    wanted.push((order, table, columns, cost));
                }
            }
        }
        wanted.sort_by_key(|&(order, _, _, cost)| (order, std::cmp::Reverse(cost)));
        let (mut queue, mut waiting, mut left) = (Vec::with_capacity(wanted.len()), Vec::new(), vec![0_usize; readers.len()]);
        for (_, table, columns, _) in wanted {
            let from = waiting.len();
            for &place in &places {
                if readers[place].tables.contains(&(table, columns)) {
                    waiting.push(place);
                    left[place] += 1;
                }
            }
            queue.push(Job { table, columns, waits: from..waiting.len() });
        }
        let ready: Vec<usize> = places.iter().copied().filter(|&place| left[place] == 0 && (readers[place].stands != Some((hour, epoch)) || readers[place].keys_moved())).collect();
        for &place in &places {
            if left[place] > 0 || ready.contains(&place) {
                readers[place].posted = Some(now);
                readers[place].marks.owed += 1;
            }
        }
        drop(readers);
        #[cfg(test)]
        {
            let chosen = lock(&self.shared.chosen).take();
            if let Some(chosen) = chosen {
                chosen();
            }
        }
        let owed = queue.len();
        {
            let mut jobs = lock(&self.shared.jobs);
            (jobs.queue, jobs.waiting, jobs.left, jobs.next) = (queue, waiting, left, 0);
            jobs.made = Made::since(viewer_foundation::view_record::on().then(Instant::now));
            jobs.from = Some(Posted { mirrors: Arc::clone(mirrors), hour, branch, epoch });
        }
        if !ready.is_empty() {
            publish(&self.shared, &kept, mirrors, &ready, (hour, branch, epoch));
        }
        drop(kept);
        if owed > 0 {
            self.record.steps += 1;
            self.record.jobs += owed as u64;
            self.spawn();
            self.shared.work.notify_all();
        }
    }

    /// Every view the last step posted is made and the mirrors let go, so the side holds them
    /// alone again. The side makes what no make thread has taken rather than sleep on one.
    pub fn settled(&mut self) {
        self.wait_made(true);
    }

    /// Every view posted made and folded, the mirrors they were made from kept: a maker that asks
    /// for other keys meanwhile is answered from them at once ([`Reader::declare_keyed`]). What a
    /// whole settle waits for, which applies no step until the next post lets them go.
    pub fn made(&mut self) {
        self.wait_made(false);
    }

    fn wait_made(&mut self, release: bool) {
        loop {
            if make_next(&self.shared, &mut self.order) {
                continue;
            }
            let mut jobs = lock(&self.shared.jobs);
            if jobs.running > 0 {
                let _ = self.shared.done.wait_timeout(jobs, Duration::from_micros(100)).unwrap_or_else(std::sync::PoisonError::into_inner);
                continue;
            }
            let (made, failed) = (std::mem::take(&mut jobs.made), jobs.failed.take());
            if release {
                jobs.from = None;
            }
            jobs.next = 0;
            jobs.queue.clear();
            jobs.waiting.clear();
            jobs.left.clear();
            drop(jobs);
            self.fold(made);
            if let Some(payload) = failed {
                std::panic::resume_unwind(payload);
            }
            return;
        }
    }

    /// The make threads, started as a step first owes a view.
    fn spawn(&mut self) {
        if !self.threads.is_empty() {
            return;
        }
        for _ in 0..MAKE_THREADS {
            let shared = Arc::clone(&self.shared);
            if let Ok(thread) = std::thread::Builder::new().name(MAKE_THREAD.to_owned()).spawn(move || make_views(&shared)) {
                self.threads.push(thread);
            }
        }
    }

    /// What a step's views cost, kept for the harness.
    fn fold(&mut self, made: Made) {
        if made.builds.is_empty() {
            return;
        }
        let mut ended = 0_u64;
        for &(table, ns, rows, how, at) in &made.builds {
            let held = self.record.tables.entry(table).or_default();
            (held.0, held.1, held.2) = (held.0 + 1, held.1 + ns, held.2 + rows as u64);
            ended = ended.max(at + ns);
            match how {
                Make::InPlace => self.record.how.0 += 1,
                Make::Copied => self.record.how.1 += 1,
                Make::Whole(why) => {
                    self.record.how.2 += 1;
                    *self.record.whole.entry(why.name()).or_default() += 1;
                }
                Make::Keyed => {}
            }
        }
        if made.origin.is_some() {
            self.record.made_ms.push(ended as f64 / 1e6);
        }
        if let Some(on) = made.on {
            self.record.on.insert(on);
        }
    }

    /// What its views hold, in bytes, by table.
    pub fn held(&self) -> Vec<(TableId, usize)> {
        reading(&self.shared).iter().enumerate().filter_map(|(table, held)| held.as_ref().map(|held| (TableId::ALL[table], held.bytes()))).collect()
    }

    /// What the store made over the run: per step the milliseconds from its post to the last of
    /// its views, the threads they were made on, and per table the makes, their milliseconds and
    /// the rows made.
    pub fn record(&self, distribution: fn(Vec<f64>) -> serde_json::Value) -> serde_json::Value {
        use serde_json::json;
        let readers: Vec<serde_json::Value> = lock(&self.shared.readers)
            .iter()
            .map(|reading| {
                json!({
                    "maker": reading.name,
                    "tables": reading.tables.len(),
                    "owed": reading.marks.owed,
                    "wait_ms": reading.marks.wait_ns as f64 / 1e6 / reading.marks.owed.max(1) as f64,
                    "build_ms": reading.marks.build_ns as f64 / 1e6 / reading.marks.builds.max(1) as f64,
                    "builds": reading.marks.builds,
                    "last": reading.marks.last,
                })
            })
            .collect();
        let learned: Vec<serde_json::Value> = lock(&self.shared.learned)
            .iter()
            .map(|(&(name, table), &(hour, times))| json!({"maker": name, "table": format!("{:?}", TableId::ALL[table]), "hour": hour, "times": times}))
            .collect();
        let mut tables: Vec<(usize, (u64, u64, u64))> = self.record.tables.iter().map(|(&table, &held)| (table, held)).collect();
        tables.sort_by_key(|&(_, (_, ns, _))| std::cmp::Reverse(ns));
        json!({
            "steps": self.record.steps,
            "jobs": self.record.jobs,
            "views_made_ms": distribution(self.record.made_ms.clone()),
            "made_on": self.record.on.iter().cloned().collect::<Vec<String>>(),
            "readers": readers,
            "learned": learned,
            "in_place": self.record.how.0, "copied": self.record.how.1, "whole": self.record.how.2,
            "whole_why": self.record.whole.iter().map(|(&why, &count)| (why.to_owned(), json!(count))).collect::<serde_json::Map<String, serde_json::Value>>(),
            "tables": tables.iter().map(|&(table, (makes, ns, rows))| json!({"table": format!("{:?}", TableId::ALL[table]), "makes": makes, "make_ms": ns as f64 / 1e6, "rows": rows})).collect::<Vec<serde_json::Value>>(),
            "scope": "The views the map side made from its own mirrors inside its step: steps that owed one, jobs posted, views_made_ms from a step's post to the last of its views, and per table its makes, their milliseconds and the rows made.",
        })
    }
}

/// The side's view store as a maker holds it: it opens its reader when its worker starts, and the
/// store outlives it.
#[derive(Clone)]
pub struct Views {
    shared: Arc<Shared>,
}

impl Views {
    /// The hour the display shows, which the side prepares its rows ahead of: every window reader
    /// ([`AtHead`]) answers from no later than it. None (a saved frame, whose head the side's rows
    /// stand at, or no head yet): the newest.
    pub fn show_through(&self, hour: Option<Hour>) {
        self.shared.display.store(hour.map_or(NOT_SHOWN, i64::from), Ordering::Release);
    }

    /// The hour [`Self::show_through`] last set; none for the newest.
    pub fn shown_through(&self) -> Option<Hour> {
        let hour = self.shared.display.load(Ordering::Acquire);
        (hour != NOT_SHOWN).then_some(hour as Hour)
    }

    /// The seek ([`playhead::Playhead::epoch`]) the side last posted its views at: what was read at
    /// an older one is another history's.
    pub fn seek(&self) -> u64 {
        self.shared.epoch.load(Ordering::Acquire)
    }

    /// The playhead the side reads.
    pub fn entry(&self) -> &Arc<Entry> {
        &self.shared.entry
    }

    /// A maker's reader of `needs` ([`Store::reader`]).
    pub fn reader(&self, name: &'static str, needs: &[Need]) -> Reader {
        open(&self.shared, name, needs, false, Pace::Step, false)
    }

    /// A reader of `needs` whose sets are made at most as often as `pace` asks: what feeds no
    /// picture of the map (a tooltip, a panel, a dump, a proof's line) never costs a step that
    /// does not owe it a view.
    pub fn reader_at(&self, name: &'static str, needs: &[Need], pace: Pace) -> Reader {
        open(&self.shared, name, needs, false, pace, false)
    }

    /// [`Self::reader_at`] keeping the frames before its newest, for a window part that answers at
    /// the hour the display shows ([`Reader::frame_through`]) or at a click's own hour
    /// ([`Reader::frame_of`]).
    pub fn reader_through(&self, name: &'static str, needs: &[Need], pace: Pace) -> Reader {
        open(&self.shared, name, needs, false, pace, true)
    }

    /// [`Self::reader_at`] for the display frame: opened, declared and closed by the side at its next
    /// post, so the frame takes nothing of the store's but a try of its readers ([`Reader::frame_now`]).
    pub fn reader_later(&self, name: &'static str, needs: &[Need], pace: Pace) -> Reader {
        let id = self.shared.next.fetch_add(1, Ordering::AcqRel);
        lock(&self.shared.asked).push(Asked::Open { id, name, needs: needs.to_vec(), pace });
        Reader { shared: Arc::clone(&self.shared), id, later: true }
    }

    /// A maker's reader of `needs` that takes, with each frame, the rows of them made anew since
    /// the frame it took before ([`Reader::take_keyed`]): what changed is told, never found by
    /// comparing tables.
    pub fn telling(&self, name: &'static str, needs: &[Need]) -> Reader {
        open(&self.shared, name, needs, true, Pace::Step, false)
    }

    /// Whether `reader` was opened on this store.
    pub fn holds(&self, reader: &Reader) -> bool {
        Arc::ptr_eq(&self.shared, &reader.shared)
    }


    /// What its views hold, in bytes, by table ([`Store::held`]).
    pub fn held(&self) -> Vec<(TableId, usize)> {
        reading(&self.shared).iter().enumerate().filter_map(|(table, held)| held.as_ref().map(|held| (TableId::ALL[table], held.bytes()))).collect()
    }
}

/// A reader of `needs` opened on `shared` at `pace`, told the rows each make made anew when
/// `told`, keeping the frames before its newest when `keeps_older`.
fn open(shared: &Arc<Shared>, name: &'static str, needs: &[Need], told: bool, pace: Pace, keeps_older: bool) -> Reader {
    let id = shared.next.fetch_add(1, Ordering::AcqRel);
    register(shared, id, name, told, pace, keeps_older);
    let reader = Reader { shared: Arc::clone(shared), id, later: false };
    reader.declared(needs);
    reader
}

/// Reader `id`'s place in the store, nothing declared yet.
fn register(shared: &Shared, id: u64, name: &'static str, told: bool, pace: Pace, keeps_older: bool) {
    let mut readers = lock(&shared.readers);
    readers.push(Reading {
        id,
        older: keeps_older.then(VecDeque::new),
        name,
        posted: None,
        published_at: None,
        costed: false,
        cost_ns: 0,
        marks: Marks::default(),
        tables: Vec::new(),
        declared: Vec::new(),
        keyed: Vec::new(),
        frame_keys: Vec::new(),
        keyed_views: Vec::new(),
        frame: None,
        stands: None,
        published: 0,
        told: told.then(Telling::default),
        arriving: Vec::new(),
        open: true,
        pace,
        due_at: None,
        ready: true,
    });
}

impl Drop for Store {
    fn drop(&mut self) {
        lock(&self.shared.jobs).stop = true;
        self.shared.work.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// A step's change sets noted into every set the makers read, while the side applies them to its
/// mirrors: the tables are held for the step, so no view is being made meanwhile.
pub struct Noting<'a> {
    kept: RwLockWriteGuard<'a, Vec<Option<Kept>>>,
    layers: &'a [bool],
}

impl super::mirror::Notes for Noting<'_> {
    fn awake(&self, table: usize) -> bool {
        self.layers.get(table).copied().unwrap_or(false) || self.kept.get(table).is_some_and(Option::is_some)
    }

    fn gap(&mut self) {
        noted_gap(&mut self.kept);
    }

    fn applied(&mut self, table: usize, changes: &Changes, moved: bool) {
        noted_apply(&mut self.kept, table, changes, moved);
    }
}

/// A gap step noted into every set of `kept`: every table holds nothing until the next step that
/// names it.
fn noted_gap(kept: &mut [Option<Kept>]) {
    for kept in kept.iter_mut().flatten() {
        if kept.gap {
            continue;
        }
        kept.gap = true;
        for held in &mut kept.views {
            let held = mine(&mut held.made);
            held.stamp += 1;
            held.touched = Err(Whole::Gap);
        }
    }
}

/// `changes` noted into every set of `kept`'s `table` holding a column it wrote.
fn noted_apply(kept: &mut [Option<Kept>], table: usize, changes: &Changes, moved: bool) {
    {
        let Some(Some(kept)) = kept.get_mut(table) else { return };
        kept.gap = false;
        // Every view holds every live row, so a row born or gone or a status that moved changes
        // them all; a cell written changes those holding its column alone.
        let membership = changes.reset() || !changes.deleted().is_empty() || !changes.inserted().is_empty() || moved;
        let written = changes.columns().iter().filter(|written| !written.is_empty()).fold(0_u64, |held, written| held | if written.column() < 64 { 1 << written.column() } else { view::EVERY });
        for held in &mut kept.views {
            let columns = held.columns;
            let held = mine(&mut held.made);
            let rows = membership || written & columns != 0;
            if rows {
                held.stamp += 1;
            }
            if rows || changes.renumber().is_some() {
                note(&mut held.touched, changes, moved, columns, held.bound);
            }
        }
    }
}

/// Rows of a few tables that their owner steps through, in order, to the hours it asks for: one
/// reader of the playhead of its own, so the rows at the end of any step are there however many
/// steps the map took in one frame (the map text's events' context). A table the recording holds
/// nothing of is not declared ([`Self::absent`]): its owner refuses it by name.
pub struct Stepped {
    entry: Arc<Entry>,
    reader: playhead::Reader,
    consumer: playhead::Consumer,
    mirrors: Mirrors,
    kept: Vec<Option<Kept>>,
    tables: Vec<usize>,
    absent: Vec<TableId>,
    /// The last hour of the last step applied, and its seek.
    hour: Option<(Hour, u64)>,
    order: Vec<(u128, u32)>,
    /// Taken and not applied: a newer seek's, until a request of that seek.
    steps: Vec<Arc<playhead::Step>>,
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
            declared.push(playhead::Declared { table: table.index(), columns: (0..columns.len()).collect(), keyed: true, passing: false, hourly: false });
            let mut set = Kept::new(columns.clone());
            set.at(u64::MAX);
            kept[table.index()] = Some(set);
        }
        let playhead = entry.playhead();
        let reader = playhead.declare(declared, Some(super::mirror::rebuild(Arc::clone(&entry.pages))))?;
        let consumer = playhead.consumer(reader);
        Ok(Self {
            entry: Arc::clone(entry),
            reader,
            consumer,
            mirrors: Mirrors::default(),
            kept,
            tables: held.iter().map(|table| table.index()).collect(),
            absent,
            hour: None,
            order: Vec::new(),
            steps: Vec::new(),
        })
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
        let mut tables = Vec::new();
        let steps: Vec<Arc<playhead::Step>> = self.steps.drain(..ready).collect();
        for step in &steps {
            self.hour = Some((step.last(), epoch));
            for changes in step.tables() {
                if changes.reset()
                    && let Some(state) = self.consumer.state(step, changes.table())
                    && let Ok(mut rebuilt) = state.downcast::<Mirror>()
                {
                    rebuilt.set_capture(capture);
                    self.mirrors.insert(rebuilt);
                }
            }
            let mut notes = SteppedNotes { kept: &mut self.kept };
            self.mirrors.apply_timed(step, &mut tables, &mut Default::default(), &mut notes);
        }
        self.hour.filter(|&(_, of)| of == epoch).map(|(hour, _)| hour)
    }

    /// The rows of its tables as the steps taken left them, stamped `branch` and `hour`; none
    /// while a table has not arrived or stands at another hour.
    pub fn frame(&mut self, branch: BranchId, hour: Hour) -> Option<DecodedFrame> {
        self.frame_with(branch, hour, false)
    }

    /// [`Self::frame`] once the steps have run past `hour`: a table home but standing at another
    /// hour is refused by name rather than waited for, as its rows of `hour` will not come.
    pub fn frame_past(&mut self, branch: BranchId, hour: Hour) -> Option<DecodedFrame> {
        self.frame_with(branch, hour, true)
    }

    fn frame_with(&mut self, branch: BranchId, hour: Hour, past: bool) -> Option<DecodedFrame> {
        let mirror = |table: usize| self.mirrors.get(TableId::ALL[table]);
        let answers = |table: usize| self.kept[table].as_ref().is_some_and(|kept| if past { kept.home(u64::MAX, mirror(table)) } else { kept.answers(u64::MAX, mirror(table), hour) });
        if !self.tables.iter().all(|&table| answers(table)) {
            return None;
        }
        let mut frame = DecodedFrame { branch, hour, tables: std::array::from_fn(|index| not_declared(TableId::ALL[index])), metrics: Metrics::default() };
        let mut made = Made::default();
        for &table in &self.tables {
            let kept = self.kept[table].as_ref().expect("held");
            let mirror = self.mirrors.get(TableId::ALL[table]);
            frame.tables[table] = match mirror.filter(|mirror| !stands_at(mirror, hour)) {
                Some(standing) => SharedView::from(TableView::Unavailable { reason: format!("{:?} was not read at hour {hour}: the steps reached it only at hour {:?}", TableId::ALL[table], standing.head()) }),
                None => kept.view(table, u64::MAX, mirror, &mut self.order, &mut made),
            };
        }
        Some(frame)
    }
}

impl Drop for Stepped {
    fn drop(&mut self) {
        self.entry.playhead().retire(self.reader);
    }
}

/// A step noted into the tables one [`Stepped`] holds: it owns them, so no lock is taken.
struct SteppedNotes<'a> {
    kept: &'a mut Vec<Option<Kept>>,
}

impl super::mirror::Notes for SteppedNotes<'_> {
    fn awake(&self, table: usize) -> bool {
        self.kept.get(table).is_some_and(Option::is_some)
    }

    fn gap(&mut self) {
        noted_gap(self.kept);
    }

    fn applied(&mut self, table: usize, changes: &Changes, moved: bool) {
        noted_apply(self.kept, table, changes, moved);
    }
}

/// Makes the next view no thread has taken, with `order` as its scratch; false when the step owes
/// none. The frames of the makers the job answers are made as it ends, so a maker is given its
/// rows while the views another reads are still being made.
fn make_next(shared: &Shared, order: &mut Vec<(u128, u32)>) -> bool {
    let kept = reading(shared);
    let (at, table, columns, mirrors, stands, origin) = {
        let mut jobs = lock(&shared.jobs);
        if jobs.next == jobs.queue.len() {
            return false;
        }
        let Some(from) = &jobs.from else { return false };
        let (mirrors, stands, origin) = (Arc::clone(&from.mirrors), (from.hour, from.branch, from.epoch), jobs.made.origin);
        let at = jobs.next;
        (jobs.next, jobs.running) = (at + 1, jobs.running + 1);
        (at, jobs.queue[at].table, jobs.queue[at].columns, mirrors, stands, origin)
    };
    let mut made = Made::since(origin);
    let failed = catch_unwind(AssertUnwindSafe(|| {
        if let Some(held) = kept[table].as_ref() {
            held.view(table, columns, mirrors.get(TableId::ALL[table]), order, &mut made);
        }
    }))
    .err();
    // Every maker told of this set is told what the make made anew before any frame holding it is
    // made for one of them.
    telling(shared, &kept, &mut made);
    let ready = lock(&shared.jobs).built(at, made, failed);
    if !ready.is_empty() {
        publish(shared, &kept, &mirrors, &ready, stands);
    }
    drop(kept);
    // The mirrors are let go before the job counts as done, so the side holds them alone the
    // moment its wait for them ends ([`Store::settled`]).
    drop(mirrors);
    lock(&shared.jobs).running -= 1;
    shared.done.notify_all();
    true
}

/// One make thread: every view the steps owe that no other thread took, until the store is
/// dropped.
fn make_views(shared: &Shared) {
    let mut order: Vec<(u128, u32)> = Vec::new();
    loop {
        while make_next(shared, &mut order) {}
        let mut jobs = lock(&shared.jobs);
        while !jobs.stop && jobs.next == jobs.queue.len() {
            jobs = shared.work.wait(jobs).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if jobs.stop {
            return;
        }
    }
}

/// The frame each place of `places` reads, made from the views standing now: its declared tables'
/// rows, the rows of its own keys for a table it reads by key, every other table not declared, at
/// the hour the side applied.
fn publish(shared: &Shared, kept: &[Option<Kept>], mirrors: &Mirrors, places: &[usize], (hour, branch, epoch): (Hour, BranchId, u64)) {
    let (mut order, mut made) = (Vec::new(), Made::default());
    let mut readers = lock(&shared.readers);
    for &place in places {
        let Some(reading) = readers.get(place) else { continue };
        // A maker whose every table is not home yet is given no frame: it keeps the product it
        // has, and never builds from a frame a table it declared is missing from.
        let home = |&(table, columns): &(usize, u64)| !shared.written[table] || kept[table].as_ref().is_some_and(|kept| kept.answers(columns, mirrors.get(TableId::ALL[table]), hour));
        let waiting: Vec<TableId> = reading
            .tables
            .iter()
            .copied()
            .chain(reading.keyed.iter().map(|keyed| (keyed.table, u64::MAX)))
            .filter(|set| !home(set))
            .map(|(table, _)| TableId::ALL[table])
            .collect();
        if !waiting.is_empty() {
            viewer_foundation::view_record::add("map views", "frames_withheld", 1);
            readers[place].arriving = waiting;
            continue;
        }
        let mut held: Vec<Option<SharedView>> = vec![None; TABLE_COUNT];
        let unwritten = |table: usize| -> SharedView {
            shared.unwritten[table]
                .get_or_init(|| {
                    match shared.entry.capture.get() {
                        Some(&mode) => TableView::NotRecorded { mode },
                        None => TableView::Unavailable { reason: format!("{:?} is not recorded here, in a recording whose capture mode the playhead was not given", TableId::ALL[table]) },
                    }
                    .into()
                })
                .clone()
        };
        for &(table, columns) in &reading.tables {
            if !shared.written[table] {
                held[table] = Some(unwritten(table));
            } else if let Some(kept) = kept[table].as_ref() {
                held[table] = Some(kept.view(table, columns, mirrors.get(TableId::ALL[table]), &mut order, &mut made));
            }
        }
        // The rows of its own keys: made again only when a step touched the table or it asked for
        // other keys, and the rows of keys it did not hold before are new to it.
        let (id, keyed) = (reading.id, reading.keyed.clone());
        let before = std::mem::take(&mut readers[place].keyed_views);
        let (mut views, mut keys) = (Vec::with_capacity(keyed.len()), Vec::with_capacity(keyed.len()));
        for keyed in &keyed {
            if !shared.written[keyed.table] {
                held[keyed.table] = Some(unwritten(keyed.table));
                continue;
            }
            let Some(kept) = kept[keyed.table].as_ref() else { continue };
            let stamp = kept.stamp(u64::MAX);
            let was = before.iter().find(|(table, ..)| *table == keyed.table);
            let view = match was {
                Some((_, shown, version, _, view)) if *shown == stamp && *version == keyed.version => view.clone(),
                _ => {
                    let tells = kept.tells(u64::MAX);
                    let view = kept.keyed(keyed.table, mirrors.get(TableId::ALL[keyed.table]), &keyed.keys, &mut order, &mut made);
                    let asked: Vec<(u128, u64)> = order.iter().map(|&(key, _)| key).filter(|&key| was.is_none_or(|(.., keys, _)| !keys.contains(&prefix(key)))).map(|key| (key, view::EVERY)).collect();
                    if tells && !asked.is_empty() {
                        made.told_reader.push((id, keyed.table, asked));
                    }
                    view
                }
            };
            held[keyed.table] = Some(view.clone());
            keys.push((keyed.table, Arc::clone(&keyed.keys)));
            views.push((keyed.table, stamp, keyed.version, Arc::clone(&keyed.keys), view));
        }
        let frame = DecodedFrame { branch, hour, tables: std::array::from_fn(|table| held[table].take().unwrap_or_else(|| not_declared(TableId::ALL[table]))), metrics: Metrics::default() };
        // Every maker told of these sets is told what was made anew before it holds the frame.
        told(&mut readers, kept, &mut made);
        let reading = &mut readers[place];
        if let (Some(older), Some(was), Some((_, seek))) = (reading.older.as_mut(), reading.frame.take(), reading.stands) {
            older.push_front((was, seek));
            older.truncate(FRAMES_KEPT);
        }
        reading.frame = Some(Arc::new(frame));
        reading.stands = Some((hour, epoch));
        reading.published += 1;
        reading.due_at = reading.pace.interval().map(|interval| Instant::now() + interval);
        reading.ready = false;
        if reading.published_at.is_none() {
            let made_at = Instant::now();
            reading.published_at = Some(made_at);
            if let Some(posted) = reading.posted {
                reading.marks.wait_ns += made_at.saturating_duration_since(posted).as_nanos() as u64;
            }
        }
        (reading.frame_keys, reading.keyed_views) = (keys, views);
        reading.arriving.clear();
        if let Some(telling) = &mut reading.told {
            for (table, rows) in std::mem::take(&mut telling.pending) {
                tell(&mut telling.unread, table, rows.as_ref());
            }
        }
    }
    drop(readers);
    shared.made.notify_all();
}

/// The frame of the maker `id`, made again now from the mirrors the side's last step left: what a
/// maker that asked for the rows of other keys is answered with while the side settles. It counts
/// as a job running, so the side holds the mirrors alone again before it applies ([`Store::settled`]).
/// Nothing while the side holds them (between a settle and the next step's post).
fn republish(shared: &Shared, id: u64) {
    let Some((mirrors, stands)) = ({
        let mut jobs = lock(&shared.jobs);
        let held = jobs.from.as_ref().map(|from| (Arc::clone(&from.mirrors), (from.hour, from.branch, from.epoch)));
        if held.is_some() {
            jobs.running += 1;
        }
        held
    }) else {
        return;
    };
    let kept = reading(shared);
    let place = Reader::place(&lock(&shared.readers), id);
    if let Some(place) = place {
        publish(shared, &kept, &mirrors, &[place], stands);
    }
    drop(kept);
    drop(mirrors);
    lock(&shared.jobs).running -= 1;
    shared.done.notify_all();
}

/// What `made` made anew since the last call, told to every maker that is told of those sets
/// ([`Views::telling`]), and the rows of one maker's newly asked keys to it alone.
fn telling(shared: &Shared, kept: &[Option<Kept>], made: &mut Made) {
    if made.told.len() == made.told_from && made.told_reader.len() == made.told_reader_from {
        return;
    }
    let mut readers = lock(&shared.readers);
    told(&mut readers, kept, made);
}

/// [`telling`] with the readers held.
fn told(readers: &mut [Reading], kept: &[Option<Kept>], made: &mut Made) {
    for &(table, columns, ref rows) in &made.told[made.told_from..] {
        let whole = kept[table].as_ref().is_some_and(|kept| over(columns, kept.kinds.len()) == over(u64::MAX, kept.kinds.len()));
        for reading in readers.iter_mut().filter(|reading| reading.told.is_some()) {
            let reads = reading.tables.contains(&(table, columns)) || (whole && reading.keyed.iter().any(|keyed| keyed.table == table));
            if let (true, Some(telling)) = (reads, &mut reading.told) {
                tell(&mut telling.pending, TableId::ALL[table], rows.as_ref().map(|rows| rows.iter().map(|(key, columns)| (key, columns))));
            }
        }
    }
    made.told_from = made.told.len();
    for &(id, table, ref rows) in &made.told_reader[made.told_reader_from..] {
        if let Some(telling) = readers.iter_mut().find(|reading| reading.id == id).and_then(|reading| reading.told.as_mut()) {
            tell(&mut telling.pending, TableId::ALL[table], Some(rows.iter().map(|(key, columns)| (key, columns))));
        }
    }
    made.told_reader_from = made.told_reader.len();
}

/// The hour a product made from a frame of rows stands for: the frame's own, then each later hour
/// its reader says no step changed those rows by ([`Reader::through`]), kept once the frame is
/// replaced. Held weakly: the frame's rows are not kept for it.
#[derive(Clone, Debug)]
pub struct Stands {
    frame: std::sync::Weak<DecodedFrame>,
    hour: Hour,
}

impl Stands {
    /// Made from `frame`, at `hour`: its own unless its maker found it current at later rows.
    pub fn at(frame: std::sync::Weak<DecodedFrame>, hour: Hour) -> Self {
        Self { frame, hour }
    }

    /// Asks `through` (a reader's [`Reader::through`]) again; the hour it stands for.
    pub fn look(&mut self, through: impl FnOnce(*const DecodedFrame) -> Option<Hour>) -> Hour {
        if let Some(hour) = through(self.frame.as_ptr()) {
            self.hour = self.hour.max(hour);
        }
        self.hour
    }

    pub fn hour(&self) -> Hour {
        self.hour
    }
}

/// One maker's handle on the store: the rows of its tables at the hour the side applied.
pub struct Reader {
    shared: Arc<Shared>,
    id: u64,
    /// The display frame's: what it asks of the store is applied by the side ([`Shared::asked`]).
    later: bool,
}

impl Reader {
    /// Its place in the store; none once it is closed.
    fn place(readers: &[Reading], id: u64) -> Option<usize> {
        readers.iter().position(|held| held.id == id)
    }

    /// The newest frame made for it.
    pub fn frame(&self) -> Option<Arc<DecodedFrame>> {
        let readers = lock(&self.shared.readers);
        readers.get(Self::place(&readers, self.id)?)?.frame.clone()
    }

    /// [`Self::frame_now`] at the newest frame whose hour is no later than `hour`: a frame of the
    /// display's own never shows rows of an hour the display has not reached, nor of a seek before
    /// the side's last post.
    pub fn frame_now_through(&self, hour: Hour) -> Option<Arc<DecodedFrame>> {
        let readers = match self.shared.readers.try_lock() {
            Ok(readers) => readers,
            Err(std::sync::TryLockError::Poisoned(held)) => held.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        self.through_in(&readers, hour)
    }

    /// [`Self::frame_now_through`], waiting for the store's readers: a click answered once.
    pub fn frame_through(&self, hour: Hour) -> Option<Arc<DecodedFrame>> {
        self.through_in(&lock(&self.shared.readers), hour)
    }

    fn through_in(&self, readers: &[Reading], hour: Hour) -> Option<Arc<DecodedFrame>> {
        let reading = readers.get(Self::place(readers, self.id)?)?;
        let seek = self.seek();
        let newest = reading.frame.iter().filter(|_| reading.stands.is_some_and(|(_, at)| at == seek));
        newest.chain(reading.older.iter().flatten().filter(|&&(_, at)| at == seek).map(|(frame, _)| frame)).find(|frame| frame.hour <= hour).cloned()
    }

    /// Its frame of `hour` and of the side's last posted seek, the newest or one of the few before
    /// it a reader that keeps them holds; none while no such frame is made for it.
    pub fn frame_of(&self, hour: Hour) -> Option<Arc<DecodedFrame>> {
        let readers = lock(&self.shared.readers);
        let reading = readers.get(Self::place(&readers, self.id)?)?;
        let seek = self.seek();
        let newest = reading.frame.iter().filter(|_| reading.stands.is_some_and(|(_, at)| at == seek));
        newest.chain(reading.older.iter().flatten().filter(|&&(_, at)| at == seek).map(|(frame, _)| frame)).find(|frame| frame.hour == hour).cloned()
    }

    /// [`Self::frame`] without waiting for the store: none while the store is making the makers'
    /// frames, which it does holding its readers and which at a piece's crossing is hundreds of
    /// thousands of cells' worth of work. A reader of the display frame's own takes this, so the
    /// frame keeps what it was last given rather than waiting for the simulation's side.
    pub fn frame_now(&self) -> Option<Arc<DecodedFrame>> {
        let readers = match self.shared.readers.try_lock() {
            Ok(readers) => readers,
            Err(std::sync::TryLockError::Poisoned(held)) => held.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                viewer_foundation::view_record::add("map views", "frames_the_frame_did_not_wait_for", 1);
                return None;
            }
        };
        // A frame of a seek before the side's last post is another history's.
        let reading = readers.get(Self::place(&readers, self.id)?)?;
        reading.frame.clone().filter(|_| reading.stands.is_some_and(|(_, at)| at == self.seek()))
    }

    /// The newest frame made for it with the count of frames made, taken together: a maker that
    /// has built this one waits for a newer ([`Self::wait_newer`]).
    pub fn newest(&self) -> Option<(Arc<DecodedFrame>, u64)> {
        let readers = lock(&self.shared.readers);
        let reading = readers.get(Self::place(&readers, self.id)?)?;
        Some((reading.frame.clone()?, reading.published))
    }

    /// [`Self::newest`] with the seek its frame was made at: a product of an older one is not
    /// this picture's.
    pub fn newest_seek(&self) -> Option<(Arc<DecodedFrame>, u64, u64)> {
        let readers = lock(&self.shared.readers);
        let reading = readers.get(Self::place(&readers, self.id)?)?;
        Some((reading.frame.clone()?, reading.published, reading.stands?.1))
    }

    /// The seek ([`playhead::Playhead::epoch`]) the side last posted its views at: a frame of an
    /// older one is not this picture's, as the side has sought since it was made.
    pub fn seek(&self) -> u64 {
        self.shared.epoch.load(Ordering::Acquire)
    }

    /// The hour `frame` stands at; none for a frame it no longer holds. Every view of it was
    /// made from the mirrors as the side's step left them, so that is the hour the side applied.
    pub fn through(&self, frame: *const DecodedFrame) -> Option<Hour> {
        self.through_seek(frame).map(|(hour, _)| hour)
    }

    /// The hour and the seek `frame` stands at; none for a frame it no longer holds.
    pub fn through_seek(&self, frame: *const DecodedFrame) -> Option<(Hour, u64)> {
        let readers = lock(&self.shared.readers);
        let reading = readers.get(Self::place(&readers, self.id)?)?;
        reading.frame.as_ref().filter(|held| std::ptr::eq(Arc::as_ptr(held), frame)).and_then(|_| reading.stands)
    }

    /// The tables it declared that the side's mirrors do not hold yet: what its next frame waits
    /// for. Empty once a frame holding them all is made for it.
    pub fn arriving(&self) -> Vec<TableId> {
        let readers = lock(&self.shared.readers);
        Self::place(&readers, self.id).map_or_else(Vec::new, |at| readers[at].arriving.clone())
    }

    /// Frames made for it since it opened: a maker that built the last of them waits for a newer.
    /// The sets of a table's columns this reader is given, as the store holds them: zero while the
    /// store knows no column kinds of the tables it needs, when it is given nothing at all.
    pub fn reads(&self) -> usize {
        let readers = lock(&self.shared.readers);
        Self::place(&readers, self.id).map_or(0, |at| readers[at].tables.len())
    }

    pub fn published(&self) -> u64 {
        let readers = lock(&self.shared.readers);
        Self::place(&readers, self.id).map_or(0, |at| readers[at].published)
    }

    /// Waits up to `limit` for a frame newer than `seen`; how many have been made now.
    pub fn wait_newer(&self, seen: u64, limit: Duration) -> u64 {
        let now = Instant::now();
        let until = now + limit;
        let mut readers = lock(&self.shared.readers);
        if let Some(at) = Self::place(&readers, self.id) {
            readers[at].ready = true;
        }
        if let Some(at) = Self::place(&readers, self.id)
            && let Some(made) = readers[at].published_at.filter(|_| readers[at].published == seen && !readers[at].costed)
        {
            let reading = &mut readers[at];
            let took = now.saturating_duration_since(made).as_nanos() as u64;
            reading.cost_ns = if reading.cost_ns == 0 { took } else { (reading.cost_ns * 3 + took) / 4 };
            reading.marks.build_ns += took;
            reading.marks.builds += 1;
            reading.costed = true;
        }
        loop {
            let published = Self::place(&readers, self.id).map_or(0, |at| readers[at].published);
            let now = Instant::now();
            if published > seen || now >= until {
                return published;
            }
            readers = self.shared.made.wait_timeout(readers, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Its reader has finished with the frame it was given: the step after this makes its sets
    /// again ([`Pace::Ready`]). A reader of another pace says nothing.
    pub fn ready(&self) {
        let mut readers = lock(&self.shared.readers);
        if let Some(at) = Self::place(&readers, self.id) {
            readers[at].ready = true;
        }
    }


    /// Its newest frame that satisfies `ready`, waited for within `limit`; refused by name when
    /// none came, with the tables its frame is still waiting for.
    pub fn wait(&self, mut ready: impl FnMut(&DecodedFrame) -> bool, limit: Duration) -> io::Result<Arc<DecodedFrame>> {
        let until = Instant::now() + limit;
        let mut readers = lock(&self.shared.readers);
        loop {
            if let Some(at) = Self::place(&readers, self.id)
                && let Some(frame) = readers[at].frame.clone().filter(|frame| ready(frame))
            {
                return Ok(frame);
            }
            let now = Instant::now();
            if now >= until {
                let arriving = Self::place(&readers, self.id).map_or_else(Vec::new, |at| readers[at].arriving.clone());
                let waiting = if arriving.is_empty() { String::new() } else { format!(" (waiting for {arriving:?})") };
                return Err(io::Error::other(format!("No rows at the head came within {limit:?}{waiting}")));
            }
            readers = self.shared.made.wait_timeout(readers, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Its newest frame with the keys each table it reads by key holds in it, taken together.
    pub fn frame_keyed(&self) -> Option<(Arc<DecodedFrame>, HeldKeys)> {
        let readers = lock(&self.shared.readers);
        let reading = readers.get(Self::place(&readers, self.id)?)?;
        Some((reading.frame.clone()?, HeldKeys::of_tables(reading.frame_keys.clone())))
    }

    /// Its newest frame, the rows of its tables made anew since the frame it last took (for a
    /// maker opened with [`Views::telling`]; another is told nothing) and the keys each table it
    /// reads by key holds in the frame, taken together; none while it has no frame.
    pub fn take_keyed(&self) -> Option<(Arc<DecodedFrame>, Told, HeldKeys)> {
        let mut readers = lock(&self.shared.readers);
        let at = Self::place(&readers, self.id)?;
        let reading = &mut readers[at];
        let frame = reading.frame.clone()?;
        let told = reading.told.as_mut().map(|telling| std::mem::take(&mut telling.unread)).unwrap_or_default();
        Some((frame, told, HeldKeys::of_tables(reading.frame_keys.clone())))
    }

    /// Reads `table` by key from now on: the rows whose keys' top 64 bits (for a division's
    /// equipment, the division) are among `keys`, and no other. They come in its frame as that
    /// table's view, made from the side's whole mirror of it, so a key newly asked is served at
    /// once with no join; the frame's view holds those keys' rows alone, so its maker must ask no
    /// other ([`Self::frame_keyed`]). A table read so is not among its whole tables.
    ///
    /// The rows of the keys newly asked are made here, from the mirrors the side's last step left:
    /// a maker handed an entity while a layer settles is answered within that settle, not at the
    /// side's next step, which is what a saved frame waits for.
    pub fn declare_keyed(&self, table: TableId, keys: Arc<BTreeSet<u64>>) {
        let mut readers = lock(&self.shared.readers);
        let Some(at) = Self::place(&readers, self.id) else { return };
        let joined = match readers[at].keyed.iter_mut().find(|keyed| keyed.table == table.index()) {
            Some(keyed) if Arc::ptr_eq(&keyed.keys, &keys) => return,
            Some(keyed) => {
                (keyed.keys, keyed.version) = (keys, keyed.version + 1);
                false
            }
            None => {
                readers[at].keyed.push(Keyed { table: table.index(), keys, version: 0 });
                readers[at].keyed.sort_unstable_by_key(|keyed| keyed.table);
                true
            }
        };
        let needs = readers[at].declared.clone();
        drop(readers);
        // A table newly read by key is a table the side's mirrors must hold whole: declared with
        // its reader's own, which bumps the version the side declares again on.
        if joined {
            self.declared(&needs);
        }
        republish(&self.shared, self.id);
    }

    /// Reads every column of `tables` **and no other table** from now on: a maker that has
    /// learned a table passes every table it reads, not the one it learned. A set it no longer
    /// reads is let go, and the rows of a table new to it are made from the side's mirrors at
    /// once.
    pub fn declare_only(&self, tables: &[TableId]) {
        let readers = lock(&self.shared.readers);
        let Some(at) = Self::place(&readers, self.id) else { return };
        if readers[at].declared.iter().map(|need| need.table).eq(tables.iter().copied()) && readers[at].declared.iter().all(|need| need.columns == u64::MAX) {
            return;
        }
        drop(readers);
        self.declared(&tables.iter().map(|&table| Need::every(table)).collect::<Vec<Need>>());
    }

    /// [`Self::declare_only`] with the columns of each table: `needs` are every set it reads
    /// from now on, and no other.
    pub fn declare_only_needs(&self, needs: &[Need]) {
        match self.later {
            true => lock(&self.shared.asked).push(Asked::Declare { id: self.id, needs: needs.to_vec() }),
            false => self.declared(needs),
        }
    }

    /// `needs` declared: the sets it reads, the tables the store keeps for them, and the side told
    /// to declare its mirrors again.
    fn declared(&self, needs: &[Need]) {
        declare(&self.shared, self.id, needs, true);
    }
}

/// Reader `id`'s `needs` declared ([`Reader::declared`]); the rows of what it now reads made from
/// the side's last mirrors at once when `republishes`, else at the side's next post.
fn declare(shared: &Arc<Shared>, id: u64, needs: &[Need], republishes: bool) {
    {
        let mut sets: BTreeMap<usize, u64> = BTreeMap::new();
        for need in needs {
            *sets.entry(need.table.index()).or_default() |= need.columns;
        }
        let mut kept = writing(&shared);
        let mut tables = Vec::with_capacity(sets.len());
        for (&table, &columns) in &sets {
            let Some(kinds) = shared.kinds.get(table) else { continue };
            tables.push((table, over(columns, kinds.len())));
        }
        let mut readers = lock(&shared.readers);
        if let Some(at) = Reader::place(&readers, id) {
            let learned: Vec<usize> = tables.iter().map(|&(table, _)| table).filter(|table| !readers[at].tables.iter().any(|&(held, _)| held == *table)).collect();
            if !learned.is_empty() && !readers[at].tables.is_empty() {
                let hour = lock(&shared.jobs).from.as_ref().map(|from| from.hour);
                let name = readers[at].name;
                let mut held = lock(&shared.learned);
                for table in learned {
                    held.entry((name, table)).or_insert((hour, 0)).1 += 1;
                }
            }
            // A frame made for the declaration before this one does not answer this one: it is let
            // go, and the maker has none until a frame holding every table it now reads is made.
            // Otherwise a maker that learns a table reads the frame it already has, finds the table
            // not declared in it a second time, and refuses its product instead of waiting.
            if readers[at].tables != tables {
                (readers[at].frame, readers[at].stands) = (None, None);
                readers[at].frame_keys.clear();
                readers[at].keyed_views.clear();
            }
            readers[at].tables = tables;
            readers[at].declared = needs.to_vec();
        }
        // Per table, the sets every open maker reads: those it reads whole, and the whole table
        // for one it reads by key, with whether a maker is told of it and whether any reads it
        // whole (a table read by key alone notes every row touched, however many).
        let mut read: BTreeMap<usize, Vec<(u64, bool, bool)>> = BTreeMap::new();
        for reading in readers.iter().filter(|reading| reading.open) {
            let tells = reading.told.is_some();
            let wanted = reading.tables.iter().map(|&(table, columns)| (table, columns, true)).chain(reading.keyed.iter().map(|keyed| (keyed.table, u64::MAX, false)));
            for (table, columns, whole) in wanted {
                let kinds = shared.kinds.get(table).map_or(0, Vec::len);
                let columns = over(columns, kinds);
                let held = read.entry(table).or_default();
                match held.iter_mut().find(|(set, _, _)| *set == columns) {
                    Some((_, told, read_whole)) => (*told, *read_whole) = (*told || tells, *read_whole || whole),
                    None => held.push((columns, tells, whole)),
                }
            }
        }
        drop(readers);
        for (&table, sets) in &read {
            let Some(kinds) = shared.kinds.get(table) else { continue };
            let held = kept[table].get_or_insert_with(|| Kept::new(kinds.clone()));
            let columns = sets.iter().fold(0, |columns, &(set, _, _)| columns | set);
            // A column newly declared is a mirror rebuilt: its views are made whole from it.
            let widened = columns & !held.columns != 0;
            for &(set, told, whole) in sets {
                let at = held.at(set);
                let made = mine(&mut held.views[at].made);
                (made.told, made.whole_read) = (told, whole);
                if !whole {
                    made.bound = usize::MAX;
                }
            }
            if widened {
                held.arriving();
            }
        }
        // A set no maker reads any more is let go, and a table no maker reads at all with it.
        for (table, held) in kept.iter_mut().enumerate() {
            let Some(kept) = held else { continue };
            let sets = read.get(&table).map(Vec::as_slice).unwrap_or_default();
            kept.views.retain(|set| sets.iter().any(|&(columns, _, _)| columns == set.columns));
            kept.columns = kept.views.iter().fold(0, |held, set| held | set.columns);
            if kept.views.is_empty() {
                *held = None;
            }
        }
        drop(kept);
        // The rows of what it now reads are made from the mirrors the side's last step left, so a
        // maker that learns a table reads it at the hour it learned it, with no step between and
        // nothing for the side, the playhead or a stepped frame to do.
        if republishes {
            republish(shared, id);
        }
    }
}

/// Frames before its newest a reader of the display frame's own keeps: the publications the side
/// prepares ahead of the display's hour are a few.
const FRAMES_KEPT: usize = 4;

/// [`Shared::display`] with no hour shown to bound the window's readers.
const NOT_SHOWN: i64 = i64::MIN;

/// A reader of the side's views as a window part holds one: opened on the store the first time it
/// is asked, its sets those given and those [`Self::discover`] finds it reads, closed by
/// [`Self::end_frame`] after a frame that did not ask (a hover that ended, a panel closed). The
/// tables it learned are kept, so it opens with all of them next time.
pub struct AtHead {
    name: &'static str,
    open: Option<Reader>,
    needs: Vec<Need>,
    pace: Pace,
    asked: bool,
    /// The frame asks it every frame it draws and keeps its answer, so it never waits for the
    /// store's lock ([`Self::keeps_its_answer`]).
    now: bool,
}

impl std::fmt::Debug for AtHead {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("AtHead").field("name", &self.name).field("open", &self.open.is_some()).field("tables", &self.needs.len()).finish()
    }
}

impl AtHead {
    /// Every column of `tables`, at `pace`.
    pub fn new(name: &'static str, tables: &[TableId], pace: Pace) -> Self {
        Self::with(name, &tables.iter().copied().map(Need::every).collect::<Vec<Need>>(), pace)
    }

    /// A reader of `needs`: tables with the columns it reads; a table it reads that it did not
    /// name is learned whole.
    pub fn with(name: &'static str, needs: &[Need], pace: Pace) -> Self {
        Self { name, open: None, needs: needs.to_vec(), pace, asked: false, now: false }
    }

    /// A reader the frame asks on every frame it draws and whose answer it keeps: it takes what
    /// the store has published and never waits for its lock ([`Reader::frame_now`]). Every other
    /// reader waits, because it answers something asked for once — a click on the map, a combat
    /// named at load, a picture that must be whole — which a publication must not cost.
    pub fn keeps_its_answer(mut self) -> Self {
        self.now = true;
        self
    }

    /// A reader that declares nothing and learns every table it reads, at the cadence of what
    /// feeds no picture of the map. The window asks it every frame while what it reads is open (a
    /// combat window, a marker's tooltip), so it keeps its answer and never waits for the store.
    pub fn learning(name: &'static str) -> Self {
        Self::with(name, &[], Pace::PerSecond(4)).keeps_its_answer()
    }

    /// The newest views of `views`' store no later than the hour the display shows
    /// ([`Views::show_through`]); none while a table it declared is not home yet, and for a reader
    /// that keeps its answer none while the store holds its readers, so the display frame keeps
    /// what it gave rather than wait ([`Self::keeps_its_answer`]).
    pub fn frame(&mut self, views: &Views) -> Option<Arc<DecodedFrame>> {
        if let Some(hour) = views.shown_through() {
            return self.frame_through(views, hour);
        }
        let now = self.now;
        let open = self.kept(views);
        if now { open.frame_now() } else { open.frame() }
    }

    /// [`Self::frame`] at the newest frame no later than `hour`: what the display shows at `hour`,
    /// never rows prepared past it.
    pub fn frame_through(&mut self, views: &Views, hour: Hour) -> Option<Arc<DecodedFrame>> {
        let now = self.now;
        let open = self.kept(views);
        if now { open.frame_now_through(hour) } else { open.frame_through(hour) }
    }

    /// Kept open through this frame on `views`' store, asked or not: its frames keep coming.
    pub fn keep(&mut self, views: &Views) {
        self.kept(views);
    }

    /// Its frame of `hour` ([`Reader::frame_of`]); none while no frame of that hour is made.
    pub fn frame_of(&mut self, views: &Views, hour: Hour) -> Option<Arc<DecodedFrame>> {
        self.kept(views).frame_of(hour)
    }

    /// Its reader on `views`' store, opened as it reads; asked this frame.
    fn kept(&mut self, views: &Views) -> &Reader {
        self.asked = true;
        let reader = match self.open.take() {
            Some(reader) if views.holds(&reader) => reader,
            _ => match self.now {
                true => views.reader_later(self.name, &self.needs, self.pace),
                false => views.reader_through(self.name, &self.needs, self.pace),
            },
        };
        self.open.insert(reader)
    }

    /// Reads `needs` and no other set from now on: a set it no longer names is let go, and one new
    /// to it is made from the side's mirrors at once.
    pub fn declare(&mut self, needs: &[Need]) {
        let mut needs = needs.to_vec();
        needs.sort_unstable_by_key(|need| need.table.index());
        if needs == self.needs {
            return;
        }
        self.needs = needs;
        if let Some(reader) = &self.open {
            reader.declare_only_needs(&self.needs);
        }
    }

    /// `read` over a frame this gave; none when it read a table not declared yet, which is
    /// declared and made from the side's mirrors at once.
    pub fn discover<T>(&mut self, read: impl FnOnce() -> T) -> Option<T> {
        let (value, missed) = replay_frame::frame::discovering_fields(read);
        if missed.is_empty() {
            return Some(value);
        }
        self.learn_needs(missed.into_iter().map(|(table, fields)| Need::columns(table, fields)));
        None
    }

    /// Declares `tables` whole too, which a read made on another thread found undeclared
    /// (`discovering` is per thread).
    pub fn learn(&mut self, tables: &[TableId]) {
        let new: Vec<Need> = tables.iter().copied().filter(|table| !self.needs.iter().any(|need| need.table == *table && need.columns == u64::MAX)).map(Need::every).collect();
        if !new.is_empty() {
            self.learn_needs(new);
        }
    }

    fn learn_needs(&mut self, needs: impl IntoIterator<Item = Need>) {
        for need in needs {
            match self.needs.iter_mut().find(|held| held.table == need.table) {
                Some(held) => held.columns |= need.columns,
                None => self.needs.push(need),
            }
        }
        self.needs.sort_unstable_by_key(|need| need.table.index());
        if let Some(reader) = &self.open {
            reader.declare_only_needs(&self.needs);
        }
    }

    /// Closes the reader unless it was asked since the last call: its sets are let go and nothing
    /// is held.
    pub fn end_frame(&mut self) {
        if !std::mem::take(&mut self.asked) {
            self.open = None;
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Its reader's newest frame that satisfies `ready`, waited for within `limit`; refused by
    /// name while it is not open.
    pub fn wait(&self, ready: impl FnMut(&DecodedFrame) -> bool, limit: Duration) -> io::Result<Arc<DecodedFrame>> {
        self.open.as_ref().ok_or_else(|| io::Error::other("The reader at the head is not open"))?.wait(ready, limit)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        match self.later {
            true => lock(&self.shared.asked).push(Asked::Close { id: self.id }),
            false => close(&self.shared, self.id),
        }
    }
}

/// Reader `id` closed: its frame let go and its sets with it.
fn close(shared: &Arc<Shared>, id: u64) {
    let mut readers = lock(&shared.readers);
    if let Some(at) = Reader::place(&readers, id) {
        readers[at].open = false;
        (readers[at].frame, readers[at].stands) = (None, None);
    }
    drop(readers);
    declare(shared, id, &[], true);
}
