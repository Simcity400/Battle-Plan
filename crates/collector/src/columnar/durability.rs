//! Makes the journals' ended segments durable on a background thread, so a slow disk never
//! holds the spool thread, and through it the frames the game waits on. One sync is in flight
//! at most: a segment that ends while the last one's sync runs waits for it, so a power loss
//! can leave at most the last two segments incomplete. One thread serves the session, started
//! with the rows' writer before the hook; each recording learns of its own failed syncs.

use std::{
    fs::File,
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread::JoinHandle,
    time::Instant,
};

const STACK_BYTES: usize = 256 * 1024;
/// Sync walls in 100 µs steps; the last holds every longer one, bounded above by `max_ns`.
const STEP_NS: u64 = 100_000;
const STEPS: usize = 2048;

pub(crate) struct Durability {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// An ended segment and the failure slot of the recording it belongs to.
struct Ended {
    file: File,
    slot: *const Slot,
}

// SAFETY: a slot outlives every segment given with it: its recording settles before freeing it.
unsafe impl Send for Ended {}

struct State {
    ended: Option<Ended>,
    syncing: bool,
    stop: bool,
    timing: Timing,
}

/// One recording's first failed sync; boxed by the recording so its address stays put.
#[derive(Default)]
pub(super) struct Slot(Mutex<Option<io::Error>>);

impl Slot {
    pub(super) fn take(&self) -> Option<io::Error> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// Each sync's wall, and the spool thread's waits for one still in flight.
#[derive(Clone, Copy)]
pub(super) struct Timing {
    steps: [u32; STEPS],
    count: u64,
    max_ns: u64,
    waits: u64,
    wait_ns: u64,
    wait_max_ns: u64,
}

impl Timing {
    const fn new() -> Self {
        Self {
            steps: [0; STEPS],
            count: 0,
            max_ns: 0,
            waits: 0,
            wait_ns: 0,
            wait_max_ns: 0,
        }
    }

    fn record(&mut self, ns: u64) {
        let step = ((ns / STEP_NS) as usize).min(STEPS - 1);
        self.steps[step] = self.steps[step].saturating_add(1);
        self.count += 1;
        self.max_ns = self.max_ns.max(ns);
    }

    /// An upper bound of the `quantile` sync's wall: its step's upper edge, or the longest
    /// sync for the open last step.
    fn quantile_ns(&self, quantile: f64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let rank = ((self.count - 1) as f64 * quantile).round() as u64;
        let mut seen = 0;
        for (step, &count) in self.steps.iter().enumerate() {
            seen += u64::from(count);
            if seen > rank {
                return Some(if step == STEPS - 1 {
                    self.max_ns
                } else {
                    ((step as u64 + 1) * STEP_NS).min(self.max_ns)
                });
            }
        }
        Some(self.max_ns)
    }

    pub(super) fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "count": self.count,
            "p50_ns": self.quantile_ns(0.5),
            "p99_ns": self.quantile_ns(0.99),
            "max_ns": self.max_ns,
            "waits": self.waits,
            "wait_ns": self.wait_ns,
            "wait_max_ns": self.wait_max_ns,
        })
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed.wait(state).unwrap_or_else(PoisonError::into_inner)
    }
}

impl Durability {
    pub(crate) fn start() -> io::Result<Arc<Self>> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                ended: None,
                syncing: false,
                stop: false,
                timing: Timing::new(),
            }),
            changed: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("hoi4-journal-durability".into())
            .stack_size(STACK_BYTES)
            .spawn(move || serve(&worker))?;
        Ok(Arc::new(Self {
            shared,
            thread: Some(thread),
        }))
    }

    /// Hands an ended segment over once the last one's sync is done. `slot` must outlive
    /// the sync: its owner calls [`Self::settle`] before freeing it.
    pub(super) fn give(&self, file: File, slot: &Slot) {
        let mut state = self.shared.lock();
        if state.ended.is_some() || state.syncing {
            let started = Instant::now();
            while state.ended.is_some() || state.syncing {
                state = self.shared.wait(state);
            }
            let waited = started.elapsed().as_nanos() as u64;
            state.timing.waits += 1;
            state.timing.wait_ns += waited;
            state.timing.wait_max_ns = state.timing.wait_max_ns.max(waited);
        }
        state.ended = Some(Ended { file, slot });
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Waits until every segment given is durable or failed.
    pub(super) fn settle(&self) {
        let mut state = self.shared.lock();
        while state.ended.is_some() || state.syncing {
            state = self.shared.wait(state);
        }
    }

    pub(super) fn timing(&self) -> Timing {
        self.shared.lock().timing
    }
}

impl Drop for Durability {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(shared: &Shared) {
    crate::runtime::OwnPriority::new().follow(false);
    let mut state = shared.lock();
    loop {
        let Some(Ended { file, slot }) = state.ended.take() else {
            if state.stop {
                return;
            }
            state = shared.wait(state);
            continue;
        };
        state.syncing = true;
        drop(state);
        let started = Instant::now();
        let synced = file.sync_data();
        let elapsed = started.elapsed().as_nanos() as u64;
        drop(file);
        if let Err(error) = synced {
            // SAFETY: `syncing` is still set, so the slot's owner is not past `settle`.
            let slot = unsafe { &*slot };
            slot.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert(error);
        }
        state = shared.lock();
        state.syncing = false;
        state.timing.record(elapsed);
        shared.changed.notify_all();
    }
}
