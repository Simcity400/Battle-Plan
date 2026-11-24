use super::*;
use collector_test_support::with_allocations_denied;
use std::{cell::Cell, hint::black_box, sync::mpsc, thread::JoinHandle};
use windows_sys::Win32::Foundation::{
    STATUS_ACCESS_DENIED, STATUS_ALERTED, STATUS_INVALID_HANDLE, STATUS_TIMEOUT,
};

struct TestThreads {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl TestThreads {
    fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            threads: Vec::new(),
        }
    }

    fn finish(&mut self) -> bool {
        self.stop.store(true, Ordering::Release);
        let mut healthy = true;
        for thread in self.threads.drain(..) {
            healthy &= thread.join().is_ok();
        }
        healthy
    }
}

impl Drop for TestThreads {
    fn drop(&mut self) {
        self.finish();
    }
}

#[test]
fn worker_sampler_settings_defaults_options_and_refusals() {
    let directory = std::env::temp_dir().join("hoi4-worker-sampler");
    let directory = directory.as_path();
    let defaults = Settings::parse(None, directory).unwrap();
    assert!(defaults.enabled);
    assert_eq!(defaults.interval_us, DEFAULT_INTERVAL_US);
    assert_eq!(defaults.output, directory.join("worker-samples.bin"));
    let configured = Settings::parse(Some(r"interval_us=50"), directory).unwrap();
    assert!(configured.enabled);
    assert_eq!(configured.interval_us, 50);
    assert_eq!(configured.output, directory.join("worker-samples.bin"));
    for option in [
        "interval_us=0",
        "interval_us=-1",
        "interval_us=4294967296",
        "window_kib=32",
        r"out=C:\captures\profile.bin",
        "mystery=1",
    ] {
        let error = Settings::parse(Some(option), directory).unwrap_err();
        assert!(error.contains("worker_sampler"), "{option}: {error}");
    }
    let disabled = Settings::parse(Some("off,interval_us=50"), directory).unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.interval_us, 50);
}

#[test]
fn worker_sampler_resume_retries_publish_before_delay_and_retain_unproved_holds() {
    let statuses = [
        STATUS_TIMEOUT,
        STATUS_ALERTED,
        STATUS_INVALID_HANDLE,
        STATUS_ACCESS_DENIED,
    ];
    for previous_count in [0, 1, 4] {
        let retries = AtomicU64::new(0);
        let attempts = Cell::new(0_usize);
        let waits = Cell::new(0_usize);
        let mut delays = 0;
        let mut visible = true;
        let resumed = resume_until_released(
            &retries,
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() <= statuses.len() {
                    u32::MAX
                } else {
                    previous_count
                }
            },
            || {
                let status = statuses[waits.get()];
                waits.set(waits.get() + 1);
                status
            },
            || {
                visible &= retries.load(Ordering::Relaxed) == attempts.get() as u64;
                delays += 1;
            },
        );
        assert!(resumed && visible);
        assert_eq!(attempts.get(), statuses.len() + 1);
        assert_eq!(waits.get(), statuses.len());
        assert_eq!(delays, statuses.len());
        assert_eq!(retries.load(Ordering::Relaxed), statuses.len() as u64);
    }
}

#[test]
fn worker_sampler_resume_proved_exit_discards_capture_without_delay() {
    let retries = AtomicU64::new(0);
    let mut attempts = 0;
    let mut waits = 0;
    let mut delays = 0;
    let released = resume_until_released(
        &retries,
        || {
            attempts += 1;
            u32::MAX
        },
        || {
            waits += 1;
            STATUS_WAIT_0
        },
        || delays += 1,
    );
    assert!(!released, "terminated target retained a usable capture");
    assert_eq!((attempts, waits, delays), (1, 1, 0));
    assert_eq!(retries.load(Ordering::Relaxed), 1);
}

