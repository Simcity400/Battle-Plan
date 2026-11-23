use super::*;
use std::sync::{Condvar, Mutex, mpsc};

#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
    fn wait(&self) {
        let (open, timeout) = self
            .changed
            .wait_timeout_while(
                self.open.lock().unwrap(),
                collector_test_support::TEST_HANG_GUARD,
                |open| !*open,
            )
            .unwrap();
        assert!(*open && !timeout.timed_out(), "final-input fixture gate");
    }
}

fn character(part: usize, dispatch: &Arc<acquisition::FinalInputsDispatch>) -> acquisition::Job {
    acquisition::Job::Characters(acquisition::CharactersRequest {
        final_inputs: Some(Arc::clone(dispatch)),
        context: collector_characters::extended::characters::CharactersCopyContext {
            base: 0,
            game_state: 0,
            force_full: false,
            active: collector_characters::extended::characters::PART_MASKS[part],
        },
        payload: collector_characters::extended::characters::CharactersJob::vacant(part),
        roles: collector_characters::extended::characters::Roles::Independent,
    })
}

fn final_request() -> extended::FinalInputsRequest {
    let mut captured = extended::Captured::with_capacities(&extended::Capacities {
        division_sources: 5_000,
        ..Default::default()
    });
    captured.take_final_inputs_request(
        &engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap(),
        0,
        0,
        41,
        false,
    )
}

fn source(index: usize, failure: usize) -> acquisition::Job {
    acquisition::Job::Provinces(acquisition::ProvincesRequest {
        base: index,
        game_state: failure,
        payload: engine_native::cprovince::Capture::try_with_capacity(0, 0, 0).unwrap(),
    })
}

#[test]
fn final_sources_early_join_requires_every_successful_report_without_consuming_any_ticket() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool = acquisition::Pool::with_test_runner(1, |job| {
        let acquisition::Job::Provinces(request) = job else {
            unreachable!()
        };
        assert_ne!(request.game_state, 2, "synthetic source failure");
        let raw_ok = request.game_state == 0;
        match request.base {
            0 => acquisition::Report::IntelResidual(Default::default()),
            1 => {
                acquisition::Report::Capitulation(collector_intel::intel::CapitulationCopyReport {
                    raw_ok,
                    ..Default::default()
                })
            }
            2 => acquisition::Report::Session(collector_session::session::SessionCopyReport {
                raw_ok,
                ..Default::default()
            }),
            3 => acquisition::Report::Relations(collector_intel::intel::RelationsCopyReport {
                raw_ok,
                ..Default::default()
            }),
            4 => acquisition::Report::Variants([0; collector_production::production::N]),
            _ => unreachable!(),
        }
    });
    let mut batch = acquisition::Batch::new(&pool);
    let producers: [PendingAcquisition; 4] = std::array::from_fn(|index| unsafe {
        submit_acquisition(&shared, &mut batch, source(index, 0))
    });
    batch.wait_ready(|batch| {
        producers
            .iter()
            .all(|p| pending_complete(batch, p))
            .then_some(())
    });
    collector_test_support::with_allocations_denied(|| {
        assert!(final_sources_published(
            &batch,
            producers.each_ref().map(Some)
        ));
        assert!(final_sources_published(
            &batch,
            [Some(&producers[0]), Some(&producers[1]), None, None]
        ));
    });
    assert_eq!(pool.calibration().executor.uncollected, 4);
    for index in [0, 1] {
        let mut refs = producers.each_ref().map(Some);
        refs[index] = None;
        assert!(!final_sources_published(&batch, refs));
    }
    for index in 1..4 {
        let bad = unsafe { submit_acquisition(&shared, &mut batch, source(index, 1)) };
        batch.wait_ready(|batch| pending_complete(batch, &bad).then_some(()));
        let mut refs = producers.each_ref().map(Some);
        refs[index] = Some(&bad);
        assert!(!final_sources_published(&batch, refs));
        drop(unsafe { finish_acquisition(&shared, &mut batch, bad) });
    }
    let rejected = park(&mut batch, source(0, 0));
    let PendingAcquisition::Rejected(ticket) = rejected else {
        panic!()
    };
    let unpublished = PendingAcquisition::Accepted(ticket);
    for missing in [&rejected, &unpublished] {
        let mut refs = producers.each_ref().map(Some);
        refs[0] = Some(missing);
        assert!(!final_sources_published(&batch, refs));
    }
    let failed = unsafe { submit_acquisition(&shared, &mut batch, source(0, 2)) };
    batch.wait_ready(|batch| pending_complete(batch, &failed).then_some(()));
    let mut refs = producers.each_ref().map(Some);
    refs[0] = Some(&failed);
    assert!(!final_sources_published(&batch, refs));
}

