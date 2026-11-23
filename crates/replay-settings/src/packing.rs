
use std::{
    collections::VecDeque,
    io, panic,
    sync::{
        Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

type Gate = dyn Fn() + Send + Sync;

struct Budget {
    progress: Mutex<Option<crate::progress::Progress>>,
    background: std::sync::Arc<std::sync::atomic::AtomicBool>,
    io: Mutex<std::time::Instant>,
    catch_up: std::sync::atomic::AtomicBool,
    io_wait_ns: std::sync::atomic::AtomicU64,
    started: std::time::Instant,
    progress_due_ns: std::sync::atomic::AtomicU64,
    pool: std::sync::OnceLock<std::sync::Weak<rayon::ThreadPool>>,
    foreground: usize,
    priority_failed: std::sync::atomic::AtomicBool,
    promoted: Mutex<Option<std::time::Instant>>,
    timings: Mutex<[Option<std::time::Duration>; 5]>,
    io_bytes: std::sync::atomic::AtomicU64,
    gate: std::sync::OnceLock<std::sync::Arc<Gate>>,
}

thread_local! {
    static BUDGET: std::cell::RefCell<Option<std::sync::Arc<Budget>>> = const { std::cell::RefCell::new(None) };
}

pub struct Packing {
    pool: std::sync::Arc<rayon::ThreadPool>,
    budget: std::sync::Arc<Budget>,
}

pub(crate) fn progress() -> Option<crate::progress::Progress> {
    BUDGET.with_borrow(|budget| {
        budget
            .as_ref()
            .and_then(|budget| lock(&budget.progress).clone())
    })
}

impl Packing {
    pub fn new() -> io::Result<Self> {
        let foreground = (thread::available_parallelism().map_or(1, usize::from) / 2).max(1);
        Self::with_workers(foreground)
    }

    fn with_workers(foreground: usize) -> io::Result<Self> {
        let foreground = foreground.max(1);
        let budget = std::sync::Arc::new(Budget {
            progress: Mutex::new(None),
            background: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            io: Mutex::new(std::time::Instant::now()),
            catch_up: std::sync::atomic::AtomicBool::new(false),
            io_wait_ns: std::sync::atomic::AtomicU64::new(0),
            started: std::time::Instant::now(),
            progress_due_ns: std::sync::atomic::AtomicU64::new(0),
            pool: std::sync::OnceLock::new(),
            foreground,
            priority_failed: std::sync::atomic::AtomicBool::new(false),
            promoted: Mutex::new(None),
            timings: Mutex::new([None; 5]),
            io_bytes: std::sync::atomic::AtomicU64::new(0),
            gate: std::sync::OnceLock::new(),
        });
        let started = budget.clone();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(foreground.saturating_sub(1).max(1))
            .thread_name(|at| format!("recorder packing {at}"))
            .start_handler(move |_| {
                BUDGET.with_borrow_mut(|slot| *slot = Some(started.clone()));
                refresh();
            })
            .exit_handler(|_| {
                job_priority::confine_background_core(false);
                job_priority::foreground_thread();
            })
            .build()
            .map_err(io::Error::other)?;
        pool.broadcast(|_| ());
        if budget.priority_failed.load(Ordering::Acquire) {
            return Err(io::Error::other(
                "recorder workers could not enter background mode",
            ));
        }
        let pool = std::sync::Arc::new(pool);
        let _ = budget.pool.set(std::sync::Arc::downgrade(&pool));
        Ok(Self { pool, budget })
    }

    pub fn run<T>(&self, work: impl FnOnce() -> T) -> T {
        let current_progress = crate::progress::Progress::current();
        let previous_progress =
            std::mem::replace(&mut *lock(&self.budget.progress), current_progress);
        struct RestoreProgress<'a>(&'a Budget, Option<crate::progress::Progress>);
        impl Drop for RestoreProgress<'_> {
            fn drop(&mut self) {
                *lock(&self.0.progress) = self.1.take();
            }
        }
        let _progress = RestoreProgress(&self.budget, previous_progress);
        struct Restore(Option<std::sync::Arc<Budget>>, job_priority::Caller, bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                BUDGET.with_borrow_mut(|slot| *slot = self.0.take());
                job_priority::confine_background_core(self.2);
                let _ = job_priority::worker(self.2);
                self.1.clone().enter();
            }
        }
        let caller = job_priority::Caller::current();
        let background = job_priority::is_background_thread();
        let previous = BUDGET.with_borrow_mut(|slot| slot.replace(self.budget.clone()));
        let _restore = Restore(previous, caller, background);
        refresh();
        work()
    }

    pub(super) fn scope<'scope, F, T>(work: F) -> T
    where
        F: FnOnce(&rayon::Scope<'scope>) -> T,
    {
        let pool = BUDGET
            .with_borrow(|budget| {
                budget
                    .as_ref()
                    .and_then(|budget| budget.pool.get())
                    .and_then(std::sync::Weak::upgrade)
            })
            .expect("packing owns its workers");
        pool.in_place_scope(work)
    }

    pub fn promote(&self) {
        let mut promoted = lock(&self.budget.promoted);
        if promoted.is_none() {
            *promoted = Some(std::time::Instant::now());
        }
        drop(promoted);
        self.budget.background.store(false, Ordering::Release);
        self.pool.spawn_broadcast(|_| refresh());
    }

    pub fn set_gate(&self, gate: impl Fn() + Send + Sync + 'static) {
        let _ = self.budget.gate.set(std::sync::Arc::new(gate));
    }

    pub fn wait_for_game(&self) {
        wait_for_game(&self.budget);
    }

    pub fn catch_up(&self, enabled: bool) {
        if self.budget.catch_up.swap(enabled, Ordering::AcqRel) != enabled {
            self.pool.spawn_broadcast(|_| refresh());
        }
        refresh();
    }

    pub fn watch_progress(&self, active: bool) {
        let due = if active {
            (self.budget.started.elapsed() + std::time::Duration::from_secs(10))
                .as_nanos()
                .min(u64::MAX as u128) as u64
        } else {
            0
        };
        self.budget.progress_due_ns.store(due, Ordering::Release);
    }

    pub fn mode(&self) -> &'static str {
        if !self.budget.background.load(Ordering::Acquire) {
            "foreground"
        } else if self.budget.catch_up.load(Ordering::Acquire) {
            "catch_up_normal"
        } else {
            "background"
        }
    }

    pub fn workers() -> usize {
        workers()
    }

    pub fn timings(&self) -> [Option<std::time::Duration>; 5] {
        *lock(&self.budget.timings)
    }

    pub fn phase(phase: Phase) -> PhaseTimer {
        PhaseTimer {
            budget: BUDGET.with_borrow(Clone::clone),
            phase,
            started: std::time::Instant::now(),
        }
    }

    pub fn io(bytes: usize) {
        if bytes == 0 {
            return;
        }
        let budget = BUDGET.with_borrow(Clone::clone);
        let Some(budget) = budget else { return };
        budget.io_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        if !budget.background.load(Ordering::Acquire) {
            return;
        }
        let due = {
            let mut next = lock(&budget.io);
            *next = io_due(*next, std::time::Instant::now(), bytes);
            *next
        };
        let began = std::time::Instant::now();
        if began < due {
            while budget.background.load(Ordering::Acquire) && std::time::Instant::now() < due {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            budget.io_wait_ns.fetch_add(
                began.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                Ordering::Relaxed,
            );
        }
        refresh();
    }

    pub fn io_bytes(&self) -> u64 {
        self.budget.io_bytes.load(Ordering::Relaxed)
    }

    pub fn io_wait(&self) -> std::time::Duration {
        std::time::Duration::from_nanos(self.budget.io_wait_ns.load(Ordering::Relaxed))
    }

    pub fn active() -> bool {
        BUDGET.with_borrow(|budget| budget.is_some())
    }
}

