use super::*;
use std::sync::{Condvar, Mutex, mpsc};

#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn open(&self) {
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
        assert!(
            *open && !timeout.timed_out(),
            "stitch fixture gate never opened"
        );
    }
}

fn peer(index: usize) -> acquisition::Job {
    acquisition::Job::Provinces(acquisition::ProvincesRequest {
        base: index,
        game_state: 0,
        payload: engine_native::cprovince::Capture::try_with_capacity(0, 0, 0).unwrap(),
    })
}

fn deferred_stitch_failure(naval_panics: bool) {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let naval_gate = Arc::new(Gate::default());
    let peer_gate = Arc::new(Gate::default());
    let (started, running) = mpsc::channel();
    let pool = acquisition::Pool::with_test_runner(2, {
        let naval_gate = Arc::clone(&naval_gate);
        let peer_gate = Arc::clone(&peer_gate);
        move |job| match job {
            acquisition::Job::OrdersStitch(_) => {
                acquisition::Report::OrdersStitch(acquisition::StitchReport {
                    contained_panic: true,
                })
            }
            acquisition::Job::Naval(_) => {
                started.send("naval").unwrap();
                naval_gate.wait();
                assert!(!naval_panics, "synthetic Naval failure after admission");
                acquisition::Report::Naval(Default::default())
            }
            acquisition::Job::Provinces(request) => {
                if request.base == 0 {
                    started.send("peer").unwrap();
                    peer_gate.wait();
                }
                acquisition::Report::ShipModels
            }
            _ => unreachable!("stitch barrier fixture job"),
        }
    });
    let mut batch = acquisition::Batch::new(&pool);
    let stitch = unsafe {
        submit_acquisition(
            &shared,
            &mut batch,
            acquisition::Job::OrdersStitch(stitch_request(0, None)),
        )
    };
    batch.wait_ready(|batch| pending_complete(batch, &stitch).then_some(()));
    assert!(!stitch_can_advance(&batch, &stitch, false));
    let naval = unsafe {
        submit_acquisition(
            &shared,
            &mut batch,
            acquisition::Job::Naval(acquisition::NavalRequest {
                countries: engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap(),
                context: collector_naval_history::extended::naval_history::NavalCopyContext {
                    base: 0,
                    game_state: 0,
                    hour: 1,
                    force_full: false,
                },
                payload: Default::default(),
            }),
        )
    };
    assert_eq!(
        running
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap(),
        "naval"
    );
    let blocker = unsafe { submit_acquisition(&shared, &mut batch, peer(0)) };
    assert_eq!(
        running
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap(),
        "peer"
    );
    let lower = unsafe { submit_acquisition(&shared, &mut batch, peer(1)) };
    assert_eq!(pool.calibration().executor.queued, 1);
    assert!(!pending_complete(&batch, &lower));
    let mut barrier = StitchFailureBarrier::default();
    assert!(!barrier.ready(&batch, Some(&stitch), Some(&naval), true));
    barrier.reach(&batch);
    assert_eq!(barrier.admissible, Some(true));
    assert!(!barrier.ready(&batch, Some(&stitch), Some(&naval), true));

    peer_gate.open();
    let next = batch.wait_ready(|batch| {
        if stitch_can_advance(batch, &stitch, false)
            || barrier.ready(batch, Some(&stitch), Some(&naval), true)
        {
            Some("stitch")
        } else {
            pending_complete(batch, &lower).then_some("lower")
        }
    });
    assert_eq!(next, "lower");
    assert!(!batch.stopped());
    let completed = unsafe { finish_acquisition(&shared, &mut batch, lower) };
    assert!(
        completed.report.is_ok(),
        "lower-priority work was cancelled"
    );
    let acquisition::Job::Provinces(request) = completed.request else {
        unreachable!("lower-priority request lost")
    };
    assert_eq!(request.base, 1);
    assert!(
        unsafe { finish_acquisition(&shared, &mut batch, blocker) }
            .report
            .is_ok()
    );

    naval_gate.open();
    batch.wait_ready(|batch| pending_complete(batch, &naval).then_some(()));
    assert_eq!(batch.stopped(), naval_panics);
    barrier.reach(&batch);
    assert_eq!(barrier.admissible, Some(true));
    assert!(!barrier.ready(&batch, Some(&stitch), Some(&naval), false));
    assert!(barrier.ready(&batch, Some(&stitch), Some(&naval), true));
    let completed = unsafe { finish_acquisition(&shared, &mut batch, naval) };
    assert_eq!(completed.report.is_err(), naval_panics);
    if naval_panics {
        assert!(matches!(
            completed.report,
            Err(acquisition::Failure::Panicked)
        ));
    }
    drop(completed);
    let completed = unsafe { finish_acquisition(&shared, &mut batch, stitch) };
    let Ok(acquisition::Report::OrdersStitch(report)) = completed.report else {
        unreachable!("contained stitch panic must remain a report")
    };
    let acquisition::Job::OrdersStitch(mut request) = completed.request else {
        unreachable!("stitch request lost")
    };
    settle_stitch(
        &batch,
        &mut request,
        report,
        None,
        barrier.admissible.unwrap(),
    );
    let reason = "army order acquisition failed";
    assert_eq!(request.orders.rows.outcome, Outcome::Unavailable { reason });
    assert_eq!(request.plans.outcomes()[0], Outcome::Unavailable { reason });
    let calibration = pool.calibration();
    assert!(batch.stopped());
    assert_eq!(calibration.executor.panicked, 1 + u64::from(naval_panics));
    assert_eq!(
        calibration.executor.stopped, 0,
        "no queued work was cancelled"
    );
    assert_eq!(calibration.executor.uncollected, 0);
    assert_eq!(calibration.executor.caller_executed, 0);
    assert_eq!(
        calibration.roles[collector_workers::roles::Role::OrdersStitch as usize]
            .counters
            .panicked,
        1
    );
}

#[test]
fn stitch_failure_waits_for_naval_before_cancelling_queued_work() {
    deferred_stitch_failure(false);
}

#[test]
fn stitch_failure_keeps_pre_naval_admission_when_naval_panics() {
    deferred_stitch_failure(true);
}
