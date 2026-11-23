use super::*;
use collector_raw::raw::{RawLog, Recorded};
use engine_native::{Memory, carmy_group, corders_group, memory::SliceMemory};
use std::sync::atomic::AtomicUsize;

const GROUP: usize = 0x30000;
const FLAG: usize = GROUP + corders_group::IS_ARMY_GROUP;
const MOTOR: usize = GROUP + corders_group::MOTORIZATION_LEVEL;
const HEADER: usize = GROUP + carmy_group::MEMBER_DATA;
const MASK: u32 = 0x1C0;

#[allow(clippy::field_reassign_with_default)]
fn request(count: i32) -> OrdersRequest {
    let mut payload = naval_history::NavalJob::default();
    payload.log = RawLog::new(16, 128);
    let mut header = [0; 16];
    header[12..16].copy_from_slice(&count.to_le_bytes());
    for (at, bytes) in [
        (FLAG, &[0][..]),
        (FLAG, &[1][..]),
        (MOTOR, &[0][..]),
        (HEADER, &header[..]),
        (FLAG, &[0][..]),
    ] {
        let source = SliceMemory { base: at, bytes };
        assert!(payload.log.take(&source, at, bytes.len(), MASK).is_some());
    }
    OrdersRequest {
        countries: engine_allocation::shared(
            engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
        )
        .unwrap(),
        base: 0,
        game_state: 0,
        orders: collector_divisions::orders::Orders::vacant(),
        plans: collector_plans::extended::plans::Captured::vacant(),
        group_source: Some(payload),
    }
}

fn inspect_return(mut request: OrdersRequest, bytes: usize, spans: usize, count: i32) {
    let payload = request.group_source.take().unwrap();
    assert_eq!(payload.log.bytes.as_ptr() as usize, bytes);
    assert_eq!(payload.log.spans.as_ptr() as usize, spans);
    assert_eq!(payload.log.spans.len(), 5);
    let ordinary = Recorded::build(&payload.log).unwrap();
    let mut flag = [9];
    assert!(ordinary.copy(FLAG, &mut flag));
    assert_eq!(flag, [0]);
    assert!(ordinary.copy(FLAG, &mut flag));
    assert_eq!(flag, [1]);
    let mut header = [0; 16];
    assert!(ordinary.copy(HEADER, &mut header));
    assert_eq!(&header[12..16], &count.to_le_bytes());
    drop(ordinary);
    assert!(request.group_source.is_none());
}

fn loan(job: &Job) -> &naval_history::NavalJob {
    let Job::ChildFronts(request) = job else {
        panic!("fixture must retain its submitted ChildFronts variant");
    };
    request.group_source.as_ref().unwrap()
}

fn inspect_job(job: Job, bytes: usize, spans: usize, count: i32) {
    let Job::ChildFronts(request) = job else {
        panic!("executor must return the submitted ChildFronts variant");
    };
    inspect_return(request, bytes, spans, count);
}

#[test]
fn executor_success_plan_failure_and_caught_panic_return_the_original_naval_loan() {
    for mode in 0..3 {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            collector_workers::acquisition::Pool::start(0, 1, move |job: &mut Job| {
                let Job::ChildFronts(request) = job else {
                    panic!("unexpected fixture runner");
                };
                observed.fetch_add(1, Ordering::Relaxed);
                let payload = request.group_source.as_ref().unwrap();
                {
                    let ordinary = Recorded::build(&payload.log).unwrap();
                    assert!(ordinary.copy(FLAG, &mut [0]));
                    assert!(ordinary.copy(FLAG, &mut [0]));
                }
                let source = naval_history::OrderGroupSource::new(payload).unwrap();
                let mut flag = [9];
                assert!(source.copy(FLAG, &mut flag));
                assert_eq!(flag, [1]);
                if mode == 2 {
                    panic!("synthetic ChildFronts failure with an active OOB reader");
                }
                drop(source);
                if mode == 1 {
                    request.plans.fail_child_fronts();
                }
                Report::ChildFronts(OrdersReport)
            })
        }
        .unwrap();
        let mut pending = Some(Job::ChildFronts(request(7)));
        let payload = loan(pending.as_ref().unwrap());
        let bytes = payload.log.bytes.as_ptr() as usize;
        let spans = payload.log.spans.as_ptr() as usize;
        let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
        let completed = pool.finish(ticket).unwrap();
        if mode == 2 {
            assert!(matches!(completed.report, Err(Failure::Panicked)));
        } else {
            assert!(matches!(completed.report, Ok(Report::ChildFronts(_))));
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        inspect_job(completed.request, bytes, spans, 7);
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { pool.shutdown() }.unwrap();
    }
}

