//! Readers follow the walk's live claim window; its owned guard releases them on every exit.
use std::{
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard, Weak, atomic::AtomicBool},
    time::Duration,
};

/// Where the walk is in its obligation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    Pending,
    /// It reads the block at its cursor.
    Running,
    Terminal,
}

impl Lifecycle {
    /// Its name in measurement reports.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Terminal => "terminal",
        }
    }
}

/// One coherent view of the walk's obligation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub lifecycle: Lifecycle,
    /// The block it reads now, or starts at.
    pub cursor: usize,
    /// The furthest block its pre-pass lanes have claimed; None before any.
    pub claimed: Option<usize>,
}

impl Snapshot {
    pub(crate) fn window(&self) -> Option<(usize, usize)> {
        match self.lifecycle {
            Lifecycle::Pending => Some((self.cursor, self.cursor)),
            Lifecycle::Running => Some((
                self.cursor,
                self.claimed
                    .map_or(self.cursor, |claimed| claimed.max(self.cursor)),
            )),
            Lifecycle::Terminal => None,
        }
    }
}

// Cancellation and pool wake-ups are not signalled through every reader condvar.
pub(crate) const POLL: Duration = Duration::from_millis(25);

pub(crate) struct WalkPace {
    state: Mutex<Snapshot>,
    #[cfg(test)]
    pub(crate) reads: (Mutex<super::pipeline::independent::TestReads>, Condvar),
    #[cfg(test)]
    pub(crate) family_claims: Mutex<Vec<(usize, Snapshot)>>,
    /// Notified at every change, for readers waiting on the walk.
    changed: Condvar,
    /// Each lifecycle it took, in order, in tests.
    #[cfg(test)]
    history: Mutex<Vec<Lifecycle>>,
}

impl WalkPace {
    fn new(first: usize, blocks: usize) -> Arc<Self> {
        let lifecycle = if first < blocks {
            Lifecycle::Pending
        } else {
            Lifecycle::Terminal
        };
        Arc::new(Self {
            state: Mutex::new(Snapshot {
                lifecycle,
                cursor: first,
                claimed: None,
            }),
            changed: Condvar::new(),
            #[cfg(test)]
            reads: (
                Mutex::new(super::pipeline::independent::TestReads::default()),
                Condvar::new(),
            ),
            #[cfg(test)]
            family_claims: Mutex::new(Vec::new()),
            #[cfg(test)]
            history: Mutex::new(vec![lifecycle]),
        })
    }

    pub(crate) fn start(first: usize, blocks: usize) -> (Arc<Self>, Ended) {
        let pace = Self::new(first, blocks);
        *CURRENT.lock().unwrap_or_else(|error| error.into_inner()) = Some(Arc::downgrade(&pace));
        #[cfg(test)]
        STARTED.with(|started| *started.borrow_mut() = Some(Arc::clone(&pace)));
        let ended = Ended(Arc::clone(&pace));
        (pace, ended)
    }

    pub(crate) fn terminal(blocks: usize) -> Arc<Self> {
        Self::new(blocks, blocks)
    }

    fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Change the lifecycle to `lifecycle`, keeping it in tests.
    fn enter(&self, state: &mut Snapshot, lifecycle: Lifecycle) {
        if state.lifecycle != lifecycle {
            state.lifecycle = lifecycle;
            #[cfg(test)]
            self.history
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(lifecycle);
        }
    }

    pub(crate) fn at(&self, block: usize, blocks: usize) {
        let mut state = self.lock();
        if state.lifecycle == Lifecycle::Terminal {
            return;
        }
        if block < blocks {
            state.cursor = block;
            self.enter(&mut state, Lifecycle::Running);
        } else {
            self.enter(&mut state, Lifecycle::Terminal);
        }
        drop(state);
        self.changed.notify_all();
    }

    pub(crate) fn claimed(&self, block: usize) {
        let mut state = self.lock();
        if state.lifecycle != Lifecycle::Terminal {
            state.claimed = Some(state.claimed.map_or(block, |claimed| claimed.max(block)));
        }
        drop(state);
        self.changed.notify_all();
    }

