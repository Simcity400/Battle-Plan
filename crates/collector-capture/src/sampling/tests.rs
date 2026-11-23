mod final_inputs_tests;
mod presentation_tests;
mod stitch_barrier;

use super::*;

struct NoDivisionReads;
impl engine_native::Memory for NoDivisionReads {
    fn copy(&self, address: usize, _: &mut [u8]) -> bool {
        panic!("division handoff retried native input at {address:#x}");
    }
}

struct NoRosterRetry<'a>(&'a [u8]);
impl engine_native::Memory for NoRosterRetry<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        use super::divisions_test_kit as kit;
        assert!(
            ![kit::C0 + ccountry::DIVISIONS, kit::C1 + ccountry::DIVISIONS].contains(&address),
            "dependent worker retried the original division roster"
        );
        assert!(
            !(kit::A1..kit::A3 + 0x1000).contains(&address),
            "dependent worker retried original unit inputs at {address:#x}"
        );
        kit::memory(self.0).copy(address, output)
    }
}

fn park(batch: &mut acquisition::Batch<'_>, request: acquisition::Job) -> PendingAcquisition {
    let mut retained = Some(request);
    PendingAcquisition::Rejected(batch.park(&mut retained).expect("free acquisition slot"))
}

#[test]
fn a_slow_loan_dropped_by_an_unwinding_capture_leaves_the_doctrine_observer_store_alive() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let bound = collector_observers::doctrine_changes::bound_context();
    let stores = unsafe { lend_slow_stores(&shared) };
    let lent = stores
        .doctrines
        .as_deref()
        .map(|store| store as *const _ as usize);
    assert_eq!(lent, Some(bound));
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _stores = stores;
        panic!("slow capture unwinds with its stores on loan");
    }));
    assert!(unwound.is_err());
    assert_eq!(
        engine_allocation::SharedBox::as_ptr(&shared.doctrine_store) as usize,
        bound
    );
    assert_eq!(
        engine_allocation::SharedBox::strong_count(&shared.doctrine_store),
        1
    );
    assert!(shared.doctrine_store.lock().is_ok());
}

#[test]
fn rejected_division_inputs_keep_the_original_owner_and_cannot_retry_native_inputs() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let payload = DivisionsJob::new_worker();
    let owner = engine_allocation::SharedBox::clone(payload.census.unit_inputs().unwrap());
    let context = DivisionsCopyContext {
        game_state: usize::MAX,
        details_active: false,
        ..Default::default()
    };
    let request = acquisition::Job::DivisionInputs(acquisition::RawRequest {
        countries: engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap(),
        context,
        payload,
    });
    let rejected = park(&mut batch, request);
    let reads = collector_memory::read_accounting::Capture::new();
    let completed = collector_test_support::with_allocations_denied(|| unsafe {
        finish_acquisition(&shared, &mut batch, rejected)
    });
    assert!(matches!(
        completed.report,
        Err(acquisition::Failure::Stopped)
    ));
    assert_eq!(
        pool.calibration().roles[collector_workers::roles::Role::DivisionInputs as usize]
            .counters
            .inline_completed,
        0
    );
    #[cfg(feature = "acquisition_profile")]
    assert_eq!(
        completed.table_reads,
        collector_memory::read_accounting::ZERO
    );
    let acquisition::Job::DivisionInputs(mut request) = completed.request else {
        panic!("rejected division input owner lost");
    };
    assert!(engine_allocation::SharedBox::ptr_eq(
        &owner,
        request.payload.census.unit_inputs().unwrap()
    ));
    assert!(!request.payload.inputs_captured);
    let report = copy_divisions_core_refresh(&NoDivisionReads, context, &mut request.payload);
    assert!(!report.raw_ok);
    assert!(request.payload.core_finished);
    assert!(engine_allocation::SharedBox::ptr_eq(
        &owner,
        request.payload.census.unit_inputs().unwrap()
    ));
    assert!(!copy_divisions_core_refresh(&NoDivisionReads, context, &mut request.payload).raw_ok);
    assert_eq!(reads.finish(), collector_memory::read_accounting::ZERO);
    let counters =
        pool.calibration().roles[collector_workers::roles::Role::DivisionInputs as usize].counters;
    assert_eq!(counters.inline_completed, 0);
    assert_eq!(pool.calibration().executor.uncollected, 0);
}

#[test]
fn a_rejected_submission_parks_in_the_pool_and_its_join_returns_it() {
    assert!(size_of::<PendingAcquisition>() <= 2 * size_of::<acquisition::Ticket>());
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let payload = DivisionsJob::new_worker();
    let owner = engine_allocation::SharedBox::clone(payload.census.unit_inputs().unwrap());
    let request = acquisition::Job::DivisionInputs(acquisition::RawRequest {
        countries: engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap(),
        context: DivisionsCopyContext {
            game_state: usize::MAX,
            details_active: false,
            ..Default::default()
        },
        payload,
    });
    let pending = unsafe { submit_acquisition(&shared, &mut batch, request) };
    assert!(matches!(pending, PendingAcquisition::Rejected(_)));
    let calibration = pool.calibration();
    assert_eq!(calibration.executor.uncollected, 0);
    assert_eq!(calibration.executor.queued, 0);
    let completed = collector_test_support::with_allocations_denied(|| unsafe {
        finish_acquisition(&shared, &mut batch, pending)
    });
    assert!(matches!(
        completed.report,
        Err(acquisition::Failure::Stopped)
    ));
    let acquisition::Job::DivisionInputs(request) = completed.request else {
        panic!("parked division input owner lost");
    };
    assert!(engine_allocation::SharedBox::ptr_eq(
        &owner,
        request.payload.census.unit_inputs().unwrap()
    ));
    let counters =
        pool.calibration().roles[collector_workers::roles::Role::DivisionInputs as usize].counters;
    assert_eq!(counters.rejected, 1);
    assert_eq!(counters.inline_completed, 0);
}

#[test]
fn rejected_air_missions_reuses_the_heap_payload_and_publishes_named_first_hour_refusals() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let mut captured = extended::Captured::with_capacities(&extended::Capacities::default());
    let countries = engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap();
    for hour in [3, 17] {
        captured.air_wing_pilot().prepare_hour(hour, true);
        let request = captured.take_air_missions_request(&countries, 0, 0, None);
        let owner = &*request.pack as *const _;
        let pending = park(&mut batch, acquisition::Job::AirMissions(request));
        let completed = collector_test_support::with_allocations_denied(|| unsafe {
            finish_acquisition(&shared, &mut batch, pending)
        });
        assert!(matches!(
            completed.report,
            Ok(acquisition::Report::AirMissions(_))
        ));
        assert_eq!(
            pool.calibration().roles[collector_workers::roles::Role::AirMissions as usize]
                .counters
                .inline_completed,
            0
        );
        #[cfg(feature = "acquisition_profile")]
        assert_eq!(
            completed.table_reads,
            collector_memory::read_accounting::ZERO
        );
        let acquisition::Job::AirMissions(request) = completed.request else {
            panic!("Air mission payload lost")
        };
        assert_eq!(&*request.pack as *const _, owner);
        captured.restore_air_missions_request(request, false);
        let tables = captured.air_wing_pilot().tables();
        let mut source = replay_frame::frame::DecodedFrame {
            branch: 1,
            hour,
            tables: std::array::from_fn(|_| {
                replay_frame::frame::TableView::Unavailable {
                    reason: "outside Air mission rejection fixture".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        for (table, view) in collector_air::extended::air::TABLES.into_iter().zip(tables) {
            if let replay_frame::frame::TableView::Unavailable { reason } = &view {
                assert!(!reason.is_empty());
            }
            *source.tables[table.index()] = view;
        }
        replay_frame::frame::FrameEncoder::new()
            .encode(source, true)
            .unwrap();
    }
}

#[test]
fn a_rejected_presentation_fails_beside_a_completed_movement_pass() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let mut request = acquisition::PresentationRequest {
        dispatch: Default::default(),
        countries: engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap(),
        context: collector_divisions::extended::division_presentation::CaptureContext {
            base: 0,
            game_state: 0,
            hour: 0,
        },
        captured: collector_divisions::extended::division_presentation::Captured::vacant(),
        inputs: None,
        army_stats: None,
    };
    let movement = request.take_movement();
    let pending =
        unsafe { submit_acquisition(&shared, &mut batch, acquisition::Job::Presentation(request)) };
    assert!(matches!(pending, PendingAcquisition::Rejected(_)));
    let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
    let acquisition::Job::Presentation(mut request) = completed.request else {
        panic!("parked presentation lost");
    };
    let report = completed.report.map(|report| {
        let acquisition::Report::Presentation(report) = report else {
            panic!("Presentation report type")
        };
        report
    });
    assert_eq!(
        request
            .restore_movement(
                movement,
                Ok(
                    collector_divisions::extended::division_presentation::CaptureReport {
                        ticks: [1, 3, 0, 2, 0, 0, 0, 5, 7, 11, 13, 17],
                    }
                ),
                report,
            )
            .err(),
        Some(acquisition::Failure::Stopped)
    );
}

#[test]
fn a_rejected_fronts_job_returns_its_part_and_keeps_the_orders_failure() {
    use collector_plans::extended::plans::{Capacities, Captured};
    use engine_native::walk::Outcome;
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    for (orders_failure, reason) in [
        (
            Some("army order acquisition failed"),
            "army order acquisition failed",
        ),
        (
            Some("army order worker admission failed"),
            "army order worker admission failed",
        ),
        (None, "front acquisition failed"),
    ] {
        let mut plans = Captured::with_capacities(&Capacities::default());
        if let Some(failure) = orders_failure {
            plans.fail(failure);
        }
        let fronts = plans.take_fronts();
        let bytes = fronts.allocated_bytes();
        let request = acquisition::Job::Fronts(acquisition::FrontsRequest { base: 0, fronts });
        let pending = unsafe { submit_acquisition(&shared, &mut batch, request) };
        assert!(matches!(pending, PendingAcquisition::Rejected(_)));
        let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
        assert!(matches!(
            completed.report,
            Err(acquisition::Failure::Stopped)
        ));
        let acquisition::Job::Fronts(request) = completed.request else {
            panic!("rejected fronts part lost");
        };
        assert_eq!(request.fronts.allocated_bytes(), bytes);
        plans.install_fronts(request.fronts, completed.report.is_ok());
        assert_eq!(plans.outcomes()[2], Outcome::Unavailable { reason });
    }
}

#[test]
fn a_rejected_order_part_returns_its_part_with_the_admission_reason() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let part = Box::new(collector_divisions::orders_parts::Part::new());
    let at: *const collector_divisions::orders_parts::Part = &*part;
    let request = acquisition::Job::OrdersPart(acquisition::OrdersPartRequest {
        base: 0,
        index: 2,
        part,
        rejected: false,
    });
    let pending = unsafe { submit_acquisition(&shared, &mut batch, request) };
    assert!(matches!(pending, PendingAcquisition::Rejected(_)));
    let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
    assert!(completed.report.is_ok());
    let acquisition::Job::OrdersPart(request) = completed.request else {
        panic!("rejected order part lost");
    };
    assert!(std::ptr::eq(&*request.part, at));
    assert_eq!(request.index, 2);
    assert_eq!(
        request.failure(completed.report.is_err()),
        Some("army order worker admission failed")
    );
    let ran = acquisition::OrdersPartRequest {
        rejected: false,
        ..request
    };
    assert_eq!(ran.failure(true), Some("army order acquisition failed"));
    assert_eq!(ran.failure(false), None);
}

#[test]
fn a_rejected_stitch_keeps_an_earlier_order_failure() {
    use collector_plans::extended::plans::{Capacities, Captured};
    use engine_native::walk::Outcome;
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    for (earlier, reason) in [
        (
            Some("army order acquisition failed"),
            "army order acquisition failed",
        ),
        (None, "army order worker admission failed"),
    ] {
        let mut request = acquisition::OrdersRequest {
            countries: engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap(),
            base: 0,
            game_state: 0,
            // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
            orders: unsafe {
                collector_map_capture::map_capture::MapCapacities::default().orders()
            },
            plans: Captured::with_capacities(&Capacities::default()),
            group_source: None,
        };
        if let Some(reason) = earlier {
            request.fail(reason);
        }
        let pending = unsafe {
            submit_acquisition(&shared, &mut batch, acquisition::Job::OrdersStitch(request))
        };
        assert!(matches!(pending, PendingAcquisition::Rejected(_)));
        let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
        let acquisition::Job::OrdersStitch(request) = completed.request else {
            panic!("rejected stitch lost");
        };
        assert_eq!(
            request.orders.rows.outcome,
            Outcome::Unavailable { reason },
            "{earlier:?}"
        );
        assert_eq!(
            request.plans.outcomes()[0],
            Outcome::Unavailable { reason },
            "{earlier:?}"
        );
    }
}

fn stitch_request(base: usize, earlier: Option<&'static str>) -> acquisition::OrdersRequest {
    use collector_plans::extended::plans::{Capacities, Captured};
    let mut request = acquisition::OrdersRequest {
        countries: engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap(),
        base,
        game_state: 0,
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        orders: unsafe { collector_map_capture::map_capture::MapCapacities::default().orders() },
        plans: Captured::with_capacities(&Capacities::default()),
        group_source: None,
    };
    if let Some(reason) = earlier {
        request.fail(reason);
    }
    request
}

#[test]
fn a_stitch_panic_stops_the_pool_only_at_the_stitch_join() {
    use engine_native::walk::Outcome;
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let job =
        || acquisition::Job::OrdersStitch(stitch_request(acquisition::STITCH_PANIC_BASE, None));
    let pending = unsafe { submit_acquisition(&shared, &mut batch, job()) };
    assert!(matches!(pending, PendingAcquisition::Accepted(_)));
    batch.wait_ready(|batch| pending_complete(batch, &pending).then_some(()));
    assert!(!stitch_can_advance(&batch, &pending, false));
    assert!(stitch_can_advance(&batch, &pending, true));
    let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
    let Ok(acquisition::Report::OrdersStitch(report)) = completed.report else {
        panic!("a contained stitch panic is a report");
    };
    assert!(report.contained_panic);
    assert!(!batch.stopped(), "the panic did not stop the pool");
    let later = unsafe { submit_acquisition(&shared, &mut batch, job()) };
    assert!(matches!(later, PendingAcquisition::Accepted(_)));
    drop(unsafe { finish_acquisition(&shared, &mut batch, later) });
    let acquisition::Job::OrdersStitch(mut request) = completed.request else {
        panic!("stitch request lost");
    };
    settle_stitch(&batch, &mut request, report, None, true);
    assert!(batch.stopped(), "the join applies the stop");
    let reason = "army order acquisition failed";
    assert_eq!(request.orders.rows.outcome, Outcome::Unavailable { reason });
    assert_eq!(request.plans.outcomes()[0], Outcome::Unavailable { reason });
    let calibration = pool.calibration();
    assert_eq!(calibration.executor.panicked, 1);
    assert_eq!(
        calibration.roles[collector_workers::roles::Role::OrdersStitch as usize]
            .counters
            .panicked,
        1
    );
    let refused = unsafe { submit_acquisition(&shared, &mut batch, job()) };
    assert!(matches!(refused, PendingAcquisition::Rejected(_)));
    drop(unsafe { finish_acquisition(&shared, &mut batch, refused) });
}

#[test]
fn readiness_releases_clean_stitches_but_defers_rejected_stitches() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let job = acquisition::Job::OrdersStitch(stitch_request(0, Some("original census refusal")));
    let pending = unsafe { submit_acquisition(&shared, &mut batch, job) };
    batch.wait_ready(|batch| stitch_can_advance(batch, &pending, false).then_some(()));
    let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
    let acquisition::Job::OrdersStitch(request) = completed.request else {
        unreachable!()
    };
    assert_eq!(
        request.orders.rows.outcome,
        Outcome::Unavailable {
            reason: "original census refusal"
        }
    );
    drop(request);
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let pending = unsafe {
        submit_acquisition(
            &shared,
            &mut batch,
            acquisition::Job::OrdersStitch(stitch_request(0, None)),
        )
    };
    assert!(matches!(pending, PendingAcquisition::Rejected(_)));
    assert!(!stitch_can_advance(&batch, &pending, false));
    assert!(stitch_can_advance(&batch, &pending, true));
    let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
    let acquisition::Job::OrdersStitch(request) = completed.request else {
        unreachable!()
    };
    assert_eq!(
        request.orders.rows.outcome,
        Outcome::Unavailable {
            reason: "army order worker admission failed"
        }
    );
}

