use super::*;

#[test]
fn closing_a_busy_fanin_never_blocks_and_keeps_the_ordered_fallback() {
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let executor = Arc::new(
        unsafe {
            collector_workers::acquisition::Pool::start(1, 8, |_: &mut Job| {
                Report::DivisionInputs(false)
            })
        }
        .unwrap(),
    );
    let calibration = Arc::new(SharedCalibration::default());
    let inputs = PresentationInputs::new(&executor, &calibration);
    let mut state = inputs.state.lock().unwrap();
    state.closed = false;
    state.remaining = 2;
    assert!(inputs.try_close().is_none());
    assert!(!state.closed);
    drop(state);
    drop(inputs.try_close().expect("unlocked handoff"));
    let state = inputs.state.lock().unwrap();
    assert!(state.closed);
    assert_eq!(state.remaining, 2);
}

#[test]
fn failed_input_closes_before_the_last_notification_and_keeps_ordered_fallback() {
    use collector_workers::acquisition::Publication;
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let executor = Arc::new(
        unsafe {
            collector_workers::acquisition::Pool::start(1, 8, |_: &mut Job| {
                Report::DivisionInputs(false)
            })
        }
        .unwrap(),
    );
    let calibration = Arc::new(SharedCalibration::default());
    let inputs = PresentationInputs::new(&executor, &calibration);
    {
        let mut state = inputs.state.lock().unwrap();
        state.closed = false;
        state.remaining = 1;
        state.master = Some((Census::default(), Err(Failure::Stopped)));
    }
    let mut handoff = inputs.try_close().expect("unlocked handoff");
    let mut census = Census::default();
    assert_eq!(handoff.restore_master(&mut census), (false, None));
    drop(handoff);
    inputs.published();
    let state = inputs.state.lock().unwrap();
    assert!(state.master.is_none());
    assert!(state.pending.is_none());
    assert!(!state.joined);
}