#[test]
fn worker_sampler_concurrent_join_waits_for_panic_status_publication() {
    let panicked = thread::spawn(|| panic!("synthetic sampler panic"));
    while !panicked.is_finished() {
        thread::yield_now();
    }
    let control = Arc::new(Control {
        on_panic: record_stop_panic,
        stop: Arc::new(AtomicBool::new(false)),
        active: Arc::new(AtomicBool::new(false)),
        thread: Mutex::new(Some(panicked)),
        ready: AtomicBool::new(true),
        join_complete: AtomicBool::new(false),
    });
    let metadata = META.lock().unwrap_or_else(|e| e.into_inner());
    let first = Arc::clone(&control);
    let joiner = thread::spawn(move || first.stop_join());
    while control.thread.lock().unwrap().is_some() {
        thread::yield_now();
    }
    let second = Arc::clone(&control);
    let second_returned = Arc::new(AtomicBool::new(false));
    let returned = Arc::clone(&second_returned);
    let follower_waits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let waits = Arc::clone(&follower_waits);
    let follower = thread::spawn(move || {
        second.stop_join_waiting(|| {
            waits.fetch_add(1, Ordering::Release);
            thread::yield_now();
        });
        returned.store(true, Ordering::Release);
    });
    let guard = std::time::Instant::now() + Duration::from_secs(120);
    while follower_waits.load(Ordering::Acquire) < 2 && !follower.is_finished() {
        if std::time::Instant::now() >= guard {
            drop(metadata);
            panic!("the follower neither waited nor returned under the hang guard");
        }
        thread::yield_now();
    }
    let premature = control.join_complete.load(Ordering::Acquire)
        || follower.is_finished()
        || second_returned.load(Ordering::Acquire);
    drop(metadata);
    let first_joined = joiner.join().is_ok();
    let guard = std::time::Instant::now() + Duration::from_secs(120);
    while !follower.is_finished() {
        if std::time::Instant::now() >= guard {
            control.join_complete.store(true, Ordering::Release);
            let _ = follower.join();
            panic!("the follower never saw the join complete under the hang guard");
        }
        thread::yield_now();
    }
    let second_joined = follower.join().is_ok();
    let joined = first_joined && second_joined;
    assert!(joined);
    assert!(
        !premature,
        "another caller escaped before panic status was stored"
    );
    assert!(control.join_complete.load(Ordering::Acquire));
    assert!(second_returned.load(Ordering::Acquire));
}

#[test]
fn worker_sampler_real_pool_job_attribution_and_guard_reset() {
    let mut scratch = Scratch::default();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let (ready_tx, ready_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_barrier = Arc::clone(&barrier);
    let releaser = thread::spawn(move || {
        if entered_rx.recv().is_ok() {
            let _ = release_rx.recv_timeout(collector_test_support::TEST_HANG_GUARD);
            release_barrier.wait();
        }
    });
    let job_barrier = Arc::clone(&barrier);
    let run = move |_: &mut ()| {
        // SAFETY: The isolated fixture owns this registered worker for the duration of the job.
        let _job = unsafe { job(crate::roles::Role::World) };
        let _ = entered_tx.send(());
        let _ = ready_tx.send(unsafe { GetCurrentThreadId() });
        job_barrier.wait();
        7_u8
    };
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = unsafe { crate::acquisition::Pool::start(1, 1, run) }.unwrap();
    let mut target = pool
        .thread_ids()
        .first()
        .copied()
        .and_then(|id| Target::open(id, 7, 0, 19));
    let mut request = Some(());
    // SAFETY: this job owns no foreign pointers and is collected before the
    // pool/its captured barrier are dropped. There is only one collector.
    let submitted = unsafe { pool.submit(&mut request) };
    let Ok(ticket) = submitted else {
        let shutdown =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { pool.shutdown() };
        drop(pool);
        let _ = release_tx.send(());
        let released = releaser.join();
        assert!(shutdown.is_ok());
        assert!(released.is_ok());
        panic!("synthetic worker job admission failed");
    };
    let id = ready_rx.recv_timeout(collector_test_support::TEST_HANG_GUARD);
    let record = target
        .as_mut()
        .and_then(|target| sample(target, &mut scratch));
    let observed_job = scratch.job;
    let observed_identity = target.as_ref().map(|target| {
        (
            target.role(),
            target.info.slot,
            target.info.worker_index,
            target.info.thread_id,
        )
    });
    let _ = release_tx.send(());
    let completion = pool.finish(ticket);
    let cleared_job = target
        .as_ref()
        .and_then(|target| target.job.as_ref())
        .map(SampledJob::id);
    let shutdown =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() };
    drop(pool);
    let released = releaser.join();
    assert!(id.is_ok(), "worker did not report job entry: {id:?}");
    let record = record.expect("CPU-gated sampling did not record the blocked worker");
    assert_eq!(record.role, 7);
    assert_eq!(record.slot, 19);
    assert_eq!(record.worker, 0);
    assert_eq!(record.job, crate::roles::Role::World as u16 + 1);
    assert!(
        record.cycles > 0,
        "worker dispatch/job did not advance cycles"
    );
    assert_ne!(record.pc, 0, "sample record has no captured instruction");
    assert_eq!(observed_job, crate::roles::Role::World as u16 + 1);
    assert_eq!(observed_identity, Some((7, 19, Some(0), id.unwrap())));
    assert!(completion.is_some());
    assert_eq!(cleared_job, Some(0));
    assert!(shutdown.is_ok());
    assert!(released.is_ok());
}

