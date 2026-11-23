use super::*;
use std::sync::mpsc;

const HOLD: u8 = u8::MAX;
struct Held(Arc<AtomicBool>);
impl Drop for Held {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn held_pool(
    capacity: usize,
    eligible: fn(&u8) -> bool,
    run: impl Fn(u8) -> u8 + Send + Sync + 'static,
) -> (Pool<u8, u8>, Held, Ticket) {
    let release = Arc::new(AtomicBool::new(false));
    let held = Arc::clone(&release);
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = unsafe {
        Pool::start(1, capacity + 1, move |job: &mut u8| {
            if *job == HOLD {
                while !held.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                return HOLD;
            }
            run(*job)
        })
    }
    .unwrap();
    pool.reserve_worker_slots(capacity + 1).unwrap();
    pool.set_callback_help(eligible);
    let hold = unsafe { pool.submit_with_priority(&mut Some(HOLD), 0, true, true) }.unwrap();
    let until = std::time::Instant::now() + collector_test_support::TEST_HANG_GUARD;
    while pool.calibration().active == 0 {
        assert!(
            std::time::Instant::now() < until,
            "worker never took the held job"
        );
        std::thread::yield_now();
    }
    (pool, Held(release), hold)
}

#[test]
fn callback_help_preserves_eligible_priority_fifo_and_collection_ownership() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&order);
    let (pool, held, hold) = held_pool(
        4,
        |job| *job != HOLD && *job != 0,
        move |job| {
            seen.lock().unwrap().push(job);
            job
        },
    );
    let tickets: Vec<_> = [(0, 0), (2, 2), (3, 1), (4, 1)]
        .into_iter()
        .map(|(job, priority)| unsafe {
            pool.submit_with_priority(&mut Some(job), priority, true, false)
                .unwrap()
        })
        .collect();
    assert_eq!(
        pool.park_until_ready(|| pool.completed_report(tickets[1])),
        Ok(2)
    );
    assert_eq!(*order.lock().unwrap(), [3, 4, 2]);
    assert_eq!(pool.calibration().uncollected, 5);
    for (index, expected) in [(1, 2), (2, 3), (3, 4)] {
        assert_eq!(pool.finish(tickets[index]).unwrap().report, Ok(expected));
    }
    assert!(!pool.help_callback());
    pool.shared.stop();
    assert_eq!(
        pool.finish(tickets[0]).unwrap().report,
        Err(Failure::Stopped)
    );
    drop(held);
    assert_eq!(pool.finish(hold).unwrap().report, Ok(HOLD));
    assert_eq!(pool.calibration().callback_help_executed, 3);
    assert_eq!(pool.calibration().uncollected, 0);
}

#[test]
fn callback_help_never_runs_rejected_work_or_runs_before_readiness_is_checked() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let (pool, held, hold) = held_pool(
        1,
        |job| *job != HOLD,
        move |job| {
            seen.fetch_add(1, Ordering::Relaxed);
            job
        },
    );
    let ticket = unsafe { pool.submit_with_priority(&mut Some(7), 0, true, false) }.unwrap();
    let mut rejected = Some(9);
    assert_eq!(
        unsafe { pool.submit_with_priority(&mut rejected, 0, true, false) },
        Err(SubmitError::Full)
    );
    assert_eq!(pool.park_until_ready(|| Some(11)), 11);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        pool.park_until_ready(|| pool.completed_report(ticket)),
        Ok(7)
    );
    assert_eq!(pool.finish(ticket).unwrap().report, Ok(7));
    assert_eq!(rejected, Some(9));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    drop(held);
    assert_eq!(pool.finish(hold).unwrap().report, Ok(HOLD));
}

#[test]
fn callback_help_contains_panics_and_preserves_the_cancelled_completion() {
    let (pool, held, hold) = held_pool(
        2,
        |job| *job != HOLD,
        |job| {
            assert_ne!(job, 1, "synthetic helped panic");
            job
        },
    );
    let first = unsafe { pool.submit_with_priority(&mut Some(1), 0, true, false) }.unwrap();
    let second = unsafe { pool.submit_with_priority(&mut Some(2), 0, true, false) }.unwrap();
    assert_eq!(
        pool.park_until_ready(|| pool.completed_report(first)),
        Err(Failure::Panicked)
    );
    assert_eq!(pool.finish(first).unwrap().report, Err(Failure::Panicked));
    assert_eq!(pool.finish(second).unwrap().report, Err(Failure::Stopped));
    drop(held);
    assert_eq!(pool.finish(hold).unwrap().report, Ok(HOLD));
    pool.settle();
    assert_eq!(pool.calibration().callback_help_executed, 1);
    assert_eq!(pool.calibration().active, 0);
    assert_eq!(pool.calibration().uncollected, 0);
}