trait Moment: Copy + Ord + std::ops::Add<std::time::Duration, Output = Self> {
    fn earlier(self, by: std::time::Duration) -> Option<Self>;
}

impl Moment for std::time::Instant {
    fn earlier(self, by: std::time::Duration) -> Option<Self> {
        self.checked_sub(by)
    }
}

fn io_due<T: Moment>(next: T, now: T, bytes: usize) -> T {
    const RATE: u64 = 32 << 20;
    const BURST: u64 = 8 << 20;
    let burst = std::time::Duration::from_secs_f64(BURST as f64 / RATE as f64);
    let start = now.earlier(burst).map_or(next, |floor| next.max(floor));
    start + std::time::Duration::from_secs_f64(bytes as f64 / RATE as f64)
}

pub struct Budgeted<T>(T);

impl<T> Budgeted<T> {
    pub fn new(inner: T) -> Self {
        Self(inner)
    }

    pub fn get_ref(&self) -> &T {
        &self.0
    }

    fn transfer_length(length: usize) -> usize {
        if crate::progress::Progress::current().is_some()
            || BUDGET.with_borrow(|budget| {
                budget
                    .as_ref()
                    .is_some_and(|budget| budget.background.load(Ordering::Acquire))
            })
        {
            length.min(64 << 10)
        } else {
            length
        }
    }
}

