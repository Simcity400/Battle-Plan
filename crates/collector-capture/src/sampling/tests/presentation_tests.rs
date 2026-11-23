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
            .wait_timeout_while(
                self.open.lock().unwrap(),
                collector_test_support::TEST_HANG_GUARD,
                |open| !*open,
            )
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
        collector_divisions::extended::division_presentation::Captured::with_capacities(
            &Default::default(),
        );
    captured.status_workers = 3;
    acquisition::PresentationRequest {
        dispatch: Default::default(),
        countries: engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap(),
        context: collector_divisions::extended::division_presentation::CaptureContext {
            base: 0,
            game_state: 0,
            hour: 1,
        },
        captured,
        inputs: Some(
            engine_allocation::shared(collector_divisions::division_census::UnitInputs::fixture(
                vec![engine_native::carmy::DivisionSeed::default(); 1_536],
                engine_native::walk::Outcome::Rows { len: 1_536 },
            ))
            .unwrap(),
        ),
        army_stats: Some(
            collector_divisions::extended::division_presentation::held_store::Loan::fixture(
                Box::new(collector_division_parts::attack_stats::ArmyStatStore::new()),
            ),
        ),
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
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
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
                acquisition::Job::PresentationPrepare(_)
                | acquisition::Job::PresentationSettle(_) => {
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
    let inputs = engine_allocation::SharedBox::clone(request.inputs.as_ref().unwrap());
    let store = request.army_stats.as_deref().unwrap() as *const _;
    let mut presentation = PresentationPending {
        prepare: Some(unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::PresentationPrepare(request),
            )
        }),
        ..Default::default()
    };
    running
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
    for index in 0..2 {
        let pending = unsafe { submit_acquisition(&shared, &mut batch, unrelated(index)) };
        collector_test_support::with_allocations_denied(|| unsafe {
            presentation.wait_for(&shared, &mut batch, Some(&pending));
            drop(finish_acquisition(&shared, &mut batch, pending));
        });
        assert!(presentation.prepare.is_some());
        assert!(!presentation.ready(&batch));
    }
    prepare.open();
    batch.wait_ready(|batch| presentation.ready(batch).then_some(()));
    collector_test_support::with_allocations_denied(|| unsafe {
        presentation.advance(&shared, &mut batch);
    });
    for _ in 0..3 {
        range_running
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
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
    let pending = unsafe { submit_acquisition(&shared, &mut batch, unrelated(2)) };
    last_range.open();
    collector_test_support::with_allocations_denied(|| unsafe {
        presentation.wait_for(&shared, &mut batch, Some(&pending));
        drop(finish_acquisition(&shared, &mut batch, pending));
    });
    assert!(!presentation.waiting());
    assert!(presentation.pending.is_some());
    let (request, report) = collector_test_support::with_allocations_denied(|| unsafe {
        presentation.finish(&shared, &mut batch)
    });
    assert!(report.is_ok());
    assert!(engine_allocation::SharedBox::ptr_eq(
        request.inputs.as_ref().unwrap(),
        &inputs
    ));
    assert_eq!(request.army_stats.as_deref().unwrap() as *const _, store);
    assert_eq!(pool.calibration().executor.uncollected, 0);
    assert_eq!(pool.calibration().executor.caller_executed, 0);
}

