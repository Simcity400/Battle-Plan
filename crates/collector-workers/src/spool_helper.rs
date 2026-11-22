use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread::JoinHandle,
};

pub const MAX_HELPERS: usize = 31;

pub type Job<'a> = dyn Fn() + Sync + 'a;

pub fn pool_size() -> usize {
    crate::acquisition::usable_cpus()
        .saturating_sub(1)
        .min(MAX_HELPERS)
}

const ENCODING_HELPERS: usize = 2;

pub struct Helpers<'a> {
    pub pool: &'a mut [SpoolHelper],
    pub backlogged: bool,
}

impl<'a> Helpers<'a> {
    pub fn encoding_lanes(&self) -> usize {
        1 + self.pool.len().min(ENCODING_HELPERS)
    }

    pub fn join_each<R>(&mut self, job: &Job<'_>, own: impl FnOnce() -> R) -> R {
        self.join_some(ENCODING_HELPERS, &|_: usize| job(), own)
    }

    pub fn join_some<R>(
        &mut self,
        count: usize,
        job: &(dyn Fn(usize) + Sync),
        own: impl FnOnce() -> R,
    ) -> R {
        let mut own = Some(own);
        let mut result = None;
        let count = count.min(self.pool.len());
        let helpers = &mut self.pool[..count];
        join_from(helpers, 0, self.backlogged, job, &mut || {
            result = own.take().map(|own| own());
        });
        result.expect("the own share ran once")
    }
}

fn join_from(
    helpers: &mut [SpoolHelper],
    at: usize,
    backlogged: bool,
    job: &(dyn Fn(usize) + Sync),
    own: &mut dyn FnMut(),
) {
    match helpers.split_first_mut() {
        None => own(),
        Some((helper, rest)) => {
            let mine = || job(at);
            helper.join(backlogged, &mine, || {
                join_from(rest, at + 1, backlogged, job, own)
            });
        }
    }
}

