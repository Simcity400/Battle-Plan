//! Preparation by segments of a columnar recording.
//!
//! A pool of workers walks each segment ([`Segment`]) in a job of its own, from its restart or
//! from empty tables at its run's start, through the operators of the families ([`Lane`]) the
//! job carries: each hour goes to them as the walker's typed columns ([`crate::columns`]) and
//! the hour's changes ([`State::changes`]), not rebuilt rows. A family whose operator cannot
//! start where a segment starts ([`Operator::starts_at`]) goes on with the operator that
//! walked the segment before it in its branch: that walk hands it on, or parks it, written out
//! to the spool when it can be; a walker's tables are never parked. Each item an operator
//! projects is written to the spool as it comes ([`Item`]), a file of its family's for each
//! segment, and its family's ordered emit reads them back in segment order, a chunk at a time,
//! once the segment's walk filed them, as a task of the same pool, so families emit alongside
//! one another and the walks. Nothing but work to do decides what starts: an emit that can go
//! on first, then help for a walk under way, then the first segment some family has yet to
//! walk. A walk's operators share a
//! [`World`] built for the walk and advanced once an hour before them ([`run_in`]). A family
//! goes on in a branch from where an earlier run of the recording, as it stood then, left it:
//! no walk takes it to the segments before, and its operator takes up there what it carried
//! past the last segment that run held ([`Lane::resumes`], [`Chunk::carry`]).
//!
//! What fails a family is its own ([`Outcome`]). A refusal of the recorded data
//! (`InvalidData`) ends its branch at that hour, as the row path's families do; any other
//! error, or a panic, ends the family from that segment on, its operator and emit never used
//! again. Damage a walk meets loses the segment for the families reading what it covers, and
//! each goes on as its [`Recovery`] says; a damaged restart costs only the walk of the
//! segments before it, measured and held as that walk is handed out again. Every other
//! family, and branch, goes on; a family's failure stands once nothing before it in segment
//! order ended its branch first. The run's own failures, a lack of memory among them and what
//! a family's code says is the run's ([`fatal`]), stop every family, and cancellation leaves
//! what was not emitted pending, neither lost, refused nor failed.

use crate::{
    columnar_frames::{Budget, Damage, DamageScope, Recording, Segment, Walker, schema},
    rows::TableId,
};
use engine_allocation::labels::{self, Kind, Owner};
pub use replay_columnar::{manifest::Manifest, walk::State};
use std::{
    any::Any,
    cell::Cell,
    cmp::Reverse,
    collections::BTreeMap,
    fmt, io,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[cfg(test)]
use std::sync::Condvar;

mod crew;
mod lane;
mod schedule;
mod spool;
mod store;
#[cfg(test)]
mod tests;

pub use replay_spool::{Placeholder, Spooled};

use crew::Board;
use lane::{Context, Erased, Kept, Walking};
use schedule::{Closing, Gone, Plan, Schedule, Status, Task};
use spool::Folder;
use store::{Parked, Store};

/// What a walk's operators share, built for each walk ([`run_in`]) and advanced once an hour,
/// before them, to the state the walker holds. It is a function of that state alone: an
/// operator going on from the segment before meets a world built anew, never one parked. It is
/// the run's own: its failure stops the run, but for damage it names, which loses the segment
/// to every family walking it. A walk's operators may read it on several workers at once.
pub trait World: Sync {
    fn hour(&mut self, state: &State, manifest: &Manifest<'_>) -> io::Result<()>;
}

impl World for () {
    fn hour(&mut self, _: &State, _: &Manifest<'_>) -> io::Result<()> {
        Ok(())
    }
}

/// One hour of a segment's walk.
pub struct Hour<'a, W = ()> {
    pub branch: u64,
    pub hour: i32,
    pub run: u32,
    /// The hour's ordinal in its run.
    pub ordinal: u32,
    /// The walker holds what its restart restored, or empty tables at its run's start, or,
    /// where a family going on past its segment's first hour starts anew, what a restart
    /// before the hour would ([`Resume`]), and this hour: nothing of the hours before. False at
    /// the first hour of a segment whose operator goes on from the segment before in its run,
    /// and where a family goes on from its carry.
    pub starts: bool,
    /// The hour starts a segment, a unit the recording can restart at, or is the first a family
    /// going on past its segment's first hour walks ([`Resume`]).
    pub opens: bool,
    pub state: &'a State,
    pub manifest: &'a Manifest<'a>,
    /// The walk's world at this hour.
    pub world: &'a W,
}

/// Where a walk starts.
#[derive(Clone, Copy)]
pub enum Start<'a> {
    /// A restart: the state as the hour before the walk's first left it.
    Restart(&'a State),
    /// A run's start, from empty tables; `continues` when an earlier run is of its branch.
    Run { continues: bool },
}

/// Where an operator puts the items it projects.
pub trait Sink<T> {
    fn push(&mut self, item: T) -> io::Result<()>;
}

impl<T> Sink<T> for Vec<T> {
    fn push(&mut self, item: T) -> io::Result<()> {
        engine_allocation::reserve(self, 1)?;
        Vec::push(self, item);
        Ok(())
    }
}

/// An item a family's operators project for its ordered emit: written to the spool as it is
/// pushed and read back for the emit, exactly ([`Spooled`]).
pub trait Item: Spooled + Send {}

impl<T: Spooled + Send> Item for T {}

/// A family's work on a segment, and on the segments after it in its branch that it cannot
/// start, reading the walk's world `W`.
///
/// An `InvalidData` error from [`Self::start`], [`Self::cross`], [`Self::hour`] or
/// [`Self::end`] refuses the branch's data from that hour on: what the operator pushed before
/// it reaches the emit, so it pushes an hour's items only once nothing more of that hour can
/// fail. An error naming recorded damage ([`Damage`]) loses the segment; one [`fatal`] made
/// stops the run; any other error, or a panic, ends the family. Its operators are never used
/// again after any of them.
pub trait Operator<W = ()>: Send {
    type Output: Item;

    /// Whether it can start at `start`. When it cannot, the operator that walked the segment
    /// before in its branch (the run's segment before, or the branch's earlier run's last)
    /// goes on through it. Both decide alike.
    fn starts_at(&self, start: Start<'_>) -> bool {
        let _ = start;
        true
    }

    /// The walk's start, before its first hour, `first` of `branch`. What the hour before
    /// held is known here only.
    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        let _ = (start, branch, first);
        Ok(())
    }

    /// It goes on into a later run of its branch, whose first hour, `first`, applies to empty
    /// tables. Called once per run, before that run's first hour.
    fn cross(&mut self, first: i32) -> io::Result<()> {
        let _ = first;
        Ok(())
    }

    fn hour(&mut self, hour: &Hour<'_, W>, out: &mut dyn Sink<Self::Output>) -> io::Result<()>;

    /// The segment's end, after its last hour; it may go on into the segment after.
    fn end(&mut self, out: &mut dyn Sink<Self::Output>) -> io::Result<()> {
        let _ = out;
        Ok(())
    }

    /// The bytes it owns on the heap while parked for the segment after, which it cannot
    /// start; None for an operator that starts every segment and is never parked.
    fn carried(&self) -> Option<usize> {
        None
    }

    /// Writes what it carries into the segment after, whole, for the continuation store to
    /// spill; false when it cannot, and stays resident.
    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        let _ = out;
        Ok(false)
    }

    /// Takes up what [`Self::save`] wrote, in place of what it holds: a new operator then
    /// goes on as the saved one would have.
    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let _ = bytes;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "This operator is never spilled",
        ))
    }
}

/// Items one segment projected for a family, in order; the segment's last chunk ends it.
pub struct Chunk<'a, T> {
    pub segment: &'a Segment,
    pub items: Vec<T>,
    pub last: bool,
    /// On the segment's last chunk, why the family's walk of it ended short, when it did.
    pub ending: Option<&'a Ending>,
    /// On the segment's last chunk, what the family's operator would hand on past it, which it
    /// cannot start: saved past the last segment the recording holds of its branch, for a later
    /// run of the recording grown to go on with ([`Resume::carry`]). None when it starts there
    /// anew, and when the walk of the segment ended short.
    pub carry: Option<Carry<'a>>,
}

/// What a family's operator would hand on past a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carry<'a> {
    /// What [`Operator::save`] wrote of it.
    Saved(&'a [u8]),
    /// Not saved, as within its branch, or because its operator cannot: no later run goes on
    /// from it.
    Unsaved,
}

/// Where a family goes on in a branch whose hours before ordinal `ordinal` of run `run` reached
/// its emit in an earlier run ([`Lane::resumes`]). Its operator starts there anew when it can,
/// as at a restart when that is past its segment's first hour; else it takes up `carry`
/// ([`Operator::load`]), what it would have handed on there ([`Chunk::carry`]).
#[derive(Clone, Copy, Debug)]
pub struct Resume<'a> {
    pub branch: u64,
    pub run: u32,
    pub ordinal: u32,
    pub carry: Option<&'a [u8]>,
}

/// Why a family's segment reached its emit short.
#[derive(Debug)]
pub enum Ending {
    /// It refused its branch's data from `hour` on: its items of the hours before reached
    /// the emit, and nothing of the branch's later segments does.
    Refused { hour: i32, error: io::Error },
    /// None of its items reached the emit; its branch's later segments follow its family's
    /// [`Recovery`].
    Lost(Loss),
}

/// Why a family lost a segment.
#[derive(Clone, Debug)]
pub enum Loss {
    /// Recorded bytes it reads there cannot be read: [`Damage::of`] names their scope.
    Damaged(Arc<io::Error>),
    /// Its operator goes on from segment `.0` of its branch, which it lost.
    After(usize),
}

/// What a family does once it lost a segment of a branch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Recovery {
    /// Nothing of the branch after the loss reaches its emit: what it holds of the branch's
    /// history has a gap it cannot fill.
    #[default]
    Ends,
    /// It starts again at the branch's next segment its operator can start, from that
    /// segment's own state; its emit takes the loss first, and holds nothing across it.
    Resumes,
}

/// A family's own code, by what it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Making an operator: [`Lane::new`]'s `make`.
    Make,
    /// Deciding where its operators start, and both deciding alike.
    Decide,
    Start,
    Cross,
    Hour,
    End,
    /// Writing out its items, by their codec, or its parked operator.
    Save,
    /// Reading them back.
    Load,
    Emit,
}

/// What ended a family: of its segments from `at` on, of every branch, none reached its emit
/// whole, `at` itself at most in part; those before did.
#[derive(Debug)]
pub struct Failure {
    pub at: usize,
    pub operation: Operation,
    /// It panicked; `error` holds what the panic said.
    pub panicked: bool,
    pub error: io::Error,
}

/// A family's segment that reached its emit short.
#[derive(Debug)]
pub struct Ended {
    pub at: usize,
    pub branch: u64,
    pub ending: Ending,
}

/// What became of a family in a run.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Its segments that reached its emit short, in segment order.
    pub ended: Vec<Ended>,
    pub failure: Option<Failure>,
    /// The segments, from the first, it went past: emitted, or passed over once their branch
    /// ended or it failed. Short of every segment only when the run was cancelled: the rest
    /// are pending.
    pub settled: usize,
    /// The most it spooled in one walk of a segment.
    pub largest_output: Option<usize>,
}