#[test]
fn presentation_readiness_failed_preparation_or_range_returns_its_loan_without_retry() {
    for fail_prepare in [true, false] {
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let stitches = Arc::new(AtomicUsize::new(0));
        let pool = acquisition::Pool::with_test_runner(3, {
            let stitches = Arc::clone(&stitches);
            move |job| {
                match job {
                    acquisition::Job::PresentationPrepare(_)
                    | acquisition::Job::PresentationSettle(_)
                        if fail_prepare =>
                    {
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
        let mut request = request();
        batch.arm_presentation(&mut request);
        let inputs = engine_allocation::SharedBox::clone(request.inputs.as_ref().unwrap());
        let mut presentation = PresentationPending {
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
        assert!(engine_allocation::SharedBox::ptr_eq(
            request.inputs.as_ref().unwrap(),
            &inputs
        ));
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
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool = acquisition::Pool::with_test_runner(1, |_| panic!("rejected presentation ran"));
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { pool.shutdown() }.unwrap();
    let mut batch = acquisition::Batch::new(&pool);
    let request = request();
    let inputs = engine_allocation::SharedBox::clone(request.inputs.as_ref().unwrap());
    let mut presentation = PresentationPending {
        prepare: Some(unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::PresentationPrepare(request),
            )
        }),
        ..Default::default()
    };
    let (request, report) = collector_test_support::with_allocations_denied(|| unsafe {
        presentation.finish(&shared, &mut batch)
    });
    assert!(matches!(report, Err(acquisition::Failure::Stopped)));
    assert!(engine_allocation::SharedBox::ptr_eq(
        request.inputs.as_ref().unwrap(),
        &inputs
    ));
    assert!(presentation.parts.iter().all(Option::is_none));
    assert_eq!(pool.calibration().executor.uncollected, 0);
    assert_eq!(pool.calibration().executor.caller_executed, 0);
}

#[test]
fn presentation_readiness_batch_drain_owns_parked_preparation_and_all_range_leases() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
        acquisition::Pool::with_test_runner(3, |job| acquisition::run_for_test(job, &Unreadable));
    let mut request = request();
    let inputs = engine_allocation::SharedBox::clone(request.inputs.as_ref().unwrap());
    {
        let mut batch = acquisition::Batch::new(&pool);
        batch.arm_presentation(&mut request);
        let mut presentation = PresentationPending {
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
    }
    assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 1);
    assert_eq!(pool.calibration().executor.uncollected, 0);
}

#[test]
fn presentation_second_preparation_yields_to_unrelated_dispatches() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let gate = Arc::new(Gate::default());
    let prepares = AtomicUsize::new(0);
    let (started, running) = mpsc::channel();
    let pool = acquisition::Pool::with_test_runner(3, {
        let gate = Arc::clone(&gate);
        move |job| {
            if matches!(
                job,
                acquisition::Job::PresentationPrepare(_) | acquisition::Job::PresentationSettle(_)
            ) && prepares.fetch_add(1, Ordering::Relaxed) == 1
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
    running
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
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
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let last_preparation = Arc::new(Gate::default());
    let last = Arc::new(Gate::default());
    let stitched = Arc::new(Gate::default());
    let (prepared, preparing) = mpsc::channel();
    let (settled, settling) = mpsc::channel();
    let (started, running) = mpsc::channel();
    let preparations = AtomicUsize::new(0);
    let finals = AtomicUsize::new(0);
    let pool = acquisition::Pool::with_test_runner(4, {
        let last_preparation = Arc::clone(&last_preparation);
        let last = Arc::clone(&last);
        let stitched = Arc::clone(&stitched);
        move |job| {
            match job {
                acquisition::Job::PresentationStatus(request)
                    if matches!(
                        &request.part,
                        Some(collector_divisions::extended::division_presentation::StatusWork::Preparation { .. })
                    ) =>
                {
                    let index = preparations.fetch_add(1, Ordering::Relaxed);
                    prepared.send(index).unwrap();
                    if index == 2 {
                        last_preparation.wait();
                    }
                }
                acquisition::Job::PresentationSettle(_) => {
                    settled.send(()).unwrap();
                }
                acquisition::Job::PresentationStatus(request)
                    if matches!(
                        &request.part,
                        Some(collector_divisions::extended::division_presentation::StatusWork::Status { .. })
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
                    stitched.wait();
                    return acquisition::Report::ShipModels;
                }
                _ => {}
            }
            acquisition::run_for_test(job, &Unreadable)
        }
    });
    let mut batch = acquisition::Batch::new(&pool);
    let mut request = request();
    batch.arm_presentation(&mut request);
    let mut presentation = PresentationPending {
        prepare: Some(unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::PresentationPrepare(request),
            )
        }),
        ..Default::default()
    };
    for _ in 0..3 {
        preparing
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
    }
    assert!(settling.try_recv().is_err(), "a prefix cannot admit Settle");
    last_preparation.open();
    settling
        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
        .unwrap();
    for _ in 0..3 {
        running
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
    }
    assert_eq!(pool.calibration().executor.uncollected, 8);
    assert!(presentation.pending.is_none());
    let unrelated = unsafe { submit_acquisition(&shared, &mut batch, unrelated(0)) };
    last.open();
    stitched.wait();
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
        collector_workers::roles::Role::PresentationPreparation,
        collector_workers::roles::Role::PresentationStatus,
    ] {
        let counters = calibration.roles[role as usize].counters;
        assert_eq!(counters.accepted, 3);
        assert_eq!(counters.completed, 3);
        assert_eq!((counters.rejected, counters.inline_completed), (0, 0));
    }
    for role in [
        collector_workers::roles::Role::PresentationPrepare,
        collector_workers::roles::Role::PresentationSettle,
        collector_workers::roles::Role::Presentation,
    ] {
        let counters = calibration.roles[role as usize].counters;
        assert_eq!((counters.accepted, counters.completed), (1, 1));
        assert_eq!((counters.rejected, counters.inline_completed), (0, 0));
    }
    assert_eq!(calibration.executor.uncollected, 0);
    assert_eq!(calibration.executor.caller_executed, 0);
}

#[test]
fn settle_worker_releases_status_without_callback_collection_and_drains_its_children() {
    for drain in [false, true] {
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let (started, running) = mpsc::channel();
        let pool = acquisition::Pool::with_test_runner(3, move |job| {
            if matches!(job, acquisition::Job::PresentationStatus(request)
                if matches!(&request.part, Some(collector_divisions::extended::division_presentation::StatusWork::Status { .. })))
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
            inputs = engine_allocation::SharedBox::clone(request.inputs.as_ref().unwrap());
            presentation.prepare = Some(unsafe {
                submit_acquisition(
                    &shared,
                    &mut batch,
                    acquisition::Job::PresentationPrepare(request),
                )
            });
            for _ in 0..2 {
                batch.wait_ready(|batch| presentation.ready(batch).then_some(()));
                unsafe { presentation.advance(&shared, &mut batch) };
            }
            for _ in 0..3 {
                running
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
            }
            assert!(presentation.prepare.is_some());
            assert!(presentation.parts.iter().all(Option::is_none));
            if !drain {
                assert!(
                    unsafe { presentation.finish(&shared, &mut batch) }
                        .1
                        .is_ok()
                );
            }
        }
        assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 1);
        assert_eq!(pool.calibration().executor.uncollected, 0);
        assert_eq!(pool.calibration().executor.caller_executed, 0);
    }
}

#[test]
fn armed_setup_children_drain_without_collection_even_if_the_parent_fails_after_dispatch() {
    use collector_memory::read_accounting::{self, Capture, fixtures};
    use replay_data::rows::TableId;

    struct ReleaseOnDrop<'a>(&'a Gate, &'a Gate);
    impl Drop for ReleaseOnDrop<'_> {
        fn drop(&mut self) {
            self.0.open();
            self.1.open();
        }
    }

    for (fail_parent, fail_child) in [(false, false), (true, false), (false, true)] {
        let reads = Capture::new();
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let children = Arc::new(Gate::default());
        let parent = Arc::new(Gate::default());
        let (started, running) = mpsc::channel();
        let trace = Arc::new(fixtures::ReadTrace::default());
        let pool = acquisition::Pool::with_test_runner(4, {
            let children = Arc::clone(&children);
            let parent = Arc::clone(&parent);
            let ranges = AtomicUsize::new(0);
            let runner_trace = Arc::clone(&trace);
            move |job| {
                runner_trace.record((), || {
                if matches!(job, acquisition::Job::PresentationStatus(request) if request.preparation)
                {
                    let index = ranges.fetch_add(1, Ordering::Relaxed);
                    started.send(()).unwrap();
                    children.wait();
                    read_accounting::with_table(TableId::DivisionStatus, || fixtures::read(5));
                    assert!(!fail_child || index != 0, "range fixture failure");
                }
                let report = acquisition::run_for_test(job, &Unreadable);
                if matches!(job, acquisition::Job::PresentationPrepare(_)) {
                    parent.wait();
                    assert!(!fail_parent, "setup fixture failure after range release");
                }
                report
            })
            }
        });
        let mut request = request();
        let inputs = engine_allocation::SharedBox::clone(request.inputs.as_ref().unwrap());
        let slot = collector_divisions::extended::division_presentation::held_store::Slot::new(
            Box::new(collector_division_parts::attack_stats::ArmyStatStore::new()),
        )
        .unwrap();
        request.army_stats = slot.take();
        let lent = request.army_stats.as_deref().unwrap()
            as *const collector_division_parts::attack_stats::ArmyStatStore;
        {
            let mut batch = acquisition::Batch::new(&pool);
            let _release = ReleaseOnDrop(&children, &parent);
            batch.arm_presentation(&mut request);
            let _pending = unsafe {
                submit_acquisition(
                    &shared,
                    &mut batch,
                    acquisition::Job::PresentationPrepare(request),
                )
            };
            for _ in 0..3 {
                running
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
            }
            assert_eq!(
                pool.calibration().roles
                    [collector_workers::roles::Role::PresentationPreparation as usize]
                    .counters
                    .accepted,
                3
            );
            assert_eq!(
                pool.calibration().roles
                    [collector_workers::roles::Role::PresentationStatus as usize]
                    .counters
                    .accepted,
                0
            );
        }
        assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 1);
        assert_eq!(&*slot.take().unwrap() as *const _, lent);
        let stats = pool.calibration();
        assert_eq!(
            (
                stats.executor.active,
                stats.executor.uncollected,
                stats.executor.caller_executed
            ),
            (0, 0, 0)
        );
        assert!(stats.executor.accepted >= 4);
        assert_eq!(stats.executor.accepted, stats.executor.completed);
        for role in &stats.roles {
            assert_eq!(
                role.counters.accepted, role.counters.completed,
                "{:?}",
                role.role
            );
        }
        let ranges =
            stats.roles[collector_workers::roles::Role::PresentationPreparation as usize].counters;
        assert_eq!((ranges.accepted, ranges.completed), (3, 3));
        assert_eq!(
            (ranges.panicked, ranges.stopped),
            (u64::from(fail_child), 0)
        );
        assert_eq!(
            stats.roles[collector_workers::roles::Role::PresentationPrepare as usize]
                .counters
                .panicked,
            u64::from(fail_parent)
        );
        if fail_parent {
            assert_eq!(
                stats.roles[collector_workers::roles::Role::PresentationSettle as usize]
                    .counters
                    .accepted,
                0
            );
        }
        let mut expected = read_accounting::ZERO;
        expected[TableId::DivisionStatus.index()] = fixtures::copies(3, 15);
        assert_eq!(trace.take(()), expected);
        #[cfg(not(feature = "acquisition_profile"))]
        assert_eq!(reads.finish(), read_accounting::ZERO);
        #[cfg(feature = "acquisition_profile")]
        assert_eq!(reads.finish(), expected);
    }
}

#[test]
fn presentation_continuations_reuse_pool_storage_without_callback_allocations() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
        acquisition::Pool::with_test_runner(3, |job| acquisition::run_for_test(job, &Unreadable));
    for _ in 0..2 {
        let request = request();
        let inputs = engine_allocation::SharedBox::clone(request.inputs.as_ref().unwrap());
        let mut batch =
            collector_test_support::with_allocations_denied(|| acquisition::Batch::new(&pool));
        let request = collector_test_support::with_allocations_denied(|| {
            let mut request = request;
            batch.arm_presentation(&mut request);
            let ledger = batch.arm_presentation_inputs(request, 0, None, false);
            let mut handoff = ledger.try_close().expect("unlocked input handoff");
            handoff.take_request().unwrap()
        });
        let mut presentation = PresentationPending {
            prepare: Some(unsafe {
                submit_acquisition(
                    &shared,
                    &mut batch,
                    acquisition::Job::PresentationPrepare(request),
                )
            }),
            ..Default::default()
        };
        let (request, report) = unsafe { presentation.finish(&shared, &mut batch) };
        assert!(report.is_ok());
        drop(request);
        drop(batch);
        assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 1);
        assert_eq!(pool.calibration().executor.uncollected, 0);
    }
}