#[test]
fn final_inputs_publication_waits_for_every_part_and_the_admission_fence_without_callback_joins() {
    for required in [0_usize, 1 << 2, 0b1110] {
        for arm_before_last in [false, true] {
            let shared =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Shared::new(0, 0, 1) };
            let gates: [Arc<Gate>; CHARACTER_PARTS] = std::array::from_fn(|_| Arc::default());
            let (started, running) = mpsc::channel();
            let (finished, finishing) = mpsc::channel();
            let pool = acquisition::Pool::with_test_runner(4, {
                let gates = gates.clone();
                move |job| match job {
                    acquisition::Job::Characters(request) => {
                        started.send(request.payload.part).unwrap();
                        gates[request.payload.part].wait();
                        acquisition::Report::Characters(
                            collector_characters::extended::characters::CharactersCopyReport {
                                raw_ok: true,
                                ..Default::default()
                            },
                        )
                    }
                    acquisition::Job::FinalInputs(_) => {
                        finished.send(()).unwrap();
                        acquisition::Report::FinalInputs(Default::default())
                    }
                    _ => unreachable!(),
                }
            });
            let mut batch = acquisition::Batch::new(&pool);
            let dispatch = batch.final_inputs();
            let mut tickets: [Option<PendingAcquisition>; CHARACTER_PARTS] =
                [None; CHARACTER_PARTS];
            for (part, ticket) in tickets.iter_mut().enumerate().skip(1) {
                if required & (1 << part) != 0 {
                    *ticket = Some(unsafe {
                        submit_acquisition(&shared, &mut batch, character(part, &dispatch))
                    });
                }
            }
            for _ in 0..required.count_ones() {
                running
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
            }
            let last = (1..CHARACTER_PARTS).rfind(|part| required & (1 << part) != 0);
            for part in (1..CHARACTER_PARTS)
                .rev()
                .filter(|&part| Some(part) != last)
            {
                gates[part].release();
            }
            if !arm_before_last {
                if let Some(last) = last {
                    gates[last].release();
                }
                batch.wait_ready(|batch| {
                    tickets
                        .iter()
                        .flatten()
                        .all(|p| pending_complete(batch, p))
                        .then_some(())
                });
                assert!(
                    finishing.try_recv().is_err(),
                    "publication cannot bypass admission"
                );
            }
            let request = final_request();
            let countries = engine_allocation::SharedBox::clone(&request.countries);
            let bytes = request.allocated_bytes();
            collector_test_support::with_allocations_denied(|| {
                dispatch.arm(request, required);
            });
            if arm_before_last && let Some(last) = last {
                assert!(
                    finishing.try_recv().is_err(),
                    "a prefix released final inputs"
                );
                gates[last].release();
            }
            finishing
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
            assert_eq!(
                pool.calibration().executor.uncollected,
                required.count_ones() as usize + 1
            );
            for pending in tickets.into_iter().flatten() {
                let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
                assert!(matches!(
                    completed.report,
                    Ok(acquisition::Report::Characters(_))
                ));
            }
            let (pending, unrun) = dispatch.take();
            assert!(unrun.is_none());
            let (ticket, parked) = batch.adopt_presentation_ticket(pending.unwrap());
            assert!(!parked);
            let completed = unsafe {
                finish_acquisition(&shared, &mut batch, PendingAcquisition::Accepted(ticket))
            };
            let acquisition::Job::FinalInputs(request) = completed.request else {
                panic!()
            };
            assert!(engine_allocation::SharedBox::ptr_eq(
                &request.countries,
                &countries
            ));
            assert_eq!(request.allocated_bytes(), bytes);
            let calibration = pool.calibration();
            assert_eq!(calibration.executor.uncollected, 0);
            assert_eq!(calibration.executor.caller_executed, 0);
            let counters =
                calibration.roles[collector_workers::roles::Role::FinalInputs as usize].counters;
            assert_eq!(
                (counters.accepted, counters.completed, counters.rejected),
                (1, 1, 0)
            );
        }
    }
}