#[test]
fn callback_join_can_help_its_admitted_worker_only_target() {
    let (pool, held, hold) = held_pool(1, |job| *job != HOLD, |job| job);
    let ticket = unsafe { pool.submit_with_priority(&mut Some(7), 0, true, false) }.unwrap();
    let mut completed = None;
    assert!(pool.finish_into_timed(ticket, &mut completed));
    assert_eq!(completed.unwrap().report, Ok(7));
    assert_eq!(pool.calibration().callback_help_executed, 1);
    drop(held);
    assert_eq!(pool.finish(hold).unwrap().report, Ok(HOLD));
    assert_eq!(pool.calibration().uncollected, 0);
}

#[test]
fn publication_notification_keeps_no_cell_borrow_and_settle_waits_for_it() {
    collector_memory::terminal::set_context(collector_memory::terminal::Context {
        branch: Some(17),
        ..Default::default()
    });
    struct Work(Option<Arc<Notification>>);
    struct Notification {
        pool: std::sync::Weak<Pool<Work, u8>>,
        ticket: Mutex<Option<Ticket>>,
        registered: Condvar,
        entered: mpsc::Sender<()>,
        released: Mutex<mpsc::Receiver<()>>,
    }
    impl Publication for Notification {
        fn published(&self) {
            let pool = self.pool.upgrade().unwrap();
            let (ticket, timeout) = self
                .registered
                .wait_timeout_while(
                    self.ticket.lock().unwrap(),
                    collector_test_support::TEST_HANG_GUARD,
                    |ticket| ticket.is_none(),
                )
                .unwrap();
            assert!(!timeout.timed_out());
            assert_eq!(pool.completed_report(ticket.unwrap()), Some(Ok(7)));
            self.entered.send(()).unwrap();
            self.released
                .lock()
                .unwrap()
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
        }
    }
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = Arc::new(unsafe { Pool::start(1, 1, |_: &mut Work| 7) }.unwrap());
    pool.set_on_completed(|work, _| {
        work.0
            .as_ref()
            .map(|notification| Arc::clone(notification) as Arc<dyn Publication>)
    });
    let (entered, ready) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let notification = Arc::new(Notification {
        pool: Arc::downgrade(&pool),
        ticket: Mutex::new(None),
        entered,
        registered: Condvar::new(),
        released: Mutex::new(released),
    });
    let ticket = unsafe {
        pool.submit_with_priority(
            &mut Some(Work(Some(Arc::clone(&notification)))),
            0,
            false,
            false,
        )
        .unwrap()
    };
    *notification.ticket.lock().unwrap() = Some(ticket);
    notification.registered.notify_all();
    pool.wake_workers(1);
    ready
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
    assert_eq!(pool.finish(ticket).unwrap().report, Ok(7));
    collector_memory::terminal::set_context(collector_memory::terminal::Context {
        branch: Some(23),
        ..Default::default()
    });
    let next = unsafe { pool.submit(&mut Some(Work(None))).unwrap() };
    assert_eq!(unsafe { (*pool.shared.context.0.get()).branch }, Some(17));
    assert_eq!(pool.calibration().active, 1);
    let settling = Arc::clone(&pool);
    let settled = std::thread::spawn(move || settling.settle());
    let until = std::time::Instant::now() + collector_test_support::TEST_HANG_GUARD;
    while pool.shared.settling.load(Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < until, "settle did not register");
        std::thread::yield_now();
    }
    assert!(!settled.is_finished());
    release.send(()).unwrap();
    settled.join().unwrap();
    assert_eq!(pool.finish(next).unwrap().report, Err(Failure::Stopped));
    let last = unsafe { pool.submit(&mut Some(Work(None))).unwrap() };
    assert_eq!(pool.finish(last).unwrap().report, Ok(7));
    assert_eq!(unsafe { (*pool.shared.context.0.get()).branch }, Some(23));
    pool.settle();
    assert_eq!(pool.calibration().active, 0);
    assert_eq!(pool.calibration().uncollected, 0);
}

#[test]
fn publication_transfer_panic_still_publishes_and_drains_the_failed_job() {
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = unsafe { Pool::start(1, 1, |_: &mut u8| 7) }.unwrap();
    pool.set_on_completed(|_, _| panic!("synthetic publication transfer failure"));
    let ticket = unsafe { pool.submit(&mut Some(1)).unwrap() };
    assert_eq!(pool.finish(ticket).unwrap().report, Err(Failure::Panicked));
    pool.settle();
    assert!(pool.stopped());
    assert_eq!(pool.calibration().uncollected, 0);
}

#[test]
fn publication_notification_panic_still_retires_its_active_lease() {
    struct Notify;
    impl Publication for Notify {
        fn published(&self) {
            panic!("synthetic continuation failure")
        }
    }
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = unsafe { Pool::start(1, 1, |_: &mut u8| 7) }.unwrap();
    pool.set_on_completed(|_, _| Some(Arc::new(Notify)));
    let ticket = unsafe { pool.submit(&mut Some(1)).unwrap() };
    assert_eq!(pool.finish(ticket).unwrap().report, Ok(7));
    pool.settle();
    assert!(pool.stopped());
    assert_eq!(pool.calibration().active, 0);
    assert_eq!(pool.calibration().uncollected, 0);
    assert_eq!(pool.calibration().panicked, 1);
}

