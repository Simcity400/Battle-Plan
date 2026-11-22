
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering::Relaxed},
};

const EVERY: u64 = 64;

static CHECKED: AtomicU64 = AtomicU64::new(0);
static DIFFERING: AtomicU64 = AtomicU64::new(0);

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLEPLAN_VIEWER_CHECK_INSTALL").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_INSTALL")).is_some_and(|value| !value.is_empty() && value != "0"))
}

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

pub fn found(product: &str, why: impl std::fmt::Display) {
    DIFFERING.fetch_add(1, Relaxed);
    eprintln!("BATTLEPLAN_VIEWER_CHECK_INSTALL: {product}: {why}");
}

pub fn counted() -> (u64, u64) {
    (CHECKED.load(Relaxed), DIFFERING.load(Relaxed))
}
