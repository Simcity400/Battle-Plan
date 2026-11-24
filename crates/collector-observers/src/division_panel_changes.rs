use crate::watch_registry::{Notification, WatchRegistry};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
static OPERANDS: WatchRegistry = WatchRegistry::marking();
static VARIANTS: WatchRegistry = WatchRegistry::marking();
static LIFETIMES: WatchRegistry = WatchRegistry::marking();
static VARIANT_DEMAND: AtomicUsize = AtomicUsize::new(0);
static VARIANT_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static VARIANT_AVAILABILITY: AtomicU64 = AtomicU64::new(0);
static VARIANT_VALIDATE: AtomicUsize = AtomicUsize::new(0);
static STATES: WatchRegistry = WatchRegistry::new();
static WORLD_EPOCH: AtomicU64 = AtomicU64::new(1);
pub fn world_epoch() -> u64 {
    WORLD_EPOCH.load(Ordering::SeqCst)
}
static REFUSED: AtomicU64 = AtomicU64::new(0);
pub fn refused() {
    REFUSED.fetch_add(1, Ordering::Relaxed);
}
static DEMAND: AtomicUsize = AtomicUsize::new(0);
static STATE_DEMAND: AtomicUsize = AtomicUsize::new(0);
pub fn patched(enabled: bool) {
    OPERANDS.set_enabled(enabled);
}
pub fn variant_availability(stamp: u64, validate: fn(u64) -> bool) {
    VARIANT_VALIDATE.store(validate as *const () as usize, Ordering::SeqCst);
    VARIANT_AVAILABILITY.store(stamp, Ordering::SeqCst);
    VARIANTS.set_enabled(validate(stamp));
    LIFETIMES.set_enabled(validate(stamp));
}
pub fn variant_capability() -> Option<u64> {
    let stamp = VARIANT_AVAILABILITY.load(Ordering::SeqCst);
    variant_capability_is_current(stamp).then_some(stamp)
}
pub fn variant_capability_is_current(stamp: u64) -> bool {
    let validator = VARIANT_VALIDATE.load(Ordering::SeqCst);
    if stamp == 0 || validator == 0 || VARIANT_AVAILABILITY.load(Ordering::SeqCst) != stamp {
        return false;
    }
    let validate: fn(u64) -> bool = unsafe { std::mem::transmute(validator) };
    validate(stamp) && VARIANT_AVAILABILITY.load(Ordering::SeqCst) == stamp
}
pub fn variant_invalidation_before() {
    VARIANT_ACTIVE.fetch_add(1, Ordering::SeqCst);
}
pub fn variant_invalidation_after(unwound: bool) {
    if unwound {
        VARIANTS.poison();
    }
    VARIANTS.touch_every();
    VARIANT_ACTIVE.fetch_sub(1, Ordering::SeqCst);
}
pub fn variant_retiring(receiver: usize) {
    LIFETIMES.touch(receiver);
}
pub fn variant_before(_receiver: usize) {
    VARIANT_ACTIVE.fetch_add(1, Ordering::SeqCst);
}
pub fn variant_after(receiver: usize, unwound: bool) {
    if unwound {
        VARIANTS.poison();
        LIFETIMES.poison();
    }
    VARIANTS.touch(receiver);
    VARIANT_ACTIVE.fetch_sub(1, Ordering::SeqCst);
}
pub fn variant_sequence(receiver: usize) -> Option<(u64, u64)> {
    let stamp = variant_capability()?;
    if VARIANT_ACTIVE.load(Ordering::SeqCst) != 0 {
        return None;
    }
    LIFETIMES.sequence(receiver)?;
    let result = VARIANTS.sequence(receiver);
    (VARIANT_ACTIVE.load(Ordering::SeqCst) == 0 && variant_capability_is_current(stamp))
        .then_some(result)
        .flatten()
}
pub fn state_forwarder(enabled: bool) {
    STATES.set_enabled(enabled);
}
pub fn notify(receiver: usize) -> Notification<'static> {
    OPERANDS.notify(receiver)
}
pub fn state_notify(state: Option<usize>) -> Notification<'static> {
    match state {
        Some(state) => STATES.notify(state),
        None => STATES.notify_every(),
    }
}
pub fn sequence(receiver: usize) -> Option<(u64, u64)> {
    OPERANDS.sequence(receiver)
}
pub fn state_sequence(state: u32) -> Option<(u64, u64)> {
    STATES.sequence(state as usize)
}
pub fn sequence_at(receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
    OPERANDS.observed_sequence_at(receiver, slot)
}
pub fn state_sequence_at(state: u32, slot: usize) -> Option<((u64, u64), usize)> {
    STATES.observed_sequence_at(state as usize, slot)
}
pub fn register_at(receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
    OPERANDS.sequence_at(receiver, slot)
}
pub fn state_register_at(state: u32, slot: usize) -> Option<((u64, u64), usize)> {
    STATES.sequence_at(state as usize, slot)
}

