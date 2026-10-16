//! Order-preserving parallel map over a bounded number of worker threads.
//!
//! Content loads read and decode one file per item, and a modded content set
//! can hold tens of thousands of flag, portrait or country files. Spawning a
//! thread per file creates that many OS threads at once, and a spawn the OS
//! refuses panics the load. These helpers keep the worker count at the
//! machine's parallelism and return the results in input order, so callers
//! merge them exactly as a serial load would.
//!
//! [`Workers`] lends a batch the threads of a pool that already exists (the
//! hour builders'), so work split across threads adds none.

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

/// A pool that takes jobs without waiting.
pub trait Spawn: Send + Sync {
    /// Queues `job`, or gives it back when the pool cannot take it now.
    fn try_spawn(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), Box<dyn FnOnce() + Send>>;
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

    /// [`Self::run`], with `beside` run on one pool thread, never the
    /// calling one, if a pool thread takes a turn: it is given whether the
    /// tasks are all done and should return soon once they are. It runs
    /// only while tasks run, so it takes no time of its own and may not run
    /// at all: what it does must not change what the tasks compute.
    pub fn run_beside(
        &self,
        count: usize,
        task: &(dyn Fn(usize) + Sync),
        beside: Option<&Beside<'_>>,
    ) {
        let wanted = if beside.is_some() {
            count
        } else {
            count.saturating_sub(1)
        };
        let helpers = self.threads.saturating_sub(1).min(wanted);
        let pool = match &self.pool {
            Some(pool) if helpers > 0 && count > 0 => pool,
            _ => {
                (0..count).for_each(task);
                return;
            }
        };
        // SAFETY: the lifetime is erased only for the pool's jobs, which run
        // `task` only while registered in `Turns::state` before it is closed,
        // and this call does not return (or unwind) until it is closed with
        // no job registered.
        let task: *const (dyn Fn(usize) + Sync + 'static) = unsafe {
            std::mem::transmute::<*const (dyn Fn(usize) + Sync + '_), *const (dyn Fn(usize) + Sync)>(
                task,
            )
        };
        // SAFETY: as for `task`.
        let beside = beside.map(|beside| unsafe {
            std::mem::transmute::<*const Beside<'_>, *const Beside<'static>>(beside)
        });
        let turns = Arc::new(Turns {
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
        });
        for _ in 0..helpers {
            let turn = Arc::clone(&turns);
            if pool.try_spawn(Box::new(move || turn.help())).is_err() {
                break;
            }
        }
        let own = catch_unwind(AssertUnwindSafe(|| turns.work()));
        if own.is_err() {
            turns.failed.store(true, Ordering::Release);
        }
        let mut state = turns.lock();
        state.closed = true;
        while state.running > 0 {
            state = turns
                .idle
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        let helper = state.panic.take();
        drop(state);
        if let Err(panic) = own {
            resume_unwind(panic);
        }
        if let Some(panic) = helper {
            resume_unwind(panic);
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
    fn lock(&self) -> MutexGuard<'_, TurnState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs tasks until none remains.
    fn work(&self) {
        loop {
            let at = self.next.fetch_add(1, Ordering::Relaxed);
            if at >= self.count {
                return;
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
        let result = catch_unwind(AssertUnwindSafe(|| {
            crate::worker_priority::at_least(self.priority, || {
                if let Some(beside) = self.beside
                    && !self.beside_taken.swap(true, Ordering::AcqRel)
                {
                    // SAFETY: as for `task`.
                    unsafe { (*beside)(&|| self.done()) };
                }
                self.work();
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
mod tests {
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
