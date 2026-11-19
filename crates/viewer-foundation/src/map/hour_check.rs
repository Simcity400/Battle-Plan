//! `BATTLE_PLAN_VIEWER_CHECK_HOUR`: every display frame shows one hour of the replay.
//!
//! A layer whose publication carries the hours' change sets installs the sets its frame's head has
//! reached ([`crate::map::simulation_rate`]). The judge takes a sampled frame and compares what
//! that layer has on screen with the layer's own state at that frame's head hour: a difference, a
//! set installed before its hour or one left behind after it fails by name.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static CHECKED: AtomicU64 = AtomicU64::new(0);
static DIFFERING: AtomicU64 = AtomicU64::new(0);

pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_HOUR").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_HOUR")).is_some_and(|value| !value.is_empty() && value != "0"))
}

/// One more frame's layer judged, whatever it found.
pub fn judged() {
    if on() {
        CHECKED.fetch_add(1, Relaxed);
    }
}

/// `part` shows something else at `hour` than it settled there.
pub fn found(part: &str, why: impl std::fmt::Display) {
    DIFFERING.fetch_add(1, Relaxed);
    eprintln!("BATTLE_PLAN_VIEWER_CHECK_HOUR: {part}: {why}");
}

/// Checks made and differences found, for the run's report.
pub fn counted() -> (u64, u64) {
    (CHECKED.load(Relaxed), DIFFERING.load(Relaxed))
}