#[test]
fn final_inputs_wait_only_for_their_producers_and_prior_dispatch_admission() {
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let ticket = batch
        .park(&mut Some(acquisition::Job::OrdersStitch(stitch_request(
            0, None,
        ))))
        .unwrap();
    let unpublished = PendingAcquisition::Accepted(ticket);
    let rejected = PendingAcquisition::Rejected(ticket);
    let producers = 4 + CHARACTER_PARTS - 1;
    collector_test_support::with_allocations_denied(|| {
        assert!(!final_inputs_ready(&batch, false, [None]));
        assert!(final_inputs_ready(&batch, true, [None]));
        for missing in 0..producers {
            assert!(!final_inputs_ready(
                &batch,
                true,
                (0..producers).map(|index| Some(if index == missing {
                    &unpublished
                } else {
                    &rejected
                })),
            ));
        }
        assert!(final_inputs_ready(
            &batch,
            true,
            (0..producers).map(|_| Some(&rejected)),
        ));
        assert!(!final_inputs_ready(
            &batch,
            false,
            (0..producers).map(|_| Some(&rejected)),
        ));
    });
    drop(batch.unpark(ticket).unwrap());
}

#[test]
fn a_drained_stitch_panic_is_counted_and_stops_the_pool() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    for stopped_first in [false, true] {
        let pool =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { acquisition::Pool::start(2, collector_workers::roles::MAX_JOBS) }.unwrap();
        {
            let mut batch = acquisition::Batch::new(&pool);
            let job = acquisition::Job::OrdersStitch(stitch_request(
                acquisition::STITCH_PANIC_BASE,
                None,
            ));
            let PendingAcquisition::Accepted(ticket) =
                (unsafe { submit_acquisition(&shared, &mut batch, job) })
            else {
                panic!("a running pool admits the stitch");
            };
            let started = std::time::Instant::now();
            while !batch.is_complete(ticket) {
                assert!(started.elapsed() < std::time::Duration::from_secs(10));
                std::thread::yield_now();
            }
            assert!(!batch.stopped());
            if stopped_first {
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { pool.shutdown() }.unwrap();
            }
        }
        let calibration = pool.calibration();
        assert_eq!(
            calibration.executor.panicked,
            u64::from(!stopped_first),
            "{stopped_first}"
        );
        assert_eq!(
            calibration.roles[collector_workers::roles::Role::OrdersStitch as usize]
                .counters
                .panicked,
            u64::from(!stopped_first)
        );
        assert!(pool.stopped());
    }
}

#[test]
fn a_stitch_settles_with_the_reason_of_its_join_segment() {
    use engine_native::walk::Outcome;
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let batch = acquisition::Batch::new(&pool);
    let stopped = Some("acquisition worker stopped");
    let panicked = acquisition::StitchReport {
        contained_panic: true,
    };
    let clean = acquisition::StitchReport::default();
    let admission = Some("army order worker admission failed");
    let acquisition_failed = Some("army order acquisition failed");
    let earlier = Some("army order part did not run");
    for (report, failure, admissible, prior, expected) in [
        (clean, stopped, false, None, admission),
        (panicked, None, false, None, admission),
        (clean, None, false, None, None),
        (clean, stopped, true, None, acquisition_failed),
        (clean, stopped, false, earlier, earlier),
        (panicked, None, false, earlier, earlier),
    ] {
        let mut request = stitch_request(0, prior);
        settle_stitch(&batch, &mut request, report, failure, admissible);
        let outcome = request.orders.rows.outcome;
        match expected {
            Some(reason) => assert_eq!(outcome, Outcome::Unavailable { reason }),
            None => assert!(!request.orders.failed(), "{outcome:?}"),
        }
    }
    assert_eq!(pool.calibration().executor.panicked, 0);
}

#[test]
fn the_callback_carries_the_order_split_to_the_next_hour_on_any_frame() {
    use collector_divisions::orders::Orders;
    use collector_divisions::orders_parts::{MAX_PARTS, Part};
    use collector_extended_support::bench_world::{self, Shape};
    use collector_map_capture::map_capture::MapCapacities;
    let world = bench_world::build(
        40,
        Shape {
            named: true,
            theatre_links: true,
        },
    );
    let memory = world.slice();
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 10_000_000) };
    let hour = |orders: &mut Orders, millis: u64, fail: bool| {
        unsafe { begin_order_parts(&shared, orders) };
        orders.walk_roots(&memory, world.base);
        orders.prepare_parts();
        let count = orders.part_count();
        for index in 0..count {
            let mut part = orders.take_part(index).unwrap();
            part.run(&memory, world.base);
            part.census_ticks = if index == 0 {
                shared.frequency * millis / 1_000
            } else {
                0
            };
            part.plan_ticks = 0;
            orders.return_part(index, part);
        }
        if fail {
            orders.fail("army order acquisition failed");
        } else {
            orders.stitch();
            assert!(!orders.failed());
            orders.plan_split();
        }
        unsafe { end_order_parts(&shared, orders) };
        count
    };
    // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
    let mut frames = unsafe {
        [
            MapCapacities::default().orders(),
            MapCapacities::default().orders(),
            MapCapacities::default().orders(),
        ]
    };
    assert_eq!(hour(&mut frames[0], 3, false), 1);
    assert_eq!(hour(&mut frames[1], 0, true), MAX_PARTS);
    assert!(frames[1].parts_allocated_bytes() >= MAX_PARTS * size_of::<Part>());
    assert_eq!(hour(&mut frames[0], 3, false), 1);
    assert_eq!(hour(&mut frames[2], 0, false), MAX_PARTS);
}

#[test]
fn failed_division_inputs_keep_the_original_roster_fault_through_core_and_decode() {
    use super::divisions_test_kit as kit;
    struct FirstRosterFault<'a> {
        bytes: &'a [u8],
        attempts: std::cell::Cell<usize>,
    }
    impl engine_native::Memory for FirstRosterFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == kit::C0 + ccountry::DIVISIONS {
                self.attempts.set(self.attempts.get() + 1);
                if self.attempts.get() == 1 {
                    return false;
                }
            }
            kit::memory(self.bytes).copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let memory = FirstRosterFault {
        bytes: &bytes,
        attempts: Default::default(),
    };
    let context = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    let mut job = DivisionsJob::new_worker();
    assert!(copy_divisions_inputs_refresh(&memory, context, &mut job));
    let reason = "division roster unreadable or invalid";
    let unavailable = Outcome::Unavailable { reason };
    let owner = engine_allocation::SharedBox::clone(job.census.unit_inputs().unwrap());
    assert_eq!(owner.outcome(), unavailable);
    assert_eq!(owner.intel_sources().outcome, unavailable);
    assert_eq!(memory.attempts.get(), 1);
    let report = copy_divisions_core_refresh(&NoDivisionReads, context, &mut job);
    assert!(report.raw_ok);
    assert_eq!(job.source_outcome(), unavailable);
    assert_eq!(job.details.outcomes(), [unavailable; 5]);
    assert!(engine_allocation::SharedBox::ptr_eq(
        &owner,
        job.census.unit_inputs().unwrap()
    ));
    let repeated = copy_divisions_core_refresh(&NoDivisionReads, context, &mut job);
    assert_eq!(repeated.copy_ticks, report.copy_ticks);
    assert_eq!(memory.attempts.get(), 1);
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    decode_divisions_payload(&mut frame, kit::BASE, &mut job, &mut [0; 6]);
    assert_eq!(frame.divisions_result, unavailable);
    for table in crate::decode::DIVISIONS.into_iter().skip(1) {
        assert_eq!(frame.extended.outcomes()[table.index()], unavailable);
    }
    assert_eq!(memory.attempts.get(), 1);
}

#[test]
fn missing_and_short_original_unit_packets_refuse_every_dependent_worker() {
    use super::divisions_test_kit as kit;
    use collector_divisions::division_census::UnitInputs;
    let bytes = kit::fixture();
    let memory = NoRosterRetry(&bytes);
    let countries = engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap();
    let seed = carmy::DivisionSeed {
        pointer: kit::A1,
        roster_owner: Some(1),
        ..Default::default()
    };
    for inputs in [
        None,
        Some(
            engine_allocation::shared(UnitInputs::fixture(vec![seed], Outcome::Rows { len: 2 }))
                .unwrap(),
        ),
        Some(
            engine_allocation::shared(UnitInputs::fixture(
                vec![seed],
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 1,
                },
            ))
            .unwrap(),
        ),
        Some(
            engine_allocation::shared(UnitInputs::fixture(
                Vec::new(),
                Outcome::Unavailable {
                    reason: "original division input read failed",
                },
            ))
            .unwrap(),
        ),
    ] {
        let mut request = acquisition::PresentationRequest {
            dispatch: Default::default(),
            countries: engine_allocation::SharedBox::clone(&countries),
            context: collector_divisions::extended::division_presentation::CaptureContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour: 0,
            },
            captured: collector_divisions::extended::division_presentation::Captured::vacant(),
            inputs: inputs.clone(),
            army_stats: None,
        };
        let mut movement = acquisition::Job::PresentationMovement(request.take_movement());
        let mut presentation = acquisition::Job::Presentation(request);
        let acquisition::Report::Presentation(report) =
            acquisition::run_for_test(&mut presentation, &memory)
        else {
            panic!("Presentation report type")
        };
        let acquisition::Report::PresentationMovement(movement_ticks) =
            acquisition::run_for_test(&mut movement, &memory)
        else {
            panic!("PresentationMovement report type")
        };
        let acquisition::Job::Presentation(mut presentation) = presentation else {
            unreachable!()
        };
        let acquisition::Job::PresentationMovement(movement) = movement else {
            unreachable!()
        };
        assert!(
            presentation
                .restore_movement(movement, Ok(movement_ticks), Ok(report))
                .is_ok()
        );
        assert!(
            presentation
                .captured
                .outcomes()
                .iter()
                .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );

        let mut intel = acquisition::Job::Intel(acquisition::IntelRequest {
            countries: engine_allocation::SharedBox::clone(&countries),
            context: collector_air::extended::fog::IntelCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                row_limit: 0,
            },
            payload: Default::default(),
            inputs: inputs.clone(),
            input_failure: None,
        });
        assert!(matches!(
            acquisition::run_for_test(&mut intel, &memory),
            acquisition::Report::Intel(_)
        ));
        let acquisition::Job::Intel(intel) = intel else {
            unreachable!()
        };
        let mut fog = collector_air::extended::fog::Captured::vacant();
        fog.decode_intel_payload(&intel.payload, &mut [0; 1]);
        assert!(matches!(fog.outcomes()[4], Outcome::Unavailable { .. }));
    }
}