#[test]
fn final_inputs_failed_incomplete_and_unnotified_characters_return_the_unrun_loan() {
    for failure in 0..5 {
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let blocker = Arc::new(Gate::default());
        let (started, running) = mpsc::channel();
        let pool = acquisition::Pool::with_test_runner(1, {
            let blocker = Arc::clone(&blocker);
            move |job| {
                if matches!(job, acquisition::Job::Provinces(_)) {
                    started.send(()).unwrap();
                    blocker.wait();
                    return acquisition::Report::ShipModels;
                }
                assert!(
                    matches!(job, acquisition::Job::Characters(_)),
                    "final inputs retried"
                );
                assert_ne!(failure, 2, "synthetic character failure");
                acquisition::Report::Characters(
                    collector_characters::extended::characters::CharactersCopyReport {
                        raw_ok: failure != 0,
                        plan_failed: failure == 1,
                        ..Default::default()
                    },
                )
            }
        });
        let mut batch = acquisition::Batch::new(&pool);
        let dispatch = batch.final_inputs();
        let pending = if failure == 3 {
            park(&mut batch, character(2, &dispatch))
        } else {
            if failure == 4 {
                let blocker_job = acquisition::Job::Provinces(acquisition::ProvincesRequest {
                    base: 0,
                    game_state: 0,
                    payload: engine_native::cprovince::Capture::try_with_capacity(0, 0, 0).unwrap(),
                });
                unsafe { submit_acquisition(&shared, &mut batch, blocker_job) };
                running
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
            }
            let pending =
                unsafe { submit_acquisition(&shared, &mut batch, character(2, &dispatch)) };
            if failure == 4 {
                batch.stop_for_contained_panic(collector_workers::roles::Role::OrdersStitch);
                blocker.release();
            }
            batch.wait_ready(|batch| pending_complete(batch, &pending).then_some(()));
            pending
        };
        let request = final_request();
        let countries = engine_allocation::SharedBox::clone(&request.countries);
        let bytes = request.allocated_bytes();
        dispatch.arm(request, 1 << 2);
        drop(unsafe { finish_acquisition(&shared, &mut batch, pending) });
        let (pending, request) = dispatch.take();
        assert!(pending.is_none());
        let request = request.unwrap();
        assert!(engine_allocation::SharedBox::ptr_eq(
            &request.countries,
            &countries
        ));
        assert_eq!(request.allocated_bytes(), bytes);
        assert_eq!(
            pool.calibration().roles[collector_workers::roles::Role::FinalInputs as usize]
                .counters
                .accepted,
            0
        );
        drop(batch);
        assert_eq!(pool.calibration().executor.uncollected, 0);
        assert_eq!(pool.calibration().executor.caller_executed, 0);
    }
}

#[test]
fn final_inputs_rejected_successor_uses_the_existing_parked_failure_without_native_retry() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool = acquisition::Pool::with_test_runner(1, |_| panic!("rejected successor ran"));
    let mut batch = acquisition::Batch::new(&pool);
    let dispatch = batch.final_inputs();
    batch.stop_for_contained_panic(collector_workers::roles::Role::OrdersStitch);
    let request = final_request();
    let countries = engine_allocation::SharedBox::clone(&request.countries);
    dispatch.arm(request, 0);
    let (pending, request) = dispatch.take();
    assert!(request.is_none());
    let (ticket, parked) = batch.adopt_presentation_ticket(pending.unwrap());
    assert!(parked);
    let completed =
        unsafe { finish_acquisition(&shared, &mut batch, PendingAcquisition::Rejected(ticket)) };
    assert!(matches!(
        completed.report,
        Err(acquisition::Failure::Stopped)
    ));
    assert_eq!(
        pool.calibration().roles[collector_workers::roles::Role::FinalInputs as usize]
            .counters
            .inline_completed,
        0
    );
    #[cfg(feature = "acquisition_profile")]
    assert_eq!(
        completed.table_reads,
        collector_memory::read_accounting::ZERO
    );
    let acquisition::Job::FinalInputs(request) = completed.request else {
        panic!()
    };
    assert!(engine_allocation::SharedBox::ptr_eq(
        &request.countries,
        &countries
    ));
    let calibration = pool.calibration();
    assert_eq!(calibration.executor.uncollected, 0);
    assert_eq!(calibration.executor.caller_executed, 0);
}