#[test]
fn input_publishers_release_presentation_before_callback_collection_and_drain_on_exit() {
    use super::super::divisions_test_kit as kit;
    for (drain, collect_early) in [(false, false), (false, true), (true, false)] {
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let mut bytes = kit::fixture();
        let roster = bytes.len();
        bytes.resize(roster + 1_024 * 8, 0);
        for slot in bytes[roster..].chunks_exact_mut(8) {
            slot.copy_from_slice(&(kit::A1 as u64).to_le_bytes());
        }
        let header = kit::C0 + ccountry::DIVISIONS;
        bytes[header..header + 8].copy_from_slice(&(roster as u64).to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&1_024_u32.to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&1_024_u32.to_le_bytes());
        let context = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            details_active: false,
            ..Default::default()
        };
        let mut reference = DivisionsJob::new_worker();
        assert!(copy_divisions_inputs_refresh(
            &kit::memory(&bytes),
            context,
            &mut reference
        ));
        let expected: Vec<_> = reference
            .census
            .unit_inputs()
            .unwrap()
            .seeds()
            .iter()
            .map(|seed| seed.division)
            .collect();
        let last = Arc::new(Gate::default());
        let prepare = Arc::new(Gate::default());
        let (started, running) = mpsc::channel();
        let (prepared, preparing) = mpsc::channel();
        let (consumed, consuming) = mpsc::channel();
        let pool = acquisition::Pool::with_test_runner(3, {
            let last = Arc::clone(&last);
            let prepare = Arc::clone(&prepare);
            move |job| {
                match job {
                    acquisition::Job::DivisionInputParts(_) => {
                        started.send(()).unwrap();
                        last.wait();
                    }
                    acquisition::Job::PresentationPrepare(_) => {
                        prepared.send(()).unwrap();
                        prepare.wait();
                        return acquisition::Report::PresentationPrepare(());
                    }
                    acquisition::Job::Divisions(_)
                    | acquisition::Job::PresentationMovement(_)
                    | acquisition::Job::PresentationSpeedTerms(_) => {
                        consumed.send(job.role()).unwrap();
                        return match job {
                            acquisition::Job::Divisions(_) => {
                                acquisition::Report::Divisions(Default::default())
                            }
                            acquisition::Job::PresentationMovement(_) => {
                                acquisition::Report::PresentationMovement(Default::default())
                            }
                            acquisition::Job::PresentationSpeedTerms(_) => {
                                acquisition::Report::PresentationSpeedTerms(Default::default())
                            }
                            _ => unreachable!(),
                        };
                    }
                    _ => {}
                }
                acquisition::run_for_test(job, &kit::memory(&bytes))
            }
        });
        {
            let mut batch = acquisition::Batch::new(&pool);
            let mut request = request();
            request.inputs = None;
            batch.arm_presentation(&mut request);
            let dispatch = batch.arm_presentation_inputs(request, 1, None, false);
            let countries =
                engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap();
            let mut payload = DivisionsJob::new_worker();
            payload.slots = Some((1, 2));
            payload.presentation = Some(Arc::clone(&dispatch));
            let master = unsafe {
                submit_acquisition(
                    &shared,
                    &mut batch,
                    acquisition::Job::DivisionInputs(acquisition::RawRequest {
                        countries: engine_allocation::SharedBox::clone(&countries),
                        context,
                        payload,
                    }),
                )
            };
            let part = unsafe {
                submit_acquisition(
                    &shared,
                    &mut batch,
                    acquisition::Job::DivisionInputParts(acquisition::DivisionPartRequest {
                        countries,
                        base: kit::BASE,
                        game_state: kit::GAME_STATE,
                        slots: (2, u32::MAX),
                        census: Box::new(collector_divisions::division_census::Census::new_worker()),
                        presentation: Some((Arc::clone(&dispatch), 0)),
                    }),
                )
            };
            running
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
            assert!(preparing.try_recv().is_err());
            let mut master = Some(master);
            let early = collect_early.then(|| unsafe {
                finish_acquisition(&shared, &mut batch, master.take().unwrap())
            });
            assert!(preparing.try_recv().is_err());
            last.open();
            preparing
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
            if !drain {
                let master = early.unwrap_or_else(|| unsafe {
                    finish_acquisition(&shared, &mut batch, master.take().unwrap())
                });
                let part = unsafe { finish_acquisition(&shared, &mut batch, part) };
                let acquisition::Job::DivisionInputs(mut master) = master.request else {
                    unreachable!()
                };
                let acquisition::Job::DivisionInputParts(mut part) = part.request else {
                    unreachable!()
                };
                let mut handoff = batch.wait_ready(|_| dispatch.try_close());
                assert_eq!(
                    handoff.restore_master(&mut master.payload.census),
                    (true, None)
                );
                handoff.restore_part(0, &mut part.census);
                let inputs = master.payload.census.unit_inputs().unwrap();
                assert_eq!(
                    inputs
                        .seeds()
                        .iter()
                        .map(|seed| seed.division)
                        .collect::<Vec<_>>(),
                    expected
                );
                let (pending, movement, speed_terms, split) = handoff.take_dispatched().unwrap();
                drop(handoff);
                assert!(split);
                assert!(engine_allocation::SharedBox::ptr_eq(
                    movement.inputs.as_ref().unwrap(),
                    inputs,
                ));
                assert!(engine_allocation::SharedBox::ptr_eq(
                    speed_terms.inputs.as_ref().unwrap(),
                    inputs,
                ));
                let consumers = [
                    unsafe {
                        submit_acquisition(&shared, &mut batch, acquisition::Job::Divisions(master))
                    },
                    unsafe {
                        submit_acquisition(
                            &shared,
                            &mut batch,
                            acquisition::Job::PresentationSpeedTerms(speed_terms),
                        )
                    },
                    unsafe {
                        submit_acquisition(
                            &shared,
                            &mut batch,
                            acquisition::Job::PresentationMovement(movement),
                        )
                    },
                ];
                let mut roles: Vec<_> = (0..3)
                    .map(|_| {
                        consuming
                            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                            .unwrap()
                    })
                    .collect();
                roles.sort_by_key(|role| *role as usize);
                assert_eq!(
                    roles,
                    [
                        collector_workers::roles::Role::Divisions,
                        collector_workers::roles::Role::PresentationMovement,
                        collector_workers::roles::Role::PresentationSpeedTerms,
                    ]
                );
                prepare.open();
                for pending in consumers {
                    drop(unsafe { finish_acquisition(&shared, &mut batch, pending) });
                }
                let (ticket, parked) = batch.adopt_presentation_ticket(pending);
                assert!(!parked);
                let mut completed = None;
                assert!(batch.finish_into(ticket, &mut completed));
                assert!(matches!(
                    completed.as_ref().unwrap().report,
                    Ok(acquisition::Report::PresentationPrepare(()))
                ));
            } else {
                prepare.open();
            }
        }
        assert_eq!(pool.calibration().executor.uncollected, 0);
        assert_eq!(pool.calibration().executor.caller_executed, 0);
    }
}