pub fn observed_sequence(receiver: usize) -> Option<(u64, u64)> {
    OPERANDS.observed_sequence(receiver)
}
pub fn observed_lifetime_sequence(receiver: usize) -> Option<(u64, u64)> {
    LIFETIMES.observed_sequence(receiver)
}
pub fn observed_variant_stamp() -> Option<((u64, u64), u64)> {
    let stamp = variant_capability()?;
    if VARIANT_ACTIVE.load(Ordering::SeqCst) != 0 {
        return None;
    }
    let quiet = VARIANTS.quiet()?;
    (VARIANT_ACTIVE.load(Ordering::SeqCst) == 0 && variant_capability_is_current(stamp))
        .then_some((quiet, stamp))
}
pub fn observed_variant_sequence(receiver: usize) -> Option<(u64, u64)> {
    let stamp = variant_capability()?;
    if VARIANT_ACTIVE.load(Ordering::SeqCst) != 0 {
        return None;
    }
    LIFETIMES.observed_sequence(receiver)?;
    let result = VARIANTS.observed_sequence(receiver);
    (VARIANT_ACTIVE.load(Ordering::SeqCst) == 0 && variant_capability_is_current(stamp))
        .then_some(result).flatten()
}
pub fn take(visit: impl FnMut(usize)) -> Option<u64> {
    let stamp = variant_capability()?;
    if VARIANT_ACTIVE.load(Ordering::SeqCst) != 0 {
        return None;
    }
    let epoch = VARIANTS.take(visit);
    (VARIANT_ACTIVE.load(Ordering::SeqCst) == 0 && variant_capability_is_current(stamp))
        .then_some(epoch)
        .flatten()
}
pub fn take_operands(visit: impl FnMut(usize)) -> Option<u64> {
    OPERANDS.take(visit)
}
pub fn take_lifetimes(visit: impl FnMut(usize)) -> Option<u64> {
    let stamp = variant_capability()?;
    if VARIANT_ACTIVE.load(Ordering::SeqCst) != 0 {
        return None;
    }
    let epoch = LIFETIMES.take(visit);
    (VARIANT_ACTIVE.load(Ordering::SeqCst) == 0 && variant_capability_is_current(stamp))
        .then_some(epoch)
        .flatten()
}
pub fn demand(operands: usize, variants: usize, states: usize) {
    DEMAND.fetch_max(operands, Ordering::Relaxed);
    VARIANT_DEMAND.fetch_max(variants, Ordering::Relaxed);
    STATE_DEMAND.fetch_max(states, Ordering::Relaxed);
}
pub fn boundary(invalidate: bool) {
    if invalidate {
        WORLD_EPOCH.fetch_add(1, Ordering::SeqCst);
    }
    OPERANDS.boundary(invalidate);
    VARIANTS.boundary(invalidate);
    LIFETIMES.boundary(invalidate);
    STATES.boundary(invalidate);
}
pub fn presize() {
    OPERANDS.offer(DEMAND.load(Ordering::Relaxed));
    STATES.offer(STATE_DEMAND.load(Ordering::Relaxed));
    VARIANTS.offer(VARIANT_DEMAND.load(Ordering::Relaxed));
    LIFETIMES.offer(VARIANT_DEMAND.load(Ordering::Relaxed));
    VARIANTS.collect();
    LIFETIMES.collect();
    OPERANDS.collect();
    STATES.collect();
}
pub fn active() -> usize {
    OPERANDS
        .active()
        .saturating_add(STATES.active())
        .saturating_add(VARIANT_ACTIVE.load(Ordering::SeqCst))
}
/// # Safety
/// Both patch owners must be quiescent, and the acquisition pool, which reads these registries,
/// must have stopped.
pub unsafe fn release() {
    unsafe {
        OPERANDS.release();
        VARIANTS.release();
        LIFETIMES.release();
        STATES.release();
    }
}

