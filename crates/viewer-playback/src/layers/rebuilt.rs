//! A product today's builder makes whole from a frame of rows, made from rows at the head on a worker
//! the layer owns. The ships' names and the aircraft's models are chosen by the content's scripts
//! over any table, so their builders read rows ([`crate::playhead_host::rows`]), learning the tables
//! they read; the frame thread only hands the worker the hour the map applied and takes the newest
//! product. A newer hour supersedes one not started, and the frame never waits for the worker.
//! The worker takes each newer frame of rows once its product before is made, and builds only when
//! a table it reads changed since its product's rows; otherwise it asks the builder whether the
//! product stands at the newer rows ([`Build::stands`]), and if so its product is current there too
//! ([`Built::current`]) with nothing built.
//! A table the recording holds nothing of is never declared: the product is refused by name.
//! Builds run under the recording's executable tables; a build that panics is refused by name.
//! In the window the layer takes the view's perspective, its builder makes the vision from the same
//! rows, and the layer hands each product it takes to the scene ([`Build::install`]).

use super::Published;
use crate::playhead_host::{
    Entry,
    rows::{Cadence, Recording, RowReader, Rows},
};
use engine_executable::executable_tables::Tables;
use replay_data::{
    Hour,
    rows::{TableId, TableRows},
};
use replay_frame::frame::{DecodedFrame, SharedView, TableView, discovering};
use std::{
    io,
    panic::AssertUnwindSafe,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicI32, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use viewer_foundation::map::{
    perspective::{Choice, Vision},
    presentation::Continuity,
};

/// How often the worker looks for new rows when nothing wakes it: a frame at 60 Hz.
const TICK: Duration = Duration::from_micros(16_667);

/// How long the window's whole settle waits for the product of its hour.
const WHOLE: Duration = Duration::from_secs(30);

/// What a product is built for besides the rows: the viewer's perspective and what it sees; none
/// when the builder makes the vision from the rows themselves.
pub type Seen = (Choice, Option<Arc<Vision>>);

/// Today's builder of a product over a frame of rows.
pub trait Build: Send + Sync + 'static {
    type Product: Send + Sync + 'static;
    /// The recording's executable tables, in scope for every build: the triggers it evaluates read them.
    fn tables(&self) -> Option<Tables>;
    fn build(&self, frame: &DecodedFrame, seen: &Seen) -> io::Result<Self::Product>;
    /// [`Self::build`] keeping nothing from a build before: what a compose from `frame` gives.
    fn anew(&self, frame: &DecodedFrame, seen: &Seen) -> io::Result<Self::Product> {
        self.build(frame, seen)
    }
    /// Whether `product`, built from rows whose every table `frame` holds unchanged, is the product
    /// of `frame` too: what it read beyond the rows (the hour itself) reads the same. Never, unless
    /// the builder can tell.
    fn stands(&self, product: &Self::Product, frame: &DecodedFrame, seen: &Seen) -> bool {
        let _ = (product, frame, seen);
        false
    }
    /// Its product reads the hour itself (the content's triggers compare dates): its reader
    /// declares the Clock, so it is made rows at every step, and rows whose other tables are the
    /// same are judged by [`Self::stands`] at the new hour rather than built again.
    fn hourly(&self) -> bool {
        false
    }
    /// Takes what of `view` it builds for besides the perspective; true when that changed, so the
    /// product held is let go and only one built after the change is taken.
    fn viewed(&self, view: &super::View) -> bool {
        let _ = view;
        false
    }
    /// Each named part of the build that made `product`, in nanoseconds, for the worker's sums.
    fn parts(&self, product: &Self::Product, each: &mut dyn FnMut(&'static str, u64)) {
        let _ = (product, each);
    }
    /// Each named count of the build that made `product`, for the worker's sums.
    fn counts(&self, product: &Self::Product, each: &mut dyn FnMut(&'static str, u64)) {
        let _ = (product, each);
    }
    /// Hands `product` (none: draw nothing) to the scene, giving back the product it replaced.
    fn install(&self, gpu: &mut super::Gpu<'_>, product: Option<&Self::Product>, continuity: Continuity) -> io::Result<Option<Self::Product>> {
        let _ = (gpu, product, continuity);
        Ok(None)
    }
}

/// One product and what it was built from: the rows and their hour, the vision and its generation,
/// and the build's nanoseconds; an error is the builder's refusal by name. The worker may find
/// it current at a later hour's rows too ([`Self::current`]).
pub struct Built<P> {
    pub hour: Hour,
    pub generation: u64,
    pub product: Result<P, String>,
    pub ns: u64,
    pub frame: Option<Arc<DecodedFrame>>,
    pub seen: Option<Seen>,
    current: AtomicI32,
    /// The hour the rows it is current at stand at: no step since changed a table it reads.
    stands: AtomicI32,
}

impl<P> Built<P> {
    fn new(hour: Hour, generation: u64, product: Result<P, String>, ns: u64, frame: Option<Arc<DecodedFrame>>, seen: Option<Seen>) -> Self {
        Self { hour, generation, product, ns, frame, seen, current: AtomicI32::new(hour), stands: AtomicI32::new(hour) }
    }

    /// The hour of the newest rows it is the product of: its own, or later rows that changed
    /// nothing it read.
    pub fn current(&self) -> Hour {
        self.current.load(Ordering::Acquire)
    }

    /// The newest hour its rows stand at: [`Self::current`], or a later hour no step reached a
    /// table it reads by. What its lag is measured from; a builder that reads the hour itself is
    /// built again only for rows the reader makes.
    pub fn stands(&self) -> Hour {
        self.stands.load(Ordering::Acquire).max(self.current())
    }

    /// Whether it is the product of the rows of `hour`.
    fn of(&self, hour: Hour) -> bool {
        self.hour == hour || self.current() == hour
    }
}

/// What the worker measured: builds, their nanoseconds (each one while timed), the frames of rows
/// published to it that it never took, builds discarded for a table they read undeclared, and rows
/// found to change nothing its product read ([`Build::stands`]) with the nanoseconds every such
/// check took, found or not; the builds' named parts and counts
/// summed ([`Build::parts`], [`Build::counts`]); how far behind the map's head the rows of each
/// build were when it began, by [`BEHIND`] bucket, with their sum and most; and when the first
/// build began and the last ended.
#[derive(Clone, Debug, Default)]
pub struct Builds {
    pub count: u64,
    pub total_ns: u64,
    pub most_ns: u64,
    pub ns: Vec<u64>,
    pub skipped: u64,
    pub discovered: u64,
    pub confirmed: u64,
    pub confirm_ns: u64,
    pub parts: Vec<(&'static str, u64)>,
    pub counts: Vec<(&'static str, u64)>,
    pub behind: [u64; BEHIND.len()],
    pub behind_sum: u64,
    pub behind_most: u64,
    pub span: Option<(Instant, Instant)>,
}

/// The upper bounds, in game hours, of the buckets of how far behind the head a build's rows were.
pub const BEHIND: [u64; 7] = [0, 24, 96, 384, 1_536, 6_144, u64::MAX];

impl Builds {
    fn record(&mut self, began: Instant, ns: u64, behind: Option<u64>, timed: bool) {
        (self.count, self.total_ns, self.most_ns) = (self.count + 1, self.total_ns + ns, self.most_ns.max(ns));
        if timed {
            self.ns.push(ns);
        }
        if let Some(behind) = behind {
            self.behind[BEHIND.iter().position(|&bound| behind <= bound).unwrap_or(BEHIND.len() - 1)] += 1;
            (self.behind_sum, self.behind_most) = (self.behind_sum + behind, self.behind_most.max(behind));
        }
        let ended = Instant::now();
        self.span = Some((self.span.map_or(began, |(first, _)| first), ended));
    }

    fn part(&mut self, name: &'static str, ns: u64) {
        add(&mut self.parts, name, ns);
    }

    fn count(&mut self, name: &'static str, count: u64) {
        add(&mut self.counts, name, count);
    }
}

fn add(sums: &mut Vec<(&'static str, u64)>, name: &'static str, value: u64) {
    match sums.iter_mut().find(|(held, _)| *held == name) {
        Some((_, sum)) => *sum += value,
        None => sums.push((name, value)),
    }
}

struct State<P> {
    source: Option<(Arc<Entry>, Recording)>,
    seen: Option<Seen>,
    generation: u64,
    /// Whether the view draws the product: closed, the worker holds no reader and no product.
    open: bool,
    stop: bool,
    /// The hour the map applied last, which the rows are driven to.
    head: Option<Hour>,
    rows: Option<Arc<Rows>>,
    /// A reader declared by the caller of [`Rebuilt::set_source`], for the worker to take.
    reader: Option<RowReader>,
    built: Option<Arc<Built<P>>>,
    tables: Vec<TableId>,
    builds: Builds,
    /// The hour of the newest rows the worker took, and the tables declared whose rows have not
    /// arrived yet.
    seen_hour: Option<Hour>,
    awaiting: Vec<TableId>,
    /// Why every product of this recording is refused: a table it reads that the recording holds
    /// nothing of, or rows at the head refused.
    refused: Option<String>,
    /// Why the worker stopped, if it did: it builds nothing more.
    stopped: Option<String>,
    /// What the layer let go (products, the scene's replaced plans), dropped here: a plan and its
    /// frame of rows are not freed on the frame thread.
    retired: Vec<Box<dyn Send>>,
}

struct Shared<P> {
    state: Mutex<State<P>>,
    wake: Condvar,
    built: Condvar,
}

impl<P> Shared<P> {
    fn lock(&self) -> MutexGuard<'_, State<P>> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The worker and what it shares with the layer.
pub struct Rebuilt<P> {
    shared: Arc<Shared<P>>,
    worker: Option<JoinHandle<()>>,
    /// Whether the layer drives the rows to the hour the map applied (a caller that plays the
    /// playhead itself); otherwise the rows follow the playhead's clock, which is the map's.
    drive: bool,
    /// Its name, and whether it reports what it measured when it stops.
    name: &'static str,
    timed: bool,
    cadence: Cadence,
}

impl<P: Send + Sync + 'static> Rebuilt<P> {
    /// A worker named `name` building with `build` over `tables` and those it learns it reads, from
    /// rows at `cadence`.
    pub fn new<B: Build<Product = P>>(name: &'static str, build: Arc<B>, tables: &[TableId], cadence: Cadence, drive: bool, timed: bool) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                source: None,
                seen: None,
                generation: 0,
                open: false,
                stop: false,
                head: None,
                rows: None,
                reader: None,
                built: None,
                tables: {
                    let mut tables = tables.to_vec();
                    if build.hourly() && !tables.contains(&TableId::Clock) {
                        tables.push(TableId::Clock);
                    }
                    tables
                },
                builds: Builds::default(),
                seen_hour: None,
                awaiting: Vec::new(),
                refused: None,
                stopped: None,
                retired: Vec::new(),
            }),
            wake: Condvar::new(),
            built: Condvar::new(),
        });
        let working = Arc::clone(&shared);
        let worker = std::thread::Builder::new().name(name.into()).spawn(move || {
            viewer_foundation::worker_priority::background_priority();
            let worked = std::panic::catch_unwind(AssertUnwindSafe(|| work(&working, &*build, &Rules { name, cadence, drive, timed })));
            if let Err(panic) = worked {
                working.lock().stopped = Some(format!("The {name} worker stopped: {}", panicked(&*panic)));
                working.built.notify_all();
            }
        })?;
        Ok(Self { shared, worker: Some(worker), drive, name, timed, cadence })
    }

    /// The playhead whose rows it reads, and the vision it builds for: a new vision builds again.
    /// Its reader is declared here, and the call returns once the rows' worker has declared it, so
    /// a seek after it brings the tables (a table declared later joins the playhead past its head).
    pub fn set_source(&self, entry: Arc<Entry>, recording: Recording, seen: Seen) -> io::Result<()> {
        self.attach(entry, recording)?;
        self.set_seen(seen);
        Ok(())
    }

    /// The playhead whose rows it reads; it builds nothing until it is given what it builds for.
    pub fn attach(&self, entry: Arc<Entry>, recording: Recording) -> io::Result<()> {
        let mut state = self.shared.lock();
        if state.source.as_ref().is_some_and(|(held, _)| Arc::ptr_eq(held, &entry)) {
            return Ok(());
        }
        let absent = absent(&entry, &state.tables);
        state.refused = (!absent.is_empty()).then(|| refusal(&format!("the {} layer", self.name), &absent));
        let rows = Rows::of(&entry, recording)?;
        state.reader = state.refused.is_none().then(|| rows.reader(&state.tables, self.cadence));
        state.awaiting = if state.reader.is_some() { state.tables.clone() } else { Vec::new() };
        let declared = state.reader.is_some().then(|| Arc::clone(&rows));
        (state.source, state.rows) = (Some((entry, recording)), Some(rows));
        drop(state);
        if let Some(rows) = declared {
            rows.settled();
        }
        Ok(())
    }

    /// What it builds for: the next build is for it.
    pub fn set_seen(&self, seen: Seen) {
        let mut state = self.shared.lock();
        state.seen = Some(seen);
        state.generation += 1;
        drop(state);
        self.shared.wake.notify_all();
    }

    /// What it builds for changed: the next build is for it. The generation a build after the
    /// change carries.
    pub fn renew(&self) -> u64 {
        let mut state = self.shared.lock();
        state.generation += 1;
        let generation = state.generation;
        drop(state);
        self.shared.wake.notify_all();
        generation
    }

    /// The perspective of the vision it builds for.
    pub fn perspective(&self) -> Option<Choice> {
        self.shared.lock().seen.as_ref().map(|(choice, _)| choice.clone())
    }

    /// Opens or closes it: closed, the worker lets its reader and product go.
    pub fn set_open(&self, open: bool) {
        let mut state = self.shared.lock();
        let declared = !open && state.reader.take().is_some();
        if state.open != open || declared {
            state.open = open;
            if !open {
                state.built = None;
            }
            drop(state);
            self.shared.wake.notify_all();
        }
    }

    /// The map applied `hour`: the rows are driven there when this layer drives them.
    pub fn applied(&self, hour: Hour) {
        let mut state = self.shared.lock();
        if state.head == Some(hour) {
            return;
        }
        state.head = Some(hour);
        let rows = if self.drive { state.rows.clone() } else { None };
        drop(state);
        if let Some(rows) = rows {
            rows.drive(hour);
        }
        self.shared.wake.notify_all();
    }

    /// The newest product the worker built, if newer than `held`.
    pub fn newer(&self, held: Option<&Arc<Built<P>>>) -> Option<Arc<Built<P>>> {
        let state = self.shared.lock();
        let built = state.built.as_ref()?;
        let newer = held.is_none_or(|held| !Arc::ptr_eq(held, built));
        newer.then(|| Arc::clone(built))
    }

    /// Hands what the layer let go back to the worker, which drops it.
    pub fn retire(&self, gone: impl Send + 'static) {
        self.shared.lock().retired.push(Box::new(gone));
        self.shared.wake.notify_all();
    }

    /// Waits until the worker has the product of `hour`'s rows with the newest vision; for a caller
    /// that holds the head (the proof tool, tests), never the window. A refused worker answers its
    /// refusal at once; a stopped one is an error at once.
    pub fn wait(&self, hour: Hour, limit: Duration) -> io::Result<Arc<Built<P>>> {
        let until = Instant::now() + limit;
        let mut state = self.shared.lock();
        loop {
            if let Some(built) = &state.built
                && built.of(hour)
                && built.generation == state.generation
            {
                return Ok(Arc::clone(built));
            }
            if let Some(why) = &state.stopped {
                return Err(io::Error::other(why.clone()));
            }
            if let Some(why) = &state.refused {
                return Ok(Arc::new(Built::new(hour, state.generation, Err(why.clone()), 0, None, None)));
            }
            let now = Instant::now();
            if now >= until {
                let had = state.built.as_ref().map(|built| (built.hour, built.current(), built.generation));
                return Err(io::Error::other(format!(
                    "The worker built no product at hour {hour} with vision {} in {limit:?}; it holds {had:?}, its newest rows are at {:?}, it awaits the rows of {:?}; the rows' worker: {:?}",
                    state.generation,
                    state.seen_hour,
                    state.awaiting,
                    state.rows.as_ref().and_then(|rows| rows.error())
                )));
            }
            state = self.shared.built.wait_timeout(state, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Waits until the worker took the rows of `hour`; false at `until`.
    #[cfg(test)]
    pub fn looked(&self, hour: Hour, until: Instant) -> bool {
        let mut state = self.shared.lock();
        loop {
            if state.seen_hour == Some(hour) {
                return true;
            }
            let now = Instant::now();
            if now >= until || state.stopped.is_some() {
                return false;
            }
            state = self.shared.built.wait_timeout(state, (until - now).min(TICK)).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    pub fn builds(&self) -> Builds {
        self.shared.lock().builds.clone()
    }

    /// The tables it reads, given and learned.
    pub fn tables(&self) -> Vec<TableId> {
        self.shared.lock().tables.clone()
    }

    /// The tables it declared whose rows have not arrived yet.
    pub fn awaiting(&self) -> Vec<TableId> {
        self.shared.lock().awaiting.clone()
    }

    /// Why the worker stopped, if it did.
    pub fn stopped(&self) -> Option<String> {
        self.shared.lock().stopped.clone()
    }
}

impl<P> Drop for Rebuilt<P> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.stop = true;
        if self.timed {
            report(self.name, &state);
        }
        drop(state);
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The tables of `tables` the recording holds nothing of: declared, the playhead could not seek
/// them, so they are refused by name instead.
pub fn absent(entry: &Entry, tables: &[TableId]) -> Vec<TableId> {
    tables.iter().copied().filter(|&table| !crate::playhead_host::details::held_at_all(entry, table)).collect()
}

/// Why `what` is refused when the recording holds nothing of `absent`.
pub fn refusal(what: &str, absent: &[TableId]) -> String {
    let names: Vec<String> = absent.iter().map(|table| format!("{table:?}")).collect();
    format!("The recording holds no {}, which {what} reads", names.join(", "))
}

/// The message a panic carried.
pub fn panicked(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|text| (*text).to_owned()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic with no message".to_owned())
}

/// `run` under the recording's executable tables of `build`; a panic is its refusal by name.
fn attempt<B: Build, T>(build: &B, run: impl FnOnce() -> io::Result<T>) -> Result<T, String> {
    let ran = std::panic::catch_unwind(AssertUnwindSafe(|| engine_executable::executable_tables::scope(build.tables(), run)));
    match ran {
        Ok(product) => product.map_err(|error| error.to_string()),
        Err(panic) => Err(format!("The build panicked: {}", panicked(&*panic))),
    }
}

/// `build` over `frame`, and every table it read undeclared. A build usually stops at the first
/// table it misses, so it goes on over the frame with each missed table given empty until it
/// misses no new one: one round learns every table the build reaches.
fn learning<B: Build>(build: &B, frame: &DecodedFrame, seen: &Seen) -> (Result<B::Product, String>, Vec<TableId>) {
    let (product, mut missed) = discovering(|| attempt(build, || build.build(frame, seen)));
    let mut trial: Option<DecodedFrame> = None;
    let mut given = 0;
    while given < missed.len() {
        let trial = trial.get_or_insert_with(|| frame.clone());
        for &table in &missed[given..] {
            trial.tables[table.index()] = SharedView::from(TableView::Complete(TableRows::empty(table)));
        }
        given = missed.len();
        let (_, more) = discovering(|| attempt(build, || build.build(trial, seen)));
        let more: Vec<TableId> = more.into_iter().filter(|table| !missed.contains(table)).collect();
        missed.extend(more);
    }
    (product, missed)
}

/// Whether `product` of `built`'s rows is the product of `frame` too: `frame` holds every one of
/// `tables` as the same rows, and the builder finds what it read beyond them reads the same, under
/// the recording's tables, reading no table undeclared.
fn stands<B: Build>(build: &B, built: &Built<B::Product>, frame: &DecodedFrame, tables: &[TableId], seen: &Seen) -> bool {
    let (Some(then), Ok(product)) = (&built.frame, &built.product) else { return false };
    // The Clock an hourly builder declares is the hour, which its own check judges.
    let same = then.branch == frame.branch && tables.iter().filter(|&&table| !(build.hourly() && table == TableId::Clock)).all(|table| SharedView::ptr_eq(&then.tables[table.index()], &frame.tables[table.index()]));
    if !same {
        return false;
    }
    let (stands, missed) = discovering(|| attempt(build, || Ok(build.stands(product, frame, seen))));
    missed.is_empty() && stands == Ok(true)
}

/// Whether `frame` holds every one of `tables`: a frame made before a table was declared does not,
/// and a build over it would learn that table again.
pub fn holds(frame: &DecodedFrame, tables: &[TableId]) -> bool {
    tables.iter().all(|table| !frame.tables[table.index()].is_not_declared())
}

/// What a timed worker measured, one line on stderr: its builds' milliseconds, frames passed over,
/// the tables it read and what the rows at the head hold for them.
fn report<P>(name: &str, state: &State<P>) {
    let builds = &state.builds;
    let mut ms: Vec<f64> = builds.ns.iter().map(|&ns| ns as f64 / 1e6).collect();
    ms.sort_unstable_by(f64::total_cmp);
    let at = |q: f64| ms.get(((ms.len() as f64 - 1.0) * q).round() as usize).copied();
    let held = state.rows.as_ref().map(|rows| rows.held());
    let mine = |list: &[(TableId, usize)]| -> usize { list.iter().filter(|(table, _)| state.tables.contains(table)).map(|(_, bytes)| bytes).sum() };
    let line = serde_json::json!({
        "worker": name, "builds": builds.count, "build_ms": {"p50": at(0.5), "p99": at(0.99), "most": builds.most_ns as f64 / 1e6, "mean": builds.total_ns as f64 / 1e6 / builds.count.max(1) as f64},
        "passed_over": builds.skipped, "discovered": builds.discovered, "confirmed": builds.confirmed, "tables": state.tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
        "awaiting": state.awaiting.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(), "refused": state.refused, "stopped": state.stopped,
        "rows_worker_mirrors_bytes": held.as_ref().map(|held| mine(&held.mirrors)), "rows_bytes": held.as_ref().map(|held| mine(&held.rows)),
    });
    eprintln!("{line}");
}

/// The worker's name and rules, as the layer gave them.
struct Rules {
    name: &'static str,
    cadence: Cadence,
    /// Whether the layer drives the rows to the hour the map applied.
    drive: bool,
    timed: bool,
}

/// Publishes `why` as the product at the map's hour and the newest vision, once for each.
fn refuse<P>(shared: &Shared<P>, state: &mut State<P>, why: String) {
    let Some(hour) = state.head else { return };
    if state.built.as_ref().is_some_and(|built| built.hour == hour && built.generation == state.generation) {
        return;
    }
    state.built = Some(Arc::new(Built::new(hour, state.generation, Err(why), 0, None, None)));
    shared.built.notify_all();
}

/// What the worker found to build from: the newest frame of rows at the head and its publish
/// count; nothing yet; or a refusal to publish first.
enum Found<'a, P> {
    Frame(MutexGuard<'a, State<P>>, Arc<DecodedFrame>, u64),
    Wait(MutexGuard<'a, State<P>>),
    Again,
}

/// The newest frame of rows at the head for `held`, a reader of the source's rows.
fn from_rows<'a, P>(mut state: MutexGuard<'a, State<P>>, held: &mut Option<RowReader>, rules: &Rules) -> Found<'a, P> {
    if held.is_none() {
        *held = state.reader.take();
    }
    if held.is_none() {
        let rows = match (state.rows.clone(), state.source.clone()) {
            (Some(rows), _) => Ok(rows),
            (None, Some((entry, recording))) => Rows::of(&entry, recording),
            (None, None) => return Found::Wait(state),
        };
        match rows {
            Ok(rows) => {
                *held = Some(rows.reader(&state.tables, rules.cadence));
                state.awaiting = state.tables.clone();
                if rules.drive
                    && let Some(head) = state.head
                {
                    rows.drive(head);
                }
                state.rows = Some(rows);
            }
            Err(error) => {
                state.refused = Some(format!("Rows at the head are refused: {error}"));
                return Found::Again;
            }
        }
    }
    match held.as_ref().and_then(RowReader::newest).filter(|newest| holds(&newest.frame, &state.tables)) {
        Some(newest) => Found::Frame(state, newest.frame, newest.published),
        None => Found::Wait(state),
    }
}

fn work<B: Build>(shared: &Shared<B::Product>, build: &B, rules: &Rules) {
    let mut reader: Option<RowReader> = None;
    // The rows last taken, their publish count and the generation they were taken for.
    let mut last: Option<(Arc<DecodedFrame>, u64, u64)> = None;
    loop {
        let mut state = shared.lock();
        if state.stop {
            return;
        }
        if !state.retired.is_empty() {
            let retired = std::mem::take(&mut state.retired);
            drop(state);
            drop(retired);
            continue;
        }
        if !state.open || state.seen.is_none() {
            // Closed, it holds no reader: its tables leave the rows and come back as a reset. A
            // reader declared for it before the view first opens it ([`Rebuilt::attach`]) is kept
            // for then, so the seek after brings its tables; closing drops it ([`Rebuilt::set_open`]).
            (reader, last) = (None, None);
            state.awaiting.clear();
            let _ = shared.wake.wait_timeout(state, TICK);
            continue;
        }
        if let Some(why) = state.refused.clone() {
            (reader, state.reader) = (None, None);
            state.awaiting.clear();
            refuse(shared, &mut state, why);
            let _ = shared.wake.wait_timeout(state, TICK);
            continue;
        }
        let generation = state.generation;
        let (mut state, frame, published) = match from_rows(state, &mut reader, rules) {
            Found::Frame(state, frame, published) => (state, frame, published),
            Found::Wait(state) => {
                let _ = shared.wake.wait_timeout(state, TICK);
                continue;
            }
            Found::Again => continue,
        };
        state.awaiting.clear();
        if last.as_ref().is_some_and(|(held, _, taken)| Arc::ptr_eq(held, &frame) && *taken == generation) || state.generation != generation {
            if let Some(built) = state.built.as_ref().filter(|built| built.generation == generation && built.current() >= frame.hour)
                && let Some(through) = reader.as_ref().and_then(|reader| reader.through(Arc::as_ptr(&frame)))
                && through > built.stands()
            {
                built.stands.store(through, Ordering::Release);
            }
            if state.generation != generation {
                continue;
            }
            drop(state);
            // Its product of these rows made: it has finished with them, and waits for the next.
            if let Some(reader) = &reader {
                reader.ready();
                let _ = reader.wait_newer(published, TICK);
            }
            continue;
        }
        if let Some((_, before, _)) = &last {
            state.builds.skipped += published.saturating_sub(before + 1);
        }
        last = Some((Arc::clone(&frame), published, generation));
        let seen = state.seen.clone().expect("checked above");
        state.seen_hour = Some(frame.hour);
        let held = state.built.clone().filter(|built| built.generation == generation);
        let tables = state.tables.clone();
        drop(state);
        if let Some(held) = held {
            let checked = Instant::now();
            let stood = stands(build, &held, &frame, &tables, &seen);
            let mut state = shared.lock();
            state.builds.confirm_ns += checked.elapsed().as_nanos() as u64;
            if stood {
                held.current.store(frame.hour, Ordering::Release);
                state.builds.confirmed += 1;
                drop(state);
                shared.built.notify_all();
                continue;
            }
        }
        let behind = shared.lock().head.map(|head| u64::try_from(head - frame.hour).unwrap_or(0));
        let started = Instant::now();
        let (product, missed) = learning(build, &frame, &seen);
        let ns = started.elapsed().as_nanos() as u64;
        let mut state = shared.lock();
        if !missed.is_empty() {
            let absent = state.source.as_ref().map_or_else(Vec::new, |(entry, _)| absent(entry, &missed));
            if !absent.is_empty() {
                state.refused = Some(refusal(&format!("the {} layer", rules.name), &absent));
                continue;
            }
            state.tables.extend(missed.iter().copied());
            state.tables.sort_unstable_by_key(|table| table.index());
            state.tables.dedup();
            state.awaiting = missed;
            state.builds.discovered += 1;
            if let Some(reader) = &reader {
                reader.declare(&state.tables);
            }
            continue;
        }
        let builds = &mut state.builds;
        builds.record(started, ns, behind, rules.timed);
        if let Ok(product) = &product {
            build.parts(product, &mut |name, ns| builds.part(name, ns));
            build.counts(product, &mut |name, count| builds.count(name, count));
        }
        if state.open {
            state.built = Some(Arc::new(Built::new(frame.hour, generation, product, ns, Some(frame), Some(seen))));
        }
        drop(state);
        shared.built.notify_all();
    }
}

/// A layer drawing a product rebuilt whole on its worker: of the map's mirrors it reads only the
/// Clock's rows, for the hour the map holds (a seek reaches it as a compose, not a step); it hands the
/// worker that hour and takes the newest product in `settle`. It carries nothing: the worker's
/// work is not the frame thread's, and the hour of the product it shows is its [`Published`].
pub struct Whole<B: Build> {
    name: &'static str,
    build: Arc<B>,
    /// Whether a view draws the product.
    drawn: fn(&B, &super::View) -> bool,
    /// The bytes a product holds.
    bytes: fn(&B::Product) -> usize,
    rebuilt: Rebuilt<B::Product>,
    held: Option<Arc<Built<B::Product>>>,
    open: bool,
    /// The generation of the first build for what the view last asked ([`Build::viewed`]): one
    /// older was built for something else and is never taken.
    since: u64,
    /// Hours handed to the worker, and products taken.
    marked: [usize; 2],
    window: Option<Window>,
}

/// What the window's layer keeps: the perspective it builds for, and what the scene is to be given
/// next (the held product, or nothing while closed); the first product after a compose starts anew.
#[derive(Default)]
struct Window {
    perspective: Option<Choice>,
    install: Option<Continuity>,
    reset: bool,
    /// The scene holds no plan: closed, nothing need be installed again.
    empty: bool,
}

impl<B: Build> Whole<B> {
    /// A layer whose worker takes rows at `cadence`, and builds only when a table it reads changed
    /// since its product's rows or the builder finds the product does not stand ([`Build::stands`]).
    /// `drive`: the layer drives the rows to the map's hour (a caller that plays the playhead
    /// itself); otherwise they follow the map's head. `timed`: every build's time is kept.
    #[expect(clippy::too_many_arguments, reason = "a layer's worker, its builder and its rules are given together")]
    pub fn new(name: &'static str, build: B, drawn: fn(&B, &super::View) -> bool, bytes: fn(&B::Product) -> usize, tables: &[TableId], cadence: Cadence, drive: bool, timed: bool) -> io::Result<Self> {
        let build = Arc::new(build);
        let rebuilt = Rebuilt::new(name, Arc::clone(&build), tables, cadence, drive, timed)?;
        Ok(Self { name, build, drawn, bytes, rebuilt, held: None, open: false, since: 0, marked: [0; 2], window: None })
    }

    /// The layer the window draws: its rows from `entry`, built for the view's perspective, each
    /// product it takes handed to the scene.
    pub fn in_window(mut self, entry: Arc<Entry>, recording: Recording) -> io::Result<Self> {
        self.rebuilt.attach(entry, recording)?;
        self.window = Some(Window::default());
        Ok(self)
    }

    /// Whether the window draws its product rather than today's: closed (it draws nothing, as
    /// today's draws nothing there) or holding a product. Until its first product, today's.
    pub fn shows(&self) -> bool {
        !self.open || self.held.is_some()
    }

    /// Why the product it shows is refused, by name.
    pub fn refusal(&self) -> Option<&str> {
        self.held.as_ref().filter(|_| self.open).and_then(|built| built.product.as_ref().err()).map(String::as_str)
    }

    pub fn rebuilt(&self) -> &Rebuilt<B::Product> {
        &self.rebuilt
    }

    pub fn build(&self) -> &B {
        &self.build
    }

    /// The product the layer draws, and the hour its rows are at.
    pub fn product(&self) -> Option<&Arc<Built<B::Product>>> {
        self.held.as_ref()
    }

    pub fn drawn(&self) -> bool {
        self.open
    }

    pub fn marked(&self) -> [usize; 2] {
        self.marked
    }

    /// The product built anew from the rows and vision `built` was built from: what a compose
    /// from the same state gives.
    pub fn again(&self, built: &Built<B::Product>) -> Option<io::Result<B::Product>> {
        let (frame, seen) = (built.frame.as_ref()?, built.seen.as_ref()?);
        Some(discovering(|| attempt(&*self.build, || self.build.anew(frame, seen))).0.map_err(io::Error::other))
    }
}

impl<B: Build> super::Layer for Whole<B> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn placed(&mut self, view: &crate::layers::View) -> Option<u64> {
        Some(u64::from((self.drawn)(&self.build, view)))
    }

    fn inputs(&mut self, view: &super::View, inputs: &mut super::Inputs<'_>) -> io::Result<()> {
        self.open = (self.drawn)(&self.build, view);
        self.rebuilt.set_open(self.open);
        if let Some(window) = &mut self.window
            && window.perspective.as_ref() != Some(&view.perspective)
        {
            window.perspective = Some(view.perspective.clone());
            self.rebuilt.set_seen((view.perspective.clone(), None));
        }
        if self.build.viewed(view) {
            self.since = self.rebuilt.renew();
            if let Some(old) = self.held.take() {
                self.rebuilt.retire(old);
            }
            // What the scene shows was made for the view before: open, it is taken down now.
            if let Some(window) = self.window.as_mut().filter(|_| self.open) {
                (window.install, window.reset) = (Some(Continuity::Reset), true);
            }
        }
        if !self.open {
            if let Some(old) = self.held.take() {
                self.rebuilt.retire(old);
            }
            if let Some(window) = self.window.as_mut().filter(|window| !window.empty) {
                window.install = Some(Continuity::Reset);
            }
            return Ok(());
        }
        inputs.table(TableId::Clock, false)
    }

    fn compose(&mut self, _mirrors: &crate::playhead_host::mirror::Mirrors) -> io::Result<()> {
        if let Some(window) = &mut self.window {
            window.reset = true;
        }
        Ok(())
    }

    fn apply(&mut self, step: &playhead::Step, _mirrors: &crate::playhead_host::mirror::Mirrors) {
        self.rebuilt.applied(step.last());
        if self.open {
            self.marked[0] += 1;
        }
    }

    fn settle(&mut self, mirrors: &crate::playhead_host::mirror::Mirrors, budget: &mut super::Budget) -> io::Result<()> {
        if !self.open {
            return Ok(());
        }
        if let Some(hour) = mirrors.get(TableId::Clock).and_then(|clock| clock.head()) {
            self.rebuilt.applied(hour);
            // The window's whole settle (the stepped harness) draws the product of its hour.
            if self.window.is_some() && budget.unlimited_now() {
                let _ = self.rebuilt.wait(hour, WHOLE);
            }
        }
        if let Some(newer) = self.rebuilt.newer(self.held.as_ref()).filter(|built| built.generation >= self.since) {
            if let Some(old) = self.held.replace(newer) {
                self.rebuilt.retire(old);
            }
            self.marked[1] += 1;
            if let Some(window) = &mut self.window {
                window.install = Some(if std::mem::take(&mut window.reset) { Continuity::Reset } else { Continuity::Continue });
            }
        }
        Ok(())
    }

    /// The product taken into the scene, which carries what it shows into it as today's install
    /// does (anew after a compose); closed or refused, nothing.
    fn upload(&mut self, gpu: &mut super::Gpu<'_>, budget: &mut super::Budget) -> io::Result<()> {
        let Some(window) = &mut self.window else { return Ok(()) };
        let Some(continuity) = window.install else { return Ok(()) };
        if !budget.left() {
            return Ok(());
        }
        let product = self.held.as_ref().filter(|_| self.open).and_then(|built| built.product.as_ref().ok());
        if let Some(replaced) = self.build.install(gpu, product, continuity)? {
            self.rebuilt.retire(replaced);
        }
        (window.install, window.empty) = (None, product.is_none());
        Ok(())
    }

    fn uploaded(&self) -> bool {
        self.window.as_ref().is_none_or(|window| window.install.is_none())
    }

    fn carried(&self) -> super::Carried {
        super::Carried::default()
    }

    /// Nothing before its first product; then the hour its rows stand at ([`Built::stands`]): it
    /// counts in the lag and does not hold the window clock's drawn hour.
    fn published(&self) -> Published {
        if !self.open {
            return Published::Current;
        }
        match &self.held {
            None => Published::Nothing,
            Some(built) => Published::Cadence(built.stands()),
        }
    }

    fn held(&self) -> usize {
        self.held.as_ref().and_then(|built| built.product.as_ref().ok()).map_or(0, self.bytes)
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
pub mod tests;
