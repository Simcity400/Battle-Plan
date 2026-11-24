use super::*;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

fn fixture(high: bool) -> Arc<Pipeline> {
    let pipeline = Arc::new(Pipeline::new());
    if !high {
        for _ in 0..collector_extended_support::hourly::BUFFERS {
            pipeline.publish();
        }
    }
    // SAFETY: Only the current session's designated thread registers this role.
    unsafe { pipeline.register(Role::Decode) };
    pipeline
}
fn change(pipeline: &Pipeline, high: bool) {
    pipeline.available.store(
        if high {
            0
        } else {
            collector_extended_support::hourly::BUFFERS
        },
        Ordering::SeqCst,
    );
    pipeline.refresh();
}

fn apply(pipeline: &Pipeline, budget: &mut Budget, now: u64, native: &AtomicU32) {
    pipeline.reconcile(
        budget,
        || now,
        |_, _, value| {
            native.store(value, Ordering::SeqCst);
            true
        },
    );
}

#[test]
fn pressure_is_early_targeted_and_only_during_capture() {
    assert_eq!((0..=4).map(level).collect::<Vec<_>>(), [2, 1, 0, 0, 0]);
    let pipeline = fixture(false);
    assert_eq!(pipeline.pressure(), 0);
    let callback = pipeline.callback();
    assert_eq!(for_role(pipeline.pressure(), Role::Decode.slot()), 0);
    assert_eq!(for_role(pipeline.pressure(), Role::Sizing.slot()), 0);
    pipeline.claim();
    assert_eq!(for_role(pipeline.pressure(), Role::Spool.slot()), 1);
    assert_eq!(for_role(pipeline.pressure(), Role::Decode.slot()), 1);
    drop(callback);
    assert_eq!(pipeline.pressure(), 0);
}

#[test]
fn every_final_read_registration_and_syscall_race_retries_off_callback() {
    for boundary in [
        Boundary::WriterRegistered,
        Boundary::DesiredRead,
        Boundary::BeforeSyscall,
        Boundary::AfterSyscall,
        Boundary::Published,
        Boundary::FinalDesiredRead,
    ] {
        for high in [false, true] {
            let pipeline = fixture(high);
            let _callback = pipeline.callback();
            pipeline.targets[0]
                .get()
                .unwrap()
                .applied
                .store(UNKNOWN, Ordering::SeqCst);
            let weak = Arc::downgrade(&pipeline);
            let fired = Arc::new(AtomicBool::new(false));
            let hook_fired = Arc::clone(&fired);
            assert!(
                pipeline
                    .hook
                    .set(Box::new(move |at| {
                        if at == boundary && !hook_fired.swap(true, Ordering::SeqCst) {
                            change(&weak.upgrade().unwrap(), !high);
                        }
                    }))
                    .is_ok()
            );
            let native = AtomicU32::new(1);
            let mut budget = Budget::default();
            apply(&pipeline, &mut budget, 0, &native);
            assert!(fired.load(Ordering::SeqCst), "{boundary:?}");
            apply(&pipeline, &mut budget, 0, &native);
            assert_eq!(
                native.load(Ordering::SeqCst),
                if high { 0 } else { 2 },
                "{boundary:?}"
            );
            assert!(!pipeline.pending.load(Ordering::SeqCst), "{boundary:?}");
        }
    }
}

#[test]
fn writer_gate_has_one_attempt_and_cannot_overwrite_a_correction() {
    let pipeline = fixture(true);
    let _callback = pipeline.callback();
    let weak = Arc::downgrade(&pipeline);
    let fired = AtomicBool::new(false);
    assert!(
        pipeline
            .hook
            .set(Box::new(move |at| {
                if at == Boundary::WriterRegistered && !fired.swap(true, Ordering::SeqCst) {
                    let pipeline = weak.upgrade().unwrap();
                    pipeline.reconcile(
                        &mut Budget::default(),
                        || 0,
                        |_, _, _| panic!("second writer entered"),
                    );
                }
            }))
            .is_ok()
    );
    let mut budget = Budget::default();
    let native = AtomicU32::new(0);
    apply(&pipeline, &mut budget, 0, &native);
    assert!(pipeline.pending.load(Ordering::SeqCst));
    apply(&pipeline, &mut budget, 0, &native);
    assert!(!pipeline.pending.load(Ordering::SeqCst));
    assert_eq!(native.load(Ordering::SeqCst), 2);
}

