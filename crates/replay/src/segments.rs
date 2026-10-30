//! Preparation by segments of a columnar recording.
//!
//! A pool of workers walks each segment ([`Segment`]) in a job of its own, from its restart or
//! from empty tables at its run's start, through the operators of the families ([`Lane`]) the
//! job carries: each hour goes to them as the walker's typed columns ([`crate::columns`]) and
//! the hour's changes ([`State::changes`]), not rebuilt rows. A family whose operator cannot
//! start where a segment starts ([`Operator::starts_at`]) goes on with the operator that
//! walked the segment before it in its branch: that walk hands it on, or parks it in a store
//! that spills what its budget does not hold ([`Scratch`]); a walker's tables are never
//! parked. Each family's items reach its ordered emit in segment order, a chunk at a time, as
//! a task of the same pool, so families emit alongside one another and the walks. No segment
//! starts [`Settings::window`] segments or more past the first some family has not emitted,
//! nor before it holds the share of the walkers' budget its reads need, and none but that
//! first starts while the output it is taken to need, beside what is held and what walks
//! reserved, would overdraw the output budget, nor while a family's slots are spent. No task
//! waits for credit: one that cannot have it is not handed out.

use crate::columnar_frames::{Budget, Recording, Segment, Walker, schema};
use replay_columnar::manifest::Manifest;
pub use replay_columnar::walk::State;
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

mod lane;
mod schedule;
mod store;
#[cfg(test)]
mod tests;

pub use store::{Folder, Scratch, Spilled};

use lane::{Context, Emitted, Erased, Reserved, Shared, Taken, Walking};
use schedule::{Plan, Schedule, Task};
use store::Store;

/// One hour of a segment's walk.
pub struct Hour<'a> {
    pub branch: u64,
    pub hour: i32,
    pub run: u32,
    /// The hour's ordinal in its run.
    pub ordinal: u32,
    /// The walker holds what its restart restored, or empty tables at its run's start, and
    /// this hour: nothing of the hours before. False at the first hour of a segment whose
    /// operator goes on from the segment before in its run.
    pub starts: bool,
    /// The hour starts a segment, a unit the recording can restart at.
    pub opens: bool,
    pub state: &'a State,
    pub manifest: &'a Manifest<'a>,
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

/// An item a family's operators project for its ordered emit.
pub trait Item: Send + Sized {
    /// Whether [`Self::save`] writes them: a walk then holds a chunk of them at a time past
    /// the output budget, spilling the rest. Others stay resident.
    const SPILLS: bool = false;

    /// The bytes it owns on the heap; the budget counts its own size with its chunk's.
    fn heap(&self) -> usize;

    /// Writes `items` whole, for the spool, when they spill.
    fn save(items: &[Self], out: &mut Vec<u8>) -> io::Result<()> {
        let _ = (items, out);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "These items are never spilled",
        ))
    }

    /// The items [`Self::save`] wrote, holding no more than they did when saved: reading them
    /// back is admitted for those bytes, and a run refuses a chunk that comes back larger.
    fn load(bytes: &[u8]) -> io::Result<Vec<Self>> {
        let _ = bytes;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "These items are never spilled",
        ))
    }
}

impl Item for () {
    fn heap(&self) -> usize {
        0
    }
}

/// A family's work on a segment, and on the segments after it in its branch that it cannot
/// start.
pub trait Operator: Send {
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

    fn hour(&mut self, hour: &Hour<'_>, out: &mut dyn Sink<Self::Output>) -> io::Result<()>;

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
}

/// A family: its operator for each segment, and its ordered emit, which takes every
/// segment's chunks in segment order, one call at a time. Operators and emits run as tasks
/// of the run's workers, and must never wait for another task of the run: it may need
/// their worker.
pub struct Lane<'a>(Box<dyn Erased + 'a>);

impl<'a> Lane<'a> {
    pub fn new<O: Operator + 'a>(
        name: &'a str,
        make: impl Fn(&Segment) -> io::Result<O> + Sync + 'a,
        emit: impl FnMut(Chunk<'_, O::Output>) -> io::Result<()> + Send + 'a,
    ) -> Self {
        Self(Box::new(lane::Typed::new(
            name,
            Box::new(make),
            Box::new(emit),
        )))
    }
}