/// What a run did, and what became of each family, in the order of their lanes.
#[derive(Debug, Default)]
pub struct Ran {
    pub stats: Stats,
    pub outcomes: Vec<Outcome>,
    /// The most each segment's walks charged their tables, by segment.
    pub tables: Vec<usize>,
    /// Cancellation stopped it.
    pub cancelled: bool,
}

/// `error`, an emit's refusal of its branch's data from `hour` on. An emit's `InvalidData`
/// error naming no hour refuses from its segment's first. The run's own failure ([`fatal`])
/// is no refusal, and comes back as it is.
pub fn refused(hour: i32, error: io::Error) -> io::Error {
    if is_fatal(&error) {
        return error;
    }
    io::Error::new(io::ErrorKind::InvalidData, Refusal { hour, error })
}

/// An emit's refusal, with its hour.
#[derive(Debug)]
struct Refusal {
    hour: i32,
    error: io::Error,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for Refusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// The hour an emit's refusal names, and its own error; one naming none refuses from `first`.
fn dated(error: io::Error, first: i32) -> (i32, io::Error) {
    if !error.get_ref().is_some_and(|inner| inner.is::<Refusal>()) {
        return (first, error);
    }
    match error.into_inner().map(|inner| inner.downcast::<Refusal>()) {
        Some(Ok(refusal)) => (refusal.hour, refusal.error),
        _ => unreachable!("the error holds a refusal"),
    }
}

/// `error`, met by a family's operator or emit in what the run gives it to work with, such as
/// the store its emit writes to: the run's own failure, not the family's. It stops every
/// family, and the run returns `error`.
pub fn fatal(error: io::Error) -> io::Error {
    io::Error::new(error.kind(), Fatal(error))
}

/// Keeps an undated error family-wide even when the lane recognises its payload.
pub fn family_failure(error: io::Error) -> io::Error {
    io::Error::new(error.kind(), FamilyFailure(error))
}

#[derive(Debug)]
struct FamilyFailure(io::Error);

impl fmt::Display for FamilyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for FamilyFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// The run's own failure, met in a family's code.
#[derive(Debug)]
struct Fatal(io::Error);

impl fmt::Display for Fatal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Fatal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Whether [`fatal`] made `error`.
fn is_fatal(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<Fatal>())
}

/// The run's own failure `error` holds, when [`fatal`] made it; else `error`.
fn unfatal(error: io::Error) -> Result<io::Error, io::Error> {
    if !is_fatal(&error) {
        return Err(error);
    }
    match error.into_inner().map(|inner| inner.downcast::<Fatal>()) {
        Some(Ok(fatal)) => Ok(fatal.0),
        _ => unreachable!("the error holds the run's own"),
    }
}

/// A family: its operator for each segment, and its ordered emit, which takes every
/// segment's chunks in segment order, one call at a time. Operators and emits run as tasks
/// of the run's workers, and must never wait for another task of the run: it may need
/// their worker.
pub struct Lane<'a, W = ()> {
    erased: Box<dyn Erased<W> + 'a>,
    refusal: Option<fn(&io::Error) -> bool>,
    /// The tables its operators read; None for every one.
    reads: Option<&'a [TableId]>,
    /// The branches it has nothing left to do on, in ascending order.
    passes: &'a [u64],
    /// Where it goes on in branches an earlier run left, in ascending order of branch.
    resumes: &'a [Resume<'a>],
    recovery: Recovery,
}

impl<'a, W: 'a> Lane<'a, W> {
    pub fn new<O: Operator<W> + 'a>(
        name: &'a str,
        make: impl Fn(&Segment) -> io::Result<O> + Sync + 'a,
        emit: impl FnMut(Chunk<'_, O::Output>) -> io::Result<()> + Send + 'a,
    ) -> Self {
        Self {
            erased: Box::new(lane::Typed::new(name, Box::new(make), Box::new(emit))),
            refusal: None,
            reads: None,
            passes: &[],
            resumes: &[],
            recovery: Recovery::Ends,
        }
    }

    /// Its operators read `tables` alone: damage to another table's units leaves it walking.
    pub fn reads(self, tables: &'a [TableId]) -> Self {
        Self {
            reads: Some(tables),
            ..self
        }
    }

    /// What it does once it lost a segment of a branch; its branch ends there unless told.
    pub fn recovers(self, recovery: Recovery) -> Self {
        Self { recovery, ..self }
    }

    /// Recognises additional branch-local refusal payloads without changing their error kind.
    pub fn refuses(self, refusal: fn(&io::Error) -> bool) -> Self {
        Self {
            refusal: Some(refusal),
            ..self
        }
    }

    fn is_refusal(&self, error: &io::Error) -> bool {
        !error
            .get_ref()
            .is_some_and(|inner| inner.is::<FamilyFailure>())
            && (error.kind() == io::ErrorKind::InvalidData
                || self.refusal.is_some_and(|refusal| refusal(error)))
    }

    /// Its family has nothing left to do on `branches`, in ascending order: no walk takes it
    /// there, and its emit is handed none of their segments.
    pub fn passes(self, branches: &'a [u64]) -> Self {
        Self {
            passes: branches,
            ..self
        }
    }

    /// Its family goes on in each branch of `resumes`, in ascending order and none it passes,
    /// from where an earlier run of the recording as it stood then left it: no walk takes it to
    /// the segments before, and its emit is handed none of the hours before.
    pub fn resumes(self, resumes: &'a [Resume<'a>]) -> Self {
        Self { resumes, ..self }
    }

    /// Whether its operators read table `table`.
    fn reading(&self, table: usize) -> bool {
        self.reads
            .is_none_or(|tables| tables.iter().any(|read| read.index() == table))
    }
}

/// How a run works: on how many workers. Nothing else bounds it: its items wait on disk.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Workers, walking segments and running the families' emits alike.
    pub threads: usize,
}

impl Settings {
    pub fn new(threads: usize) -> Self {
        Self {
            threads: threads.max(1),
        }
    }
}

/// What a run did, and the most it held at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Walks of a segment: one per job, a segment some families walked apart counting again.
    pub walks: usize,
    /// Walks that went on from the walk of the segment before, on its worker.
    pub chained: usize,
    /// Families a walk left for the segment before to hand their operator on.
    pub deferred: usize,
    /// Segments walked or waiting for emit at once.
    pub segments: usize,
    /// The most the spool held on disk at once, items and parked operators, and what it
    /// wrote in all.
    pub spool_bytes: u64,
    pub spooled: u64,
    /// Parked operators' bytes held resident at once, those being parked and written out
    /// included.
    pub carry_bytes: usize,
    /// Bytes of parked operators being written out or read back at once.
    pub carry_scratch: usize,
    pub parked: usize,
    /// Parked operators spilled, and read back.
    pub spilled: usize,
    pub loaded: usize,
    /// Segments walked again, with no family, for the state a damaged restart holds.
    pub derived: usize,
    /// Segments whose own pages a measure read: each once, however many walks begin before
    /// it, but for measures under way at once.
    pub measured: usize,
    /// Walks of a segment begun again for its other families after one panicked.
    pub rewalked: usize,
    /// The walkers' buffers held at once.
    pub walker_bytes: usize,
    /// The most one walk's tables were charged at once.
    pub table_bytes: usize,
}

/// Stops the runs it is handed, from any thread, waking each wait they are in.
#[derive(Default)]
pub struct Cancellation {
    cancelled: AtomicBool,
    runs: Mutex<Vec<Arc<Signal>>>,
    /// Workers waiting for work, for tests to cancel a run while it waits.
    #[cfg(test)]
    idle: (Mutex<usize>, Condvar),
}

impl Cancellation {
    pub fn cancel(&self) {
        let runs = lock(&self.runs);
        // Its runs stop before it reads as cancelled: code that sees it so finds them stopped.
        for signal in runs.iter() {
            signal.cancel();
        }
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Wakes `signal`'s run once cancelled, until dropped.
    fn enter(&self, signal: &Arc<Signal>) -> io::Result<Entered<'_>> {
        let mut runs = lock(&self.runs);
        runs.try_reserve(1).map_err(|_| engine_allocation::oom())?;
        runs.push(Arc::clone(signal));
        if self.is_cancelled() {
            signal.cancel();
        }
        drop(runs);
        Ok(Entered {
            cancellation: self,
            signal: Arc::clone(signal),
        })
    }
}

#[cfg(test)]
impl Cancellation {
    fn idle(&self, waiting: bool) {
        let mut idle = lock(&self.idle.0);
        *idle = if waiting { *idle + 1 } else { *idle - 1 };
        self.idle.1.notify_all();
    }

    /// Waits until `count` workers wait for work.
    fn wait_for_idle(&self, count: usize) {
        self.wait_for_idle_until(|idle| idle >= count);
    }

    /// Waits until `ready` holds of the workers waiting for work, asked again as they change
    /// and whenever [`Self::poke`] says what else it reads did.
    fn wait_for_idle_until(&self, ready: impl Fn(usize) -> bool) {
        let mut idle = lock(&self.idle.0);
        while !ready(*idle) {
            idle = self
                .idle
                .1
                .wait(idle)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Wakes those in [`Self::wait_for_idle_until`] to ask again.
    fn poke(&self) {
        drop(lock(&self.idle.0));
        self.idle.1.notify_all();
    }
}

struct Entered<'c> {
    cancellation: &'c Cancellation,
    signal: Arc<Signal>,
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        lock(&self.cancellation.runs).retain(|held| !Arc::ptr_eq(held, &self.signal));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// A failure inside a run: its own, which stops every family, or a family's own code's.
#[derive(Debug)]
enum Fault {
    Run(io::Error),
    Error(Operation, io::Error),
    /// A panic, with what it said.
    Panic(Operation, String),
}

impl From<io::Error> for Fault {
    fn from(error: io::Error) -> Self {
        Self::Run(error)
    }
}

impl Fault {
    /// `error`, returned by a family's code as `operation`: the run's own when [`fatal`] made it.
    fn of(operation: Operation, error: io::Error) -> Self {
        match unfatal(error) {
            Ok(error) => Self::Run(error),
            Err(error) => Self::Error(operation, error),
        }
    }

    /// The same failure as an error, for the operator whose push met it to return.
    fn echo(&self) -> io::Error {
        match self {
            Self::Run(error) | Self::Error(_, error) => {
                io::Error::new(error.kind(), error.to_string())
            }
            Self::Panic(_, said) => io::Error::other(said.clone()),
        }
    }
}

/// What a panic said.
fn said(payload: &(dyn Any + Send)) -> String {
    match (
        payload.downcast_ref::<&str>(),
        payload.downcast_ref::<String>(),
    ) {
        (Some(said), _) => (*said).to_owned(),
        (_, Some(said)) => said.clone(),
        _ => "A panic said nothing".to_owned(),
    }
}

/// Runs `call`, a family's own code, as `operation`: its panic is the family's.
fn caught<T>(operation: Operation, call: impl FnOnce() -> io::Result<T>) -> Result<T, Fault> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(Fault::of(operation, error)),
        Err(payload) => Err(Fault::Panic(operation, said(&*payload))),
    }
}

