//! A reader's own state of one table, made by its rebuild where the table's lane was sought and
//! carried to it by the reset step. The playhead holds it, counted, until its reader takes it.

use crate::lane::Lane;
use std::{
    any::Any,
    fmt, io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// A reader's state of the table of the lane given, made from the lane's cells of the columns the
/// reader declared of it (ascending). Runs on a fetcher, once the lane is sought.
pub type Rebuild = Arc<dyn Fn(&Lane, &[usize]) -> io::Result<State> + Send + Sync>;

pub struct State {
    value: Option<Box<dyn Any + Send>>,
    bytes: usize,
    flight: Option<Arc<Flight>>,
}

impl State {
    /// `value`, which holds `bytes`.
    pub fn new(value: impl Any + Send, bytes: usize) -> Self {
        Self { value: Some(Box::new(value)), bytes, flight: None }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The value, when it is a `T`.
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

    pub(crate) fn hold(&mut self, flight: &Arc<Flight>) {
        flight.add(self.bytes);
        self.flight = Some(Arc::clone(flight));
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

/// The states made and not yet taken: count and bytes, now and at their peak.
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

    /// (count, bytes) now and at the peak.
    pub(crate) fn read(&self) -> ((usize, usize), (usize, usize)) {
        let load = |held: &AtomicUsize| held.load(Ordering::Relaxed);
        ((load(&self.count), load(&self.bytes)), (load(&self.peak_count), load(&self.peak_bytes)))
    }
}