#[test]
fn missing_and_short_owned_division_source_packets_refuse_final_equipment_consumers() {
    use super::divisions_test_kit as kit;
    use collector_new_divisions::new_divisions::{Counters, Mark};
    let bytes = kit::fixture();
    let context = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    let countries = engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap();
    let hour = replay_data::cadence::refresh_slot(TableId::DivisionEquipment).unwrap() as i32;
    assert!(!replay_data::cadence::should_walk(
        TableId::CountryFuel,
        hour,
        false
    ));
    for displaced_length in [0, 1] {
        let mut job = DivisionsJob::new_worker();
        assert!(copy_divisions_inputs_refresh(
            &kit::memory(&bytes),
            context,
            &mut job
        ));
        assert!(copy_divisions_core_refresh(&kit::memory(&bytes), context, &mut job).raw_ok);
        assert_eq!(job.source_outcome(), Outcome::Rows { len: 3 });
        let mut sink = extended::Captured::with_capacities(&extended::Capacities {
            division_sources: displaced_length,
            ..Default::default()
        });
        job.supply_sources(&mut sink, None);
        let mut frame =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Captured::new() };
        job.supply_sources(&mut frame.extended, None);
        frame.extended.set_new_division_mark(Mark::Above(Counters {
            unit: 0,
            registry: 0,
        }));
        let mut request = frame.extended.take_final_inputs_request(
            &countries,
            kit::BASE,
            kit::GAME_STATE,
            hour,
            false,
        );
        let report = request.capture(&NoRosterRetry(&bytes));
        frame
            .extended
            .restore_final_inputs_request(request, report, None);
        let unavailable = Outcome::Unavailable {
            reason: "division source packet incomplete",
        };
        for table in [
            TableId::NewDivisionEquipment,
            TableId::DivisionEquipment,
            TableId::ArmyUpgradeGroups,
            TableId::ArmyUpgradeRequests,
            TableId::ArmyUpgradeDeliveries,
            TableId::ArmyUpgradeProduced,
        ] {
            assert_eq!(frame.extended.outcomes()[table.index()], unavailable);
        }
    }
}

#[test]
fn rejected_allocating_worker_preserves_payload_without_callback_reads() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let mut selection = collector_certificates::slicing::SliceSelection::new();
    selection.reset(5, false);
    let mut payload = collector_intel::intel::RelationsJob::new();
    payload.selection = Some(selection);
    payload.fingerprints = Some(Box::new(
        collector_certificates::slicing::SliceFingerprints::new(),
    ));
    let store = payload.fingerprints.as_deref().unwrap() as *const _;
    let request = acquisition::Job::Relations(acquisition::RawRequest {
        countries: engine_allocation::shared(Cache::try_with_capacity(1).unwrap()).unwrap(),
        context: collector_intel::intel::RelationsCopyContext {
            base: 0,
            game_state: usize::MAX,
            active: [true; 2],
            epoch: 0,
        },
        payload,
    });
    let rejected = park(&mut batch, request);
    let reads = collector_memory::read_accounting::Capture::new();
    let completed = collector_test_support::with_allocations_denied(|| unsafe {
        finish_acquisition(&shared, &mut batch, rejected)
    });
    assert!(matches!(
        completed.report,
        Err(acquisition::Failure::Stopped)
    ));
    #[cfg(feature = "acquisition_profile")]
    assert_eq!(
        completed.table_reads,
        collector_memory::read_accounting::ZERO
    );
    assert_eq!(reads.finish(), collector_memory::read_accounting::ZERO);
    let acquisition::Job::Relations(request) = completed.request else {
        panic!("rejected worker lost its payload");
    };
    let mut captured = collector_intel::intel::Captured::vacant();
    let (returned, payload) = captured.join_relations(
        request.payload,
        Default::default(),
        Some(acquisition_failure_reason(acquisition::Failure::Stopped)),
    );
    assert_eq!(
        returned.as_deref().map(|value| value as *const _),
        Some(store)
    );
    assert!(payload.is_err());
    assert!(captured.relations_raw_mode);
    assert_eq!(pool.calibration().executor.uncollected, 0);
}

#[test]
fn failed_division_job_publishes_hourly_census_reason_even_without_details() {
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    for details_active in [false, true] {
        frame.extended.reset();
        let before = frame.extended.outcomes();
        let reason = "accepted division acquisition panicked";
        fail_divisions_capture(&mut frame, reason, details_active);
        let unavailable = Outcome::Unavailable { reason };
        assert_eq!(frame.divisions_result, unavailable);
        assert_eq!(
            frame.extended.outcomes()[replay_data::rows::TableId::DivisionCensus.index()],
            unavailable
        );
        for table in [
            replay_data::rows::TableId::DivisionEquipment,
            replay_data::rows::TableId::ArmyUpgradeGroups,
            replay_data::rows::TableId::ArmyUpgradeRequests,
            replay_data::rows::TableId::ArmyUpgradeDeliveries,
            replay_data::rows::TableId::ArmyUpgradeProduced,
        ] {
            assert_eq!(
                frame.extended.outcomes()[table.index()],
                if details_active {
                    unavailable
                } else {
                    before[table.index()]
                }
            );
        }
    }
}

#[test]
fn the_daily_hour_stores_both_id_counters_and_later_hours_compare_by_handle_type() {
    use collector_new_divisions::new_divisions::{Counters, Mark};
    struct Counterset(i32, u32);
    impl Memory for Counterset {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let bytes = if address == engine_native::cship::NEXT_ID_RVA.0 {
                self.0.to_le_bytes()
            } else if address == engine_native::creference_registry::ID_COUNTER_RVA.0 {
                self.1.to_le_bytes()
            } else {
                return false;
            };
            if output.len() != 4 {
                return false;
            }
            output.copy_from_slice(&bytes);
            true
        }
    }
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    assert_eq!(
        new_division_mark(&shared, &Counterset(9, 9), false),
        Mark::Unknown
    );
    assert_eq!(
        new_division_mark(&shared, &Counterset(40, 7_000), true),
        Mark::Daily
    );
    let Mark::Above(counters) = new_division_mark(&shared, &Counterset(99, 99), false) else {
        panic!("a daily walk stores its counters");
    };
    assert_eq!(
        counters,
        Counters {
            unit: 40,
            registry: 7_000
        }
    );
    assert!(counters.minted_after((7_001 << 32) | 0x1268));
    assert!(!counters.minted_after((41 << 32) | 0x1268));
    assert!(!counters.minted_after((39 << 32) | u64::from(engine_native::cship::HANDLE_TYPE)));
    assert!(counters.minted_after((40 << 32) | u64::from(engine_native::cship::HANDLE_TYPE)));
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    new_division_mark(&shared, &Unreadable, true);
    assert_eq!(
        new_division_mark(&shared, &Counterset(1, 1), false),
        Mark::Unknown
    );
}

#[test]
fn divisions_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
    use collector_primitives::raw_memory::{Capacity, Charge, Tracker};
    static LOCAL: Tracker = Tracker::new();
    let mut job = DivisionsJob {
        capacity_charge: Charge::empty_on(&LOCAL),
        ..Default::default()
    };
    job.census.reserve_sources(1);
    job.update_capacity_charge();
    let expected =
        |job: &DivisionsJob| job.census.allocated_bytes() + job.details.allocated_bytes() as u64;
    let initial = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(initial, expected(&job));
    job.grow();
    let grown = LOCAL.snapshot().current.auxiliary_vectors;
    assert!(grown > initial);
    assert_eq!(grown, expected(&job));
    assert!(
        job.details
            .ready_for([0, job.census.source_requirement(), 0, 0, 0])
    );
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    let frame_before = frame.allocated_bytes();
    let payload_before = expected(&job);
    let detail_bytes = job.details.allocated_bytes() as u64;
    assert!(
        detail_bytes
            >= job.census.source_requirement() as u64
                * size_of::<engine_deployment::carmy_upgrades_requests::Row>() as u64
    );
    decode_divisions_payload(&mut frame, 0, &mut job, &mut [0; 6]);
    let payload_after = expected(&job);
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, payload_after);
    assert_eq!(
        frame.allocated_bytes() as u64 + payload_after - frame_before as u64 - payload_before,
        detail_bytes
    );
    job.clear();
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, payload_after);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}

#[test]
fn callback_time_waits_for_decode_and_includes_the_game_thread_tail() {
    let base = AtomicI64::new(0);
    let tail = AtomicI64::new(0);
    assert_eq!(completed_callback_ticks(&base, &tail), None);
    base.store(17, Ordering::Release);
    assert_eq!(completed_callback_ticks(&base, &tail), None);
    tail.store(3, Ordering::Release);
    assert_eq!(completed_callback_ticks(&base, &tail), Some(20));
    base.store(0, Ordering::Release);
    assert_eq!(completed_callback_ticks(&base, &tail), None);
    base.store(23, Ordering::Release);
    assert_eq!(completed_callback_ticks(&base, &tail), Some(26));
}

#[test]
fn a_dropped_frame_invalidates_its_sliced_reads_and_a_clockless_one_does_not() {
    use collector_certificates::slicing::{RESEARCH, SliceSelection};
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let store = || {
        unsafe { &mut *shared.slice_fingerprints[RESEARCH].get() }
            .as_mut()
            .unwrap()
    };
    let owners = [(7, Some(1)), (9, Some(1))];
    let early_at = |hour: i32| {
        let mut next = SliceSelection::for_test(hour, false, &[]);
        for (owner, value) in owners {
            next.scan(store(), owner, value);
        }
        store().finish_scan();
        [7, 9].map(|owner| next.contains(owner))
    };
    early_at(0);
    assert_eq!(early_at(1), [false, false]);
    for (clock, expected) in [(Some(clock(7, 1)), [true, false]), (None, [false, false])] {
        let index =
        // SAFETY: This consumer owns the queue removal for the named pool.
        unsafe { shared.frames.pop() }.expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.extended.prepare_hour(7, false);
        frame.clock = clock;
        drop_owned_frame(&shared, frame, index);
        assert_eq!(
            early_at(8),
            expected,
            "clock {:?}",
            frame.clock.as_ref().map(|c| c.hour)
        );
    }
}

#[test]
fn a_family_the_spool_could_not_publish_is_read_in_full_next_hour() {
    use collector_certificates::slicing::{RAID_STATE, RESEARCH, SliceSelection};
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let store = |family: usize| {
        unsafe { &mut *shared.slice_fingerprints[family].get() }
            .as_mut()
            .unwrap()
    };
    let early_at = |family: usize, hour: i32| {
        let mut next = SliceSelection::for_test(hour, false, &[]);
        for owner in [7, 9] {
            next.scan(store(family), owner, Some(1));
        }
        store(family).finish_scan();
        next.early_count
    };
    for family in [RESEARCH, RAID_STATE] {
        early_at(family, 0);
        assert_eq!(early_at(family, 1), 0);
    }
    shared
        .forget_families
        .fetch_or(1 << RESEARCH, Ordering::Release);
    forget_unpublished_families(&shared);
    assert_eq!(early_at(RESEARCH, 2), 2);
    assert_eq!(early_at(RAID_STATE, 2), 0);
    forget_unpublished_families(&shared);
    assert_eq!(early_at(RESEARCH, 3), 0);
}

#[test]
fn a_refused_hand_off_records_its_hour_and_returns_the_frame() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let index =
    // SAFETY: This consumer owns the queue removal for the named pool.
    unsafe { shared.frames.pop() }.expect("pooled frame");
    shared.frame_priority.claim();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    frame.clock = Some(clock(42, 7));
    let dropped_before = shared.dropped.load(Ordering::Relaxed);
    drop_owned_frame(&shared, frame, index);
    assert_eq!(shared.dropped.load(Ordering::Relaxed), dropped_before + 1);
    assert!(shared.frames.ready_is_empty());
    assert_eq!(shared.frames.len(), BUFFERS);
    let pending = unsafe { &*shared.pending.get() };
    let runs: Vec<_> = pending.iter().collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        (runs[0].first_hour, runs[0].last_hour, runs[0].count),
        (42, 42, 1)
    );
    assert_eq!(runs[0].loaded_hour, Some(42));
}

#[test]
fn pool_initialization_allocates_all_slots_and_counts_actual_bytes() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    assert_eq!(shared.frames.len(), BUFFERS);
    assert_eq!(shared.frames.live(), BUFFERS);
    let mut bytes = engine_allocation::SharedBox::<AtomicU64>::BYTES;
    bytes += unsafe { (&*shared.roster.get()).as_ref().unwrap() }.allocated_bytes();
    bytes += unsafe { &*shared.country_cache.get() }.allocated_bytes();
    bytes += shared.country_intel.allocated_bytes();
    bytes += unsafe { (&*shared.raid_air_scores.get()).as_ref().unwrap() }.allocated_bytes();
    bytes += unsafe { &*shared.fixed_layout_buffer.get() }.capacity();
    bytes += engine_allocation::SharedBox::<collector_workers::frame_priority::Pipeline>::BYTES;
    bytes += shared.frames.slot_bytes() as usize;
    bytes += engine_allocation::SharedBox::<crate::frame_capacities::Capacities>::BYTES;
    for slot in shared.frames.slots() {
        let frame = slot.frame.load(Ordering::Acquire);
        assert!(!frame.is_null());
        let frame = unsafe { &*frame };
        let home = frame.extended.country_intel_home().unwrap();
        assert!(home.shares(&shared.country_intel));
        assert!(engine_allocation::SharedBox::ptr_eq(
            &frame.extended.template_home().unwrap(),
            &shared.template_home
        ));
        assert!(engine_allocation::SharedBox::ptr_eq(
            &frame.extended.naval_combat_home().unwrap(),
            &shared.naval_combat_home
        ));
        bytes += frame.allocated_bytes();
    }
    assert_eq!(shared.pool_owned(), bytes as u64);
}

