use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64};

static TEST: Mutex<()> = Mutex::new(());
static EVENTS: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicBool = AtomicBool::new(false);
fn event(value: u64) {
    EVENTS
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |old| {
            Some(old * 16 + value)
        })
        .unwrap();
}
fn enabled(value: Availability) {
    LIVE.store(value.is_current(), Ordering::SeqCst);
}
unsafe fn receive(writer: Writer, pointer: usize, phase: Phase) {
    assert_eq!(writer, Writer::Update);
    assert_eq!(pointer, 0x1234);
    event(match phase {
        Phase::Before => 1,
        Phase::After => 3,
        Phase::Unwound => 4,
    });
}
static RECEIVER: Forwarder = Forwarder {
    call: receive,
    enabled,
};

#[test]
fn reset_withdraws_without_waiting_and_registration_cannot_admit_a_later_move() {
    let _serial = TEST.lock().unwrap();
    EVENTS.store(0, Ordering::SeqCst);
    let forwarding: &'static Forwarding = Box::leak(Box::new(Forwarding::new(|| false)));
    forwarding.available(true);
    forwarding.register(Some(&RECEIVER));
    let stale = forwarding.admission.load(Ordering::SeqCst);
    let invocation = unsafe { forwarding.begin(Writer::Update, 0x1234) }.unwrap();
    {
        let _control = forwarding.state.lock().unwrap();
        // A control-thread quiescence wait has already stopped dispatch.
        forwarding
            .receiver
            .store(std::ptr::null_mut(), Ordering::SeqCst);
        collector_test_support::with_allocations_denied(|| forwarding.invalidate());
    }
    assert!(!LIVE.load(Ordering::SeqCst));
    assert_eq!(forwarding.active.load(Ordering::SeqCst), 1);
    unsafe { invocation.finish() };
    forwarding.register(Some(&RECEIVER));
    assert!(!LIVE.load(Ordering::SeqCst));
    assert!(unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.is_none());
    forwarding.available_at(stale, true);
    assert!(!LIVE.load(Ordering::SeqCst));
    assert!(unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.is_none());
    forwarding.available(true);
    assert!(LIVE.load(Ordering::SeqCst));
    forwarding.register(None);
}

#[test]
fn disabled_forwarding_has_no_notifications_or_allocation() {
    let _serial = TEST.lock().unwrap();
    EVENTS.store(0, Ordering::SeqCst);
    let forwarding: &'static Forwarding = Box::leak(Box::new(Forwarding::new(|| false)));
    forwarding.register(Some(&RECEIVER));
    assert!(!LIVE.load(Ordering::SeqCst));
    collector_test_support::with_allocations_denied(|| {
        assert!(unsafe { forwarding.begin(Writer::Update, 0x1234) }.is_none());
    });
    assert_eq!(forwarding.active.load(Ordering::SeqCst), 0);
    assert_eq!(EVENTS.load(Ordering::SeqCst), 0);
    forwarding.register(None);
}

#[test]
fn entry_return_and_unwind_surround_the_original_without_allocation() {
    let _serial = TEST.lock().unwrap();
    let forwarding: &'static Forwarding = Box::leak(Box::new(Forwarding::new(|| false)));
    forwarding.available(true);
    forwarding.register(Some(&RECEIVER));
    assert!(LIVE.load(Ordering::SeqCst));
    EVENTS.store(0, Ordering::SeqCst);
    let original = |args: [usize; 4]| {
        event(2);
        args[0] ^ args[1] ^ args[2] ^ args[3]
    };
    let result = collector_test_support::with_allocations_denied(|| {
        let invocation = unsafe { forwarding.begin(Writer::Update, 0x1234) }.unwrap();
        let result = original([1, 2, 4, 8]);
        unsafe { invocation.finish() };
        result
    });
    assert_eq!(result, 15);
    assert_eq!(EVENTS.load(Ordering::SeqCst), 0x123);
    EVENTS.store(0, Ordering::SeqCst);
    let unwound = std::panic::catch_unwind(|| {
        let _invocation = unsafe { forwarding.begin(Writer::Update, 0x1234) }.unwrap();
        event(2);
        panic!("synthetic original unwinds");
    });
    assert!(unwound.is_err());
    assert_eq!(EVENTS.load(Ordering::SeqCst), 0x124);
    assert_eq!(forwarding.active.load(Ordering::SeqCst), 0);
    forwarding.available(false);
    assert!(!LIVE.load(Ordering::SeqCst));
    forwarding.register(None);
}

