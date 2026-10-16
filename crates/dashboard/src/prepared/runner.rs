//! A bounded queue for cancellable background preparation.

use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
};

/// The existing Windows production worker budget, shared with loaders and harnesses.
/// Domain state belongs on the heap; a larger stack must not conceal its growth.
pub const WORKER_STACK_BYTES: usize = 2 * 1024 * 1024;

pub fn worker_thread(name: impl Into<String>) -> thread::Builder {
    thread::Builder::new()
        .name(name.into())
        .stack_size(WORKER_STACK_BYTES)
}

type Work = Box<dyn FnOnce(&AtomicBool) + Send>;

struct Job {
    cancelled: Arc<AtomicBool>,
    work: Work,
}

pub struct Runner {
    revision: Arc<AtomicU64>,
    queue: Mutex<Option<SyncSender<Job>>>,
    cancelled: Mutex<Vec<Weak<AtomicBool>>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Runner {
    pub fn new() -> io::Result<Self> {
        let (send, receive) = mpsc::sync_channel::<Job>(2);
        let receive = Arc::new(Mutex::new(receive));
        let revision = Arc::new(AtomicU64::new(0));
        let mut threads = Vec::new();
        for index in 0..2 {
            let receive = Arc::clone(&receive);
            let revision = Arc::clone(&revision);
            match worker_thread(format!("replay preparation {index}")).spawn(move || {
                background_priority();
                loop {
                    let job = receive
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .recv();
                    let Ok(job) = job else { break };
                    // A work item owns its domain error/panic reporting; isolate
                    // the queue too so one failed campaign cannot stop another.
                    let _ = catch_unwind(AssertUnwindSafe(|| (job.work)(&job.cancelled)));
                    // A finished item changes queue availability. Idle admission
                    // waits for this instead of retrying a no-progress reply.
                    revision.fetch_add(1, Ordering::Release);
                }
            }) {
                Ok(thread) => threads.push(thread),
                Err(error) => {
                    drop(send);
                    for thread in threads {
                        let _ = thread.join();
                    }
                    return Err(error);
                }
            }
        }
        Ok(Self {
            revision,
            queue: Mutex::new(Some(send)),
            cancelled: Mutex::new(Vec::new()),
            threads: Mutex::new(threads),
        })
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub(crate) fn notify_change(&self) {
        self.revision.fetch_add(1, Ordering::Release);
    }

    pub fn submit(
        &self,
        cancelled: Arc<AtomicBool>,
        work: impl FnOnce(&AtomicBool) + Send + 'static,
    ) -> io::Result<()> {
        let queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        let send = queue
            .as_ref()
            .ok_or_else(|| io::Error::other("Preparation is closing"))?;
        let job = Job {
            cancelled: Arc::clone(&cancelled),
            work: Box::new(work),
        };
        match send.try_send(job) {
            Ok(()) => {
                let mut active = self
                    .cancelled
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                active.retain(|flag| flag.strong_count() > 0);
                active.push(Arc::downgrade(&cancelled));
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Preparation workers are busy",
            )),
            Err(TrySendError::Disconnected(_)) => Err(io::Error::other("Preparation is closing")),
        }
    }

    pub fn shutdown(&self) {
        // Lock in submission order so no admitted job escapes cancellation.
        let queue = self
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        for cancelled in self
            .cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter_map(Weak::upgrade)
        {
            cancelled.store(true, Ordering::Release);
        }
        drop(queue);
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let current = thread::current().id();
        for thread in threads.drain(..) {
            if thread.thread().id() != current {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn background_priority() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn worker_completion_wakes_admission_once_without_polling_jobs() {
        let runner = Runner::new().unwrap();
        assert_eq!(runner.revision(), 0);
        runner
            .submit(Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        // Joining proves completion without a request/status poll loop.
        runner.shutdown();
        assert_eq!(runner.revision(), 1);
        for _ in 0..5_000 {
            assert_eq!(runner.revision(), 1);
        }
        runner.notify_change();
        assert_eq!(runner.revision(), 2);
    }

    #[test]
    fn shutdown_cancels_admitted_work_and_joins_owned_threads() {
        let runner = Runner::new().unwrap();
        let (started, receive) = mpsc::channel();
        let flag = Arc::new(AtomicBool::new(false));
        runner
            .submit(Arc::clone(&flag), move |cancelled| {
                started.send(()).unwrap();
                while !cancelled.load(Ordering::Acquire) {
                    thread::yield_now();
                }
            })
            .unwrap();
        receive.recv_timeout(Duration::from_secs(5)).unwrap();
        runner.shutdown();
        assert!(flag.load(Ordering::Acquire));
        assert!(
            runner
                .submit(Arc::new(AtomicBool::new(false)), |_| {})
                .is_err()
        );
    }

    #[test]
    fn panicking_work_does_not_stop_the_preparation_queue() {
        let runner = Runner::new().unwrap();
        runner
            .submit(Arc::new(AtomicBool::new(false)), |_| {
                panic!("synthetic preparation failure")
            })
            .unwrap();
        let (send, receive) = mpsc::channel();
        runner
            .submit(Arc::new(AtomicBool::new(false)), move |_| {
                send.send(()).unwrap();
            })
            .unwrap();
        receive.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
