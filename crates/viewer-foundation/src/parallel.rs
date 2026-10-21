//! Order-preserving parallel map over a bounded number of worker threads.
//!
//! Content loads read and decode one file per item, and a modded content set
//! can hold tens of thousands of flag, portrait or country files. Spawning a
//! thread per file creates that many OS threads at once, and a spawn the OS
//! refuses panics the load. These helpers keep the worker count at the
//! machine's parallelism and return the results in input order, so callers
//! merge them exactly as a serial load would.
//!
//! [`Workers`] lends a batch an existing bounded pool. Synchronous frame work
//! can use protected idle helpers before borrowing hour builders.

mod turn_profile;

use std::sync::MutexGuard;
use std::{
    any::Any,
    num::NonZeroUsize,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};
use turn_profile::{Batch, Counts};

/// Coarse caller work and join time for one borrowed turn batch. No scheduling changes.
#[derive(Clone, Copy, Default)]
pub struct RunProfile {
    pub own: std::time::Duration,
    pub wait: std::time::Duration,
}
thread_local! {
    static RUN_PROFILE: std::cell::Cell<Option<RunProfile>> = const { std::cell::Cell::new(None) };
}

pub fn profile_run<R>(enabled: bool, work: impl FnOnce() -> R) -> (R, RunProfile) {
    if !enabled {
        return (work(), RunProfile::default());
    }
    struct Restore(Option<RunProfile>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RUN_PROFILE.set(self.0);
        }
    }
    let _restore = Restore(RUN_PROFILE.replace(Some(RunProfile::default())));
    let result = work();
    (result, RUN_PROFILE.get().unwrap_or_default())
}

fn profile_clock() -> Option<std::time::Instant> {
    RUN_PROFILE.get().map(|_| std::time::Instant::now())
}

fn profile_elapsed(clock: Option<std::time::Instant>, wait: bool) {
    if let Some(clock) = clock {
        RUN_PROFILE.with(|slot| {
            slot.set(slot.get().map(|mut profile| {
                if wait {
                    profile.wait += clock.elapsed();
                } else {
                    profile.own += clock.elapsed();
                }
                profile
            }));
        });
    }
}

/// Maps `items` through `work` on at most `available_parallelism` threads,
/// returning the results in input order.
///
/// Items are split into contiguous runs, one per worker. A worker the OS
/// refuses to start leaves its run to the calling thread instead of failing.
pub fn map<T, R, F>(items: &[T], work: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let workers = thread::available_parallelism()
        .map_or(4, NonZeroUsize::get)
        .min(items.len());
    if workers <= 1 {
        return items.iter().map(&work).collect();
    }
    let run = items.len().div_ceil(workers);
    let work = &work;
    thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(run)
            .map(|chunk| {
                let spawned = thread::Builder::new()
                    .spawn_scoped(scope, move || chunk.iter().map(work).collect::<Vec<R>>());
                (chunk, spawned.ok())
            })
            .collect();
        let mut out = Vec::with_capacity(items.len());
        for (chunk, handle) in handles {
            match handle {
                Some(handle) => match handle.join() {
                    Ok(results) => out.extend(results),
                    Err(panic) => std::panic::resume_unwind(panic),
                },
                None => out.extend(chunk.iter().map(work)),
            }
        }
        out
    })
}

/// The logical processors this process may run on: its affinity mask's,
/// never the machine's count when the process is limited to fewer.
pub fn usable_processors() -> usize {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};
        let (mut process, mut system) = (0usize, 0usize);
        // SAFETY: the pseudo handle needs no closing, and both out pointers
        // are valid for the call.
        let read =
            unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) };
        if read != 0 && process != 0 {
            return process.count_ones() as usize;
        }
    }
    thread::available_parallelism().map_or(1, NonZeroUsize::get)
}

/// A job beside a run's tasks ([`Workers::run_beside`]): given whether the
/// tasks are all done.
pub type Beside<'a> = dyn Fn(&dyn Fn() -> bool) + Sync + 'a;

