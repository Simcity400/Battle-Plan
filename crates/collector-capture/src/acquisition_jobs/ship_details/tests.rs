use super::*;
use engine_native::{Memory, walk::Outcome};
use std::sync::mpsc;

struct Unreadable;
impl Memory for Unreadable {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        false
    }
}
fn request(ships: usize) -> ShipModelsRequest {
    let mut captured = ship_models::Captured::with_capacities(&Default::default());
    captured.begin_sources();
    for index in 0..ships {
        captured.visit_source(
            0x1000 * (index + 1),
            &engine_native::cship::Row {
                id: index as u64 + 1,
                roster_ordinal: index as u32 * 3,
                ..Default::default()
            },
        );
    }
    captured.finish_sources(Outcome::Rows { len: ships });
    ShipModelsRequest {
        countries: engine_allocation::shared(
            engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
        )
        .unwrap(),
        context: ship_models::CaptureContext::new(11),
        game_state: 0x1000,
        captured,
        census: collector_map_capture::map_capture::ShipCensus::vacant(),
        details: None,
    }
}
fn run_detail(request: &mut ShipModelDetailsRequest, fail: bool) -> ship_models::CaptureReport {
    if fail && request.part > 0 {
        request.captured.fail_owned_capture(if request.part == 1 {
            "first detail failure"
        } else {
            "later detail failure"
        })
    } else {
        request
            .captured
            .capture_deferred(&Unreadable, request.context)
    }
}
fn join(mut requests: Vec<ShipModelDetailsRequest>) -> ship_models::Captured {
    requests.sort_by_key(|request| request.part);
    let mut requests = requests.into_iter();
    let mut captured = requests.next().unwrap().captured;
    for request in requests {
        captured.restore_part(request.part - 1, request.captured);
    }
    captured.finish_split();
    captured
}
fn encoded(captured: &ship_models::Captured) -> Vec<u8> {
    use replay_frame::frame::{FrameEncoder, FrameInput, Metrics, TableInput, TableView};
    let mut frame = FrameInput {
        branch: 7,
        hour: 24,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableInput::Sampled(TableView::Unavailable {
                reason: "fixture".into(),
            })
        }),
    };
    frame.tables[ship_models::TABLES[0].index()] =
        captured.table_inputs(None).into_iter().next().unwrap();
    let mut bytes = Vec::new();
    FrameEncoder::new()
        .encode_input(frame, true)
        .unwrap()
        .encode(&mut bytes)
        .unwrap();
    bytes
}
#[test]
fn census_publication_releases_details_before_collection_with_identical_bytes_and_failure_order() {
    for (ships, fail) in [
        (0, false),
        (3, false),
        (1200, false),
        (2049, false),
        (1200, true),
    ] {
        let mut reference = request(ships);
        reference.captured.set_context(reference.context);
        let reference = take_parts(
            &mut reference.captured,
            &reference.countries,
            reference.context.base,
        )
        .into_iter()
        .flatten()
        .map(|mut part| {
            run_detail(&mut part, fail);
            part
        })
        .collect();
        let reference = join(reference);
        let (started, running) = mpsc::channel();
        let pool = Pool::with_test_runner(3, move |job| match job {
            Job::ShipModels(_) => Report::ShipModels,
            Job::ShipModelDetails(request) => {
                let report = run_detail(request, fail);
                started.send(request.part).unwrap();
                Report::ShipModelDetails(report)
            }
            _ => unreachable!(),
        });
        let mut batch = Batch::new(&pool);
        let mut request = request(ships);
        batch.arm_ship_details(&mut request);
        let ticket = unsafe { batch.submit(&mut Some(Job::ShipModels(request))) }.unwrap();
        let count = ship_models::parts_for(ships);
        for _ in 0..count {
            running
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
        }
        assert!(batch.is_complete(ticket));
        let mut completion = None;
        assert!(batch.finish_into(ticket, &mut completion));
        let Job::ShipModels(mut request) = completion.unwrap().request else {
            unreachable!()
        };
        let details = request.details.take().unwrap();
        let pending = details
            .take(&mut request.captured)
            .unwrap()
            .expect("publisher dispatched before callback");
        assert_eq!(pending.iter().flatten().count(), count);
        let mut restored = Vec::new();
        for pending in pending.into_iter().rev().flatten() {
            assert!(!pending.parked);
            let (ticket, _) = batch.adopt_presentation_ticket(pending);
            let mut completion = None;
            assert!(batch.finish_into(ticket, &mut completion));
            let completion = completion.unwrap();
            assert!(completion.report.is_ok());
            let Job::ShipModelDetails(request) = completion.request else {
                unreachable!()
            };
            restored.push(request);
        }
        let actual = join(restored);
        assert_eq!(
            encoded(&actual),
            encoded(&reference),
            "ships={ships}, fail={fail}"
        );
        if fail {
            assert_eq!(
                actual.outcomes(),
                [Outcome::Unavailable {
                    reason: "first detail failure"
                }]
            );
        }
    }
}
#[test]
fn callback_winning_the_publication_race_retains_the_original_detail_fallback() {
    let pool = Pool::with_test_runner(1, |_| unreachable!());
    let batch = Batch::new(&pool);
    let mut request = request(1200);
    batch.arm_ship_details(&mut request);
    let dispatch = request.details.take().unwrap();
    dispatch.store(&mut request.captured, &Ok(Report::ShipModels));
    assert_eq!(request.captured.allocated_bytes(), 0);
    assert!(dispatch.take(&mut request.captured).unwrap().is_none());
    collector_workers::acquisition::Publication::published(&*dispatch);
    assert!(dispatch.lock().pending.iter().all(Option::is_none));
    assert_eq!(request.captured.census_len(), 1200);
    assert_eq!(
        take_parts(
            &mut request.captured,
            &request.countries,
            request.context.base
        )
        .iter()
        .flatten()
        .count(),
        3
    );
}
#[test]
fn failed_census_never_releases_details_and_retains_its_original_capture() {
    for failure in [Failure::Panicked, Failure::Stopped] {
        let pool = Pool::with_test_runner(1, |_| unreachable!());
        let batch = Batch::new(&pool);
        let mut request = request(1200);
        let bytes = request.captured.allocated_bytes();
        batch.arm_ship_details(&mut request);
        let dispatch = request.details.take().unwrap();
        dispatch.store(&mut request.captured, &Err(failure));
        collector_workers::acquisition::Publication::published(&*dispatch);
        assert!(dispatch.take(&mut request.captured).unwrap().is_none());
        assert_eq!(request.captured.allocated_bytes(), bytes);
        assert_eq!(request.captured.census_len(), 1200);
    }
}
#[test]
fn rejected_detail_continuations_are_retained_and_unadopted_children_drain() {
    let pool = Pool::with_test_runner(1, |_| unreachable!());
    {
        let batch = Batch::new(&pool);
        let mut request = request(1200);
        batch.arm_ship_details(&mut request);
        let dispatch = request.details.take().unwrap();
        dispatch.store(&mut request.captured, &Ok(Report::ShipModels));
        // SAFETY: The admitted capture owns failure settlement for this acquisition pool.
        unsafe { pool.executor.stop_for_contained_panic() };
        collector_workers::acquisition::Publication::published(&*dispatch);
        let state = dispatch.lock();
        assert!(state.dispatched);
        assert_eq!(state.pending.iter().flatten().count(), 3);
        assert!(state.pending.iter().flatten().all(|pending| pending.parked));
    }
    let calibration = pool.executor.calibration();
    assert_eq!(
        (
            calibration.uncollected,
            calibration.active,
            calibration.queued
        ),
        (0, 0, 0)
    );
    assert!(pool.ship_details.lock().closed);
}
#[test]
fn unwinding_callback_drains_publisher_released_children_without_adopting_them() {
    let (started, running) = mpsc::channel();
    let pool = Pool::with_test_runner(2, move |job| match job {
        Job::ShipModels(_) => Report::ShipModels,
        Job::ShipModelDetails(request) => {
            started.send(()).unwrap();
            Report::ShipModelDetails(run_detail(request, false))
        }
        _ => unreachable!(),
    });
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        let mut batch = Batch::new(&pool);
        let mut request = request(1200);
        batch.arm_ship_details(&mut request);
        unsafe { batch.submit(&mut Some(Job::ShipModels(request))) }.unwrap();
        running
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        panic!("synthetic callback unwind");
    }));
    assert!(unwound.is_err());
    let calibration = pool.executor.calibration();
    assert_eq!(
        (
            calibration.uncollected,
            calibration.active,
            calibration.queued
        ),
        (0, 0, 0)
    );
    assert!(pool.ship_details.lock().pending.iter().all(Option::is_none));
}
