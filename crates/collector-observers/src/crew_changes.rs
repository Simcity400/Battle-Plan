//! Certificates for acquired air/naval crews. The native country-input hooks
//! own the refresh/damage writers; equipment destruction forwards aircraft
//! removal. A disabled or full registry refuses capture instead of polling.
use crate::watch_registry::{Notification, WatchRegistry};
use std::sync::atomic::{AtomicBool, Ordering};
static WATCHES: WatchRegistry = WatchRegistry::new();
static READS: [std::sync::atomic::AtomicU64; 3] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 3];
pub fn record(counts: [u64; 3]) {
    for (total, count) in READS.iter().zip(counts) {
        total.fetch_add(count, Ordering::Relaxed);
    }
}
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
pub fn sequence(object: usize) -> Option<(u64, u64)> {
    WATCHES.register(object).map(|(token, _)| token)
}
pub fn offer(demand: usize) {
    WATCHES.offer(demand.saturating_add(WATCHES.registered()));
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
pub fn status() -> impl serde::Serialize {
    #[derive(serde::Serialize)]
    struct Status<T> {
        certificate: T,
        scope: &'static str,
        read: u64,
        carried: u64,
        unavailable: u64,
    }
    Status {
        certificate: WATCHES.status(),
        scope: "new or writer-marked objects only; an unavailable certificate refuses crews without polling",
        read: READS[0].load(Ordering::Relaxed),
        carried: READS[1].load(Ordering::Relaxed),
        unavailable: READS[2].load(Ordering::Relaxed),
    }
}
/// Free the crew certificate tables.
///
/// # Safety
/// Every owning and forwarding hook must be removed and drained, and the
/// acquisition pool must be stopped so no notification or capture can run.
pub unsafe fn release() {
    unsafe { WATCHES.release() };
}
