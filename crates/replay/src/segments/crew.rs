//! Workers helping a walk run its families' hours, the walk's worker among them.

use std::{
    io,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

// Locks are taken in the order schedule, crews, hour.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The walks under way, for workers to help.
#[derive(Default)]
pub(super) struct Board {
    crews: Mutex<Vec<Arc<Crew>>>,
    wake: Condvar,
    /// Bumped on every change to what the schedule may hand out.
    changes: AtomicU64,
    /// Workers waiting on the board for any walk's hour.
    idle: AtomicUsize,
}

impl Board {
    /// The changes so far, read before asking the schedule for a task.
    pub(super) fn changes(&self) -> u64 {
        self.changes.load(Ordering::SeqCst)
    }

    /// What the schedule may hand out changed: every waiting worker goes back to it.
    pub(super) fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
        let crews = lock(&self.crews);
        for crew in crews.iter() {
            drop(lock(&crew.hour));
            crew.posted.notify_all();
        }
        drop(crews);
        self.wake.notify_all();
    }

    /// The earliest walk under way, while its open hour has room for another worker.
    pub(super) fn front(&self) -> Option<Arc<Crew>> {
        let crews = lock(&self.crews);
        crews
            .iter()
            .min_by_key(|crew| crew.at)
            .filter(|crew| crew.wanting())
            .map(Arc::clone)
    }

    /// Waits for the earliest walk with room; None once the schedule changed past `seen`.
    pub(super) fn next(&self, seen: u64) -> Option<Arc<Crew>> {
        let mut crews = lock(&self.crews);
        loop {
            if self.changes() != seen {
                return None;
            }
            // Counted before looking: a walk posting since either is seen or wakes it.
            self.idle.fetch_add(1, Ordering::SeqCst);
            let wanting = crews
                .iter()
                .filter(|crew| crew.wanting())
                .min_by_key(|crew| crew.at)
                .map(Arc::clone);
            if wanting.is_some() {
                self.idle.fetch_sub(1, Ordering::SeqCst);
                return wanting;
            }
            crews = self
                .wake
                .wait(crews)
                .unwrap_or_else(PoisonError::into_inner);
            self.idle.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// The crew of a walk of segment `at`, its hours open to helpers until dropped.
    pub(super) fn join(&self, at: usize) -> io::Result<Joined<'_>> {
        let crew = Arc::new(Crew {
            at,
            hour: Mutex::new(Open::default()),
            left: Condvar::new(),
            posted: Condvar::new(),
            next: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
            room: AtomicUsize::new(0),
        });
        let mut crews = lock(&self.crews);
        engine_allocation::reserve(&mut *crews, 1)?;
        crews.push(Arc::clone(&crew));
        Ok(Joined { board: self, crew })
    }

    /// Wakes up to `room` workers waiting on the board.
    fn posted(&self, room: usize) {
        if room == 0 || self.idle.load(Ordering::SeqCst) == 0 {
            return;
        }
        drop(lock(&self.crews));
        for _ in 0..room.min(self.idle.load(Ordering::SeqCst)) {
            self.wake.notify_one();
        }
    }
}

/// An hour's work, its lifetime erased; [`Posted`] withdraws and drains it before the borrows end.
#[derive(Clone, Copy)]
struct Job(*const (dyn Fn(usize) + Sync));

// SAFETY: it points at a closure any thread may call, `Sync`, while it is posted.
unsafe impl Send for Job {}

#[derive(Default)]
struct Open {
    job: Option<Job>,
    /// Helpers inside the hour.
    active: usize,
    /// Hours posted so far.
    hours: u64,
    /// The walk ended: no hour is posted again.
    closed: bool,
}

/// A walk's hours as helpers take them.
pub(super) struct Crew {
    at: usize,
    hour: Mutex<Open>,
    /// Every helper left the hour.
    left: Condvar,
    /// An hour was posted, the walk ended, or the schedule changed.
    posted: Condvar,
    /// The open hour's next operator, its count, and the helpers it still has room for.
    next: AtomicUsize,
    count: AtomicUsize,
    room: AtomicUsize,
}

impl Crew {
    /// Whether its open hour has operators left and room for another helper.
    fn wanting(&self) -> bool {
        self.room.load(Ordering::SeqCst) > 0
            && self.next.load(Ordering::SeqCst) < self.count.load(Ordering::SeqCst)
    }

    /// Helps hour after hour while each has room and work, until the schedule changes.
    pub(super) fn help(&self, board: &Board, seen: u64) {
        let mut open = lock(&self.hour);
        loop {
            if open.closed || board.changes() != seen {
                return;
            }
            let Some(job) = open.job.filter(|_| self.wanting()) else {
                return;
            };
            let hour = open.hours;
            open.active += 1;
            self.room.fetch_sub(1, Ordering::SeqCst);
            drop(open);
            let took = {
                let _leaving = Leaving(self);
                self.take(job)
            };
            open = lock(&self.hour);
            if took == 0 {
                return;
            }
            while open.hours == hour && !open.closed && board.changes() == seen {
                open = self
                    .posted
                    .wait(open)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }

    /// Runs the open hour's operators not yet taken, one at a time: how many it ran.
    fn take(&self, job: Job) -> usize {
        let count = self.count.load(Ordering::SeqCst);
        let mut took = 0;
        loop {
            let index = self.next.fetch_add(1, Ordering::SeqCst);
            if index >= count {
                return took;
            }
            // SAFETY: the job is posted until every helper inside the hour left ([`Posted`]).
            unsafe { (*job.0)(index) };
            took += 1;
        }
    }
}

struct Leaving<'c>(&'c Crew);

impl Drop for Leaving<'_> {
    fn drop(&mut self) {
        let mut open = lock(&self.0.hour);
        open.active -= 1;
        if open.active == 0 {
            self.0.left.notify_all();
        }
    }
}

/// A walk's crew, its hours open to helpers.
pub(super) struct Joined<'b> {
    board: &'b Board,
    crew: Arc<Crew>,
}

