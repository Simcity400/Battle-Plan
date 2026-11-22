
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

const TICK: Duration = Duration::from_millis(125);

const THREAD: &str = "rows apart";

pub struct Apart {
    entry: Arc<Entry>,
    store: Store,
    reader: playhead::Reader,
    consumer: playhead::Consumer,
    mirrors: Arc<mirror::Mirrors>,
    branch: BranchId,
    applied: Option<(Hour, u64)>,
}

impl Apart {
    pub fn new(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
        let store = Store::new(entry);
        let reader = entry.playhead().declare(to_declared(&for_the_makers(entry)), Some(mirror::rebuild(Arc::clone(&entry.pages))))?;
        let consumer = entry.playhead().consumer(reader);
        Ok(Self { entry: Arc::clone(entry), store, reader, consumer, mirrors: Arc::default(), branch, applied: None })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn drive(&mut self, hour: Hour) -> bool {
        let entry = Arc::clone(&self.entry);
        self.consumer.advance_blocking(hour);
        let reached = entry.playhead().wait_for(|| {
            self.take_to(hour, true);
            self.applied == Some((hour, entry.playhead().epoch())) && self.home()
        });
        if reached.is_ok() {
            if let Some((applied, epoch)) = self.applied {
                self.store.post(&self.mirrors, applied, self.branch, epoch, false);
            }
            self.store.settled();
        }
        reached.is_ok()
    }

    #[cfg(test)]
    pub fn post_at(&mut self, hour: Hour) {
        self.store.settled();
        let epoch = self.applied.map_or(0, |(_, epoch)| epoch);
        self.store.post(&self.mirrors, hour, self.branch, epoch, false);
        self.store.settled();
    }

    #[cfg(any(test, feature = "test-support"))]
    fn home(&self) -> bool {
        for_the_makers(&self.entry).keys().all(|&table| self.mirrors.get(replay_data::rows::TableId::ALL[table]).is_some())
    }

    fn take_to(&mut self, hour: Hour, blocking: bool) {
        self.store.settled();
        let Some(mirrors) = Arc::get_mut(&mut self.mirrors) else { return };
        let mut steps: Vec<Arc<Step>> = Vec::new();
        #[cfg(any(test, feature = "test-support"))]
        if blocking {
            self.consumer.take_blocking(hour, &mut steps);
        } else {
            self.consumer.take(hour, &mut steps);
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            let _ = blocking;
            self.consumer.take(hour, &mut steps);
        }
        let mut tables = Vec::new();
        for step in &steps {
            if step.last() > hour {
                continue;
            }
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
    }

    fn step(&mut self, hour: Hour) {
        self.consumer.advance(hour);
        self.take_to(hour, false);
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

pub struct Following {
    views: Views,
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Following {
    pub fn open(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
        let mut apart = Apart::new(entry, branch)?;
        let views = apart.store().handle();
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let entry = Arc::clone(entry);
        let thread = std::thread::Builder::new().name(THREAD.into()).spawn({
            let stop = Arc::clone(&stop);
            move || {
                viewer_runtime::worker_priority::background_priority();
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
