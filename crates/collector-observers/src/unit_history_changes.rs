use crate::watch_registry::{Notification, WatchRegistry};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
static WATCHES: WatchRegistry = WatchRegistry::marking();
static EDITS: WatchRegistry = WatchRegistry::marking();
static ENTRY_DEMAND: AtomicUsize = AtomicUsize::new(0);
static PATCHED: AtomicBool = AtomicBool::new(false);
static FORWARDED: AtomicBool = AtomicBool::new(false);
static DEMAND: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicU64 = AtomicU64::new(0);
static READS: AtomicU64 = AtomicU64::new(0);
static CARRIED: AtomicU64 = AtomicU64::new(0);
fn enabled() {
    let enabled = PATCHED.load(Ordering::Acquire) && FORWARDED.load(Ordering::Acquire);
    WATCHES.set_enabled(enabled);
    EDITS.set_enabled(enabled);
}
pub fn patched(value: bool) {
    PATCHED.store(value, Ordering::Release);
    enabled();
}
pub fn forwarder(value: bool) {
    FORWARDED.store(value, Ordering::Release);
    enabled();
}
pub fn notify(receiver: usize) -> Notification<'static> {
    WATCHES.notify(receiver)
}
pub fn notify_edit(receiver: usize) -> Notification<'static> {
    EDITS.notify(receiver)
}
pub fn edit_sequence(receiver: usize) -> Option<(u64, u64)> {
    EDITS.sequence(receiver)
}
pub fn take_edits(visit: impl FnMut(usize)) -> Option<u64> {
    EDITS.take(visit)
}
pub fn take(visit: impl FnMut(usize)) -> Option<u64> {
    WATCHES.take(visit)
}
pub fn entry_demand(entries: usize) {
    ENTRY_DEMAND.fetch_max(entries, Ordering::Relaxed);
}
pub fn sequence(receiver: usize) -> Option<(u64, u64)> {
    WATCHES.sequence(receiver)
}
pub fn sequence_at(receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
    WATCHES.observed_sequence_at(receiver, slot)
}
pub fn range_sequence(receiver: usize) -> Option<(u64, u64)> {
    WATCHES.range_sequence(receiver)
}
pub fn registry() -> &'static WatchRegistry {
    &WATCHES
}
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
    EDITS.boundary(invalidate);
}
pub fn active() -> usize {
    WATCHES.active().saturating_add(EDITS.active())
}
pub fn presize() {
    WATCHES.offer(DEMAND.load(Ordering::Relaxed));
    WATCHES.collect();
    EDITS.offer(
        DEMAND
            .load(Ordering::Relaxed)
            .saturating_add(ENTRY_DEMAND.load(Ordering::Relaxed)),
    );
    EDITS.collect();
}
pub fn demand(divisions: usize) {
    DEMAND.fetch_max(divisions.saturating_mul(4), Ordering::Relaxed);
}
pub fn read() {
    READS.fetch_add(1, Ordering::Relaxed);
}
pub fn carried() {
    CARRIED.fetch_add(1, Ordering::Relaxed);
}
pub fn refused() {
    REFUSED.fetch_add(1, Ordering::Relaxed);
}
/// # Safety
/// All owning and forwarding hooks must be disabled and quiescent, and the acquisition pool, which
/// reads these registries, must have stopped.
pub unsafe fn release() {
    unsafe {
        WATCHES.release();
        EDITS.release();
    }
}
