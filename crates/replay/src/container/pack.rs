//! Bounded block packing: ordered coordinator state, parallel finite table/section work.
//!
//! Offline callers may also pipeline independent finished blocks on OS workers.

use std::{
    collections::VecDeque,
    io, panic,
    sync::{
        Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

type CompressionSource = dyn Fn() -> Option<crate::compression::Compression> + Send + Sync;

struct Budget {
    compression: AtomicUsize,
    compression_source: Mutex<Option<std::sync::Arc<CompressionSource>>>,
    progress: Mutex<Option<crate::progress::Progress>>,
    // Playing limits remain active even after catch-up leaves OS background mode.
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
    #[cfg(test)]
    readback_table: Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
}

thread_local! {
    static BUDGET: std::cell::RefCell<Option<std::sync::Arc<Budget>>> = const { std::cell::RefCell::new(None) };
}

/// Recorder-owned persistent workers: four active CPU credits and 32 MiB/s while playing.
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

    /// Packing at the compression `path`'s recorder settings choose, re-read for each block.
    pub fn from_settings(path: &std::path::Path) -> io::Result<Self> {
        use crate::recorder_settings::RecorderSettings;
        let settings = RecorderSettings::load_from(path)?;
        let packing = Self::new()?;
        packing.set_compression(settings.compression);
        let path = path.to_owned();
        packing.set_compression_source(move || {
            RecorderSettings::load_from(&path)
                .ok()
                .map(|current| current.compression)
        });
        Ok(packing)
    }

    fn with_workers(foreground: usize) -> io::Result<Self> {
        let foreground = foreground.max(1);
        let budget = std::sync::Arc::new(Budget {
            compression: AtomicUsize::new(0),
            compression_source: Mutex::new(None),
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
            #[cfg(test)]
            readback_table: Mutex::new(None),
        });
        let started = budget.clone();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(foreground.saturating_sub(1).max(1))
            .thread_name(|at| format!("recorder packing {at}"))
            .start_handler(move |_| {
                BUDGET.with_borrow_mut(|slot| *slot = Some(started.clone()));
                refresh();
            })
            .exit_handler(|_| job_priority::foreground_thread())
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

    /// Promote these workers in place, at their next table, section or frame boundary.
    pub fn promote(&self) {
        let mut promoted = lock(&self.budget.promoted);
        if promoted.is_none() {
            *promoted = Some(std::time::Instant::now());
        }
        drop(promoted);
        self.budget.background.store(false, Ordering::Release);
        self.pool.spawn_broadcast(|_| refresh());
    }

    pub fn set_compression(&self, compression: crate::compression::Compression) {
        self.budget.compression.store(
            usize::from(compression == crate::compression::Compression::High),
            Ordering::Release,
        );
    }

    pub fn set_compression_source(
        &self,
        source: impl Fn() -> Option<crate::compression::Compression> + Send + Sync + 'static,
    ) {
        *lock(&self.budget.compression_source) = Some(std::sync::Arc::new(source));
    }

    /// Leave OS background mode while catching up, retaining the playing CPU and logical I/O limits.
    pub fn catch_up(&self, enabled: bool) {
        if self.budget.catch_up.swap(enabled, Ordering::AcqRel) != enabled {
            self.pool.spawn_broadcast(|_| refresh());
        }
        refresh();
    }

    /// Busy operations refresh priority themselves if the coordinator cannot sample its backlog.
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
        const RATE: u64 = 32 << 20;
        const BURST: u64 = 8 << 20;
        let due = {
            let mut next = lock(&budget.io);
            let now = std::time::Instant::now();
            let burst = std::time::Duration::from_secs_f64(BURST as f64 / RATE as f64);
            let start = (*next).max(now.checked_sub(burst).unwrap_or(now));
            *next = start + std::time::Duration::from_secs_f64(bytes as f64 / RATE as f64);
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

    /// Sum of limiter waits on all packing threads, not coordinator wall time.
    pub fn io_wait(&self) -> std::time::Duration {
        std::time::Duration::from_nanos(self.budget.io_wait_ns.load(Ordering::Relaxed))
    }

    pub fn active() -> bool {
        BUDGET.with_borrow(|budget| budget.is_some())
    }
}

pub(crate) fn compression() -> crate::compression::Compression {
    use crate::compression::Compression;
    let budget = BUDGET.with_borrow(Clone::clone);
    let Some(budget) = budget else {
        return Compression::Low;
    };
    let source = lock(&budget.compression_source).clone();
    if let Some(current) = source.and_then(|source| source()) {
        budget
            .compression
            .store(usize::from(current == Compression::High), Ordering::Release);
    }
    if budget.compression.load(Ordering::Acquire) == 0 {
        Compression::Low
    } else {
        Compression::High
    }
}

/// Charge completed file I/O once, including short operations, against the current packing job.
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

/// Exclusive coordinator wall time, reported only for phases actually entered after Stop.
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

pub(super) fn readback_table() {
    refresh();
    #[cfg(test)]
    {
        let watch = BUDGET.with_borrow(|budget| {
            budget
                .as_ref()
                .and_then(|budget| lock(&budget.readback_table).clone())
        });
        if let Some(watch) = watch {
            watch();
        }
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
            if let Err(error) = job_priority::worker(background)
                && !budget.priority_failed.swap(true, Ordering::AcqRel)
            {
                eprintln!("Recorder worker priority transition failed: {error}");
            }
        }
    });
}

#[cfg(test)]
thread_local! {
    /// Tests pack this thread's recordings on this many threads.
    pub(crate) static WORKERS: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

/// The threads a pack keeps busy, the calling thread included: half the
/// machine's logical CPUs, at least one.
pub(crate) fn workers() -> usize {
    #[cfg(test)]
    if let Some(workers) = WORKERS.get() {
        return workers;
    }
    if let Some(workers) = BUDGET.with_borrow(|budget| {
        budget.as_ref().map(|budget| {
            if budget.background.load(Ordering::Acquire) {
                4.min(budget.foreground)
            } else {
                budget.foreground
            }
        })
    }) {
        return workers;
    }
    (thread::available_parallelism().map_or(1, usize::from) / 2).max(1)
}

/// Finished blocks wait for a worker at most this many at a time, so a pack
/// holds at most its workers' blocks, these and the block being written.
pub(crate) const WAITING: usize = 1;

/// The threads a pack may keep busy, and how many are: the calling thread,
/// the workers packing a block, and the helpers free ones are lent to.
pub(crate) struct Cores {
    limit: usize,
    busy: Mutex<usize>,
    freed: Condvar,
}

impl Cores {
    /// `limit` threads, the calling one busy.
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            busy: Mutex::new(1),
            freed: Condvar::new(),
        }
    }

    /// `limit` threads, none busy: the calling thread only waits.
    pub(crate) fn idle(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            busy: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    /// `work` on this thread, as one busy thread once one is free.
    pub(crate) fn busy<T>(&self, work: impl FnOnce() -> T) -> T {
        self.acquire();
        let _lent = Lent(self, 1);
        work()
    }

    /// Up to `wanted` more busy threads, as many as are free now.
    fn take(&self, wanted: usize) -> usize {
        let mut busy = lock(&self.busy);
        let taken = self.limit.saturating_sub(*busy).min(wanted);
        *busy += taken;
        taken
    }

    /// One more busy thread, once one is free.
    fn acquire(&self) {
        let mut busy = lock(&self.busy);
        while *busy >= self.limit {
            busy = self
                .freed
                .wait(busy)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *busy += 1;
    }

    fn give(&self, count: usize) {
        *lock(&self.busy) -= count;
        self.freed.notify_all();
    }
}

/// Cores taken for helpers, given back however their work ends.
struct Lent<'a>(&'a Cores, usize);

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        self.0.give(self.1);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `work` over `items` with the result `items.into_iter().map(work).collect()`
/// has, the first error or panic in item order, on this thread and on as many
/// helper threads as `cores` has free.
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
                // A helper the OS refuses leaves its items to the others.
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

/// Closes a lane however its reader stops, so its workers end.
struct Closing<'a, T, U>(&'a Mutex<Lane<T, U>>, &'a Condvar);

impl<T, U> Drop for Closing<'_, T, U> {
    fn drop(&mut self) {
        let mut lane = lock(self.0);
        lane.closed = true;
        lane.waiting.clear();
        self.1.notify_all();
    }
}