impl<T: io::Read> io::Read for Budgeted<T> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        refresh();
        let length = Self::transfer_length(bytes.len());
        let count = self.0.read(&mut bytes[..length])?;
        if count != 0 {
            crate::progress::Progress::completed();
        }
        Packing::io(count);
        Ok(count)
    }
}

impl<T: io::Write> io::Write for Budgeted<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        refresh();
        let count = self.0.write(&bytes[..Self::transfer_length(bytes.len())])?;
        if count != 0 {
            crate::progress::Progress::completed();
        }
        Packing::io(count);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: io::Seek> io::Seek for Budgeted<T> {
    fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
        self.0.seek(position)
    }
}

#[derive(Clone, Copy)]
pub enum Phase {
    Drain,
    Pack,
    Write,
    Readback,
    Publication,
}

pub struct PhaseTimer {
    budget: Option<std::sync::Arc<Budget>>,
    phase: Phase,
    started: std::time::Instant,
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        let Some(budget) = &self.budget else { return };
        let Some(promoted) = *lock(&budget.promoted) else {
            return;
        };
        let elapsed = self.started.max(promoted).elapsed();
        let mut timings = lock(&budget.timings);
        *timings[self.phase as usize].get_or_insert(std::time::Duration::ZERO) += elapsed;
    }
}

fn stalled(budget: &Budget, elapsed: std::time::Duration) -> bool {
    let due = budget.progress_due_ns.load(Ordering::Acquire);
    due != 0
        && elapsed.as_nanos() >= u128::from(due)
        && budget.background.load(Ordering::Acquire)
        && budget
            .catch_up
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
}

pub(super) fn refresh() {
    BUDGET.with_borrow(|budget| {
        if let Some(budget) = budget {
            if budget.progress_due_ns.load(Ordering::Relaxed) != 0 && stalled(budget, budget.started.elapsed()) {
                println!("Packing escalation: 10 s without a coordinator backlog sample during active work; CPU credits and logical I/O budget unchanged");
            }
            let background = budget.background.load(Ordering::Acquire)
                && !budget.catch_up.load(Ordering::Acquire);
            job_priority::confine_background_core(background);
            if let Err(error) = job_priority::worker(background)
                && !budget.priority_failed.swap(true, Ordering::AcqRel)
            {
                eprintln!("Recorder worker priority transition failed: {error}");
            }
            wait_for_game(budget);
        }
    });
}

pub(crate) fn wait_here() {
    BUDGET.with_borrow(|budget| {
        if let Some(budget) = budget {
            wait_for_game(budget);
        }
    });
}

