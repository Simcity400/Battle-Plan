
mod turn_profile;

pub fn name_shared_pool() {
    let _ = rayon::ThreadPoolBuilder::new()
        .thread_name(|at| format!("shared rayon {at}"))
        .build_global();
}

pub use content_reader_layers::parallel::usable_processors;

use std::sync::MutexGuard;
use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use turn_profile::{Batch, Counts};

pub type Beside<'a> = dyn Fn(&dyn Fn() -> bool) + Sync + 'a;

pub trait Spawn: Send + Sync {
    fn try_spawn(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), Box<dyn FnOnce() + Send>>;
    fn threads(&self) -> usize;
}

#[derive(Clone)]
pub struct Workers {
    pool: Option<Arc<dyn Spawn>>,
    threads: usize,
}

impl Default for Workers {
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
    pub fn new(pool: Arc<dyn Spawn>) -> Self {
        let threads = (1 + pool.threads()).min(usable_processors()).max(1);
        Self {
            pool: Some(pool),
            threads,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn limited(mut self, threads: usize) -> Self {
        self.threads = self.threads.min(threads.max(1));
        self
    }

    pub fn threads(&self) -> usize {
        self.threads
    }

    pub fn join<A: Send, B: Send>(
        &self,
        left: impl FnOnce() -> A + Send,
        right: impl FnOnce() -> B + Send,
    ) -> (A, B) {
        if self.threads == 1 {
            return (left(), right());
        }
        let left = Mutex::new(Some(left));
        let right = Mutex::new(Some(right));
        let a = Mutex::new(None);
        let b = Mutex::new(None);
        fn take<T>(slot: &Mutex<Option<T>>) -> Option<T> {
            slot.lock().unwrap_or_else(|e| e.into_inner()).take()
        }
        self.run(2, &|at| match at {
            0 => *a.lock().unwrap_or_else(|e| e.into_inner()) = take(&left).map(|work| work()),
            _ => *b.lock().unwrap_or_else(|e| e.into_inner()) = take(&right).map(|work| work()),
        });
        (
            a.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("ran above"),
            b.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("ran above"),
        )
    }

    pub fn run(&self, count: usize, task: &(dyn Fn(usize) + Sync)) {
        self.run_beside(count, task, None);
    }

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
                match pool.try_spawn(Box::new(job_priority::carry(move || gate.help()))) {
                    Ok(()) => admitted.profile.accepted(),
                    Err(_) => break,
                }
            }
        }
        work(&admitted)
    }

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
                match pool.try_spawn(Box::new(job_priority::carry(move || turn.help()))) {
                    Ok(()) => profile.accepted(),
                    Err(_) => break,
                }
            }
        }));
        turns.finish(spawned.and_then(|()| catch_unwind(AssertUnwindSafe(|| turns.work(true)))));
    }
}

pub struct Admitted {
    capacity: usize,
    gate: Arc<Admission>,
    profile: Batch,
}

impl Admitted {
    pub fn capacity(&self) -> usize {
        self.capacity
    }

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

    pub fn run(&self, maximum: usize, task: &(dyn Fn(usize, usize) + Sync)) -> usize {
        self.run_beside(maximum, task, None)
    }

    pub fn run_beside(
        &self,
        maximum: usize,
        task: &(dyn Fn(usize, usize) + Sync),
        beside: Option<&Beside<'_>>,
    ) -> usize {
        self.run_turns(maximum, task, beside)
    }

    fn run_turns(
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
        let result = catch_unwind(AssertUnwindSafe(|| turns.work(true)));
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

struct Turns {
    next: AtomicUsize,
    count: usize,
    task: *const (dyn Fn(usize) + Sync),
    beside: Option<*const Beside<'static>>,
    beside_taken: AtomicBool,
    completed: AtomicUsize,
    failed: AtomicBool,
    priority: Option<i32>,
    state: Mutex<TurnState>,
    idle: Condvar,
    profile: Option<Arc<Counts>>,
}

// SAFETY: `task` is `Sync`, and the borrowed callback it points at outlives every call: both
// entry points (`Workers::run_beside` and `Admitted::run_turns`) keep it alive through
// `Turns::finish`, which closes admission and waits until no registered helper remains.
unsafe impl Send for Turns {}
unsafe impl Sync for Turns {}

#[derive(Default)]
struct TurnState {
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

    fn finish(&self, own: std::thread::Result<()>) {
        if own.is_err() {
            self.failed.store(true, Ordering::Release);
        }
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

    fn work(&self, caller: bool) {
        loop {
            let at = self.next.fetch_add(1, Ordering::Relaxed);
            if at >= self.count {
                return;
            }
            if caller && let Some(counts) = &self.profile {
                counts.caller.fetch_add(1, Ordering::Relaxed);
            }
            // SAFETY: the calling thread (`caller`) runs this synchronously inside
            // `Workers::run_beside` or `Admitted::run_turns`, which hold the borrowed callback
            // throughout. A helper registered before `Turns::finish` closed admission and may run
            // after the close, but `finish` waits until no helper is registered, and both entry
            // points keep the callback alive through that wait.
            unsafe { (*self.task)(at) };
            self.completed.fetch_add(1, Ordering::Release);
        }
    }

    fn done(&self) -> bool {
        self.completed.load(Ordering::Acquire) >= self.count || self.failed.load(Ordering::Acquire)
    }

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

    pub(crate) fn threads(count: usize) -> Arc<Threads> {
        let (sender, receiver) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..count {
            let receiver = Arc::clone(&receiver);
            job_priority::spawn(move || {
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
            let barrier = std::sync::Barrier::new(count);
            workers.run(count, &|_| {
                barrier.wait();
            });
        }
    }

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