#[test]
fn a_stalled_spool_never_holds_the_game_and_the_pool_returns_to_its_steady_frames() {
    use std::{collections::HashSet, sync::mpsc};
    let shared = Arc::new(
        // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
        unsafe { Shared::new(0, 0, 1) },
    );
    let hours = 3 * BUFFERS + 1;
    let (send, taken) = mpsc::channel();
    let producer = {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            for hour in 0..hours {
                let mut grow_ns = u64::MAX;
                let index =
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { shared
                    .take_frame(&mut grow_ns) }
                    .expect("a frame for every hour");
                unsafe { (*shared.frames[index].load(Ordering::Acquire)).qpc = hour as i64 };
                // SAFETY: The producer owns this slot and its initialized decode job.
                unsafe { shared.frames.push_decode(index) };
                send.send((index, grow_ns)).unwrap();
            }
        })
    };
    let mut indices = Vec::new();
    for hour in 0..hours {
        let (index, grow_ns) = taken
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap_or_else(|_| panic!("the game thread waited for a frame at hour {hour}"));
        assert_eq!(
            grow_ns == 0,
            hour < BUFFERS,
            "hour {hour} grew {grow_ns} ns"
        );
        indices.push(index);
    }
    producer.join().unwrap();
    assert_eq!(indices.iter().collect::<HashSet<_>>().len(), hours);
    let backlog = shared.frames.gauge();
    assert_eq!(backlog.live, hours);
    assert_eq!(backlog.in_flight, hours);
    assert_eq!(backlog.grown, (hours - BUFFERS) as u64);
    assert!(backlog.in_flight_bytes > 0);
    for (hour, &expected) in indices.iter().enumerate() {
        let index =
        // SAFETY: The sole decode consumer owns this queue and takes responsibility for the slot.
        unsafe { shared.frames.pop_decode() }.expect("every queued hour");
        assert_eq!(index, expected, "hour {hour} out of order");
        assert_eq!(
            unsafe { (*shared.frames[index].load(Ordering::Acquire)).qpc },
            hour as i64
        );
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame(index) };
    }
    assert_eq!(
        // SAFETY: The sole decode consumer owns this queue and takes responsibility for the slot.
        unsafe { shared.frames.pop_decode() },
        None
    );
    let caught_up = shared.frames.gauge();
    assert_eq!(
        (caught_up.live, caught_up.free, caught_up.in_flight),
        (BUFFERS, BUFFERS, 0)
    );
    assert_eq!(caught_up.freed, (hours - BUFFERS) as u64);
    assert_eq!(caught_up.in_flight_bytes, 0);
    let mut grow_ns = u64::MAX;
    let index =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut grow_ns) }.unwrap();
    assert_eq!(grow_ns, 0);
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.recycle_frame(index) };
}

#[test]
fn a_slowed_builder_keeps_a_spare_ahead_so_the_game_thread_never_builds_a_frame() {
    use crate::frame_pool::{AHEAD, STEADY};
    use std::sync::mpsc;
    let shared = Arc::new(
        // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
        unsafe { Shared::new(0, 0, collector_test_support::qpc_frequency()) },
    );
    let watching = Arc::clone(&shared);
    collector_test_support::watched(
        move || format!("{:?}", watching.frames.gauge()),
        move |steps| {
            let (started, builds) = mpsc::channel::<()>();
            let (release, released) = mpsc::channel::<()>();
            let released = Mutex::new(released);
            let observe = move |at| {
                if at == BuildAhead::Starting {
                    let _ = started.send(());
                    let _ = released
                        .lock()
                        .unwrap()
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD);
                }
            };
            let gauge = || format!("{:?}", shared.frames.gauge());
            let hour = |at: usize, held: &mut Vec<usize>| {
                steps.at(format_args!("hour {at}: taking its frame; {}", gauge()));
                let mut grow_ns = u64::MAX;
                let index =
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { shared
                    .take_frame(&mut grow_ns) }
                    .unwrap_or_else(|| panic!("hour {at}: no frame: {}", gauge()));
                held.push(index);
                steps.at(format_args!("hour {at}: took frame {index}; {}", gauge()));
                grow_ns
            };
            let build_started = |at: usize| {
                steps.at(format_args!(
                    "after hour {at}: waiting for a build ahead; {}",
                    gauge()
                ));
                builds
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap_or_else(|_| {
                        panic!(
                            "no frame was being built ahead after hour {at}: {}",
                            gauge()
                        )
                    });
            };
            let mut held = Vec::new();
            let builder = TestBuilder::following(&shared, release, || {}, observe);
            for at in 1..=STEADY - AHEAD {
                assert_eq!(hour(at, &mut held), 0, "hour {at}: {}", gauge());
            }
            assert_eq!(hour(STEADY - 1, &mut held), 0, "{}", gauge());
            build_started(STEADY - 1);
            let hours = 4 * STEADY;
            for at in STEADY..=hours {
                assert_eq!(
                    hour(at, &mut held),
                    0,
                    "hour {at} built its own frame: {}",
                    gauge()
                );
                let gauge = shared.frames.gauge();
                assert_eq!(gauge.grown, 0, "hour {at}: {gauge:?}");
                assert_eq!(
                    gauge.ahead_taken,
                    (at - STEADY) as u64,
                    "hour {at}: {gauge:?}"
                );
                builder.release();
                build_started(at);
            }
            let behind = shared.frames.gauge();
            assert_eq!(
                (behind.in_flight, behind.free, behind.grown),
                (hours, 1, 0),
                "{behind:?}"
            );
            assert_eq!(
                behind.built_ahead,
                (hours - STEADY + 1) as u64,
                "{behind:?}"
            );
            assert_eq!(hour(hours + 1, &mut held), 0, "{}", gauge());
            let grew = hour(hours + 2, &mut held);
            let burst = shared.frames.gauge();
            assert_eq!(burst.grown, 1, "{burst:?}");
            assert!(grew > 0, "{burst:?}");
            assert!(
                grew < collector_test_support::TEST_HANG_GUARD.as_nanos() as u64,
                "{grew} ns: {burst:?}"
            );
            assert_eq!(
                (burst.grow_ns, burst.grow_max_ns),
                (grew, grew),
                "{burst:?}"
            );
            steps.at(format_args!("ending the builder; {}", gauge()));
            builder.end(gauge).unwrap();
            let behind = shared.frames.gauge();
            assert_eq!(behind.in_flight, held.len(), "{behind:?}");
            assert_eq!(behind.free, AHEAD - 1, "{behind:?}");
            assert_eq!(
                behind.built_ahead,
                (hours - STEADY + 2) as u64,
                "{behind:?}"
            );
            for index in held.drain(..) {
                steps.at(format_args!("recycling frame {index}; {}", gauge()));
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { shared.recycle_frame(index) };
            }
            let caught_up = shared.frames.gauge();
            assert_eq!(
                (caught_up.live, caught_up.free, caught_up.in_flight),
                (STEADY, STEADY, 0),
                "{caught_up:?}"
            );
            assert_eq!(
                caught_up.freed,
                (behind.live - STEADY) as u64,
                "{caught_up:?}"
            );
        },
    );
}

struct TestBuilder {
    stop: Arc<std::sync::atomic::AtomicBool>,
    release: Option<std::sync::mpsc::Sender<()>>,
    builder: Option<collector_test_support::Ending<std::io::Result<()>>>,
}

impl TestBuilder {
    fn following(
        shared: &Arc<Shared>,
        release: std::sync::mpsc::Sender<()>,
        follow: impl FnMut() + Send + 'static,
        observe: impl FnMut(BuildAhead) + Send + 'static,
    ) -> Self {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let builder = {
            let (shared, stop) = (Arc::clone(shared), Arc::clone(&stop));
            collector_test_support::spawn_ending(move ||
            // SAFETY: This is the session's designated builder, which the runtime joins before
            // releasing Shared.
            unsafe { shared.build_ahead_until_observing(&stop, follow, observe) })
        };
        Self {
            stop,
            release: Some(release),
            builder: Some(builder),
        }
    }

    fn release(&self) {
        if let Some(release) = &self.release {
            release.send(()).unwrap();
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        drop(self.release.take());
        if let Some(builder) = &self.builder {
            builder.thread().unpark();
        }
    }

    fn end(mut self, gauge: impl Fn() -> String) -> std::io::Result<()> {
        self.stop();
        let builder = self.builder.take().expect("a started builder");
        builder.end(|| format!("the frame builder, stopped, ({})", gauge()))
    }
}

impl Drop for TestBuilder {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn a_stopped_frame_builder_builds_nothing_more_and_ends() {
    use crate::frame_pool::STEADY;
    use std::sync::mpsc;
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Stopped {
        Parked,
        MidBuild,
        BeforeItsLook,
    }
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Seen {
        Build(BuildAhead),
        Looking,
    }
    for stopped in [Stopped::Parked, Stopped::MidBuild, Stopped::BeforeItsLook] {
        let shared = Arc::new(
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Shared::new(0, 0, collector_test_support::qpc_frequency()) },
        );
        let watching = Arc::clone(&shared);
        collector_test_support::watched(
            move || format!("{stopped:?}: {:?}", watching.frames.gauge()),
            move |steps| {
                let gauge = || format!("{stopped:?}: {:?}", shared.frames.gauge());
                let (seen, sightings) = mpsc::channel::<Seen>();
                let (release, released) = mpsc::channel::<()>();
                let released = Arc::new(Mutex::new(released));
                let hold = move || {
                    let _ = released
                        .lock()
                        .unwrap()
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD);
                };
                let observe = {
                    let (seen, hold) = (Mutex::new(seen.clone()), hold.clone());
                    move |at| {
                        let _ = seen.lock().unwrap().send(Seen::Build(at));
                        if stopped == Stopped::MidBuild && at == BuildAhead::Starting {
                            hold();
                        }
                    }
                };
                let mut looks = 0;
                let builder = TestBuilder::following(&shared, release, move || {
                    looks += 1;
                    if looks > 1 && stopped == Stopped::BeforeItsLook {
                        let _ = seen.send(Seen::Looking);
                        hold();
                    }
                }, observe);
                let mut starts = 0;
                let mut wait_for = |wanted: Seen| {
                    steps.at(format_args!("waiting for {wanted:?}; {}", gauge()));
                    loop {
                        let at = sightings
                            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                            .unwrap_or_else(|_| panic!("never saw {wanted:?}: {}", gauge()));
                        starts += usize::from(at == Seen::Build(BuildAhead::Starting));
                        if at == wanted {
                            break;
                        }
                    }
                };
                wait_for(Seen::Build(BuildAhead::Parking));
                if stopped != Stopped::Parked {
                    for at in 0..STEADY {
                        steps.at(format_args!("taking frame {at}; {}", gauge()));
                        let mut grow_ns = u64::MAX;
                        // SAFETY: This fixture owns its frame and worker state and runs without a
                        // live attachment.
                        unsafe { shared.take_frame(&mut grow_ns) }.expect("a free frame");
                        assert_eq!(grow_ns, 0, "{}", gauge());
                    }
                    wait_for(match stopped {
                        Stopped::MidBuild => Seen::Build(BuildAhead::Starting),
                        _ => Seen::Looking,
                    });
                }
                steps.at(format_args!("stopping the builder; {}", gauge()));
                builder.end(gauge).unwrap();
                starts += sightings
                    .try_iter()
                    .filter(|&at| at == Seen::Build(BuildAhead::Starting))
                    .count();
                let ended = shared.frames.gauge();
                let built = usize::from(stopped == Stopped::MidBuild);
                assert_eq!(starts, built, "{stopped:?}: {ended:?}");
                assert_eq!(ended.built_ahead, built as u64, "{stopped:?}: {ended:?}");
                let free = if stopped == Stopped::Parked {
                    STEADY
                } else {
                    built
                };
                assert_eq!(ended.free, free, "{stopped:?}: {ended:?}");
                assert_eq!(ended.grown, 0, "{stopped:?}: {ended:?}");
            },
        );
    }
}

#[test]
fn acquisition_panic_stops_admission_and_preserves_losses_without_waiting() {
    use std::sync::atomic::AtomicUsize;

    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("panic payload must not be dropped across the callback boundary");
        }
    }

    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let index =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.unwrap();
    unsafe { (&mut *shared.pending.get()).push(Some(clock(40, 7).borrowed())) };
    let acquisitions = AtomicUsize::new(0);
    let completed = unsafe {
        guarded_acquisition(&shared, Some(clock(41, 7).borrowed()), || {
            acquisitions.fetch_add(1, Ordering::Relaxed);
            std::panic::panic_any(PanicOnDrop);
        })
    };
    assert!(!completed);
    assert!(shared.acquisition_failed());
    assert_eq!(shared.acquisition_panics.load(Ordering::Acquire), 1);
    assert_eq!(shared.frames.len(), BUFFERS - 1);
    assert_eq!(
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) },
        None
    );
    assert_eq!(shared.frames.live(), BUFFERS);

    assert!(!unsafe {
        guarded_acquisition(&shared, Some(clock(42, 7).borrowed()), || {
            acquisitions.fetch_add(1, Ordering::Relaxed);
        })
    });
    assert_eq!(acquisitions.load(Ordering::Relaxed), 1);
    assert_eq!(shared.acquisition_panics.load(Ordering::Acquire), 1);
    assert_eq!(shared.dropped.load(Ordering::Relaxed), 2);
    let losses = unsafe { &*shared.pending.get() };
    let run = losses.iter().next().unwrap();
    assert_eq!((run.first_hour, run.last_hour, run.count), (40, 42, 3));

    while
    // SAFETY: This consumer owns the queue removal for the named pool.
    unsafe { shared.frames.pop() }.is_some() {}
    assert_eq!(
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) },
        None
    );
    assert_eq!(
        shared.frames.slot(index).completed.load(Ordering::Acquire),
        0
    );
}