/// [`caught`], for the code of a walk's family `index`, which calls back into the run's own:
/// that clears `culprit` while it runs, and its panic goes on as the run's.
fn marked<T>(
    culprit: &Cell<Option<usize>>,
    index: usize,
    operation: Operation,
    call: impl FnOnce() -> io::Result<T>,
) -> Result<T, Fault> {
    culprit.set(Some(index));
    let result = catch_unwind(AssertUnwindSafe(call));
    let blamed = culprit.replace(None);
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(Fault::of(operation, error)),
        Err(payload) if blamed.is_some() => Err(Fault::Panic(operation, said(&*payload))),
        Err(payload) => resume_unwind(payload),
    }
}

/// A run's schedule, and the board its workers wait on, helping the walks under way.
struct Signal {
    schedule: Mutex<Schedule>,
    board: Board,
    stopped: AtomicBool,
    cancelled: AtomicBool,
}

impl Signal {
    fn lock(&self) -> MutexGuard<'_, Schedule> {
        lock(&self.schedule)
    }

    /// Wakes the workers waiting, once what the schedule may hand out has changed.
    fn changed(&self) {
        self.board.changed();
    }

    /// Keeps the run's first failure and stops every worker; what fails once the run is
    /// cancelled only stops it.
    fn fail(&self, error: io::Error) {
        // One not kept drops once the lock is let go: a family may have made it.
        let spare = {
            let mut schedule = self.lock();
            if self.cancelled.load(Ordering::SeqCst) || schedule.failure.is_some() {
                Some(error)
            } else {
                schedule.failure = Some(error);
                None
            }
        };
        drop(spare);
        self.stop();
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.stop();
    }

    /// Stops every worker: what tasks finish since settles nothing more of any family.
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.lock().halted = true;
        self.changed();
    }
}

/// Stops the run when its thread unwinds from a panic, so no other thread waits for it;
/// the scope then joins every worker and goes on with the panic.
struct Unwinding<'s>(&'s Signal);

impl Drop for Unwinding<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0
                .lock()
                .failure
                .get_or_insert_with(|| io::Error::other("A run's worker panicked"));
            self.0.stop();
        }
    }
}

/// A segment's walk: the families it carries, each with the operator the walk before handed
/// on, if any.
struct Job<'r, W> {
    at: usize,
    /// The segment its walk begins at, as its share was measured.
    from: usize,
    lanes: Vec<(usize, Option<Box<dyn Walking<W> + 'r>>)>,
    /// The state holds the tables the walk of the segment before, of the same run, left.
    hot: bool,
}

/// A family's operator in a walk: `cold` when it started at the walk's first hour.
struct Attached<'r, W> {
    lane: usize,
    operator: Box<dyn Walking<W> + 'r>,
    cold: bool,
    /// Its walk of the segment began: it started, or crossed into its run.
    begun: bool,
    /// Hours of the segment it saw before another family's panic began the walk again.
    seen: u32,
    /// Hours of this walk of the segment it went through.
    through: u32,
    /// Nanoseconds its last hour took.
    cost: u64,
    /// It goes on into the segment after, which it cannot start.
    hands: bool,
    /// The ordinal it joins the walk at, past the segment's first hour where its family goes on
    /// there ([`Resume`]).
    joins: u32,
    /// Joining past the segment's first hour: what its family carries there, kept until the walk
    /// holds the hour before, where it decides ([`Run::decide`]).
    pending: Option<Option<Vec<u8>>>,
    /// What it would hand on past the last segment of its branch, which it cannot start.
    kept: Option<Kept>,
    fate: Option<Fate>,
}

impl<'r, W> Attached<'r, W> {
    fn new(lane: usize, operator: Box<dyn Walking<W> + 'r>, cold: bool) -> Self {
        Self {
            lane,
            operator,
            cold,
            begun: false,
            seen: 0,
            through: 0,
            cost: 0,
            hands: false,
            joins: 0,
            pending: None,
            kept: None,
            fate: None,
        }
    }

    fn live(&self) -> bool {
        self.fate.is_none()
    }

    /// The fault a call of its family's code met, `called`'s, or else one its items' sink
    /// kept though the call went on past it, which ends its walk all the same.
    fn met(&mut self, called: Result<(), Fault>) -> Option<Fault> {
        match called {
            Ok(()) => self.operator.fault(),
            Err(fault) => Some(fault),
        }
    }

    /// Damage to what it reads from ordinal `from` on loses it the segment, but for a refusal
    /// it made before, which the damage does not explain, or its failure.
    fn lose(&mut self, fate: Fate, from: u32) {
        let stands = match &self.fate {
            Some(Fate::Failed { .. } | Fate::Lost(_)) => true,
            Some(Fate::Refused { ordinal, .. }) => *ordinal < from,
            Some(Fate::Table) | None => false,
        };
        if !stands {
            self.fate = Some(fate);
        }
    }

    /// Whether its walk goes on: its operator walks, or what it pushed before its refusal waits
    /// for the walk to check the pages it came from.
    fn walks(&self) -> bool {
        matches!(self.fate, None | Some(Fate::Refused { .. }))
    }
}

/// A family's hour in a walk's hour, with what its call came to.
struct Turn<'t, 'r, W> {
    /// Its place in the walk.
    index: usize,
    attached: &'t mut Attached<'r, W>,
    called: Option<Called>,
}

/// What a family's hour returned, or its panic, the family's own or the run's.
enum Called {
    Returned(Result<(), Fault>),
    Panicked {
        family: bool,
        payload: Box<dyn Any + Send>,
    },
}

/// The workers families whose last hours took `costs` keep busy: sum over largest, rounded up.
fn width(costs: impl Iterator<Item = u64>) -> usize {
    let (sum, most, count) = costs.fold((0_u64, 0_u64, 0_usize), |(sum, most, count), cost| {
        (sum.saturating_add(cost), most.max(cost), count + 1)
    });
    if most == 0 {
        return count;
    }
    usize::try_from(sum.div_ceil(most)).map_or(count, |width| width.min(count))
}