/// The service that accepted a helper, for opt-in per-batch accounting.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperSource {
    Protected,
    Builder,
}

/// A pool that takes jobs without waiting.
pub trait Spawn: Send + Sync {
    /// Queues `job`, or gives it back when the pool cannot take it now.
    fn try_spawn(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), Box<dyn FnOnce() + Send>>;
    /// The same admission operation, identifying protected frame capacity.
    #[doc(hidden)]
    fn try_spawn_classified(
        &self,
        job: Box<dyn FnOnce() + Send>,
    ) -> Result<HelperSource, Box<dyn FnOnce() + Send>> {
        self.try_spawn(job).map(|()| HelperSource::Builder)
    }
    /// The pool's threads.
    fn threads(&self) -> usize;
}

/// Threads a batch may use: the calling thread and, while they are free,
/// those of a shared pool, at most as many in all as the process has
/// usable processors ([`usable_processors`]).
#[derive(Clone)]
pub struct Workers {
    pool: Option<Arc<dyn Spawn>>,
    threads: usize,
}

impl Default for Workers {
    /// The calling thread alone.
    fn default() -> Self {
        Self {
            pool: None,
            threads: 1,
        }
    }
}

impl std::fmt::Debug for Workers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workers")
            .field("threads", &self.threads)
            .finish()
    }
}

impl Workers {
    /// The calling thread and `pool`'s.
    pub fn new(pool: Arc<dyn Spawn>) -> Self {
        let threads = (1 + pool.threads()).min(usable_processors()).max(1);
        Self {
            pool: Some(pool),
            threads,
        }
    }

    /// At most `threads` in all, for tests and benches.
    #[cfg(any(test, feature = "test-support"))]
    pub fn limited(mut self, threads: usize) -> Self {
        self.threads = self.threads.min(threads.max(1));
        self
    }

    /// The threads a batch may use, the calling one included.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Runs `task(0..count)`, each index once, on the calling thread and on
    /// up to `threads - 1` pool threads that take a turn while tasks remain;
    /// returns once every task has run. Tasks are taken in index order, and
    /// whichever thread runs one does not change what it computes. A pool
    /// thread runs its turn at the calling thread's priority, when that is
    /// higher than its own: the caller waits for the tasks it took. A task's
    /// panic is resumed here once no pool thread runs a task any more.
    pub fn run(&self, count: usize, task: &(dyn Fn(usize) + Sync)) {
        self.run_beside(count, task, None);
    }

    /// Lends idle helpers before caller-side preparation, without waiting for busy builders.
    /// The callback runs here; its [`Admitted::run`] sizes tasks from helpers that actually
    /// reached the admission gate during preparation, not jobs still queued on the pool.
    pub fn with_admitted<R>(&self, maximum: usize, work: impl FnOnce(&Admitted) -> R) -> R {
        let admitted = Admitted {
            capacity: maximum.min(self.threads),
            profile: Batch::new("admitted", maximum),
            gate: Arc::new(Admission {
                state: Mutex::new(AdmissionState {
                    open: true,
                    waiting: 0,
                    turns: None,
                }),
                ready: Condvar::new(),
            }),
        };
        if let Some(pool) = &self.pool {
            for _ in 0..admitted.capacity.saturating_sub(1) {
                let gate = Arc::clone(&admitted.gate);
                match pool.try_spawn_classified(Box::new(move || gate.help())) {
                    Ok(source) => admitted.profile.accepted(source),
                    Err(_) => break,
                }
            }
        }
        work(&admitted)
    }

