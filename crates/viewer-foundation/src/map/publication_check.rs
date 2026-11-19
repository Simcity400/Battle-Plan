//! `BATTLE_PLAN_VIEWER_CHECK_PUBLICATION`: a publication is one moment on screen.
//!
//! Every display frame, each drawn layer says what it shows. The judge fails by name on a product
//! the frame left for a later one, on a picture that went backwards, and on a layer standing
//! further behind the publication's hour than [`crate::map::simulation_rate`]'s bound allows.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static CHECKED: AtomicU64 = AtomicU64::new(0);
static DIFFERING: AtomicU64 = AtomicU64::new(0);

pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_PUBLICATION").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_PUBLICATION")).is_some_and(|value| !value.is_empty() && value != "0"))
}

/// One more thing judged, whatever it found.
pub fn judged() {
    if on() {
        CHECKED.fetch_add(1, Relaxed);
    }
}

/// `part`'s picture went backwards: it showed `shown` after having shown `before`.
pub fn backwards(part: &'static str, before: impl std::fmt::Display, shown: impl std::fmt::Display) {
    if on() {
        found(part, format!("showed {shown} after {before}"));
    }
}

pub fn found(part: &str, why: impl std::fmt::Display) {
    DIFFERING.fetch_add(1, Relaxed);
    eprintln!("BATTLE_PLAN_VIEWER_CHECK_PUBLICATION: {part}: {why}");
}

/// Checks made and differences found, for the run's report.
pub fn counted() -> (u64, u64) {
    (CHECKED.load(Relaxed), DIFFERING.load(Relaxed))
}