#[test]
fn worker_sampler_no_deadlock_allocating_mutex_threads_make_progress() {
    const WORKERS: usize = 2;
    let mut running = TestThreads::new();
    let shared = Arc::new(Mutex::new(0_u64));
    let progress = Arc::new([const { AtomicU64::new(0) }; WORKERS]);
    let (ids_tx, ids_rx) = mpsc::channel();
    for index in 0..WORKERS {
        let stop = Arc::clone(&running.stop);
        let shared = Arc::clone(&shared);
        let progress = Arc::clone(&progress);
        let ids_tx = ids_tx.clone();
        running.threads.push(thread::spawn(move || {
            let _ = ids_tx.send((index, unsafe { GetCurrentThreadId() }));
            let mut iteration = 0_usize;
            while !stop.load(Ordering::Acquire) {
                let mut bytes = vec![index as u8; 64 + iteration % 128];
                let mut value = Box::new([0_u64; 16]);
                {
                    let mut lock = shared.lock().unwrap_or_else(|e| e.into_inner());
                    *lock = lock.wrapping_add(1);
                    let under_lock = vec![*lock; 32];
                    bytes[0] = under_lock[0] as u8;
                    value[0] = *lock;
                    black_box(under_lock);
                }
                black_box((bytes, value));
                progress[index].fetch_add(1, Ordering::Release);
                iteration = iteration.wrapping_add(1);
            }
        }));
    }
    drop(ids_tx);
    let mut targets = Vec::new();
    for slot in 0..WORKERS {
        if let Ok((index, id)) = ids_rx.recv_timeout(collector_test_support::TEST_HANG_GUARD)
            && let Some(target) = Target::open(id, 7, index as u8, slot as u16)
        {
            targets.push(target);
        }
    }
    let mut scratch = Scratch::default();
    let begin = Instant::now();
    let mut rounds = 0_u64;
    let mut captures = 0_u64;
    let mut windows = 0_usize;
    while windows < 20 {
        let previous =
            std::array::from_fn::<_, WORKERS, _>(|index| progress[index].load(Ordering::Acquire));
        loop {
            for target in &targets {
                if with_allocations_denied(|| capture_window(target, &mut scratch)) {
                    captures += 1;
                }
                let allocation = vec![black_box(rounds); 128];
                {
                    let mut lock = shared.lock().unwrap_or_else(|e| e.into_inner());
                    *lock = lock.wrapping_add(allocation[0]);
                    black_box(Box::new(*lock));
                }
                black_box(allocation);
            }
            rounds += 1;
            let current = std::array::from_fn::<_, WORKERS, _>(|index| {
                progress[index].load(Ordering::Acquire)
            });
            if (0..WORKERS).all(|index| current[index] > previous[index]) {
                break;
            }
            assert!(
                begin.elapsed() < collector_test_support::TEST_HANG_GUARD,
                "a target stalled in window {windows}: {previous:?} then {current:?}"
            );
            thread::yield_now();
        }
        windows += 1;
    }
    let joined = running.finish();
    assert!(joined, "allocating target thread panicked");
    assert_eq!(targets.len(), WORKERS, "could not open every stress target");
    assert!(rounds >= 20, "sampler completed too few rounds: {rounds}");
    assert!(
        captures >= rounds,
        "too few successful captures: {captures}/{rounds}"
    );
}

struct LocalCallback<'a> {
    gate: &'a GameGate,
    region: &'a AtomicU16,
}

impl<'a> LocalCallback<'a> {
    fn enter(gate: &'a GameGate, region: &'a AtomicU16) -> Self {
        region.store(1, Ordering::SeqCst);
        gate.in_callback.store(true, Ordering::SeqCst);
        Self { gate, region }
    }
}

impl Drop for LocalCallback<'_> {
    fn drop(&mut self) {
        self.region.store(2, Ordering::SeqCst);
        self.gate.leave();
        self.region.store(0, Ordering::SeqCst);
    }
}

struct LocalClaim<'a>(&'a GameGate);

impl<'a> LocalClaim<'a> {
    fn take(gate: &'a GameGate) -> Option<Self> {
        gate.claim().then_some(Self(gate))
    }
}

impl Drop for LocalClaim<'_> {
    fn drop(&mut self) {
        self.0.touching.store(false, Ordering::SeqCst);
    }
}

