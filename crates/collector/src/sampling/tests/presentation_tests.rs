//! Publication gates exercise the production callback's presentation scheduler.
use super::*;
use std::sync::{Condvar, Mutex, atomic::AtomicUsize, mpsc};

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
            .wait_timeout_while(self.open.lock().unwrap(), crate::TEST_HANG_GUARD, |open| {
                !*open
            })
            .unwrap();
        assert!(
            *open && !timeout.timed_out(),
            "presentation fixture gate never opened"
        );
    }
}

struct Unreadable;
impl engine_native::Memory for Unreadable {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        false
    }
}

fn request() -> acquisition::PresentationRequest {
    let mut captured =
        extended::division_presentation::Captured::with_capacities(&Default::default());
    captured.status_workers = 3;
    acquisition::PresentationRequest {
        dispatch: Default::default(),
        countries: Arc::new(Cache::try_with_capacity(0).unwrap()),
        context: extended::division_presentation::CaptureContext {
            base: 0,
            game_state: 0,
            hour: 1,
        },
        captured,
        inputs: Some(Arc::new(crate::division_census::UnitInputs::fixture(
            vec![engine_native::carmy::DivisionSeed::default(); 1_536],
            engine_native::walk::Outcome::Rows { len: 1_536 },
        ))),
        army_stats: Some(extended::division_presentation::held_store::Loan::fixture(Box::new(extended::attack_stats::ArmyStatStore::new()))),
    }
}

fn unrelated(index: usize) -> acquisition::Job {
    acquisition::Job::Provinces(acquisition::ProvincesRequest {
        base: index,
        game_state: 0,
        payload: engine_native::cprovince::Capture::try_with_capacity(0, 0, 0).unwrap(),
    })
}

#[test]
fn presentation_readiness_never_joins_preparation_before_unrelated_dispatches() {
    let shared = Shared::new(0, 0, 1);
    let prepare = Arc::new(Gate::default());
    let last_range = Arc::new(Gate::default());
    let later_join = Arc::new(Gate::default());
    let (started, running) = mpsc::channel();
    let (range_started, range_running) = mpsc::channel();
    let ranges = AtomicUsize::new(0);
    let pool = acquisition::Pool::with_test_runner(4, {
        let prepare = Arc::clone(&prepare);
        let last_range = Arc::clone(&last_range);
        let later_join = Arc::clone(&later_join);
        move |job| {
            match job {
                acquisition::Job::PresentationPrepare(_) | acquisition::Job::PresentationSettle(_) => {
                    started.send(()).unwrap();
                    prepare.wait();
                }
                acquisition::Job::PresentationStatus(_) => {
                    let index = ranges.fetch_add(1, Ordering::Relaxed);
                    range_started.send(index).unwrap();
                    if index == 2 {
                        last_range.wait();
                    }
                }
                acquisition::Job::Presentation(_) => {
                    // The real stitch asserts exclusive preparation ownership.
                    let report = acquisition::run_for_test(job, &Unreadable);
                    later_join.open();
                    return report;
                }
                acquisition::Job::Provinces(request) => {
                    if request.base == 2 {
                        later_join.wait();
                    }
                    return acquisition::Report::ShipModels;
                }
                _ => unreachable!("presentation fixture job"),
            }
            acquisition::run_for_test(job, &Unreadable)
        }
    });
    let mut batch = acquisition::Batch::new(&pool);
    let request = request();
    let inputs = Arc::clone(request.inputs.as_ref().unwrap());
    let store = request.army_stats.as_deref().unwrap() as *const _;
    let mut presentation = PresentationPending {
        // SAFETY: synthetic worker reads only the fixture; batch drains all tickets.
        prepare: Some(unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::PresentationPrepare(request),
            )
        }),
        ..Default::default()
    };
    running.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
    for index in 0..2 {
        // An unrelated producer and its consumer both dispatch and collect
        // while preparation remains gated. No elapsed-time guess orders this.
        let pending = unsafe { submit_acquisition(&shared, &mut batch, unrelated(index)) };
        crate::test_support::with_allocations_denied(|| unsafe {
            presentation.wait_for(&shared, &mut batch, Some(&pending));
            drop(finish_acquisition(&shared, &mut batch, pending));
        });
        assert!(presentation.prepare.is_some());
        assert!(!presentation.ready(&batch));
    }
    prepare.open();
    batch.wait_ready(|batch| presentation.ready(batch).then_some(()));
    crate::test_support::with_allocations_denied(|| unsafe {
        presentation.advance(&shared, &mut batch);
    });
    for _ in 0..3 {
        range_running.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
    }
    batch.wait_ready(|batch| {
        (presentation
            .parts
            .iter()
            .flatten()
            .filter(|part| pending_complete(batch, part))
            .count()
            == 2)
            .then_some(())
    });
    assert!(
        !presentation.ready(&batch),
        "stitch must await the last published range"
    );
    assert!(presentation.pending.is_none());
    // Model an assembly-only join after the early scheduler has exited. It
    // cannot finish until the scheduler dispatches the presentation stitch.
    let pending = unsafe { submit_acquisition(&shared, &mut batch, unrelated(2)) };
    last_range.open();
    crate::test_support::with_allocations_denied(|| unsafe {
        presentation.wait_for(&shared, &mut batch, Some(&pending));
        drop(finish_acquisition(&shared, &mut batch, pending));
    });
    assert!(!presentation.waiting());
    assert!(presentation.pending.is_some());
    let (request, report) = crate::test_support::with_allocations_denied(|| unsafe {
        presentation.finish(&shared, &mut batch)
    });
    assert!(report.is_ok());
    assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
    assert_eq!(request.army_stats.as_deref().unwrap() as *const _, store);
    assert_eq!(pool.calibration().executor.uncollected, 0);
    assert_eq!(pool.calibration().executor.caller_executed, 0);
}