/// `take(work(next()))` in item order, as a loop would, with `work` on up to `threads` threads.
pub(crate) fn ordered<T: Send, U: Send>(
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
        // Pool tasks finish a batch; none waits for its coordinator to feed or accept work.
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

type Packed<R> = thread::Result<io::Result<R>>;

struct Queue<J, R> {
    waiting: VecDeque<(usize, J)>,
    /// By submission, none until packed or after an earlier failure.
    packed: Vec<Option<Packed<R>>>,
    /// The first submission whose packing failed.
    failed: Option<usize>,
    closed: bool,
}

struct Pool<'a, J, R> {
    cores: Cores,
    queue: Mutex<Queue<J, R>>,
    changed: Condvar,
    pack: &'a (dyn Fn(J, &Cores) -> io::Result<R> + Sync),
}

impl<J, R> Pool<'_, J, R> {
    fn wait<'g>(&self, queue: MutexGuard<'g, Queue<J, R>>) -> MutexGuard<'g, Queue<J, R>> {
        self.changed
            .wait(queue)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// One worker: pack waiting jobs until the queue closes empty.
    fn work(&self) {
        let mut queue = lock(&self.queue);
        loop {
            let Some((at, job)) = queue.waiting.pop_front() else {
                if queue.closed {
                    return;
                }
                queue = self.wait(queue);
                continue;
            };
            self.changed.notify_all();
            // A job after a failed one cannot change the result.
            let skip = queue.failed.is_some_and(|failed| failed < at);
            drop(queue);
            let packed = (!skip).then(|| {
                self.cores.acquire();
                let packed =
                    panic::catch_unwind(panic::AssertUnwindSafe(|| (self.pack)(job, &self.cores)));
                self.cores.give(1);
                packed
            });
            queue = lock(&self.queue);
            if let Some(packed) = packed {
                if !matches!(packed, Ok(Ok(_))) {
                    queue.failed = Some(queue.failed.map_or(at, |failed| failed.min(at)));
                }
                queue.packed[at] = Some(packed);
                self.changed.notify_all();
            }
        }
    }
}