impl Joined<'_> {
    /// Runs `job` for each index below `count` here and on up to `width` - 1 helpers.
    pub(super) fn hour(&self, count: usize, width: usize, job: &(dyn Fn(usize) + Sync)) {
        let crew = &*self.crew;
        let erased: *const (dyn Fn(usize) + Sync + '_) = job;
        // SAFETY: only the lifetime changes; `Posted` drains every helper before it ends.
        let erased = Job(unsafe {
            std::mem::transmute::<
                *const (dyn Fn(usize) + Sync + '_),
                *const (dyn Fn(usize) + Sync + 'static),
            >(erased)
        });
        let room = width.clamp(1, count.max(1)) - 1;
        {
            let mut open = lock(&crew.hour);
            crew.next.store(0, Ordering::SeqCst);
            crew.count.store(count, Ordering::SeqCst);
            crew.room.store(room, Ordering::SeqCst);
            open.job = Some(erased);
            open.hours += 1;
        }
        let _posted = Posted(crew);
        // Its helpers wait for the next hour whatever its room: one finding none leaves.
        crew.posted.notify_all();
        self.board.posted(room);
        crew.take(erased);
    }
}

impl Drop for Joined<'_> {
    fn drop(&mut self) {
        lock(&self.board.crews).retain(|crew| !Arc::ptr_eq(crew, &self.crew));
        lock(&self.crew.hour).closed = true;
        self.crew.posted.notify_all();
    }
}

/// An hour posted, withdrawn once dropped, once no helper is inside it.
struct Posted<'c>(&'c Crew);

impl Drop for Posted<'_> {
    fn drop(&mut self) {
        let crew = self.0;
        let mut open = lock(&crew.hour);
        open.job = None;
        crew.room.store(0, Ordering::SeqCst);
        crew.count.store(0, Ordering::SeqCst);
        while open.active > 0 {
            open = crew
                .left
                .wait(open)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}
