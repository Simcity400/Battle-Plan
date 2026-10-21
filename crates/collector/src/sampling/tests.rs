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

/// A rejected submission as `submit_retained` leaves it: parked in the pool.
fn park(batch: &mut acquisition::Batch<'_>, request: acquisition::Job) -> PendingAcquisition {
    let mut retained = Some(request);
    PendingAcquisition::Rejected(batch.park(&mut retained).expect("free acquisition slot"))
}

#[test]
fn rejected_division_inputs_keep_the_original_owner_and_cannot_retry_native_inputs() {
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let payload = DivisionsJob::new_worker();
    let owner = Arc::clone(payload.census.unit_inputs().unwrap());
    let context = DivisionsCopyContext {
        game_state: usize::MAX,
        details_active: false,
        ..Default::default()
    };
    let request = acquisition::Job::DivisionInputs(acquisition::RawRequest {
        countries: Arc::new(Cache::try_with_capacity(0).unwrap()),
        context,
        payload,
    });
    let rejected = park(&mut batch, request);
    let reads = crate::read_accounting::Capture::new();
    let completed = crate::test_support::with_allocations_denied(|| {
        // SAFETY: rejection must return the owned packet without executing it.
        unsafe { finish_acquisition(&shared, &mut batch, rejected) }
    });
    assert!(matches!(
        completed.report,
        Err(acquisition::Failure::Stopped)
    ));
    assert_eq!(completed.table_reads, crate::read_accounting::ZERO);
    let acquisition::Job::DivisionInputs(mut request) = completed.request else {
        panic!("rejected division input owner lost");
    };
    assert!(Arc::ptr_eq(
        &owner,
        request.payload.census.unit_inputs().unwrap()
    ));
    assert!(!request.payload.inputs_captured);
    let report = copy_divisions_core_refresh(&NoDivisionReads, context, &mut request.payload);
    assert!(!report.raw_ok);
    assert!(request.payload.core_finished);
    assert_eq!(request.payload.payload_bytes(), [0; 6]);
    assert!(Arc::ptr_eq(
        &owner,
        request.payload.census.unit_inputs().unwrap()
    ));
    assert!(!copy_divisions_core_refresh(&NoDivisionReads, context, &mut request.payload).raw_ok);
    assert_eq!(reads.finish(), crate::read_accounting::ZERO);
    let counters = pool.calibration().roles[acquisition::Role::DivisionInputs as usize].counters;
    assert_eq!(counters.inline_completed, 0);
    assert_eq!(pool.calibration().executor.uncollected, 0);
}

/// The hourly body holds a pending join for every dispatched job at once, on
/// the game thread's stack: a join is a ticket, and a rejected request waits
/// parked in the pool rather than in the join.
#[test]
fn a_rejected_submission_parks_in_the_pool_and_its_join_returns_it() {
    assert!(size_of::<PendingAcquisition>() <= 2 * size_of::<acquisition::Ticket>());
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    pool.shutdown().unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let payload = DivisionsJob::new_worker();
    let owner = Arc::clone(payload.census.unit_inputs().unwrap());
    let request = acquisition::Job::DivisionInputs(acquisition::RawRequest {
        countries: Arc::new(Cache::try_with_capacity(0).unwrap()),
        context: DivisionsCopyContext {
            game_state: usize::MAX,
            details_active: false,
            ..Default::default()
        },
        payload,
    });
    // SAFETY: the stopped pool runs nothing; the batch drains its tickets.
    let pending = unsafe { submit_acquisition(&shared, &mut batch, request) };
    assert!(matches!(pending, PendingAcquisition::Rejected(_)));
    let calibration = pool.calibration();
    assert_eq!(calibration.executor.uncollected, 0);
    assert_eq!(calibration.executor.queued, 0);
    let completed = crate::test_support::with_allocations_denied(|| {
        // SAFETY: as above.
        unsafe { finish_acquisition(&shared, &mut batch, pending) }
    });
    assert!(matches!(
        completed.report,
        Err(acquisition::Failure::Stopped)
    ));
    let acquisition::Job::DivisionInputs(request) = completed.request else {
        panic!("parked division input owner lost");
    };
    assert!(Arc::ptr_eq(
        &owner,
        request.payload.census.unit_inputs().unwrap()
    ));
    let counters = pool.calibration().roles[acquisition::Role::DivisionInputs as usize].counters;
    assert_eq!(counters.rejected, 1);
    assert_eq!(counters.inline_completed, 0);
}