#[test]
fn worker_sampler_game_handshake_stays_in_callback_or_exit_and_discards_exit_race() {
    let mut running = TestThreads::new();
    let gate = Arc::new(GameGate::new());
    let region = Arc::new(AtomicU16::new(0));
    let leave_first = Arc::new(AtomicBool::new(false));
    let exits = Arc::new(AtomicU64::new(0));
    let (id_tx, id_rx) = mpsc::channel();
    let stop = Arc::clone(&running.stop);
    let worker_gate = Arc::clone(&gate);
    let worker_region = Arc::clone(&region);
    let worker_leave = Arc::clone(&leave_first);
    let worker_exits = Arc::clone(&exits);
    running.threads.push(thread::spawn(move || {
        let _ = id_tx.send(unsafe { GetCurrentThreadId() });
        {
            let _callback = LocalCallback::enter(&worker_gate, &worker_region);
            while !worker_leave.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
        }
        worker_exits.fetch_add(1, Ordering::Release);
        while !stop.load(Ordering::Acquire) {
            {
                let _callback = LocalCallback::enter(&worker_gate, &worker_region);
                for _ in 0..128 {
                    std::hint::spin_loop();
                }
            }
            worker_exits.fetch_add(1, Ordering::Release);
            for _ in 0..32 {
                std::hint::spin_loop();
            }
        }
    }));
    let id = id_rx.recv_timeout(collector_test_support::TEST_HANG_GUARD);
    let mut target = id.ok().and_then(|id| Target::open(id, 7, 0, 0));
    if let Some(target) = &mut target {
        target.job = Some(SampledJob::fixture(Arc::clone(&region)));
    }
    let mut scratch = Scratch::default();
    let guard = Instant::now() + collector_test_support::TEST_HANG_GUARD;
    let waited = |what: &str| assert!(Instant::now() < guard, "{what} within the hang guard");
    let mut forced_claim = None;
    while forced_claim.is_none() {
        forced_claim = LocalClaim::take(&gate);
        waited("the fake game entered no callback to claim");
        thread::yield_now();
    }
    let claimed_first = forced_claim.is_some();
    leave_first.store(true, Ordering::Release);
    while gate.in_callback.load(Ordering::SeqCst) {
        waited("the claimed callback never cleared");
        thread::yield_now();
    }
    let forced_clear = claimed_first && !gate.in_callback.load(Ordering::SeqCst);
    let forced_capture = forced_clear
        && target
            .as_ref()
            .is_some_and(|target| capture_window(target, &mut scratch));
    let forced_region = scratch.job;
    let exits_before_release = exits.load(Ordering::Acquire);
    drop(forced_claim);
    while exits.load(Ordering::Acquire) == 0 {
        waited("the callback did not exit after release");
        thread::yield_now();
    }
    let first_exit_completed = exits.load(Ordering::Acquire) != 0;
    let mut captures = 0;
    let mut outside = 0;
    let mut invalid_region = 0;
    let mut discarded = u64::from(forced_capture);
    if let Some(target) = &target {
        while captures < 256 {
            waited("the sampler did not make 256 handshake captures");
            if let Some(claim) = LocalClaim::take(&gate) {
                if capture_window(target, &mut scratch) {
                    captures += 1;
                    outside += u64::from(scratch.job == 0);
                    invalid_region += u64::from(scratch.job > 2);
                    discarded += u64::from(!gate.in_callback.load(Ordering::SeqCst));
                }
                drop(claim);
            }
            thread::yield_now();
        }
    }
    gate.touching.store(false, Ordering::SeqCst);
    let joined = running.finish();
    assert!(joined);
    assert!(target.is_some(), "could not open fake game thread");
    assert!(claimed_first && forced_clear && forced_capture);
    assert_eq!(
        forced_region, 2,
        "forced race left the callback exit region"
    );
    assert_eq!(
        exits_before_release, 0,
        "callback escaped a held sampler claim"
    );
    assert!(
        first_exit_completed,
        "exit wait did not terminate after release"
    );
    assert!(captures > 0, "no repeated handshake captures");
    assert_eq!(outside, 0, "suspended fake game outside its callback");
    assert_eq!(invalid_region, 0);
    assert!(
        discarded > 0,
        "clear-before-suspend exit race was not exercised"
    );
    assert!(!gate.in_callback.load(Ordering::SeqCst));
    assert!(!gate.touching.load(Ordering::SeqCst));
    assert_eq!(region.load(Ordering::Acquire), 0);
}