/// `operator`'s `hour` of segment `at`, on this worker.
fn hour_on<W>(
    operator: &mut (dyn Walking<W> + '_),
    at: usize,
    hour: &Hour<'_, W>,
    folder: &Folder,
) -> Called {
    // Cleared while the run's own code it calls back into runs.
    let culprit = Cell::new(Some(0));
    let context = Context {
        folder,
        culprit: &culprit,
    };
    match catch_unwind(AssertUnwindSafe(|| operator.hour(at, hour, &context))) {
        Ok(result) => Called::Returned(result.map_err(|error| Fault::of(Operation::Hour, error))),
        Err(payload) => Called::Panicked {
            family: culprit.get().is_some(),
            payload,
        },
    }
}

/// The first ordinal of its run that damage `error` covers; its run's first when it names none.
fn damaged_from(error: &io::Error) -> u32 {
    match Damage::of(error).map(|damage| &damage.scope) {
        Some(DamageScope::Page { first, .. }) => *first,
        Some(DamageScope::Extent(extent)) => extent.first,
        _ => 0,
    }
}

/// How a family's walk of a segment ended short.
enum Fate {
    /// It refused its branch from `hour`, ordinal `ordinal` of its run, on: what it pushed
    /// before stands.
    Refused {
        hour: i32,
        ordinal: u32,
        error: io::Error,
    },
    /// A table it reads was lost; the walk names the damage once it ends, or a panic begins
    /// it again.
    Table,
    Lost(Loss),
    Failed {
        operation: Operation,
        panicked: bool,
        error: io::Error,
    },
}

/// What a family made, given up under the schedule's lock: its errors' drops are its own
/// code, so it is dropped only once the lock is let go ([`Gone`]). Set aside without a
/// failing allocation: one that failed would drop it there.
enum Waste {
    Failure(Failure),
    Ending(Ending),
    Loss(Loss),
    Fault(Fault),
    Fate(Fate),
}

/// What a walk's start found.
enum Begun {
    /// The state its first hour applies to, after walking this many segments for it.
    Ready(usize),
    /// The restart it begins at is damaged: the walk goes back to begin before it.
    Back,
    /// Nothing says what its first hour applies to.
    Lost(io::Error),
    Stopped,
}

/// Builds a walk's world for its segment and the lanes whose operators it visits.
pub type Worlds<'w, W> = dyn Fn(&Segment, &[usize]) -> io::Result<W> + Sync + 'w;

/// What a run's workers share.
struct Run<'r, 'a, W> {
    recording: &'r Recording,
    segments: &'r [Segment],
    plan: Plan,
    lanes: &'r [Lane<'a, W>],
    worlds: &'r Worlds<'r, W>,
    signal: &'r Signal,
    /// What the walkers' buffers hold, counted: nothing limits it.
    budget: Arc<Budget>,
    folder: &'r Folder,
    store: Store<'r, W>,
    /// What the families of walks given back after a panic did of their segment, by segment,
    /// for the walk of it handed out again.
    resumed: Mutex<Vec<(usize, Attached<'r, W>)>>,
    /// Where each family going on in a branch an earlier run left goes on, by segment then
    /// family: the ordinal it joins the walk at, and its carry.
    joins: BTreeMap<(usize, usize), (u32, Option<&'a [u8]>)>,
    #[cfg(test)]
    cancellation: &'r Cancellation,
}

impl<'r, 'a, W: World> Run<'r, 'a, W> {
    fn erased(&self, lane: usize) -> &'r (dyn Erased<W> + 'r) {
        let lanes: &'r [Lane<'a, W>] = self.lanes;
        &*lanes[lane].erased
    }

    fn work(&self) {
        let _unwinding = Unwinding(self.signal);
        let _memory = labels::enter(Owner::RUN, Kind::Executor);
        let walking = labels::enter(Owner::RUN, Kind::Walker);
        let mut walker = match Walker::without_waiting(self.recording, &self.budget) {
            Ok(walker) => walker,
            Err(error) => return self.signal.fail(error),
        };
        let mut state = match schema().and_then(State::new) {
            Ok(state) => state,
            Err(error) => return self.signal.fail(error),
        };
        drop(walking);
        loop {
            let task = {
                let mut schedule = self.signal.lock();
                loop {
                    // Read before asking: a change since sends this worker back to ask again.
                    let seen = self.signal.board.changes();
                    if schedule.failure.is_some()
                        || self.signal.stopped.load(Ordering::SeqCst)
                        || schedule.finished()
                    {
                        return;
                    }
                    // The emits take segments in order, so the earliest walk is helped first.
                    let front = self.signal.board.front();
                    let picked =
                        schedule.pick(&self.plan, front.is_none(), &mut |reads| walker.hold(reads));
                    match picked {
                        Ok(Some(task)) => {
                            if matches!(task, Task::Emit { .. }) && walker.holds() {
                                // An emit may take long: its share goes to another's walk.
                                walker.park();
                                self.signal.changed();
                            }
                            break task;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            drop(schedule);
                            return self.signal.fail(error);
                        }
                    }
                    if let Some(front) = front {
                        if walker.holds() {
                            walker.park();
                            self.signal.changed();
                        }
                        drop(schedule);
                        front.help(&self.signal.board, seen);
                        schedule = self.signal.lock();
                        continue;
                    }
                    if walker.holds() {
                        // Its share goes back before it waits, for another's walk.
                        walker.park();
                        self.signal.changed();
                        continue;
                    }
                    drop(schedule);
                    self.help(seen);
                    schedule = self.signal.lock();
                }
            };
            let done = match task {
                Task::Measure { at } => self.measure(&mut walker, at),
                Task::Emit { lane, at } => self.emit(lane, at),
                Task::Walk { at, from, lanes } => {
                    self.walk(&mut walker, &mut state, at, from, lanes)
                }
            };
            self.signal.lock().done();
            self.signal.changed();
            if let Err(error) = done {
                return self.signal.fail(error);
            }
        }
    }

    /// Helps the walks under way until what the schedule may hand out changed past `seen`.
    fn help(&self, seen: u64) {
        loop {
            #[cfg(test)]
            self.cancellation.idle(true);
            let crew = self.signal.board.next(seen);
            #[cfg(test)]
            self.cancellation.idle(false);
            let Some(crew) = crew else {
                return;
            };
            crew.help(&self.signal.board, seen);
        }
    }

    /// Whether the run stopped: what its tasks meet since is no family's, and what they did
    /// not settle stays pending.
    fn stopped(&self) -> bool {
        self.signal.stopped.load(Ordering::SeqCst)
    }

    /// Measures what walking segment `at` reads, from the segment its walk begins at: of that
    /// one's restart, and of each segment's own pages from there, what is not measured yet.
    fn measure(&self, walker: &mut Walker<'_>, at: usize) -> io::Result<()> {
        let _span = replay_frame::timeline::span("measure").at(at);
        let _memory = labels::enter(Owner::RUN, Kind::Walker);
        let (from, restart, pages) = self.signal.lock().unmeasured(at)?;
        let restart = if restart {
            Some(walker.measure_restart(&self.segments[from])?)
        } else {
            None
        };
        let mut read = engine_allocation::vec(pages.len())?;
        for page in pages {
            read.push((page, walker.measure_pages(&self.segments[page])?));
        }
        self.signal.lock().measured(at, from, restart, &read);
        self.signal.changed();
        Ok(())
    }

    /// Hands segment `at`'s items to `lane`'s emit, read back from the spool: the emit's
    /// refusal ends its branch, its other failures end its family.
    fn emit(&self, lane: usize, at: usize) -> io::Result<()> {
        let _span = replay_frame::timeline::span("emit")
            .at(at)
            .lane(self.erased(lane).name());
        let emitted = self.erased(lane).emit(at, &self.segments[at]);
        let planned = *self.plan.get(at);
        let mut gone = Gone::default();
        {
            let mut schedule = self.signal.lock();
            let plan = &self.plan;
            match emitted {
                Ok(ending) => {
                    schedule.emitted(plan, lane, at, planned.branch, ending, &mut gone)?;
                }
                Err(Fault::Run(error)) => return Err(error),
                // What an emit meets once the run is cancelled is no family's.
                Err(fault) if self.signal.cancelled.load(Ordering::SeqCst) => {
                    schedule.unemit(lane);
                    gone.waste.push(Waste::Fault(fault));
                }
                Err(Fault::Error(Operation::Emit, error))
                    if self.lanes[lane].is_refusal(&error) =>
                {
                    let (hour, error) = dated(error, planned.first);
                    let ending = Ending::Refused { hour, error };
                    // Its branch ends before its emit moves on past a failure there.
                    if let Err(failed) = schedule.close(plan, lane, Closing::Branch(at), &mut gone)
                    {
                        gone.waste.push(Waste::Ending(ending));
                        return Err(failed);
                    }
                    schedule.emitted(plan, lane, at, planned.branch, Some(ending), &mut gone)?;
                }
                Err(fault) => {
                    schedule.unemit(lane);
                    let failure = failure_at(at, fault)?;
                    schedule.fail(&self.plan, lane, failure, &mut gone)?;
                }
            }
        }
        self.signal.changed();
        self.drop_gone(gone);
        Ok(())
    }

    /// Segment `at`'s walk of `lanes` from `from`, and every walk it goes on into.
    fn walk(
        &self,
        walker: &mut Walker<'_>,
        state: &mut State,
        at: usize,
        from: usize,
        lanes: Vec<usize>,
    ) -> io::Result<()> {
        let mut job = Some(Job {
            at,
            from,
            lanes: engine_allocation::collect_vec(lanes.into_iter().map(|lane| (lane, None)))?,
            hot: false,
        });
        while let Some(next) = job.take() {
            job = self.segment(walker, state, next)?;
        }
        Ok(())
    }

    /// Parks `operator` of `lane`, holding `bytes`, for segment `at`, and writes out, with no
    /// lock held, the parked operators that can be, which the store takes out as it parks: a
    /// walk finding one being written leaves it until it lands, and
    /// none takes its segment meanwhile. A family whose parked operator failed as it was
    /// written out fails at that segment before any walk may take it again, or find it gone:
    /// until then it reads as being written, even to a walk holding the segment already.
    fn park(
        &self,
        at: usize,
        lane: usize,
        operator: Box<dyn Walking<W> + 'r>,
        bytes: usize,
    ) -> io::Result<()> {
        let mut out = self.store.park(at, lane, operator, bytes, self.lanes)?;
        while !out.is_empty() {
            let cells = engine_allocation::collect_vec(out.iter().map(|&(cell, _, _)| cell))?;
            self.signal.lock().write(&cells);
            let written = self.store.write(out, self.folder);
            let mut gone = Gone::default();
            let mut victims = match written {
                Ok(victims) => victims,
                Err(error) => {
                    self.signal.lock().written(&cells);
                    return Err(error);
                }
            };
            victims.reverse();
            {
                let mut schedule = self.signal.lock();
                if !self.stopped() {
                    while let Some((cell, victim, fault)) = victims.pop() {
                        let failure = failure_at(cell, fault)?;
                        self.failed(&mut schedule, victim, failure, false, &mut gone)?;
                    }
                }
                schedule.written(&cells);
            }
            self.signal.changed();
            self.drop_gone(gone);
            drop(victims);
            // Those that cannot spill stay resident.
            out = self.store.evict()?;
        }
        Ok(())
    }

    /// What is parked for `lane` of segment `at`, if anything.
    fn parked(&self, at: usize, lane: usize) -> Result<Option<Parked<'r, W>>, Fault> {
        self.store
            .take(at, lane, &self.segments[at], self.erased(lane), self.folder)
    }

    /// A new operator of `lane` for segment `at`, when it can start there at `start`; one
    /// that cannot start a segment that `leads` its branch fails.
    fn fresh(
        &self,
        at: usize,
        lane: usize,
        start: Start<'_>,
        leads: bool,
    ) -> Result<Option<Box<dyn Walking<W> + 'r>>, Fault> {
        let erased = self.erased(lane);
        let operator = caught(Operation::Make, || {
            erased.fresh(lane, &self.segments[at], self.folder, None)
        })?;
        if caught(Operation::Decide, || Ok(operator.starts_at(start)))? {
            return Ok(Some(operator));
        }
        if leads {
            return Err(Fault::Error(
                Operation::Decide,
                invalid(format!(
                    "{} cannot start segment {at}, which no segment of its branch precedes",
                    erased.name()
                )),
            ));
        }
        Ok(None)
    }

    /// The operator of `lane` for segment `at`, where its family goes on from ordinal `joins`
    /// with `carry` ([`Resume`]): a new one when it can start there at `start`, else one taking
    /// up its carry. One that can do neither fails: no segment before is walked for it. One
    /// joining past the segment's first hour decides once the walk holds the hour before.
    fn joined(
        &self,
        at: usize,
        lane: usize,
        (joins, carry): (u32, Option<&[u8]>),
        start: Start<'_>,
    ) -> Result<Attached<'r, W>, Fault> {
        let segment = &self.segments[at];
        let erased = self.erased(lane);
        let mut operator = caught(Operation::Make, || {
            erased.fresh(lane, segment, self.folder, None)
        })?;
        if joins > segment.first {
            let kept = caught(Operation::Make, || {
                carry.map(engine_allocation::copy_vec).transpose()
            })?;
            let mut attached = Attached::new(lane, operator, false);
            attached.joins = joins;
            attached.pending = Some(kept);
            return Ok(attached);
        }
        let cold = caught(Operation::Decide, || Ok(operator.starts_at(start)))?;
        if !cold {
            let Some(carry) = carry else {
                return Err(Fault::Error(
                    Operation::Decide,
                    invalid(format!(
                        "{} cannot start segment {at}, where it goes on, and carries nothing there",
                        erased.name()
                    )),
                ));
            };
            caught(Operation::Load, || operator.load(carry))?;
        }
        let mut attached = Attached::new(lane, operator, cold);
        attached.joins = joins;
        Ok(attached)
    }

    /// Decides for `attached`, once the walk holds in `state` the hour before it joins, whether
    /// its operator starts anew there or takes up `carry`, what its family would have handed on
    /// there. One that can do neither fails.
    fn decide(
        &self,
        attached: &mut Attached<'r, W>,
        carry: Option<Vec<u8>>,
        state: &State,
    ) -> Result<(), Fault> {
        let operator = &mut attached.operator;
        let cold = caught(Operation::Decide, || {
            Ok(operator.starts_at(Start::Restart(state)))
        })?;
        if !cold {
            let Some(carry) = carry else {
                return Err(Fault::Error(
                    Operation::Decide,
                    invalid(format!(
                        "{} cannot start where it goes on, and carries nothing there",
                        self.erased(attached.lane).name()
                    )),
                ));
            };
            caught(Operation::Load, || operator.load(&carry))?;
        }
        attached.cold = cold;
        Ok(())
    }

    /// Puts into `state` what segment `at`'s first hour applies to, from `from`, the segment
    /// its walk begins at: `from`'s restart, or its run's empty tables, then the walk of the
    /// segments from `from` to `at`, which leaves the tables `at`'s own restart holds. The
    /// walker's share was measured and taken for all of it.
    fn begin(
        &self,
        walker: &mut Walker<'_>,
        state: &mut State,
        at: usize,
        from: usize,
    ) -> io::Result<Begun> {
        match walker.begin(&self.segments[from], state) {
            Ok(()) => {}
            // A segment with a restart follows another of its run.
            Err(error) if Damage::of(&error).is_some() => {
                return Ok(if self.segments[from].restart.is_some() {
                    Begun::Back
                } else {
                    Begun::Lost(error)
                });
            }
            Err(error) => return Err(error),
        }
        let stopped = &self.signal.stopped;
        for earlier in from..at {
            let walked = walker.walk_isolated(&self.segments[earlier], state, |_, _, _, _| {
                Ok(!stopped.load(Ordering::Relaxed))
            });
            match walked {
                Ok(true) => {}
                Ok(false) => return Ok(Begun::Stopped),
                Err(error) if Damage::of(&error).is_some() => return Ok(Begun::Lost(error)),
                Err(error) => return Err(error),
            }
        }
        Ok(Begun::Ready(at - from))
    }

    /// Gives segment `at`'s walk of `lanes`, which could not restore `from`'s restart, back,
    /// to begin before `from` once what that walk reads is measured and its share taken, as
    /// no task waits for a share: the operators handed on to it wait parked for it, and its
    /// families' cells are open again.
    fn back(
        &self,
        at: usize,
        from: usize,
        lanes: Vec<(usize, Option<Box<dyn Walking<W> + 'r>>)>,
    ) -> io::Result<()> {
        let mut held = engine_allocation::vec(lanes.len())?;
        // Families failing to park their operator, and whether the walk holds the segment.
        let mut failures = Vec::new();
        for (lane, handed) in lanes {
            held.push(lane);
            let Some(operator) = handed else {
                continue;
            };
            let fault = match caught(Operation::Save, || Ok(operator.carried())) {
                Ok(Some(bytes)) => {
                    self.park(at, lane, operator, bytes)?;
                    continue;
                }
                Ok(None) => Fault::Error(
                    Operation::Save,
                    invalid(format!(
                        "{} cannot be parked for segment {at}: its operator does not say what it \
                         holds",
                        self.erased(lane).name()
                    )),
                ),
                Err(fault) => fault,
            };
            engine_allocation::reserve(&mut failures, 1)?;
            failures.push((lane, failure_at(at, fault)?, true));
        }
        // Taken one by one under the lock: those left drop once it is let go.
        failures.reverse();
        let mut gone = Gone::default();
        {
            let mut schedule = self.signal.lock();
            if self.stopped() {
                return Ok(());
            }
            while let Some((lane, failure, holding)) = failures.pop() {
                self.failed(&mut schedule, lane, failure, holding, &mut gone)?;
            }
            schedule.derive(&self.plan, at, from, &held, &mut gone)?;
        }
        self.signal.changed();
        self.drop_gone(gone);
        Ok(())
    }

    /// Gives segment `at`'s walk back after a family's panic, as [`Self::back`] does, once
    /// beginning it again from `from` takes a walk of the segments before that its share does
    /// not hold: each family of `walking` keeps what it did of the segment for the walk handed
    /// out again, as it would have once that walk had begun cold, and those of `failures`,
    /// which failed before it walked, fail. A family whose walk ended failed or lost keeps only
    /// its fate: what it projected, none of which reaches its emit, is given back now. It
    /// derived `derived` segments, and began again `rewalked` times.
    fn resume(
        &self,
        at: usize,
        from: usize,
        mut walking: Vec<Attached<'r, W>>,
        mut failures: Vec<(usize, Failure)>,
        (derived, rewalked): (usize, usize),
    ) -> io::Result<()> {
        for attached in walking.iter_mut().filter(|attached| !attached.walks()) {
            attached.operator.discard(self.folder);
        }
        let mut held = engine_allocation::vec(walking.len())?;
        {
            let mut resumed = lock(&self.resumed);
            engine_allocation::reserve(&mut *resumed, walking.len())?;
            for attached in walking {
                held.push(attached.lane);
                resumed.push((at, attached));
            }
        }
        let mut gone = Gone::default();
        {
            let mut schedule = self.signal.lock();
            if self.stopped() {
                return Ok(());
            }
            schedule.stats.derived += derived;
            schedule.stats.rewalked += rewalked;
            failures.reverse();
            while let Some((lane, failure)) = failures.pop() {
                self.failed(&mut schedule, lane, failure, true, &mut gone)?;
            }
            schedule.derive(&self.plan, at, from, &held, &mut gone)?;
        }
        self.signal.changed();
        self.drop_gone(gone);
        Ok(())
    }

    /// What a walk of segment `at` given back after a panic left of the families of `lanes`,
    /// for this walk of it to go on with.
    fn resumed(
        &self,
        at: usize,
        lanes: &[(usize, Option<Box<dyn Walking<W> + 'r>>)],
    ) -> io::Result<Vec<Attached<'r, W>>> {
        // Before the lock, so what it took drops after it, should taking more fail.
        let mut taken = Vec::new();
        let mut resumed = lock(&self.resumed);
        for &(lane, _) in lanes {
            let Some(index) = resumed
                .iter()
                .position(|(held, attached)| *held == at && attached.lane == lane)
            else {
                continue;
            };
            engine_allocation::reserve(&mut taken, 1)?;
            taken.push(resumed.swap_remove(index).1);
        }
        Ok(taken)
    }

    /// Drops what waits for `lane`'s walk of segment `at`: the operator parked for it, and what
    /// a walk of it given back after a panic left.
    fn forget(&self, at: usize, lane: usize) {
        self.store.discard(at, lane);
        let resumed = {
            let mut resumed = lock(&self.resumed);
            resumed
                .iter()
                .position(|(held, attached)| *held == at && attached.lane == lane)
                .map(|index| resumed.swap_remove(index).1)
        };
        if let Some(mut attached) = resumed {
            attached.operator.discard(self.folder);
        }
    }

    /// The tables a walk lost to their own damage, `tables`, are lost to the families of
    /// `walking` reading them.
    fn lose_tables(&self, walking: &mut [Attached<'_, W>], tables: Vec<(usize, io::Error)>) {
        for (table, error) in tables {
            let from = damaged_from(&error);
            let loss = Loss::Damaged(Arc::new(error));
            for attached in walking.iter_mut() {
                if self.lanes[attached.lane].reading(table) {
                    attached.lose(Fate::Lost(loss.clone()), from);
                }
            }
        }
    }

    /// What `fault`, met by `attached` at `hour`, ordinal `ordinal` of its run, does to its
    /// walk; the run's own failures, a lack of memory among them, stop the run.
    fn fate(
        &self,
        fault: Fault,
        attached: &mut Attached<'_, W>,
        (hour, ordinal): (i32, u32),
    ) -> io::Result<Fate> {
        // What its items' sink met is not the operator's own.
        let fault = attached.operator.fault().unwrap_or(fault);
        let reading = |operation: Operation| {
            matches!(
                operation,
                Operation::Start | Operation::Cross | Operation::Hour | Operation::End
            )
        };
        Ok(match fault {
            Fault::Run(error) => return Err(error),
            Fault::Panic(operation, said) => Fate::Failed {
                operation,
                panicked: true,
                error: io::Error::other(said),
            },
            Fault::Error(_, error) if error.kind() == io::ErrorKind::OutOfMemory => {
                return Err(error);
            }
            // Its read of the recording met damage.
            Fault::Error(operation, error)
                if reading(operation) && Damage::of(&error).is_some() =>
            {
                Fate::Lost(Loss::Damaged(Arc::new(error)))
            }
            Fault::Error(operation, error)
                if reading(operation) && self.lanes[attached.lane].is_refusal(&error) =>
            {
                Fate::Refused {
                    hour,
                    ordinal,
                    error,
                }
            }
            Fault::Error(operation, error) => Fate::Failed {
                operation,
                panicked: false,
                error,
            },
        })
    }

    /// Files what `attached`'s hour came to; the `first` family to panic hands its panic back.
    fn settle(
        &self,
        attached: &mut Attached<'r, W>,
        called: Called,
        first: bool,
        (when, hours): ((i32, u32), u32),
    ) -> io::Result<Option<Box<dyn Any + Send>>> {
        match called {
            Called::Returned(called) => {
                attached.through = hours + 1;
                if let Some(fault) = attached.met(called) {
                    attached.fate = Some(self.fate(fault, attached, when)?);
                }
                Ok(None)
            }
            // The run's own panic goes on as the run's.
            Called::Panicked {
                family: false,
                payload,
            } => resume_unwind(payload),
            Called::Panicked { payload, .. } if first => Ok(Some(payload)),
            // Another family panicking at the same hour fails there too.
            Called::Panicked { payload, .. } => {
                attached.through = hours + 1;
                let fault = Fault::Panic(Operation::Hour, said(&*payload));
                attached.fate = Some(self.fate(fault, attached, when)?);
                Ok(None)
            }
        }
    }

    /// Family `lane` loses segment `at`, which its walk holds, for `loss`: the loss is filed
    /// for its emit, and its branch ends there, or the operators waiting to go on from it are
    /// lost with it, as its recovery says.
    fn lost(
        &self,
        schedule: &mut Schedule,
        at: usize,
        lane: usize,
        loss: Loss,
        gone: &mut Gone,
    ) -> io::Result<()> {
        if schedule.status(at, lane) == Status::Abandoned {
            gone.waste.push(Waste::Loss(loss));
            return schedule.abandoned(&self.plan, at, lane, gone);
        }
        let erased = self.erased(lane);
        erased.settle(at, Ending::Lost(loss));
        schedule.settle(at, lane, Status::Lost);
        match self.lanes[lane].recovery {
            Recovery::Ends => schedule.close(&self.plan, lane, Closing::Branch(at), gone),
            Recovery::Resumes => {
                let mut from = at;
                while let Some(later) = schedule.waiting_after(&self.plan, from, lane) {
                    erased.settle(later, Ending::Lost(Loss::After(from)));
                    schedule.settle(later, lane, Status::Lost);
                    from = later;
                }
                Ok(())
            }
        }
    }

    /// Family `lane` fails at the failure's segment, unless what comes before it in segment
    /// order ends it there first ([`Schedule::fail`]). A walk `holding` that segment gives it
    /// back.
    fn failed(
        &self,
        schedule: &mut Schedule,
        lane: usize,
        failure: Failure,
        holding: bool,
        gone: &mut Gone,
    ) -> io::Result<()> {
        let at = failure.at;
        schedule.fail(&self.plan, lane, failure, gone)?;
        if holding && schedule.status(at, lane) == Status::Abandoned {
            schedule.abandoned(&self.plan, at, lane, gone)?;
        }
        Ok(())
    }

    /// Drops, once the schedule is let go, what the cells a closure covered filed for their
    /// emits and what waited for their walks, and what families made that was given up.
    fn drop_gone(&self, gone: Gone) {
        let Gone { cells, waste } = gone;
        for (lane, at, filed) in cells {
            if filed {
                self.erased(lane).discard(at);
            }
            self.forget(at, lane);
        }
        for waste in waste {
            match waste {
                Waste::Failure(failure) => drop(failure),
                Waste::Ending(ending) => drop(ending),
                Waste::Loss(loss) => drop(loss),
                Waste::Fault(fault) => drop(fault),
                Waste::Fate(fate) => drop(fate),
            }
        }
    }

    /// [`Self::walk_segment`], recording the most the walk charged its tables however it ends.
    fn segment(
        &self,
        walker: &mut Walker<'_>,
        state: &mut State,
        job: Job<'r, W>,
    ) -> io::Result<Option<Job<'r, W>>> {
        let at = job.at;
        let mut span = replay_frame::timeline::span("walk").at(at);
        let _memory = labels::enter(Owner::RUN, Kind::Walker);
        // A cold walk begins by replacing the tables the walk before left.
        if !job.hot {
            state.forget_charge_peak();
        }
        let mut charged = 0;
        let next = self.walk_segment(walker, state, job, &mut charged);
        charged = charged.max(state.take_charge_peak());
        span.bytes(charged);
        self.signal.lock().charged(at, charged);
        next
    }

    /// Walks `job`'s segment through its families' operators, files what they projected,
    /// with why a family's walk ended short, and hands on or parks the operators that cannot
    /// start the segment after in their branch: the walk it goes on into, when that segment
    /// may start now. `charged` takes the peak of each state it rebuilds.
    fn walk_segment(
        &self,
        walker: &mut Walker<'_>,
        state: &mut State,
        job: Job<'r, W>,
        charged: &mut usize,
    ) -> io::Result<Option<Job<'r, W>>> {
        let Job {
            at,
            from,
            lanes,
            hot,
        } = job;
        let segment = &self.segments[at];
        let planned = *self.plan.get(at);
        let mut derived = 0;
        if !hot {
            match self.begin(walker, state, at, from) {
                Ok(Begun::Ready(walks)) => derived = walks,
                Ok(Begun::Stopped) => return Ok(None),
                Err(_) if self.stopped() => return Ok(None),
                Err(error) => return Err(error),
                Ok(Begun::Back) => {
                    self.back(at, from, lanes)?;
                    return Ok(None);
                }
                // No family can start, or go on, where nothing says what the tables hold.
                Ok(Begun::Lost(damage)) => {
                    let loss = Loss::Damaged(Arc::new(damage));
                    let mut gone = Gone::default();
                    {
                        let mut schedule = self.signal.lock();
                        if self.stopped() {
                            return Ok(None);
                        }
                        for &(lane, _) in &lanes {
                            self.lost(&mut schedule, at, lane, loss.clone(), &mut gone)?;
                        }
                    }
                    self.signal.changed();
                    for (lane, handed) in lanes {
                        drop(handed);
                        self.forget(at, lane);
                    }
                    self.drop_gone(gone);
                    return Ok(None);
                }
            }
        }
        let start = match segment.restart {
            Some(_) => Start::Restart(&*state),
            None => Start::Run {
                continues: planned.before.is_some(),
            },
        };
        let mut walking: Vec<Attached<'r, W>> = engine_allocation::vec(lanes.len())?;
        let mut refusing = Vec::new();
        // Families whose parked operator is being written out, left until it lands.
        let mut landing = Vec::new();
        // Families failing before they walk, the segment held for them.
        let mut failures: Vec<(usize, Failure)> = Vec::new();
        let mut resumed = self.resumed(at, &lanes)?;
        for (lane, handed) in lanes {
            if let Some(index) = resumed.iter().position(|attached| attached.lane == lane) {
                walking.push(resumed.swap_remove(index));
                continue;
            }
            if let Some(&join) = self.joins.get(&(at, lane)) {
                match self.joined(at, lane, join, start) {
                    Ok(attached) => walking.push(attached),
                    Err(fault) => {
                        engine_allocation::reserve(&mut failures, 1)?;
                        failures.push((lane, failure_at(at, fault)?));
                    }
                }
                continue;
            }
            let carried = match handed {
                Some(operator) => Ok(Some(Parked::Ready(operator))),
                None => self.parked(at, lane),
            };
            let fresh = match carried {
                Ok(Some(Parked::Ready(operator))) => {
                    walking.push(Attached::new(lane, operator, false));
                    continue;
                }
                Ok(Some(Parked::Writing)) => {
                    engine_allocation::reserve(&mut landing, 1)?;
                    landing.push(lane);
                    continue;
                }
                Ok(None) => self.fresh(at, lane, start, planned.before.is_none()),
                Err(fault) => Err(fault),
            };
            match fresh {
                Ok(Some(operator)) => walking.push(Attached::new(lane, operator, true)),
                Ok(None) => {
                    engine_allocation::reserve(&mut refusing, 1)?;
                    refusing.push(lane);
                }
                Err(fault) => {
                    engine_allocation::reserve(&mut failures, 1)?;
                    failures.push((lane, failure_at(at, fault)?));
                }
            }
        }
        let mut gone = Gone::default();
        let mut closed: Vec<Attached<'r, W>> = Vec::new();
        let ready = {
            let mut schedule = self.signal.lock();
            if self.stopped() {
                return Ok(None);
            }
            let deferred = schedule.refused(&self.plan, at, &refusing, &mut gone)?;
            for &(lane, before) in &deferred.lost {
                self.lost(&mut schedule, at, lane, Loss::After(before), &mut gone)?;
            }
            // Families closed since the walk took them: their branch ended, or they failed.
            let mut index = 0;
            while index < walking.len() {
                let lane = walking[index].lane;
                if schedule.status(at, lane) == Status::Abandoned {
                    schedule.abandoned(&self.plan, at, lane, &mut gone)?;
                    engine_allocation::reserve(&mut closed, 1)?;
                    closed.push(walking.remove(index));
                } else {
                    index += 1;
                }
            }
            deferred.ready
        };
        if !refusing.is_empty() || !closed.is_empty() {
            self.signal.changed();
        }
        for mut attached in closed {
            attached.operator.discard(self.folder);
        }
        self.drop_gone(gone);
        for lane in ready {
            match self.parked(at, lane) {
                Ok(Some(Parked::Ready(operator))) => {
                    engine_allocation::reserve(&mut walking, 1)?;
                    walking.push(Attached::new(lane, operator, false));
                }
                Ok(Some(Parked::Writing)) => {
                    engine_allocation::reserve(&mut landing, 1)?;
                    landing.push(lane);
                }
                Ok(None) => {
                    let error = invalid(format!(
                        "{} cannot start segment {at}, and the segment before handed it nothing",
                        self.erased(lane).name()
                    ));
                    engine_allocation::reserve(&mut failures, 1)?;
                    failures.push((
                        lane,
                        failure_at(at, Fault::Error(Operation::Decide, error))?,
                    ));
                }
                Err(fault) => {
                    engine_allocation::reserve(&mut failures, 1)?;
                    failures.push((lane, failure_at(at, fault)?));
                }
            }
        }
        if !landing.is_empty() {
            let mut gone = Gone::default();
            {
                let mut schedule = self.signal.lock();
                if self.stopped() {
                    return Ok(None);
                }
                schedule.waits(&self.plan, at, &landing, &mut gone)?;
            }
            self.signal.changed();
            self.drop_gone(gone);
        }
        walking.sort_unstable_by_key(|attached| attached.lane);
        // Families starting here start; those going on into a later run cross into it. One
        // joining the walk past its first hour starts there, or goes on from its carry.
        let run_start = segment.restart.is_none();
        for attached in walking.iter_mut().filter(|attached| !attached.begun) {
            attached.begun = true;
            if attached.joins > segment.first {
                continue;
            }
            let operator = &mut attached.operator;
            let begun = if attached.cold {
                caught(Operation::Start, || {
                    operator.start(start, planned.branch, planned.first)
                })
            } else if run_start {
                caught(Operation::Cross, || operator.cross(planned.first))
            } else {
                continue;
            };
            if let Err(fault) = begun {
                let when = (planned.first, segment.first);
                attached.fate = Some(self.fate(fault, attached, when)?);
            }
        }
        let culprit = Cell::new(None);
        let context = Context {
            folder: self.folder,
            culprit: &culprit,
        };
        // Its hours go to the workers that help it while it has families to share.
        let crew = match walking.len() {
            0 | 1 => None,
            _ => Some(self.signal.board.join(at)?),
        };
        let stopped = &self.signal.stopped;
        // The walk reached the segment's end with some family, leaving its last tables, and
        // lost none of them.
        let (mut whole, mut intact) = (false, true);
        let mut rewalks = 0;
        let walked = loop {
            if !walking.iter().any(Attached::walks) {
                break Ok(true);
            }
            // The world of this walk of the segment, for the families it visits: a walk begun
            // again after a panic builds its own.
            let visited = engine_allocation::collect_vec(
                walking
                    .iter()
                    .filter(|attached| attached.live())
                    .map(|attached| attached.lane),
            )?;
            let mut world = {
                let _memory = labels::enter(Owner::RUN, Kind::World);
                (self.worlds)(segment, &visited)?
            };
            let (mut ordinal, mut hours, mut noted) = (segment.first, 0_u32, 0);
            // The hour being walked, and what of a family's code, for a panic there.
            let (mut now, mut doing) = (planned.first, Operation::Hour);
            let attempt = catch_unwind(AssertUnwindSafe(|| {
                walker.walk_isolated(segment, state, |state, manifest, first, lost| {
                    now = manifest.hour;
                    // The run's own, before any family: no family is blamed for it.
                    let advanced = {
                        let _memory = labels::enter(Owner::RUN, Kind::World);
                        world.hour(state, manifest)
                    };
                    advanced?;
                    // A table lost to its own damage is lost to the families reading it.
                    if lost.len() != noted {
                        for (table, error) in &lost[noted..] {
                            let since = damaged_from(error);
                            for attached in walking.iter_mut() {
                                if self.lanes[attached.lane].reading(*table) {
                                    attached.lose(Fate::Table, since);
                                }
                            }
                        }
                        noted = lost.len();
                    }
                    // A family joining late decides once the walk holds the hour before.
                    for (index, attached) in walking.iter_mut().enumerate() {
                        if attached.fate.is_some()
                            || hours < attached.seen
                            || ordinal >= attached.joins
                        {
                            continue;
                        }
                        attached.through = hours + 1;
                        if ordinal + 1 == attached.joins
                            && let Some(carry) = attached.pending.take()
                            && let Err(fault) = self.decide(attached, carry, state)
                        {
                            let when = (manifest.hour + 1, ordinal + 1);
                            attached.fate = Some(self.fate(fault, attached, when)?);
                            continue;
                        }
                        // One starting anew where it joins starts once the hour before is.
                        if attached.cold && ordinal + 1 == attached.joins {
                            let next = manifest.hour + 1;
                            culprit.set(Some(index));
                            doing = Operation::Start;
                            let restart = Start::Restart(state);
                            let result = attached.operator.start(restart, planned.branch, next);
                            culprit.set(None);
                            doing = Operation::Hour;
                            let called = result.map_err(|error| Fault::of(Operation::Start, error));
                            if let Some(fault) = attached.met(called) {
                                let when = (next, ordinal + 1);
                                attached.fate = Some(self.fate(fault, attached, when)?);
                            }
                        }
                    }
                    let due = |attached: &Attached<'r, W>| {
                        attached.fate.is_none()
                            && hours >= attached.seen
                            && ordinal >= attached.joins
                    };
                    let (world, folder) = (&world, self.folder);
                    let hour_of = |attached: &Attached<'r, W>| {
                        let joining = ordinal == attached.joins && ordinal > segment.first;
                        Hour {
                            branch: planned.branch,
                            hour: manifest.hour,
                            run: segment.run,
                            ordinal,
                            starts: (first && (attached.cold || run_start))
                                || (joining && attached.cold),
                            opens: ordinal == segment.first || joining,
                            state,
                            manifest,
                            world,
                        }
                    };
                    let when = (manifest.hour, ordinal);
                    let mut panicked = None;
                    let shared_out = walking.iter().filter(|&attached| due(attached)).count() > 1;
                    let crewed = crew.as_ref().filter(|_| shared_out);
                    if let Some(crew) = crewed {
                        let mut turns = engine_allocation::vec(walking.len())?;
                        for (index, attached) in walking.iter_mut().enumerate() {
                            if due(&*attached) {
                                turns.push(Turn {
                                    index,
                                    attached,
                                    called: None,
                                });
                            }
                        }
                        // The costliest last hour first, on as many workers as the costs keep busy.
                        turns.sort_unstable_by_key(|turn| Reverse(turn.attached.cost));
                        let width = width(turns.iter().map(|turn| turn.attached.cost));
                        let mut slots =
                            engine_allocation::collect_vec(turns.into_iter().map(Mutex::new))?;
                        crew.hour(slots.len(), width, &|slot: usize| {
                            let mut turn = lock(&slots[slot]);
                            let Turn {
                                attached, called, ..
                            } = &mut *turn;
                            let began = Instant::now();
                            let hour = hour_of(&**attached);
                            let operator = &mut *attached.operator;
                            *called = Some(hour_on(operator, at, &hour, folder));
                            attached.cost =
                                u64::try_from(began.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        });
                        let mut turns: Vec<&mut Turn<'_, 'r, W>> =
                            engine_allocation::collect_vec(slots.iter_mut().map(|slot| {
                                slot.get_mut().unwrap_or_else(PoisonError::into_inner)
                            }))?;
                        // The run's own panic outranks anything a family's hour returned.
                        for turn in &mut turns {
                            if let Some(Called::Panicked { family: false, .. }) = turn.called
                                && let Some(Called::Panicked { payload, .. }) = turn.called.take()
                            {
                                resume_unwind(payload);
                            }
                        }
                        // In lane order, as run serially: the first panic begins the walk again.
                        turns.sort_unstable_by_key(|turn| turn.index);
                        for turn in turns {
                            let called = turn.called.take().expect("every turn ran");
                            let first = panicked.is_none();
                            if let Some(payload) =
                                self.settle(&mut *turn.attached, called, first, (when, hours))?
                            {
                                panicked = Some((turn.index, payload));
                            }
                        }
                    } else if let Some((index, attached)) = walking
                        .iter_mut()
                        .enumerate()
                        .find(|(_, attached)| due(&**attached))
                    {
                        let hour = hour_of(&*attached);
                        let called = hour_on(&mut *attached.operator, at, &hour, folder);
                        if let Some(payload) = self.settle(attached, called, true, (when, hours))? {
                            panicked = Some((index, payload));
                        }
                    }
                    if let Some((index, payload)) = panicked {
                        culprit.set(Some(index));
                        resume_unwind(payload);
                    }
                    let going = walking.iter().any(Attached::walks);
                    ordinal += 1;
                    hours += 1;
                    Ok(going && !stopped.load(Ordering::Relaxed))
                })
            }));
            match attempt {
                Ok(walked) => {
                    whole = matches!(walked, Ok(true));
                    break walked;
                }
                Err(payload) => {
                    // A family's panic is its own; the run's goes on.
                    let Some(index) = culprit.take() else {
                        resume_unwind(payload)
                    };
                    // What its items' sink met first stands, the run's own failure stopping it.
                    let fault = Fault::Panic(doing, said(&*payload));
                    let attached = &mut walking[index];
                    attached.fate = Some(self.fate(fault, attached, (now, ordinal))?);
                    // The others walk the segment again from where each was.
                    for attached in walking.iter_mut() {
                        attached.seen = attached.seen.max(attached.through);
                    }
                    // The tables lost so far stay lost to their readers once the walker forgets.
                    let tables = walker.take_lost();
                    // A table the recording withholds is lost to every walk alike.
                    intact &= tables
                        .iter()
                        .all(|(table, _)| self.recording.withheld().iter().any(|(held, _)| held == table));
                    self.lose_tables(&mut walking, tables);
                    // What unwound with the panic is not used again: the walker starts afresh
                    // with the share it holds, and the tables are begun again.
                    walker.renew()?;
                    *charged = (*charged).max(state.take_charge_peak());
                    *state = State::new(schema()?)?;
                    rewalks += 1;
                    match self.begin(walker, state, at, from) {
                        Ok(Begun::Ready(walks)) => derived += walks,
                        Ok(Begun::Stopped) => return Ok(None),
                        // A walk gone on from the one before needs a walk of the segments
                        // before that its share does not hold: it goes back, as begun cold.
                        Ok(Begun::Back) => {
                            self.resume(at, from, walking, failures, (derived, rewalks))?;
                            return Ok(None);
                        }
                        Ok(Begun::Lost(damage)) => break Err(damage),
                        Err(_) if stopped.load(Ordering::SeqCst) => return Ok(None),
                        Err(error) => return Err(error),
                    }
                }
            }
        };
        let tables = walker.take_lost();
        // A table the recording withholds is lost to every walk alike.
        intact &= tables
            .iter()
            .all(|(table, _)| self.recording.withheld().iter().any(|(held, _)| held == table));
        match walked {
            Ok(_) if stopped.load(Ordering::SeqCst) => return Ok(None),
            Ok(_) => {}
            Err(_) if stopped.load(Ordering::SeqCst) => return Ok(None),
            // The segment's own damage: every family walking it loses it.
            Err(error) if Damage::of(&error).is_some() => {
                let since = damaged_from(&error);
                let loss = Loss::Damaged(Arc::new(error));
                for attached in &mut walking {
                    attached.lose(Fate::Lost(loss.clone()), since);
                }
            }
            Err(error) => return Err(error),
        }
        // A table's own damage is lost to its readers, found as late as its page's end.
        self.lose_tables(&mut walking, tables);
        let last = (
            planned.first + (segment.last - segment.first) as i32,
            segment.last,
        );
        for (index, attached) in walking.iter_mut().enumerate() {
            if !attached.live() {
                continue;
            }
            let operator = &mut attached.operator;
            let ended = marked(&culprit, index, Operation::End, || {
                operator.end(at, &context)
            });
            if let Some(fault) = attached.met(ended) {
                attached.fate = Some(self.fate(fault, attached, last)?);
            }
        }
        if let Some(next) = planned.after {
            let start = if self.segments[next].run == segment.run {
                Start::Restart(&*state)
            } else {
                Start::Run { continues: true }
            };
            for attached in walking.iter_mut().filter(|attached| attached.live()) {
                let operator = &attached.operator;
                match caught(Operation::Decide, || Ok(operator.starts_at(start))) {
                    Ok(starts) => {
                        attached.hands = !starts;
                        attached.kept = attached.hands.then_some(Kept::Unsaved);
                    }
                    Err(fault) => attached.fate = Some(self.fate(fault, attached, last)?),
                }
            }
        } else {
            // The recording holds no more of the branch: what each family would hand on past it,
            // to a restart or a later run, is kept for a run of the recording grown.
            for (index, attached) in walking.iter_mut().enumerate() {
                if !attached.live() {
                    continue;
                }
                let operator = &attached.operator;
                let kept = marked(&culprit, index, Operation::Save, || {
                    let starts = operator.starts_at(Start::Restart(&*state))
                        && operator.starts_at(Start::Run { continues: true });
                    if starts {
                        return Ok(None);
                    }
                    let mut out = Vec::new();
                    Ok(Some(match operator.save(&mut out)? {
                        true => Kept::Saved(out),
                        false => Kept::Unsaved,
                    }))
                });
                match kept {
                    Ok(kept) => attached.kept = kept,
                    Err(fault) => attached.fate = Some(self.fate(fault, attached, last)?),
                }
            }
        }
        let mut projected = engine_allocation::vec(walking.len())?;
        for (index, attached) in walking.iter_mut().enumerate() {
            if !matches!(attached.fate, None | Some(Fate::Refused { .. })) {
                attached.operator.discard(self.folder);
                continue;
            }
            let operator = &mut attached.operator;
            match marked(&culprit, index, Operation::Save, || {
                operator.close(&context)
            }) {
                Ok(bytes) => projected.push((attached.lane, bytes)),
                Err(fault) => {
                    let fate = self.fate(fault, attached, last)?;
                    attached.operator.discard(self.folder);
                    attached.fate = Some(fate);
                }
            }
        }
        let next = planned.after;
        // A walk going on into the segment after keeps its tables only once it walked this
        // one through, every table whole.
        let hot =
            whole && intact && next.is_some_and(|next| self.segments[next].run == segment.run);
        if let Some(next) = next {
            let hands = walking
                .iter()
                .any(|attached| attached.live() && attached.hands);
            if (hot || hands) && self.signal.lock().reads(next).is_none() {
                self.measure(walker, next)?;
            }
        }
        let mut gone = Gone::default();
        let mut dropped: Vec<Attached<'r, W>> = engine_allocation::vec(walking.len())?;
        let mut kept: Vec<Attached<'r, W>> = engine_allocation::vec(walking.len())?;
        let (chained, begins, next) = {
            let mut schedule = self.signal.lock();
            // Faults its families met as it ended, the run stopped, are none of theirs.
            if self.stopped() {
                return Ok(None);
            }
            schedule.estimate(&projected);
            schedule.stats.derived += derived;
            schedule.stats.rewalked += rewalks;
            // Each taken in turn, those left dropping once the lock is let go.
            failures.reverse();
            while let Some((lane, failure)) = failures.pop() {
                self.failed(&mut schedule, lane, failure, true, &mut gone)?;
            }
            // Each family goes into `kept` or `dropped`, held for room, before anything that can
            // fail: none drops under the lock.
            walking.reverse();
            while let Some(attached) = walking.pop() {
                let lane = attached.lane;
                let arrived = schedule.arrived(at, lane);
                if schedule.status(at, lane) == Status::Abandoned {
                    dropped.push(attached);
                    schedule.abandoned(&self.plan, at, lane, &mut gone)?;
                    continue;
                }
                // An operator parked for a segment its family started anew: both did not decide
                // alike.
                let undecided = attached.cold && planned.before.is_some() && arrived;
                if attached.fate.is_none() && !undecided {
                    kept.push(attached);
                    continue;
                }
                dropped.push(attached);
                let Some(attached) = dropped.last_mut() else {
                    unreachable!("it was just held");
                };
                if undecided {
                    let decided = attached.fate.replace(Fate::Failed {
                        operation: Operation::Decide,
                        panicked: false,
                        error: invalid(format!(
                            "{} started segment {at}, which the segment before went on into",
                            self.erased(lane).name()
                        )),
                    });
                    if let Some(decided) = decided {
                        gone.waste.push(Waste::Fate(decided));
                    }
                }
                match attached.fate.take() {
                    None => unreachable!("a family dropped ended short"),
                    Some(Fate::Refused { hour, error, .. }) => {
                        let ending = Some(Ending::Refused { hour, error });
                        attached.operator.file(at, ending, None, false, self.folder);
                        schedule.settle(at, lane, Status::Refused);
                        schedule.close(&self.plan, lane, Closing::Branch(at), &mut gone)?;
                    }
                    Some(Fate::Lost(loss)) => {
                        self.lost(&mut schedule, at, lane, loss, &mut gone)?
                    }
                    Some(Fate::Table) => unreachable!("a walk names every table it lost"),
                    Some(Fate::Failed {
                        operation,
                        panicked,
                        error,
                    }) => {
                        let failure = Failure {
                            at,
                            operation,
                            panicked,
                            error,
                        };
                        self.failed(&mut schedule, lane, failure, true, &mut gone)?;
                    }
                }
            }
            let Some(next) = next else {
                let lanes = engine_allocation::collect_vec(kept.iter().map(|kept| kept.lane))?;
                for attached in &mut kept {
                    let carried = attached.kept.take();
                    attached
                        .operator
                        .file(at, None, carried, false, self.folder);
                }
                schedule.sealed(at, &lanes);
                drop(schedule);
                self.signal.changed();
                self.drop_gone(gone);
                for mut attached in dropped.into_iter().chain(kept) {
                    attached.operator.discard(self.folder);
                }
                return Ok(None);
            };
            let handed = engine_allocation::collect_vec(
                kept.iter()
                    .filter(|attached| attached.hands)
                    .map(|attached| attached.lane),
            )?;
            let chained = schedule.chain(&self.plan, (at, next), &handed, hot, &mut |reads| {
                walker.hold(reads)
            })?;
            for lane in schedule.started_without(next, &handed, &chained)? {
                let error = invalid(format!(
                    "{} went on into segment {next}, which started without it",
                    self.erased(lane).name()
                ));
                let failure = failure_at(next, Fault::Error(Operation::Decide, error))?;
                self.failed(&mut schedule, lane, failure, false, &mut gone)?;
            }
            // An operator going on into a segment its family will not walk goes no further.
            for attached in &mut kept {
                attached.hands &= chained.contains(&attached.lane)
                    || !matches!(
                        schedule.status(next, attached.lane),
                        Status::Closed | Status::Abandoned | Status::Lost
                    );
                let carried = attached.kept.take();
                attached
                    .operator
                    .file(at, None, carried, attached.hands, self.folder);
            }
            // Every family but those to park is sealed now: a chained one goes on with its walk.
            let unparked = engine_allocation::collect_vec(
                kept.iter()
                    .filter(|attached| !attached.hands || chained.contains(&attached.lane))
                    .map(|attached| attached.lane),
            )?;
            schedule.sealed(at, &unparked);
            // Where the walk it goes on into begins, as its share was measured.
            (chained, schedule.begins(next), next)
        };
        self.signal.changed();
        self.drop_gone(gone);
        for mut attached in dropped {
            attached.operator.discard(self.folder);
        }
        let mut chain = engine_allocation::vec(chained.len())?;
        // Families whose operator parked for `next`, or that failed to park it.
        let mut parked = Vec::new();
        let mut failures = Vec::new();
        for attached in kept {
            let lane = attached.lane;
            if !attached.hands {
                continue;
            }
            if chained.contains(&lane) {
                chain.push((lane, Some(attached.operator)));
                continue;
            }
            engine_allocation::reserve(&mut parked, 1)?;
            parked.push(lane);
            let operator = attached.operator;
            let error = match caught(Operation::Save, || Ok(operator.carried())) {
                Ok(Some(bytes)) => {
                    self.park(next, lane, operator, bytes)?;
                    continue;
                }
                Ok(None) => Fault::Error(
                    Operation::Save,
                    invalid(format!(
                        "{} cannot be parked for segment {next}: its operator does not say what \
                         it holds",
                        self.erased(lane).name()
                    )),
                ),
                Err(fault) => fault,
            };
            engine_allocation::reserve(&mut failures, 1)?;
            failures.push((lane, failure_at(next, error)?));
        }
        if !parked.is_empty() || !failures.is_empty() {
            failures.reverse();
            let mut gone = Gone::default();
            {
                let mut schedule = self.signal.lock();
                if self.stopped() {
                    return Ok(None);
                }
                while let Some((lane, failure)) = failures.pop() {
                    self.failed(&mut schedule, lane, failure, false, &mut gone)?;
                }
                for lane in parked {
                    // Closed while its operator parked: what it filed goes, with the operator.
                    if schedule.status(at, lane) == Status::Abandoned {
                        schedule.abandoned(&self.plan, at, lane, &mut gone)?;
                        engine_allocation::reserve(&mut gone.cells, 2)?;
                        gone.cells.push((lane, at, true));
                        gone.cells.push((lane, next, false));
                        continue;
                    }
                    match schedule.status(next, lane) {
                        // Its walk of `next` started without the operator parked for it.
                        Status::Sealed | Status::Refused => {
                            let error = invalid(format!(
                                "{} went on into segment {next}, which started without it",
                                self.erased(lane).name()
                            ));
                            let failure = failure_at(next, Fault::Error(Operation::Decide, error))?;
                            self.failed(&mut schedule, lane, failure, false, &mut gone)?;
                            engine_allocation::reserve(&mut gone.cells, 1)?;
                            gone.cells.push((lane, next, false));
                        }
                        Status::Closed | Status::Abandoned | Status::Lost => {
                            engine_allocation::reserve(&mut gone.cells, 1)?;
                            gone.cells.push((lane, next, false));
                        }
                        // Its walk, had it started anew, finds it parked as it ends.
                        Status::Running => schedule.arrives(next, lane),
                        Status::Open | Status::Waiting => {}
                    }
                    schedule.sealed(at, &[lane]);
                }
            }
            self.signal.changed();
            self.drop_gone(gone);
        }
        for &lane in &chained {
            if !chain.iter().any(|(held, _)| *held == lane) {
                chain.push((lane, None));
            }
        }
        if chain.is_empty() {
            return Ok(None);
        }
        chain.sort_unstable_by_key(|(lane, _)| *lane);
        Ok(Some(Job {
            at: next,
            from: begins,
            lanes: chain,
            hot,
        }))
    }
}