    /// [`Self::run`], with `beside` run on one pool thread, never the
    /// calling one, if a pool thread takes a turn: it is given whether the
    /// tasks are all done and should return soon once they are. It may not
    /// run at all: what it does must not change what the tasks compute. Returns
    /// once every task and any started job beside them has finished.
    pub fn run_beside(
        &self,
        count: usize,
        task: &(dyn Fn(usize) + Sync),
        beside: Option<&Beside<'_>>,
    ) {
        let profile = Batch::new("run", count);
        let wanted = if beside.is_some() {
            count
        } else {
            count.saturating_sub(1)
        };
        let helpers = self.threads.saturating_sub(1).min(wanted);
        let pool = match &self.pool {
            Some(pool) if helpers > 0 && count > 0 => pool,
            _ => {
                if let Some(counts) = &profile.counts {
                    counts.caller.store(count, Ordering::Relaxed);
                }
                (0..count).for_each(task);
                return;
            }
        };
        let turns = Turns::new(count, task, beside, profile.counts.clone());
        let spawned = catch_unwind(AssertUnwindSafe(|| {
            for _ in 0..helpers {
                let turn = Arc::clone(&turns);
                match pool.try_spawn_classified(Box::new(move || turn.help())) {
                    Ok(source) => profile.accepted(source),
                    Err(_) => break,
                }
            }
        }));
        turns.finish(spawned.and_then(|()| catch_unwind(AssertUnwindSafe(|| turns.work(true)))));
    }
}

/// One caller-side preparation scope and its idle helper turns. Call [`Self::run`] or
/// [`Self::run_beside`] once, after preparation. Dropping the scope without running releases every waiting helper.
pub struct Admitted {
    capacity: usize,
    gate: Arc<Admission>,
    profile: Batch,
}

impl Admitted {
    /// Upper bound from the caller's requested work and [`Workers::threads`].
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Waits for a fixture pool's helpers to reach this scope. Only use when the fixture
    /// guarantees those jobs start independently of the caller; ordinary batches never wait.
    #[cfg(any(test, feature = "test-support"))]
    pub fn wait_for_helpers(&self, count: usize) {
        assert!(count <= self.capacity.saturating_sub(1));
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.waiting < count {
            state = self
                .gate
                .ready
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Runs `task(index, count)` for each contiguous index below the final count: at most
    /// `maximum`, the scope's capacity, and one plus the helpers already waiting. A busy
    /// pool runs one task in place. Returns the count; zero work runs no task.
    pub fn run(&self, maximum: usize, task: &(dyn Fn(usize, usize) + Sync)) -> usize {
        self.run_beside(maximum, task, None)
    }

    /// [`Self::run`], with a job beside the tasks under [`Workers::run_beside`]'s
    /// contract. Even an admitted helper may not take a turn before the caller finishes,
    /// so callers must handle a job that did not run.
    pub fn run_beside(
        &self,
        maximum: usize,
        task: &(dyn Fn(usize, usize) + Sync),
        beside: Option<&Beside<'_>>,
    ) -> usize {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.open, "admitted batch already run");
        let count = maximum.min(self.capacity).min(state.waiting + 1);
        let task = |index| task(index, count);
        if let Some(counts) = &self.profile.counts {
            counts
                .admitted
                .store(count.saturating_sub(1), Ordering::Relaxed);
        }
        let turns = Turns::new(count, &task, beside, self.profile.counts.clone());
        state.open = false;
        state.turns = Some(Arc::clone(&turns));
        drop(state);
        self.gate.ready.notify_all();
        let own = profile_clock();
        let result = catch_unwind(AssertUnwindSafe(|| turns.work(true)));
        profile_elapsed(own, false);
        turns.finish(result);
        count
    }
}

impl Drop for Admitted {
    fn drop(&mut self) {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        state.open = false;
        state.turns = None;
        drop(state);
        self.gate.ready.notify_all();
    }
}

struct Admission {
    state: Mutex<AdmissionState>,
    ready: Condvar,
}

struct AdmissionState {
    open: bool,
    waiting: usize,
    turns: Option<Arc<Turns>>,
}

impl Admission {
    fn help(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Busy builders may reach this job after the batch or even the scope ended.
        if !state.open {
            return;
        }
        state.waiting += 1;
        self.ready.notify_all();
        while state.open {
            state = self.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        let turns = state.turns.clone();
        drop(state);
        if let Some(turns) = turns {
            turns.help();
        }
    }
}

/// One [`Workers::run`]: the next task and who is running tasks.
struct Turns {
    next: AtomicUsize,
    count: usize,
    task: *const (dyn Fn(usize) + Sync),
    /// Run beside the tasks by the first pool thread that takes a turn.
    beside: Option<*const Beside<'static>>,
    beside_taken: AtomicBool,
    /// Tasks run to their end.
    completed: AtomicUsize,
    /// A task panicked: the rest may never complete.
    failed: AtomicBool,
    /// The calling thread's priority.
    priority: Option<i32>,
    state: Mutex<TurnState>,
    idle: Condvar,
    profile: Option<Arc<Counts>>,
}

// SAFETY: `task` is `Sync` and is called only as `Workers::run` allows.
unsafe impl Send for Turns {}
// SAFETY: as above; the other fields are `Sync`.
unsafe impl Sync for Turns {}

#[derive(Default)]
struct TurnState {
    /// No pool thread may start running tasks any more.
    closed: bool,
    running: usize,
    panic: Option<Box<dyn Any + Send>>,
}

impl Turns {
    fn new(
        count: usize,
        task: &(dyn Fn(usize) + Sync),
        beside: Option<&Beside<'_>>,
        profile: Option<Arc<Counts>>,
    ) -> Arc<Self> {
        // SAFETY: the lifetime is erased only for the pool's jobs, which run
        // `task` only while registered in `Turns::state` before it is closed.
        // Both callers finish this run before the borrowed callbacks expire:
        // `finish` closes it and waits until no job is registered.
        let task: *const (dyn Fn(usize) + Sync + 'static) = unsafe {
            std::mem::transmute::<*const (dyn Fn(usize) + Sync + '_), *const (dyn Fn(usize) + Sync)>(
                task,
            )
        };
        // SAFETY: as for `task`.
        let beside = beside.map(|beside| unsafe {
            std::mem::transmute::<*const Beside<'_>, *const Beside<'static>>(beside)
        });
        Arc::new(Self {
            next: AtomicUsize::new(0),
            count,
            task,
            beside,
            beside_taken: AtomicBool::new(false),
            completed: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
            priority: crate::worker_priority::current_priority(),
            state: Mutex::default(),
            idle: Condvar::new(),
            profile,
        })
    }