#[test]
fn input_publishers_without_the_status_side_join_the_census_and_submit_nothing() {
    use super::super::divisions_test_kit as kit;
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let bytes = kit::fixture();
    let context = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        details_active: false,
        ..Default::default()
    };
    let mut reference = DivisionsJob::new_worker();
    assert!(copy_divisions_inputs_refresh(
        &kit::memory(&bytes),
        context,
        &mut reference
    ));
    let expected: Vec<_> = reference
        .census
        .unit_inputs()
        .unwrap()
        .seeds()
        .iter()
        .map(|seed| seed.division)
        .collect();
    let (ran, roles) = mpsc::channel();
    let pool = acquisition::Pool::with_test_runner(2, move |job| {
        ran.send(job.role()).unwrap();
        acquisition::run_for_test(job, &kit::memory(&bytes))
    });
    {
        let mut batch = acquisition::Batch::new(&pool);
        let mut request = request();
        request.inputs = None;
        request.army_stats = None;
        let dispatch = batch.arm_presentation_inputs(request, 1, None, true);
        let countries = engine_allocation::shared(Cache::try_with_capacity(0).unwrap()).unwrap();
        let mut payload = DivisionsJob::new_worker();
        payload.slots = Some((1, 2));
        payload.presentation = Some(Arc::clone(&dispatch));
        let master = unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::DivisionInputs(acquisition::RawRequest {
                    countries: engine_allocation::SharedBox::clone(&countries),
                    context,
                    payload,
                }),
            )
        };
        let part = unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::DivisionInputParts(acquisition::DivisionPartRequest {
                    countries,
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    slots: (2, u32::MAX),
                    census: Box::new(collector_divisions::division_census::Census::new_worker()),
                    presentation: Some((Arc::clone(&dispatch), 0)),
                }),
            )
        };
        let master = unsafe { finish_acquisition(&shared, &mut batch, master) };
        let part = unsafe { finish_acquisition(&shared, &mut batch, part) };
        let deadline = std::time::Instant::now() + collector_test_support::TEST_HANG_GUARD;
        while pool.calibration().executor.active != 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "publishers never ended"
            );
            std::thread::yield_now();
        }
        let acquisition::Job::DivisionInputs(mut master) = master.request else {
            unreachable!()
        };
        let acquisition::Job::DivisionInputParts(mut part) = part.request else {
            unreachable!()
        };
        let mut handoff = batch.wait_ready(|_| dispatch.try_close());
        assert_eq!(
            handoff.restore_master(&mut master.payload.census),
            (true, None)
        );
        handoff.restore_part(0, &mut part.census);
        assert_eq!(
            master
                .payload
                .census
                .unit_inputs()
                .unwrap()
                .seeds()
                .iter()
                .map(|seed| seed.division)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(handoff.take_dispatched().is_none());
        let mut presentation = PresentationPending {
            omitted: Some(handoff.take_request().expect("request held back")),
            ..Default::default()
        };
        drop(handoff);
        let (request, report) = unsafe { presentation.finish(&shared, &mut batch) };
        assert!(report.is_ok());
        assert!(request.inputs.is_none() && request.army_stats.is_none());
        assert!(matches!(
            request.captured.outcomes()[1],
            engine_native::walk::Outcome::Unavailable { .. }
        ));
    }
    let mut seen: Vec<_> = roles.try_iter().collect();
    seen.sort_by_key(|role| *role as usize);
    assert_eq!(
        seen,
        [
            collector_workers::roles::Role::DivisionInputParts,
            collector_workers::roles::Role::DivisionInputs
        ]
    );
    assert_eq!(pool.calibration().executor.uncollected, 0);
}