/// What a run may do and hold at once. These budget memory and speculation; none limits
/// what a recording holds.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Workers, walking segments and running the families' emits alike.
    pub threads: usize,
    /// Segments walked or waiting for emit, past and including the first some family has
    /// not emitted.
    pub window: usize,
    /// Projected items held resident, by their bytes, with what walks reserved as they
    /// started, before segments past the first some family has not emitted wait, and their
    /// spilled chunks wait to be read back; chunks closed past it spill.
    pub output_bytes: usize,
    /// Projected bytes held resident past `output_bytes`, for the first segment some family
    /// has not emitted, which starts whatever is held and reads its spilled chunks back, and
    /// walks projecting past what they reserved; items that cannot spill fail the run past it.
    pub working_bytes: usize,
    /// Segments a family may hold sealed for its emit before segments past the first some
    /// family has not emitted wait.
    pub slots: usize,
    /// Parked operators held resident; the furthest from their segment spill past it.
    pub carry_bytes: usize,
    /// Projected bytes a chunk holds before it closes: what a walk reserves for a family
    /// whose items spill.
    pub chunk_bytes: usize,
    /// The walkers' buffers at once, the run's own: a walk takes the share its segment's
    /// reads need as it starts.
    pub walker_bytes: usize,
}

impl Settings {
    pub fn new(threads: usize) -> Self {
        let threads = threads.max(1);
        let output_bytes = 768 << 20;
        Self {
            threads,
            window: threads.saturating_mul(2),
            output_bytes,
            // The largest segment the budget admits past the first also fits as the first.
            working_bytes: output_bytes,
            slots: threads.saturating_mul(2),
            carry_bytes: 512 << 20,
            chunk_bytes: 8 << 20,
            walker_bytes: 2048 << 20,
        }
    }