#[test]
fn presentation_readiness_failed_preparation_or_range_returns_its_loan_without_retry() {
    for fail_prepare in [true, false] {
        let shared = Shared::new(0, 0, 1);
        let stitches = Arc::new(AtomicUsize::new(0));
        let pool = acquisition::Pool::with_test_runner(3, {
            let stitches = Arc::clone(&stitches);
            move |job| {
                match job {
                    acquisition::Job::PresentationPrepare(_) | acquisition::Job::PresentationSettle(_) if fail_prepare => {
                        panic!("preparation fixture fault")
                    }
                    acquisition::Job::PresentationStatus(_) => panic!("range fixture fault"),
                    acquisition::Job::Presentation(_) => {
                        stitches.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
                acquisition::run_for_test(job, &Unreadable)
            }
        });
        let mut batch = acquisition::Batch::new(&pool);
        let request = request();
        let inputs = Arc::clone(request.inputs.as_ref().unwrap());
        let mut presentation = PresentationPending {
            // SAFETY: synthetic requests only; batch drains all accepted ranges.
            prepare: Some(unsafe {
                submit_acquisition(
                    &shared,
                    &mut batch,
                    acquisition::Job::PresentationPrepare(request),
                )
            }),
            ..Default::default()
        };
        let (mut request, report) = unsafe { presentation.finish(&shared, &mut batch) };
        assert!(report.is_err());
        if fail_prepare {
            assert!(matches!(report, Err(acquisition::Failure::Panicked)));
        }
        assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
        request
            .captured
            .fail_owned_capture("division presentation acquisition failed");
        assert!(!request.captured.preparation_pending());
        assert_eq!(
            stitches.load(Ordering::Relaxed),
            0,
            "no serial retry after failure"
        );
        assert_eq!(pool.calibration().executor.uncollected, 0);
        assert_eq!(pool.calibration().executor.caller_executed, 0);
    }
}

#[test]
fn presentation_readiness_rejected_preparation_never_runs_inline_or_retries() {
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::with_test_runner(1, |_| panic!("rejected presentation ran"));
    pool.shutdown().unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let request = request();
    let inputs = Arc::clone(request.inputs.as_ref().unwrap());
    let mut presentation = PresentationPending {
        // SAFETY: the stopped pool cannot read native memory.
        prepare: Some(unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::PresentationPrepare(request),
            )
        }),
        ..Default::default()
    };
    let (request, report) = crate::test_support::with_allocations_denied(|| unsafe {
        presentation.finish(&shared, &mut batch)
    });
    assert!(matches!(report, Err(acquisition::Failure::Stopped)));
    assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
    assert!(presentation.parts.iter().all(Option::is_none));
    assert_eq!(pool.calibration().executor.uncollected, 0);
    assert_eq!(pool.calibration().executor.caller_executed, 0);
}