fn clock(hour: i32, signal: i64) -> Clock {
    Clock {
        game_state: 1,
        hour,
        play_time: 0,
        play_session_started_at: signal,
        seed: 1,
        campaign_id: Owned::default(),
        speed: 4,
        tick_busy: 0,
    }
}

#[test]
fn lost_ranges_preserve_reload_origins_and_never_bridge_discontinuities() {
    let mut batch = LossBatch::default();
    batch.push(Some(clock(10, 1).borrowed()));
    batch.push(Some(clock(11, 1).borrowed()));
    batch.push(Some(clock(5, 2).borrowed()));
    batch.push(Some(clock(8, 2).borrowed()));
    let runs: Vec<_> = batch.iter().collect();
    assert_eq!(runs.len(), 3);
    assert_eq!(
        (runs[0].first_hour, runs[0].last_hour, runs[0].count),
        (10, 11, 2)
    );
    assert_eq!(runs[1].loaded_hour, Some(5));
    batch.push(None);
    assert_eq!(batch.unknown_count, 1);
}

#[test]
fn losses_after_the_update_record_the_hour_it_ended_as_the_lost_and_loaded_hour() {
    assert_eq!(
        SAMPLE_BOUNDARY,
        replay_data::identity::SampleBoundary::AfterHourlyUpdate
    );
    let mut batch = LossBatch::default();
    batch.push(Some(clock(20, 3).borrowed()));
    batch.push(Some(clock(21, 3).borrowed()));
    let runs: Vec<_> = batch.iter().collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        (runs[0].first_hour, runs[0].last_hour, runs[0].loaded_hour),
        (20, 21, Some(20))
    );
}

#[test]
fn capacity_growth_keeps_primary_rows_and_name_arena_sized() {
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    let before = frame.allocated_bytes();
    let mut capacities = INITIAL_CAPACITIES;
    frame.wars_result = Outcome::Truncated {
        len: 1024,
        unread_rows: 7,
    };
    frame.factions_result = Outcome::Truncated {
        len: 64,
        unread_rows: 2,
    };
    frame.faction_members_result = Outcome::Truncated {
        len: 1024,
        unread_rows: 3,
    };
    frame.faction_names_required = 20_000;
    frame.state_names_required = 40_000;
    frame.grow(&mut capacities);
    assert_eq!(frame.wars.len(), 2048);
    assert_eq!(frame.factions.len(), 128);
    assert_eq!(frame.faction_members.len(), 2048);
    assert_eq!(frame.faction_names.len(), 32768);
    assert!(frame.state_names.len() >= 40_000);
    assert_eq!(frame.state_names.len(), capacities[11]);
    assert!(frame.allocated_bytes() > before);
    let mut idle =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    idle.grow(&mut capacities);
    assert_eq!(idle.allocated_bytes(), frame.allocated_bytes());
}

fn cadence_clock(hour: i32, signal: i64) -> Clock {
    let mut clock = clock(hour, signal);
    clock.campaign_id = Owned {
        bytes: b"campaign".to_vec(),
        len: 8,
    };
    clock
}

fn prepared_cadence() -> CadenceState {
    let mut cadence = CadenceState::default();
    cadence.prev_campaign.prepare(8).unwrap();
    cadence
}

#[test]
fn cadence_rotation_walks_recorded_families_once_per_24_hours() {
    use replay_data::rows::TableId;
    use replay_data::{cadence::Cadence, cadence::cadence, cadence::is_refresh};
    let recorded = |table: TableId| {
        table
            .last_change()
            .present(replay_data::format::FormatVersion::CURRENT.version())
    };
    for base in [i32::MIN, -25, -1, 0, 1, 60_820_531, i32::MAX - 23] {
        let mut refreshes = [0; replay_data::rows::TABLE_COUNT];
        for offset in 0..24 {
            let hour: i32 = base + offset;
            let slot = hour.rem_euclid(24);
            for table in TableId::ALL {
                let expected = match cadence(table) {
                    Cadence::Hourly | Cadence::Sliced => true,
                    Cadence::Daily(s) => s as i32 == slot,
                    Cadence::Static => false,
                };
                assert_eq!(is_refresh(table, hour), expected, "{table:?} at {hour}");
                let walk = replay_data::cadence::should_walk(table, hour, false);
                assert_eq!(
                    walk,
                    expected && recorded(table),
                    "{table:?} walk at {hour}"
                );
                assert_eq!(
                    replay_data::cadence::should_walk(table, hour, true),
                    recorded(table),
                    "{table:?} forced at {hour}"
                );
                refreshes[table.index()] += usize::from(walk);
            }
        }
        for table in TableId::ALL {
            let expected = match cadence(table) {
                _ if !recorded(table) => 0,
                Cadence::Hourly | Cadence::Sliced => 24,
                Cadence::Daily(_) => 1,
                Cadence::Static => 0,
            };
            assert_eq!(refreshes[table.index()], expected, "{table:?} near {base}");
        }
    }
}

#[test]
fn native_world_change_forces_full_acquisition_with_matching_clock_fields() {
    let mut cadence = prepared_cadence();
    assert!(cadence.force_full(&cadence_clock(100, 1).borrowed()));
    cadence.prev_world = collector_lifetime::world_lifetime::generation() + 1;
    assert!(!cadence.continuous(&cadence_clock(101, 1).borrowed()));
    assert!(cadence.force_full(&cadence_clock(101, 1).borrowed()));
    assert!(cadence.continuous(&cadence_clock(102, 1).borrowed()));
    assert!(!cadence.force_full(&cadence_clock(102, 1).borrowed()));
}

#[test]
fn branch_starts_loads_and_gaps_force_full_refresh_but_keyframes_do_not() {
    let mut cadence = prepared_cadence();
    assert!(cadence.force_full(&cadence_clock(100, 1).borrowed()));
    assert!(!cadence.force_full(&cadence_clock(101, 1).borrowed()));
    assert!(!cadence.force_full(&cadence_clock(102, 1).borrowed()));
    assert!(cadence.force_full(&cadence_clock(103, 2).borrowed()));
    assert!(cadence.force_full(&cadence_clock(500, 2).borrowed()));
    for hour in 501..=500 + 2 * 720 {
        assert!(
            !cadence.force_full(&cadence_clock(hour, 2).borrowed()),
            "hour {hour}"
        );
    }
    let mut fresh = prepared_cadence();
    assert!(fresh.force_full(&cadence_clock(10, 1).borrowed()));
    let mut changed = cadence_clock(11, 1);
    changed.campaign_id.bytes[0] ^= 1;
    assert!(fresh.force_full(&changed.borrowed()));
}

#[test]
fn periodic_keyframe_keeps_observation_continuous_but_load_gap_and_campaign_do_not() {
    let mut cadence = prepared_cadence();
    assert!(!cadence.continuous(&cadence_clock(100, 1).borrowed()));
    cadence.force_full(&cadence_clock(100, 1).borrowed());
    for hour in 101..=820 {
        let clock = cadence_clock(hour, 1);
        assert!(cadence.continuous(&clock.borrowed()));
        assert!(!cadence.force_full(&clock.borrowed()));
    }
    assert!(!cadence.continuous(&cadence_clock(822, 1).borrowed()));
    assert!(!cadence.continuous(&cadence_clock(821, 2).borrowed()));
    let mut changed = cadence_clock(821, 1);
    changed.campaign_id.bytes[0] ^= 1;
    assert!(!cadence.continuous(&changed.borrowed()));
}

#[test]
fn table_retry_preserves_admitted_politics_mutations() {
    use collector_politics::politics_changes::{
        self as politics, Event, LaneCursor, StagingFixture,
    };
    let _serial = politics::staging_test();
    let memory = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    let fixture = StagingFixture::new(1, 1);
    fixture.observe(
        Event {
            country: 5_000,
            before: 100,
            after: 73,
            ..Default::default()
        },
        &memory,
    );
    let mut cursors = vec![LaneCursor::default()];
    // SAFETY: `fixture` stays alive and unmoved on the stack through `handoff.drain` below.
    let handoff = unsafe { fixture.handoff(&mut cursors) }.unwrap();
    // SAFETY: The caller owns the isolated fixture state required by this constructor.
    let shared = unsafe { Shared::new(0, 0, 1) };
    let mut cadence = prepared_cadence();
    cadence.force_full(&cadence_clock(61320026, 1).borrowed());
    // SAFETY: The spool requests recovery after unpublished output in the current session.
    unsafe { shared.request_retry_full() };
    let clock = cadence_clock(61320027, 1);
    let continuous = cadence.continuous(&clock.borrowed());
    let recovery = shared.force_next.swap(false, Ordering::AcqRel);
    let refresh = shared.refresh_next.swap(false, Ordering::AcqRel);
    assert!(cadence.force_full(&clock.borrowed()) | recovery | refresh);
    if !continuous || recovery {
        // SAFETY: Test owns its isolated politics fixture and exercises recovery invalidation.
        unsafe { politics::invalidate() };
    }
    let mut events = [Event::default()];
    let mut bytes = [0; 128];
    // SAFETY: `fixture` is still alive and `cursors` came from `fixture.handoff` above.
    let drained = unsafe { handoff.drain(&cursors, &mut events, &mut bytes) }.unwrap();
    assert_eq!(drained.copied, 1);
    assert_eq!(
        (events[0].country, events[0].before, events[0].after),
        (5_000, 100, 73)
    );
    assert!(!shared.force_next.load(Ordering::Acquire));
    assert!(!shared.refresh_next.load(Ordering::Acquire));
}

#[test]
fn wars_typed_walk_preserves_primary_native_rows() {
    use collector_states::wars_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let split = kit::direct(&memory, 64);
    assert_eq!(split.owned_outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.owned[0].first_country, 1);
    assert_eq!(split.owned[0].second_country, 2);
    assert_eq!(split.owned[0].start_hour, i32::MIN);
    assert_eq!(split.owned[0].first_casualties, i64::MAX);
    assert_eq!(split.owned[1].first_country, 3);
    assert_eq!(split.owned[1].second_country, 1);
    assert_eq!(split.owned[1].first_casualties, 11);
}

#[test]
fn wars_typed_walk_keeps_full_validation_from_any_starting_storage() {
    use collector_states::wars_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for cap in [0usize, 1, 2, 64] {
        let direct = kit::direct(&memory, cap);
        assert_eq!(
            direct.owned_outcome,
            Outcome::Rows { len: 2 },
            "capacity {cap}"
        );
    }
}

#[test]
fn wars_primary_faults_preserve_native_validation_scope() {
    use collector_states::wars_test_kit as kit;
    use engine_native::{ccountry, cgame_state, crelation, memory::SliceMemory};
    use engine_native_politics::cdiplomacy_status;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for (fault, complete) in [
        (kit::GAME_STATE + cgame_state::COUNTRIES, false),
        (kit::GAME_STATE + cgame_state::COUNTRY_LINKS, false),
        (kit::CTAB + 8, false),
        (kit::LTAB + 4, false),
        (kit::C0, false),
        (kit::C0 + ccountry::ID, false),
        (kit::C0 + ccountry::DIPLOMACY_STATUS, false),
        (kit::D0, false),
        (kit::WAR1 + crelation::TYPE_ID, false),
        (kit::WAR1 + crelation::START_HOUR, false),
        (kit::D0 + cdiplomacy_status::OWNED_RELATIONS, false),
        (kit::O0 + 8, false),
        (kit::OTHER + crelation::TYPE_ID, false),
        (kit::RETIRED + crelation::TYPE_ID, false),
        (kit::D0 + cdiplomacy_status::RELATIONS, false),
        (kit::M0 + 8, true),
        (kit::status(1, 2), true),
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory, 64);
        assert_eq!(
            matches!(direct.owned_outcome, Outcome::Rows { .. }),
            complete,
            "fault={fault:x}",
        );
    }
}

#[test]
fn divisions_split_copy_matches_direct_walk() {
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory, [64; 6]);
    let (split, report, _) =
    // SAFETY: This isolated fixture owns its Divisions read accounting.
    unsafe { kit::split(&memory, [64; 6]) };
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.divisions_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.divisions[0].id, 101);
    assert_eq!(split.divisions[0].province, 1);
    assert_eq!(split.divisions[0].hit_points, 100_000);
    assert_eq!(split.divisions[1].province, 0);
    assert_eq!(split.divisions[2].template, 0);
    assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.equipment[1].amount_raw, -50);
    assert_eq!(split.groups_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.groups[0].priority, 3);
    assert_eq!(split.requests_outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.deliveries_outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.deliveries[0].progress_raw, Some(300));
    assert_eq!(split.produced_outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.produced[0].amount_raw, 9);
}