    pub(crate) fn permits(&self, block: usize) -> bool {
        permits(&self.lock(), block)
    }

    pub(crate) fn wait_for<G>(
        &self,
        block: usize,
        cancelled: &AtomicBool,
        park: impl FnOnce() -> G,
    ) -> io::Result<()> {
        if self.permits(block) {
            return Ok(());
        }
        let _parked = park();
        let mut state = self.lock();
        loop {
            crate::metadata::check_cancel(cancelled)?;
            if permits(&state, block) {
                return Ok(());
            }
            state = self
                .changed
                .wait_timeout(state, POLL)
                .map_or_else(|error| error.into_inner().0, |(state, _)| state);
        }
    }

    /// One coherent view of it now.
    pub(crate) fn snapshot(&self) -> Snapshot {
        *self.lock()
    }

    /// Each lifecycle it took, in order.
    #[cfg(test)]
    pub(crate) fn history(&self) -> Vec<Lifecycle> {
        self.history
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

pub(crate) struct Ended(Arc<WalkPace>);

impl Drop for Ended {
    fn drop(&mut self) {
        let pace = &self.0;
        let mut state = pace.lock();
        pace.enter(&mut state, Lifecycle::Terminal);
        drop(state);
        pace.changed.notify_all();
    }
}

/// [`WalkPace::permits`] of one snapshot.
fn permits(state: &Snapshot, block: usize) -> bool {
    state.window().is_none_or(|(_, last)| block <= last)
}

// The acceptance tool prepares one recording at a time; this registry owns no obligation.
static CURRENT: Mutex<Option<Weak<WalkPace>>> = Mutex::new(None);

pub(crate) fn current() -> Option<Snapshot> {
    read_current(&CURRENT)
}

fn read_current(registry: &Mutex<Option<Weak<WalkPace>>>) -> Option<Snapshot> {
    let pace = registry
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()?
        .upgrade();
    pace.map(|pace| pace.snapshot())
}

#[cfg(test)]
thread_local! {
    static STARTED: std::cell::RefCell<Option<Arc<WalkPace>>> =
        const { std::cell::RefCell::new(None) };
}

/// The walk obligation started last on this thread, kept for its tests.
#[cfg(test)]
pub(crate) fn started() -> Option<Arc<WalkPace>> {
    STARTED.with(|started| started.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_walk_reports_only_its_first_block_and_a_running_one_its_claims() {
        let (pace, _ended) = WalkPace::start(3, 10);
        let pending = pace.snapshot();
        assert_eq!(pending.lifecycle, Lifecycle::Pending);
        assert_eq!(pending.window(), Some((3, 3)));
        // Claims before the walk's first loop do not widen a pending window.
        pace.claimed(6);
        assert_eq!(pace.snapshot().window(), Some((3, 3)));
        pace.at(3, 10);
        assert_eq!(pace.snapshot().window(), Some((3, 6)));
        // Claims only grow.
        pace.claimed(4);
        assert_eq!(pace.snapshot().claimed, Some(6));
        pace.at(4, 10);
        assert_eq!(pace.snapshot().window(), Some((4, 6)));
        // Past its lanes' furthest claim, the window is its cursor alone.
        pace.at(7, 10);
        assert_eq!(pace.snapshot().window(), Some((7, 7)));
        assert_eq!(
            pace.history(),
            [Lifecycle::Pending, Lifecycle::Running],
            "one change into running"
        );
    }

    #[test]
    fn terminal_absorbs_and_a_walk_with_nothing_left_starts_terminal() {
        let (pace, _ended) = WalkPace::start(2, 5);
        pace.at(2, 5);
        // At its last block's end it reads no more.
        pace.at(5, 5);
        assert_eq!(pace.snapshot().window(), None);
        pace.at(3, 5);
        pace.claimed(4);
        let ended = pace.snapshot();
        assert_eq!(ended.lifecycle, Lifecycle::Terminal);
        assert_eq!((ended.cursor, ended.claimed), (2, None));
        assert_eq!(
            pace.history(),
            [Lifecycle::Pending, Lifecycle::Running, Lifecycle::Terminal]
        );
        let (done, _ended) = WalkPace::start(5, 5);
        done.at(5, 5);
        assert_eq!(done.snapshot().window(), None);
        assert_eq!(
            done.history(),
            [Lifecycle::Terminal],
            "never pending or running"
        );
    }

    #[test]
    fn the_walks_guard_ends_it_however_the_walk_ends() {
        // Dropped with a walk that is never called.
        let (pace, ended) = WalkPace::start(0, 4);
        let walk = move || {
            let _ended = ended;
        };
        drop(walk);
        assert_eq!(pace.snapshot().lifecycle, Lifecycle::Terminal);
        // Dropped while the walk unwinds.
        let (pace, ended) = WalkPace::start(0, 4);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _ended = ended;
            pace_panics()
        }));
        assert!(unwound.is_err());
        assert_eq!(pace.snapshot().lifecycle, Lifecycle::Terminal);
        // A poisoned obligation still ends, without a panic.
        let (pace, ended) = WalkPace::start(0, 4);
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _state = pace.state.lock().unwrap();
            pace_panics()
        }));
        assert!(poisoned.is_err() && pace.state.is_poisoned());
        drop(ended);
        assert_eq!(pace.snapshot().lifecycle, Lifecycle::Terminal);
    }

    fn pace_panics() {
        panic!("injected walk panic");
    }

    #[test]
    fn the_sampler_uses_a_weak_obligation_and_observes_its_end() {
        let (pace, ended) = WalkPace::start(1, 3);
        let registry = Mutex::new(Some(Arc::downgrade(&pace)));
        assert_eq!(read_current(&registry), Some(pace.snapshot()));
        drop(ended);
        assert_eq!(
            read_current(&registry).unwrap().lifecycle,
            Lifecycle::Terminal
        );
        drop(pace);
        STARTED.with(|started| *started.borrow_mut() = None);
        assert!(read_current(&registry).is_none());
    }

    #[test]
    fn a_permitted_read_waits_for_nothing_and_parks_nothing() {
        let (pace, _ended) = WalkPace::start(2, 10);
        let cancelled = AtomicBool::new(false);
        // Behind a pending walk's first block, or at it.
        for block in [0, 2] {
            pace.wait_for(block, &cancelled, || panic!("parked a permitted read"))
                .unwrap();
        }
    }

    #[test]
    fn a_blocked_read_parks_once_until_the_walk_reaches_it_ends_or_is_cancelled() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Unpark<'a>(&'a AtomicUsize);
        impl Drop for Unpark<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        for ending in ["reached", "ended", "cancelled"] {
            let (pace, ended) = WalkPace::start(0, 10);
            let mut ended = Some(ended);
            let cancelled = AtomicBool::new(false);
            let (parks, unparks) = (AtomicUsize::new(0), AtomicUsize::new(0));
            let (parked, waiting) = std::sync::mpsc::channel();
            let outcome = std::thread::scope(|scope| {
                let waiter = scope.spawn(|| {
                    pace.wait_for(5, &cancelled, || {
                        parks.fetch_add(1, Ordering::SeqCst);
                        parked.send(()).unwrap();
                        Unpark(&unparks)
                    })
                });
                // Parked: the read is blocked.
                waiting.recv().unwrap();
                match ending {
                    "reached" => {
                        pace.at(3, 10);
                        pace.claimed(5);
                    }
                    "ended" => drop(ended.take()),
                    _ => cancelled.store(true, Ordering::Release),
                }
                waiter.join().unwrap()
            });
            assert_eq!(
                (parks.load(Ordering::SeqCst), unparks.load(Ordering::SeqCst)),
                (1, 1),
                "{ending}: parked once, restored once"
            );
            match ending {
                "cancelled" => assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::Interrupted),
                _ => outcome.unwrap(),
            }
        }
    }
}
