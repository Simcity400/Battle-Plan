//! Completion-driven dispatch uses gates, never completion-time guesses.
use super::*;
use std::sync::mpsc;

#[test]
fn readiness_dispatches_dependents_before_an_unrelated_fixed_join() {
    struct Job {
        stage: usize,
        loan: Box<[usize; 4]>,
    }
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let (started, running) = mpsc::channel();
    let pool = Pool::start(2, 3, move |job: &mut Job| {
        if job.stage == 0 {
            started.send(()).unwrap();
            released
                .lock()
                .unwrap()
                .recv_timeout(crate::TEST_HANG_GUARD)
                .unwrap();
        }
        if job.stage != 0 {
            job.loan[job.stage - 1] = job.stage;
        }
        job.stage
    })
    .unwrap();
    let submit = |job| unsafe { pool.submit(&mut Some(job)).unwrap() };
    let unrelated = submit(Job {
        stage: 0,
        loan: Box::default(),
    });
    running.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
    let loan = Box::new([0; 4]);
    let address = (&*loan) as *const [usize; 4];
    let mut pending = submit(Job { stage: 1, loan });
    // DivisionInputs -> Divisions -> ArmyIcons -> Late: each consumer is
    // admitted after collecting its producer, not after the first fixed join.
    for stage in 1..=4 {
        pool.wait_ready(|| pool.is_complete(pending).then_some(()));
        assert!(!pool.is_complete(unrelated));
        let completed = pool.finish(pending).unwrap();
        assert_eq!(completed.report, Ok(stage));
        assert_eq!((&*completed.request.loan) as *const [usize; 4], address);
        assert_eq!(
            completed.request.loan[..stage],
            (1..=stage).collect::<Vec<_>>()
        );
        if stage < 4 {
            pending = submit(Job {
                stage: stage + 1,
                loan: completed.request.loan,
            });
        }
    }
    release.send(()).unwrap();
    assert_eq!(pool.finish(unrelated).unwrap().report, Ok(0));
    assert_eq!(pool.calibration().uncollected, 0);
    assert_eq!(pool.calibration().caller_executed, 0);
}

#[test]
fn readiness_cannot_lose_publication_before_registration_or_before_address_wait() {
    for before_registration in [true, false] {
        let (release, released) = mpsc::channel();
        let released = Mutex::new(released);
        let pool = Pool::start(1, 1, move |job: &mut u8| {
            released
                .lock()
                .unwrap()
                .recv_timeout(crate::TEST_HANG_GUARD)
                .unwrap();
            *job
        })
        .unwrap();
        let ticket = unsafe { pool.submit(&mut Some(7)).unwrap() };
        let mut calls = 0;
        pool.wait_ready_with_spin(std::time::Duration::ZERO, || {
            calls += 1;
            if calls == if before_registration { 1 } else { 2 } {
                // The late case publishes after the registered sequence
                // snapshot but before a deliberately stale readiness result.
                release.send(()).unwrap();
                let began = std::time::Instant::now();
                while !pool.is_complete(ticket) {
                    assert!(
                        began.elapsed() < crate::TEST_HANG_GUARD,
                        "producer did not publish"
                    );
                    std::thread::yield_now();
                }
                return None;
            }
            pool.is_complete(ticket).then_some(())
        });
        assert_eq!(calls, if before_registration { 2 } else { 3 });
        assert_eq!(pool.finish(ticket).unwrap().report, Ok(7));
        assert!(!pool.shared.ready_waiting.load(Ordering::SeqCst));
    }
}

#[test]
fn readiness_failure_and_cancellation_keep_partial_requests_and_loans() {
    struct Job {
        value: u8,
        loan: Box<u8>,
    }
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let (started, running) = mpsc::channel();
    let pool = Pool::start(1, 2, move |job: &mut Job| -> u8 {
        started.send(()).unwrap();
        released
            .lock()
            .unwrap()
            .recv_timeout(crate::TEST_HANG_GUARD)
            .unwrap();
        job.value = 9;
        panic!("synthetic producer panic");
    })
    .unwrap();
    let first = Job {
        value: 1,
        loan: Box::new(11),
    };
    let second = Job {
        value: 2,
        loan: Box::new(22),
    };
    let addresses = [(&*first.loan) as *const u8, (&*second.loan) as *const u8];
    let first = unsafe { pool.submit(&mut Some(first)).unwrap() };
    running.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
    let second = unsafe { pool.submit(&mut Some(second)).unwrap() };
    release.send(()).unwrap();
    pool.wait_ready(|| pool.is_complete(first).then_some(()));
    assert_eq!(pool.completed_report(first), Some(Err(Failure::Panicked)));
    assert_eq!(pool.completed_report(first), Some(Err(Failure::Panicked)));
    let first = pool.finish(first).unwrap();
    let second = pool.finish(second).unwrap();
    assert_eq!(first.report, Err(Failure::Panicked));
    assert_eq!(second.report, Err(Failure::Stopped));
    assert_eq!((first.request.value, second.request.value), (9, 2));
    assert_eq!((&*first.request.loan) as *const u8, addresses[0]);
    assert_eq!((&*second.request.loan) as *const u8, addresses[1]);
    assert!(running.try_recv().is_err(), "cancelled producer ran");
    assert_eq!(pool.calibration().uncollected, 0);
    assert_eq!(
        (pool.calibration().panicked, pool.calibration().stopped),
        (1, 1)
    );
}

