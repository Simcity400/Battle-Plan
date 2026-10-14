//! A persistent conversion helper for the spool thread; the spool thread
//! starts two ([`Helpers`]).
//!
//! [`SpoolHelper::join`] runs a borrowed job on the helper while the calling
//! spool thread runs its own share, and returns only after both finished. The
//! helper is started once, before hooks are published, and parks between
//! hours: an hour allocates nothing and spawns nothing. It runs below normal
//! priority, following the spool thread's verdict when capture buffers are
//! backlogged, so it never competes with the game above the spool thread.
//! Output placement is fixed by the caller, so the result is identical to
//! running both shares on one thread.

use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread::JoinHandle,
};

/// A job borrowed for one [`SpoolHelper::join`] call.
pub(crate) type Job<'a> = dyn Fn() + Sync + 'a;

/// The conversion helpers a spool frame may use and the spool thread's
/// backlog verdict, which they follow.
#[derive(Clone, Copy)]
pub(crate) struct Helpers<'a> {
    pub(crate) first: &'a SpoolHelper,
    pub(crate) second: Option<&'a SpoolHelper>,
    pub(crate) backlogged: bool,
}

impl<'a> Helpers<'a> {
    /// Run `job` on every helper and `own` on this thread; return after all
    /// finished. A panic on any side is resumed after every job finished.
    pub(crate) fn join_each<R>(self, job: &'a Job<'a>, own: impl FnOnce() -> R) -> R {
        match self.second {
            Some(second) => self.first.join(self.backlogged, job, || {
                second.join(self.backlogged, job, own)
            }),
            None => self.first.join(self.backlogged, job, own),
        }
    }
}

