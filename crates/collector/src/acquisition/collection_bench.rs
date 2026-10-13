//! Uses the baseline executor API so this bench can be cherry-picked unchanged.
use super::{MAX_WORKERS, Pool};
use std::{
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[test]
#[ignore = "release micro-bench; run alone with --ignored --nocapture --test-threads=1"]
fn join_collection_against_fifteen_workers() {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(bench)
        .unwrap()
        .join()
        .unwrap();
}

fn spin(duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        std::hint::spin_loop();
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + crate::TEST_HANG_GUARD;
    while !ready() {
        assert!(Instant::now() < until, "collection bench did not progress");
        std::thread::yield_now();
    }
}

fn summarize(label: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    let mean = samples.iter().map(|&ns| u128::from(ns)).sum::<u128>() / samples.len() as u128;
    println!(
        "{label}: mean {mean} ns, p50 {} ns, p95 {} ns, p99 {} ns, max {} ns ({} samples)",
        samples[samples.len() / 2],
        samples[(samples.len() - 1) * 95 / 100],
        samples[(samples.len() - 1) * 99 / 100],
        samples[samples.len() - 1],
        samples.len(),
    );
}

fn bench() {
    const JOB_BYTES: usize = size_of::<crate::acquisition_jobs::Job>();
    const BACKGROUND: usize = MAX_WORKERS * 4;
    const ROUNDS: usize = 500;
    struct Request {
        target: bool,
        bytes: [u8; JOB_BYTES - 1],
    }
    assert_eq!(size_of::<Request>(), JOB_BYTES);
    let target_release = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let recycled = Arc::new(AtomicU64::new(0));
    let (entered_tx, entered_rx) = mpsc::channel();
    let pool = {
        let target_release = Arc::clone(&target_release);
        Arc::new(
            Pool::start(MAX_WORKERS, BACKGROUND + 1, move |job: &mut Request| {
                std::hint::black_box(&mut job.bytes);
                if job.target {
                    entered_tx.send(()).unwrap();
                    wait_until(|| target_release.load(Ordering::Acquire));
                } else {
                    spin(Duration::from_micros(150));
                }
            })
            .unwrap(),
        )
    };
    pool.reserve_worker_slots(BACKGROUND).unwrap();
    let recycler = {
        let pool = Arc::clone(&pool);
        let stop = Arc::clone(&stop);
        let recycled = Arc::clone(&recycled);
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                let mut tickets: Vec<_> = (0..BACKGROUND)
                    .map(|_| {
                        let mut job = Some(Request {
                            target: false,
                            bytes: [0; JOB_BYTES - 1],
                        });
                        unsafe { pool.submit_with_priority(&mut job, 1, true, true) }.unwrap()
                    })
                    .collect();
                while !stop.load(Ordering::Acquire) {
                    for ticket in &mut tickets {
                        if !pool.is_complete(*ticket) {
                            continue;
                        }
                        // Disjoint synthetic payloads, already published only:
                        // this recycler cannot execute a job or register a waiter.
                        // Both executors admit racing collection of ready cells;
                        // the measured thread remains the sole pending waiter.
                        let done = pool.finish(*ticket).unwrap();
                        assert_eq!(done.report, Ok(()));
                        let mut job = Some(done.request);
                        *ticket =
                            unsafe { pool.submit_with_priority(&mut job, 1, true, true) }.unwrap();
                        recycled.fetch_add(1, Ordering::Release);
                    }
                    std::thread::yield_now();
                }
                for ticket in tickets {
                    wait_until(|| pool.is_complete(ticket));
                    assert_eq!(pool.finish(ticket).unwrap().report, Ok(()));
                }
            })
            .unwrap()
    };
    // Warm the continuously recycled queue before either measurement class.
    wait_until(|| recycled.load(Ordering::Acquire) >= BACKGROUND as u64);
    let mut frequency = 0_i64;
    // SAFETY: a valid local output; the session uses the same Windows QPC clock.
    assert_ne!(
        unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
        },
        0
    );
    assert!(frequency > 0);
    println!(
        "{JOB_BYTES}-byte jobs, {MAX_WORKERS} workers, {BACKGROUND} continuously recycled background slots"
    );
    let mut published_wall = Vec::with_capacity(ROUNDS);
    let mut waiting_stall = Vec::with_capacity(ROUNDS);
    let mut waiting_wall = Vec::with_capacity(ROUNDS);
    for already_published in [true, false] {
        for _ in 0..ROUNDS * 4 {
            let samples = if already_published {
                published_wall.len()
            } else {
                waiting_stall.len()
            };
            if samples == ROUNDS {
                break;
            }
            target_release.store(false, Ordering::Release);
            let mut target = Some(Request {
                target: true,
                bytes: [0; JOB_BYTES - 1],
            });
            let ticket = unsafe { pool.submit_with_priority(&mut target, 0, false, true) }.unwrap();
            entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
            if already_published {
                target_release.store(true, Ordering::Release);
                wait_until(|| pool.is_complete(ticket));
            }
            let (begin_tx, begin_rx) = mpsc::channel();
            std::thread::scope(|scope| {
                let target_release = &target_release;
                let release = scope.spawn(move || {
                    begin_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
                    if !already_published {
                        spin(Duration::from_micros(250));
                    }
                    target_release.store(true, Ordering::Release);
                });
                begin_tx.send(()).unwrap();
                let join_qpc = crate::sampling::qpc();
                let begin = Instant::now();
                let completion = pool.finish(ticket).unwrap();
                let wall = begin.elapsed().as_nanos() as u64;
                release.join().unwrap();
                assert_eq!(completion.report, Ok(()));
                if already_published {
                    published_wall.push(wall);
                } else if completion.timeline.published_qpc >= join_qpc {
                    // A preemption between releasing the helper and entering
                    // finish can make this an already-published join. Exclude
                    // it rather than mixing ready collections into wait tails.
                    let ticks = completion
                        .timeline
                        .collected_qpc
                        .saturating_sub(completion.timeline.published_qpc)
                        .max(0) as u128;
                    waiting_stall.push((ticks * 1_000_000_000 / frequency as u128) as u64);
                    waiting_wall.push(wall);
                }
            });
        }
        assert_eq!(
            if already_published {
                published_wall.len()
            } else {
                waiting_stall.len()
            },
            ROUNDS,
            "could not establish the requested join timing under this load"
        );
    }
    stop.store(true, Ordering::Release);
    recycler.join().unwrap();
    println!(
        "{} background jobs recycled during warm-up and measurement",
        recycled.load(Ordering::Acquire)
    );
    let calibration = pool.calibration();
    assert_eq!(calibration.caller_executed, 0);
    assert_eq!(calibration.uncollected, 0);
    summarize("already published: join wall", &mut published_wall);
    summarize(
        "published during join: publication to collection",
        &mut waiting_stall,
    );
    summarize("published during join: join wall", &mut waiting_wall);
    pool.shutdown().unwrap();
}