    /// Close before returning or unwinding, so delayed pool jobs cannot call the borrowed task.
    fn finish(&self, own: std::thread::Result<()>) {
        if own.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        let wait = profile_clock();
        let mut state = self.lock();
        state.closed = true;
        while state.running > 0 {
            state = self
                .idle
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        let helper = state.panic.take();
        drop(state);
        profile_elapsed(wait, true);
        if let Err(panic) = own {
            resume_unwind(panic);
        }
        if let Some(panic) = helper {
            resume_unwind(panic);
        }
    }

    fn lock(&self) -> MutexGuard<'_, TurnState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs tasks until none remains.
    fn work(&self, caller: bool) {
        loop {
            let at = self.next.fetch_add(1, Ordering::Relaxed);
            if at >= self.count {
                return;
            }
            if caller && let Some(counts) = &self.profile {
                counts.caller.fetch_add(1, Ordering::Relaxed);
            }
            // SAFETY: called only while `Workers::run` has not returned
            // (see there).
            unsafe { (*self.task)(at) };
            self.completed.fetch_add(1, Ordering::Release);
        }
    }

    /// Whether every task has run, or one panicked.
    fn done(&self) -> bool {
        self.completed.load(Ordering::Acquire) >= self.count || self.failed.load(Ordering::Acquire)
    }

    /// A pool thread's turn: tasks until none remains, unless the run is
    /// already over.
    fn help(&self) {
        {
            let mut state = self.lock();
            if state.closed {
                return;
            }
            state.running += 1;
        }
        if let Some(counts) = &self.profile {
            counts.started.fetch_add(1, Ordering::Relaxed);
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            crate::worker_priority::at_least(self.priority, || {
                if let Some(beside) = self.beside
                    && !self.beside_taken.swap(true, Ordering::AcqRel)
                {
                    if let Some(counts) = &self.profile {
                        counts.beside.fetch_add(1, Ordering::Relaxed);
                    }
                    // SAFETY: as for `task`.
                    unsafe { (*beside)(&|| self.done()) };
                }
                self.work(false);
            });
        }));
        let mut state = self.lock();
        if let Err(panic) = result {
            self.next.store(self.count, Ordering::Relaxed);
            self.failed.store(true, Ordering::Release);
            state.panic.get_or_insert(panic);
        }
        state.running -= 1;
        drop(state);
        self.idle.notify_all();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    #[test]
    fn results_keep_the_input_order_for_many_items() {
        let items: Vec<u32> = (0..75_000).collect();
        let out = super::map(&items, |value| u64::from(*value) * 3);
        assert_eq!(out.len(), items.len());
        assert!(out.iter().enumerate().all(|(i, v)| *v == i as u64 * 3));
        assert!(super::map(&[] as &[u32], |value| *value).is_empty());
        assert_eq!(super::map(&[7u8], |value| *value + 1), [8]);
    }