#[test]
fn a_target_registered_after_the_final_read_is_not_lost() {
    let pipeline = fixture(true);
    let _callback = pipeline.callback();
    let weak = Arc::downgrade(&pipeline);
    let fired = AtomicBool::new(false);
    assert!(
        pipeline
            .hook
            .set(Box::new(move |at| {
                if at == Boundary::FinalDesiredRead && !fired.swap(true, Ordering::SeqCst) {
                    // SAFETY: Only the current session's designated thread registers this role.
                    unsafe { weak.upgrade().unwrap().register(Role::Sizing) };
                }
            }))
            .is_ok()
    );
    let mut budget = Budget::default();
    apply(&pipeline, &mut budget, 0, &AtomicU32::new(0));
    assert!(pipeline.pending.load(Ordering::SeqCst));
    apply(&pipeline, &mut budget, 0, &AtomicU32::new(0));
    assert_eq!(
        pipeline.targets[Role::Sizing.slot()]
            .get()
            .unwrap()
            .applied
            .load(Ordering::SeqCst),
        2
    );
    assert!(!pipeline.pending.load(Ordering::SeqCst));
}

#[test]
fn failed_syscalls_stay_unknown_and_retry_without_another_refresh() {
    let pipeline = fixture(true);
    let _callback = pipeline.callback();
    let mut budget = Budget::default();
    let calls = AtomicUsize::new(0);
    pipeline.reconcile(
        &mut budget,
        || 0,
        |_, _, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            false
        },
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(pipeline.pending.load(Ordering::SeqCst));
    assert_eq!(
        pipeline.targets[Role::Decode.slot()]
            .get()
            .unwrap()
            .applied
            .load(Ordering::SeqCst),
        UNKNOWN
    );
    let native = AtomicU32::new(0);
    apply(&pipeline, &mut budget, 0, &native);
    assert_eq!(native.load(Ordering::SeqCst), 2);
    assert!(!pipeline.pending.load(Ordering::SeqCst));
}

#[test]
fn a_long_sizing_item_is_demoted_at_expiry_and_cooldown_starts_after_success() {
    let pipeline = fixture(true);
    let _callback = pipeline.callback();
    // SAFETY: Only the current session's designated thread registers this role.
    unsafe { pipeline.register(Role::Sizing) };
    let mut budget = Budget::default();
    let native = AtomicU32::new(0);
    apply(&pipeline, &mut budget, 0, &native);
    assert_eq!(native.load(Ordering::SeqCst), 2);
    pipeline.reconcile(
        &mut budget,
        || BOOST_NS,
        |_, _, value| {
            assert_eq!(value, 0);
            false
        },
    );
    assert!(matches!(budget, Budget::Rest(None)));
    apply(&pipeline, &mut budget, BOOST_NS * 20, &native);
    assert_eq!(native.load(Ordering::SeqCst), 0);
    apply(&pipeline, &mut budget, BOOST_NS * 20 + REST_NS - 1, &native);
    assert_eq!(native.load(Ordering::SeqCst), 0);
    apply(&pipeline, &mut budget, BOOST_NS * 20 + REST_NS, &native);
    assert_eq!(native.load(Ordering::SeqCst), 2);
}

#[test]
fn expiry_during_the_last_syscall_queues_demotion() {
    let pipeline = fixture(true);
    let _callback = pipeline.callback();
    let mut budget = Budget::default();
    let clock = AtomicU64::new(0);
    pipeline.reconcile(
        &mut budget,
        || clock.load(Ordering::SeqCst),
        |_, _, _| {
            clock.store(BOOST_NS, Ordering::SeqCst);
            true
        },
    );
    assert!(pipeline.pending.load(Ordering::SeqCst));
    let native = AtomicU32::new(2);
    apply(&pipeline, &mut budget, BOOST_NS, &native);
    assert_eq!(native.load(Ordering::SeqCst), 0);
}