#[test]
fn final_status_worker_failure_returns_the_original_loan_without_stitch_or_retry() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let release = Arc::new(Gate::default());
    let stitches = Arc::new(AtomicUsize::new(0));
    let (started, running) = mpsc::channel();
    let pool = acquisition::Pool::with_test_runner(3, {
        let release = Arc::clone(&release);
        let stitches = Arc::clone(&stitches);
        move |job| {
            if let acquisition::Job::PresentationStatus(request) = job
                && let Some((_, index)) = &request.stitch
            {
                started.send(*index).unwrap();
                release.wait();
                assert_ne!(*index, 1, "synthetic final-status failure");
            }
            if matches!(job, acquisition::Job::Presentation(_)) {
                stitches.fetch_add(1, Ordering::Relaxed);
            }
            acquisition::run_for_test(job, &Unreadable)
        }
    });
    let mut batch = acquisition::Batch::new(&pool);
    let request = request();
    let store = request.army_stats.as_deref().unwrap() as *const _;
    let inputs = engine_allocation::SharedBox::clone(request.inputs.as_ref().unwrap());
    let mut presentation = PresentationPending {
        prepare: Some(unsafe {
            submit_acquisition(
                &shared,
                &mut batch,
                acquisition::Job::PresentationPrepare(request),
            )
        }),
        ..Default::default()
    };
    for _ in 0..2 {
        batch.wait_ready(|batch| presentation.ready(batch).then_some(()));
        unsafe { presentation.advance(&shared, &mut batch) };
    }
    for _ in 0..3 {
        running
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
    }
    release.open();
    let (request, report) = unsafe { presentation.finish(&shared, &mut batch) };
    assert!(matches!(report, Err(acquisition::Failure::Panicked)));
    assert!(engine_allocation::SharedBox::ptr_eq(
        request.inputs.as_ref().unwrap(),
        &inputs
    ));
    assert_eq!(request.army_stats.as_deref().unwrap() as *const _, store);
    assert_eq!(stitches.load(Ordering::Relaxed), 0);
    assert_eq!(pool.calibration().executor.uncollected, 0);
}
