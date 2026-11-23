
use std::{io, sync::Arc};
use viewer_runtime::{builder_pool::Pool, parallel::Workers, retirement::{Dropper, builder_count}};

pub struct WorkerPool {
    pool: Arc<Pool>,
    dropper: Arc<Dropper>,
}

impl WorkerPool {
    pub fn new() -> io::Result<Arc<Self>> {
        let pool = Arc::new(Pool::new(builder_count())?);
        Ok(Arc::new(Self { pool, dropper: Arc::new(Dropper::new()?) }))
    }

    pub fn workers(&self) -> Workers {
        Workers::new(Arc::clone(&self.pool) as _)
    }

    pub fn try_submit(&self, work: Box<dyn FnOnce() + Send>) -> bool {
        self.pool.try_submit(work).is_ok()
    }

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