/// Where the writer hands its finished blocks.
pub(crate) struct Packer<'a, J, R> {
    pool: &'a Pool<'a, J, R>,
    /// No worker started: pack on this thread, as the sequential writer.
    inline: bool,
}

impl<'a, J, R> Packer<'a, J, R> {
    /// The threads the writer may lend its own work to.
    pub(crate) fn cores(&self) -> &'a Cores {
        &self.pool.cores
    }

    /// Pack `job` after the jobs submitted before it. An error is the
    /// sequential writer's when packing on this thread; otherwise it only
    /// stops writing after an earlier job failed, and [`blocks`] returns
    /// that job's error.
    pub(crate) fn submit(&mut self, job: J) -> io::Result<()> {
        refresh();
        let pool = self.pool;
        if self.inline {
            let packed = (pool.pack)(job, &pool.cores)?;
            lock(&pool.queue).packed.push(Some(Ok(Ok(packed))));
            return Ok(());
        }
        let mut queue = lock(&pool.queue);
        if queue.waiting.len() >= WAITING && queue.failed.is_none() {
            // While it waits, this thread's core is free for the others.
            pool.cores.give(1);
            while queue.waiting.len() >= WAITING && queue.failed.is_none() {
                queue = pool.wait(queue);
            }
            drop(queue);
            pool.cores.acquire();
            // Only workers took jobs or failed since.
            queue = lock(&pool.queue);
        }
        if queue.failed.is_some() {
            return Err(io::Error::other("an earlier block failed to pack"));
        }
        let at = queue.packed.len();
        queue.packed.push(None);
        queue.waiting.push_back((at, job));
        pool.changed.notify_all();
        Ok(())
    }
}

impl<J, R> Drop for Packer<'_, J, R> {
    fn drop(&mut self) {
        lock(&self.pool.queue).closed = true;
        self.pool.changed.notify_all();
    }
}