#[derive(Clone, Copy)]
struct Erased(*const Job<'static>);
// SAFETY: the pointee is `Sync` and outlives its use (see `Erased`).
unsafe impl Send for Erased {}

#[derive(Default)]
struct State {
    job: Option<Erased>,
    running: bool,
    panicked: bool,
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

pub struct SpoolHelper {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    thread_id: u32,
    #[cfg(feature = "worker_sampler")]
    sampler: Option<crate::SamplerJoin>,
}

impl SpoolHelper {
    pub fn start(
        owner: &crate::Owner,
        pipeline: engine_allocation::SharedBox<crate::frame_priority::Pipeline>,
        index: usize,
    ) -> io::Result<Self> {
        if index >= MAX_HELPERS {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Self::start_worker(
            owner.worker(),
            Some((pipeline, crate::frame_priority::Role::Helper(index))),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn start_for_test() -> io::Result<Self> {
        Self::start_worker(crate::WorkerStart::for_test(), None)
    }

    fn start_worker(
        start: crate::WorkerStart,
        priority: Option<(
            engine_allocation::SharedBox<crate::frame_priority::Pipeline>,
            crate::frame_priority::Role,
        )>,
    ) -> io::Result<Self> {
        #[cfg(feature = "worker_sampler")]
        let sampler = start.sampler_join();
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        let (id_sender, id_receiver) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("hoi4-spool-helper".into())
            .stack_size(crate::priority::BACKGROUND_STACK_BYTES)
            .spawn(move || {
                crate::private_heap::off_hold_path();
                let registration = start.bind();
                #[cfg(feature = "worker_sampler")]
                let _sampler_exit = registration.sampled();
                let id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
                let _ = id_sender.send(id);
                serve(&worker, priority, &registration);
            })?;
        let thread_id = id_receiver
            .recv()
            .map_err(|_| io::Error::from(io::ErrorKind::Other))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            thread_id,
            #[cfg(feature = "worker_sampler")]
            sampler,
        })
    }

    pub fn thread_id(&self) -> u32 {
        self.thread_id
    }

    fn join<'a, R>(&mut self, _backlogged: bool, job: &'a Job<'a>, own: impl FnOnce() -> R) -> R {
        // SAFETY: erases the borrow of `job`; `Finish` below waits for the
        // helper before this frame can return or unwind.
        let erased =
            Erased(unsafe { std::mem::transmute::<&'a Job<'a>, &'static Job<'static>>(job) });
        {
            let mut state = self.shared.lock();
            state.job = Some(erased);
            state.panicked = false;
            self.shared.changed.notify_all();
        }
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
        if self.thread.is_some()
            && let Some(sampler) = self.sampler.take()
        {
            sampler.before_join();
        }
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    shared: &Shared,
    pipeline: Option<(
        engine_allocation::SharedBox<crate::frame_priority::Pipeline>,
        crate::frame_priority::Role,
    )>,
    worker: &crate::Worker,
) {
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::SpoolHelper);
    let mut priority = crate::priority::OwnPriority::frame_pipeline(worker);
    let mut refresh = || {
        if let Some((pipeline, role)) = &pipeline {
            priority.follow_pipeline(worker, pipeline, *role);
        } else {
            priority.follow(false);
        }
    };
    refresh();
    let mut state = shared.lock();
    loop {
        if let Some(job) = state.job {
            state.running = true;
            drop(state);
            refresh();
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
            refresh();
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
    fn frame_conversion_is_normal_even_without_buffer_pressure() {
        use std::sync::atomic::AtomicI32;
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_NORMAL,
        };
        let mut helper = SpoolHelper::start_for_test().unwrap();
        for backlogged in [false, true, false] {
            let observed = AtomicI32::new(i32::MIN);
            helper.join(
                backlogged,
                &|| {
                    observed.store(
                        unsafe { GetThreadPriority(GetCurrentThread()) },
                        Ordering::Relaxed,
                    );
                },
                || (),
            );
            assert_eq!(observed.load(Ordering::Relaxed), THREAD_PRIORITY_NORMAL);
        }
    }

    #[test]
    fn jobs_run_on_one_persistent_thread_and_join_waits_for_both_shares() {
        let mut helper = SpoolHelper::start_for_test().unwrap();
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
        let mut helper = SpoolHelper::start_for_test().unwrap();
        let failing = || panic!("synthetic helper failure");
        let resumed = catch_unwind(AssertUnwindSafe(|| helper.join(false, &failing, || 1)));
        assert!(resumed.is_err());
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
        assert_eq!(finished.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn join_some_runs_each_index_once_on_its_own_helper_beside_the_own_share() {
        let mut pool = [
            SpoolHelper::start_for_test().unwrap(),
            SpoolHelper::start_for_test().unwrap(),
            SpoolHelper::start_for_test().unwrap(),
        ];
        for count in [1, 2, 3, 7] {
            let ran: [AtomicU32; 3] = std::array::from_fn(|_| AtomicU32::new(0));
            let runs = AtomicUsize::new(0);
            let job = |index: usize| {
                runs.fetch_add(1, Ordering::Relaxed);
                let id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
                assert_eq!(ran[index].swap(id, Ordering::Relaxed), 0);
            };
            let own = Helpers {
                pool: &mut pool,
                backlogged: false,
            }
            .join_some(count, &job, || count * 10);
            assert_eq!(own, count * 10);
            let used = count.min(pool.len());
            assert_eq!(runs.load(Ordering::Relaxed), used);
            for (index, helper) in pool.iter().enumerate() {
                let expected = if index < used { helper.thread_id() } else { 0 };
                assert_eq!(ran[index].load(Ordering::Relaxed), expected);
            }
        }
        assert_eq!(
            Helpers {
                pool: &mut pool,
                backlogged: false
            }
            .encoding_lanes(),
            1 + ENCODING_HELPERS
        );
    }

    #[test]
    fn joining_allocates_nothing_on_the_spool_thread() {
        let mut helper = SpoolHelper::start_for_test().unwrap();
        let count = AtomicUsize::new(0);
        let job = || {
            count.fetch_add(1, Ordering::Relaxed);
        };
        helper.join(false, &job, || ());
        collector_test_support::with_allocations_denied(|| {
            helper.join(true, &job, || ());
        });
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
