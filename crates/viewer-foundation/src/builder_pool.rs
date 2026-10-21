//! Bounded FIFO workers used by hour building and borrowed raid batches.
mod capacity;
pub use capacity::CapacityListener;
use capacity::Capacity;

use crate::{parallel::Spawn, worker_priority::background_priority};
#[cfg(any(test, feature = "test-support"))]
use crate::{parallel::Workers, retirement::builder_count};
use std::{
    io,
    sync::{Arc, Mutex, mpsc},
    thread,
};
type Job = Box<dyn FnOnce() + Send>;

/// Nonblocking admission retains the rejected job and distinguishes a full queue.
pub enum TrySubmitError {
    Full(Job),
    Disconnected(Job),
}

/// Worker threads taking jobs in the order they were given.
///
/// The order matters: a part waits only for the previous hour's part, which
/// was submitted before it and so has already been taken by a thread.
pub struct Pool {
    jobs: Option<mpsc::SyncSender<Job>>,
    threads: Vec<thread::JoinHandle<()>>,
    capacity: Arc<Capacity>,
}

impl Pool {
    pub fn new(count: usize) -> io::Result<Self> {
        let (jobs, inbox) = mpsc::sync_channel::<Job>(count.saturating_mul(8).max(16));
        let inbox = Arc::new(Mutex::new(inbox));
        let capacity = Arc::new(Capacity::default());
        let threads = (0..count)
            .map(|index| {
                let inbox = Arc::clone(&inbox);
                let capacity = Arc::clone(&capacity);
                thread::Builder::new()
                    .name(format!("hour builder {index}"))
                    .spawn(move || {
                        background_priority();
                        loop {
                            let job = match inbox.lock() {
                                Ok(inbox) => inbox.recv(),
                                Err(_) => return,
                            };
                            match job {
                                Ok(job) => {
                                    // Receiver lock is released before capacity callbacks or work.
                                    capacity.dequeued();
                                    job();
                                }
                                Err(_) => return,
                            }
                        }
                    })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            jobs: Some(jobs),
            threads,
            capacity,
        })
    }

    pub fn submit(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
    }

    pub fn try_submit(&self, job: Job) -> Result<(), Job> {
        self.try_submit_status(job).map_err(|error| match error {
            TrySubmitError::Full(job) | TrySubmitError::Disconnected(job) => job,
        })
    }

    pub fn try_submit_status(&self, job: Job) -> Result<(), TrySubmitError> {
        match &self.jobs {
            Some(jobs) => jobs.try_send(job).map_err(|error| match error {
                mpsc::TrySendError::Full(job) => TrySubmitError::Full(job),
                mpsc::TrySendError::Disconnected(job) => TrySubmitError::Disconnected(job),
            }),
            None => Err(TrySubmitError::Disconnected(job)),
        }
    }

    /// Registers before trying admission. Callback only posts a nonblocking wake.
    pub fn listen_capacity(&self, wake: impl Fn() + Send + Sync + 'static) -> CapacityListener {
        self.capacity.listen(wake)
    }

}

/// The builders lend their turns to a batch split across threads
/// ([`crate::parallel::Workers`]): a turn taken while the builders are busy
/// waits in their queue, and the batch does not wait for it.
impl Spawn for Pool {
    fn try_spawn(&self, job: Job) -> Result<(), Job> {
        self.try_submit(job)
    }

    fn threads(&self) -> usize {
        self.threads.len()
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.jobs = None;
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// A builder pool as the viewer starts one, lent as [`crate::parallel::Workers`].
#[cfg(any(test, feature = "test-support"))]
pub fn builder_workers() -> io::Result<Workers> {
    Ok(Workers::new(Arc::new(Pool::new(builder_count())?)))
}

#[cfg(test)]
mod tests;