/// The failure `fault`, met at segment `at` outside a walk's hours, is; the run's own, and a
/// lack of memory, stop the run.
fn failure_at(at: usize, fault: Fault) -> io::Result<Failure> {
    match fault {
        Fault::Run(error) => Err(error),
        Fault::Error(_, error) if error.kind() == io::ErrorKind::OutOfMemory => Err(error),
        Fault::Error(operation, error) => Ok(Failure {
            at,
            operation,
            panicked: false,
            error,
        }),
        Fault::Panic(operation, said) => Ok(Failure {
            at,
            operation,
            panicked: true,
            error: io::Error::other(said),
        }),
    }
}

/// Walks every segment of `recording` on [`Settings::threads`] workers, each hour visiting
/// the operators of `lanes` that walk it, and hands each lane's emit its items in segment
/// order on those workers, spooled through the folder `spool`, which the run empties as it
/// starts and removes once done: what became of each family. A family's own failure ends it alone; the run's own failure stops every worker
/// and is returned, a panic outside any family's code going on once they have stopped; and
/// `cancellation` stops them, leaving what was not emitted pending. Damage that leaves hours
/// where no branch places them refuses the run before it starts.
pub fn run(
    recording: &Recording,
    settings: &Settings,
    spool: &Path,
    cancellation: &Cancellation,
    lanes: Vec<Lane<'_>>,
) -> io::Result<Ran> {
    run_in(recording, settings, spool, cancellation, &worldless, lanes)
}