#[test]
fn final_inputs_unadopted_successors_drain_on_unwind_and_pool_storage_rearms() {
    use collector_memory::read_accounting::{self, Capture, fixtures};
    use replay_data::rows::TableId;

    struct ReleaseOnDrop(Arc<Gate>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    for failure in 0..3 {
        let gate = Arc::new(Gate::default());
        let (started, running) = mpsc::channel();
        let trace = Arc::new(fixtures::ReadTrace::default());
        let pool = acquisition::Pool::with_test_runner(1, {
            let gate = Arc::clone(&gate);
            let runner_trace = Arc::clone(&trace);
            move |job| {
                runner_trace.record((), || {
                    started.send(()).unwrap();
                    gate.wait();
                    if matches!(job, acquisition::Job::Provinces(_)) {
                        assert_eq!(failure, 2);
                        return acquisition::Report::ShipModels;
                    }
                    assert!(matches!(job, acquisition::Job::FinalInputs(_)));
                    read_accounting::with_table(TableId::DivisionLandInputs, || fixtures::read(5));
                    assert_ne!(failure, 1, "synthetic final input failure");
                    acquisition::Report::FinalInputs(Default::default())
                })
            }
        });
        for hour in 1..=if failure == 0 { 2 } else { 1 } {
            let reads = Capture::new();
            *gate.open.lock().unwrap() = false;
            let request = final_request();
            let countries = engine_allocation::SharedBox::clone(&request.countries);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut batch = acquisition::Batch::new(&pool);
                let _release = ReleaseOnDrop(Arc::clone(&gate));
                if failure == 2 {
                    let shared =
                    // SAFETY: The caller owns the session or isolated fixture state required by
                    // this constructor.
                    unsafe { Shared::new(0, 0, 1) };
                    unsafe { submit_acquisition(&shared, &mut batch, source(0, 0)) };
                    running
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                        .unwrap();
                }
                let dispatch = batch.final_inputs();
                dispatch.arm(request, 0);
                if failure == 2 {
                    batch.stop_for_contained_panic(collector_workers::roles::Role::OrdersStitch);
                } else {
                    running
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                        .unwrap();
                }
                let counters = pool.calibration().roles
                    [collector_workers::roles::Role::FinalInputs as usize]
                    .counters;
                assert_eq!((counters.accepted, counters.completed), (hour, hour - 1));
                assert_eq!(read_accounting::snapshot(), read_accounting::ZERO);
                panic!("synthetic parent unwind with unadopted successor");
            }));
            assert!(result.is_err());
            assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
            let stats = pool.calibration();
            assert_eq!(
                (
                    stats.executor.uncollected,
                    stats.executor.active,
                    stats.executor.queued
                ),
                (0, 0, 0)
            );
            assert_eq!(stats.executor.caller_executed, 0);
            assert_eq!(stats.executor.accepted, stats.executor.completed);
            let counters =
                stats.roles[collector_workers::roles::Role::FinalInputs as usize].counters;
            assert_eq!(
                (counters.accepted, counters.completed, counters.rejected),
                (hour, hour, 0)
            );
            assert_eq!(
                (counters.panicked, counters.stopped),
                (u64::from(failure == 1), u64::from(failure == 2))
            );
            let mut expected = read_accounting::ZERO;
            if failure != 2 {
                expected[TableId::DivisionLandInputs.index()] = fixtures::copies(1, 5);
            }
            assert_eq!(trace.take(()), expected);
            #[cfg(not(feature = "acquisition_profile"))]
            assert_eq!(reads.finish(), read_accounting::ZERO);
            #[cfg(feature = "acquisition_profile")]
            assert_eq!(reads.finish(), expected);
        }
    }
}

#[test]
fn final_inputs_dispatches_while_the_unrelated_variant_job_is_still_running() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let gate = Arc::new(Gate::default());
    let (started, start) = mpsc::channel();
    let (finished, finish) = mpsc::channel();
    let pool = acquisition::Pool::with_test_runner(2, {
        let gate = Arc::clone(&gate);
        move |job| match job {
            acquisition::Job::Provinces(request) if request.base == 4 => {
                started.send(()).unwrap();
                gate.wait();
                acquisition::Report::Variants([0; collector_production::production::N])
            }
            acquisition::Job::Provinces(request) => match request.base {
                0 => acquisition::Report::IntelResidual(Default::default()),
                1 => acquisition::Report::Capitulation(
                    collector_intel::intel::CapitulationCopyReport {
                        raw_ok: true,
                        ..Default::default()
                    },
                ),
                2 => acquisition::Report::Session(collector_session::session::SessionCopyReport {
                    raw_ok: true,
                    ..Default::default()
                }),
                3 => acquisition::Report::Relations(collector_intel::intel::RelationsCopyReport {
                    raw_ok: true,
                    ..Default::default()
                }),
                _ => unreachable!(),
            },
            acquisition::Job::FinalInputs(_) => {
                finished.send(()).unwrap();
                acquisition::Report::FinalInputs(Default::default())
            }
            _ => unreachable!(),
        }
    });
    let mut batch = acquisition::Batch::new(&pool);
    let variant = unsafe { submit_acquisition(&shared, &mut batch, source(4, 0)) };
    start
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
    let producers: [PendingAcquisition; 4] = std::array::from_fn(|index| unsafe {
        submit_acquisition(&shared, &mut batch, source(index, 0))
    });
    batch.wait_ready(|batch| {
        final_sources_published(batch, producers.each_ref().map(Some)).then_some(())
    });
    assert!(!pending_complete(&batch, &variant));
    batch.final_inputs().arm(final_request(), 0);
    finish
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
    assert!(!pending_complete(&batch, &variant));
    gate.release();
}
