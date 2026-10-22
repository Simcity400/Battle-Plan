//! Blocking attempts on the acquisition callback, not worker or status waits.
use std::{
    cell::Cell,
    marker::PhantomData,
    rc::Rc,
    sync::{
        Mutex, MutexGuard, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Copy)]
pub(crate) enum Site {
    Queue,
    Inputs,
    Ranges,
    FinalInputs,
    Readiness,
    Join,
    Drain,
    StoreMutex,
    StoreReturn,
}
const NAMES: [&str; 9] = [
    "queue_mutex",
    "input_fanin",
    "range_fanin",
    "final_inputs",
    "readiness",
    "join",
    "drain",
    "store_mutex",
    "store_return",
];
struct Counters {
    calls: AtomicU64,
    ns: AtomicU64,
    max_ns: AtomicU64,
}
static COUNTERS: [Counters; NAMES.len()] = [const {
    Counters {
        calls: AtomicU64::new(0),
        ns: AtomicU64::new(0),
        max_ns: AtomicU64::new(0),
    }
}; NAMES.len()];
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
}

pub(crate) struct Scope {
    previous: bool,
    _thread: PhantomData<Rc<()>>,
}
impl Scope {
    pub(crate) fn enter() -> Self {
        Self {
            previous: ACTIVE.replace(true),
            _thread: PhantomData,
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.set(self.previous);
    }
}

pub(crate) struct Park(Option<(&'static Counters, Instant)>);
impl Park {
    pub(crate) fn new(site: Site) -> Self {
        Self(ACTIVE.get().then(|| {
            let counters = &COUNTERS[site as usize];
            let begin = Instant::now();
            counters.calls.fetch_add(1, Ordering::Relaxed);
            (counters, begin)
        }))
    }
}
impl Drop for Park {
    fn drop(&mut self) {
        if let Some((counters, begin)) = self.0 {
            let ns = begin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            counters.ns.fetch_add(ns, Ordering::Relaxed);
            counters.max_ns.fetch_max(ns, Ordering::Relaxed);
        }
    }
}

pub(crate) fn lock_callback<T>(mutex: &Mutex<T>, site: Site) -> MutexGuard<'_, T> {
    if !ACTIVE.get() {
        return mutex.lock().unwrap_or_else(|error| error.into_inner());
    }
    match mutex.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => {
            let _park = Park::new(site);
            mutex.lock().unwrap_or_else(|error| error.into_inner())
        }
    }
}

#[derive(serde::Serialize)]
pub(crate) struct Snapshot {
    site: &'static str,
    pub(crate) calls: u64,
    pub(crate) ns: u64,
    pub(crate) max_ns: u64,
}
pub(crate) fn snapshot() -> [Snapshot; NAMES.len()] {
    std::array::from_fn(|index| Snapshot {
        site: NAMES[index],
        calls: COUNTERS[index].calls.load(Ordering::Relaxed),
        ns: COUNTERS[index].ns.load(Ordering::Relaxed),
        max_ns: COUNTERS[index].max_ns.load(Ordering::Relaxed),
    })
}

#[cfg(test)]
mod tests;