/// The world of a run whose walks share none.
fn worldless(_: &Segment, _: &[usize]) -> io::Result<()> {
    Ok(())
}

/// [`run`], each walk's operators sharing the world `worlds` builds for the walk's segment and
/// the lanes whose operators it visits, by their places in `lanes`.
pub fn run_in<W: World>(
    recording: &Recording,
    settings: &Settings,
    spool: &Path,
    cancellation: &Cancellation,
    worlds: &Worlds<'_, W>,
    lanes: Vec<Lane<'_, W>>,
) -> io::Result<Ran> {
    if lanes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A run needs a family",
        ));
    }
    let ordered = |lane: &Lane<'_, W>| {
        lane.resumes
            .windows(2)
            .all(|pair| pair[0].branch < pair[1].branch)
            && lane
                .resumes
                .iter()
                .all(|resume| lane.passes.binary_search(&resume.branch).is_err())
    };
    if !lanes.iter().all(ordered) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A lane goes on in branches out of order, or in one it passes",
        ));
    }
    if let Some(damage) = recording.unlocated() {
        return Err(damage);
    }
    let settings = Settings::new(settings.threads);
    let segments = recording.segments();
    if segments.is_empty() {
        return Ok(Ran {
            outcomes: engine_allocation::collect_vec(lanes.iter().map(|_| Outcome::default()))?,
            ..Ran::default()
        });
    }
    let names = engine_allocation::collect_vec(lanes.iter().map(|lane| lane.erased.name()))?;
    let folder = Folder::open(spool, &names)?;
    let signal = Arc::new(Signal {
        schedule: Mutex::new(Schedule::new(segments.len(), lanes.len())?),
        board: Board::default(),
        stopped: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
    });
    let _entered = cancellation.enter(&signal)?;
    let plan = Plan::new(recording, segments)?;
    // What closing the passed cells lets go of drops once the lock is let go.
    let mut gone = Gone::default();
    let passed = lanes
        .iter()
        .enumerate()
        .try_for_each(|(lane, held)| signal.lock().pass(&plan, lane, held.passes, &mut gone));
    drop(gone);
    passed?;
    let mut joins = BTreeMap::new();
    for (lane, held) in lanes.iter().enumerate() {
        for resume in held.resumes {
            let mut gone = Gone::default();
            let at = (resume.branch, resume.run, resume.ordinal);
            let goes = signal.lock().resume(&plan, segments, lane, at, &mut gone);
            drop(gone);
            if let Some((at, ordinal)) = goes? {
                joins.insert((at, lane), (ordinal, resume.carry));
            }
        }
    }
    let run = Run {
        recording,
        segments,
        plan,
        lanes: &lanes,
        worlds,
        signal: &signal,
        budget: Arc::new(Budget::new(usize::MAX)),
        folder: &folder,
        store: Store::new(),
        resumed: Mutex::new(Vec::new()),
        joins,
        #[cfg(test)]
        cancellation,
    };
    std::thread::scope(|scope| {
        // First, so a failed spawn also wakes the workers already started.
        let _unwinding = Unwinding(&signal);
        for _ in 0..settings.threads {
            job_priority::spawn_in(scope, || run.work());
        }
    });
    let (outcomes, mut stats, tables) = {
        let mut schedule = signal.lock();
        if let Some(error) = schedule.failure.take() {
            return Err(error);
        }
        let tables = std::mem::take(&mut schedule.tables);
        (schedule.outcomes(), schedule.stats, tables)
    };
    let cancelled = signal.cancelled.load(Ordering::SeqCst);
    if !cancelled {
        run.store.finish(&lanes)?;
    }
    stats.walker_bytes = run.budget.peak();
    (stats.spool_bytes, stats.spooled) = folder.spooled();
    run.store.count(&mut stats);
    Ok(Ran {
        stats,
        outcomes,
        tables,
        cancelled,
    })
}
