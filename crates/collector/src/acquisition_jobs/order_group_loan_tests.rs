use super::*;
use crate::raw::{RawLog, Recorded};
use engine::{Memory, carmy_group, corders_group, memory::SliceMemory};
use std::sync::atomic::AtomicUsize;

const GROUP: usize = 0x30000;
const FLAG: usize = GROUP + corders_group::IS_ARMY_GROUP;
const MOTOR: usize = GROUP + corders_group::MOTORIZATION_LEVEL;
const HEADER: usize = GROUP + carmy_group::MEMBER_DATA;
const MASK: u32 = 0x1C0;

// NavalJob's private capture state requires constructing this fixture via Default.
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
        countries: Arc::new(engine::country_memory::Cache::try_with_capacity(0).unwrap()),
        base: 0,
        game_state: 0,
        orders: map_capture::Orders::vacant(),
        plans: crate::extended::plans::Captured::vacant(),
        group_source: Some(payload),
    }
}

fn inspect_return(mut request: OrdersRequest, bytes: usize, spans: usize, count: i32) {
    // The callback performs this same take before restoring the capture packs.
    let payload = request.group_source.take().unwrap();
    assert_eq!(payload.log.bytes.as_ptr() as usize, bytes);
    assert_eq!(payload.log.spans.as_ptr() as usize, spans);
    assert_eq!(payload.log.spans.len(), 5);
    let ordinary = Recorded::build(&payload.log);
    let mut flag = [9];
    assert!(ordinary.copy(FLAG, &mut flag));
    assert_eq!(flag, [0]);
    assert!(ordinary.copy(FLAG, &mut flag));
    assert_eq!(flag, [1]);
    let mut header = [0; 16];
    assert!(ordinary.copy(HEADER, &mut header));
    assert_eq!(&header[12..16], &count.to_le_bytes());
    drop(ordinary);
    // No retained loan remains for Request Drop to retire after normal return.
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
        let pool = crate::acquisition::Pool::start(0, 1, move |job: &mut Job| {
            let Job::ChildFronts(request) = job else {
                panic!("unexpected fixture runner");
            };
            observed.fetch_add(1, Ordering::Relaxed);
            let payload = request.group_source.as_ref().unwrap();
            {
                let ordinary = Recorded::build(&payload.log);
                assert!(ordinary.copy(FLAG, &mut [0]));
                assert!(ordinary.copy(FLAG, &mut [0]));
            }
            let source = naval_history::OrderGroupSource::new(payload);
            let mut flag = [9];
            assert!(source.copy(FLAG, &mut flag));
            assert_eq!(flag, [1]);
            if mode == 2 {
                // Unwinding must drop the borrowed reader before completing
                // the request, so its ordinary decode can rebuild/reset it.
                panic!("synthetic ChildFronts failure with an active OOB reader");
            }
            drop(source);
            if mode == 1 {
                request.plans.fail_child_fronts();
            }
            Report::ChildFronts(OrdersReport)
        })
        .unwrap();
        let mut pending = Some(Job::ChildFronts(request(7)));
        let payload = loan(pending.as_ref().unwrap());
        let bytes = payload.log.bytes.as_ptr() as usize;
        let spans = payload.log.spans.as_ptr() as usize;
        // SAFETY: this executor reads only the request's owned Rust bytes.
        let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
        let completed = pool.finish(ticket).unwrap();
        if mode == 2 {
            assert!(matches!(completed.report, Err(Failure::Panicked)));
        } else {
            assert!(matches!(completed.report, Ok(Report::ChildFronts(_))));
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        inspect_job(completed.request, bytes, spans, 7);
        pool.shutdown().unwrap();
    }
}

#[test]
fn stopped_child_fronts_request_returns_its_unread_naval_loan() {
    let pool = crate::acquisition::Pool::start(0, 1, |_: &mut Job| -> Report {
        panic!("cancelled ChildFronts runner must not execute");
    })
    .unwrap();
    let mut pending = Some(Job::ChildFronts(request(8)));
    let payload = loan(pending.as_ref().unwrap());
    let bytes = payload.log.bytes.as_ptr() as usize;
    let spans = payload.log.spans.as_ptr() as usize;
    // SAFETY: the request owns all inputs and shutdown cancels it before run.
    let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
    pool.shutdown().unwrap();
    let completed = pool.finish(ticket).unwrap();
    assert!(matches!(completed.report, Err(Failure::Stopped)));
    assert_eq!(completed.table_reads, crate::read_accounting::ZERO);
    inspect_job(completed.request, bytes, spans, 8);
}

#[test]
fn admission_rejection_keeps_the_loan_for_refusal_and_ordinary_decode() {
    let pool = crate::acquisition::Pool::start(0, 1, |_: &mut Job| -> Report {
        panic!("rejected ChildFronts must never run");
    })
    .unwrap();
    pool.shutdown().unwrap();
    let mut pending = Some(Job::ChildFronts(request(9)));
    let payload = loan(pending.as_ref().unwrap());
    let bytes = payload.log.bytes.as_ptr() as usize;
    let spans = payload.log.spans.as_ptr() as usize;
    // SAFETY: stopped admission retains the owned request without executing it.
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

/// A ChildFronts pass that demands no army-group operand allocates nothing,
/// so a joining thread may run it while it is still queued; the role, which
/// sizes the reserved slots, stays worker-only for a pass that does.
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
