
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static CHECKED: AtomicU64 = AtomicU64::new(0);
static DIFFERING: AtomicU64 = AtomicU64::new(0);

pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLEPLAN_VIEWER_CHECK_HOUR").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_HOUR")).is_some_and(|value| !value.is_empty() && value != "0"))
}

pub fn judged() {
    if on() {
        CHECKED.fetch_add(1, Relaxed);
    }
}

pub fn found(part: &str, why: impl std::fmt::Display) {
    DIFFERING.fetch_add(1, Relaxed);
    eprintln!("BATTLEPLAN_VIEWER_CHECK_HOUR: {part}: {why}");
}

pub fn counted() -> (u64, u64) {
    (CHECKED.load(Relaxed), DIFFERING.load(Relaxed))
}
