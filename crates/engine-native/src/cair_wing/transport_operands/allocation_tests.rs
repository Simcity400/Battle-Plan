use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static FAIL_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static FAILURES: AtomicUsize = AtomicUsize::new(0);
static STOPPED: AtomicBool = AtomicBool::new(false);

fn before() -> bool {
    ATTEMPTS.fetch_add(1, Ordering::Relaxed) + 1 == FAIL_AT.load(Ordering::Relaxed)
}

fn failed() {
    FAILURES.fetch_add(1, Ordering::Relaxed);
    STOPPED.store(true, Ordering::Release);
}

fn stopped() -> bool {
    STOPPED.load(Ordering::Acquire)
}

struct Hooks;

impl Hooks {
    fn install() -> Self {
        assert!(!allocation::capture_active() && !allocation::capture_stopped());
        allocation::set_hooks(Some(before), Some(failed));
        allocation::set_stop_hook(Some(stopped));
        Self
    }

    fn arm(&self, ordinal: usize) {
        ATTEMPTS.store(0, Ordering::Relaxed);
        FAIL_AT.store(ordinal, Ordering::Relaxed);
        FAILURES.store(0, Ordering::Relaxed);
        STOPPED.store(false, Ordering::Release);
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        allocation::set_hooks(None, None);
        allocation::set_stop_hook(None);
    }
}

#[test]
#[ignore = "run alone with alloc-fault-injection: allocation hooks are process-global"]
fn terminal_constructor_and_partial_growth_keep_scratch_ownership() {
    let hooks = Hooks::install();
    hooks.arm(2);
    let fallback = Scratch::owned(2);
    assert!(allocation::capture_stopped());
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 2);
    assert!(FAILURES.load(Ordering::Relaxed) != 0);
    assert_eq!(fallback.allocated_bytes(), 0);
    assert!(!fallback.fits(2));

    hooks.arm(usize::MAX);
    let mut scratch = Scratch::with_capacity(2).unwrap();
    scratch.pools[0] = Slot {
        key: 41,
        value: Some(Pool {
            index: Some(7),
            transport: Some(true),
        }),
    };
    scratch.wings[0] = Slot {
        key: 43,
        value: Some(Some(45)),
    };
    scratch.handles[0] = Slot {
        key: 45,
        value: Some(Some(47)),
    };
    scratch.required_map_capacity = 3;
    scratch.failures = 9;
    let lengths = (
        scratch.pools.len(),
        scratch.wings.len(),
        scratch.handles.len(),
    );
    let capacities = (
        scratch.pools.capacity(),
        scratch.wings.capacity(),
        scratch.handles.capacity(),
    );
    let bytes = scratch.allocated_bytes();
    let target = capacities.0.max(capacities.1).max(capacities.2) * 2;
    let slots = slot_count(target).unwrap();

    hooks.arm(2);
    assert_eq!(
        scratch.grow(target).unwrap_err().kind(),
        io::ErrorKind::OutOfMemory
    );
    assert!(allocation::capture_stopped());
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 2);
    assert_eq!(scratch.pools.len(), slots);
    assert!(scratch.pools.capacity() >= slots);
    assert_eq!(
        (scratch.wings.len(), scratch.handles.len()),
        (lengths.1, lengths.2)
    );
    assert_eq!(
        (scratch.wings.capacity(), scratch.handles.capacity()),
        (capacities.1, capacities.2)
    );
    assert_eq!(scratch.pools[0].key, 41);
    assert_eq!(scratch.pools[0].value.unwrap().index, Some(7));
    assert_eq!(scratch.wings[0].value, Some(Some(45)));
    assert_eq!(scratch.handles[0].value, Some(Some(47)));
    assert_eq!((scratch.required_map_capacity, scratch.failures), (3, 9));
    let retained = scratch.allocated_bytes();
    assert!(retained > bytes);
    assert_eq!(
        retained,
        scratch.pools.capacity() * size_of::<Slot<Pool>>()
            + scratch.wings.capacity() * size_of::<Slot<Option<u64>>>()
            + scratch.handles.capacity() * size_of::<Slot<Option<bool>>>()
    );
    assert_eq!(
        scratch.grow(target).unwrap_err().kind(),
        io::ErrorKind::OutOfMemory
    );
    assert_eq!(
        ATTEMPTS.load(Ordering::Relaxed),
        2,
        "terminal growth cannot retry allocation"
    );
    assert_eq!(scratch.allocated_bytes(), retained);

    hooks.arm(usize::MAX);
    scratch.grow(target).unwrap();
    assert!(scratch.fits(target));
    assert_eq!((scratch.required_map_capacity, scratch.failures), (0, 0));
    assert!(scratch.pools.iter().all(|slot| slot.value.is_none()));
    assert!(scratch.wings.iter().all(|slot| slot.value.is_none()));
    assert!(scratch.handles.iter().all(|slot| slot.value.is_none()));
    drop(hooks);
    assert!(!allocation::capture_active() && !allocation::capture_stopped());
}
