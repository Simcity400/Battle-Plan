//! A view store of a playhead no map side reads: its own mirrors, the steps to an hour applied to
//! them, and the views of every set its readers declared posted as a step would.
//!
//! A reader reads the store the map side makes its views in. The dashboard's follow panels run on
//! a playhead of their own ([`crate::playhead_host::Side::Follow`]) that no map draws, and a test
//! of a maker needs a side or this: the same mirrors, the same noting and the same posting, with
//! the hours a thread of its own follows, or the ones a test asks for, instead of the ones a map
//! side's step reaches.

use super::{Store, Views};
use crate::playhead_host::{Entry, for_the_makers, mirror, to_declared};
use playhead::Step;
use replay_data::{BranchId, Hour};
use std::{
    io,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// How often the thread that follows a head takes the steps to it. A reader here feeds no picture
/// of the map and asks for views a few times a second, so the steps between are applied in one
/// batch rather than at a frame's rate.
const TICK: Duration = Duration::from_millis(125);

/// The name of the thread one store apart is stepped on.
const THREAD: &str = "rows apart";

/// A store its owner steps: the hours it is driven to are applied to its own mirrors.
pub struct Apart {
    entry: Arc<Entry>,
    store: Store,
    reader: playhead::Reader,
    consumer: playhead::Consumer,
    mirrors: Arc<mirror::Mirrors>,
    branch: BranchId,
    /// The last hour applied, and the seek of the step that applied it.
    applied: Option<(Hour, u64)>,
}

impl Apart {
    /// A store of `entry`'s rows, nothing declared yet.
    pub fn new(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
        let store = Store::new(entry);
        // Every table the recording has, as a map side declares them: a table a reader learns is
        // already at the head.
        let reader = entry.playhead().declare(to_declared(&for_the_makers(entry)), Some(mirror::rebuild(Arc::clone(&entry.pages))))?;
        let consumer = entry.playhead().consumer(reader);
        Ok(Self { entry: Arc::clone(entry), store, reader, consumer, mirrors: Arc::default(), branch, applied: None })
    }

    /// The store its readers open on.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Every step to `hour` applied to the mirrors and the views its readers read posted and made,
    /// waiting for the hour's steps and for every table the recording has to be home (a seek's
    /// reset brings them); false once the playhead stops first.
    #[cfg(any(test, feature = "test-support"))]
    pub fn drive(&mut self, hour: Hour) -> bool {
        let entry = Arc::clone(&self.entry);
        let reached = entry.playhead().wait_for(|| {
            self.step(hour);
            self.applied == Some((hour, entry.playhead().epoch())) && self.home()
        });
        if reached.is_ok() {
            self.store.settled();
        }
        reached.is_ok()
    }

    /// Its mirrors posted as if they stood at `hour`, as a step that left a table stalled short of
    /// its last hour posts them.
    #[cfg(test)]
    pub fn post_at(&mut self, hour: Hour) {
        self.store.settled();
        let epoch = self.applied.map_or(0, |(_, epoch)| epoch);
        self.store.post(&self.mirrors, hour, self.branch, epoch, false);
        self.store.settled();
    }

    /// Whether the mirrors hold every table declared.
    #[cfg(any(test, feature = "test-support"))]
    fn home(&self) -> bool {
        for_the_makers(&self.entry).keys().all(|&table| self.mirrors.get(replay_data::rows::TableId::ALL[table]).is_some())
    }

    /// The steps to `hour` applied, and the views posted.
    fn step(&mut self, hour: Hour) {
        self.store.settled();
        let mut steps: Vec<Arc<Step>> = Vec::new();
        self.consumer.advance(hour);
        self.consumer.take(hour, &mut steps);
        let Some(mirrors) = Arc::get_mut(&mut self.mirrors) else { return };
        let mut tables = Vec::new();
        for step in &steps {
            if step.last() > hour {
                continue;
            }
            // A table joins as a reset whose rebuilt mirror the playhead attached for this reader.
            for changes in step.tables() {
                if changes.reset()
                    && let Some(state) = self.consumer.state(step, changes.table())
                    && let Ok(mut rebuilt) = state.downcast::<mirror::Mirror>()
                {
                    rebuilt.set_capture(self.entry.capture.get().copied());
                    mirrors.insert(rebuilt);
                }
            }
            mirrors.apply_timed(step, &mut tables, &mut Default::default(), &mut self.store.noting(&[]));
            self.applied = Some((step.last(), step.epoch()));
        }
        // The mirrors are posted at the seek their steps were of: before a seek's reset is
        // applied they are the history before it.
        if let Some((applied, epoch)) = self.applied {
            self.store.post(&self.mirrors, applied, self.branch, epoch, false);
        }
    }
}

impl Drop for Apart {
    fn drop(&mut self) {
        self.entry.playhead().retire(self.reader);
    }
}

/// An [`Apart`] stepped on a thread of its own, following its playhead's own head: what serves the
/// readers of a playhead no map draws.
pub struct Following {
    views: Views,
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Following {
    /// A store of `entry`'s rows, stepped to its playhead's head from now on.
    pub fn open(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
        let mut apart = Apart::new(entry, branch)?;
        let views = apart.store().handle();
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let entry = Arc::clone(entry);
        let thread = std::thread::Builder::new().name(THREAD.into()).spawn({
            let stop = Arc::clone(&stop);
            move || {
                viewer_foundation::worker_priority::background_priority();
                loop {
                    apart.step(entry.playhead().head(Instant::now()));
                    let mut stopping = stop.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if *stopping {
                        return;
                    }
                    stopping = stop.1.wait_timeout(stopping, TICK).unwrap_or_else(std::sync::PoisonError::into_inner).0;
                    if *stopping {
                        return;
                    }
                }
            }
        })?;
        Ok(Self { views, stop, thread: Some(thread) })
    }

    /// The store its readers open on.
    pub fn views(&self) -> &Views {
        &self.views
    }
}

impl Drop for Following {
    fn drop(&mut self) {
        *self.stop.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.stop.1.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