fn wait_for_game(budget: &Budget) {
    if budget.background.load(Ordering::Acquire)
        && let Some(gate) = budget.gate.get()
    {
        gate();
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static WORKERS: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

pub fn workers() -> usize {
    #[cfg(test)]
    if let Some(workers) = WORKERS.get() {
        return workers;
    }
    if let Some(workers) = BUDGET.with_borrow(|budget| {
        budget.as_ref().map(|budget| {
            if budget.background.load(Ordering::Acquire) {
                1
            } else {
                budget.foreground
            }
        })
    }) {
        return workers;
    }
    (thread::available_parallelism().map_or(1, usize::from) / 2).max(1)
}

pub(crate) struct Cores {
    limit: usize,
    busy: Mutex<usize>,
    freed: Condvar,
}

impl Cores {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            busy: Mutex::new(1),
            freed: Condvar::new(),
        }
    }

    fn take(&self, wanted: usize) -> usize {
        let mut busy = lock(&self.busy);
        let taken = self.limit.saturating_sub(*busy).min(wanted);
        *busy += taken;
        taken
    }

    fn give(&self, count: usize) {
        *lock(&self.busy) -= count;
        self.freed.notify_all();
    }
}

struct Lent<'a>(&'a Cores, usize);

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        self.0.give(self.1);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn each<T: Send, U: Send>(
    cores: &Cores,
    items: Vec<T>,
    work: impl Fn(T) -> io::Result<U> + Sync,
) -> io::Result<Vec<U>> {
    let helpers = cores.take(items.len().saturating_sub(1));
    if helpers == 0 {
        return items
            .into_iter()
            .map(|item| {
                refresh();
                let result = work(item);
                if result.is_ok() {
                    crate::progress::Progress::completed();
                }
                result
            })
            .collect();
    }
    let lent = Lent(cores, helpers);
    let items: Vec<_> = items
        .into_iter()
        .map(|item| Mutex::new(Some(item)))
        .collect();
    let results: Vec<_> = items.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let run = || {
        loop {
            let at = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(at) else {
                return;
            };
            let item = lock(item).take().expect("each item is taken once");
            refresh();
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| work(item)));
            if matches!(&result, Ok(Ok(_))) {
                crate::progress::Progress::completed();
            }
            *lock(&results[at]) = Some(result);
        }
    };
    if Packing::active() {
        Packing::scope(|scope| {
            for _ in 0..helpers {
                let run = &run;
                scope.spawn(move |_| run());
            }
            run();
        });
    } else {
        let progress = crate::progress::Progress::current();
        thread::scope(|scope| {
            for _ in 0..helpers {
                let progress = progress.clone();
                let priority = job_priority::JobPriority::current();
                let _ = thread::Builder::new().spawn_scoped(scope, move || {
                    let _progress = progress.map(crate::progress::Progress::install);
                    priority.enter(job_priority::Level::Normal);
                    run()
                });
            }
            run();
        });
    }
    drop(lent);
    results
        .into_iter()
        .map(|result| {
            match result
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .expect("every item ran")
            {
                Ok(result) => result,
                Err(panic) => panic::resume_unwind(panic),
            }
        })
        .collect()
}

struct Lane<T, U> {
    waiting: VecDeque<(usize, T)>,
    done: std::collections::BTreeMap<usize, thread::Result<U>>,
    closed: bool,
}

struct Closing<'a, T, U>(&'a Mutex<Lane<T, U>>, &'a Condvar);

impl<T, U> Drop for Closing<'_, T, U> {
    fn drop(&mut self) {
        let mut lane = lock(self.0);
        lane.closed = true;
        lane.waiting.clear();
        self.1.notify_all();
    }
}

