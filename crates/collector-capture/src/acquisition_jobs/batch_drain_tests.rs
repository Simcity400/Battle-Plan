use super::*;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

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

struct Jobs {
    started: [Gate; 2],
    finished: [Mutex<bool>; 2],
    unwinding: Gate,
    failed: Gate,
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
            collector_memory::terminal::fail(
                collector_memory::terminal::Phase::Acquisition,
                collector_memory::terminal::Code::Native,
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
    Pool::with_test_runner(workers, move |job| runner.run(job))
}

fn job(index: usize) -> Job {
    Job::Provinces(ProvincesRequest {
        base: index,
        game_state: 0,
        payload: engine_native::cprovince::Capture::try_with_capacity(2, 0, 0).unwrap(),
    })
}

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

/// # Safety
/// Run only in an isolated fixture process with no collector attached and no concurrent terminal-
/// state readers or writers. This fixture resets and injects the process-wide terminal witness.
pub unsafe fn exercise(case: &str) {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    match case {
        "terminal" => {
            let jobs = Jobs::new([false; 2], None);
            let pool = pool(1, &jobs);
            unwinding_callback(&pool, &jobs, 2, &[0], || {
                collector_memory::terminal::fail(
                    collector_memory::terminal::Phase::Acquisition,
                    collector_memory::terminal::Code::Native,
                );
                assert!(collector_memory::terminal::failed());
            });
            assert!(jobs.finished(0));
            assert!(!jobs.started(1));
            assert_drained(&pool, &jobs);
        }
        "midway" => {
            let jobs = Jobs::new([false, true], Some(0));
            let pool = pool(2, &jobs);
            unwinding_callback(&pool, &jobs, 2, &[0, 1], || {
                assert!(!collector_memory::terminal::failed());
            });
            assert!(collector_memory::terminal::failed());
            assert!(jobs.finished(0) && jobs.finished(1));
            assert_drained(&pool, &jobs);
        }
        _ => panic!("unknown batch drain case {case}"),
    }
    println!("collector-batch-drain-{case}=ok");
}

#[test]
fn an_unwinding_callback_settles_a_contained_stitch_panic_beside_its_siblings() {
    let jobs = Jobs::new([false; 2], None);
    let runner = Arc::clone(&jobs);
    let pool = Pool::with_test_runner(1, move |job: &mut Job| match job {
        Job::OrdersStitch(_) => Report::OrdersStitch(StitchReport {
            contained_panic: true,
        }),
        job => runner.run(job),
    });
    let stitch = || {
        use collector_plans::extended::plans::{Capacities, Captured};
        Job::OrdersStitch(OrdersRequest {
            countries: engine_allocation::shared(
                engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
            )
            .unwrap(),
            base: 0,
            game_state: 0,
            // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
            orders: unsafe {
                collector_map_capture::map_capture::MapCapacities::default().orders()
            },
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
            unsafe { batch.submit(&mut Some(job(index))) }.unwrap();
        }
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