#[test]
fn presentation_readiness_batch_drain_owns_parked_preparation_and_all_range_leases() {
    let shared = Shared::new(0, 0, 1);
    let pool =
        acquisition::Pool::with_test_runner(3, |job| acquisition::run_for_test(job, &Unreadable));
    let request = request();
    let inputs = Arc::clone(request.inputs.as_ref().unwrap());
    {
        let mut batch = acquisition::Batch::new(&pool);
        let mut presentation = PresentationPending {
            // SAFETY: all synthetic jobs and parked preparation drain with the batch.
            prepare: Some(unsafe {
                submit_acquisition(
                    &shared,
                    &mut batch,
                    acquisition::Job::PresentationPrepare(request),
                )
            }),
            ..Default::default()
        };
        batch.wait_ready(|batch| presentation.ready(batch).then_some(()));
        unsafe { presentation.advance(&shared, &mut batch) };
        assert!(presentation.prepared.is_some());
        assert_eq!(presentation.parts.iter().flatten().count(), 3);
        // The callback may leave before collecting a single range. No request
        // or source lease is hidden in the ticket-only scheduler on that exit.
    }
    assert_eq!(Arc::strong_count(&inputs), 1);
    assert_eq!(pool.calibration().executor.uncollected, 0);
}

#[test]
fn presentation_second_preparation_yields_to_unrelated_dispatches() {
    let shared = Shared::new(0, 0, 1);
    let gate = Arc::new(Gate::default());
    let prepares = AtomicUsize::new(0);
    let (started, running) = mpsc::channel();
    let pool = acquisition::Pool::with_test_runner(3, {
        let gate = Arc::clone(&gate);
        move |job| {
            if matches!(job, acquisition::Job::PresentationPrepare(_) | acquisition::Job::PresentationSettle(_))
                && prepares.fetch_add(1, Ordering::Relaxed) == 1
            {
                started.send(()).unwrap();
                gate.wait();
            }
            if matches!(job, acquisition::Job::Provinces(_)) {
                return acquisition::Report::ShipModels;
            }
            acquisition::run_for_test(job, &Unreadable)
        }
    });
    let mut batch = acquisition::Batch::new(&pool);
    let mut presentation = PresentationPending {
        // SAFETY: synthetic memory only; this batch retains and drains every lease.
        prepare: Some(unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::PresentationPrepare(request()),
            )
        }),
        ..Default::default()
    };
    for _ in 0..2 {
        batch.wait_ready(|batch| presentation.ready(batch).then_some(()));
        unsafe { presentation.advance(&shared, &mut batch) };
    }
    running.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
    let pending = unsafe { submit_acquisition(&shared, &mut batch, unrelated(0)) };
    unsafe {
        presentation.wait_for(&shared, &mut batch, Some(&pending));
        drop(finish_acquisition(&shared, &mut batch, pending));
    }
    assert!(presentation.prepare.is_some());
    assert!(!presentation.ready(&batch));
    gate.open();
    assert!(
        unsafe { presentation.finish(&shared, &mut batch) }
            .1
            .is_ok()
    );
    assert_eq!(pool.calibration().executor.uncollected, 0);
    assert_eq!(pool.calibration().executor.caller_executed, 0);
}