#[test]
fn divisions_split_copy_matches_direct_walk_from_short_storage() {
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory, [64; 6]);
    let (split, report, _) =
    // SAFETY: This isolated fixture owns its Divisions read accounting.
    unsafe { kit::split(&memory, [1; 6]) };
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.divisions_outcome, Outcome::Rows { len: 3 });
}

#[test]
fn divisions_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::divisions_test_kit as kit;
    use engine_deployment::{carmy_upgrades_requests, cupgrade_delivery};
    use engine_native::{
        cequipment_variant_pool, cgame_state, cprovince, cunit, memory::SliceMemory,
    };
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    let group = 0x60070usize;
    for (fault, complete) in [
        (kit::GAME_STATE + cgame_state::COUNTRIES, [false; 6]),
        (kit::CTAB + 8, [false; 6]),
        (kit::C0, [false; 6]),
        (kit::C0 + engine_native::ccountry::ID, [false; 6]),
        (kit::C0 + engine_native::ccountry::DIVISIONS, [false; 6]),
        (0x23000, [false; 6]),
        (kit::A1, [false; 6]),
        (kit::A1 + cunit::HANDLE, [false; 6]),
        (kit::A1 + cunit::ROSTER_OWNER, [false; 6]),
        (kit::A1 + cunit::LOCATION, [false; 6]),
        (kit::P1, [false; 6]),
        (kit::P1 + cprovince::ID, [false; 6]),
        (kit::A1 + carmy::STANDARD_TEMPLATE, [false; 6]),
        (0x40000, [false; 6]),
        (kit::A1 + carmy::STRENGTH, [false; 6]),
        (kit::A1 + carmy::ORDERS_GROUP, [false; 6]),
        (0x41000, [false; 6]),
        (
            kit::A1 + carmy::EQUIPMENT_POOL,
            [true, false, true, true, true, true],
        ),
        (
            kit::A1 + carmy::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
            [true, false, true, true, true, true],
        ),
        (0x42000, [true, false, true, true, true, true]),
        (0x50000, [true, false, true, false, true, true]),
        (
            kit::A1 + carmy::REQUESTS,
            [true, true, false, false, false, false],
        ),
        (group, [true, true, false, false, false, false]),
        (
            group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT,
            [true, true, false, false, false, false],
        ),
        (
            group + carmy_upgrades_requests::REQUESTS_DATA,
            [true, true, true, false, true, true],
        ),
        (group + 0x200, [true, true, true, false, true, true]),
        (
            group + carmy_upgrades_requests::DELIVERIES_DATA,
            [true, true, true, true, false, false],
        ),
        (group + 0x300, [true, true, true, true, false, false]),
        (0x64000, [true, true, true, true, false, false]),
        (
            0x64000 + cupgrade_delivery::STATUS,
            [true, true, true, true, false, true],
        ),
        (
            0x64000 + cupgrade_delivery::PROGRESS,
            [true, true, true, true, false, true],
        ),
        (
            0x64000 + cupgrade_delivery::TOTAL_PROGRESS,
            [true, true, true, true, false, true],
        ),
        (
            0x64020 + cequipment_variant_pool::ALLOW_ZERO,
            [true, true, true, true, false, true],
        ),
        (
            0x64020 + cequipment_variant_pool::ITEMS,
            [true, true, true, true, true, false],
        ),
        (0x64100, [true, true, true, true, true, false]),
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory, [64; 6]);
        let (split, report, _) =
        // SAFETY: This isolated fixture owns its Divisions read accounting.
        unsafe { kit::split(&memory, [64; 6]) };
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(
            [
                split.divisions_outcome,
                split.equipment_outcome,
                split.groups_outcome,
                split.requests_outcome,
                split.deliveries_outcome,
                split.produced_outcome,
            ]
            .map(|outcome| matches!(outcome, Outcome::Rows { .. })),
            complete,
            "fault={fault:x}"
        );
    }
}

fn divisions_presentation_memory() -> impl engine_native::Memory {
    use super::divisions_test_kit as kit;
    use engine_native::csub_unit_definition as stats;
    struct Memory(Vec<u8>);
    impl engine_native::Memory for Memory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == carmy::COUNTER_ATTRITION_THRESHOLD_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&500_i64.to_le_bytes());
                return true;
            }
            kit::memory(&self.0).copy(address, output)
        }
    }
    let mut bytes = kit::fixture();
    let stats = 0x65000_usize;
    bytes[stats..stats + 8].copy_from_slice(&(stats::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[stats + stats::MAXIMUM_STRENGTH..stats + stats::MAXIMUM_STRENGTH + 8]
        .copy_from_slice(&500_000_i64.to_le_bytes());
    bytes[stats + stats::MAXIMUM_ORGANISATION..stats + stats::MAXIMUM_ORGANISATION + 8]
        .copy_from_slice(&700_000_i64.to_le_bytes());
    for army in [kit::A1, kit::A2, kit::A3] {
        bytes[army + cunit::STAT_BLOCK..army + cunit::STAT_BLOCK + 8]
            .copy_from_slice(&(stats as u64).to_le_bytes());
        let acclimatization = army + carmy::ACCLIMATIZATION;
        bytes[acclimatization..acclimatization + 8].copy_from_slice(
            &(engine_native::acclimatization_data::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
    }
    Memory(bytes)
}

#[test]
fn divisions_owned_success_supplies_both_live_companions_before_decode_without_erasing_them() {
    use super::divisions_test_kit as kit;
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    let memory = divisions_presentation_memory();
    let ctx = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    let mut first = DivisionsJob::new();
    let report =
    // SAFETY: The isolated fixture owns the Divisions read accounting for this copy.
    unsafe { copy_divisions_refresh(&memory, ctx, &mut first) };
    assert!(report.raw_ok);
    assert_eq!(first.source_outcome(), Outcome::Rows { len: 3 });
    let (first_sources, _) = first.census.sources();
    assert_eq!(first_sources[0].division, 101);
    assert_eq!(first_sources[0].template, 0x3455_0000_0034);
    assert_eq!(first_sources[0].country, 1);
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    first.supply_sources(&mut frame.extended, None);
    frame
        .extended
        .install_unit_inputs(first.census.unit_inputs().cloned());
    frame.extended.set_divisions_raw_mode(true);
    let ticks = frame
        .extended
        .capture(&memory, 0, kit::GAME_STATE, 0, false);
    for table in [TableId::DivisionMovement, TableId::DivisionStatus] {
        assert!(ticks[table.index()] > 0);
        assert_eq!(
            frame.extended.outcomes()[table.index()],
            Outcome::Rows { len: 3 }
        );
    }
    frame.extended.release_unit_inputs();
    frame.extended.decode_division_presentation_inputs();
    let before = frame.extended.test_division_presentation_tables();
    let [
        TableView::Complete(TableRows::DivisionMovement(movement)),
        TableView::Complete(TableRows::DivisionStatus(status)),
        ..,
    ] = &before
    else {
        panic!("complete presentation tables");
    };
    assert_eq!(
        movement.iter().map(|r| r.division).collect::<Vec<_>>(),
        [101, 102, 103]
    );
    assert_eq!(
        status.iter().map(|r| r.division).collect::<Vec<_>>(),
        [101, 102, 103]
    );
    frame.extended.set_divisions_raw_mode(true);
    decode_divisions_payload(&mut frame, 0, &mut first, &mut [0; 6]);
    assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
    assert_eq!(
        frame
            .extended
            .test_division_detail()
            .division_equipment
            .outcome,
        Outcome::Rows { len: 3 }
    );
    assert_eq!(frame.extended.test_division_presentation_tables(), before);
}

#[test]
fn divisions_copied_sources_distinguish_semantic_faults_and_empty_census() {
    use super::divisions_test_kit as kit;
    let ctx = DivisionsCopyContext {
        base: 0,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    for fault in [
        kit::A2,
        kit::A2 + carmy::STANDARD_TEMPLATE,
        0x41000,
        0x41008,
    ] {
        let mut bytes = kit::fixture();
        bytes[fault..fault + 8].fill(0xff);
        let mut first = DivisionsJob::new();
        let report =
        // SAFETY: The isolated fixture owns the Divisions read accounting for this copy.
        unsafe { copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first) };
        assert!(report.raw_ok);
        assert!(matches!(
            first.source_outcome(),
            Outcome::Unavailable { .. }
        ));
    }
    let mut bytes = kit::fixture();
    for address in [kit::CTAB + 8, kit::CTAB + 16, kit::CTAB + 24] {
        bytes[address..address + 8].fill(0);
    }
    let mut first = DivisionsJob::new();
    let report =
    // SAFETY: The isolated fixture owns the Divisions read accounting for this copy.
    unsafe { copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first) };
    assert!(report.raw_ok);
    assert!(matches!(
        first.source_outcome(),
        Outcome::Unavailable { .. }
    ));
    assert_eq!(first.census.census_outcome(), Outcome::Rows { len: 0 });
    let mut bytes = kit::fixture();
    for country in [kit::C0, kit::C1] {
        bytes[country + ccountry::DIVISIONS..country + ccountry::DIVISIONS + 16].fill(0);
    }
    // SAFETY: The isolated fixture owns the Divisions read accounting for this copy.
    unsafe { copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first) };
    assert_eq!(first.source_outcome(), Outcome::Rows { len: 0 });
    assert!(first.census.sources().0.is_empty());
}

#[test]
fn divisions_owned_census_survives_a_private_core_field_fault() {
    use super::divisions_test_kit as kit;
    struct Fault<'a>(&'a [u8]);
    impl engine_native::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let fault = kit::A1 + carmy::STRENGTH;
            if address <= fault && address.saturating_add(output.len()) > fault {
                return false;
            }
            kit::memory(self.0).copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let mut census = collector_divisions::division_census::Census::new();
    census.capture(&Fault(&bytes), kit::BASE, kit::GAME_STATE);
    assert_eq!(census.census_outcome(), Outcome::Rows { len: 3 });
    assert_eq!(
        census
            .entries()
            .iter()
            .map(|entry| entry.pointer)
            .collect::<Vec<_>>(),
        [kit::A1, kit::A2, kit::A3]
    );
    assert_eq!(
        census.rows().1,
        Outcome::Unavailable {
            reason: "division hit points unreadable"
        }
    );
}

#[test]
fn divisions_shared_empty_country_census_preserves_independent_fog_success() {
    use super::divisions_test_kit as kit;
    let mut bytes = kit::fixture();
    for address in [kit::CTAB + 8, kit::CTAB + 16, kit::CTAB + 24] {
        bytes[address..address + 8].fill(0);
    }
    let memory = kit::memory(&bytes);
    let mut census = collector_divisions::division_census::Census::new();
    census.capture(&memory, kit::BASE, kit::GAME_STATE);
    let direct = engine_visibility::unit_viewer_entry::walk(
        &memory,
        kit::BASE,
        kit::GAME_STATE,
        &mut Vec::new(),
    );
    assert_eq!(direct, Outcome::Rows { len: 0 });
    let shared = engine_visibility::unit_viewer_entry::walk_with_divisions_buffer(
        &memory,
        kit::BASE,
        kit::GAME_STATE,
        engine_native::walk::Buffer::Count,
        Some(census.intel_sources()),
    );
    assert_eq!(shared, direct);
    assert_eq!(
        census.rows().1,
        Outcome::Unavailable {
            reason: "country table has no readable countries"
        }
    );
}

#[test]
fn divisions_owned_batch_does_not_retry_a_failed_field_into_success() {
    use super::divisions_test_kit as kit;
    struct OnceFault<'a> {
        bytes: &'a [u8],
        pending: std::cell::Cell<bool>,
    }
    impl engine_native::Memory for OnceFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let fault = kit::A1 + carmy::STRENGTH;
            if address <= fault
                && address.saturating_add(output.len()) > fault
                && self.pending.replace(false)
            {
                return false;
            }
            kit::memory(self.bytes).copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let memory = OnceFault {
        bytes: &bytes,
        pending: std::cell::Cell::new(true),
    };
    let direct = kit::direct(&memory, [64; 6]);
    assert_eq!(
        direct.divisions_outcome,
        Outcome::Unavailable {
            reason: "division hit points unreadable"
        }
    );
    memory.pending.set(true);
    let mut census = collector_divisions::division_census::Census::new();
    census.capture(&memory, kit::BASE, kit::GAME_STATE);
    assert_eq!(census.rows().1, direct.divisions_outcome);
}