#[test]
fn live_removal_waits_for_the_paired_return_and_allows_reregistration() {
    let _serial = TEST.lock().unwrap();
    let forwarding: &'static Forwarding = Box::leak(Box::new(Forwarding::new(|| false)));
    forwarding.available(true);
    forwarding.register(Some(&RECEIVER));
    EVENTS.store(0, Ordering::SeqCst);
    let carry = forwarding.capability(forwarding.admission.load(Ordering::SeqCst));
    assert!(carry.is_current());
    let removed = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let invocation = unsafe { forwarding.begin(Writer::Update, 0x1234) }.unwrap();
        let removal = &removed;
        let control = &forwarding;
        let removal_thread = scope.spawn(move || {
            control.register(None);
            removal.store(true, Ordering::SeqCst);
        });
        while !forwarding.receiver.load(Ordering::SeqCst).is_null() {
            std::thread::yield_now();
        }
        assert!(!removed.load(Ordering::SeqCst));
        assert!(
            !carry.is_current(),
            "dispatch withdrawal revokes existing carry immediately"
        );
        assert!(LIVE.load(Ordering::SeqCst));
        assert!(unsafe { forwarding.begin(Writer::Update, 0x1234) }.is_none());
        unsafe { invocation.finish() };
        removal_thread.join().unwrap();
        assert!(removed.load(Ordering::SeqCst));
    });
    assert!(!LIVE.load(Ordering::SeqCst));
    assert_eq!(EVENTS.load(Ordering::SeqCst), 0x13);
    forwarding.register(Some(&RECEIVER));
    assert!(LIVE.load(Ordering::SeqCst));
    assert!(
        !carry.is_current(),
        "reregistration must not revive an earlier baseline"
    );
    forwarding.register(None);
}

#[test]
fn forwarded_writers_match_the_owned_descriptors_and_variant_receivers() {
    use engine_native::cequipment_variant::mutations::{MUTATIONS, Receiver};
    let selected: Vec<_> = MUTATIONS
        .iter()
        .enumerate()
        .filter_map(|(site, descriptor)| {
            writer(site).map(|writer| {
                assert!(matches!(
                    descriptor.receiver,
                    Receiver::Variant
                        | Receiver::Removed
                        | Receiver::Published
                        | Receiver::CreateCommand
                        | Receiver::Command
                ));
                (writer, descriptor.name)
            })
        })
        .collect();
    assert_eq!(
        selected,
        [
            (Writer::RebuildStats, "RebuildStats"),
            (Writer::RefreshDerivedState, "RefreshDerivedState"),
            (Writer::MoveAssign, "MoveAssign"),
            (Writer::Publish, "Publish"),
            (Writer::Destroy, "Destroy"),
            (Writer::Update, "Update"),
        ]
    );
}

#[test]
fn a_publisher_paused_after_admission_cas_cannot_enable_after_reset() {
    let _serial = TEST.lock().unwrap();
    let forwarding: &'static Forwarding = Box::leak(Box::new(Forwarding::new(|| false)));
    forwarding.register(Some(&RECEIVER));
    let pause = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let pause = &pause;
        let publisher = scope.spawn(move || {
            forwarding.available_at_with(forwarding.admission.load(Ordering::SeqCst), true, || {
                pause.wait();
                pause.wait();
            })
        });
        pause.wait();
        let stale = forwarding.capability(forwarding.admission.load(Ordering::SeqCst));
        forwarding.invalidate();
        assert!(!LIVE.load(Ordering::SeqCst));
        assert!(!stale.is_current());
        assert!(unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.is_none());
        pause.wait();
        publisher.join().unwrap();
        assert!(!LIVE.load(Ordering::SeqCst));
        assert!(forwarding.receiver.load(Ordering::SeqCst).is_null());
    });
    forwarding.register(None);
}

