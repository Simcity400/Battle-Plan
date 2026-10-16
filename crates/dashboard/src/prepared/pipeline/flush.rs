//! Block-end flushing on scoped threads. Each family flushes into its own fork
//! of the block's store writer; the coordinator then adopts every step's writes
//! and applies its refusal check and timings in the fixed serial order, so the
//! store's files, manifest and sections are exactly those of serial flushing.
use super::{FamilyTimings, Refused, finish_timing, refuses, start_timing};
use std::{
    any::Any,
    collections::BTreeMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{Mutex, PoisonError},
};

type Timings = Option<BTreeMap<&'static str, FamilyTimings>>;

/// A step's result. `Err` stops preparation, as a `?` in serial flushing does;
/// `Ok(None)` is work without a refusal check; `Ok(Some(result))` goes through
/// the family's refusal check, as `family!` does.
pub(super) type Outcome = io::Result<Option<io::Result<()>>>;

type Run<'a> = dyn FnMut(usize, &mut store::Writer, &mut Timings) -> Outcome + Send + 'a;

/// One family's flush steps: each step's serial position and the family whose
/// refusal skips it. A task's steps run in order on one thread, so each sees
/// the task's earlier writes and refusals. A family whose refusal reaches a
/// later step's family (its `FAMILIES` dependents) must share that task.
pub(super) struct Task<'a> {
    steps: &'static [(usize, &'static str)],
    run: Box<Run<'a>>,
}

impl<'a> Task<'a> {
    /// `run(step, writer, timings)` performs `steps[step]`.
    pub(super) fn new(
        steps: &'static [(usize, &'static str)],
        run: impl FnMut(usize, &mut store::Writer, &mut Timings) -> Outcome + Send + 'a,
    ) -> Self {
        Self {
            steps,
            run: Box::new(run),
        }
    }
}

/// Flushes every task. With one thread, steps run in serial order straight into
/// `writer`, timed in `timings`. Otherwise tasks run concurrently, each into a
/// fork of `writer`; their steps are timed in `workers`, as lane work is, and
/// the whole parallel flush once in `timings`.
pub(super) fn run(
    tasks: Vec<Task<'_>>,
    threads: usize,
    writer: &mut store::Writer,
    timings: &mut Timings,
    workers: &mut Timings,
    refused: &mut Refused,
    hour: i32,
) -> io::Result<()> {
    if threads <= 1 || tasks.len() <= 1 {
        return serial(tasks, writer, timings, refused, hour);
    }
    let started = start_timing(timings);
    let result = parallel(tasks, threads, writer, workers, refused, hour);
    finish_timing(timings, started, "flush.parallel", true);
    result
}

fn serial(
    mut tasks: Vec<Task<'_>>,
    writer: &mut store::Writer,
    timings: &mut Timings,
    refused: &mut Refused,
    hour: i32,
) -> io::Result<()> {
    let mut order: Vec<_> = tasks
        .iter()
        .enumerate()
        .flat_map(|(task, value)| {
            value
                .steps
                .iter()
                .enumerate()
                .map(move |(step, &(position, family))| (position, task, step, family))
        })
        .collect();
    order.sort_unstable();
    #[cfg(test)]
    let knobs = Knobs::current();
    for (_position, task, step, family) in order {
        if refused.live(family) {
            #[cfg(test)]
            knobs.record(_position, hour);
            if let Some(result) = (tasks[task].run)(step, writer, timings)? {
                #[cfg(test)]
                let result = knobs.refuse(family, hour, result);
                refused.check(family, hour, result)?;
            }
        }
    }
    Ok(())
}

/// A step a worker ran or skipped, with its writes for adoption.
struct Done {
    position: usize,
    family: &'static str,
    /// `None` when the family had refused, so the step did not run.
    outcome: Option<Result<Outcome, Box<dyn Any + Send>>>,
    staged: Option<store::Staged>,
    timings: Timings,
}

