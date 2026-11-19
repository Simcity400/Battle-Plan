//! `BATTLE_PLAN_VIEWER_CHECK_INSTALL`: where an install trusts what its maker says changed, the frame also
//! prepares the product as a whole preparation would have left it and compares what the GPU would
//! hold, failing by name. Off, [`due`] answers false for the cost of an atomic read; on, it answers
//! true once every [`EVERY`] installs of that site, so a run pays the whole preparation at a
//! bounded rate.

use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering::Relaxed},
};

/// Installs of one site between two checks.
const EVERY: u64 = 64;

static CHECKED: AtomicU64 = AtomicU64::new(0);
static DIFFERING: AtomicU64 = AtomicU64::new(0);

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_INSTALL").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_INSTALL")).is_some_and(|value| !value.is_empty() && value != "0"))
}

/// Whether this install of the site counting its installs in `installs` is checked.
pub fn due(installs: &AtomicU64) -> bool {
    if !on() {
        return false;
    }
    let due = installs.fetch_add(1, Relaxed) % EVERY == 0;
    if due {
        CHECKED.fetch_add(1, Relaxed);
    }
    due
}

/// A check of `product` found `why`: which product, which range, which hour.
pub fn found(product: &str, why: impl std::fmt::Display) {
    DIFFERING.fetch_add(1, Relaxed);
    eprintln!("BATTLE_PLAN_VIEWER_CHECK_INSTALL: {product}: {why}");
}

/// Checks made and differences found, for the run's report.
pub fn counted() -> (u64, u64) {
    (CHECKED.load(Relaxed), DIFFERING.load(Relaxed))
}