#[test]
fn readiness_dispatches_dependents_before_an_unrelated_fixed_join() {
    struct Job {
        stage: usize,
        loan: Box<[usize; 4]>,
    }
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let (started, running) = mpsc::channel();
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = unsafe {
        Pool::start(2, 3, move |job: &mut Job| {
            if job.stage == 0 {
                started.send(()).unwrap();
                released
                    .lock()
                    .unwrap()
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
            }
            if job.stage != 0 {
                job.loan[job.stage - 1] = job.stage;
            }
            job.stage
        })
    }
    .unwrap();
    let submit = |job| unsafe { pool.submit(&mut Some(job)).unwrap() };
    let unrelated = submit(Job {
        stage: 0,
        loan: Box::default(),
    });
    running
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
    let loan = Box::new([0; 4]);
    let address = (&*loan) as *const [usize; 4];
    let mut pending = submit(Job { stage: 1, loan });
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
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(1, 1, move |job: &mut u8| {
                released
                    .lock()
                    .unwrap()
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                *job
            })
        }
        .unwrap();
        let ticket = unsafe { pool.submit(&mut Some(7)).unwrap() };
        let mut calls = 0;
        pool.park_until_ready(|| {
            calls += 1;
            if calls == if before_registration { 1 } else { 2 } {
                release.send(()).unwrap();
                let began = std::time::Instant::now();
                while !pool.is_complete(ticket) {
                    assert!(
                        began.elapsed() < collector_test_support::TEST_HANG_GUARD,
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
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = unsafe {
        Pool::start(1, 2, move |job: &mut Job| -> u8 {
            started.send(()).unwrap();
            released
                .lock()
                .unwrap()
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
            job.value = 9;
            panic!("synthetic producer panic");
        })
    }
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
    running
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
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
    // SAFETY: Test owns these local acquisition pools and drops them before test exit.
    let pool = unsafe { Pool::start(1, 1, |job: &mut u8| *job) }.unwrap();
    let other = unsafe { Pool::start(1, 1, |job: &mut u8| *job) }.unwrap();
    let ticket = unsafe { pool.submit(&mut Some(7)).unwrap() };
    pool.wait_ready(|| pool.is_complete(ticket).then_some(()));
    collector_test_support::with_allocations_denied(|| {
        assert_eq!(pool.completed_report(ticket), Some(Ok(7)));
        assert_eq!(pool.completed_report(ticket), Some(Ok(7)));
        assert_eq!(other.completed_report(ticket), None);
    });
    assert_eq!(pool.calibration().uncollected, 1);
    let completion = pool.finish(ticket).unwrap();
    assert_eq!(completion.report, Ok(7));
    assert_eq!(pool.completed_report(ticket), None);
    let parked = pool.park(&mut Some(8), false).unwrap();
    assert_eq!(pool.completed_report(parked), None);
    assert_eq!(pool.completed_report(ticket), None);
    assert_eq!(pool.unpark(parked), Some(8));
}

#[test]
fn readiness_registration_is_removed_on_unwind() {
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = unsafe { Pool::start(1, 1, |job: &mut u8| *job) }.unwrap();
    let mut calls = 0;
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            pool.park_until_ready::<()>(|| {
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
fn readiness_multi_producer_barrier_keeps_dispatch_order_and_first_failure() {
    for failed_slots in 0..4 {
        let (release, released) = mpsc::channel();
        let released = Mutex::new(released);
        let (finished, finishes) = mpsc::channel();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(2, 3, move |slot: &mut usize| {
                if *slot == 0 {
                    released
                        .lock()
                        .unwrap()
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                        .unwrap();
                }
                finished.send(*slot).unwrap();
                if failed_slots & (1 << *slot) != 0 {
                    Err("original table refusal")
                } else {
                    Ok(*slot)
                }
            })
        }
        .unwrap();
        pool.reserve_worker_slots(2).unwrap();
        let tickets = [0, 1].map(|slot| unsafe {
            pool.submit_with_priority(&mut Some(slot), 0, true, true)
                .unwrap()
        });
        assert_eq!(
            finishes
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap(),
            1
        );
        std::thread::scope(|scope| {
            let (checked, checking) = mpsc::channel();
            scope.spawn(move || {
                checking
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
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
        assert_eq!(
            finishes
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap(),
            0
        );
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

#[test]
fn a_detached_handoff_notification_between_readiness_recheck_and_park_is_not_lost() {
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let pool = unsafe { Pool::start(1, 2, |_: &mut u8| 0_u8) }.unwrap();
    let mut polls = 0;
    let result = pool.park_until_ready(|| {
        polls += 1;
        match polls {
            1 => None,
            2 => {
                pool.notify_ready();
                None
            }
            _ => Some(7),
        }
    });
    assert_eq!(result, 7);
    assert_eq!(polls, 3);
    assert_eq!(pool.shared.counters.join_sleeps.load(Ordering::Relaxed), 1);
    assert!(!pool.shared.ready_waiting.load(Ordering::Relaxed));
}