#[test]
fn divisions_seeded_worker_preserves_primary_trace_and_never_repeats_core() {
    use super::divisions_test_kit as kit;
    use collector_memory::read_accounting;
    use replay_data::rows::TableId;
    struct Traced<'a> {
        bytes: &'a [u8],
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl engine_native::Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            kit::memory(self.bytes).copy(address, output)
        }
    }
    struct NoRead;
    impl engine_native::Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("completed core repeated a read");
        }
    }
    let bytes = kit::fixture();
    let capture = read_accounting::Capture::new();
    let memory = CountedMemory::new(Traced {
        bytes: &bytes,
        calls: Default::default(),
    });
    let composed_memory = Traced {
        bytes: &bytes,
        calls: Default::default(),
    };
    let ctx = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    let mut payload = DivisionsJob::new();
    assert!(copy_divisions_inputs_refresh(&memory, ctx, &mut payload));
    let seed_reads = read_accounting::snapshot();
    assert!(seed_reads[TableId::Divisions.index()].bytes > 0);
    let inputs = engine_allocation::SharedBox::clone(payload.census.unit_inputs().unwrap());
    let report = copy_divisions_core_refresh(&memory, ctx, &mut payload);
    assert!(report.raw_ok);
    assert!(engine_allocation::SharedBox::ptr_eq(
        &inputs,
        payload.census.unit_inputs().unwrap()
    ));
    let accounted = read_accounting::snapshot();
    let repeated = copy_divisions_core_refresh(&NoRead, ctx, &mut payload);
    assert_eq!(repeated.copy_ticks, report.copy_ticks);
    assert_eq!(read_accounting::snapshot(), accounted);
    let mut composed = DivisionsJob::new();
    // SAFETY: The isolated fixture owns the Divisions read accounting for this copy.
    unsafe { copy_divisions_refresh(&composed_memory, ctx, &mut composed) };
    assert_eq!(payload.census.rows(), composed.census.rows());
    assert_eq!(*memory.0.calls.borrow(), *composed_memory.calls.borrow());
    assert_eq!(
        memory
            .0
            .calls
            .borrow()
            .iter()
            .filter(|(at, _)| *at == kit::A1 + carmy::STRENGTH)
            .count(),
        1
    );
    drop(inputs);
    let mut actual =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    let mut expected =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    decode_divisions_payload(&mut actual, kit::BASE, &mut payload, &mut [0; 6]);
    decode_divisions_payload(&mut expected, kit::BASE, &mut composed, &mut [0; 6]);
    assert_eq!(actual.divisions, expected.divisions);
    assert_eq!(
        actual.extended.test_division_detail().tables(),
        expected.extended.test_division_detail().tables()
    );
    let mut total = ReadCounts::ZERO;
    for (index, counts) in accounted.iter().enumerate() {
        total += *counts;
        if [
            TableId::Divisions,
            TableId::DivisionEquipment,
            TableId::ArmyUpgradeGroups,
        ]
        .iter()
        .any(|table| table.index() == index)
        {
            assert!(counts.spans > 0 && counts.bytes > 0);
        } else {
            assert_eq!(*counts, ReadCounts::ZERO);
        }
    }
    let trace = memory.0.calls.borrow();
    assert_eq!(
        total.spans,
        trace.iter().filter(|(_, len)| *len != 0).count() as u64
    );
    assert_eq!(
        total.bytes,
        trace.iter().map(|(_, len)| *len as u64).sum::<u64>()
    );
    assert_eq!(capture.finish(), accounted);
}

#[test]
fn divisions_seeded_worker_rejects_changed_context_without_reacquiring_inputs() {
    use super::divisions_test_kit as kit;
    struct NoRead;
    impl engine_native::Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("invalid worker context must not acquire live state");
        }
    }
    let bytes = kit::fixture();
    let context = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    let mut job = DivisionsJob::new();
    assert!(copy_divisions_inputs_refresh(
        &kit::memory(&bytes),
        context,
        &mut job,
    ));
    let lease = engine_allocation::SharedBox::clone(job.census.unit_inputs().unwrap());
    let changed = DivisionsCopyContext {
        game_state: context.game_state + 8,
        ..context
    };
    assert!(!copy_divisions_core_refresh(&NoRead, changed, &mut job).raw_ok);
    assert!(engine_allocation::SharedBox::ptr_eq(
        &lease,
        job.census.unit_inputs().unwrap()
    ));
    assert!(!copy_divisions_core_refresh(&NoRead, context, &mut job).raw_ok);
}

#[test]
fn inactive_division_details_skip_capacity_reads_and_decode_installation() {
    use super::divisions_test_kit as kit;
    struct Traced<'a> {
        bytes: &'a [u8],
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl engine_native::Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            kit::memory(self.bytes).copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let memory = CountedMemory::new(Traced {
        bytes: &bytes,
        calls: Default::default(),
    });
    let reference = Traced {
        bytes: &bytes,
        calls: Default::default(),
    };
    let context = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        source_capacity: 64,
        detail_capacities: [usize::MAX; 5],
        details_active: false,
    };
    let mut job = DivisionsJob {
        census: collector_divisions::division_census::Census::new(),
        ..Default::default()
    };
    let reads = collector_memory::read_accounting::Capture::new();
    assert!(copy_divisions_inputs_refresh(&memory, context, &mut job));
    let lease = engine_allocation::SharedBox::clone(job.census.unit_inputs().unwrap());
    let report = copy_divisions_core_refresh(&memory, context, &mut job);
    assert!(report.raw_ok);
    assert!(engine_allocation::SharedBox::ptr_eq(
        &lease,
        job.census.unit_inputs().unwrap()
    ));
    assert_eq!(job.details.allocated_bytes(), 0);
    let skipped = Outcome::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON,
    };
    assert_eq!(job.details.outcomes(), [skipped; 5]);
    assert_eq!(report.copy_ticks[1..], [0; 5]);
    let counts = reads.finish();
    assert!(counts[TableId::Divisions.index()].spans > 0);
    for (index, counts) in counts.iter().enumerate() {
        if index != TableId::Divisions.index() {
            assert_eq!(*counts, ReadCounts::ZERO);
        }
    }
    let mut census = collector_divisions::division_census::Census::new();
    census.capture_inputs(&reference, context.base, context.game_state);
    census.capture_core(&reference);
    assert_eq!(*memory.0.calls.borrow(), *reference.calls.borrow());
    assert_eq!(job.census.rows(), census.rows());
    drop(lease);

    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    frame
        .extended
        .fail_division_details(replay_data::cadence::SKIPPED_REASON);
    frame.extended.set_divisions_raw_mode(true);
    let detail = frame.extended.test_division_detail();
    let pointer = detail.division_equipment.rows.as_ptr();
    let capacity = detail.division_equipment.rows.len();
    let mut ticks = [0; 6];
    decode_divisions_payload(&mut frame, context.base, &mut job, &mut ticks);
    assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
    let detail = frame.extended.test_division_detail();
    assert_eq!(detail.division_equipment.rows.as_ptr(), pointer);
    assert_eq!(detail.division_equipment.rows.len(), capacity);
    assert_eq!(detail.division_equipment.outcome, skipped);
    assert!(!detail.divisions_raw_mode);
    assert_eq!(ticks[1..], [0; 5]);
}

#[test]
fn divisions_copy_decode_counters_keep_game_thread_cost_separate() {
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    let mut first = DivisionsJob::new();
    let report =
    // SAFETY: The isolated fixture owns the Divisions read accounting for this copy.
    unsafe { copy_divisions_refresh(&memory, ctx, &mut first) };
    assert!(report.raw_ok);
    assert!(report.copy_ticks[0] > 0);
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    frame.table_ns[6] = report.copy_ticks[0];
    let before = frame.table_ns;
    let mut ticks = [0; 6];
    super::decode_divisions_payload(&mut frame, kit::BASE, &mut first, &mut ticks);
    assert_eq!(frame.table_ns, before);
    assert!(ticks[0] > 0);
    assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
}

#[test]
fn divisions_split_from_any_pooled_storage_matches_the_whole_direct_walk() {
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory, [64; 6]);
    for cap in [0usize, 1, 3, 64] {
        let caps = [cap; 6];
        let (split, report, _) =
        // SAFETY: This isolated fixture owns its Divisions read accounting.
        unsafe { kit::split(&memory, caps) };
        assert!(report.raw_ok, "cap={cap}");
        kit::assert_matches_direct(&split, &direct);
    }
    let (exact, _, _) =
    // SAFETY: This isolated fixture owns its Divisions read accounting.
    unsafe { kit::split(&memory, [3; 6]) };
    assert_eq!(exact.divisions_outcome, Outcome::Rows { len: 3 });
    for groups in [0, 1] {
        let mut caps = [3; 6];
        caps[2] = groups;
        let direct = kit::direct(&memory, [3; 6]);
        let (split, report, _) =
        // SAFETY: This isolated fixture owns its Divisions read accounting.
        unsafe { kit::split(&memory, caps) };
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.groups_outcome, Outcome::Rows { len: 3 });
    }
}

fn run_naval_observer_test_in_child(test: &str) -> bool {
    const CHILD: &str = "BATTLEPLAN_NAVAL_ACQUISITION_TEST";
    if std::env::var_os(CHILD).is_some() {
        return false;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, "1")
        .status()
        .unwrap();
    assert!(status.success());
    true
}

#[test]
fn no_frame_acquisition_marks_naval_sources_unavailable_before_reuse() {
    use collector_naval_results::{
        naval_result_sources as sources,
        naval_result_sources::{Entry, tests::Observation},
    };
    if run_naval_observer_test_in_child(
        "sampling::tests::no_frame_acquisition_marks_naval_sources_unavailable_before_reuse",
    ) {
        return;
    }
    let _observation = Observation::new();
    sources::tests::setup(1, 0);
    sources::boundary(Some(clock(40, 7).with_campaign_id(0)));
    sources::boundary(Some(clock(41, 7).with_campaign_id(0)));
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    shared.roster_ready.store(true, Ordering::Release);
    let index = None;
    unsafe {
        acquire(
            &shared,
            Dispatch {
                world_generation: collector_lifetime::world_lifetime::generation(),
                campaign_epoch: 1,
                index,
                entry: Some(clock(41, 7).borrowed()),
                entry_ticks: 0,
                dispatched: 0,
                phases: HourPhases::default(),
                full: false,
            },
        );
    }
    let mut entries = vec![Entry::default(); 1];
    assert_eq!(
        sources::drain(&mut entries).unwrap_err(),
        "naval result sources lost observation; interval unavailable"
    );
    let held =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.unwrap();
    sources::boundary(Some(clock(42, 7).with_campaign_id(0)));
    assert_eq!(
        sources::drain(&mut entries).unwrap_err(),
        "naval result sources recovery interval is partial"
    );
    sources::boundary(Some(clock(43, 7).with_campaign_id(0)));
    assert_eq!(sources::drain(&mut entries).unwrap(), 0);
    assert_eq!(shared.dropped.load(Ordering::Relaxed), 1);
    let losses = unsafe { &*shared.pending.get() };
    let run = losses.iter().next().unwrap();
    assert_eq!((run.first_hour, run.last_hour, run.count), (41, 41, 1));
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.recycle_frame(held) };
}

#[test]
fn contained_acquisition_failure_marks_the_naval_event_interval_unavailable() {
    use collector_naval_results::{
        naval_result_sources as sources,
        naval_result_sources::{Entry, tests::Observation},
    };
    if run_naval_observer_test_in_child(
        "sampling::tests::contained_acquisition_failure_marks_the_naval_event_interval_unavailable",
    ) {
        return;
    }
    let _observation = Observation::new();
    sources::tests::setup(1, 0);
    sources::boundary(Some(clock(40, 7).with_campaign_id(0)));
    sources::boundary(Some(clock(41, 7).with_campaign_id(0)));
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let acquired = std::cell::Cell::new(false);
    let mut update_returned_first = false;
    assert!(!unsafe {
        synthetic_callback(
            &shared,
            Some(clock(41, 7).borrowed()),
            || {
                acquired.set(true);
                panic!("synthetic acquisition failure")
            },
            || update_returned_first = !acquired.get(),
        )
    });
    assert!(update_returned_first);
    assert!(acquired.get());
    assert_eq!(shared.acquisition_panics.load(Ordering::Acquire), 1);
    assert_eq!(
        sources::drain(&mut vec![Entry::default(); 1]).unwrap_err(),
        "naval result sources lost observation; interval unavailable"
    );
}

struct CampaignClockMemory {
    bytes: Vec<u8>,
}
impl engine_native::Memory for CampaignClockMemory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&0x1000_u64.to_le_bytes());
            true
        } else if address == cgame_state::PLAY_SESSION_STARTED_AT_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&7_i64.to_le_bytes());
            true
        } else {
            engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }
}
fn campaign_clock_memory(len: usize) -> CampaignClockMemory {
    let mut bytes = vec![0; 0x2001 + len];
    bytes[cgame_state::CURRENT_HOUR..cgame_state::CURRENT_HOUR + 4]
        .copy_from_slice(&42_i32.to_le_bytes());
    let campaign = cgame_state::CAMPAIGN_ID;
    let payload = if len <= 15 { campaign } else { 0x2000 };
    bytes[payload..payload + len].fill(b'a');
    if len > 15 {
        bytes[campaign..campaign + 8].copy_from_slice(&0x3000_u64.to_le_bytes());
    }
    bytes[campaign + 16..campaign + 24].copy_from_slice(&(len as u64).to_le_bytes());
    bytes[campaign + 24..campaign + 32].copy_from_slice(&(len.max(15) as u64).to_le_bytes());
    CampaignClockMemory { bytes }
}
fn campaign_shared() -> Shared {
    // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
    // boundary or detach.
    unsafe { Shared::with_fixture_sizing(0, 0, 1, &crate::country_scale::Sizing::for_slots(0), 0) }.unwrap()
}