#[test]
fn final_status_publication_dispatches_stitch_before_unrelated_join() {
    let shared = Shared::new(0, 0, 1);
    let last = Arc::new(Gate::default());
    let stitched = Arc::new(Gate::default());
    let (started, running) = mpsc::channel();
    let finals = AtomicUsize::new(0);
    let pool = acquisition::Pool::with_test_runner(4, {
        let last = Arc::clone(&last);
        let stitched = Arc::clone(&stitched);
        move |job| {
            match job {
                acquisition::Job::PresentationStatus(request)
                    if matches!(
                        &request.part,
                        extended::division_presentation::StatusWork::Status { .. }
                    ) =>
                {
                    let index = finals.fetch_add(1, Ordering::Relaxed);
                    started.send(index).unwrap();
                    if index == 2 {
                        last.wait();
                    }
                }
                acquisition::Job::Presentation(_) => {
                    stitched.open();
                }
                acquisition::Job::Provinces(_) => {
                    // Models an unrelated long Movement/SpeedTerms/FinalInputs
                    // join: it cannot publish until stitch is already dispatched.
                    stitched.wait();
                    return acquisition::Report::ShipModels;
                }
                _ => {}
            }
            acquisition::run_for_test(job, &Unreadable)
        }
    });
    let mut batch = acquisition::Batch::new(&pool);
    let mut presentation = PresentationPending {
        // SAFETY: fixture memory only; the batch drains both waves and their store loan.
        prepare: Some(unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::PresentationPrepare(request()),
            )
        }),
        ..Default::default()
    };
    // Initial setup -> preparation ranges -> ordered prepare -> final ranges.
    for _ in 0..3 {
        batch.wait_ready(|batch| presentation.ready(batch).then_some(()));
        unsafe { presentation.advance(&shared, &mut batch) };
    }
    for _ in 0..3 {
        running.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
    }
    assert!(!presentation.ready(&batch));
    assert!(presentation.pending.is_none());
    let unrelated = unsafe { submit_acquisition(&shared, &mut batch, unrelated(0)) };
    last.open();
    unsafe {
        presentation.wait_for(&shared, &mut batch, Some(&unrelated));
        drop(finish_acquisition(&shared, &mut batch, unrelated));
    }
    assert!(
        presentation.pending.is_some(),
        "final ranges alone release stitch"
    );
    assert!(
        unsafe { presentation.finish(&shared, &mut batch) }
            .1
            .is_ok()
    );
    let calibration = pool.calibration();
    for role in [
        acquisition::Role::PresentationPreparation,
        acquisition::Role::PresentationStatus,
    ] {
        let counters = calibration.roles[role as usize].counters;
        assert_eq!(counters.accepted, 3);
        assert_eq!(counters.completed, 3);
    }
    assert_eq!(calibration.executor.uncollected, 0);
    assert_eq!(calibration.executor.caller_executed, 0);
}

#[test]
fn settle_worker_releases_status_without_callback_collection_and_drains_its_children() {
    for drain in [false, true] {
        let shared = Shared::new(0, 0, 1);
        let (started, running) = mpsc::channel();
        let pool = acquisition::Pool::with_test_runner(3, move |job| {
            if matches!(job, acquisition::Job::PresentationStatus(request)
                if matches!(&request.part, extended::division_presentation::StatusWork::Status { .. }))
            {
                started.send(()).unwrap();
            }
            acquisition::run_for_test(job, &Unreadable)
        });
        let inputs;
        {
            let mut batch = acquisition::Batch::new(&pool);
            let mut presentation = PresentationPending::default();
            let request = request();
            inputs = Arc::clone(request.inputs.as_ref().unwrap());
            // SAFETY: synthetic memory; both the Batch and its child's ledger drain.
            presentation.prepare = Some(unsafe {
                submit_acquisition(&shared, &mut batch, acquisition::Job::PresentationPrepare(request))
            });
            // Initial setup and observation ranges publish; the ordered worker
            // is dispatched but deliberately never collected before this proof.
            for _ in 0..2 {
                batch.wait_ready(|batch| presentation.ready(batch).then_some(()));
                unsafe { presentation.advance(&shared, &mut batch) };
            }
            for _ in 0..3 {
                running.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
            }
            assert!(presentation.prepare.is_some());
            assert!(presentation.parts.iter().all(Option::is_none));
            if !drain {
                assert!(unsafe { presentation.finish(&shared, &mut batch) }.1.is_ok());
            }
        }
        assert_eq!(Arc::strong_count(&inputs), 1);
        assert_eq!(pool.calibration().executor.uncollected, 0);
        assert_eq!(pool.calibration().executor.caller_executed, 0);
    }
}
