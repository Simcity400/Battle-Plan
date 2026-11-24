
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

