//! Packing a recording's blocks on a bounded pool of threads.
//!
//! The writer encodes every frame on the calling thread, in recorded order:
//! block boundaries, keyframe anchors and each block's predictions, row keys
//! and models are decided there. A finished block depends on nothing else, so
//! [`blocks`] compresses finished blocks on worker threads while the calling
//! thread writes the next, and returns them in order with the result the
//! sequential writer would have returned.

use std::{
    collections::VecDeque,
    io, panic,
    sync::{
        Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

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
    (thread::available_parallelism().map_or(1, usize::from) / 2).max(1)
}

/// Finished blocks wait for a worker at most this many at a time, so a pack
/// holds at most its workers' blocks, these and the block being written.
const WAITING: usize = 1;

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
        return items.into_iter().map(work).collect();
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
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| work(item)));
            *lock(&results[at]) = Some(result);
        }
    };
    thread::scope(|scope| {
        for _ in 0..helpers {
            // A helper the OS refuses leaves its items to the others.
            let _ = thread::Builder::new().spawn_scoped(scope, run);
        }
        run();
    });
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

impl<J, R> Packer<'_, J, R> {
    /// The threads the writer may lend its own work to.
    pub(crate) fn cores(&self) -> &Cores {
        &self.pool.cores
    }

    /// Pack `job` after the jobs submitted before it. An error is the
    /// sequential writer's when packing on this thread; otherwise it only
    /// stops writing after an earlier job failed, and [`blocks`] returns
    /// that job's error.
    pub(crate) fn submit(&mut self, job: J) -> io::Result<()> {
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
    let written = thread::scope(|scope| {
        let started = (1..workers)
            .filter(|_| {
                thread::Builder::new()
                    .spawn_scoped(scope, || pool.work())
                    .is_ok()
            })
            .count();
        let mut packer = Packer {
            pool: &pool,
            inline: started == 0,
        };
        // A job submitted before the writer panicked still fails first.
        panic::catch_unwind(panic::AssertUnwindSafe(|| write(&mut packer)))
    });
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
