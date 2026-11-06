//! A bounded fixture pool with queued owned jobs, no sleeps or admission-time assumptions.
use std::sync::{Arc, Mutex, mpsc};
use viewer_foundation::parallel::{Spawn, Workers};

type Job = Box<dyn FnOnce() + Send>;
struct Pool {
    sender: Mutex<Option<mpsc::Sender<Job>>>,
    threads: Vec<std::thread::JoinHandle<()>>,
}
impl Pool {
    fn new(count: usize) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let threads = (0..count)
            .map(|_| {
                let receiver = receiver.clone();
                job_priority::spawn(move || {
                    loop {
                        let job = receiver.lock().unwrap().recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => return,
                        }
                    }
                })
            })
            .collect();
        Arc::new(Self {
            sender: Mutex::new(Some(sender)),
            threads,
        })
    }
}
impl Spawn for Pool {
    fn try_spawn(&self, job: Job) -> Result<(), Job> {
        let job: Job = Box::new(job_priority::carry(job));
        self.sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(job)
            .map_err(|error| error.0)
    }
    fn threads(&self) -> usize {
        self.threads.len()
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        self.sender.get_mut().unwrap().take();
        for thread in self.threads.drain(..) {
            thread.join().unwrap();
        }
    }
}

pub(super) fn install(scene: &mut viewer_world::map::scene::Scene) {
    scene.use_workers(Workers::new(Pool::new(2)));
    // Only this fixture's independent, otherwise idle pool is allowed to wait.
    scene.test_faults.native_admission = Some(Arc::new(|admitted| {
        admitted.wait_for_helpers(admitted.capacity().saturating_sub(1));
    }));
}

pub(super) fn saw_parallel(scene: &viewer_world::map::scene::Scene) -> bool {
    scene
        .test_faults
        .native_batches
        .borrow()
        .iter()
        .any(|(_, buffers)| buffers.is_some_and(|count| count > 2))
}

#[test]
fn native_fixture_pool_releases_every_batch_and_abandoned_preparation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let workers = Workers::new(Pool::new(2));
    let width = workers.threads();
    for maximum in [0, 1, 7, 2, 7, 0, 7] {
        // Drop an admitted batch without encoding, as preparation failure does.
        workers.with_admitted(width, |admitted| {
            admitted.wait_for_helpers(width - 1);
        });
        let seen = AtomicUsize::new(0);
        let count = workers.with_admitted(width, |admitted| {
            admitted.wait_for_helpers(width - 1);
            admitted.run(maximum, &|_, count| {
                assert_eq!(count, maximum.min(width));
                seen.fetch_add(1, Ordering::Relaxed);
            })
        });
        assert_eq!(count, maximum.min(width));
        assert_eq!(seen.load(Ordering::Relaxed), count);
    }
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        workers.with_admitted(width, |admitted| {
            admitted.wait_for_helpers(width - 1);
            panic!("fixture preparation failure");
        });
    }));
    assert!(failed.is_err());
    workers.with_admitted(width, |admitted| {
        admitted.wait_for_helpers(width - 1);
        assert_eq!(admitted.run(width, &|_, _| {}), width);
    });
}