#[test]
fn callback_end_demotes_mid_item_and_notifications_and_unwind_allocate_nothing() {
    let pipeline = fixture(true);
    let callback = pipeline.callback();
    let mut budget = Budget::default();
    let native = AtomicU32::new(0);
    apply(&pipeline, &mut budget, 0, &native);
    collector_test_support::with_allocations_denied(|| {
        pipeline.refresh();
        drop(callback);
        apply(&pipeline, &mut budget, 0, &native);
    });
    assert_eq!(native.load(Ordering::SeqCst), 0);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _callback = pipeline.callback();
        panic!("synthetic unwind");
    }));
    assert_eq!(pipeline.callbacks.load(Ordering::SeqCst), 0);
    assert_eq!(pipeline.pressure(), 0);
}

#[test]
fn shutdown_racing_the_last_write_still_gets_a_normal_pass() {
    let pipeline = fixture(true);
    let _callback = pipeline.callback();
    let mut budget = Budget::default();
    pipeline.reconcile(
        &mut budget,
        || 0,
        |_, _, _| {
            pipeline.stopping.store(true, Ordering::SeqCst);
            true
        },
    );
    assert!(pipeline.pending.load(Ordering::SeqCst));
    let native = AtomicU32::new(2);
    apply(&pipeline, &mut budget, 0, &native);
    assert_eq!(native.load(Ordering::SeqCst), 0);
}

#[test]
fn the_owned_reconciler_starts_boost_disabled_and_joins_without_targets() {
    let pipeline = engine_allocation::shared(Pipeline::new()).unwrap();
    let mut worker =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { Reconciler::start(pipeline.clone()) }.unwrap();
    assert_ne!(worker.thread_id(), 0);
    let mut disabled = 0;
    let valid = unsafe {
        windows_sys::Win32::System::Threading::GetThreadPriorityBoost(
            worker.thread.as_ref().unwrap().as_raw_handle().cast(),
            &raw mut disabled,
        )
    };
    assert_ne!(valid, 0);
    assert_ne!(disabled, 0);
    worker.finish();
    assert!(worker.thread.is_none());
    assert!(pipeline.stopping.load(Ordering::SeqCst));
    assert!(!pipeline.writing.load(Ordering::SeqCst));
}

#[test]
fn a_delayed_announcer_cannot_hide_a_later_real_change() {
    let pipeline = fixture(true);
    let _callback = pipeline.callback();
    let old = pipeline.pressure();
    change(&pipeline, false);
    let mut budget = Budget::default();
    apply(&pipeline, &mut budget, 0, &AtomicU32::new(0));
    pipeline.announced.swap(old, Ordering::SeqCst);
    pipeline.notify();
    apply(&pipeline, &mut budget, 0, &AtomicU32::new(0));
    assert_eq!(pipeline.announced.load(Ordering::SeqCst), 0);
    change(&pipeline, true);
    assert!(pipeline.pending.load(Ordering::SeqCst));
    let native = AtomicU32::new(0);
    apply(&pipeline, &mut budget, REST_NS, &native);
    assert_eq!(native.load(Ordering::SeqCst), 2);
}

#[test]
fn availability_counts_every_published_frame_and_each_claim() {
    let pipeline = Pipeline::new();
    let _callback = pipeline.callback();
    pipeline.publish();
    assert_eq!(pipeline.free_count(), 1);
    assert_eq!(for_role(pipeline.pressure(), 0), 1);
    pipeline.publish();
    pipeline.claim();
    assert_eq!(pipeline.free_count(), 1);
    pipeline.claim();
    assert_eq!(pipeline.free_count(), 0);
    assert_eq!(for_role(pipeline.pressure(), 0), 2);
    pipeline.claim();
    assert_eq!(pipeline.free_count(), 0);
    for _ in 0..collector_extended_support::hourly::BUFFERS * 3 {
        pipeline.publish();
    }
    assert_eq!(
        pipeline.free_count(),
        collector_extended_support::hourly::BUFFERS * 3
    );
    assert_eq!(pipeline.pressure(), 0);
}
#[test]
fn ticking_decode_gets_its_own_slot_state() {
    assert_eq!(for_role(TICKING, Role::Decode.slot()), TICK_DECODE);
    assert_eq!(for_role(TICKING, Role::Spool.slot()), TICK);
}