/// `write` with a [`Packer`] that packs each submitted job with `pack` on up
/// to `workers` threads, this one included, and the packed jobs in
/// submission order. The result is the sequential writer's, which packs each
/// job as it is submitted: the first failed job's error or panic, else
/// `write`'s error or panic, since every job was submitted before it.
pub(crate) fn blocks<J: Send, R: Send, P>(
    workers: usize,
    pack: impl Fn(J, &Cores) -> io::Result<R> + Sync,
    write: impl FnOnce(&mut Packer<'_, J, R>) -> io::Result<P>,
) -> io::Result<(P, Vec<R>)> {
    let pool = Pool {
        cores: Cores::new(workers),
        queue: Mutex::new(Queue {
            waiting: VecDeque::new(),
            packed: Vec::new(),
            failed: None,
            closed: false,
        }),
        changed: Condvar::new(),
        pack: &pack,
    };
    let written = if Packing::active() {
        // Keep ordered block ownership here; only finite table/section work enters the pool.
        let mut packer = Packer {
            pool: &pool,
            inline: true,
        };
        panic::catch_unwind(panic::AssertUnwindSafe(|| write(&mut packer)))
    } else {
        let progress = crate::progress::Progress::current();
        thread::scope(|scope| {
            let started = (1..workers)
                .filter(|_| {
                    let priority = job_priority::JobPriority::current();
                    thread::Builder::new()
                        .spawn_scoped(scope, || {
                            let _progress =
                                progress.clone().map(crate::progress::Progress::install);
                            priority.enter(job_priority::Level::Normal);
                            pool.work()
                        })
                        .is_ok()
                })
                .count();
            let mut packer = Packer {
                pool: &pool,
                inline: started == 0,
            };
            // A job submitted before the writer panicked still fails first.
            panic::catch_unwind(panic::AssertUnwindSafe(|| write(&mut packer)))
        })
    };
    let queue = pool
        .queue
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let mut packed = Vec::with_capacity(queue.packed.len());
    for job in queue.packed {
        match job.expect("a job is skipped only after an earlier one failed") {
            Ok(Ok(job)) => packed.push(job),
            Ok(Err(error)) => return Err(error),
            Err(panic) => panic::resume_unwind(panic),
        }
    }
    match written {
        Ok(written) => Ok((written?, packed)),
        Err(panic) => panic::resume_unwind(panic),
    }
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pack_fallback_tests.rs"]
mod fallback_tests;

#[cfg(test)]
mod continuous_tests {
    use super::*;

    #[test]
    fn finalizer_chunk_progress_reaches_persistent_workers_and_is_removed_after_the_job() {
        use std::sync::{Arc, atomic::AtomicU64};
        let packing = Packing::with_workers(2).unwrap();
        packing.promote();
        let chunks = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&chunks);
        let progress = crate::progress::Progress::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        })
        .install();
        packing.run(|| {
            packing
                .pool
                .broadcast(|_| crate::progress::Progress::completed());
        });
        let completed = chunks.load(Ordering::Relaxed);
        assert_eq!(completed, packing.pool.current_num_threads() as u64);
        drop(progress);
        packing.run(|| {
            packing
                .pool
                .broadcast(|_| crate::progress::Progress::completed())
        });
        assert_eq!(chunks.load(Ordering::Relaxed), completed);
    }

    #[test]
    fn takeover_uses_the_same_persistent_pool_and_restores_the_caller() {
        let packing = Packing::new().unwrap();
        let before = packing.pool.broadcast(|_| std::thread::current().id());
        packing.run(|| {
            assert!(workers() <= 4);
            let cores = Cores::new(workers());
            assert_eq!(
                each(&cores, (0..100).collect(), |at| Ok(at + 1)).unwrap(),
                (1..=100).collect::<Vec<_>>()
            );
        });
        assert!(!Packing::active());
        packing.promote();
        packing.run(|| {
            assert_eq!(workers(), packing.budget.foreground);
            let cores = Cores::new(workers());
            assert_eq!(
                each(&cores, (0..100).collect(), |at| Ok(at + 1))
                    .unwrap()
                    .len(),
                100
            );
        });
        assert_eq!(
            before,
            packing.pool.broadcast(|_| std::thread::current().id())
        );
        assert!(!Packing::active());
    }

    #[test]
    fn stop_promotes_an_inflight_job_without_cancelling_or_resubmitting_it() {
        use std::sync::{Arc, Barrier, mpsc};
        let packing = Arc::new(Packing::new().unwrap());
        let identities = packing.pool.broadcast(|_| std::thread::current().id());
        let calls = Arc::new((0..20).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        let release = Arc::new(Barrier::new(2));
        let (started, waiting) = mpsc::channel();
        let running = packing.clone();
        let counts = calls.clone();
        let blocked = release.clone();
        let coordinator = thread::spawn(move || {
            running.run(|| {
                let cores = Cores::new(workers());
                each(&cores, (0..20).collect(), |at| {
                    counts[at].fetch_add(1, Ordering::Relaxed);
                    if at == 0 {
                        started.send(()).unwrap();
                        blocked.wait();
                    }
                    Ok(at)
                })
                .unwrap()
            })
        });
        waiting.recv().unwrap();
        packing.promote();
        release.wait();
        assert_eq!(coordinator.join().unwrap(), (0..20).collect::<Vec<_>>());
        assert!(calls.iter().all(|calls| calls.load(Ordering::Relaxed) == 1));
        assert_eq!(
            identities,
            packing.pool.broadcast(|_| std::thread::current().id())
        );
        assert!(
            packing
                .pool
                .broadcast(|_| !job_priority::is_background_thread())
                .into_iter()
                .all(|foreground| foreground)
        );
    }

    #[test]
    fn stop_promotes_readback_at_each_table_including_the_external_coordinator() {
        use crate::container::{
            Parallel, Replay,
            tests::{CAMPAIGN, records},
        };
        use std::collections::HashMap;
        use std::sync::{Arc, Barrier};
        for requested in [1, 4] {
            let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
            let reference = Replay::open(replay.bytes().to_vec()).unwrap();
            let expected = reference.read_block(&reference.index[0]).unwrap();
            let packing = Arc::new(Packing::new().unwrap());
            let count = requested.min(packing.budget.foreground).min(
                crate::rows::TableId::ALL
                    .len()
                    .div_ceil(super::super::PARALLEL_FRAMES),
            );
            let arrived = Arc::new(Barrier::new(count + 1));
            let release = Arc::new(Barrier::new(count + 1));
            let resumed = Arc::new(Barrier::new(count));
            let observations = Arc::new(Mutex::new(HashMap::<thread::ThreadId, Vec<bool>>::new()));
            let seen = observations.clone();
            let first = arrived.clone();
            let blocked = release.clone();
            let second = resumed.clone();
            *lock(&packing.budget.readback_table) = Some(Arc::new(move || {
                let turn = {
                    let mut seen = lock(&seen);
                    let states = seen.entry(thread::current().id()).or_default();
                    states.push(job_priority::is_background_thread());
                    states.len()
                };
                if turn == 1 {
                    first.wait();
                    blocked.wait();
                } else if turn == 2 {
                    second.wait();
                }
            }));
            let running = packing.clone();
            let coordinator = thread::spawn(move || {
                let caller = thread::current().id();
                let result = running.run(|| {
                    replay
                        .read_block_parallel(
                            &replay.index[0],
                            &Parallel {
                                workers: count,
                                on_worker: &|| {},
                            },
                        )
                        .unwrap()
                });
                (caller, result)
            });
            arrived.wait();
            packing.promote();
            release.wait();
            let (caller, result) = coordinator.join().unwrap();
            assert_eq!(result, expected);
            let observed = lock(&observations);
            assert_eq!(observed.len(), count);
            assert!(
                observed.contains_key(&caller),
                "the coordinator must process tables too"
            );
            assert!(observed.values().all(|states| states.len() >= 2
                && states[0]
                && states[1..].iter().all(|background| !background)));
        }
    }

    #[test]
    fn io_budget_charges_short_operations_once_and_not_failed_or_empty_operations() {
        use io::{Read, Seek, Write};
        struct Short(io::Cursor<Vec<u8>>);
        impl Read for Short {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let length = bytes.len().min(2);
                self.0.read(&mut bytes[..length])
            }
        }
        impl Write for Short {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes == b"fail" {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
                self.0.write(&bytes[..bytes.len().min(3)])
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl Seek for Short {
            fn seek(&mut self, to: io::SeekFrom) -> io::Result<u64> {
                self.0.seek(to)
            }
        }
        let packing = Packing::new().unwrap();
        packing.run(|| {
            let mut file = Budgeted::new(Short(io::Cursor::new(Vec::new())));
            file.write_all(b"1234567").unwrap();
            assert_eq!(packing.io_bytes(), 7);
            assert_eq!(
                file.write(b"fail").unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(packing.io_bytes(), 7);
            file.seek(io::SeekFrom::Start(0)).unwrap();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"1234567");
            assert_eq!(packing.io_bytes(), 14);
            assert_eq!(file.read(&mut []).unwrap(), 0);
            assert_eq!(packing.io_bytes(), 14);
        });
    }

    #[test]
    fn phases_not_observed_are_not_zero_and_stop_does_not_reset_progress() {
        let packing = Packing::new().unwrap();
        assert_eq!(packing.timings(), [None; 5]);
        packing.promote();
        packing.run(|| {
            let _phase = Packing::phase(Phase::Pack);
        });
        let timed = packing.timings();
        assert!(timed[Phase::Pack as usize].is_some());
        assert!(timed[Phase::Readback as usize].is_none());
        packing.promote();
        assert_eq!(packing.timings(), timed);
    }
}