    /// The most projected output held resident before items that cannot spill fail the run.
    fn ceiling(&self) -> usize {
        self.output_bytes.saturating_add(self.working_bytes)
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
    /// Projected bytes held resident at once, spilled chunks being read back included.
    pub output_bytes: usize,
    /// Parked operators' bytes held resident at once, one being parked and not yet spilled
    /// included.
    pub carry_bytes: usize,
    /// Bytes of parked operators being written out or read back at once.
    pub carry_scratch: usize,
    pub parked: usize,
    /// Parked operators spilled, and read back.
    pub spilled: usize,
    pub loaded: usize,
    /// Projected chunks spilled.
    pub chunks_spilled: usize,
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
        self.cancelled.store(true, Ordering::SeqCst);
        for signal in lock(&self.runs).iter() {
            signal.cancel();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Wakes `signal`'s run once cancelled, until dropped.
    fn enter(&self, signal: &Arc<Signal>) -> io::Result<Entered<'_>> {
        {
            let mut runs = lock(&self.runs);
            runs.try_reserve(1).map_err(|_| engine_allocation::oom())?;
            runs.push(Arc::clone(signal));
        }
        if self.is_cancelled() {
            signal.cancel();
        }
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

/// A run's schedule, and how its workers and its cancellation wake one another.
struct Signal {
    schedule: Mutex<Schedule>,
    changed: Condvar,
    stopped: AtomicBool,
}

impl Signal {
    fn lock(&self) -> MutexGuard<'_, Schedule> {
        lock(&self.schedule)
    }

    fn wait<'g>(&self, schedule: MutexGuard<'g, Schedule>) -> MutexGuard<'g, Schedule> {
        self.changed
            .wait(schedule)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes the workers waiting, once whatever they read under the lock has changed.
    fn changed(&self) {
        drop(self.lock());
        self.changed.notify_all();
    }

    /// Keeps the run's first failure and stops every worker.
    fn fail(&self, error: io::Error) {
        self.lock().failure.get_or_insert(error);
        self.stop();
    }

    fn cancel(&self) {
        self.fail(io::Error::new(
            io::ErrorKind::Interrupted,
            "The run was cancelled",
        ));
    }

    /// Stops every worker.
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
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
/// on, if any, and what it reserved of the output budget.
struct Job<'r, 'c> {
    at: usize,
    lanes: Vec<(usize, Option<Box<dyn Walking + 'r>>)>,
    /// The state holds the tables the walk of the segment before, of the same run, left.
    hot: bool,
    reserved: Reserved<'c>,
}

/// A family's operator in a walk: `cold` when it started at the walk's first hour.
struct Attached<'r> {
    lane: usize,
    operator: Box<dyn Walking + 'r>,
    cold: bool,
}

/// What a run's workers share.
struct Run<'r, 'a> {
    recording: &'r Recording,
    segments: &'r [Segment],
    plan: Plan,
    lanes: &'r [Lane<'a>],
    signal: &'r Signal,
    /// The walkers' budget, the run's alone: no walker waits for it.
    budget: Arc<Budget>,
    store: Store<'r>,
    shared: Shared<'r>,
    #[cfg(test)]
    cancellation: &'r Cancellation,
}

impl<'r, 'a> Run<'r, 'a> {
    fn erased(&self, lane: usize) -> &'r (dyn Erased + 'r) {
        let lanes: &'r [Lane<'a>] = self.lanes;
        &*lanes[lane].0
    }

    fn work(&self) {
        let _unwinding = Unwinding(self.signal);
        let mut walker = match Walker::without_waiting(self.recording, &self.budget) {
            Ok(walker) => walker,
            Err(error) => return self.signal.fail(error),
        };
        let mut state = match schema().and_then(State::new) {
            Ok(state) => state,
            Err(error) => return self.signal.fail(error),
        };
        loop {
            let task = {
                let mut schedule = self.signal.lock();
                loop {
                    if schedule.failure.is_some()
                        || self.signal.stopped.load(Ordering::SeqCst)
                        || schedule.finished()
                    {
                        return;
                    }
                    let picked = schedule.pick(
                        &self.plan,
                        &self.shared.settings,
                        &self.shared.credit,
                        &mut |reads| walker.hold(reads),
                    );
                    match picked {
                        Ok(Some(task)) => {
                            if matches!(task, Task::Emit { .. }) && walker.holds() {
                                // An emit may take long: its share goes to another's walk.
                                walker.park();
                                self.signal.changed.notify_all();
                            }
                            break task;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            drop(schedule);
                            return self.signal.fail(error);
                        }
                    }
                    if walker.holds() {
                        // Its share goes back before it waits, for another's walk.
                        walker.park();
                        self.signal.changed.notify_all();
                        continue;
                    }
                    if let Some((lane, at, need)) = schedule.stuck() {
                        drop(schedule);
                        return self.signal.fail(self.unrestorable(lane, at, need));
                    }
                    #[cfg(test)]
                    self.cancellation.idle(true);
                    schedule = self.signal.wait(schedule);
                    #[cfg(test)]
                    self.cancellation.idle(false);
                }
            };
            let done = match task {
                Task::Measure { at } => self.measure(&mut walker, at),
                Task::Emit { lane, at, restore } => self.emit(lane, at, restore),
                Task::Walk {
                    at,
                    lanes,
                    reserved,
                } => self.walk(&mut walker, &mut state, at, lanes, reserved),
            };
            self.signal.lock().done();
            self.signal.changed.notify_all();
            if let Err(error) = done {
                return self.signal.fail(error);
            }
        }
    }

    /// Measures what walking segment `at` reads.
    fn measure(&self, walker: &mut Walker<'_>, at: usize) -> io::Result<()> {
        let reads = walker.measure(&self.segments[at])?;
        self.signal.lock().measured(at, reads);
        self.signal.changed.notify_all();
        Ok(())
    }

    fn emit(&self, lane: usize, at: usize, restore: Option<Taken<'_>>) -> io::Result<()> {
        let emitted = self
            .erased(lane)
            .emit(at, &self.segments[at], &self.shared, restore)?;
        {
            let mut schedule = self.signal.lock();
            match emitted {
                Emitted::Ended => schedule.emitted(lane, at),
                Emitted::Spilled(need) => schedule.restores(lane, need),
            }
        }
        self.signal.changed.notify_all();
        Ok(())
    }

    /// The failure of a run whose `lane` cannot read back its spilled chunk of segment `at`,
    /// `need` bytes, beside what is held, with nothing left to make room.
    fn unrestorable(&self, lane: usize, at: usize, need: usize) -> io::Error {
        let settings = &self.shared.settings;
        io::Error::new(
            io::ErrorKind::OutOfMemory,
            format!(
                "{}'s chunk of segment {at} takes {need} bytes to read back, past the output \
                 budget of {} bytes and its working allowance of {} bytes beside the {} bytes \
                 held",
                self.erased(lane).name(),
                settings.output_bytes,
                settings.working_bytes,
                self.shared.credit.held()
            ),
        )
    }

    /// Segment `at`'s walk of `lanes`, and every walk it goes on into.
    fn walk<'c>(
        &'c self,
        walker: &mut Walker<'_>,
        state: &mut State,
        at: usize,
        lanes: Vec<usize>,
        reserved: Reserved<'c>,
    ) -> io::Result<()> {
        let mut job = Some(Job {
            at,
            lanes: engine_allocation::collect_vec(lanes.into_iter().map(|lane| (lane, None)))?,
            hot: false,
            reserved,
        });
        while let Some(next) = job.take() {
            job = self.segment(walker, state, next)?;
        }
        Ok(())
    }

    /// The operator of `lane` parked for segment `at`, if any.
    fn parked(&self, at: usize, lane: usize) -> io::Result<Option<Box<dyn Walking + 'r>>> {
        self.store
            .take(at, lane, &self.segments[at], self.erased(lane))
    }

    /// Walks `job`'s segment through its families' operators, seals what they projected,
    /// and hands on or parks those that cannot start the segment after in their branch: the
    /// walk it goes on into, when that segment may start now.
    fn segment<'c>(
        &'c self,
        walker: &mut Walker<'_>,
        state: &mut State,
        job: Job<'r, 'c>,
    ) -> io::Result<Option<Job<'r, 'c>>> {
        let Job {
            at,
            lanes,
            hot,
            reserved,
        } = job;
        let segment = &self.segments[at];
        let planned = *self.plan.get(at);
        if !hot {
            walker.begin(segment, state)?;
        }
        let start = match segment.restart {
            Some(_) => Start::Restart(&*state),
            None => Start::Run {
                continues: planned.before.is_some(),
            },
        };
        let mut walking: Vec<Attached<'r>> = engine_allocation::vec(lanes.len())?;
        let mut refused = Vec::new();
        for (lane, handed) in lanes {
            let carried = match handed {
                Some(operator) => Some(operator),
                None => self.parked(at, lane)?,
            };
            if let Some(operator) = carried {
                walking.push(Attached {
                    lane,
                    operator,
                    cold: false,
                });
                continue;
            }
            let operator = self.erased(lane).fresh(segment)?;
            if operator.starts_at(start) {
                walking.push(Attached {
                    lane,
                    operator,
                    cold: true,
                });
            } else if planned.before.is_none() {
                return Err(invalid(format!(
                    "{} cannot start segment {at}, which no segment of its branch precedes",
                    self.erased(lane).name()
                )));
            } else {
                engine_allocation::reserve(&mut refused, 1)?;
                refused.push(lane);
            }
        }
        if !refused.is_empty() {
            let ready = self.signal.lock().refused(&self.plan, at, &refused)?;
            self.signal.changed.notify_all();
            for lane in ready {
                let operator = self.parked(at, lane)?.ok_or_else(|| {
                    invalid(format!(
                        "{} cannot start segment {at}, and the segment before handed it nothing",
                        self.erased(lane).name()
                    ))
                })?;
                walking.push(Attached {
                    lane,
                    operator,
                    cold: false,
                });
            }
            walking.sort_unstable_by_key(|attached| attached.lane);
        }
        if walking.is_empty() {
            return Ok(None);
        }
        for attached in &mut walking {
            if attached.cold {
                attached
                    .operator
                    .start(start, planned.branch, planned.first)?;
            } else if segment.restart.is_none() {
                attached.operator.cross(planned.first)?;
            }
        }
        let context = Context {
            shared: &self.shared,
            at,
            reserved: &reserved,
        };
        let stopped = &self.signal.stopped;
        let run_start = segment.restart.is_none();
        let mut ordinal = segment.first;
        let walked = walker.walk(segment, state, |state, manifest, first| {
            for attached in walking.iter_mut() {
                let hour = Hour {
                    branch: planned.branch,
                    hour: manifest.hour,
                    run: segment.run,
                    ordinal,
                    starts: first && (attached.cold || run_start),
                    opens: ordinal == segment.first,
                    state,
                    manifest,
                };
                attached.operator.hour(&hour, &context)?;
            }
            ordinal += 1;
            Ok(!stopped.load(Ordering::Relaxed))
        })?;
        if !walked {
            return Ok(None);
        }
        let mut projected = engine_allocation::vec(walking.len())?;
        for attached in &mut walking {
            attached.operator.end(&context)?;
            projected.push((attached.lane, attached.operator.seal(&context)?));
        }
        drop(reserved);
        let lanes = engine_allocation::collect_vec(walking.iter().map(|attached| attached.lane))?;
        // The operators that cannot start the segment after go on into it.
        let mut handing: Vec<(usize, Box<dyn Walking + 'r>)> = Vec::new();
        if let Some(next) = planned.after {
            let start = if self.segments[next].run == segment.run {
                Start::Restart(&*state)
            } else {
                Start::Run { continues: true }
            };
            for attached in walking {
                if !attached.operator.starts_at(start) {
                    engine_allocation::reserve(&mut handing, 1)?;
                    handing.push((attached.lane, attached.operator));
                }
            }
        }
        let Some(next) = planned.after else {
            {
                let mut schedule = self.signal.lock();
                schedule.estimate(&projected);
                schedule.sealed(at, &lanes);
            }
            self.signal.changed.notify_all();
            return Ok(None);
        };
        let hot = self.segments[next].run == segment.run;
        let handed = engine_allocation::collect_vec(handing.iter().map(|(lane, _)| *lane))?;
        if (hot || !handed.is_empty()) && self.signal.lock().reads(next).is_none() {
            self.measure(walker, next)?;
        }
        let (chained, reserved) = {
            let mut schedule = self.signal.lock();
            schedule.estimate(&projected);
            let (chained, reserved) = schedule.chain(
                &self.plan,
                &self.shared.settings,
                &self.shared.credit,
                next,
                &handed,
                hot,
                &mut |reads| walker.hold(reads),
            )?;
            if let Some(lane) = schedule.started_without(next, &handed, &chained) {
                return Err(invalid(format!(
                    "{} went on into segment {next}, which started without it",
                    self.erased(lane).name()
                )));
            }
            // Every family but those to park is sealed now: a chained one goes on with its walk.
            let parking: Vec<usize> = handed
                .iter()
                .copied()
                .filter(|lane| !chained.contains(lane))
                .collect();
            let unparked = engine_allocation::collect_vec(
                lanes.iter().copied().filter(|lane| !parking.contains(lane)),
            )?;
            schedule.sealed(at, &unparked);
            (chained, reserved)
        };
        self.signal.changed.notify_all();
        let mut chain = engine_allocation::vec(chained.len())?;
        let mut parked = Vec::new();
        for (lane, operator) in handing {
            if chained.contains(&lane) {
                chain.push((lane, Some(operator)));
            } else {
                self.store
                    .park(next, lane, operator, self.lanes, self.shared.scratch)?;
                engine_allocation::reserve(&mut parked, 1)?;
                parked.push(lane);
            }
        }
        if !parked.is_empty() {
            self.signal.lock().sealed(at, &parked);
            self.signal.changed.notify_all();
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
            lanes: chain,
            hot,
            reserved,
        }))
    }
}

