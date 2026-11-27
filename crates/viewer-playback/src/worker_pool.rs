
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

    pub fn try_submit(&self, work: Box<dyn FnOnce() + Send>, wake: impl FnOnce() + Send + 'static) -> bool {
        self.pool
            .try_submit(Box::new(move || {
                work();
                wake();
            }))
            .is_ok()
    }

    pub fn dropper(&self) -> &Arc<Dropper> {
        &self.dropper
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn submitted_work_runs_its_wake_after_finishing() {
        let pool = WorkerPool::new().unwrap();
        let (send, receive) = mpsc::channel();
        let work_send = send.clone();
        assert!(pool.try_submit(
            Box::new(move || {
                work_send.send("work").unwrap();
            }),
            move || {
                send.send("wake").unwrap();
            },
        ));
        let first = receive.recv_timeout(Duration::from_secs(10)).unwrap();
        let second = receive.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!([first, second], ["work", "wake"]);
    }
}

