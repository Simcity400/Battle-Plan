//! The callback's drainage barrier: a `Batch` dropped by an unwinding
//! callback returns only once every job it accepted has been collected,
//! finished or cancelled before it started, a terminal allocation failure
//! included. Gates, not sleeps, order each job against the unwind.
use super::*;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// A one-way gate a job waits at.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Gate {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }

    fn wait(&self) {
        let open = self.open.lock().unwrap();
        let (open, timeout) = self
            .opened
            .wait_timeout_while(open, Duration::from_secs(30), |open| !*open)
            .unwrap();
        assert!(
            *open && !timeout.timed_out(),
            "a drain fixture gate never opened"
        );
    }
}

/// Per job (its request's `base`): whether it started and finished, the gate
/// it waits at before finishing, and whether it records a terminal
/// allocation failure as it ends (and then opens `failed`).
struct Jobs {
    started: [Gate; 2],
    finished: [Mutex<bool>; 2],
    /// Opened once the callback has begun to unwind: no job can finish
    /// before its ticket is left to the batch's drop.
    unwinding: Gate,
    /// Opened by the job that records the failure, after recording it.
    failed: Gate,
    /// The job waits for `failed` rather than `unwinding`.
    after_failure: [bool; 2],
    fails: Option<usize>,
}

impl Jobs {
    fn new(after_failure: [bool; 2], fails: Option<usize>) -> Arc<Self> {
        Arc::new(Self {
            started: Default::default(),
            finished: Default::default(),
            unwinding: Gate::default(),
            failed: Gate::default(),
            after_failure,
            fails,
        })
    }

    fn run(&self, job: &Job) -> Report {
        let Job::Provinces(request) = job else {
            unreachable!("drain fixtures submit province jobs")
        };
        let index = request.base;
        self.started[index].open();
        self.unwinding.wait();
        if self.after_failure[index] {
            self.failed.wait();
        }
        if self.fails == Some(index) {
            crate::allocation::fail(
                crate::allocation::Phase::Acquisition,
                crate::allocation::Code::Native,
            );
            self.failed.open();
        }
        *self.finished[index].lock().unwrap() = true;
        Report::ShipModels
    }

    fn started(&self, index: usize) -> bool {
        *self.started[index].open.lock().unwrap()
    }

    fn finished(&self, index: usize) -> bool {
        *self.finished[index].lock().unwrap()
    }
}

fn pool(workers: usize, jobs: &Arc<Jobs>) -> Pool {
    let runner = Arc::clone(jobs);
    let executor =
        acquisition::Pool::start(workers, MAX_JOBS, move |job: &mut Job| runner.run(job)).unwrap();
    executor.reserve_worker_slots(WORKER_ONLY_JOBS).unwrap();
    Pool {
        executor: Arc::new(executor),
        calibration: Arc::new(SharedCalibration::default()),
        worker_count: workers,
    }
}

/// A worker-only job no join may run on the collector.
fn job(index: usize) -> Job {
    Job::Provinces(ProvincesRequest {
        base: index,
        game_state: 0,
        payload: engine_native::cprovince::Capture::try_with_capacity(2, 0, 0).unwrap(),
    })
}

/// A callback that submits `count` jobs, waits for each of `running` to
/// start, lets `before_unwind` act, then panics. Its gate guard is declared
/// after the batch, so the unwind drops it first: the jobs are released only
/// once the unwind has begun and their tickets are left to the batch's drop.
fn unwinding_callback(
    pool: &Pool,
    jobs: &Jobs,
    count: usize,
    running: &[usize],
    before_unwind: impl FnOnce(),
) {
    struct OpenOnUnwind<'a>(&'a Gate);
    impl Drop for OpenOnUnwind<'_> {
        fn drop(&mut self) {
            self.0.open();
        }
    }
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        let mut batch = Batch::new(pool);
        for index in 0..count {
            // SAFETY: the jobs read no memory; the batch drains them.
            unsafe { batch.submit(&mut Some(job(index))) }.unwrap();
        }
        for &index in running {
            jobs.started[index].wait();
        }
        before_unwind();
        let _gate = OpenOnUnwind(&jobs.unwinding);
        panic!("injected callback fault");
    }));
    assert!(unwound.is_err());
}

/// After the unwinding callback returned: every job that started has
/// finished, and the pool holds no running, queued or uncollected job.
fn assert_drained(pool: &Pool, jobs: &Jobs) {
    let executor = pool.calibration().executor;
    assert_eq!(
        (executor.uncollected, executor.active, executor.queued),
        (0, 0, 0),
        "the callback returned before its batch was drained"
    );
    for index in 0..2 {
        assert_eq!(
            jobs.started(index),
            jobs.finished(index),
            "job {index} still running after the callback returned"
        );
    }
}

