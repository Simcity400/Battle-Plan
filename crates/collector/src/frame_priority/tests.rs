use super::*;
use std::sync::atomic::AtomicU64;

fn fixture(high: bool) -> Arc<Pipeline> {
    let pipeline = Arc::new(Pipeline::new());
    if !high {
        for index in 0..crate::sampling::BUFFERS {
            pipeline.publish(index);
        }
    }
    // These synthetic handles reach only mock setters, never native priority changes.
    pipeline.register(Role::Decode);
    pipeline
}
fn change(pipeline: &Pipeline, high: bool) {
    pipeline.available.store(
        if high {
            0
        } else {
            (1 << crate::sampling::BUFFERS) - 1
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
    assert_eq!(
        (0..=4).map(|free| level(free, 0)).collect::<Vec<_>>(),
        [2, 2, 1, 0, 0]
    );
    let pipeline = fixture(false);
    pipeline.expect_store(&AtomicBool::new(true));
    assert_eq!(pipeline.pressure(), 0);
    let callback = pipeline.callback();
    assert_eq!(for_role(pipeline.pressure(), Role::Decode as usize), 2);
    assert_eq!(for_role(pipeline.pressure(), Role::Sizing as usize), 0);
    pipeline.store_returned();
    assert_eq!(pipeline.pressure(), 0);
    pipeline.claim(0);
    pipeline.claim(1);
    assert_eq!(for_role(pipeline.pressure(), Role::Spool as usize), 1);
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
            // Reconciliation runs without another callback refresh, even after the final desired read.
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
                    weak.upgrade().unwrap().register(Role::Sizing);
                }
            }))
            .is_ok()
    );
    let mut budget = Budget::default();
    apply(&pipeline, &mut budget, 0, &AtomicU32::new(0));
    assert!(pipeline.pending.load(Ordering::SeqCst));
    apply(&pipeline, &mut budget, 0, &AtomicU32::new(0));
    assert_eq!(
        pipeline.targets[Role::Sizing as usize]
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
        pipeline.targets[Role::Decode as usize]
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
fn a_pass_is_bounded_to_five_setters_and_steady_refreshes_do_not_set_or_wake() {
    let pipeline = fixture(true);
    let callback = pipeline.callback();
    for role in [
        Role::Spool,
        Role::FirstHelper,
        Role::SecondHelper,
        Role::Sizing,
    ] {
        pipeline.register(role);
    }
    let calls = AtomicUsize::new(0);
    let mut budget = Budget::default();
    pipeline.reconcile(
        &mut budget,
        || 0,
        |_, _, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            true
        },
    );
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    for _ in 0..20 {
        pipeline.refresh();
    }
    assert!(!pipeline.pending.load(Ordering::SeqCst));
    pipeline.reconcile(&mut budget, || 0, |_, _, _| panic!("unchanged setter"));
    drop(callback);
    pipeline.reconcile(
        &mut budget,
        || 0,
        |_, _, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            true
        },
    );
    assert_eq!(calls.load(Ordering::SeqCst), 10);
}

#[test]
fn a_long_sizing_item_is_demoted_at_expiry_and_cooldown_starts_after_success() {
    let pipeline = fixture(true);
    let _callback = pipeline.callback();
    pipeline.register(Role::Sizing);
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
    crate::test_support::with_allocations_denied(|| {
        pipeline.refresh();
        let waiting = pipeline.wait_for_store();
        waiting.retry();
        drop(waiting);
        drop(callback);
        apply(&pipeline, &mut budget, 0, &native);
    });
    assert_eq!(native.load(Ordering::SeqCst), 0);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _callback = pipeline.callback();
        let _waiting = pipeline.wait_for_store();
        panic!("synthetic unwind");
    }));
    assert_eq!(pipeline.callbacks.load(Ordering::SeqCst), 0);
    assert_eq!(pipeline.store_waiters.load(Ordering::SeqCst), 0);
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
    let pipeline = Arc::new(Pipeline::new());
    let mut worker = Reconciler::start(Arc::clone(&pipeline)).unwrap();
    assert_ne!(worker.thread_id(), 0);
    let mut disabled = 0;
    // SAFETY: this handle belongs to the test's own reconciler and remains joined below.
    let valid = unsafe {
        windows_sys::Win32::System::Threading::GetThreadPriorityBoost(
            worker.thread.as_ref().unwrap().as_raw_handle() as _,
            &mut disabled,
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
    // A resumes after B's completed correction, publishing its stale pressure but never writing priority.
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
fn availability_publication_cannot_undo_a_racing_claim() {
    let pipeline = Pipeline::new();
    let _callback = pipeline.callback();
    pipeline.publish(0);
    assert_eq!(pipeline.free_count(), 1);
    assert_eq!(for_role(pipeline.pressure(), 0), 2);
    pipeline.publish(1);
    pipeline.claim(0);
    assert_eq!(pipeline.free_count(), 1);
    pipeline.claim(1);
    assert_eq!(pipeline.free_count(), 0);
    pipeline.publish(0);
    pipeline.claim(0);
    assert_eq!(pipeline.free_count(), 0);
    pipeline.publish(0);
    pipeline.publish(1);
    pipeline.publish(2);
    pipeline.publish(3);
    assert_eq!(pipeline.free_count(), 4);
    assert_eq!(pipeline.pressure(), 0);
}