    /// A pool of plain threads for the tests.
    pub(crate) struct Threads(
        Mutex<std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>>,
        usize,
    );

    impl super::Spawn for Threads {
        fn try_spawn(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), Box<dyn FnOnce() + Send>> {
            self.0.lock().unwrap().send(job).map_err(|error| error.0)
        }
        fn threads(&self) -> usize {
            self.1
        }
    }

    /// `count` plain threads taking jobs until the pool is dropped.
    pub(crate) fn threads(count: usize) -> Arc<Threads> {
        let (sender, receiver) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..count {
            let receiver = Arc::clone(&receiver);
            std::thread::spawn(move || {
                loop {
                    let job = receiver.lock().unwrap().recv();
                    match job {
                        Ok(job) => job(),
                        Err(_) => return,
                    }
                }
            });
        }
        Arc::new(Threads(Mutex::new(sender), count))
    }

    /// Every task runs once, whichever thread takes it.
    #[test]
    fn workers_run_every_task_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let pool = threads(3);
        for limit in 1..=4 {
            let workers = super::Workers::new(pool.clone()).limited(limit);
            for count in [0, 1, 2, 7, 100] {
                let seen: Vec<AtomicUsize> = (0..count).map(|_| AtomicUsize::new(0)).collect();
                workers.run(count, &|at| {
                    seen[at].fetch_add(1, Ordering::Relaxed);
                });
                assert!(seen.iter().all(|n| n.load(Ordering::Relaxed) == 1));
            }
        }
    }

