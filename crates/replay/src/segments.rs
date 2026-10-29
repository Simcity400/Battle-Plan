//! Preparation by segments of a columnar recording.
//!
//! Every segment ([`Segment`]) is walked once, on its own worker, from its restart: each
//! hour goes to the operators made for that segment, which read the walker's typed columns
//! ([`crate::columns`]) and the hour's changes ([`State::changes`]), not rebuilt rows. A
//! segment after its run's first starts cold, from its restart alone, unless an operator
//! cannot start from what the restart holds ([`Operator::starts_at`]): the walk of the
//! segment before then goes on through it. Each segment's results reach the caller in
//! segment order as soon as the segments before it have, so a family's ordered work (its
//! keys, runs and holders) proceeds while later segments are still walked.

use crate::columnar_frames::{Recording, Segment, Walker, schema};
use replay_columnar::manifest::Manifest;
pub use replay_columnar::walk::State;
use std::{
    any::Any,
    collections::BTreeMap,
    io,
    sync::{
        Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
mod tests;

/// One hour of a segment's walk.
pub struct Hour<'a> {
    pub branch: u64,
    pub hour: i32,
    pub run: u32,
    /// The hour's ordinal in its run.
    pub ordinal: u32,
    /// The walker holds what its restart restored, or empty tables at its run's start, and
    /// this hour: nothing of the hours before.
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

/// A family's work on one segment.
pub trait Operator: Send {
    /// Whether a walk can start at `start`. When an operator cannot, the walk of the segment
    /// before it in its branch (the run's segment before, or the branch's earlier run's last)
    /// goes on through it and hands its results. Both walks decide alike.
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
    /// The walk goes on into a later run of its branch, whose first hour, `first`, applies to
    /// empty tables.
    fn cross(&mut self, first: i32) -> io::Result<()> {
        let _ = first;
        Ok(())
    }
    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()>;
    /// The result of the segment just walked, for the family's ordered work: taken at each
    /// segment's end, the walk going on through the next.
    fn take(&mut self) -> io::Result<Box<dyn Any + Send>>;
}

/// The operators each segment runs, in one order for every segment.
pub type Operators = Vec<Box<dyn Operator>>;

/// One segment's results, in its operators' order.
pub type Results = Vec<Box<dyn Any + Send>>;

/// Segments walked and not yet handed on, and the first failure.
struct Progress {
    next: usize,
    emitted: usize,
    done: BTreeMap<usize, Results>,
    failure: Option<io::Error>,
}

/// A run's progress, and how its workers wait for one another.
struct Schedule<'a> {
    recording: &'a Recording,
    ahead: usize,
    progress: Mutex<Progress>,
    changed: Condvar,
    stopped: AtomicBool,
}

impl Schedule<'_> {
    fn lock(&self) -> MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps the run's first failure and stops every worker.
    fn fail(&self, error: io::Error) {
        self.lock().failure.get_or_insert(error);
        self.stop();
    }

    /// Stops every worker, those waiting for the recording's budget too.
    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.changed.notify_all();
        self.recording.wake();
    }

    /// A walk going on through segment `next` waits for the look-ahead, but only once every
    /// segment before it has a walk of its own: one it would wait for might need its worker.
    /// Its walker gives back its buffers before it waits.
    fn turn(&self, next: usize, walker: &mut Walker<'_>) -> bool {
        let mut held = self.lock();
        loop {
            if held.failure.is_some() || self.stopped.load(Ordering::Relaxed) {
                return false;
            }
            if next - held.emitted < self.ahead || held.next < next {
                return true;
            }
            walker.park();
            held = self
                .changed
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Stops the run when its thread unwinds from a panic, so no other thread waits for it;
/// the scope then joins every worker and goes on with the panic.
struct Unwinding<'a>(&'a Schedule<'a>);

impl Drop for Unwinding<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0
                .lock()
                .failure
                .get_or_insert_with(|| io::Error::other("A segment walk panicked"));
            self.0.stop();
        }
    }
}

fn starts_at(operators: &Operators, start: Start<'_>) -> bool {
    operators.iter().all(|operator| operator.starts_at(start))
}

/// The hour `segment` starts at.
fn first_hour(recording: &Recording, segment: &Segment) -> io::Result<i32> {
    let run = recording.run(segment.run)?;
    i32::try_from(i64::from(run.first_hour) + i64::from(segment.first)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "A segment starts past the hours",
        )
    })
}

