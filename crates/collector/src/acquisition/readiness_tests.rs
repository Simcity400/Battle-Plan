//! Completion-driven dispatch uses gates, never completion-time guesses.
use super::*;
use std::sync::mpsc;

/// The job that holds the pool's only worker.
const HOLD: u8 = u8::MAX;
/// Releases the held worker when dropped.
struct Held(Arc<AtomicBool>);
impl Drop for Held {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// As in production: every slot reserved for workers, and the pool's one worker held
/// on a job the callback may not help, so admitted work waits for the callback.
fn held_pool(
    capacity: usize,
    eligible: fn(&u8) -> bool,
    run: impl Fn(u8) -> u8 + Send + Sync + 'static,
) -> (Pool<u8, u8>, Held, Ticket) {
    let release = Arc::new(AtomicBool::new(false));
    let held = Arc::clone(&release);
    let pool = Pool::start(1, capacity + 1, move |job: &mut u8| {
        if *job == HOLD {
            while !held.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            return HOLD;
        }
        run(*job)
    })
    .unwrap();
    pool.reserve_worker_slots(capacity + 1).unwrap();
    pool.set_callback_help(eligible);
    let hold = unsafe { pool.submit_with_priority(&mut Some(HOLD), 0, true, true) }.unwrap();
    let until = std::time::Instant::now() + crate::TEST_HANG_GUARD;
    while pool.calibration().active == 0 {
        assert!(std::time::Instant::now() < until, "worker never took the held job");
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
fn outstanding_scan_rejects_replacement_identity_before_its_generation_is_published() {
    use crate::{
        acquisition_diagnostics::{Identity, Outstanding, Slot},
        acquisition_jobs::Role,
    };
    for accepted in [false, true] {
        let slot = Slot::default();
        let old = Identity {
            role: Role::Variants as usize,
            part: Some(1),
            kind: 1,
            ..Default::default()
        };
        let replacement = Identity {
            role: Role::GatedProjects as usize,
            part: Some(2),
            ..Default::default()
        };
        // SAFETY: this synthetic slot has no concurrent owners.
        unsafe {
            slot.admit(old, true);
        }
        let value = AtomicU64::new(word(7, RUNNING));
        let word_before = value.load(Ordering::Acquire);
        slot.publish(100);
        value.store(word(7, PUBLISHED), Ordering::Release);
        value.store(word(7, COLLECTING), Ordering::Release);
        // SAFETY: this test owns the synthetic COLLECTING cell.
        unsafe {
            slot.collected(Ticket::diagnostic(1, 0, 7), Timeline::default());
        }
        value.store(word(7, VACANT), Ordering::Release);
        // SAFETY: this test owns VACANT; new metadata precedes the next PENDING release.
        unsafe {
            slot.admit(replacement, accepted);
        }
        assert_eq!(slot.identity(), replacement);
        assert_eq!(value.load(Ordering::Acquire) >> 3, word_before >> 3);
        let mut outstanding = Outstanding {
            pool: 1,
            ..Default::default()
        };
        crate::test_support::with_allocations_denied(|| {
            record_outstanding_slot(&mut outstanding, 0, word_before, &value, &slot);
        });
        assert_eq!(outstanding.changed_slots, 1);
        assert_eq!(outstanding.generations[0], 0);
        assert_eq!(outstanding.identities[0], 0);
        value.store(word(8, PENDING), Ordering::Release);
        record_outstanding_slot(&mut outstanding, 0, word_before, &value, &slot);
        assert_eq!(outstanding.changed_slots, 2);
        assert_eq!(outstanding.generations[0], 0);
        record_outstanding_slot(&mut outstanding, 0, word(8, PENDING), &value, &slot);
        assert_eq!(outstanding.changed_slots, 2);
        assert_eq!(outstanding.generations[0], if accepted { 8 } else { 0 });
    }
}

#[test]
fn outstanding_scan_rejects_phase_changes_and_keeps_only_stable_accepted_slots() {
    use crate::acquisition_diagnostics::{Identity, Outstanding, Slot};
    let slot = Slot::default();
    // SAFETY: this synthetic slot is exclusively owned here.
    unsafe {
        slot.admit(Identity::default(), true);
    }
    for phase in [PENDING, RUNNING] {
        for after in [VACANT, PENDING, RUNNING, PUBLISHED, COLLECTING] {
            let value = AtomicU64::new(word(7, after));
            let mut outstanding = Outstanding::default();
            record_outstanding_slot(&mut outstanding, 0, word(7, phase), &value, &slot);
            assert_eq!(outstanding.changed_slots, u64::from(after != phase));
            assert_eq!(
                outstanding.generations[0],
                if after == phase { 7 } else { 0 }
            );
        }
    }
}

#[test]
fn publication_notification_keeps_no_cell_borrow_and_settle_waits_for_it() {
    crate::allocation::set_context(crate::allocation::Context {
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
        fn published(&self, at: i64) {
            assert!(at > 0);
            let pool = self.pool.upgrade().unwrap();
            let (ticket, timeout) = self
                .registered
                .wait_timeout_while(
                    self.ticket.lock().unwrap(),
                    crate::TEST_HANG_GUARD,
                    |ticket| ticket.is_none(),
                )
                .unwrap();
            assert!(!timeout.timed_out());
            assert_eq!(pool.completed_report(ticket.unwrap()), Some(Ok(7)));
            self.entered.send(()).unwrap();
            self.released
                .lock()
                .unwrap()
                .recv_timeout(crate::TEST_HANG_GUARD)
                .unwrap();
        }
    }
    let pool = Arc::new(Pool::start(1, 1, |_: &mut Work| 7).unwrap());
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
    // Install the ticket before the worker is released.
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
    ready.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
    assert_eq!(pool.finish(ticket).unwrap().report, Ok(7));
    // The publisher is still in its detached notification. Reusing its cell
    // must neither alias its old request nor end its active/context lease.
    crate::allocation::set_context(crate::allocation::Context {
        branch: Some(23),
        ..Default::default()
    });
    let next = unsafe { pool.submit(&mut Some(Work(None))).unwrap() };
    // SAFETY: admission must preserve this immutable context while active.
    assert_eq!(unsafe { (*pool.shared.context.0.get()).branch }, Some(17));
    assert_eq!(pool.calibration().active, 1);
    let settling = Arc::clone(&pool);
    let settled = std::thread::spawn(move || settling.settle());
    let until = std::time::Instant::now() + crate::TEST_HANG_GUARD;
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
    // SAFETY: the last publication ended its context borrow; no admission races.
    assert_eq!(unsafe { (*pool.shared.context.0.get()).branch }, Some(23));
    pool.settle();
    assert_eq!(pool.calibration().active, 0);
    assert_eq!(pool.calibration().uncollected, 0);
}

#[test]
fn publication_transfer_panic_still_publishes_and_drains_the_failed_job() {
    let pool = Pool::start(1, 1, |_: &mut u8| 7).unwrap();
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
        fn published(&self, _: i64) {
            panic!("synthetic continuation failure")
        }
    }
    let pool = Pool::start(1, 1, |_: &mut u8| 7).unwrap();
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
        pool.park_until_ready(|| {
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
    let (_, published) = pool.completed_state(ticket).unwrap();
    crate::test_support::with_allocations_denied(|| {
        assert_eq!(pool.completed_report(ticket), Some(Ok(7)));
        assert_eq!(pool.completed_report(ticket), Some(Ok(7)));
        assert_eq!(pool.completed_state(ticket), Some((Ok(7), published)));
        assert_eq!(other.completed_report(ticket), None);
        assert_eq!(other.completed_state(ticket), None);
    });
    assert_eq!(pool.calibration().uncollected, 1);
    let completion = pool.finish(ticket).unwrap();
    assert_eq!(completion.report, Ok(7));
    assert_eq!(completion.timeline.published_qpc, published);
    assert_eq!(pool.completed_report(ticket), None);
    assert_eq!(pool.completed_state(ticket), None);
    let parked = pool.park(&mut Some(8), false).unwrap();
    assert_eq!(pool.completed_report(parked), None);
    assert_eq!(pool.completed_state(parked), None);
    assert_eq!(pool.completed_report(ticket), None);
    assert_eq!(pool.unpark(parked), Some(8));
}

#[test]
fn readiness_registration_is_removed_on_unwind() {
    let pool = Pool::start(1, 1, |job: &mut u8| *job).unwrap();
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
fn callback_readiness_park_records_its_wait_without_worker_waits() {
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
    let before = callback_parks::snapshot()[callback_parks::Site::Readiness as usize].calls;
    let releaser = std::thread::spawn(move || {
        let until = std::time::Instant::now() + crate::TEST_HANG_GUARD;
        while callback_parks::snapshot()[callback_parks::Site::Readiness as usize].calls == before {
            assert!(
                std::time::Instant::now() < until,
                "callback did not reach its wait"
            );
            std::thread::yield_now();
        }
        release.send(()).unwrap();
    });
    {
        let _callback = callback_parks::Scope::enter();
        pool.park_until_ready(|| {
            pool.is_complete(ticket).then_some(())
        });
    }
    releaser.join().unwrap();
    let after = callback_parks::snapshot();
    assert!(after[callback_parks::Site::Readiness as usize].calls > before);
    assert!(
        after[callback_parks::Site::Readiness as usize].ns
            >= after[callback_parks::Site::Readiness as usize].max_ns
    );
    assert_eq!(pool.finish(ticket).unwrap().report, Ok(7));
    assert_eq!(pool.calibration().uncollected, 0);
    assert_eq!(pool.calibration().caller_executed, 0);
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

#[test]
fn a_detached_handoff_notification_between_readiness_recheck_and_park_is_not_lost() {
    let pool = Pool::start(1, 2, |_: &mut u8| 0_u8).unwrap();
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

struct PausedPublication {
    entered: mpsc::Sender<()>,
    released: Mutex<mpsc::Receiver<()>>,
}
impl Publication for PausedPublication {
    fn published(&self, _: i64) {
        self.entered.send(()).unwrap();
        receive(&self.released.lock().unwrap());
    }
}
struct WakeJob {
    value: u8,
    publication: Option<Arc<PausedPublication>>,
}
struct WakeFixture {
    pool: Pool<WakeJob, u8>,
    ticket: Ticket,
    readiness: Mutex<mpsc::Receiver<idle_diagnostics::Signal>>,
    resume_wake: mpsc::Sender<()>,
    notification: Mutex<mpsc::Receiver<()>>,
    resume_notification: mpsc::Sender<()>,
    next_started: Mutex<mpsc::Receiver<()>>,
    resume_next: mpsc::Sender<()>,
}
fn receive<T>(receiver: &mpsc::Receiver<T>) -> T {
    receiver.recv_timeout(crate::TEST_HANG_GUARD).unwrap()
}
impl WakeFixture {
    fn new() -> Self {
        let (run, running) = mpsc::channel();
        let running = Mutex::new(running);
        let (resume_next, next) = mpsc::channel();
        let next = Mutex::new(next);
        let (started, next_started) = mpsc::channel();
        let pool = Pool::start(2, 1, move |job: &mut WakeJob| {
            if job.value == 1 {
                receive(&running.lock().unwrap());
            } else {
                started.send(()).unwrap();
                receive(&next.lock().unwrap());
            }
            job.value
        })
        .unwrap();
        pool.set_on_completed(|job, _| {
            job.publication
                .as_ref()
                .map(|publication| Arc::clone(publication) as Arc<dyn Publication>)
        });
        let once = std::sync::Once::new();
        pool.set_readiness_wait_hook(move || once.call_once(|| run.send(()).unwrap()));
        let (woke, readiness) = mpsc::channel();
        let (resume_wake, paused) = mpsc::channel();
        let paused = Mutex::new(paused);
        assert!(
            pool.shared
                .hooks
                .after_readiness_wake
                .set(Box::new(move |signal| {
                    woke.send(signal).unwrap();
                    receive(&paused.lock().unwrap());
                }))
                .is_ok()
        );
        let (entered, notification) = mpsc::channel();
        let (resume_notification, released) = mpsc::channel();
        let publication = Arc::new(PausedPublication {
            entered,
            released: Mutex::new(released),
        });
        // SAFETY: the fixture retains every job and notification through settlement.
        let ticket = unsafe {
            pool.submit(&mut Some(WakeJob {
                value: 1,
                publication: Some(publication),
            }))
        }
        .unwrap();
        Self {
            pool,
            ticket,
            readiness: Mutex::new(readiness),
            resume_wake,
            notification: Mutex::new(notification),
            resume_notification,
            next_started: Mutex::new(next_started),
            resume_next,
        }
    }
    fn wait_for_readiness(&self) -> (idle_diagnostics::Wait, idle_diagnostics::Signal) {
        let comparison = self.pool.shared.ready_wake.load(Ordering::SeqCst);
        let begin = crate::sampling::qpc();
        self.pool
            .park_until_ready(|| {
                self.pool.is_complete(self.ticket).then_some(())
            });
        let signal = receive(&self.readiness.lock().unwrap());
        (
            idle_diagnostics::Wait {
                address: idle_diagnostics::Address::Readiness,
                comparison,
                observed: self.pool.shared.ready_wake.load(Ordering::SeqCst),
                begin,
                end: crate::sampling::qpc(),
                ..Default::default()
            },
            signal,
        )
    }
    fn diagnostics(&self, wait: idle_diagnostics::Wait) -> idle_diagnostics::Hour {
        self.pool.settle();
        let mut hour = idle_diagnostics::Hour {
            wait: Some(wait),
            ..Default::default()
        };
        // SAFETY: both jobs were collected and every publisher has settled.
        unsafe {
            self.pool
                .shared
                .wake_events
                .as_ref()
                .unwrap()
                .drain_into(&mut hour)
        };
        assert_eq!(hour.dropped_signals, 0);
        hour
    }
}

#[test]
fn readiness_wake_survives_callback_progress_and_same_slot_reuse_before_join_wake() {
    {
        let fixture = WakeFixture::new();
        let (wait, readiness) = fixture.wait_for_readiness();
        assert_eq!(fixture.pool.finish(fixture.ticket).unwrap().report, Ok(1));
        // SAFETY: the fixture retains the replacement until its gated worker publishes.
        let next = unsafe {
            fixture.pool.submit(&mut Some(WakeJob {
                value: 2,
                publication: None,
            }))
        }
        .unwrap();
        assert_eq!(next.slot, fixture.ticket.slot);
        assert_ne!(next.generation, fixture.ticket.generation);
        receive(&fixture.next_started.lock().unwrap());
        let (joining, registered) = mpsc::channel();
        assert!(
            fixture
                .pool
                .shared
                .hooks
                .before_join_wait
                .set(Box::new(move |ticket| {
                    joining.send(ticket).unwrap();
                }))
                .is_ok()
        );
        std::thread::scope(|scope| {
            let fixture = &fixture;
            scope.spawn(move || {
                assert_eq!(receive(&registered), next);
                fixture.resume_wake.send(()).unwrap();
                receive(&fixture.notification.lock().unwrap());
                fixture.resume_next.send(()).unwrap();
                fixture.resume_notification.send(()).unwrap();
            });
            let mut completion = None;
            assert!(fixture.pool.finish_into_timed(next, &mut completion));
            assert_eq!(completion.unwrap().report, Ok(2));
        });
        let hour = fixture.diagnostics(wait);
        assert_eq!(hour.matching_signals, 1);
        assert_eq!(hour.signal, Some(readiness));
        assert_eq!(fixture.pool.calibration().active, 0);
        assert_eq!(fixture.pool.calibration().uncollected, 0);
    }
}

#[test]
fn one_publication_records_readiness_and_its_join_address_as_separate_wakes() {
    {
        for readiness_wait in [false, true] {
            let fixture = WakeFixture::new();
            let (readiness, signal) = fixture.wait_for_readiness();
            // Keep this generation registered to exercise both wake addresses in one publication.
            fixture
                .pool
                .shared
                .awaited
                .store(fixture.ticket.generation, Ordering::SeqCst);
            fixture.resume_wake.send(()).unwrap();
            receive(&fixture.notification.lock().unwrap());
            let join = idle_diagnostics::Wait {
                address: idle_diagnostics::Address::Slot,
                ticket: Some(fixture.ticket),
                begin: readiness.end,
                end: crate::sampling::qpc(),
                ..Default::default()
            };
            fixture.pool.shared.awaited.store(0, Ordering::SeqCst);
            assert_eq!(fixture.pool.finish(fixture.ticket).unwrap().report, Ok(1));
            fixture.resume_notification.send(()).unwrap();
            let hour = fixture.diagnostics(if readiness_wait { readiness } else { join });
            assert_eq!(hour.matching_signals, 1);
            if readiness_wait {
                assert_eq!(hour.signal, Some(signal));
            } else {
                let joined = hour.signal.unwrap();
                assert_eq!(joined.ticket, Some(fixture.ticket));
                assert_eq!(joined.readiness, 0);
                assert!(joined.slot);
            }
        }
    }
}
