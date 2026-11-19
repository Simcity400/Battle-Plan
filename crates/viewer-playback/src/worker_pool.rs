//! The app's worker threads, one set a process: the builder pool, and the dropper that frees what a
//! frame retires off the frame path. The frame thread lends the pool nothing and waits for nothing
//! on it. The campaign map and the dashboard's maps share one.

use std::{io, sync::Arc};
use viewer_foundation::{
    builder_pool::Pool,
    parallel::Workers,
    retirement::{Dropper, builder_count},
};

pub struct WorkerPool {
    pool: Arc<Pool>,
    dropper: Arc<Dropper>,
}

impl WorkerPool {
    /// A builder pool of all the hardware threads the process may use (its affinity mask) less
    /// eight, at least two, leaving the window's own frame, the GPU driver and the game's own
    /// threads theirs.
    pub fn new() -> io::Result<Arc<Self>> {
        let pool = Arc::new(Pool::new(builder_count())?);
        Ok(Arc::new(Self { pool, dropper: Arc::new(Dropper::new()?) }))
    }

    /// The builders' threads, lent to work split across threads (scene creation, a raid batch)
    /// while they are free.
    pub fn workers(&self) -> Workers {
        Workers::new(Arc::clone(&self.pool) as _)
    }

    /// A bounded nonblocking admission: false when the queue is full, and the caller asks again
    /// next frame.
    pub fn try_submit(&self, work: Box<dyn FnOnce() + Send>) -> bool {
        self.pool.try_submit(work).is_ok()
    }

    /// Frees what it is given off the frame path.
    pub fn dropper(&self) -> &Arc<Dropper> {
        &self.dropper
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerPool;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn a_job_submitted_to_the_app_pool_runs_on_one_of_its_threads() {
        let pool = WorkerPool::new().unwrap();
        let (sender, ran) = mpsc::channel();
        let frame = std::thread::current().id();
        assert!(pool.try_submit(Box::new(move || {
            let _ = sender.send(std::thread::current().id());
        })));
        let on = ran.recv_timeout(Duration::from_secs(120)).expect("the submitted job ran within 120 s");
        assert_ne!(on, frame, "the job ran on a pool thread, not the submitter's");
    }
}