/// Segment `at` walked by `walker` through its operators, until stopped, and on through
/// every segment after it in its branch that they cannot start ([`Operator::starts_at`]):
/// the next of its run, else the first of its branch's next run. Each segment's results go
/// to `hand` at its end; the schedule holds the walk before it goes on, as the look-ahead
/// allows. A segment another walk goes on through hands nothing here.
fn walk(
    schedule: &Schedule<'_>,
    walker: &mut Walker<'_>,
    segments: &[Segment],
    at: usize,
    mut operators: Operators,
    hand: &mut dyn FnMut(usize, Results),
) -> io::Result<()> {
    let recording = schedule.recording;
    let branch_of = |run: u32| recording.run(run).map(|run| run.branch);
    let first = &segments[at];
    let branch = branch_of(first.run)?;
    let mut state = State::new(schema()?)?;
    walker.begin(first, &mut state)?;
    let start = match first.restart {
        Some(_) => Start::Restart(&state),
        None => Start::Run {
            continues: segments[..at].iter().any(|earlier| {
                earlier.run != first.run && branch_of(earlier.run).ok() == Some(branch)
            }),
        },
    };
    if !starts_at(&operators, start) {
        return Ok(());
    }
    let hour = first_hour(recording, first)?;
    for operator in &mut operators {
        operator.start(start, branch, hour)?;
    }
    let (mut walked, mut fresh) = (at, true);
    loop {
        let segment = &segments[walked];
        let mut ordinal = segment.first;
        walker.walk(segment, &mut state, |state, manifest, starts| {
            let hour = Hour {
                branch,
                hour: manifest.hour,
                run: segment.run,
                ordinal,
                starts: starts && fresh,
                opens: ordinal == segment.first,
                state,
                manifest,
            };
            for operator in &mut operators {
                operator.hour(&hour)?;
            }
            ordinal += 1;
            Ok(!schedule.stopped.load(Ordering::Relaxed))
        })?;
        fresh = false;
        let results =
            engine_allocation::collect_vec(operators.iter_mut().map(|operator| operator.take()))?
                .into_iter()
                .collect::<io::Result<Results>>()?;
        hand(walked, results);
        if schedule.stopped.load(Ordering::Relaxed) {
            break;
        }
        let next = match segments.get(walked + 1) {
            Some(next) if next.run == segment.run => {
                (!starts_at(&operators, Start::Restart(&state))).then_some(walked + 1)
            }
            _ => segments[walked + 1..]
                .iter()
                .position(|later| {
                    later.restart.is_none() && branch_of(later.run).ok() == Some(branch)
                })
                .map(|offset| walked + 1 + offset)
                .filter(|_| !starts_at(&operators, Start::Run { continues: true })),
        };
        let Some(next) = next else {
            break;
        };
        if !schedule.turn(next, walker) {
            break;
        }
        if segments[next].run != segment.run {
            walker.begin(&segments[next], &mut state)?;
            let first = first_hour(recording, &segments[next])?;
            for operator in &mut operators {
                operator.cross(first)?;
            }
            fresh = true;
        }
        walked = next;
    }
    Ok(())
}

/// Walks every segment of `recording` on up to `threads` workers, each hour visiting the
/// operators `make` gives the segment, and hands `emit` each segment's results on the
/// caller's thread, in segment order. At most `ahead` segments are walked or waiting at once
/// past the last one handed on, which bounds the results held. Each worker keeps one walker,
/// whose buffers it gives back before it waits. The first failure stops every worker and is
/// returned; a panic stops them too and goes on once they have stopped.
pub fn run(
    recording: &Recording,
    threads: usize,
    ahead: usize,
    make: &(dyn Fn(&Segment) -> io::Result<Operators> + Sync),
    emit: &mut dyn FnMut(&Segment, Results) -> io::Result<()>,
) -> io::Result<()> {
    let segments = recording.segments();
    let schedule = Schedule {
        recording,
        ahead: ahead.max(1),
        progress: Mutex::new(Progress {
            next: 0,
            emitted: 0,
            done: BTreeMap::new(),
            failure: None,
        }),
        changed: Condvar::new(),
        stopped: AtomicBool::new(false),
    };
    std::thread::scope(|scope| {
        // First, so a failed spawn also wakes the workers already started.
        let _unwinding = Unwinding(&schedule);
        for _ in 0..threads.clamp(1, segments.len().max(1)) {
            scope.spawn(|| {
                let _unwinding = Unwinding(&schedule);
                let mut walker = match Walker::new(recording, &schedule.stopped) {
                    Ok(walker) => walker,
                    Err(error) => return schedule.fail(error),
                };
                loop {
                    let at = {
                        let mut held = schedule.lock();
                        loop {
                            // A walk going on through segments hands them itself.
                            while held.next < segments.len()
                                && (held.next < held.emitted || held.done.contains_key(&held.next))
                            {
                                held.next += 1;
                            }
                            if held.failure.is_some() || held.next >= segments.len() {
                                return;
                            }
                            if held.next - held.emitted < schedule.ahead {
                                break;
                            }
                            walker.park();
                            held = schedule
                                .changed
                                .wait(held)
                                .unwrap_or_else(PoisonError::into_inner);
                        }
                        held.next += 1;
                        held.next - 1
                    };
                    let mut hand = |walked: usize, results: Results| {
                        schedule.lock().done.insert(walked, results);
                        schedule.changed.notify_all();
                    };
                    let walked = make(&segments[at]).and_then(|operators| {
                        walk(&schedule, &mut walker, segments, at, operators, &mut hand)
                    });
                    if let Err(error) = walked {
                        return schedule.fail(error);
                    }
                }
            });
        }
        for (at, segment) in segments.iter().enumerate() {
            let results = {
                let mut held = schedule.lock();
                loop {
                    if held.failure.is_some() {
                        return;
                    }
                    if let Some(results) = held.done.remove(&at) {
                        break results;
                    }
                    held = schedule
                        .changed
                        .wait(held)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            if let Err(error) = emit(segment, results) {
                return schedule.fail(error);
            }
            schedule.lock().emitted = at + 1;
            schedule.changed.notify_all();
        }
    });
    match schedule
        .progress
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .failure
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
