
use crate::lane::Lane;
use std::{
    any::Any,
    collections::BTreeMap,
    fmt, io,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

pub type Rebuild = Arc<dyn Fn(&Lane, &[usize]) -> io::Result<State> + Send + Sync>;

type States = Mutex<(u64, BTreeMap<u64, State>)>;

#[derive(Clone, Default)]
pub(crate) struct PendingStates(Arc<States>);

impl PendingStates {
    pub(crate) fn release(&self) -> BTreeMap<u64, State> {
        std::mem::take(&mut self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1)
    }
}

#[derive(Debug)]
pub(crate) struct PendingState(Weak<States>, u64, usize);

impl PendingState {
    pub(crate) fn bytes(&self) -> usize { self.2 }
    pub(crate) fn take(&self) -> Option<State> {
        self.0.upgrade()?.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1.remove(&self.1)
    }
}

impl Drop for PendingState {
    fn drop(&mut self) { drop(self.take()); }
}

pub struct State {
    value: Option<Box<dyn Any + Send>>,
    bytes: usize,
    flight: Option<Arc<Flight>>,
}

impl State {
    pub fn new(value: impl Any + Send, bytes: usize) -> Self {
        Self { value: Some(Box::new(value)), bytes, flight: None }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn downcast<T: Any>(mut self) -> Result<Box<T>, Self> {
        let value = self.value.take().expect("a state holds its value until taken");
        match value.downcast::<T>() {
            Ok(held) => Ok(held),
            Err(value) => {
                self.value = Some(value);
                Err(self)
            }
        }
    }

    pub(crate) fn hold(mut self, flight: &Arc<Flight>, pending: &PendingStates) -> PendingState {
        flight.add(self.bytes);
        self.flight = Some(Arc::clone(flight));
        let mut states = pending.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let held = PendingState(Arc::downgrade(&pending.0), states.0, self.bytes);
        states.0 += 1;
        states.1.insert(held.1, self);
        held
    }

    pub(crate) fn release(&mut self) {
        if let Some(flight) = self.flight.take() {
            flight.remove(self.bytes);
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.release();
    }
}

impl fmt::Debug for State {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("State").field("bytes", &self.bytes).finish_non_exhaustive()
    }
}

#[derive(Default)]
pub(crate) struct Flight {
    count: AtomicUsize,
    bytes: AtomicUsize,
    peak_count: AtomicUsize,
    peak_bytes: AtomicUsize,
}

impl Flight {
    fn add(&self, bytes: usize) {
        let count = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        let held = self.bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak_count.fetch_max(count, Ordering::Relaxed);
        self.peak_bytes.fetch_max(held, Ordering::Relaxed);
    }

    fn remove(&self, bytes: usize) {
        self.count.fetch_sub(1, Ordering::Relaxed);
        self.bytes.fetch_sub(bytes, Ordering::Relaxed);
    }

    pub(crate) fn read(&self) -> ((usize, usize), (usize, usize)) {
        let load = |held: &AtomicUsize| held.load(Ordering::Relaxed);
        ((load(&self.count), load(&self.bytes)), (load(&self.peak_count), load(&self.peak_bytes)))
    }
}