/// Walks every segment of `recording` on [`Settings::threads`] workers, each hour visiting
/// the operators of `lanes` that walk it, and hands each lane's emit its items in segment
/// order on those workers, spilling to `scratch` what the budgets do not hold. The first
/// failure, or `cancellation`, stops every worker and is returned; a panic stops them too and
/// goes on once they have stopped.
pub fn run(
    recording: &Recording,
    settings: &Settings,
    scratch: &dyn Scratch,
    cancellation: &Cancellation,
    lanes: Vec<Lane<'_>>,
) -> io::Result<Stats> {
    if lanes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A run needs a family",
        ));
    }
    let settings = Settings {
        threads: settings.threads.max(1),
        window: settings.window.max(1),
        slots: settings.slots.max(1),
        chunk_bytes: settings.chunk_bytes.max(1),
        ..*settings
    };
    let segments = recording.segments();
    if segments.is_empty() {
        return Ok(Stats::default());
    }
    let spills = engine_allocation::collect_vec(lanes.iter().map(|lane| lane.0.spills()))?;
    let signal = Arc::new(Signal {
        schedule: Mutex::new(Schedule::new(segments.len(), spills)?),
        changed: Condvar::new(),
        stopped: AtomicBool::new(false),
    });
    let _entered = cancellation.enter(&signal)?;
    let run = Run {
        recording,
        segments,
        plan: Plan::new(recording, segments)?,
        lanes: &lanes,
        signal: &signal,
        budget: Arc::new(Budget::new(settings.walker_bytes)),
        store: Store::new(settings.carry_bytes),
        shared: Shared::new(settings, scratch, &signal),
        #[cfg(test)]
        cancellation,
    };
    std::thread::scope(|scope| {
        // First, so a failed spawn also wakes the workers already started.
        let _unwinding = Unwinding(&signal);
        for _ in 0..settings.threads {
            scope.spawn(|| run.work());
        }
    });
    if let Some(error) = signal.lock().failure.take() {
        return Err(error);
    }
    run.store.finish(&lanes)?;
    let mut stats = signal.lock().stats;
    stats.output_bytes = run.shared.credit.peak();
    stats.chunks_spilled = run.shared.chunks_spilled.load(Ordering::SeqCst);
    run.store.count(&mut stats);
    Ok(stats)
}