    #[test]
    fn admitted_batches_size_from_accepted_helpers_and_keep_serial_empty_runs() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Admits {
            pool: Arc<Threads>,
            remaining: AtomicUsize,
        }
        impl super::Spawn for Admits {
            fn try_spawn(
                &self,
                job: Box<dyn FnOnce() + Send>,
            ) -> Result<(), Box<dyn FnOnce() + Send>> {
                if self
                    .remaining
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(1)
                    })
                    .is_err()
                {
                    return Err(job);
                }
                super::Spawn::try_spawn(&*self.pool, job)
            }
            fn threads(&self) -> usize {
                super::Spawn::threads(&*self.pool)
            }
        }
        let pool = threads(3);
        for accepted in [0, 1, 3] {
            for maximum in [0, 1, 2, 7] {
                let workers = super::Workers::new(Arc::new(Admits {
                    pool: Arc::clone(&pool),
                    remaining: AtomicUsize::new(accepted),
                }));
                let expected = maximum.min(workers.threads()).min(accepted + 1);
                let seen = Mutex::new(Vec::new());
                assert_eq!(
                    workers.with_admitted(maximum, |admitted| {
                        admitted.wait_for_helpers(expected.saturating_sub(1));
                        admitted.run(maximum, &|index, count| {
                            seen.lock().unwrap().push((index, count))
                        })
                    }),
                    expected
                );
                let mut seen = seen.into_inner().unwrap();
                seen.sort_unstable();
                assert_eq!(
                    seen,
                    (0..expected)
                        .map(|index| (index, expected))
                        .collect::<Vec<_>>()
                );
            }
        }
        for maximum in [0, 1, 7] {
            let seen = Mutex::new(Vec::new());
            let expected = maximum.min(1);
            assert_eq!(
                super::Workers::default().with_admitted(maximum, |admitted| admitted.run(
                    maximum,
                    &|index, count| {
                        seen.lock().unwrap().push((index, count));
                    }
                )),
                expected
            );
            assert_eq!(
                seen.into_inner().unwrap(),
                (0..expected)
                    .map(|index| (index, expected))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn admitted_helpers_share_one_count_and_finish_before_a_task_panic_returns() {
        let workers = super::Workers::new(threads(3));
        let count = workers.threads();
        let barrier = std::sync::Barrier::new(count);
        let seen = Mutex::new(Vec::new());
        assert_eq!(
            workers.with_admitted(count, |admitted| {
                admitted.wait_for_helpers(count - 1);
                admitted.run(count, &|index, admitted| {
                    assert_eq!(admitted, count);
                    // No timing assumption: all admitted helpers must enter before any finishes.
                    barrier.wait();
                    seen.lock()
                        .unwrap()
                        .push((index, std::thread::current().id()));
                })
            }),
            count
        );
        let mut seen = seen.into_inner().unwrap();
        seen.sort_by_key(|entry| entry.0);
        assert_eq!(
            seen.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            (0..count).collect::<Vec<_>>()
        );
        let ids: std::collections::HashSet<_> = seen.iter().map(|entry| entry.1).collect();
        assert_eq!(ids.len(), count);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            workers.with_admitted(count, |admitted| {
                admitted.run(count, &|index, actual| {
                    assert_ne!(index, actual - 1, "admitted task")
                })
            });
        }));
        assert!(caught.is_err());
        let seen = Mutex::new(Vec::new());
        let actual = workers.with_admitted(count, |admitted| {
            admitted.run(count, &|index, _| seen.lock().unwrap().push(index))
        });
        assert_eq!(seen.into_inner().unwrap().len(), actual);
    }

    #[test]
    fn admitted_beside_runs_on_a_helper_and_joins_before_returning() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let workers = super::Workers::new(threads(3));
        let count = workers.threads();
        let caller = std::thread::current().id();
        let beside = AtomicUsize::new(0);
        let barrier = std::sync::Barrier::new(count);
        workers.with_admitted(count, |admitted| {
            admitted.wait_for_helpers(count - 1);
            assert_eq!(
                admitted.run_beside(
                    count,
                    &|_, actual| {
                        assert_eq!(actual, count);
                        barrier.wait();
                    },
                    Some(&|_: &dyn Fn() -> bool| {
                        assert_ne!(std::thread::current().id(), caller);
                        beside.fetch_add(1, Ordering::Relaxed);
                    }),
                ),
                count
            );
        });
        assert_eq!(beside.into_inner(), usize::from(count > 1));
        for maximum in [0, 1, 7] {
            let seen = AtomicUsize::new(0);
            super::Workers::default().with_admitted(maximum, |admitted| {
                assert_eq!(
                    admitted.run_beside(
                        maximum,
                        &|_, _| {
                            seen.fetch_add(1, Ordering::Relaxed);
                        },
                        Some(&|_: &dyn Fn() -> bool| panic!("no admitted helper")),
                    ),
                    maximum.min(1)
                );
            });
            assert_eq!(seen.into_inner(), maximum.min(1));
        }
        if count > 1 {
            let entered = std::sync::Barrier::new(2);
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                workers.with_admitted(count, |admitted| {
                    admitted.wait_for_helpers(count - 1);
                    admitted.run_beside(
                        count,
                        &|index, _| {
                            if index == 0 {
                                entered.wait();
                            }
                        },
                        Some(&|_: &dyn Fn() -> bool| {
                            entered.wait();
                            panic!("admitted beside");
                        }),
                    );
                });
            }));
            assert!(caught.is_err());
        }
    }

    #[test]
    fn accepted_jobs_that_start_after_admission_returns_do_not_call_the_borrowed_task() {
        #[derive(Default)]
        struct Queued(Mutex<Vec<Box<dyn FnOnce() + Send>>>);
        impl super::Spawn for Queued {
            fn try_spawn(
                &self,
                job: Box<dyn FnOnce() + Send>,
            ) -> Result<(), Box<dyn FnOnce() + Send>> {
                self.0.lock().unwrap().push(job);
                Ok(())
            }
            fn threads(&self) -> usize {
                3
            }
        }
        let pool = Arc::new(Queued::default());
        let workers = super::Workers::new(pool.clone());
        let seen = Mutex::new(Vec::new());
        let count = workers.with_admitted(workers.threads(), |admitted| {
            admitted.run_beside(
                workers.threads(),
                &|index, _| seen.lock().unwrap().push(index),
                Some(&|_: &dyn Fn() -> bool| panic!("late helper called beside")),
            )
        });
        assert_eq!(count, 1);
        assert_eq!(seen.lock().unwrap().len(), count);
        for job in std::mem::take(&mut *pool.0.lock().unwrap()) {
            job();
        }
        assert_eq!(seen.into_inner().unwrap().len(), count);
    }

    #[test]
    fn abandoning_preparation_releases_helpers_on_return_and_unwind() {
        let workers = super::Workers::new(threads(3));
        let count = workers.threads();
        for panic in [false, true] {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                workers.with_admitted(count, |admitted| {
                    admitted.wait_for_helpers(count - 1);
                    assert!(!panic, "preparation refused");
                });
            }));
            assert_eq!(result.is_err(), panic);
            // Every pool thread must be available again; no clock or timeout decides that.
            let barrier = std::sync::Barrier::new(count);
            workers.run(count, &|_| {
                barrier.wait();
            });
        }
    }

    /// A job beside the tasks runs on a pool thread, only while they run,
    /// and stops when they are done; without a free pool thread it does not
    /// run, and the tasks all run all the same.
    #[test]
    fn a_job_beside_runs_only_while_the_tasks_run() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let workers = super::Workers::new(threads(2));
        let (ran, beside) = (AtomicUsize::new(0), AtomicBool::new(false));
        let caller = std::thread::current().id();
        workers.run_beside(
            8,
            &|_| {
                std::thread::sleep(Duration::from_millis(2));
                ran.fetch_add(1, Ordering::Relaxed);
            },
            Some(&|done: &dyn Fn() -> bool| {
                assert_ne!(std::thread::current().id(), caller);
                beside.store(true, Ordering::Relaxed);
                while !done() {
                    std::hint::spin_loop();
                }
            }),
        );
        assert_eq!(ran.load(Ordering::Relaxed), 8);
        let alone = super::Workers::default();
        let ran = AtomicUsize::new(0);
        alone.run_beside(
            3,
            &|_| {
                ran.fetch_add(1, Ordering::Relaxed);
            },
            Some(&|_: &dyn Fn() -> bool| panic!("no pool thread")),
        );
        assert_eq!(ran.into_inner(), 3);
    }

    /// A task's panic reaches the caller once the pool's threads stopped,
    /// and the pool runs later batches.
    #[test]
    fn a_task_panic_reaches_the_caller() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let workers = super::Workers::new(threads(2));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            workers.run(64, &|at| assert!(at != 40, "task 40"));
        }));
        assert!(caught.is_err());
        let ran = AtomicUsize::new(0);
        workers.run(8, &|_| {
            ran.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(ran.into_inner(), 8);
    }
}
