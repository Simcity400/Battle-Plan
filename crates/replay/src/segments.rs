//! Preparation by segments of a columnar recording.
//!
//! Every segment ([`Segment`]) is walked once, on its own worker, from its restart: each
//! hour goes to the operators made for that segment, which read the walker's typed columns
//! ([`crate::columns`]) and the hour's changes ([`State::changes`]), not rebuilt rows. A
//! segment after its run's first starts cold, from its restart alone. Each segment's results
//! reach the caller in segment order as soon as the segments before it have, so a family's
//! ordered work (its keys, runs and holders) proceeds while later segments are still walked.

use crate::columnar_frames::{Recording, Segment, schema};
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
    /// The segment's first hour: the walker holds what its restart restored and this hour,
    /// and nothing of the hours before.
    pub starts: bool,
    pub state: &'a State,
    pub manifest: &'a Manifest<'a>,
}

/// A family's work on one segment.
pub trait Operator: Send {
    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()>;
    /// The segment's result, for the family's ordered work.
    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>>;
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

/// Stops the run when its thread unwinds from a panic, so no other thread waits for it;
/// the scope then joins every worker and goes on with the panic.
struct Unwinding<'a> {
    progress: &'a Mutex<Progress>,
    changed: &'a Condvar,
    stopped: &'a AtomicBool,
}

impl Drop for Unwinding<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.stopped.store(true, Ordering::Relaxed);
            let mut held = self.progress.lock().unwrap_or_else(PoisonError::into_inner);
            held.failure
                .get_or_insert_with(|| io::Error::other("A segment walk panicked"));
            drop(held);
            self.changed.notify_all();
        }
    }
}

/// One segment walked through its operators, until `stopped`.
fn walk(
    recording: &Recording,
    segment: &Segment,
    mut operators: Operators,
    stopped: &AtomicBool,
) -> io::Result<Results> {
    let branch = recording.run(segment.run)?.branch;
    let mut state = State::new(schema()?)?;
    let mut ordinal = segment.first;
    recording.walk_segment(segment, &mut state, |state, manifest, starts| {
        let hour = Hour {
            branch,
            hour: manifest.hour,
            run: segment.run,
            ordinal,
            starts,
            state,
            manifest,
        };
        for operator in &mut operators {
            operator.hour(&hour)?;
        }
        ordinal += 1;
        Ok(!stopped.load(Ordering::Relaxed))
    })?;
    engine_allocation::collect_vec(operators.into_iter().map(|operator| operator.finish()))?
        .into_iter()
        .collect()
}

/// Walks every segment of `recording` on up to `threads` workers, each hour visiting the
/// operators `make` gives the segment, and hands `emit` each segment's results on the
/// caller's thread, in segment order. At most `ahead` segments are walked or waiting at once
/// past the last one handed on, which bounds the results held. The first failure stops
/// every worker and is returned; a panic stops them too and goes on once they have stopped.
pub fn run(
    recording: &Recording,
    threads: usize,
    ahead: usize,
    make: &(dyn Fn(&Segment) -> io::Result<Operators> + Sync),
    emit: &mut dyn FnMut(&Segment, Results) -> io::Result<()>,
) -> io::Result<()> {
    let segments = recording.segments();
    let ahead = ahead.max(1);
    let progress = Mutex::new(Progress {
        next: 0,
        emitted: 0,
        done: BTreeMap::new(),
        failure: None,
    });
    let changed = Condvar::new();
    let stopped = AtomicBool::new(false);
    let lock =
        || -> MutexGuard<'_, Progress> { progress.lock().unwrap_or_else(PoisonError::into_inner) };
    let unwinding = || Unwinding {
        progress: &progress,
        changed: &changed,
        stopped: &stopped,
    };
    let fail = |error: io::Error| {
        lock().failure.get_or_insert(error);
        stopped.store(true, Ordering::Relaxed);
        changed.notify_all();
    };
    std::thread::scope(|scope| {
        // First, so a failed spawn also wakes the workers already started.
        let _unwinding = unwinding();
        for _ in 0..threads.clamp(1, segments.len().max(1)) {
            scope.spawn(|| {
                let _unwinding = unwinding();
                loop {
                    let at = {
                        let mut held = lock();
                        loop {
                            if held.failure.is_some() || held.next >= segments.len() {
                                return;
                            }
                            if held.next - held.emitted < ahead {
                                break;
                            }
                            held = changed.wait(held).unwrap_or_else(PoisonError::into_inner);
                        }
                        held.next += 1;
                        held.next - 1
                    };
                    let segment = &segments[at];
                    let walked = make(segment)
                        .and_then(|operators| walk(recording, segment, operators, &stopped));
                    match walked {
                        Ok(results) => {
                            lock().done.insert(at, results);
                            changed.notify_all();
                        }
                        Err(error) => return fail(error),
                    }
                }
            });
        }
        for (at, segment) in segments.iter().enumerate() {
            let results = {
                let mut held = lock();
                loop {
                    if held.failure.is_some() {
                        return;
                    }
                    if let Some(results) = held.done.remove(&at) {
                        break results;
                    }
                    held = changed.wait(held).unwrap_or_else(PoisonError::into_inner);
                }
            };
            if let Err(error) = emit(segment, results) {
                return fail(error);
            }
            lock().emitted = at + 1;
            changed.notify_all();
        }
    });
    match progress
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .failure
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