#[test]
fn readiness_report_inspection_does_not_collect_or_accept_stale_foreign_or_parked_tickets() {
    let pool = Pool::start(1, 1, |job: &mut u8| *job).unwrap();
    let other = Pool::start(1, 1, |job: &mut u8| *job).unwrap();
    let ticket = unsafe { pool.submit(&mut Some(7)).unwrap() };
    pool.wait_ready(|| pool.is_complete(ticket).then_some(()));
    crate::test_support::with_allocations_denied(|| {
        assert_eq!(pool.completed_report(ticket), Some(Ok(7)));
        assert_eq!(pool.completed_report(ticket), Some(Ok(7)));
        assert_eq!(other.completed_report(ticket), None);
    });
    assert_eq!(pool.calibration().uncollected, 1);
    assert_eq!(pool.finish(ticket).unwrap().report, Ok(7));
    assert_eq!(pool.completed_report(ticket), None);
    let parked = pool.park(&mut Some(8), false).unwrap();
    assert_eq!(pool.completed_report(parked), None);
    assert_eq!(pool.completed_report(ticket), None);
    assert_eq!(pool.unpark(parked), Some(8));
}

#[test]
fn readiness_registration_is_removed_on_unwind() {
    let pool = Pool::start(1, 1, |job: &mut u8| *job).unwrap();
    let mut calls = 0;
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            pool.wait_ready_with_spin::<()>(std::time::Duration::ZERO, || {
                calls += 1;
                assert!(calls == 1, "synthetic readiness unwind");
                None
            })
        }))
        .is_err()
    );
    assert!(!pool.shared.ready_waiting.load(Ordering::SeqCst));
    assert_eq!(pool.wait_ready(|| Some(7)), 7);
}

#[test]
fn readiness_spin_observes_publication_without_registering_an_address_wait() {
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let pool = Pool::start(1, 1, move |job: &mut u8| {
        released
            .lock()
            .unwrap()
            .recv_timeout(crate::TEST_HANG_GUARD)
            .unwrap();
        *job
    })
    .unwrap();
    let ticket = unsafe { pool.submit(&mut Some(7)).unwrap() };
    let mut calls = 0;
    pool.wait_ready_with_spin(crate::TEST_HANG_GUARD, || {
        calls += 1;
        assert!(!pool.shared.ready_waiting.load(Ordering::SeqCst));
        if calls == 2 {
            release.send(()).unwrap();
            let until = std::time::Instant::now() + crate::TEST_HANG_GUARD;
            while !pool.is_complete(ticket) {
                assert!(std::time::Instant::now() < until, "producer did not publish");
                std::thread::yield_now();
            }
        }
        pool.is_complete(ticket).then_some(())
    });
    assert_eq!(calls, 2);
    assert_eq!(pool.calibration().spins, 1);
    assert_eq!(pool.calibration().join_sleeps, 0);
    assert_eq!(pool.calibration().uncollected, 1);
    assert_eq!(pool.finish(ticket).unwrap().report, Ok(7));
}

/// Parts may publish in reverse order, but the consumer waits for every
/// producer and the callback keeps serial slot order and its first refusal.
#[test]
fn readiness_multi_producer_barrier_keeps_dispatch_order_and_first_failure() {
    for failed_slots in 0..4 {
        let (release, released) = mpsc::channel();
        let released = Mutex::new(released);
        let (finished, finishes) = mpsc::channel();
        let pool = Pool::start(2, 3, move |slot: &mut usize| {
            if *slot == 0 {
                released
                    .lock()
                    .unwrap()
                    .recv_timeout(crate::TEST_HANG_GUARD)
                    .unwrap();
            }
            finished.send(*slot).unwrap();
            if failed_slots & (1 << *slot) != 0 {
                Err("original table refusal")
            } else {
                Ok(*slot)
            }
        })
        .unwrap();
        pool.reserve_worker_slots(2).unwrap();
        let tickets = [0, 1].map(|slot| unsafe {
            pool.submit_with_priority(&mut Some(slot), 0, true, true)
                .unwrap()
        });
        assert_eq!(finishes.recv_timeout(crate::TEST_HANG_GUARD).unwrap(), 1);
        std::thread::scope(|scope| {
            let (checked, checking) = mpsc::channel();
            scope.spawn(move || {
                checking.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
                release.send(()).unwrap();
            });
            let mut checked = Some(checked);
            pool.wait_ready(|| {
                let complete = tickets.iter().all(|&ticket| pool.is_complete(ticket));
                if let Some(checked) = checked.take() {
                    assert!(!complete, "consumer passed an incomplete producer barrier");
                    checked.send(()).unwrap();
                }
                complete.then_some(())
            });
        });
        assert_eq!(finishes.recv_timeout(crate::TEST_HANG_GUARD).unwrap(), 0);
        let mut order = Vec::new();
        let mut refusal = None;
        for ticket in tickets {
            let completion = pool.finish(ticket).unwrap();
            order.push(completion.request);
            if let Err(reason) = completion.report.unwrap() {
                refusal.get_or_insert((completion.request, reason));
            }
        }
        assert_eq!(order, [0, 1]);
        let serial_first_failure = (0..2).find(|slot| failed_slots & (1 << slot) != 0);
        assert_eq!(refusal.map(|(slot, _)| slot), serial_first_failure);
        let consumer = unsafe { pool.submit(&mut Some(2)).unwrap() };
        pool.wait_ready(|| pool.is_complete(consumer).then_some(()));
        assert_eq!(pool.finish(consumer).unwrap().report, Ok(Ok(2)));
        assert_eq!(pool.calibration().accepted, 3);
        assert_eq!(pool.calibration().uncollected, 0);
    }
}
