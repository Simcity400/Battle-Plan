//! Frame-only idle helpers, then borrowed hour builders, with no frame queue.

use job_priority::SpawnCarried as _;
use crate::parallel::{HelperSource, Spawn, Workers};
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex},
    thread,
};

type Job = Box<dyn FnOnce() + Send>;

/// Protected helpers use the headroom left by the caller and hour builders.
/// The advertised batch width remains the builders' width plus the caller.
pub fn helper_count(processors: usize, builders: usize) -> usize {
    let headroom = processors.saturating_sub(builders.saturating_add(1));
    processors.saturating_sub(1).min((headroom / 2).max(1))
}

#[derive(Default)]
struct State {
    busy: bool,
    stopping: bool,
    job: Option<Job>,
}

#[derive(Default)]
struct Slot {
    state: Mutex<State>,
    ready: Condvar,
}

impl Slot {
    fn run(&self) {
        crate::worker_priority::enter(crate::worker_priority::Class::FrameWork);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(job) = state.job.take() {
                drop(state);
                // Borrowed turns catch and return their own panics. An owned
                // transport job must not strand this slot if it also panics.
                let _ = catch_unwind(AssertUnwindSafe(job));
                state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.busy = false;
                self.ready.notify_all();
            } else if state.stopping {
                return;
            } else {
                state = self.ready.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}

/// One shared service per native root. Only synchronous frame turns use it;
/// long-lived hour and raid work continues to use the builders directly.
pub struct FrameAssist {
    builders: Arc<dyn Spawn>,
    slots: Vec<Arc<Slot>>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl FrameAssist {
    pub fn new(builders: Arc<dyn Spawn>) -> io::Result<Self> {
        let count = helper_count(crate::parallel::usable_processors(), builders.threads());
        Self::with_helpers(builders, count)
    }

    fn with_helpers(builders: Arc<dyn Spawn>, count: usize) -> io::Result<Self> {
        let mut service = Self {
            builders,
            slots: Vec::with_capacity(count),
            threads: Vec::with_capacity(count),
        };
        for index in 0..count {
            let slot = Arc::new(Slot::default());
            let run = Arc::clone(&slot);
            let thread = thread::Builder::new()
                .name(format!("frame assist {index}"))
                .spawn_carried(move || run.run())?;
            service.slots.push(slot);
            service.threads.push(thread);
        }
        // A partial spawn failure drops the service and joins every thread
        // already created, exactly as normal shutdown does.
        Ok(service)
    }

    /// Explicit protected capacity for deterministic consumer fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_helpers_for_tests(builders: Arc<dyn Spawn>, count: usize) -> io::Result<Self> {
        Self::with_helpers(builders, count)
    }

    /// Waits only in fixtures, after a scope releases its borrowed callbacks.
    #[cfg(any(test, feature = "test-support"))]
    pub fn wait_idle_for_tests(&self) {
        for slot in &self.slots {
            let mut state = slot.state.lock().unwrap_or_else(|e| e.into_inner());
            while state.busy {
                state = slot.ready.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}

impl Spawn for FrameAssist {
    fn try_spawn(&self, job: Job) -> Result<(), Job> {
        self.try_spawn_classified(job).map(|_| ())
    }

    fn try_spawn_classified(&self, job: Job) -> Result<HelperSource, Job> {
        // Run with the submitter's recording context, whichever helper takes it.
        let job: Job = Box::new(job_priority::carry(job));
        for slot in &self.slots {
            let mut state = slot.state.lock().unwrap_or_else(|e| e.into_inner());
            if !state.busy && !state.stopping {
                state.busy = true;
                state.job = Some(job);
                drop(state);
                slot.ready.notify_all();
                return Ok(HelperSource::Protected);
            }
        }
        self.builders.try_spawn(job).map(|()| HelperSource::Builder)
    }

    fn threads(&self) -> usize {
        self.builders.threads()
    }
}

impl Drop for FrameAssist {
    fn drop(&mut self) {
        for slot in &self.slots {
            slot.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .stopping = true;
            slot.ready.notify_all();
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Frame workers share protected capacity; spawn failure retains builder admission.
pub fn workers(builders: Arc<dyn Spawn>) -> Workers {
    match FrameAssist::new(Arc::clone(&builders)) {
        Ok(assist) => Workers::new(Arc::new(assist)),
        Err(error) => {
            eprintln!("frame assist unavailable: {error}");
            Workers::new(builders)
        }
    }
}

#[cfg(test)]
mod tests;