#[test]
fn campaign_clock_callback_keeps_long_identity_and_loss_bytes_under_allocator_denial() {
    let mut memory = campaign_clock_memory(5_001);
    let mut shared = campaign_shared();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_clock_capture(&memory) };
    let index =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.unwrap();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let before = frame.allocated_bytes();
    let capture = unsafe { shared.prepared_clock_capture(&memory) };
    let cadence = unsafe { &mut *shared.cadence.get() };
    let pending = unsafe { &mut *shared.pending.get() };
    collector_test_support::with_allocations_denied(|| {
        let (entry, epoch) = capture.read(&memory, 0).unwrap();
        assert_eq!(epoch, 1);
        assert_eq!(entry.campaign_id.len(), 5_001);
        assert!(cadence.force_full(&entry));
        frame.set_clock(Some(entry));
        pending.push(Some(entry));
        frame.set_clock(None);
        frame.set_clock(Some(entry));
        assert_eq!(
            frame.clock.as_ref().unwrap().campaign_id.as_bytes(),
            entry.campaign_id
        );
    });
    assert_eq!(frame.allocated_bytes(), before);
    assert_eq!(
        pending.iter().next().unwrap().campaign_id.as_bytes().len(),
        5_001
    );
    memory.bytes[0x2000 + 5_000] = b'b';
    memory.bytes[cgame_state::CURRENT_HOUR..cgame_state::CURRENT_HOUR + 4]
        .copy_from_slice(&43_i32.to_le_bytes());
    collector_test_support::with_allocations_denied(|| {
        let (entry, epoch) = capture.read(&memory, 0).unwrap();
        assert_eq!(epoch, 2);
        assert!(!cadence.continuous(&entry));
        assert!(cadence.force_full(&entry));
        pending.push(Some(entry));
        frame.set_clock(Some(entry));
        assert_eq!(
            frame.clock.as_ref().unwrap().campaign_id.as_bytes().last(),
            Some(&b'b')
        );
    });
    assert_eq!(pending.iter().count(), 2);
    assert_eq!(frame.allocated_bytes(), before);
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.recycle_frame(index) };
}

#[test]
fn campaign_clock_growth_refused_by_the_allocator_fails_the_read_and_a_later_callback_grows() {
    let short = campaign_clock_memory(1);
    let long = campaign_clock_memory(5_001);
    let mut shared = campaign_shared();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_clock_capture(&short) };
    let (prepared, read) = collector_test_support::with_allocations_denied(|| {
        let capture = unsafe { shared.prepared_clock_capture(&long) };
        let read = capture.read(&long, 0).is_ok();
        (capture.prepared, read)
    });
    assert_eq!((prepared, read), (1, false));
    let capture = unsafe { shared.prepared_clock_capture(&long) };
    assert_eq!(capture.prepared, 5_001);
    assert!(capture.read(&long, 0).is_ok());
}

#[test]
fn frame_growth_refused_a_slot_segment_gives_up_without_aborting() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let held: Vec<_> = (0..BUFFERS)
        .map(|_|
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap())
        .collect();
    let mut grow_ns = u64::MAX;
    let segment = collector_test_support::with_allocations_denied(||
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut grow_ns) });
    assert_eq!((segment, grow_ns), (None, 0));
    assert_eq!(
        (shared.frames.made(), shared.frames.live()),
        (BUFFERS, BUFFERS)
    );
    let grown =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut grow_ns) }.unwrap();
    assert!(grow_ns > 0);
    for index in held.into_iter().chain(Some(grown)) {
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame(index) };
    }
    assert_eq!(
        (shared.frames.made(), shared.frames.live()),
        (BUFFERS + 1, BUFFERS)
    );
}

#[test]
fn campaign_clock_preparation_grows_on_the_game_thread_without_waiting() {
    let short = campaign_clock_memory(1);
    let long = campaign_clock_memory(5_001);
    let mut shared = campaign_shared();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_clock_capture(&short) };
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_frame_clocks(&long) };
    assert_eq!(unsafe { shared.prepared_clock_capture(&short) }.prepared, 1);
    let capture = unsafe { shared.prepared_clock_capture(&long) };
    assert_eq!(capture.prepared, 5_001);
    assert!(capture.read(&long, 0).is_ok());
    let frames = collector_test_support::with_allocations_denied(|| {
        let frames: [Option<usize>; BUFFERS] = std::array::from_fn(|_|
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) });
        for index in frames.into_iter().flatten() {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.recycle_frame(index) };
        }
        frames
    });
    assert!(
        frames.iter().all(Option::is_some),
        "a frame was short: {frames:?}"
    );
}

#[test]
fn an_hour_needing_a_longer_clock_never_waits_for_the_spool_s_clock_preparation() {
    use std::sync::mpsc;
    let short = campaign_clock_memory(1);
    let long = Arc::new(campaign_clock_memory(5_001));
    let mut shared = campaign_shared();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_clock_capture(&short) };
    let (holding, held) = mpsc::channel::<()>();
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let installed = shared.clock_frame_seam.set(Box::new(move || {
        let _ = holding.send(());
        let _ = released
            .lock()
            .unwrap()
            .recv_timeout(collector_test_support::TEST_HANG_GUARD);
    }));
    assert!(installed.is_ok());
    let shared = Arc::new(shared);
    let watching = Arc::clone(&shared);
    collector_test_support::watched(
        move || format!("{:?}", watching.frames.gauge()),
        move |steps| {
            let release = release;
            let spool = {
                let (shared, long) = (Arc::clone(&shared), Arc::clone(&long));
                collector_test_support::spawn_ending(move ||
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { shared.prepare_frame_clocks(&*long) })
            };
            steps.at("waiting for the spool's clock preparation to hold a frame");
            held.recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap_or_else(|_| panic!("the spool's clock preparation never held a frame"));
            let hour = {
                let (shared, long) = (Arc::clone(&shared), Arc::clone(&long));
                collector_test_support::spawn_ending(move || {
                    let capture = unsafe { shared.prepared_clock_capture(&*long) };
                    let index =
                    // SAFETY: This fixture owns its frame and worker state and runs without a live
                    // attachment.
                    unsafe { shared.take_frame(&mut 0) };
                    let read = capture.read(&*long, 0);
                    match (index, read) {
                        (Some(index), Ok((entry, epoch))) => {
                            let frame =
                                unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
                            frame.set_clock(Some(entry));
                            let clock = frame.clock.as_ref().unwrap();
                            Ok((
                                index,
                                clock.hour,
                                clock.campaign_id.as_bytes().to_vec(),
                                epoch,
                            ))
                        }
                        (index, read) => Err(format!("frame {index:?}, read {:?}", read.err())),
                    }
                })
            };
            steps.at("the hour, while the spool's preparation is held");
            let (index, hour, campaign_id, epoch) = hour
                .end(|| "the hour, waiting for the spool's clock preparation,".into())
                .unwrap_or_else(|failure| panic!("the hour's clock failed: {failure}"));
            assert!(
                !spool.is_finished(),
                "the spool's preparation ended before the hour did"
            );
            drop(release);
            steps.at("the spool's preparation, released");
            spool.end(|| "the spool's clock preparation, released,".into());
            assert_eq!((hour, epoch), (42, 1));
            assert_eq!(campaign_id, vec![b'a'; 5_001]);
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.recycle_frame(index) };
        },
    );
}

#[test]
fn campaign_clock_in_service_frames_prepare_before_republication() {
    let short = campaign_clock_memory(1);
    let long = campaign_clock_memory(5_001);
    let mut shared = campaign_shared();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_clock_capture(&short) };
    let held: Vec<_> = (0..BUFFERS)
        .map(|_|
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap())
        .collect();
    let _ = unsafe { shared.prepared_clock_capture(&long) };
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_frame_clocks(&long) };
    for index in held {
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame_prepared(index) };
    }
    collector_test_support::with_allocations_denied(|| {
        let capture = unsafe { shared.prepared_clock_capture(&long) };
        let (entry, _) = capture.read(&long, 0).unwrap();
        let held: [usize; BUFFERS] = std::array::from_fn(|_|
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap());
        for &index in &held {
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.set_clock(Some(entry));
            assert_eq!(
                frame.clock.as_ref().unwrap().campaign_id.as_bytes().len(),
                5_001
            );
        }
        for index in held {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.recycle_frame(index) };
        }
    });
}

#[test]
fn campaign_clock_optional_preparation_denial_is_named_and_nonterminal() {
    let memory = campaign_clock_memory(5_001);
    let mut shared = campaign_shared();
    collector_test_support::with_allocations_denied(||
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_clock_capture(&memory) });
    assert!(!collector_memory::terminal::failed());
    assert_eq!(shared.clock_capture.get_mut().prepared, 0);
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.prepare_clock_capture(&memory) };
    assert_eq!(shared.clock_capture.get_mut().prepared, 5_001);
}

#[test]
fn callback_priority_raises_the_acquisition_and_restores_the_game_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_NORMAL, THREAD_PRIORITY_TIME_CRITICAL,
    };
    let current = || unsafe { GetThreadPriority(GetCurrentThread()) };
    let set = |priority| {
        assert_ne!(
            unsafe { SetThreadPriority(GetCurrentThread(), priority) },
            0
        );
    };
    for game in [THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_NORMAL] {
        set(game);
        {
            let _raised = CallbackPriority::raise();
            assert_eq!(current(), THREAD_PRIORITY_HIGHEST);
        }
        assert_eq!(current(), game);
    }
    for game in [THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_TIME_CRITICAL] {
        set(game);
        {
            let raised = CallbackPriority::raise();
            assert!(raised.0.is_none());
            assert_eq!(current(), game);
        }
        assert_eq!(current(), game);
    }
    set(THREAD_PRIORITY_NORMAL);
    let result = std::panic::catch_unwind(|| {
        let _raised = CallbackPriority::raise();
        panic!("fixture acquisition panic");
    });
    assert!(result.is_err());
    assert_eq!(current(), THREAD_PRIORITY_NORMAL);
}

#[test]
fn a_parked_spool_wakes_on_a_ready_buffer_without_waiting_for_its_deadline() {
    use std::sync::mpsc;
    for wake_before_park in [false, true] {
        let shared = Arc::new(
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Shared::new(0, 0, 1) },
        );
        shared.wake_spool();
        let (admitted_tx, admitted) = mpsc::channel();
        let release = Arc::new(std::sync::Barrier::new(2));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let spool = {
            let shared = Arc::clone(&shared);
            let release = Arc::clone(&release);
            let cancel = Arc::clone(&cancel);
            collector_test_support::spawn_ending(move || {
                shared.spool_waker.set(std::thread::current()).unwrap();
                while shared.frames.ready_is_empty() && !cancel.load(Ordering::Acquire) {
                    admitted_tx.send(()).unwrap();
                    if wake_before_park {
                        release.wait();
                    }
                    std::thread::park();
                }
            })
        };
        admitted.recv().unwrap();
        crate::decode::hand_ready(&shared, 1, 1);
        if wake_before_park {
            release.wait();
        }
        if spool
            .done
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .is_err()
        {
            let finished = spool.is_finished();
            cancel.store(true, Ordering::Release);
            spool.thread().unpark();
            let joined = spool.thread.join();
            if let Err(panic) = joined {
                std::panic::resume_unwind(panic);
            }
            assert!(
                finished,
                "the ready handoff did not wake the spool (woken before park: {wake_before_park})"
            );
        } else {
            spool.thread.join().unwrap();
        }
    }
}

#[test]
fn buffer_return_runs_on_a_small_stack() {
    const SMALL_STACK: usize = 256 * 1024;
    const _: () = assert!(SMALL_STACK * 32 <= collector_workers::priority::BACKGROUND_STACK_BYTES);
    let shared = std::sync::Arc::new(campaign_shared());
    let worker = std::sync::Arc::clone(&shared);
    let returned = std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(move || {
            let held: Vec<_> = (0..BUFFERS)
                .map(|_|
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { worker.take_frame(&mut 0) }.unwrap())
                .collect();
            for index in held {
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { worker.recycle_frame_prepared(index) };
            }
            worker.frames.len()
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(returned, BUFFERS);
}

#[test]
fn buffers_backlogged_with_one_free_buffer() {
    let shared = campaign_shared();
    let first =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.unwrap();
    let second =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.unwrap();
    assert_eq!(shared.frames.len(), 2);
    assert!(!shared.buffers_backlogged());
    let third =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.unwrap();
    assert_eq!(shared.frames.len(), 1);
    assert!(shared.buffers_backlogged());
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.recycle_frame_prepared(first) };
    assert!(!shared.buffers_backlogged());
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.recycle_frame_prepared(second) };
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.recycle_frame_prepared(third) };
    assert_eq!(shared.frames.len(), BUFFERS);
}

#[test]
fn every_raw_copy_join_has_a_raw_payload_or_a_failure() {
    const INCOMPLETE: &str = "Radar owned acquisition could not complete";
    for failure in [None, Some("Radar acquisition job panicked")] {
        for raw_ok in [false, true] {
            let joined = super::raw_copy_join_failure(failure, raw_ok, INCOMPLETE);
            assert!(raw_ok || joined.is_some(), "{failure:?} {raw_ok}");
            assert_eq!(joined.is_none(), raw_ok && failure.is_none());
            assert_eq!(
                joined,
                failure.or((!raw_ok).then_some(INCOMPLETE)),
                "{failure:?} {raw_ok}"
            );
        }
    }
}

#[test]
fn every_air_bases_join_has_a_raw_payload_or_a_failure() {
    for failure in [None, Some("AirBases acquisition job panicked")] {
        for raw_ok in [false, true] {
            let joined = super::air_bases_join_failure(failure, raw_ok);
            assert!(raw_ok || joined.is_some(), "{failure:?} {raw_ok}");
            assert_eq!(joined.is_none(), raw_ok && failure.is_none());
            if failure.is_some() {
                assert_eq!(joined, failure);
            }
        }
    }
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "no recording's tables are in scope")]
fn ordinary_frame_sizing_requires_executable_tables() {
    let sizing = crate::country_scale::Sizing::for_slots(0);
    // SAFETY: This fixture does not publish observer bindings or install hooks.
    let _ = unsafe { Shared::without_frames(0, 0, 1, &sizing) };
}