/// The job pointer with its borrow erased. Only `join` publishes it, and
/// `join` neither returns nor unwinds before the helper reports the job
/// finished, so the pointee outlives every use.
#[derive(Clone, Copy)]
struct Erased(*const Job<'static>);
// SAFETY: the pointee is `Sync` and outlives its use (see `Erased`).
unsafe impl Send for Erased {}

#[derive(Default)]
struct State {
    job: Option<Erased>,
    running: bool,
    panicked: bool,
    backlogged: bool,
    stop: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) struct SpoolHelper {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    thread_id: u32,
}

impl SpoolHelper {
    /// Start the helper thread and wait for its id.
    pub(crate) fn start() -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        let (id_sender, id_receiver) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("hoi4-spool-helper".into())
            .stack_size(crate::runtime::BACKGROUND_STACK_BYTES)
            .spawn(move || {
                #[cfg(feature = "worker_sampler")]
                let _sampler_exit = crate::thread_cpu::StopOnThreadExit;
                // SAFETY: GetCurrentThreadId has no preconditions.
                let id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
                let _ = id_sender.send(id);
                serve(&worker);
            })?;
        let thread_id = id_receiver
            .recv()
            .map_err(|_| io::Error::from(io::ErrorKind::Other))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            thread_id,
        })
    }

    pub(crate) fn thread_id(&self) -> u32 {
        self.thread_id
    }

    /// Run `job` on the helper and `own` on this thread; return `own`'s
    /// result after both finished. A panic on either side is resumed here
    /// after the helper finished with the job.
    pub(crate) fn join<'a, R>(
        &self,
        backlogged: bool,
        job: &'a Job<'a>,
        own: impl FnOnce() -> R,
    ) -> R {
        // SAFETY: erases the borrow of `job`; `Finish` below waits for the
        // helper before this frame can return or unwind.
        let erased =
            Erased(unsafe { std::mem::transmute::<&'a Job<'a>, &'static Job<'static>>(job) });
        {
            let mut state = self.shared.lock();
            state.job = Some(erased);
            state.panicked = false;
            state.backlogged = backlogged;
            self.shared.changed.notify_all();
        }
        /// Waits for the job while `own` unwinds.
        struct Finish<'a>(&'a SpoolHelper);
        impl Drop for Finish<'_> {
            fn drop(&mut self) {
                self.0.wait();
            }
        }
        let finish = Finish(self);
        let result = own();
        std::mem::forget(finish);
        if self.wait() {
            panic!("spool helper job panicked");
        }
        result
    }

    /// Wait until the published job is no longer referenced. A helper thread
    /// that ended cannot hold it. Returns whether the job panicked.
    fn wait(&self) -> bool {
        let mut state = self.shared.lock();
        loop {
            if state.job.is_none() && !state.running {
                return state.panicked;
            }
            if self
                .thread
                .as_ref()
                .is_none_or(|thread| thread.is_finished())
            {
                state.job = None;
                return true;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, std::time::Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for SpoolHelper {
    fn drop(&mut self) {
        #[cfg(feature = "worker_sampler")]
        if self.thread.is_some() {
            crate::worker_sampler::stop_before_thread_exit();
        }
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(shared: &Shared) {
    let mut priority = crate::runtime::OwnPriority::new();
    let mut state = shared.lock();
    loop {
        if let Some(job) = state.job {
            state.running = true;
            let backlogged = state.backlogged;
            drop(state);
            priority.follow(backlogged);
            // SAFETY: `join` keeps the job alive until `running` clears.
            let job = unsafe { &*job.0 };
            let panicked = catch_unwind(AssertUnwindSafe(job)).is_err();
            state = shared.lock();
            state.job = None;
            state.running = false;
            state.panicked = panicked;
            shared.changed.notify_all();
        } else if state.stop {
            return;
        } else {
            priority.follow(false);
            state = shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    #[test]
    fn jobs_run_on_one_persistent_thread_and_join_waits_for_both_shares() {
        let helper = SpoolHelper::start().unwrap();
        let ran_on = AtomicU32::new(0);
        let total = AtomicUsize::new(0);
        for round in 1..=5 {
            let job = || {
                std::thread::sleep(std::time::Duration::from_millis(5));
                total.fetch_add(round, Ordering::Relaxed);
                let id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
                let previous = ran_on.swap(id, Ordering::Relaxed);
                assert!(previous == 0 || previous == id);
            };
            let own = helper.join(round % 2 == 0, &job, || round * 10);
            assert_eq!(own, round * 10);
        }
        assert_eq!(total.load(Ordering::Relaxed), 15);
        assert_eq!(ran_on.load(Ordering::Relaxed), helper.thread_id());
        assert_ne!(helper.thread_id(), unsafe {
            windows_sys::Win32::System::Threading::GetCurrentThreadId()
        });
    }

    #[test]
    fn a_helper_panic_is_resumed_and_an_own_panic_still_waits_for_the_job() {
        let helper = SpoolHelper::start().unwrap();
        let failing = || panic!("synthetic helper failure");
        let resumed = catch_unwind(AssertUnwindSafe(|| helper.join(false, &failing, || 1)));
        assert!(resumed.is_err());
        // The helper survives a job panic.
        assert_eq!(helper.join(false, &|| {}, || 2), 2);
        let finished = AtomicUsize::new(0);
        let slow = || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            finished.fetch_add(1, Ordering::Relaxed);
        };
        let own_panic = catch_unwind(AssertUnwindSafe(|| {
            helper.join(false, &slow, || panic!("synthetic own failure"))
        }));
        assert!(own_panic.is_err());
        // Unwinding past join waited for the borrowed job to finish.
        assert_eq!(finished.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn joining_allocates_nothing_on_the_spool_thread() {
        let helper = SpoolHelper::start().unwrap();
        let count = AtomicUsize::new(0);
        let job = || {
            count.fetch_add(1, Ordering::Relaxed);
        };
        helper.join(false, &job, || ());
        crate::allocation_failure_tests::with_allocations_denied(|| {
            helper.join(true, &job, || ());
        });
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