#[test]
fn successful_inputs_wait_for_the_last_notification_without_closing_the_fanin() {
    use collector_workers::acquisition::Publication;
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let executor = Arc::new(
        unsafe {
            collector_workers::acquisition::Pool::start(1, 8, |_: &mut Job| {
                Report::DivisionInputs(false)
            })
        }
        .unwrap(),
    );
    let calibration = Arc::new(SharedCalibration::default());
    let inputs = PresentationInputs::new(&executor, &calibration);
    {
        let mut state = inputs.state.lock().unwrap();
        state.closed = false;
        state.remaining = 1;
        state.master = Some((Census::default(), Ok(Report::DivisionInputs(false))));
        state.omit_status = true;
    }
    assert!(inputs.try_close().is_none());
    assert!(!inputs.state.lock().unwrap().closed);
    inputs.published();
    let mut handoff = inputs.try_close().expect("notified handoff");
    assert_eq!(handoff.restore_master(&mut Census::default()), (true, None));
    assert!(handoff.take_dispatched().is_none());
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PublicationUnwind {
    None,
    BeforeStore,
    BeforeSubmit,
    BeforeFailureOutcome,
}

fn parked_callback_handoff(unwind: PublicationUnwind) {
    use std::sync::{atomic::AtomicBool, mpsc};
    let submit_unwind = matches!(
        unwind,
        PublicationUnwind::BeforeSubmit | PublicationUnwind::BeforeFailureOutcome
    );
    struct AbortParkedCallback<'a> {
        executor: &'a collector_workers::acquisition::Pool<Job, Report>,
        abort: &'a AtomicBool,
    }
    impl Drop for AbortParkedCallback<'_> {
        fn drop(&mut self) {
            self.abort.store(true, Ordering::Release);
            self.executor.notify_ready();
        }
    }
    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
    let executor = Arc::new(
        unsafe {
            collector_workers::acquisition::Pool::start(1, 8, |job: &mut Job| match job {
                Job::DivisionInputs(_) => Report::DivisionInputs(true),
                Job::Presentation(_) => Report::Presentation(Default::default()),
                _ => panic!("unexpected handoff fixture job"),
            })
        }
        .unwrap(),
    );
    executor.reserve_worker_slots(4).unwrap();
    executor.set_on_completed(super::super::completed);
    let inputs = Arc::new(PresentationInputs::new(&executor, &Arc::default()));
    let store = division_presentation::held_store::Slot::new(Box::new(
        collector_division_parts::attack_stats::ArmyStatStore::new(),
    ))
    .unwrap();
    let older = store.take().unwrap();
    let address = &raw const *older;
    let mut hour_store = None;
    let (publishing, published) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let resumed = Mutex::new(resumed);
    assert!(
        inputs
            .before_store
            .set(Box::new(move || {
                publishing.send(()).unwrap();
                resumed
                    .lock()
                    .unwrap()
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                assert!(
                    unwind != PublicationUnwind::BeforeStore,
                    "synthetic publication unwind while owning the handoff"
                );
            }))
            .is_ok()
    );
    assert!(
        inputs
            .before_submit
            .set(Box::new(move || {
                assert!(
                    !submit_unwind,
                    "synthetic publication unwind while submitting the retained request"
                );
            }))
            .is_ok()
    );
    let (failing, failed) = mpsc::channel();
    let (allow_failure, failure_allowed) = mpsc::channel();
    let failure_allowed = Mutex::new(failure_allowed);
    let failing_inputs = Arc::downgrade(&inputs);
    let failure_panicked = Arc::new(AtomicBool::new(false));
    let failure_panic = Arc::clone(&failure_panicked);
    assert!(
        inputs
            .before_failure
            .set(Box::new(move || {
                failing.send(()).unwrap();
                failure_allowed
                    .lock()
                    .unwrap()
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                if unwind == PublicationUnwind::BeforeFailureOutcome {
                    let inputs = failing_inputs.upgrade().unwrap();
                    let state = inputs
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    assert_eq!(state.remaining, 1);
                    assert!(!state.closed && state.failure.is_none());
                    assert!(state.request.is_some());
                    failure_panic.store(true, Ordering::Release);
                    panic!("synthetic failure handler unwind before installing its outcome");
                }
            }))
            .is_ok()
    );
    let failure_window = AtomicBool::new(false);
    let (probe, probed) = mpsc::channel();
    let (park, parked) = mpsc::channel();
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let mut captured = division_presentation::Captured::with_capacities(&Default::default());
    captured.reserve_sources(3);
    let bytes = captured.allocated_bytes();
    assert!(bytes > 0);
    inputs.arm(
        PresentationRequest {
            countries: engine_allocation::SharedBox::clone(&countries),
            context: division_presentation::CaptureContext {
                base: 0,
                game_state: 0,
                hour: 1,
            },
            captured,
            dispatch: Default::default(),
            inputs: None,
            army_stats: None,
        },
        0,
        Some(store.clone()),
        false,
    );
    let mut payload = sampling::DivisionsJob::new_worker();
    payload.presentation = Some(Arc::clone(&inputs));
    let ticket = unsafe {
        executor.submit(&mut Some(Job::DivisionInputs(RawRequest {
            countries: engine_allocation::SharedBox::clone(&countries),
            context: Default::default(),
            payload,
        })))
    }
    .unwrap();
    published
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
    assert!(executor.is_complete(ticket));
    assert!(inputs.try_close().is_none());
    let (finish, finished) = mpsc::channel();
    let mut older = Some(older);
    let abort = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let _abort = AbortParkedCallback {
            executor: &executor,
            abort: &abort,
        };
        let callback = scope.spawn(|| {
            let mut probe = Some(probe);
            let handoff = executor.wait_ready_observing(|| {
                if abort.load(Ordering::Acquire) {
                    return Some(None);
                }
                assert!(executor.is_complete(ticket));
                let handoff = inputs.try_close();
                if failure_window.load(Ordering::Acquire)
                    && let Some(probe) = probe.take()
                {
                    probe.send(handoff.is_some()).unwrap();
                }
                handoff.map(Some)
            }, || park.send(()).unwrap());
            let Some(mut handoff) = handoff else { return };
            let restored = handoff.restore_master(&mut Census::default());
            let dispatched = handoff.take_dispatched();
            let retained = handoff.take_request();
            if dispatched.is_some() {
                assert!(retained.is_none());
            } else {
                assert!(
                    retained.is_some(),
                    "production requires a retained presentation request"
                );
            }
            drop(handoff);
            finish.send((restored, dispatched, retained)).unwrap();
        });
        parked
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        assert!(
            finished.try_recv().is_err(),
            "callback closed the publisher's locked handoff"
        );
        resume.send(()).unwrap();
        if unwind != PublicationUnwind::None {
            failed
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
            assert!(inputs.state.is_poisoned());
            {
                let state = inputs
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                assert_eq!(state.remaining, 1);
                assert!(!state.closed && state.failure.is_none());
                assert!(state.request.is_some());
                assert_eq!(state.movement.is_some(), submit_unwind);
                assert_eq!(state.speed_terms.is_some(), submit_unwind);
                assert_eq!(
                    state.request.as_ref().unwrap().army_stats.is_some(),
                    submit_unwind
                );
            }
            failure_window.store(true, Ordering::Release);
            executor.notify_ready();
            assert!(
                !probed
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap(),
                "production readiness accepted the unfinished failure outcome"
            );
            assert!(finished.try_recv().is_err());
            allow_failure.send(()).unwrap();
        }
        let ((joined, failure), dispatched, retained) = finished
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        assert!(joined);
        if unwind != PublicationUnwind::None {
            assert_eq!(
                failure,
                Some("division input continuation panicked"),
                "the hour must retain its input refusal even if the failure handler unwinds"
            );
            assert!(dispatched.is_none());
            let retained = retained.expect("presentation retained");
            assert!(engine_allocation::SharedBox::ptr_eq(
                &retained.countries,
                &countries
            ));
            assert_eq!(retained.captured.allocated_bytes(), bytes);
            if submit_unwind {
                let taken = std::ptr::from_ref(retained.army_stats.as_deref().unwrap());
                assert_ne!(taken, address, "the hour took a store of its own");
                hour_store = Some(taken);
            }
            drop(retained);
            drop(older.take());
        } else {
            assert_eq!(failure, None);
            let (pending, movement, speed_terms, split) = dispatched.unwrap();
            assert!(!pending.parked && !split);
            let completion = executor.finish(pending.ticket).unwrap();
            assert!(matches!(completion.report, Ok(Report::Presentation(_))));
            let Job::Presentation(request) = &completion.request else {
                unreachable!()
            };
            let taken = std::ptr::from_ref(request.army_stats.as_deref().unwrap());
            assert_ne!(taken, address, "the hour took a store of its own");
            hour_store = Some(taken);
            drop(completion);
            drop(older.take());
            drop((movement, speed_terms));
        }
        callback.join().unwrap();
    });
    let completion = executor.finish(ticket).unwrap();
    assert!(matches!(
        completion.report,
        Ok(Report::DivisionInputs(true))
    ));
    drop(completion);
    executor.settle();
    inputs.drain();
    let stats = executor.calibration();
    assert_eq!(
        (stats.active, stats.uncollected, stats.caller_executed),
        (0, 0, 0)
    );
    assert_eq!(stats.panicked, u64::from(unwind != PublicationUnwind::None));
    assert_eq!(executor.stopped(), unwind != PublicationUnwind::None);
    assert_eq!(
        failure_panicked.load(Ordering::Acquire),
        unwind == PublicationUnwind::BeforeFailureOutcome
    );
    assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
    assert_eq!(Arc::strong_count(&inputs), 1);
    let state = inputs
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(state.closed && state.request.is_none() && state.pending.is_none());
    assert!(state.master.is_none() && state.parts.iter().all(Option::is_none));
    assert!(state.movement.is_none() && state.speed_terms.is_none());
    assert_eq!(store.owners(), 1);
    assert_eq!(
        &raw const *store.take().unwrap(),
        hour_store.unwrap_or(address)
    );
}

#[test]
fn a_parked_callback_never_waits_for_the_previous_hours_store_loan() {
    parked_callback_handoff(PublicationUnwind::None);
}

#[test]
fn a_parked_callback_is_notified_after_publication_unwind_and_failure_unlock() {
    parked_callback_handoff(PublicationUnwind::BeforeStore);
}

#[test]
fn publication_unwind_returns_the_retained_request_passes_and_store_loan() {
    parked_callback_handoff(PublicationUnwind::BeforeSubmit);
}

#[test]
fn failure_handler_unwind_closes_readiness_refuses_the_hour_and_returns_every_lease() {
    parked_callback_handoff(PublicationUnwind::BeforeFailureOutcome);
}