fn parallel(
    tasks: Vec<Task<'_>>,
    threads: usize,
    writer: &mut store::Writer,
    timings: &mut Timings,
    refused: &mut Refused,
    hour: i32,
) -> io::Result<()> {
    let measured = timings.is_some();
    // Refusals injected at this hour reach the families on every thread.
    #[cfg(test)]
    let injected: Vec<_> = tasks
        .iter()
        .flat_map(|task| task.steps.iter().map(|&(_, family)| family))
        .filter(|family| super::lanes::inject(family, hour, Ok(())).is_err())
        .map(|family| (family, hour))
        .collect();
    #[cfg(not(test))]
    let injected: Vec<(&'static str, i32)> = Vec::new();
    #[cfg(test)]
    let knobs = Knobs::current();
    let helpers = threads.min(tasks.len()) - 1;
    let mut work = Vec::with_capacity(tasks.len());
    for task in tasks {
        let fork = writer.fork()?;
        work.push((task, fork));
    }
    let queue = Mutex::new(work.into_iter());
    let snapshot: &Refused = refused;
    // A recording test holds every thread at its first task until all hold
    // one, so the steps provably spread over the threads.
    #[cfg(test)]
    let gate = knobs
        .record
        .is_some()
        .then(|| std::sync::Barrier::new(helpers + 1));
    let worker = || {
        let mut done: Vec<Done> = Vec::new();
        loop {
            let next = queue.lock().unwrap_or_else(PoisonError::into_inner).next();
            let Some((mut task, mut fork)) = next else {
                return done;
            };
            #[cfg(test)]
            if done.is_empty()
                && let Some(gate) = &gate
            {
                gate.wait();
            }
            let mut local = snapshot.clone();
            let first = done.len();
            for (step, &(position, family)) in task.steps.iter().enumerate() {
                let mut step_timings = measured.then(BTreeMap::new);
                let mut stop = false;
                let outcome = local.live(family).then(|| {
                    #[cfg(test)]
                    knobs.record(position, hour);
                    let ran = catch_unwind(AssertUnwindSafe(|| {
                        (task.run)(step, &mut fork, &mut step_timings)
                    }));
                    match ran {
                        Ok(Ok(Some(result))) => {
                            #[cfg(test)]
                            let result = super::lanes::inject(family, hour, result);
                            #[cfg(test)]
                            let result = knobs.refuse(family, hour, result);
                            match &result {
                                Err(error) if refuses(error) => {
                                    local.refuse(family, hour, error.to_string(), None);
                                }
                                Err(_) => stop = true,
                                Ok(()) => {}
                            }
                            Ok(Ok(Some(result)))
                        }
                        Ok(other) => {
                            stop = other.is_err();
                            Ok(other)
                        }
                        Err(panic) => {
                            stop = true;
                            Err(panic)
                        }
                    }
                });
                fork.end_batch();
                done.push(Done {
                    position,
                    family,
                    outcome,
                    staged: None,
                    timings: step_timings,
                });
                if stop {
                    break;
                }
            }
            // One batch per recorded step, in order.
            match fork.into_staged() {
                Ok(staged) => {
                    for (done, staged) in done[first..].iter_mut().zip(staged) {
                        done.staged = Some(staged);
                    }
                }
                Err(error) => done[first].outcome = Some(Ok(Err(error))),
            }
        }
    };
    let mut done = std::thread::scope(|scope| {
        let (worker, injected) = (&worker, &injected);
        // Helpers skip and keep the families the caller's registry does.
        #[cfg(test)]
        let registry = super::independent::test_registry();
        let helpers: Vec<_> = (0..helpers)
            .map(|_| {
                scope.spawn(move || {
                    #[cfg(test)]
                    registry.install();
                    with_injected(injected, worker)
                })
            })
            .collect();
        let mut done = worker();
        for helper in helpers {
            match helper.join() {
                Ok(more) => done.extend(more),
                Err(panic) => resume_unwind(panic),
            }
        }
        done
    });
    // Serial order: adopt each step's writes, then record its timings and
    // apply its refusal check. The first error or panic in that order wins.
    done.sort_unstable_by_key(|done| done.position);
    for done in done {
        let Some(outcome) = done.outcome else {
            if refused.live(done.family) {
                return Err(diverged(done.family));
            }
            continue;
        };
        if !refused.live(done.family) {
            return Err(diverged(done.family));
        }
        let outcome = outcome.unwrap_or_else(|panic| resume_unwind(panic));
        if let Some(staged) = done.staged {
            writer.adopt(staged)?;
        }
        super::lanes::merge(timings, done.timings);
        if let Some(result) = outcome? {
            refused.check(done.family, hour, result)?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn with_injected<T>(injected: &[(&'static str, i32)], run: impl FnOnce() -> T) -> T {
    super::lanes::with_test_refusals(injected, run)
}

#[cfg(not(test))]
fn with_injected<T>(_: &[(&'static str, i32)], run: impl FnOnce() -> T) -> T {
    run()
}

/// A task's own refusal record disagreed with serial order: a refusal reached
/// this family from a family flushed in another task.
fn diverged(family: &str) -> io::Error {
    io::Error::other(format!(
        "Parallel flush of {family} diverged from its serial refusal state"
    ))
}

#[cfg(test)]
type Recorded = std::sync::Arc<Mutex<Vec<(usize, i32, std::thread::ThreadId)>>>;

#[cfg(test)]
thread_local! {
    static RECORD: std::cell::RefCell<Option<Recorded>> = const { std::cell::RefCell::new(None) };
    static FLUSH_REFUSALS: std::cell::RefCell<Vec<(&'static str, i32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The test's step recorder and flush-only refusals, taken from the
/// coordinator's thread so that every flush thread applies them.
#[cfg(test)]
struct Knobs {
    record: Option<Recorded>,
    refusals: Vec<(&'static str, i32)>,
}

#[cfg(test)]
impl Knobs {
    fn current() -> Self {
        Self {
            record: RECORD.with(|record| record.borrow().clone()),
            refusals: FLUSH_REFUSALS.with(|refusals| refusals.borrow().clone()),
        }
    }

    fn record(&self, position: usize, hour: i32) {
        if let Some(record) = &self.record {
            record
                .lock()
                .unwrap()
                .push((position, hour, std::thread::current().id()));
        }
    }

    fn refuse(&self, family: &str, hour: i32, result: io::Result<()>) -> io::Result<()> {
        if result.is_ok()
            && self
                .refusals
                .iter()
                .any(|&(refused, at)| refused == family && at == hour)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Injected flush refusal of {family} at hour {hour}"),
            ));
        }
        result
    }
}

/// Runs `run`, returning every flush step it ran: serial position, flush hour
/// and thread, in the order the steps started.
#[cfg(test)]
pub(super) fn with_recorded_steps<T>(
    run: impl FnOnce() -> T,
) -> (T, Vec<(usize, i32, std::thread::ThreadId)>) {
    struct Reset(Option<Recorded>);
    impl Drop for Reset {
        fn drop(&mut self) {
            RECORD.with(|record| *record.borrow_mut() = self.0.take());
        }
    }
    let recorded = Recorded::default();
    let reset = Reset(RECORD.with(|record| record.replace(Some(recorded.clone()))));
    let value = run();
    drop(reset);
    let steps = std::mem::take(&mut *recorded.lock().unwrap());
    (value, steps)
}

/// Tests refuse a family's checked flush step at a flush hour, as its own
/// `InvalidData` would, without refusing the hour's observation.
#[cfg(test)]
pub(super) fn with_flush_refusals<T>(
    refusals: &[(&'static str, i32)],
    run: impl FnOnce() -> T,
) -> T {
    struct Reset(Vec<(&'static str, i32)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            FLUSH_REFUSALS.with(|refusals| *refusals.borrow_mut() = std::mem::take(&mut self.0));
        }
    }
    let _reset = Reset(FLUSH_REFUSALS.with(|value| value.replace(refusals.to_vec())));
    run()
}