#[test]
fn stopped_child_fronts_request_returns_its_unread_naval_loan() {
    let trace = Arc::new(collector_memory::read_accounting::fixtures::ReadTrace::default());
    let runner_trace = Arc::clone(&trace);
    // SAFETY: Test owns this local acquisition pool and shuts it down before test exit.
    let pool = unsafe {
        collector_workers::acquisition::Pool::start(0, 1, move |_: &mut Job| -> Report {
            runner_trace.record((), || {
                collector_memory::read_accounting::fixtures::read(1);
                panic!("cancelled ChildFronts runner must not execute");
            })
        })
    }
    .unwrap();
    let mut pending = Some(Job::ChildFronts(request(8)));
    let payload = loan(pending.as_ref().unwrap());
    let bytes = payload.log.bytes.as_ptr() as usize;
    let spans = payload.log.spans.as_ptr() as usize;
    let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let completed = pool.finish(ticket).unwrap();
    assert!(matches!(completed.report, Err(Failure::Stopped)));
    assert_eq!(trace.take(()), collector_memory::read_accounting::ZERO);
    #[cfg(feature = "acquisition_profile")]
    assert_eq!(
        completed.table_reads,
        collector_memory::read_accounting::ZERO
    );
    inspect_job(completed.request, bytes, spans, 8);
}

#[test]
fn admission_rejection_keeps_the_loan_for_refusal_and_ordinary_decode() {
    // SAFETY: Test owns this local acquisition pool and shuts it down before test exit.
    let pool = unsafe {
        collector_workers::acquisition::Pool::start(0, 1, |_: &mut Job| -> Report {
            panic!("rejected ChildFronts must never run");
        })
    }
    .unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let mut pending = Some(Job::ChildFronts(request(9)));
    let payload = loan(pending.as_ref().unwrap());
    let bytes = payload.log.bytes.as_ptr() as usize;
    let spans = payload.log.spans.as_ptr() as usize;
    assert_eq!(
        unsafe { pool.submit(&mut pending) },
        Err(SubmitError::Stopped)
    );
    let Job::ChildFronts(mut request) = pending.take().unwrap() else {
        unreachable!();
    };
    request.plans.fail_child_fronts();
    inspect_return(request, bytes, spans, 9);
}

#[test]
fn a_child_fronts_pass_without_group_demand_is_not_worker_only() {
    let job = Job::ChildFronts(request(1));
    assert!(!job.worker_only());
    assert!(job.role().worker_only());
    let payload = loan(&job);
    let (bytes, spans) = (
        payload.log.bytes.as_ptr() as usize,
        payload.log.spans.as_ptr() as usize,
    );
    inspect_job(job, bytes, spans, 1);
}

#[test]
fn readiness_stitched_order_parking_preserves_naval_loans_on_success_panic_and_refusal() {
    for mode in 0..3 {
        let pool = Pool::with_test_runner(1, move |job: &mut Job| match job {
            Job::OrdersStitch(_) => Report::OrdersStitch(StitchReport::default()),
            Job::Fronts(_) => Report::Fronts(FrontsReport),
            Job::ChildFronts(_) => {
                if mode == 1 {
                    panic!("synthetic child-front panic after parked handoff");
                }
                Report::ChildFronts(OrdersReport)
            }
            _ => unreachable!("readiness loan fixture job"),
        });
        let mut batch = Batch::new(&pool);
        let orders = request(3);
        let payload = orders.group_source.as_ref().unwrap();
        let bytes = payload.log.bytes.as_ptr() as usize;
        let spans = payload.log.spans.as_ptr() as usize;
        let mut job = Some(Job::OrdersStitch(orders));
        let stitch = unsafe { batch.submit(&mut job).unwrap() };
        batch.wait_ready(|batch| batch.is_complete(stitch).then_some(()));
        assert!(matches!(
            batch.completed_report(stitch),
            Some(Ok(Report::OrdersStitch(StitchReport {
                contained_panic: false
            })))
        ));
        let completion = batch.finish(stitch).unwrap();
        let Job::OrdersStitch(mut orders) = completion.request else {
            unreachable!()
        };
        let mut front = Some(Job::Fronts(FrontsRequest {
            base: 0,
            fronts: orders.plans.take_fronts(),
        }));
        let fronts = unsafe { batch.submit(&mut front).unwrap() };
        let mut retained = Some(Job::ChildFronts(orders));
        let parked = batch.park(&mut retained).unwrap();
        assert!(retained.is_none());
        assert!(!batch.is_complete(parked));
        batch.wait_ready(|batch| batch.is_complete(fronts).then_some(()));
        drop(batch.finish(fronts).unwrap());
        let mut child = Some(batch.unpark(parked).unwrap());
        assert_eq!(
            loan(child.as_ref().unwrap()).log.bytes.as_ptr() as usize,
            bytes
        );
        if mode == 2 {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { pool.shutdown() }.unwrap();
        }
        let completion = match unsafe { batch.submit(&mut child) } {
            Ok(ticket) => {
                batch.wait_ready(|batch| batch.is_complete(ticket).then_some(()));
                batch.finish(ticket).unwrap()
            }
            Err(SubmitError::Stopped) => {
                let ticket = batch.park(&mut child).unwrap();
                let mut completion = None;
                assert!(
                    batch.finish_parked_into(ticket, &mut completion, |_| Some(Err(
                        Failure::Stopped
                    )))
                );
                completion.unwrap()
            }
            Err(reason) => panic!("unexpected fixture admission {reason:?}"),
        };
        assert!(matches!(
            (mode, completion.report),
            (0, Ok(Report::ChildFronts(_)))
                | (1, Err(Failure::Panicked))
                | (2, Err(Failure::Stopped))
        ));
        inspect_job(completion.request, bytes, spans, 3);
        assert_eq!(pool.calibration().executor.uncollected, 0);
        assert_eq!(pool.calibration().executor.caller_executed, 0);
    }
}