#[test]
fn rejected_air_missions_reuses_the_heap_payload_and_publishes_named_first_hour_refusals() {
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    pool.shutdown().unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let mut captured = extended::Captured::with_capacities(&extended::Capacities::default());
    let countries = Arc::new(Cache::try_with_capacity(0).unwrap());
    for hour in [3, 17] {
        captured.air_wing_pilot().prepare_hour(hour, true);
        let request = captured.take_air_missions_request(&countries, 0, 0, None);
        let owner = &*request.pack as *const _;
        let pending = park(&mut batch, acquisition::Job::AirMissions(request));
        // SAFETY: the stopped pool owns only the retained rejected request.
        let completed = crate::test_support::with_allocations_denied(|| unsafe {
            finish_acquisition(&shared, &mut batch, pending)
        });
        assert!(matches!(
            completed.report,
            Ok(acquisition::Report::AirMissions(_))
        ));
        assert_eq!(completed.table_reads, crate::read_accounting::ZERO);
        let acquisition::Job::AirMissions(request) = completed.request else {
            panic!("Air mission payload lost")
        };
        assert_eq!(&*request.pack as *const _, owner);
        captured.restore_air_missions_request(request, false);
        // A cold attach and a subsequent gap recovery both admit an explicit
        // table refusal, never an incomplete row masquerading as a value.
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
        for (table, view) in extended::air::TABLES.into_iter().zip(tables) {
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

/// A rejected Presentation job is terminal like every worker-only job: its
/// join fails the merged report even when the movement pass beside it read,
/// so the whole presentation is refused once both loans are home.
#[test]
fn a_rejected_presentation_fails_beside_a_completed_movement_pass() {
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    pool.shutdown().unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let mut request = acquisition::PresentationRequest {
        dispatch: Default::default(),
        countries: Arc::new(Cache::try_with_capacity(0).unwrap()),
        context: extended::division_presentation::CaptureContext {
            base: 0,
            game_state: 0,
            hour: 0,
        },
        captured: extended::division_presentation::Captured::vacant(),
        inputs: None,
        army_stats: None,
    };
    let movement = request.take_movement();
    // SAFETY: the stopped pool runs nothing; the batch drains its tickets.
    let pending =
        unsafe { submit_acquisition(&shared, &mut batch, acquisition::Job::Presentation(request)) };
    assert!(matches!(pending, PendingAcquisition::Rejected(_)));
    // SAFETY: as above.
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
                Ok(extended::division_presentation::CaptureReport {
                    ticks: [1, 3, 0, 2, 0, 0, 0, 5, 7, 11, 13, 17],
                }),
                report,
            )
            .err(),
        Some(acquisition::Failure::Stopped)
    );
}

/// The stopped pool an Orders panic leaves rejects the Fronts job: the join
/// gets its part back whole, and a part whose Orders job failed keeps that
/// reason; a part whose Orders job completed fails as the Fronts job's own.
#[test]
fn a_rejected_fronts_job_returns_its_part_and_keeps_the_orders_failure() {
    use crate::extended::plans::{Capacities, Captured};
    use engine_native::walk::Outcome;
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    pool.shutdown().unwrap();
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
        // SAFETY: the stopped pool runs nothing; the batch drains its tickets.
        let pending = unsafe { submit_acquisition(&shared, &mut batch, request) };
        assert!(matches!(pending, PendingAcquisition::Rejected(_)));
        // SAFETY: as above.
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

/// A refused OrdersPart job returns its part unrun, and the join fails the
/// census with the admission reason a refused Orders job has; a part that
/// ran and failed keeps the acquisition reason.
#[test]
fn a_rejected_order_part_returns_its_part_with_the_admission_reason() {
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    pool.shutdown().unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let part = Box::new(crate::orders_parts::Part::new());
    let at: *const crate::orders_parts::Part = &*part;
    let request = acquisition::Job::OrdersPart(acquisition::OrdersPartRequest {
        base: 0,
        index: 2,
        part,
        rejected: false,
    });
    // SAFETY: the stopped pool runs nothing; the batch drains its tickets.
    let pending = unsafe { submit_acquisition(&shared, &mut batch, request) };
    assert!(matches!(pending, PendingAcquisition::Rejected(_)));
    // SAFETY: as above.
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

/// An Orders job or part that fails while running stops the pool, which then
/// refuses the OrdersStitch job: the census and plans keep the acquisition
/// reason, as a failed single Orders job's would. A refused stitch over a
/// census nothing failed takes the admission reason.
#[test]
fn a_rejected_stitch_keeps_an_earlier_order_failure() {
    use crate::extended::plans::{Capacities, Captured};
    use engine_native::walk::Outcome;
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    pool.shutdown().unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    for (earlier, reason) in [
        (
            Some("army order acquisition failed"),
            "army order acquisition failed",
        ),
        (None, "army order worker admission failed"),
    ] {
        let mut request = acquisition::OrdersRequest {
            countries: Arc::new(Cache::try_with_capacity(0).unwrap()),
            base: 0,
            game_state: 0,
            orders: crate::map_capture::Orders::for_test(&Default::default()),
            plans: Captured::with_capacities(&Capacities::default()),
            group_source: None,
        };
        if let Some(reason) = earlier {
            request.fail(reason);
        }
        // SAFETY: the stopped pool runs nothing; the batch drains its tickets.
        let pending = unsafe {
            submit_acquisition(&shared, &mut batch, acquisition::Job::OrdersStitch(request))
        };
        assert!(matches!(pending, PendingAcquisition::Rejected(_)));
        // SAFETY: as above.
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

/// An empty order request for the stitch tests, failed with `earlier`.
fn stitch_request(base: usize, earlier: Option<&'static str>) -> acquisition::OrdersRequest {
    use crate::extended::plans::{Capacities, Captured};
    let mut request = acquisition::OrdersRequest {
        countries: Arc::new(Cache::try_with_capacity(0).unwrap()),
        base,
        game_state: 0,
        orders: crate::map_capture::Orders::for_test(&Default::default()),
        plans: Captured::with_capacities(&Capacities::default()),
        group_source: None,
    };
    if let Some(reason) = earlier {
        request.fail(reason);
    }
    request
}

/// The stitch leaves before jobs the census does not gate are dispatched, so
/// its own panic must not stop the pool before its join: the panic is
/// contained and reported, later submissions are still admitted, and the
/// join stops the pool, counts the panic and fails the census with the
/// acquisition reason, as a stitch submitted at the join's segment and
/// failing there did. An uncontained panic stopped the pool at once.
#[test]
fn a_stitch_panic_stops_the_pool_only_at_the_stitch_join() {
    use engine_native::walk::Outcome;
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let job =
        || acquisition::Job::OrdersStitch(stitch_request(acquisition::STITCH_PANIC_BASE, None));
    // SAFETY: the batch drains every accepted ticket before the pool drops.
    let pending = unsafe { submit_acquisition(&shared, &mut batch, job()) };
    assert!(matches!(pending, PendingAcquisition::Accepted(_)));
    batch.wait_ready(|batch| pending_complete(batch, &pending).then_some(()));
    assert!(!stitch_can_advance(&batch, &pending, false));
    assert!(stitch_can_advance(&batch, &pending, true));
    // SAFETY: as above.
    let completed = unsafe { finish_acquisition(&shared, &mut batch, pending) };
    let Ok(acquisition::Report::OrdersStitch(report)) = completed.report else {
        panic!("a contained stitch panic is a report");
    };
    assert!(report.contained_panic);
    assert!(!batch.stopped(), "the panic did not stop the pool");
    // A job dispatched before the stitch's join is still admitted.
    // SAFETY: as above.
    let later = unsafe { submit_acquisition(&shared, &mut batch, job()) };
    assert!(matches!(later, PendingAcquisition::Accepted(_)));
    // SAFETY: as above.
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
        calibration.roles[acquisition::Role::OrdersStitch as usize]
            .counters
            .panicked,
        1
    );
    // SAFETY: as above.
    let refused = unsafe { submit_acquisition(&shared, &mut batch, job()) };
    assert!(matches!(refused, PendingAcquisition::Rejected(_)));
    // SAFETY: as above.
    drop(unsafe { finish_acquisition(&shared, &mut batch, refused) });
}

/// A clean stitch can release fronts before unrelated joins, while a
/// rejected stitch retains its original failure barrier and refusal path.
#[test]
fn readiness_releases_clean_stitches_but_defers_rejected_stitches() {
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    // An earlier census refusal makes this body a no-read clean completion;
    // its original table-local reason must survive the early handoff.
    let job = acquisition::Job::OrdersStitch(stitch_request(0, Some("original census refusal")));
    // SAFETY: the batch drains every accepted ticket.
    let pending = unsafe { submit_acquisition(&shared, &mut batch, job) };
    batch.wait_ready(|batch| stitch_can_advance(batch, &pending, false).then_some(()));
    // SAFETY: as above.
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
    pool.shutdown().unwrap();
    // SAFETY: the stopped pool runs no job; rejected storage stays in Batch.
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
    // SAFETY: as above.
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

/// Publication, not collection order, releases the final borrowers. Parked
/// cells supply unpublished tickets without racing a native reader.
#[test]
fn final_inputs_wait_only_for_their_producers_and_prior_dispatch_admission() {
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let ticket = batch
        .park(&mut Some(acquisition::Job::OrdersStitch(stitch_request(
            0, None,
        ))))
        .unwrap();
    let unpublished = PendingAcquisition::Accepted(ticket);
    let rejected = PendingAcquisition::Rejected(ticket);
    // Residual Intel, capitulation, optional Session and Relations, Variants
    // and the character parts: every scheduled producer must publish.
    let producers = 5 + CHARACTER_PARTS - 1;
    crate::test_support::with_allocations_denied(|| {
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
        // A rejected producer is collected through its normal failure path;
        // an unrelated unpublished job is not part of this barrier.
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

/// A contained stitch panic that the callback never settles, because it
/// unwound before the stitch's join, is settled by the drain: counted once
/// and the pool stopped, unless the pool had already stopped.
#[test]
fn a_drained_stitch_panic_is_counted_and_stops_the_pool() {
    let shared = Shared::new(0, 0, 1);
    for stopped_first in [false, true] {
        let pool = acquisition::Pool::start(2, acquisition::MAX_JOBS).unwrap();
        {
            let mut batch = acquisition::Batch::new(&pool);
            let job = acquisition::Job::OrdersStitch(stitch_request(
                acquisition::STITCH_PANIC_BASE,
                None,
            ));
            // SAFETY: the batch drains every accepted ticket on drop.
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
                pool.shutdown().unwrap();
            }
            // Dropped uncollected, as on an unwinding callback.
        }
        let calibration = pool.calibration();
        assert_eq!(
            calibration.executor.panicked,
            u64::from(!stopped_first),
            "{stopped_first}"
        );
        assert_eq!(
            calibration.roles[acquisition::Role::OrdersStitch as usize]
                .counters
                .panicked,
            u64::from(!stopped_first)
        );
        assert!(pool.stopped());
    }
}

/// A stitch settles as if submitted at its join's segment. Where the pool had
/// already stopped there, it would have been refused: a stitch that did not
/// complete (cancelled while queued, or panicked) takes the admission reason
/// and counts no panic; one that completed keeps its census. Otherwise a
/// failure takes the acquisition reason. An earlier census reason stands.
#[test]
fn a_stitch_settles_with_the_reason_of_its_join_segment() {
    use engine_native::walk::Outcome;
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    pool.shutdown().unwrap();
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

/// The callback carries the order census split from each stitched hour to
/// the next through `Shared`, whichever pooled frame either hour uses, and
/// charges the parts the root census job prepares; an hour whose stitch
/// planned nothing leaves the next hour one part.
#[test]
fn the_callback_carries_the_order_split_to_the_next_hour_on_any_frame() {
    use crate::extended::bench_world::{self, Shape};
    use crate::map_capture::{MapCapacities, Orders};
    use crate::orders_parts::{MAX_PARTS, Part};
    let world = bench_world::build(
        40,
        Shape {
            named: true,
            theatre_links: true,
        },
    );
    let memory = world.slice();
    let shared = Shared::new(0, 0, 10_000_000);
    // One hour on `orders`, with `millis` of measured part work: the parts it
    // prepares from the callback's split and the bytes they charged; `fail`
    // fails it, as a failed job does, before its stitch.
    let hour = |orders: &mut Orders, millis: u64, fail: bool| {
        // SAFETY: this test is the split's only reader and writer.
        let before = unsafe { begin_order_parts(&shared, orders) };
        orders.walk_roots_growing(&memory, world.base);
        orders.prepare_parts();
        let pool = shared.pool_bytes.load(Ordering::Relaxed);
        charge_order_parts(&shared, orders, before);
        let charged = shared.pool_bytes.load(Ordering::Relaxed) - pool;
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
        // SAFETY: as above.
        unsafe { end_order_parts(&shared, orders) };
        (count, charged)
    };
    let mut frames = [
        Orders::for_test(&MapCapacities::default()),
        Orders::for_test(&MapCapacities::default()),
        Orders::for_test(&MapCapacities::default()),
    ];
    // The first hour walks one part and measures enough work for all of them.
    assert_eq!(hour(&mut frames[0], 3, false).0, 1);
    // The next, on a fresh frame, takes that split and charges its boxes.
    let (count, charged) = hour(&mut frames[1], 0, true);
    assert_eq!(count, MAX_PARTS);
    assert!(charged >= (MAX_PARTS * size_of::<Part>()) as u64);
    // It failed before planning: the next hour walks one part, on the first
    // frame, whose one part it already holds, so it charges nothing.
    assert_eq!(hour(&mut frames[0], 3, false), (1, 0));
    // And the hour after takes that hour's split on a third frame.
    assert_eq!(hour(&mut frames[2], 0, false).0, MAX_PARTS);
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
    // Storage admission succeeds; the original native failure remains an outcome.
    assert!(copy_divisions_inputs_refresh_growing(
        &memory, context, &mut job
    ));
    let reason = "division roster unreadable or invalid";
    let unavailable = Outcome::Unavailable { reason };
    let owner = Arc::clone(job.census.unit_inputs().unwrap());
    assert_eq!(owner.outcome(), unavailable);
    assert_eq!(owner.intel_sources().outcome, unavailable);
    assert_eq!(memory.attempts.get(), 1);
    let report = copy_divisions_core_refresh(&NoDivisionReads, context, &mut job);
    assert!(report.raw_ok);
    assert_eq!(job.source_outcome(usize::MAX), unavailable);
    assert_eq!(job.details.outcomes(), [unavailable; 5]);
    assert!(Arc::ptr_eq(&owner, job.census.unit_inputs().unwrap()));
    let repeated = copy_divisions_core_refresh(&NoDivisionReads, context, &mut job);
    assert_eq!(repeated.raw_bytes, report.raw_bytes);
    assert_eq!(repeated.copy_ticks, report.copy_ticks);
    assert_eq!(memory.attempts.get(), 1);
    let mut frame = Captured::new();
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
    use crate::division_census::UnitInputs;
    let bytes = kit::fixture();
    let memory = NoRosterRetry(&bytes);
    let countries = Arc::new(Cache::try_with_capacity(0).unwrap());
    let seed = carmy::DivisionSeed {
        pointer: kit::A1,
        roster_owner: Some(1),
        ..Default::default()
    };
    for inputs in [
        None,
        Some(Arc::new(UnitInputs::fixture(
            vec![seed],
            Outcome::Rows { len: 2 },
        ))),
        Some(Arc::new(UnitInputs::fixture(
            vec![seed],
            Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            },
        ))),
        Some(Arc::new(UnitInputs::fixture(
            Vec::new(),
            Outcome::Unavailable {
                reason: "original division input read failed",
            },
        ))),
    ] {
        let mut request = acquisition::PresentationRequest {
            dispatch: Default::default(),
            countries: Arc::clone(&countries),
            context: extended::division_presentation::CaptureContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour: 0,
            },
            captured: extended::division_presentation::Captured::vacant(),
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
            countries: Arc::clone(&countries),
            context: extended::fog::IntelCopyContext {
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
        let mut fog = extended::fog::Captured::vacant();
        fog.decode_intel_payload(&intel.payload, &mut [0; 1]);
        assert!(matches!(fog.outcomes()[4], Outcome::Unavailable { .. }));
    }
}

#[test]
fn missing_and_short_owned_division_source_packets_refuse_final_equipment_consumers() {
    use super::divisions_test_kit as kit;
    use extended::new_divisions::{Counters, Mark};
    let bytes = kit::fixture();
    let context = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    let countries = Arc::new(Cache::try_with_capacity(0).unwrap());
    let hour = replay_data::cadence::refresh_slot(TableId::DivisionEquipment).unwrap() as i32;
    // Exercise the final equipment consumers on their native slot; a full
    // refresh also acquires the independent fuel contributor roster.
    assert!(!replay_data::cadence::should_walk(
        TableId::CountryFuel,
        hour,
        false
    ));
    for displaced_length in [0, 1] {
        let mut job = DivisionsJob::new_worker();
        assert!(copy_divisions_inputs_refresh_growing(
            &kit::memory(&bytes),
            context,
            &mut job
        ));
        assert!(copy_divisions_core_refresh(&kit::memory(&bytes), context, &mut job).raw_ok);
        assert_eq!(job.source_outcome(usize::MAX), Outcome::Rows { len: 3 });
        let mut sink = extended::Captured::with_capacities(&extended::Capacities {
            division_sources: displaced_length,
            ..Default::default()
        });
        // The original packet has moved; its displaced zero/short buffer cannot
        // masquerade as a second complete observation of the same census.
        job.supply_sources(&mut sink, usize::MAX, None);
        let mut frame = Captured::new();
        job.supply_sources(&mut frame.extended, usize::MAX, None);
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
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS).unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let mut selection = crate::slicing::SliceSelection::new(true);
    selection.reset(5, false);
    let mut payload = extended::intel::RelationsJob::new();
    payload.selection = Some(selection);
    payload.fingerprints = Some(Box::new(crate::slicing::SliceFingerprints::new()));
    let store = payload.fingerprints.as_deref().unwrap() as *const _;
    let request = acquisition::Job::Relations(acquisition::RawRequest {
        countries: Arc::new(Cache::try_with_capacity(1).unwrap()),
        context: extended::intel::RelationsCopyContext {
            base: 0,
            game_state: usize::MAX,
            active: [true; 2],
            opinion_cap: 4,
            tuple_cap: 4,
            epoch: 0,
        },
        payload,
    });
    let rejected = park(&mut batch, request);
    let reads = crate::read_accounting::Capture::new();
    let completed = crate::test_support::with_allocations_denied(|| {
        // SAFETY: the rejected request acquired no native state; it must only
        // return its owned storage for the caller's named unavailable outcome.
        unsafe { finish_acquisition(&shared, &mut batch, rejected) }
    });
    assert!(matches!(
        completed.report,
        Err(acquisition::Failure::Stopped)
    ));
    assert_eq!(completed.table_reads, crate::read_accounting::ZERO);
    assert_eq!(reads.finish(), crate::read_accounting::ZERO);
    let acquisition::Job::Relations(request) = completed.request else {
        panic!("rejected worker lost its payload");
    };
    let mut captured = extended::intel::Captured::vacant();
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
    let mut frame = Captured::new();
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
fn template_model_failures_fold_by_reason_and_keep_the_first() {
    use extended::division_templates::{MODEL_REASONS, ModelFailures};
    let hour = |reasons: &[(&'static str, u32)]| {
        let mut failures = ModelFailures::default();
        for (slot, reason) in failures.reasons.iter_mut().zip(reasons) {
            *slot = *reason;
            failures.rows += reason.1;
        }
        failures
    };
    let mut stats = TemplateModelStats::default();
    stats.fold(hour(&[]));
    assert_eq!((stats.hours, stats.rows, stats.first_reason), (0, 0, None));
    stats.fold(hour(&[("unreadable database", 3)]));
    stats.fold(hour(&[("bad exclusion", 1), ("unreadable database", 2)]));
    assert_eq!((stats.hours, stats.rows), (2, 6));
    assert_eq!(stats.first_reason, Some("unreadable database"));
    assert_eq!(
        &stats.reasons[..2],
        &[("unreadable database", 5), ("bad exclusion", 1)]
    );
    // Reasons past the named slots are counted together.
    let names: Vec<&'static str> = (0..MODEL_REASONS + 2)
        .map(|index| &*Box::leak(format!("reason {index}").into_boxed_str()))
        .collect();
    let mut full = TemplateModelStats::default();
    for name in names {
        full.fold(hour(&[(name, 1)]));
    }
    assert_eq!((full.rows, full.other), (MODEL_REASONS as u64 + 2, 2));
}

#[test]
fn the_daily_hour_stores_both_id_counters_and_later_hours_compare_by_handle_type() {
    use extended::new_divisions::{Counters, Mark};
    struct Counterset(i32, u32);
    impl Memory for Counterset {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let bytes = match address {
                engine_native::cship::NEXT_ID_RVA => self.0.to_le_bytes(),
                engine_native::creference_registry::ID_COUNTER_RVA => self.1.to_le_bytes(),
                _ => return false,
            };
            if output.len() != 4 {
                return false;
            }
            output.copy_from_slice(&bytes);
            true
        }
    }
    let shared = Shared::new(0, 0, 1);
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
    // A trained division carries a dynamic type minted from the registry.
    assert!(counters.minted_after((7_001 << 32) | 0x1268));
    assert!(!counters.minted_after((41 << 32) | 0x1268));
    // The unit counter's own value is the next unit id.
    assert!(!counters.minted_after((39 << 32) | u64::from(engine_native::cship::HANDLE_TYPE)));
    assert!(counters.minted_after((40 << 32) | u64::from(engine_native::cship::HANDLE_TYPE)));
    // An unreadable counter at the daily walk leaves the next hours unknown.
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
fn order_counter_capture_gates_and_clears_recycled_metadata_without_allocation() {
    let mut bytes = vec![0_u8; 0x4000];
    let mut put = |address: usize, value: &[u8]| {
        bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
    };
    for (offset, data, count) in [
        (cgame_state::COUNTRIES, 0x1500_u64, 2_u32),
        (cgame_state::COUNTRY_LINKS, 0x1600, 2),
    ] {
        put(0x1000 + offset, &data.to_le_bytes());
        put(0x1000 + offset + 8, &count.to_le_bytes());
        put(0x1000 + offset + 12, &count.to_le_bytes());
    }
    put(0x1508, &0x2000_u64.to_le_bytes());
    put(0x1604, &1_u32.to_le_bytes());
    put(0x2000, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
    put(0x2008, &1_u32.to_le_bytes());
    put(
        0x2000 + ccountry::ORDER_INSTANCE_COUNTER,
        &3812_u32.to_le_bytes(),
    );
    let memory = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut cache = Cache::try_with_capacity(32).unwrap();
    let cached = cache.prepare(&memory, 0, 0x1000);
    assert_eq!(cached.seed_status(), SeedStatus::Ready);
    let mut sample = OrderCounterCapture::with_capacity(2);
    let pointer = sample.rows.as_ptr();
    let capacity = sample.rows.capacity();
    sample.capture(
        true,
        &memory,
        engine_native::Memory::authenticated_countries(&cached, 0, 0x1000),
    );
    assert!(sample.forced);
    assert!(sample.read.unwrap().complete());
    assert_eq!(sample.read.unwrap().len, 1);
    assert_eq!(sample.rows[0].value, Some(3812));
    sample.qpc_begin = 10;
    sample.qpc_end = 12;
    sample.elapsed_ns = 20;
    sample.capture(
        false,
        &memory,
        engine_native::Memory::authenticated_countries(&cached, 0, 0x1000),
    );
    assert!(!sample.forced);
    assert_eq!(sample.read, None);
    assert_eq!(
        (sample.qpc_begin, sample.qpc_end, sample.elapsed_ns),
        (0, 0, 0)
    );
    sample.capture(true, &memory, None);
    assert!(sample.forced);
    assert_eq!(sample.read, None);
    assert_eq!(sample.rows.as_ptr(), pointer);
    assert_eq!(sample.rows.capacity(), capacity);
}

#[test]
fn order_counter_capture_accounts_actual_capacity_and_grows_only_its_storage() {
    let mut sample = OrderCounterCapture::with_capacity(2);
    assert_eq!(
        sample.allocated_bytes(),
        sample.rows.capacity() * size_of::<ccountry::OrderInstanceCounter>()
    );
    sample.read = Some(ccountry::OrderInstanceCounterRead {
        len: 2,
        required: 5,
        unreadable: 0,
    });
    let original = sample.allocated_bytes();
    sample.grow(3);
    assert_eq!(sample.rows.len(), 5);
    assert!(sample.allocated_bytes() > original);
    assert_eq!(
        sample.allocated_bytes(),
        sample.rows.capacity() * size_of::<ccountry::OrderInstanceCounter>()
    );
    let grown = sample.allocated_bytes();
    sample.reset();
    sample.grow(1);
    assert_eq!(sample.allocated_bytes(), grown);
    assert_eq!(sample.rows.len(), 5);
}

#[test]
fn divisions_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
    use crate::raw_memory::{Capacity, Charge, Tracker};
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
    let mut frame = Captured::new();
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
fn decode_queue_bound_matches_pool_invariant() {
    let shared = Shared::new(0, 0, 1);
    assert_eq!(shared.decode_queue.capacity(), BUFFERS);
    assert_eq!(shared.free.capacity(), BUFFERS);
    assert_eq!(shared.ready.capacity(), BUFFERS);
}

#[test]
fn a_dropped_frame_invalidates_its_sliced_reads_and_a_clockless_one_does_not() {
    use crate::slicing::{RESEARCH, SliceSelection};
    let shared = Shared::new(0, 0, 1);
    // SAFETY: test thread only; nothing else holds the store.
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
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.extended.prepare_hour(7, false);
        frame.clock = clock;
        drop_owned_frame(&shared, frame, index);
        // Hour 7's slice (owner 7) was not published: read it again at hour 8.
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
    use crate::slicing::{RAID_STATE, RESEARCH, SliceSelection};
    let shared = Shared::new(0, 0, 1);
    // SAFETY: test thread only; nothing else holds the stores.
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
    // The spool withheld a converted research read.
    shared
        .forget_families
        .fetch_or(1 << RESEARCH, Ordering::Release);
    forget_unpublished_families(&shared);
    assert_eq!(early_at(RESEARCH, 2), 2);
    assert_eq!(early_at(RAID_STATE, 2), 0);
    // Once only.
    forget_unpublished_families(&shared);
    assert_eq!(early_at(RESEARCH, 3), 0);
}

#[test]
fn decode_queue_full_takes_the_existing_gap_branch_with_existing_counters() {
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    for seq in 0..BUFFERS {
        shared
            .decode_queue
            .push(decode::DecodeJob {
                index,
                seq: seq as u64,
                hour: 0,
                relations_refresh: false,
                characters_refresh: [false; CHARACTER_PARTS],
                projects_refresh: false,
                lexical_refresh: false,
                divisions_refresh: false,
                air_wing_refresh: false,
                air_stats_refresh: false,
                naval_refresh: false,
                players_refresh: false,
                session_refresh: false,
                production_rest_refresh: false,
                construction_rest_refresh: false,
                economy_rest_refresh: false,
                air_bases_refresh: false,
                supremacy_refresh: false,
                sunk_refresh: false,
                radar_refresh: false,
                intel_refresh: false,
                weather_refresh: false,
                capitulation_refresh: false,
                force_full: false,
                game_elapsed: 1,
                relations_payload: None,
                characters_payload: [const { None }; CHARACTER_PARTS],
                projects_payload: None,
                lexical_payload: None,
                divisions_payload: None,
                air_wing_payload: None,
                air_stats_payload: None,
                naval_payload: None,
                players_payload: None,
                session_payload: None,
                production_rest_payload: None,
                construction_rest_payload: None,
                economy_rest_payload: None,
                air_bases_payload: None,
                supremacy_payload: None,
                sunk_payload: None,
                radar_payload: None,
                intel_payload: None,
                weather_payload: None,
                capitulation_payload: None,
            })
            .unwrap();
    }
    // A decode FIFO filled with duplicate leases is an invariant violation.
    // Keep its explicit failure path rather than losing an acquired frame silently.
    let extra = decode::DecodeJob {
        index,
        seq: u64::MAX,
        hour: 0,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 1,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    assert!(shared.decode_queue.push(extra).is_err());
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    frame.clock = Some(clock(42, 7));
    let dropped_before = shared.dropped.load(Ordering::Relaxed);
    drop_owned_frame(&shared, frame, index);
    assert_eq!(shared.dropped.load(Ordering::Relaxed), dropped_before + 1);
    assert_eq!(shared.ready.len(), 0);
    assert_eq!(shared.free.len(), BUFFERS);
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
fn cold_core_and_division_worker_buffers_have_no_chosen_population_capacity() {
    assert_eq!(INITIAL_CAPACITIES, [0; 12]);
    let frame = Captured::with_capacities(
        &INITIAL_CAPACITIES,
        &MapCapacities::default(),
        &extended::Capacities {
            division_sources: 0,
            ..Default::default()
        },
    )
    .unwrap();
    for capacity in [
        frame.order_counters.rows.capacity(),
        frame.countries.capacity(),
        frame.provinces.rows.capacity(),
        frame.provinces.conflicts.capacity(),
        frame.provinces.groups.capacity(),
        frame.states.capacity(),
        frame.state_names.capacity(),
        frame.divisions.capacity(),
        frame.wars.capacity(),
        frame.factions.capacity(),
        frame.faction_names.capacity(),
        frame.faction_members.capacity(),
    ] {
        assert_eq!(capacity, 0);
    }
    assert_eq!(frame.division_fallback.allocated_bytes(), 0);
    let job = DivisionsJob::new_worker();
    assert_eq!(job.census.allocated_bytes(), 0);
    assert_eq!(job.census.source_requirement(), 0);
    assert_eq!(job.details.allocated_bytes(), 0);
    assert_eq!(job.payload_bytes(), [0; 6]);
}

#[test]
fn pool_initialization_allocates_all_slots_and_counts_actual_bytes() {
    let shared = Shared::new(0, 0, 1);
    assert_eq!(shared.free.capacity(), BUFFERS);
    assert_eq!(shared.ready.capacity(), BUFFERS);
    assert_eq!(shared.free.len(), BUFFERS);
    let mut bytes = unsafe { (&*shared.roster.get()).as_ref().unwrap() }.allocated_bytes();
    bytes += unsafe { &*shared.country_cache.get() }.allocated_bytes();
    // One CountryIntel buffer and one held RaidAirScores table for the pool.
    bytes += shared.country_intel.allocated_bytes();
    bytes += unsafe { (&*shared.raid_air_scores.get()).as_ref().unwrap() }.allocated_bytes();
    bytes += unsafe { &*shared.fixed_layout_buffer.get() }.capacity();
    for slot in &shared.frames {
        let frame = slot.load(Ordering::Acquire);
        assert!(!frame.is_null());
        let frame = unsafe { &*frame };
        let home = frame.extended.country_intel_home().unwrap();
        assert!(home.shares(&shared.country_intel));
        bytes += frame.allocated_bytes();
    }
    assert_eq!(shared.pool_bytes.load(Ordering::Relaxed), bytes as u64);
}

#[test]
fn exhausted_pool_reuses_a_returned_frame_without_allocating_or_dropping() {
    use std::sync::mpsc;
    let shared = Arc::new(Shared::new(0, 0, 1));
    let held: Vec<_> = (0..BUFFERS)
        .map(|_| {
            let mut wait_ns = u64::MAX;
            let index = shared.take_frame(&mut wait_ns).unwrap();
            assert_eq!(wait_ns, 0);
            index
        })
        .collect();
    let original = shared.frames[held[0]].load(Ordering::Acquire);
    unsafe {
        (*original).qpc = 42;
    }
    let bytes = shared.pool_bytes.load(Ordering::Relaxed);
    let (send, receive) = mpsc::channel();
    let consumer = Arc::clone(&shared);
    let waiter = std::thread::spawn(move || {
        let mut wait_ns = 0;
        let result = consumer.take_frame(&mut wait_ns);
        send.send((result, wait_ns)).unwrap();
    });
    // Nothing is returned until the waiter is counted in the wait path, so
    // it can only leave early by not waiting at all, which ends the thread.
    while shared.buffer_wait_calls.load(Ordering::Relaxed) == 0 && !waiter.is_finished() {
        std::thread::yield_now();
    }
    let waited = shared.buffer_wait_calls.load(Ordering::Relaxed);
    let pending = receive.try_recv().is_err();
    shared.recycle_frame(held[0]);
    let result = receive.recv();
    shared.cancel_buffer_wait.store(true, Ordering::Release);
    waiter.join().unwrap();
    assert_eq!(waited, 1);
    assert!(pending);
    let (index, wait_ns) = result.unwrap();
    assert_eq!(index, Some(held[0]));
    assert_eq!(wait_ns, shared.buffer_wait_ns.load(Ordering::Relaxed));
    assert_eq!(wait_ns, shared.buffer_wait_max_ns.load(Ordering::Relaxed));
    assert_eq!(shared.frames[held[0]].load(Ordering::Acquire), original);
    assert_eq!(unsafe { (*original).qpc }, 42);
    assert_eq!(shared.pool_bytes.load(Ordering::Relaxed), bytes);
    assert_eq!(shared.dropped.load(Ordering::Relaxed), 0);
    for index in held {
        shared.recycle_frame(index);
    }
    assert_eq!(shared.free.len(), BUFFERS);
}

#[test]
fn hourly_join_tally_keeps_first_slowest_counts_stalls_and_resets() {
    let record = |role, wait, stall, growths| {
        record_hour_join(
            role,
            wait,
            stall,
            growths,
            #[cfg(feature = "callback_profile")]
            crate::acquisition::Profile::default(),
        )
    };
    reset_hour_joins();
    assert_eq!(take_hour_joins(), HourJoins::default());
    record(0, 10, JoinCosts::STALL_NS[1] - 1, 2);
    record(1, 9, JoinCosts::STALL_NS[1], 3);
    record(2, 10, JoinCosts::STALL_NS[1] * 4, 5);
    record_caller_growths(3);
    let joins = take_hour_joins();
    assert_eq!(
        joins.slowest,
        Some(SlowestJoin {
            role: 0,
            wait_ns: 10,
            collect_stall_ns: JoinCosts::STALL_NS[1] - 1,
        })
    );
    assert_eq!(joins.collect_stalls_over_1ms, 2);
    assert_eq!(joins.growths, 10);
    assert_eq!(joins.total_growths(5), 12);
    assert_eq!(take_hour_joins(), HourJoins::default());
    #[cfg(feature = "callback_profile")]
    assert_eq!(take_profile_join().role, Some(0));
    record(3, 11, 0, 1);
    record(4, 12, 0, 0);
    record(5, 12, 0, 0);
    assert_eq!(take_hour_joins().slowest.unwrap().role, 4);
    record(6, 13, JoinCosts::STALL_NS[1], 7);
    reset_hour_joins();
    assert_eq!(take_hour_joins(), HourJoins::default());
    #[cfg(feature = "callback_profile")]
    assert!(take_profile_join().role.is_none());
}

#[test]
fn exhausted_pool_wait_cancels_without_a_live_consumer() {
    use std::{sync::mpsc, time::Duration};
    let shared = Arc::new(Shared::new(0, 0, 1));
    let held: Vec<_> = (0..BUFFERS)
        .map(|_| shared.take_frame(&mut 0).unwrap())
        .collect();
    let (send, receive) = mpsc::channel();
    let consumer = Arc::clone(&shared);
    let waiter = std::thread::spawn(move || {
        send.send(consumer.take_frame(&mut 0)).unwrap();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while shared.buffer_wait_calls.load(Ordering::Relaxed) == 0
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    shared.cancel_buffer_wait.store(true, Ordering::Release);
    let result = receive.recv_timeout(crate::TEST_HANG_GUARD);
    waiter.join().unwrap();
    assert_eq!(result.unwrap(), None);
    for index in held {
        shared.recycle_frame(index);
    }
    // Cancellation does not discard a frame that is already available.
    assert!(shared.take_frame(&mut 0).is_some());
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

    let shared = Shared::new(0, 0, 1);
    let index = shared.take_frame(&mut 0).unwrap();
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
    assert!(shared.cancel_buffer_wait.load(Ordering::Acquire));
    assert_eq!(shared.acquisition_panics.load(Ordering::Acquire), 1);
    assert_eq!(shared.free.len(), BUFFERS - 1);
    assert_eq!(shared.take_frame(&mut 0), None);
    assert_eq!(shared.buffer_wait_calls.load(Ordering::Relaxed), 0);

    // During a delayed hook restore, record the next hour as a loss without
    // running acquisition again or consuming another capture buffer.
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

    // Cancellation also works when no healthy frame remains. Do not
    // reclaim the partially restored frame as if acquisition succeeded.
    while shared.free.pop().is_some() {}
    assert_eq!(shared.take_frame(&mut 0), None);
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 0);
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
fn end_of_hour_losses_record_the_entry_hour_as_the_lost_and_loaded_hour() {
    assert_eq!(
        SAMPLE_BOUNDARY,
        replay_data::identity::SampleBoundary::EndOfHour
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
    let mut frame = Captured::new();
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
    // The state display-name arena grows to what the walk asked for.
    frame.state_names_required = 40_000;
    frame.grow(&mut capacities);
    assert_eq!(frame.wars.len(), 2048);
    assert_eq!(frame.factions.len(), 128);
    assert_eq!(frame.faction_members.len(), 2048);
    assert_eq!(frame.faction_names.len(), 32768);
    assert!(frame.state_names.len() >= 40_000);
    assert_eq!(frame.state_names.len(), capacities[11]);
    assert!(frame.allocated_bytes() > before);
    let mut idle = Captured::new();
    idle.grow(&mut capacities);
    assert_eq!(idle.allocated_bytes(), frame.allocated_bytes());
}

#[test]
fn fallback_census_growth_consumes_the_failed_attempt_once() {
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let mut frame = Captured::new();
    frame.division_fallback = crate::division_census::Census::new_worker();
    frame
        .division_fallback
        .capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
    assert!(frame.division_fallback.overflowed);
    let mut capacities = INITIAL_CAPACITIES;
    frame.grow(&mut capacities);
    assert!(frame.division_fallback.initialized());
    assert!(!frame.division_fallback.overflowed);
    let grown = frame.division_fallback.allocated_bytes();
    assert!(grown > 0);

    // A subsequent frame that never used the fallback carries no new
    // overflow request and must retain, rather than double, its storage.
    frame.grow(&mut capacities);
    assert_eq!(frame.division_fallback.allocated_bytes(), grown);

    frame
        .division_fallback
        .capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
    assert_eq!(
        frame.division_fallback.outcome(64),
        Outcome::Rows { len: 3 },
    );
    let lease = Arc::clone(frame.division_fallback.unit_inputs().unwrap());
    frame.extended.install_unit_inputs(Some(Arc::clone(&lease)));
    assert_eq!(Arc::strong_count(&lease), 3);
    frame.extended.release_unit_inputs();
    assert_eq!(Arc::strong_count(&lease), 2);
    drop(lease);
    frame.grow(&mut capacities);
    assert_eq!(frame.division_fallback.allocated_bytes(), grown);
}

fn cadence_clock(hour: i32, signal: i64) -> Clock {
    let mut clock = clock(hour, signal);
    clock.campaign_id = Owned {
        bytes: b"campaign".to_vec(),
        len: 8,
    };
    clock
}

// Production prepares the campaign copy before the callback.
fn prepared_cadence() -> CadenceState {
    let mut cadence = CadenceState::default();
    cadence.prev_campaign.prepare(8).unwrap();
    cadence
}

#[test]
fn cadence_rotation_walks_recorded_families_once_per_24_hours() {
    use replay_data::rows::TableId;
    use replay_data::{cadence::Cadence, cadence::cadence, cadence::is_refresh};
    // A retired table keeps its cadence but is never walked.
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
                    // A sliced table reads one owner slice every hour.
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
    cadence.prev_world = crate::world_lifetime::generation() + 1;
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
    use crate::politics_changes::{self as politics, Event, LaneCursor, StagingFixture};
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
    let mut cursors = [LaneCursor::default()];
    let handoff = fixture.handoff(&mut cursors).unwrap();
    let shared = Shared::new(0, 0, 1);
    let mut cadence = prepared_cadence();
    cadence.force_full(&cadence_clock(61320026, 1).borrowed());
    shared.request_retry_full();
    let clock = cadence_clock(61320027, 1);
    let continuous = cadence.continuous(&clock.borrowed());
    let recovery = shared.force_next.swap(false, Ordering::AcqRel);
    let refresh = shared.refresh_next.swap(false, Ordering::AcqRel);
    assert!(cadence.force_full(&clock.borrowed()) | recovery | refresh);
    if !continuous || recovery {
        politics::invalidate();
    }
    let mut events = [Event::default()];
    let mut bytes = [0; 128];
    let drained = handoff.drain(&cursors, &mut events, &mut bytes).unwrap();
    assert_eq!(drained.copied, 1);
    assert_eq!(
        (events[0].country, events[0].before, events[0].after),
        (5_000, 100, 73)
    );
    assert!(!shared.force_next.load(Ordering::Acquire));
    assert!(!shared.refresh_next.load(Ordering::Acquire));
}

#[test]
fn wars_table_is_hourly_tier_h_with_no_refresh_gate() {
    // WarRelations keeps its hourly cadence: no stagger slot, every hour
    // walks. This pins the unchanged cadence for typed capture.
    use replay_data::rows::TableId;
    assert!(!replay_data::cadence::is_staggered(TableId::WarRelations));
    for hour in [0, 1, 2, 3, 60_820_531] {
        assert!(replay_data::cadence::is_refresh(
            TableId::WarRelations,
            hour
        ));
    }
}

#[test]
fn wars_typed_walk_preserves_primary_native_rows() {
    use super::wars_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let split = kit::direct(&memory, 64);
    // Spot-checks: the owned walk emits each war once from its first
    // endpoint, retaining native signed values.
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
fn wars_typed_capacities_keep_full_validation_and_exact_unread_counts() {
    use super::wars_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for cap in [0usize, 1, 2, 64] {
        let direct = kit::direct(&memory, cap);
        let expected = if cap >= 2 {
            Outcome::Rows { len: 2 }
        } else {
            Outcome::Truncated {
                len: cap,
                unread_rows: (2 - cap) as u32,
            }
        };
        assert_eq!(direct.owned_outcome, expected);
    }
}

#[test]
fn wars_primary_faults_preserve_native_validation_scope() {
    use super::wars_test_kit as kit;
    use engine_native::{ccountry, cdiplomacy_status, cgame_state, crelation, memory::SliceMemory};
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
    // The primary census validates each used link and ignores unrelated slots.
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
fn divisions_core_is_hourly_and_five_details_refresh_on_daily_slot_two() {
    use replay_data::{cadence::is_refresh, cadence::refresh_slot, cadence::should_walk};

    for table in [
        TableId::Divisions,
        TableId::DivisionMovement,
        TableId::DivisionStatus,
    ] {
        assert_eq!(refresh_slot(table), None, "{table:?}");
        for hour in [-25, -22, -1, 0, 1, 2, 3, 26, 60_820_531] {
            assert!(is_refresh(table, hour), "{table:?} at {hour}");
            assert!(should_walk(table, hour, true));
        }
    }
    for table in crate::decode::DIVISIONS.into_iter().skip(1) {
        assert_eq!(refresh_slot(table), Some(2), "{table:?}");
        for hour in -48i32..48 {
            let expected = hour.rem_euclid(24) == 2;
            assert_eq!(is_refresh(table, hour), expected, "{table:?} at {hour}");
            assert_eq!(should_walk(table, hour, false), expected);
            assert!(should_walk(table, hour, true));
        }
    }
}

#[test]
fn divisions_split_copy_matches_direct_walk() {
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory, [64; 6]);
    let (split, report, _) = kit::split(&memory, [64; 6]);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    // Spot-checks: null location/template/orders read as zero rows.
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
fn divisions_split_copy_matches_direct_walk_under_truncation() {
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory, [1; 6]);
    let (split, report, _) = kit::split(&memory, [1; 6]);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    // A truncated divisions walk makes every dependent census
    // incomplete on both paths with the identical reason.
    assert_eq!(
        split.divisions_outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    for outcome in [
        split.equipment_outcome,
        split.groups_outcome,
        split.requests_outcome,
        split.deliveries_outcome,
        split.produced_outcome,
    ] {
        assert_eq!(
            outcome,
            Outcome::Unavailable {
                reason: "source census incomplete"
            }
        );
    }
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
    // (fault address, expected independent table verdicts in group order).
    // Outcomes including reason strings must match the typed walks
    // exactly, including each independent table fault.
    // Shared census failures block every dependent table.
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
        // V1 backs equipment entries and the first request row.
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
            // The native delivery header independently authenticates its
            // produced pool before attempting this optional status read.
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
        let (split, report, _) = kit::split(&memory, [64; 6]);
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
            if address == carmy::COUNTER_ATTRITION_THRESHOLD_RVA && output.len() == 8 {
                output.copy_from_slice(&500_i64.to_le_bytes());
                return true;
            }
            kit::memory(&self.0).copy(address, output)
        }
    }
    let mut bytes = kit::fixture();
    // Null map entities and disabled training need no render/global reads.
    // Per-division effective stats are the only extra heap input.
    let stats = 0x65000_usize;
    bytes[stats..stats + 8].copy_from_slice(&(stats::VTABLE_RVA as u64).to_le_bytes());
    bytes[stats + stats::MAXIMUM_STRENGTH..stats + stats::MAXIMUM_STRENGTH + 8]
        .copy_from_slice(&500_000_i64.to_le_bytes());
    bytes[stats + stats::MAXIMUM_ORGANISATION..stats + stats::MAXIMUM_ORGANISATION + 8]
        .copy_from_slice(&700_000_i64.to_le_bytes());
    for army in [kit::A1, kit::A2, kit::A3] {
        bytes[army + cunit::STAT_BLOCK..army + cunit::STAT_BLOCK + 8]
            .copy_from_slice(&(stats as u64).to_le_bytes());
        let acclimatization = army + carmy::ACCLIMATIZATION;
        bytes[acclimatization..acclimatization + 8].copy_from_slice(
            &(engine_native::acclimatization_data::VTABLE_RVA as u64).to_le_bytes(),
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
    let report = copy_divisions_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    assert_eq!(first.source_outcome(64), Outcome::Rows { len: 3 });
    let (first_sources, _) = first.census.sources(64);
    assert_eq!(first_sources[0].division, 101);
    assert_eq!(first_sources[0].template, 0x3455_0000_0034);
    assert_eq!(first_sources[0].country, 1);
    let mut frame = Captured::new();
    first.supply_sources(&mut frame.extended, frame.divisions.len(), None);
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
    assert!(
        status
            .iter()
            .all(|r| r.maximum_hit_points == 500_000 && r.maximum_organisation == 700_000)
    );
    // The actual worker installation preserves acquired live tables and
    // installs the independently captured native detail rows.
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
fn divisions_copied_sources_distinguish_semantic_faults_capacity_and_empty_census() {
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
        let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first);
        assert!(report.raw_ok); // No overflow is not a successful census.
        assert!(matches!(
            first.source_outcome(64),
            Outcome::Unavailable { .. }
        ));
        if fault == kit::A2 || fault == kit::A2 + carmy::STANDARD_TEMPLATE {
            assert_eq!(
                first.source_outcome(1),
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 2
                }
            );
        }
    }
    let mut bytes = kit::fixture();
    for address in [kit::CTAB + 8, kit::CTAB + 16, kit::CTAB + 24] {
        bytes[address..address + 8].fill(0);
    }
    let mut first = DivisionsJob::new();
    let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first);
    assert!(report.raw_ok);
    assert!(matches!(
        first.source_outcome(0),
        Outcome::Unavailable { .. }
    ));
    // The pointer census is empty and complete for independent consumers;
    // core division sources still refuse the missing-country world above.
    assert_eq!(first.census.census_outcome(), Outcome::Rows { len: 0 });
    let mut bytes = kit::fixture();
    for country in [kit::C0, kit::C1] {
        bytes[country + ccountry::DIVISIONS..country + ccountry::DIVISIONS + 16].fill(0);
    }
    copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first);
    assert_eq!(first.source_outcome(0), Outcome::Rows { len: 0 });
    assert!(first.census.sources(64).0.is_empty()); // Recycled rows were cleared.
    first.census = crate::division_census::Census::default();
    let bytes = kit::fixture();
    let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first);
    assert!(!report.raw_ok && first.census.overflowed);
    assert!(matches!(
        first.source_outcome(64),
        Outcome::Unavailable { .. }
    ));
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
    let mut census = crate::division_census::Census::new();
    census.capture(&Fault(&bytes), kit::BASE, kit::GAME_STATE, 64);
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
        census.rows(64).1,
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
    let mut census = crate::division_census::Census::new();
    census.capture(&memory, kit::BASE, kit::GAME_STATE, 64);
    let direct =
        engine_visibility::unit_viewer_entry::walk(&memory, kit::BASE, kit::GAME_STATE, &mut []);
    assert_eq!(direct, Outcome::Rows { len: 0 });
    let shared = engine_visibility::unit_viewer_entry::walk_with_divisions(
        &memory,
        kit::BASE,
        kit::GAME_STATE,
        &mut [],
        Some(census.intel_sources()),
    );
    assert_eq!(shared, direct);
    assert_eq!(
        census.rows(0).1,
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
    let mut census = crate::division_census::Census::new();
    census.capture(&memory, kit::BASE, kit::GAME_STATE, 64);
    assert_eq!(census.rows(64).1, direct.divisions_outcome);
}

#[test]
fn divisions_seeded_worker_preserves_primary_trace_and_never_repeats_core() {
    use super::divisions_test_kit as kit;
    use crate::read_accounting;
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
    let inputs = Arc::clone(payload.census.unit_inputs().unwrap());
    let report = copy_divisions_core_refresh(&memory, ctx, &mut payload);
    assert!(report.raw_ok);
    assert!(Arc::ptr_eq(&inputs, payload.census.unit_inputs().unwrap()));
    let accounted = read_accounting::snapshot();
    let repeated = copy_divisions_core_refresh(&NoRead, ctx, &mut payload);
    assert_eq!(repeated.raw_bytes, report.raw_bytes);
    assert_eq!(repeated.copy_ticks, report.copy_ticks);
    assert_eq!(read_accounting::snapshot(), accounted);
    let mut composed = DivisionsJob::new();
    let original = copy_divisions_refresh(&composed_memory, ctx, &mut composed);
    assert_eq!(report.raw_bytes, original.raw_bytes);
    assert_eq!(
        payload.census.rows(ctx.row_capacity),
        composed.census.rows(ctx.row_capacity)
    );
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
    let mut actual = Captured::new();
    let mut expected = Captured::new();
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
        row_capacity: 64,
        ..Default::default()
    };
    let mut job = DivisionsJob::new();
    assert!(copy_divisions_inputs_refresh(
        &kit::memory(&bytes),
        context,
        &mut job,
    ));
    let lease = Arc::clone(job.census.unit_inputs().unwrap());
    let before = job.payload_bytes();
    let changed = DivisionsCopyContext {
        row_capacity: 0,
        ..context
    };
    assert!(!copy_divisions_core_refresh(&NoRead, changed, &mut job).raw_ok);
    assert_eq!(job.payload_bytes(), before);
    assert!(Arc::ptr_eq(&lease, job.census.unit_inputs().unwrap()));
    assert!(!copy_divisions_core_refresh(&NoRead, context, &mut job).raw_ok);
}

#[test]
fn divisions_staging_overflow_falls_back_to_typed_walk() {
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        ..Default::default()
    };
    let mut first = DivisionsJob::new();
    first.census = crate::division_census::Census::default();
    let report = copy_divisions_refresh(&memory, ctx, &mut first);
    assert!(!report.raw_ok);
    assert!(first.grow_for_next());
    // The typed walk still produces the identical rows through the
    // existing path, so the fallback keeps every row.
    let direct = kit::direct(&memory, [64; 6]);
    assert_eq!(direct.divisions_outcome, Outcome::Rows { len: 3 });
    assert_eq!(direct.equipment_outcome, Outcome::Rows { len: 3 });
    assert_eq!(direct.groups_outcome, Outcome::Rows { len: 3 });
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
        row_capacity: 64,
        source_capacity: 64,
        detail_capacities: [usize::MAX; 5],
        details_active: false,
    };
    let mut job = DivisionsJob {
        census: crate::division_census::Census::new(),
        ..Default::default()
    };
    let reads = crate::read_accounting::Capture::new();
    assert!(copy_divisions_inputs_refresh(&memory, context, &mut job));
    let lease = Arc::clone(job.census.unit_inputs().unwrap());
    let report = copy_divisions_core_refresh(&memory, context, &mut job);
    assert!(report.raw_ok);
    assert!(Arc::ptr_eq(&lease, job.census.unit_inputs().unwrap()));
    assert_eq!(job.details.allocated_bytes(), 0);
    let skipped = Outcome::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON,
    };
    assert_eq!(job.details.outcomes(), [skipped; 5]);
    assert_eq!(report.copy_ticks[1..], [0; 5]);
    assert_eq!(report.raw_bytes[1..], [0; 5]);
    let counts = reads.finish();
    assert!(counts[TableId::Divisions.index()].spans > 0);
    for (index, counts) in counts.iter().enumerate() {
        if index != TableId::Divisions.index() {
            assert_eq!(*counts, ReadCounts::ZERO);
        }
    }
    let mut census = crate::division_census::Census::new();
    census.capture_inputs(&reference, context.base, context.game_state);
    census.capture_core(&reference, context.row_capacity);
    assert_eq!(*memory.0.calls.borrow(), *reference.calls.borrow());
    assert_eq!(job.census.rows(64), census.rows(64));
    drop(lease);

    let mut frame = Captured::new();
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
    let report = copy_divisions_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    assert!(report.copy_ticks[0] > 0);
    let mut frame = Captured::new();
    frame.table_ns[6] = report.copy_ticks[0];
    let before = frame.table_ns;
    let mut ticks = [0; 6];
    super::decode_divisions_payload(&mut frame, kit::BASE, &mut first, &mut ticks);
    assert_eq!(frame.table_ns, before);
    assert!(ticks[0] > 0);
    assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
}

#[test]
fn divisions_truncation_matrix_matches_direct_with_same_caps() {
    // B3: zero-cap / one-row / exact-fit (3) / ample run typed AND
    // copy-decode against the SAME memory with the SAME pooled caps,
    // asserting identical outcome (incl. len/unread) and identical rows.
    // The 3-division fixture makes cap 1 the overflow-by-one arm for
    // divisions (total 3, truncated with dependents unavailable); cap 0
    // the massive/zero-rows arm.
    use super::divisions_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for cap in [0usize, 1, 3, 64] {
        let caps = [cap; 6];
        let direct = kit::direct(&memory, caps);
        let (split, report, _) = kit::split(&memory, caps);
        assert!(report.raw_ok, "cap={cap}");
        kit::assert_matches_direct(&split, &direct);
    }
    let (exact, _, _) = kit::split(&memory, [3; 6]);
    assert_eq!(exact.divisions_outcome, Outcome::Rows { len: 3 });
    for groups in [0, 1] {
        let mut caps = [3; 6];
        caps[2] = groups;
        let direct = kit::direct(&memory, [3; 6]);
        let (split, report, _) = kit::split(&memory, caps);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.groups_outcome, Outcome::Rows { len: 3 });
    }
}

#[test]
fn cancelled_no_frame_acquisition_marks_naval_sources_unavailable_before_reuse() {
    use crate::naval_result_sources::{self as sources, Entry, tests::Observation};
    let _observation = Observation::new();
    sources::tests::setup(1, 0);
    sources::boundary(Some(clock(40, 7).with_campaign_id(0)));
    sources::boundary(Some(clock(41, 7).with_campaign_id(0)));
    let shared = Shared::new(0, 0, 1);
    shared.roster_ready.store(true, Ordering::Release);
    let held: Vec<_> = (0..BUFFERS)
        .map(|_| shared.take_frame(&mut 0).unwrap())
        .collect();
    shared.cancel_buffer_wait.store(true, Ordering::Release);
    let index = shared.take_frame(&mut 0);
    assert_eq!(index, None);
    unsafe {
        acquire(
            &shared,
            Dispatch {
                world_generation: crate::world_lifetime::generation(),
                campaign_epoch: 1,
                index,
                entry: Some(clock(41, 7).borrowed()),
                entry_ticks: 0,
                buffer_wait_ns: 0,
                dispatched: 0,
            },
        );
    }
    let mut entries = [Entry::default(); 1];
    assert_eq!(
        sources::drain(&mut entries).unwrap_err(),
        "naval result sources lost observation; interval unavailable"
    );
    // Detach cancellation still permits a later callback to obtain a newly
    // recycled frame. That must not make the lost event interval complete.
    shared.recycle_frame(held[0]);
    assert_eq!(shared.take_frame(&mut 0), Some(held[0]));
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
    for index in held {
        shared.recycle_frame(index);
    }
}

#[test]
fn contained_acquisition_failure_marks_the_naval_event_interval_unavailable() {
    use crate::naval_result_sources::{self as sources, Entry, tests::Observation};
    let _observation = Observation::new();
    sources::tests::setup(1, 0);
    sources::boundary(Some(clock(40, 7).with_campaign_id(0)));
    sources::boundary(Some(clock(41, 7).with_campaign_id(0)));
    let shared = Shared::new(0, 0, 1);
    let mut forwarded = false;
    assert!(!unsafe {
        synthetic_callback(
            &shared,
            Some(clock(41, 7).borrowed()),
            || panic!("synthetic acquisition failure"),
            || forwarded = true,
        )
    });
    assert!(forwarded);
    assert_eq!(shared.acquisition_panics.load(Ordering::Acquire), 1);
    assert_eq!(
        sources::drain(&mut [Entry::default(); 1]).unwrap_err(),
        "naval result sources lost observation; interval unavailable"
    );
}

struct CampaignClockMemory {
    bytes: Vec<u8>,
}
impl engine_native::Memory for CampaignClockMemory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == cgame_state::SINGLETON_RVA && output.len() == 8 {
            output.copy_from_slice(&0x1000_u64.to_le_bytes());
            true
        } else if address == cgame_state::PLAY_SESSION_STARTED_AT_RVA && output.len() == 8 {
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
    Shared::with_sizing(0, 0, 1, &crate::country_scale::Sizing::for_slots(0)).unwrap()
}

#[test]
fn campaign_clock_callback_keeps_long_identity_and_loss_bytes_under_allocator_denial() {
    let mut memory = campaign_clock_memory(5_001);
    let shared = campaign_shared();
    shared.prepare_clock_capture(&memory).unwrap();
    let index = shared.take_frame(&mut 0).unwrap();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let before = frame.allocated_bytes();
    let mut capture = shared.wait_clock_preparation(&memory);
    let cadence = unsafe { &mut *shared.cadence.get() };
    let pending = unsafe { &mut *shared.pending.get() };
    crate::test_support::with_allocations_denied(|| {
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
    // A change beyond the former chosen text boundary must change observer identity.
    memory.bytes[0x2000 + 5_000] = b'b';
    memory.bytes[cgame_state::CURRENT_HOUR..cgame_state::CURRENT_HOUR + 4]
        .copy_from_slice(&43_i32.to_le_bytes());
    crate::test_support::with_allocations_denied(|| {
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
    drop(capture);
    shared.recycle_frame(index);
}

#[test]
fn campaign_clock_growth_waits_for_spool_preparation_without_callback_allocation() {
    let short = campaign_clock_memory(1);
    let long = campaign_clock_memory(5_001);
    let shared = campaign_shared();
    shared.prepare_clock_capture(&short).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while shared.clock_required() < 5_001 {
                std::thread::yield_now();
            }
            loop {
                shared.prepare_clock_capture(&long).unwrap();
                if shared
                    .clock_capture
                    .try_lock()
                    .is_ok_and(|capture| capture.prepared >= 5_001)
                {
                    break;
                }
                std::thread::yield_now();
            }
        });
        crate::test_support::with_allocations_denied(|| {
            let mut capture = shared.wait_clock_preparation(&long);
            let (entry, _) = capture.read(&long, 0).unwrap();
            let index = shared.take_frame(&mut 0).unwrap();
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.set_clock(Some(entry));
            assert_eq!(
                frame.clock.as_ref().unwrap().campaign_id.as_bytes().len(),
                5_001
            );
            shared.recycle_frame(index);
        });
    });
}

#[test]
fn campaign_clock_gate_does_not_block_frame_return_or_shutdown() {
    let short = campaign_clock_memory(1);
    let long = campaign_clock_memory(5_001);
    let shared = campaign_shared();
    shared.prepare_clock_capture(&short).unwrap();
    let capture = shared.wait_clock_preparation(&short);
    // Spool remains able to process/return frames while a callback holds the clock.
    shared.prepare_clock_capture(&long).unwrap();
    assert_eq!(capture.prepared, 1);
    drop(capture);
    shared.cancel_buffer_wait.store(true, Ordering::Release);
    let mut capture = shared.wait_clock_preparation(&long);
    assert!(capture.read(&long, 0).is_err());
}

#[test]
fn campaign_clock_in_service_frames_prepare_before_republication() {
    let short = campaign_clock_memory(1);
    let long = campaign_clock_memory(5_001);
    let shared = campaign_shared();
    shared.prepare_clock_capture(&short).unwrap();
    let held: Vec<_> = (0..BUFFERS)
        .map(|_| shared.take_frame(&mut 0).unwrap())
        .collect();
    shared.clock_required.store(5_001, Ordering::Release);
    shared.prepare_clock_capture(&long).unwrap();
    for index in held {
        shared.recycle_frame_prepared(index);
    }
    crate::test_support::with_allocations_denied(|| {
        let mut capture = shared.wait_clock_preparation(&long);
        let (entry, _) = capture.read(&long, 0).unwrap();
        for _ in 0..BUFFERS {
            let index = shared.take_frame(&mut 0).unwrap();
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.set_clock(Some(entry));
            assert_eq!(
                frame.clock.as_ref().unwrap().campaign_id.as_bytes().len(),
                5_001
            );
            shared.recycle_frame(index);
        }
    });
}

#[test]
fn campaign_clock_optional_preparation_denial_is_named_and_nonterminal() {
    let memory = campaign_clock_memory(5_001);
    let shared = campaign_shared();
    crate::test_support::with_allocations_denied(|| {
        shared.prepare_clock_capture(&memory).unwrap();
    });
    assert!(!crate::allocation::failed());
    assert_eq!(
        shared.clock_status(),
        (0, 1, Some("clock preparation advisory allocation failed"))
    );
    shared.prepare_clock_capture(&memory).unwrap();
    assert_eq!(shared.clock_capture.lock().unwrap().prepared, 5_001);
}

/// The observer boundary job runs under all of the game thread's hour
/// preparation: it is submitted before the country snapshot and joined after
/// the frame's hour preparation and the last game-thread mark, just before the
/// first burst. No other job is dispatched or joined while it is out, and none
/// is queued before its join, because a held job can still run.
#[test]
fn the_observer_boundary_joins_after_the_hours_preparation_before_any_dispatch() {
    let source = include_str!("../sampling.rs");
    let body = &source[source.find("unsafe fn acquire(").unwrap()..];
    let at = |needle: &str| {
        body.find(needle)
            .unwrap_or_else(|| panic!("hourly body lacks {needle}"))
    };
    let order = [
        "submit_observer_boundary(shared",
        "Cache::prepare_shared(",
        "frame.extended.prepare_hour(",
        "set_modifier_definitions_moved(",
        "set_new_division_mark(",
        "decode::relations_refresh_hour(",
        "finish_observer_boundary(shared",
        "batch.hold();",
        "queue!(acquisition::Job::",
    ];
    for pair in order.windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{} after {}", pair[0], pair[1]);
    }
    // The join helpers are defined after the join; none is used before it.
    let out = &body[at(order[0])..at("finish_observer_boundary(shared")];
    for join in [
        "collect_acquisition(",
        "completed!(",
        "collected!(",
        "join_characters!(",
    ] {
        assert!(
            !out.contains(join),
            "{join} while the observer boundary is out"
        );
    }
    let before_burst = &body[at(order[0])..at("batch.hold();")];
    for dispatch in ["queue!(", "submit_acquisition("] {
        assert!(
            !before_burst.contains(dispatch),
            "{dispatch} before the first burst"
        );
    }
}

/// Readiness gates replace global statement order. Audit each handler's own
/// publish/install/dispatch order, the complete split barriers, and the packs
/// that stay behind map sampling and the stitch's delayed failure barrier.
#[test]
fn the_hourly_job_graph_has_only_its_named_data_dependencies() {
    let source = include_str!("../sampling.rs");
    let body = &source[source.find("unsafe fn acquire(").unwrap()..];
    let at = |needle: &str| {
        body.find(needle)
            .unwrap_or_else(|| panic!("hourly body lacks {needle}"))
    };
    let queue = |job: &str| format!("queue!(acquisition::Job::{job}(");
    let burst = at("batch.hold();");
    let release = at("release!();");
    let scheduler = at("while late_pending.is_none() || final_inputs_pending.is_none()");
    assert!(burst < release && release < scheduler);
    for join in [
        "completed!(",
        "collected!(",
        "collect_acquisition(",
        "collect_orders(",
    ] {
        // Collection macros are defined before the burst, never invoked in it.
        assert!(
            !body[burst..release].contains(join),
            "join inside first burst: {join}"
        );
    }
    for job in [
        "ShipModels",
        "Countries",
        "Templates",
        "DivisionInputs",
        "DivisionInputParts",
        "OrderRoots",
        "AirWing",
        "Characters",
        "World",
        "Economy",
        "Research",
        "Governance",
        "Variants",
        "OrdinaryPart",
        "Slow",
        "WarOverview",
        "ProductionModifierPart",
        "ProductionDaily",
        "Stability",
        "IntelResidual",
        "AirBases",
        "Factions",
        "Players",
        "AirStats",
        "Supremacy",
        "Sunk",
        "Radar",
        "Projects",
        "Lexical",
        "Session",
        "RaidState",
        "RaidTargets",
        "RaidLaunch",
        "AirRelations",
        "SupplyTrains",
        "MapCombats",
        "ScoresRails",
    ] {
        assert!(
            burst < at(&queue(job)) && at(&queue(job)) < release,
            "{job} first burst"
        );
    }
    let handlers = &body[at("match step {")..at("            segment!(1);")];
    let handler = |step: &str| {
        let marker = format!("EarlyStep::{step} => {{");
        let start = handlers.find(&marker).unwrap() + marker.len();
        let end = handlers[start..]
            .find("EarlyStep::")
            .map_or(handlers.len(), |offset| start + offset);
        &handlers[start..end]
    };
    let ordered = |part: &str, markers: &[&str]| {
        let compact = part.split_whitespace().collect::<String>();
        let mut position = 0;
        for marker in markers {
            let marker = marker.split_whitespace().collect::<String>();
            position += compact[position..]
                .find(&marker)
                .unwrap_or_else(|| panic!("missing/out-of-order {marker}"))
                + marker.len();
        }
    };
    let readiness = &body[at("let step = batch.wait_ready(")..at("match step {")];
    for gate in [
        "presentation.ready(batch)",
        "all(&input_pending, &part_pending)",
        "all(&air_pending, &air_part_pending)",
        "orders_complete(batch, orders, parts)",
        "ready!(divisions_pending)",
        "ready!(army_icons_pending)",
        "stitch_can_advance(batch, pending, false)",
        "stitch_failure.ready(",
        "stitched_orders.is_some()",
        "wings_done",
        "ready!(naval_pending)",
        "divisions_done",
        "ready!(combat_pending)",
        "ready!(child_fronts_pending)",
        "ready!(air_missions_pending)",
        "pending_complete(batch, &scores_rails_pending)",
        "map_done && child_done && icons_done && missions_done",
        "final_inputs_ready(",
        "late_pending.is_some()",
        "intel_residual_pending.as_ref()",
        "capitulation_pending.as_ref()",
        "session_pending.as_ref()",
        "relations_pending.as_ref()",
        "variants_pending.as_ref().map(|(pending, _)| pending)",
        "characters_pending[1..].iter().map(Option::as_ref)",
    ] {
        assert!(
            readiness
                .split_whitespace()
                .collect::<String>()
                .contains(&gate.split_whitespace().collect::<String>()),
            "readiness omits {gate}"
        );
    }
    ordered(
        &body[scheduler..at("let step = batch.wait_ready(")],
        &[
            "if verdicts_done && divisions_done",
            "stitch_failure.reach(&batch)",
        ],
    );
    assert!(!handler("Naval").contains("!batch.stopped()"));
    ordered(
        handler("Presentation"),
        &["presentation.advance(shared, &mut batch)"],
    );
    let collect_macro = &body[at("macro_rules! collected {")..at("macro_rules! completed {")];
    assert!(collect_macro.contains("presentation.wait_for(shared, &mut batch, Some(&pending))"));
    ordered(
        handler("Roots"),
        &[
            "completed!(root_pending.take()",
            "frame.map.orders =",
            "charge_order_parts(",
            "take_orders_request(",
            &queue("OrdersPart"),
            &queue("Orders"),
            &queue("Naval"),
            "batch.hold();",
            &queue("Provinces"),
            &queue("States"),
            &queue("Weather"),
            &queue("Capitulation"),
            &queue("Wars"),
            &queue("Relations"),
            "release!();",
        ],
    );
    ordered(
        handler("Inputs"),
        &[
            "completed!(input_pending.take()",
            "part_pending).into_iter()",
            "completed!(pending, DivisionInputParts)",
            "census.absorb(",
            "(*shared.division_parts.get())[part] =",
            "install_unit_inputs(",
            "batch.hold();",
            &queue("Divisions"),
            &queue("PresentationPrepare"),
            &queue("Presentation"),
            &queue("PresentationMovement"),
            &queue("Intel"),
            "radar_raw_mode = true",
            "registrations_raw_mode = true",
            "intel_raw_mode = true",
            "rest_raw_mode = construction_active",
            &queue("Gated"),
            &queue("ProvinceUnitOrder"),
            "release!();",
        ],
    );
    ordered(
        handler("Ships"),
        &[
            "ship_models_pending.take()",
            "restore_ship_models_request(",
            "batch.hold();",
            "take_ship_model_parts(",
            &queue("ShipModelDetails"),
            "release!();",
        ],
    );
    ordered(
        handler("Countries"),
        &[
            "completed!(countries_pending.take()",
            "restore_countries_request(",
            "take_ordinary_request(",
            &queue("Ordinary"),
        ],
    );
    ordered(
        handler("Characters"),
        &[
            "join_characters!(0)",
            &queue("CharacterPortraits"),
            "role_census()",
            &queue("CharacterInputs"),
            &queue("Characters"),
            "drop(role_census)",
        ],
    );
    ordered(
        handler("Wings"),
        &[
            "completed!(air_pending.take()",
            "air_part_pending).into_iter()",
            "completed!(pending, AirWing)",
            "join_parts(",
            "retained_store_bytes(",
            "install_owned_mission_identities(",
            &queue("AirMissions"),
        ],
    );
    ordered(
        handler("Orders"),
        &["collect_orders(", &queue("OrdersStitch")],
    );
    ordered(
        handler("Divisions"),
        &[
            "completed!(pending, Divisions)",
            "supply_sources(",
            "*divisions_payload =",
            "set_divisions_raw_mode(true)",
            &queue("ArmyIcons"),
        ],
    );
    ordered(
        handler("ArmyIcons"),
        &[
            "completed!(pending, ArmyIcons)",
            "restore_army_icons_request(",
        ],
    );
    ordered(
        handler("Stitch"),
        &[
            "stitch_pending.take()",
            "debug_assert!(!report.contained_panic && failure.is_none())",
            "end_order_parts(",
            &queue("Fronts"),
            "plans.take_fronts()",
            "Job::ChildFronts(request)",
            "batch.park(",
        ],
    );
    ordered(
        handler("Naval"),
        &[
            "completed!(naval_pending.take()",
            "install_wings_after(",
            "frame.map.install_owned_air(",
            "naval_pilot().raw_mode",
            ".unpark(",
            "stitch_pending.take()",
            "settle_stitch(",
            r#"stitch_failure.admissible.expect("pre-Naval admission")"#,
            "end_order_parts(",
            &queue("Fronts"),
            "orders_request.group_source =",
            &queue("ChildFronts"),
        ],
    );
    ordered(
        handler("ChildFronts"),
        &[
            "child_fronts_pending.take()",
            "*naval_payload = orders_request.group_source.take()",
            "fail_child_fronts()",
            "restore_orders_request(",
        ],
    );
    ordered(
        handler("Map"),
        &[
            "completed!(combat_pending.take()",
            "restore_combat_request(",
            "frame.map.sample(",
            &queue("AirMissions"),
        ],
    );
    ordered(
        handler("Missions"),
        &[
            "completed!(scores_rails_pending, ScoresRails)",
            "air_missions_pending.take()",
            "restore_air_missions_request(",
            "install_air_verdicts(",
            "restore_scores_rails_request(",
        ],
    );
    ordered(handler("Late"), &[&queue("Late"), &queue("AirHistories")]);
    // The map may need the typed wing fallback: its air pack and Ordinary
    // score sources cannot be installed/loaned before the sample.
    assert!(
        readiness
            .split_whitespace()
            .collect::<String>()
            .contains("ifmap_done&&!missions_done")
    );
    assert!(handler("Late").contains("take_late_request("));
    ordered(
        handler("FinalInputs"),
        &[
            "intel_residual_pending.take()",
            "restore_intel_residual_request(",
            "capitulation_pending.take()",
            "capitulation_raw_mode =",
            "session_pending.take()",
            "session_pilot().raw_mode =",
            "relations_pending.take()",
            "join_relations(",
            "variants_pending.take()",
            "restore_variants_request(",
            "join_characters!(part)",
            "settle_intel_residual(",
            "take_final_inputs_request(",
        ],
    );
    for unrelated in [
        "weather_pending",
        "lexical_pending",
        "provinces_pending",
        "completed!(late_pending, Late)",
        "movement_pending",
        "character_inputs_pending",
    ] {
        assert!(
            !handler("FinalInputs").contains(unrelated),
            "{unrelated} gates final inputs"
        );
    }
    // Assembly-only joins retain their order after FinalInputs admission.
    let tail = &body[at("            segment!(1);")..];
    // Governance contributes this hour's owner recovery markers. Seal them
    // on a worker after that census joins, then restore before assembly.
    ordered(
        tail,
        &[
            "collected!(governance_pending, Governance)",
            &queue("EventFeedSeal"),
            "let acquisition::Job::EventFeedSeal(feed) = job",
            ".restore_event_feed_seal(feed",
            ".capture_tables(&cached",
        ],
    );
    ordered(
        tail,
        &[
            "completed!(weather_pending, Weather)",
            "completed!(pending, Lexical)",
            "completed!(provinces_pending, Provinces)",
            "completed!(templates_pending, Templates)",
            "completed!(slow_pending, Slow)",
            "completed!(war_overview_pending, WarOverview)",
            "completed!(stability_pending, Stability)",
        ],
    );
    ordered(
        tail,
        &[
            "completed!(pending, OrdinaryPart)",
            "completed!(ordinary_pending, Ordinary)",
            "let acquisition::Job::EventFeedSeal(feed) = job",
            ".restore_event_feed_seal(feed",
            "completed!(pending, ProductionDaily)",
            "completed!(pending, ProductionModifierPart)",
            ".capture_tables(&cached",
            "completed!(late_pending, Late)",
            "completed!(fronts_pending, Fronts)",
            "presentation.finish(shared, &mut batch)",
            "collected!(movement_pending, PresentationMovement)",
            "install_order_transport()",
            "completed!(final_pending, FinalInputs)",
            "capture_tables_late(",
            "completed!(intel_pending, Intel)",
            "completed!(air_bases_pending, AirBases)",
        ],
    );
    assert_eq!(body.matches("registrations_raw_mode = true").count(), 1);
    assert_eq!(body.matches("air_bases_report = report;").count(), 1);
    // Variants and RaidState claim before the dependency tier; the
    // remaining independent jobs follow it in descending length.
    use acquisition::Role;
    for gate in [
        Role::ObserverBoundary,
        Role::Countries,
        Role::Orders,
        Role::OrderRoots,
        Role::OrdersPart,
        Role::OrdersStitch,
        Role::ChildFronts,
        Role::AirWing,
        Role::Naval,
        Role::AirMissions,
        Role::Late,
        Role::Characters,
        Role::Divisions,
        Role::DivisionInputs,
        Role::ShipModels,
        // Raw copies joined beside the wing join.
        Role::AirBases,
        Role::Radar,
        Role::Projects,
        Role::Intel,
        // Joined before the map sample, which the Late job follows.
        Role::Players,
        Role::MapCombats,
        Role::AirStats,
        Role::Supremacy,
        Role::Sunk,
        // Joined side by side after the raw copies; the final borrowers
        // read the economy rest.
        Role::World,
        Role::Economy,
        // The longest job that waits for another: dispatched after the
        // first burst, it would queue behind every job of its class.
        Role::Presentation,
        Role::FinalInputs,
        // The longest first-burst job and the tables split from it.
        Role::Slow,
        Role::Stability,
        Role::WarOverview,
        Role::ProductionDaily,
        Role::ProductionModifierParts,
    ] {
        assert_eq!(gate.priority(), Role::Countries.priority(), "{gate:?}");
    }
    for (long, short) in [
        (Role::IntelResidual, Role::Capitulation),
        (Role::Governance, Role::Lexical),
        (Role::OrdinaryNaval, Role::OrdinaryMap),
        (Role::OrdinaryNavies, Role::OrdinaryMap),
        (Role::PresentationMovement, Role::ProvinceUnitOrder),
        (Role::CharacterInputs, Role::CharacterTraits),
        (Role::AdvisorRoles, Role::CountryLeaderRoles),
        (Role::Ordinary, Role::AirRelations),
        (Role::Templates, Role::Factions),
        (Role::Provinces, Role::States),
        (Role::Capitulation, Role::Lexical),
    ] {
        assert!(long.priority() < short.priority(), "{long:?} {short:?}");
        assert!(Role::Countries.priority() < long.priority());
    }
    // AdvisorRoles may gate FinalInputs, but does not outrank Presentation.
    assert!(Role::Presentation.priority() < Role::AdvisorRoles.priority());
    // Split from Slow, the stability tables claim no later than it.
    assert!(Role::Stability.priority() <= Role::Slow.priority());
    assert!(Role::WarOverview.priority() <= Role::Slow.priority());
    // Split from World, the economy rest claims with it.
    assert_eq!(Role::Economy.priority(), Role::World.priority());
}

#[test]
fn a_parked_spool_wakes_on_a_ready_buffer_without_waiting_for_its_deadline() {
    use std::{sync::mpsc, time::Duration, time::Instant};
    let shared = Arc::new(Shared::new(0, 0, 1));
    // Before the spool registers, a wake is a no-op rather than a fault.
    shared.wake_spool();
    let (registered, ready) = mpsc::channel();
    let spool = {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            shared.spool_waker.set(std::thread::current()).unwrap();
            registered.send(()).unwrap();
            let started = Instant::now();
            while shared.ready.is_empty() {
                // Far longer than the test allows: only the wake ends it.
                std::thread::park_timeout(Duration::from_secs(60));
                assert!(started.elapsed() < Duration::from_secs(30));
            }
            started.elapsed()
        })
    };
    ready.recv().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    shared.ready.push(1).unwrap();
    shared.wake_spool();
    let waited = spool.join().unwrap();
    assert!(waited < Duration::from_secs(10), "{waited:?}");
    // A wake that precedes the wait is kept, so publication is never lost.
    let early = {
        let shared = Arc::new(Shared::new(0, 0, 1));
        shared.spool_waker.set(std::thread::current()).unwrap();
        shared.wake_spool();
        let started = Instant::now();
        std::thread::park_timeout(Duration::from_secs(60));
        started.elapsed()
    };
    assert!(early < Duration::from_secs(10), "{early:?}");
}

/// The spool thread returns every buffer through `recycle_frame_prepared`
/// (the capture's clock preparation and allocated-byte census). Run it on a
/// thread whose stack is a small fraction of the spool thread's
/// `BACKGROUND_STACK_BYTES`: a large by-value local on that path overflows
/// here, ending this test binary, before it can reach the game process.
#[test]
fn buffer_return_runs_on_a_small_stack() {
    const SMALL_STACK: usize = 256 * 1024;
    const _: () = assert!(SMALL_STACK * 32 <= crate::runtime::BACKGROUND_STACK_BYTES);
    let shared = std::sync::Arc::new(campaign_shared());
    let worker = std::sync::Arc::clone(&shared);
    let returned = std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(move || {
            let held: Vec<_> = (0..BUFFERS)
                .map(|_| worker.take_frame(&mut 0).unwrap())
                .collect();
            for index in held {
                worker.recycle_frame_prepared(index);
            }
            worker.free.len()
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(returned, BUFFERS);
}

#[test]
fn buffers_backlogged_with_two_free_buffers() {
    let shared = campaign_shared();
    let first = shared.take_frame(&mut 0).unwrap();
    assert_eq!(shared.free.len(), 3);
    assert!(!shared.buffers_backlogged());
    let second = shared.take_frame(&mut 0).unwrap();
    assert_eq!(shared.free.len(), 2);
    assert!(shared.buffers_backlogged());
    shared.recycle_frame_prepared(first);
    assert!(!shared.buffers_backlogged());
    shared.recycle_frame_prepared(second);
    assert_eq!(shared.free.len(), BUFFERS);
}

/// The Radar and Projects joins turn an incomplete raw acquisition into a
/// failure, as the AirBases join does: their verdicts (`raw_ok || failure`)
/// were true for every join, so setting them when the jobs are queued, before
/// the Gated dispatch, reads nothing new.
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

/// The AirBases join turns an incomplete raw acquisition into a failure, so
/// every join holds a raw payload or a failure: the verdicts it set
/// (`raw_ok || failure`) were true for every join, and the Gated packs' and
/// the air pack's typed fallbacks could not run. Setting them true before
/// the Gated dispatch therefore reads nothing new; a join that could end
/// with neither would fail here.
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