static DELIVERY_PAUSE: std::sync::Barrier = std::sync::Barrier::new(2);
fn paused_enabled(value: Availability) {
    if value.word & 1 != 0 {
        DELIVERY_PAUSE.wait();
        DELIVERY_PAUSE.wait();
    }
    enabled(value);
}
static PAUSED_RECEIVER: Forwarder = Forwarder {
    call: receive,
    enabled: paused_enabled,
};
#[test]
fn reset_during_delivery_revokes_the_capability_before_the_receiver_can_certify() {
    let _serial = TEST.lock().unwrap();
    let forwarding: &'static Forwarding = Box::leak(Box::new(Forwarding::new(|| false)));
    forwarding.register(Some(&PAUSED_RECEIVER));
    std::thread::scope(|scope| {
        let publisher = scope.spawn(move || forwarding.available(true));
        DELIVERY_PAUSE.wait();
        let read = forwarding.capability(forwarding.admission.load(Ordering::SeqCst));
        assert!(read.is_current());
        collector_test_support::with_allocations_denied(|| forwarding.invalidate());
        assert!(unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.is_none());
        assert!(
            !read.is_current(),
            "a read spanning the missed move must refuse"
        );
        DELIVERY_PAUSE.wait();
        publisher.join().unwrap();
        assert!(!LIVE.load(Ordering::SeqCst));
        forwarding.register(Some(&RECEIVER));
        forwarding.available(true);
        assert!(LIVE.load(Ordering::SeqCst));
        assert!(
            !read.is_current(),
            "a later baseline cannot restore an earlier proof"
        );
    });
    forwarding.register(None);
}

// The shared hook owner counts every original, including a move that never
// reached Forwarding::begin while its dispatch gate was withdrawn.
static GAP_WRITERS: collector_certificates::watch_registry::WatchRegistry =
    collector_certificates::watch_registry::WatchRegistry::new();
static MOVE_EVENTS: AtomicU64 = AtomicU64::new(0);
static GAP_DRAIN_SEEN: AtomicBool = AtomicBool::new(false);
unsafe fn receive_move(writer: Writer, pointer: usize, phase: Phase) {
    assert_eq!(writer, Writer::MoveAssign);
    assert_eq!(pointer, 0x1234);
    MOVE_EVENTS.fetch_or(
        match phase {
            Phase::Before => 1,
            Phase::After => 2,
            Phase::Unwound => 4,
        },
        Ordering::SeqCst,
    );
}
static MOVE_RECEIVER: Forwarder = Forwarder {
    call: receive_move,
    enabled,
};
fn gap_writers_active() -> bool {
    let active = GAP_WRITERS.active() != 0;
    if active {
        GAP_DRAIN_SEEN.store(true, Ordering::SeqCst);
    }
    active
}

