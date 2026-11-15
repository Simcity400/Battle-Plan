//! The app's worker threads, one set a process: the builder pool, the frame turns lent from it,
//! the in-flight width the maps on it share, and the dropper that frees what a frame retires off
//! the frame path. The campaign map and the dashboard's maps share one; the window reaches it
//! without today's hour pipeline, which borrows it while it exists.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use viewer_foundation::{
    builder_pool::Pool,
    parallel::{Spawn, Workers},
    retirement::{Dropper, builder_count},
};

pub struct WorkerPool {
    pool: Arc<Pool>,
    frame_workers: Workers,
    width: Arc<Width>,
    dropper: Arc<Dropper>,
}

impl WorkerPool {
    /// A builder pool of all the hardware threads the process may use (its affinity mask) less
    /// eight, at least two, leaving the window's own frame, the GPU driver and the game's own
    /// threads theirs.
    pub fn new() -> io::Result<Arc<Self>> {
        let pool = Arc::new(Pool::new(builder_count())?);
        let frame_workers = viewer_foundation::frame_assist::workers(pool.clone());
        let width = Width::new(pool.threads());
        Ok(Arc::new(Self { pool, frame_workers, width, dropper: Arc::new(Dropper::new()?) }))
    }

    /// The builders' threads, lent to work split across threads (scene creation, a raid batch)
    /// while they are free.
    pub fn workers(&self) -> Workers {
        Workers::new(Arc::clone(&self.pool) as _)
    }

    /// Synchronous frame turns: protected helpers, then borrowed builders.
    pub fn frame_workers(&self) -> Workers {
        self.frame_workers.clone()
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

    pub(crate) fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }

    /// The pool's threads.
    pub(crate) fn threads(&self) -> usize {
        self.pool.threads()
    }

    /// The in-flight width every map sharing this pool takes its share of.
    pub(crate) fn width(&self) -> &Arc<Width> {
        &self.width
    }
}

/// The builders' in-flight width over one pool. Each view with hours to build takes an even
/// share, at least one hour, so N views keep one pool's worth in flight, not N.
pub(crate) struct Width {
    total: usize,
    busy: AtomicU64,
}

impl Width {
    pub(crate) fn new(total: usize) -> Arc<Self> {
        Arc::new(Self {
            total: total.max(1),
            busy: AtomicU64::new(0),
        })
    }

    /// This view's share while it has hours to build.
    pub(crate) fn share(&self) -> usize {
        let busy = usize::try_from(self.busy.load(Ordering::Relaxed))
            .unwrap_or(usize::MAX)
            .max(1);
        self.total.div_ceil(busy).max(1)
    }

    pub(crate) fn set_busy(&self, was: bool, now: bool) {
        match (was, now) {
            (false, true) => {
                self.busy.fetch_add(1, Ordering::Relaxed);
            }
            (true, false) => {
                self.busy.fetch_sub(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerPool;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn a_job_submitted_to_the_app_pool_runs_on_one_of_its_threads_with_no_hour_pipeline() {
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
