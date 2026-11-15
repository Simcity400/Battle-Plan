//! The quiet token of acquired air/naval crews. The native country-input hooks
//! own the refresh/damage writers; equipment destruction forwards aircraft
//! removal. While either family is unobserved no crew read is certified.
use crate::watch_registry::{Notification, WatchRegistry};
use std::sync::atomic::{AtomicBool, Ordering};
static WATCHES: WatchRegistry = WatchRegistry::new();
static OWN: AtomicBool = AtomicBool::new(false);
static KILLS: AtomicBool = AtomicBool::new(false);
fn apply() {
    WATCHES.set_enabled(OWN.load(Ordering::SeqCst) && KILLS.load(Ordering::SeqCst));
}
pub fn enabled(value: bool) {
    OWN.store(value, Ordering::SeqCst);
    apply();
}
pub fn observe_kills(value: bool) {
    KILLS.store(value, Ordering::SeqCst);
    apply();
}
pub fn notify(object: usize) -> Notification<'static> {
    WATCHES.notify(object)
}
/// `(epoch, revision)` while every crew writer is observed and none runs.
pub fn quiet() -> Option<(u64, u64)> {
    WATCHES.quiet()
}
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}
pub fn collect() {
    WATCHES.collect();
}
pub fn active() -> usize {
    WATCHES.active()
}
/// Free the crew certificate tables.
///
/// # Safety
/// Every owning and forwarding hook must be removed and drained, and the
/// acquisition pool must be stopped so no notification or capture can run.
pub unsafe fn release() {
    unsafe { WATCHES.release() };
}