#[test]
fn rejected_move_in_registration_gap_drains_before_capability_publication() {
    let _serial = TEST.lock().unwrap();
    let forwarding: &'static Forwarding = Box::leak(Box::new(Forwarding::new(gap_writers_active)));
    forwarding.available(true);
    forwarding.register(Some(&MOVE_RECEIVER));
    let old = forwarding.capability(forwarding.admission.load(Ordering::SeqCst));
    let withdrawn = std::sync::Barrier::new(2);
    let moved = AtomicBool::new(false);
    let registered = AtomicBool::new(false);
    MOVE_EVENTS.store(0, Ordering::SeqCst);
    std::thread::scope(|scope| {
        let control = scope.spawn(|| {
            forwarding.register_with(Some(&MOVE_RECEIVER), || {
                withdrawn.wait();
                withdrawn.wait();
            });
            registered.store(true, Ordering::SeqCst);
        });
        withdrawn.wait();
        assert!(!old.is_current());
        // Pause the rejected move immediately before its original. Its owner
        // must retain this flight even though no forwarding invocation exists.
        let flight = GAP_WRITERS.notify(0);
        assert!(unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.is_none());
        assert_eq!(forwarding.active.load(Ordering::SeqCst), 0);
        GAP_DRAIN_SEEN.store(false, Ordering::SeqCst);
        withdrawn.wait();
        while !GAP_DRAIN_SEEN.load(Ordering::SeqCst) && !registered.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        let candidate = forwarding.capability(forwarding.admission.load(Ordering::SeqCst));
        assert!(!candidate.is_current());
        assert!(!registered.load(Ordering::SeqCst));
        assert!(!moved.load(Ordering::SeqCst));
        // A later move already sees prepared dispatch and notifies, even while
        // the rejected flight prevents the capability becoming current.
        collector_test_support::with_allocations_denied(|| {
            let later_flight = GAP_WRITERS.notify(0);
            let invocation = unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.unwrap();
            unsafe { invocation.finish() };
            drop(later_flight);
        });
        assert_eq!(MOVE_EVENTS.load(Ordering::SeqCst), 3);
        assert!(!candidate.is_current());
        // Resume the original, then exit its whole callback scope.
        moved.store(true, Ordering::SeqCst);
        drop(flight);
        control.join().unwrap();
        assert!(registered.load(Ordering::SeqCst));
        assert!(candidate.is_current());
        assert!(!old.is_current());
        assert!(moved.load(Ordering::SeqCst));
        // Entry after publication must notify this epoch as well.
        let _flight = GAP_WRITERS.notify(0);
        let invocation = unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.unwrap();
        unsafe { invocation.finish() };
    });
    forwarding.register(None);
}

#[test]
fn withdraw_then_register_waits_for_rejected_move_return_or_unwind() {
    let _serial = TEST.lock().unwrap();
    for unwind in [false, true] {
        let forwarding: &'static Forwarding =
            Box::leak(Box::new(Forwarding::new(gap_writers_active)));
        forwarding.available(true);
        forwarding.register(Some(&MOVE_RECEIVER));
        let old = forwarding.capability(forwarding.admission.load(Ordering::SeqCst));
        forwarding.invalidate();
        let rejected = std::sync::Barrier::new(2);
        let resume = std::sync::Barrier::new(2);
        let registered = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                let result = std::panic::catch_unwind(|| {
                    let _flight = GAP_WRITERS.notify(0);
                    assert!(unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.is_none());
                    rejected.wait();
                    resume.wait();
                    if unwind {
                        panic!("synthetic rejected move unwinds");
                    }
                });
                assert_eq!(result.is_err(), unwind);
            });
            rejected.wait();
            GAP_DRAIN_SEEN.store(false, Ordering::SeqCst);
            let registration = scope.spawn(|| {
                forwarding.register(Some(&MOVE_RECEIVER));
                forwarding.available(true);
                registered.store(true, Ordering::SeqCst);
            });
            // Wait for the actual census check, or detect a premature return.
            while !GAP_DRAIN_SEEN.load(Ordering::SeqCst) && !registered.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            let published_early = registered.load(Ordering::SeqCst);
            let old_current = old.is_current();
            let candidate_current = forwarding
                .capability(forwarding.admission.load(Ordering::SeqCst))
                .is_current();
            resume.wait();
            writer.join().unwrap();
            registration.join().unwrap();
            assert!(!published_early);
            assert!(!old_current);
            assert!(!candidate_current);
            assert!(registered.load(Ordering::SeqCst));
            assert!(
                forwarding
                    .capability(forwarding.admission.load(Ordering::SeqCst))
                    .is_current()
            );
            let _flight = GAP_WRITERS.notify(0);
            let invocation = unsafe { forwarding.begin(Writer::MoveAssign, 0x1234) }.unwrap();
            unsafe { invocation.finish() };
        });
        forwarding.register(None);
    }
}