/// The cases run in a child process (`allocation_failure_tests`): the
/// terminal allocation state is process-wide.
pub(crate) fn exercise(case: &str) {
    crate::allocation::reset();
    crate::allocation::install_hooks();
    match case {
        // A terminal failure recorded before the callback unwinds, with a
        // job running and another queued behind it on the one worker.
        "terminal" => {
            let jobs = Jobs::new([false; 2], None);
            let pool = pool(1, &jobs);
            unwinding_callback(&pool, &jobs, 2, &[0], || {
                crate::allocation::fail(
                    crate::allocation::Phase::Acquisition,
                    crate::allocation::Code::Native,
                );
                assert!(crate::allocation::failed());
            });
            assert!(jobs.finished(0));
            // Claimed after the failure, the queued job never started.
            assert!(!jobs.started(1));
            assert_drained(&pool, &jobs);
        }
        // A terminal failure the first job records while the drain waits
        // for it, with the second running on another worker until then.
        "midway" => {
            let jobs = Jobs::new([false, true], Some(0));
            let pool = pool(2, &jobs);
            unwinding_callback(&pool, &jobs, 2, &[0, 1], || {
                assert!(!crate::allocation::failed());
            });
            assert!(crate::allocation::failed());
            assert!(jobs.finished(0) && jobs.finished(1));
            assert_drained(&pool, &jobs);
        }
        _ => panic!("unknown batch drain case {case}"),
    }
    println!("collector-batch-drain-{case}=ok");
}

/// A stitch whose body panicked publishes a contained report and leaves the
/// pool running ([`StitchReport`]); its join settles that panic. A callback
/// that unwinds before the join, with a sibling job running and another
/// queued behind it on the one worker, leaves the stitch to the batch's
/// drop: the drain counts its panic exactly once, against the executor and
/// the stitch's role, and stops the pool, and still collects every sibling
/// (the running one finishes, the queued one runs or is cancelled before it
/// starts) before the callback returns.
#[test]
fn an_unwinding_callback_settles_a_contained_stitch_panic_beside_its_siblings() {
    let jobs = Jobs::new([false; 2], None);
    let runner = Arc::clone(&jobs);
    let executor = acquisition::Pool::start(1, MAX_JOBS, move |job: &mut Job| match job {
        Job::OrdersStitch(_) => Report::OrdersStitch(StitchReport {
            contained_panic: true,
        }),
        job => runner.run(job),
    })
    .unwrap();
    executor.reserve_worker_slots(WORKER_ONLY_JOBS).unwrap();
    let pool = Pool {
        executor: Arc::new(executor),
        calibration: Arc::new(SharedCalibration::default()),
        worker_count: 1,
    };
    let stitch = || {
        use crate::extended::plans::{Capacities, Captured};
        Job::OrdersStitch(OrdersRequest {
            countries: Arc::new(
                engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
            ),
            base: 0,
            game_state: 0,
            orders: crate::map_capture::Orders::for_test(&Default::default()),
            plans: Captured::with_capacities(&Capacities::default()),
            group_source: None,
        })
    };
    struct OpenOnUnwind<'a>(&'a Gate);
    impl Drop for OpenOnUnwind<'_> {
        fn drop(&mut self) {
            self.0.open();
        }
    }
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        let mut batch = Batch::new(&pool);
        // SAFETY: the jobs read no memory; the batch drains them.
        let ticket = unsafe { batch.submit(&mut Some(stitch())) }.unwrap();
        let started = std::time::Instant::now();
        while !batch.is_complete(ticket) {
            assert!(started.elapsed() < Duration::from_secs(30));
            std::thread::yield_now();
        }
        assert!(
            !batch.stopped(),
            "a contained panic leaves the pool running"
        );
        for index in 0..2 {
            // SAFETY: as above.
            unsafe { batch.submit(&mut Some(job(index))) }.unwrap();
        }
        // The first sibling runs on the one worker; the second waits behind it.
        jobs.started[0].wait();
        let _gate = OpenOnUnwind(&jobs.unwinding);
        panic!("injected callback fault");
    }));
    assert!(unwound.is_err());
    let calibration = pool.calibration();
    assert_eq!(calibration.executor.panicked, 1);
    assert_eq!(
        calibration.roles[Role::OrdersStitch as usize]
            .counters
            .panicked,
        1
    );
    assert!(pool.stopped());
    assert!(jobs.finished(0));
    assert_drained(&pool, &jobs);
}