pub fn ordered<T: Send, U: Send>(
    threads: usize,
    mut next: impl FnMut() -> Option<T>,
    work: impl Fn(T) -> U + Sync,
    mut take: impl FnMut(U) -> bool,
) {
    let helpers = threads.saturating_sub(1);
    let inline = |next: &mut dyn FnMut() -> Option<T>, take: &mut dyn FnMut(U) -> bool| {
        while let Some(item) = next() {
            refresh();
            if !take(work(item)) {
                return;
            }
        }
    };
    if helpers == 0 {
        return inline(&mut next, &mut take);
    }
    if Packing::active() {
        let cores = Cores::new(threads);
        let ahead = helpers.saturating_mul(2);
        loop {
            let mut items = Vec::new();
            let mut exhausted = false;
            for _ in 0..ahead {
                refresh();
                match next() {
                    Some(item) => items.push(item),
                    None => {
                        exhausted = true;
                        break;
                    }
                }
            }
            let results = each(&cores, items, |item| {
                Ok(panic::catch_unwind(panic::AssertUnwindSafe(|| work(item))))
            })
            .expect("ordered work keeps panics as results");
            for result in results {
                refresh();
                let result = match result {
                    Ok(result) => result,
                    Err(panic) => panic::resume_unwind(panic),
                };
                if !take(result) {
                    return;
                }
            }
            if exhausted {
                return;
            }
        }
    }
    let lane = Mutex::new(Lane {
        waiting: VecDeque::new(),
        done: std::collections::BTreeMap::new(),
        closed: false,
    });
    let changed = Condvar::new();
    let serve = || {
        let mut state = lock(&lane);
        loop {
            if let Some((at, item)) = state.waiting.pop_front() {
                drop(state);
                refresh();
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| work(item)));
                state = lock(&lane);
                state.done.insert(at, result);
                changed.notify_all();
            } else if state.closed {
                return;
            } else {
                state = changed.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
        }
    };
    let mut drive = |started| {
        if started == 0 {
            return inline(&mut next, &mut take);
        }
        let ahead = 2 * started;
        let (mut read, mut taken, mut exhausted) = (0, 0, false);
        loop {
            while !exhausted && read - taken < ahead {
                match next() {
                    Some(item) => {
                        lock(&lane).waiting.push_back((read, item));
                        changed.notify_all();
                        read += 1;
                    }
                    None => exhausted = true,
                }
            }
            if taken == read {
                return;
            }
            let result = {
                let mut state = lock(&lane);
                loop {
                    if let Some(result) = state.done.remove(&taken) {
                        break result;
                    }
                    state = changed.wait(state).unwrap_or_else(PoisonError::into_inner);
                }
            };
            taken += 1;
            match result {
                Ok(result) => {
                    if !take(result) {
                        return;
                    }
                }
                Err(panic) => panic::resume_unwind(panic),
            }
        }
    };
    let progress = crate::progress::Progress::current();
    thread::scope(|scope| {
        let _closing = Closing(&lane, &changed);
        let started = (0..helpers)
            .filter(|_| {
                let priority = job_priority::JobPriority::current();
                thread::Builder::new()
                    .spawn_scoped(scope, || {
                        let _progress = progress.clone().map(crate::progress::Progress::install);
                        priority.enter(job_priority::Level::Normal);
                        serve()
                    })
                    .is_ok()
            })
            .count();
        drive(started);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    impl Moment for Duration {
        fn earlier(self, by: Duration) -> Option<Self> {
            self.checked_sub(by)
        }
    }

    #[test]
    fn background_io_near_the_clock_origin_keeps_the_credit_it_accrued() {
        let ms = Duration::from_millis;
        let mebibyte = Duration::from_secs_f64(1.0 / 32.0);
        assert_eq!(io_due(ms(50), ms(200), 1 << 20), ms(50) + mebibyte);
        assert_eq!(io_due(ms(50), ms(1_000), 1 << 20), ms(750) + mebibyte);
        assert_eq!(io_due(ms(900), ms(1_000), 1 << 20), ms(900) + mebibyte);
    }
}
