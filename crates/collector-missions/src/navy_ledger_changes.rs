use std::sync::atomic::{AtomicU64, Ordering};

static NAVIES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static LEADERS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

pub fn record_leaders(counts: [u64; 2]) {
    for (total, value) in LEADERS.iter().zip(counts) {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Skip {
    pub enabled: bool,
}

impl Skip {
    #[cfg(any(test, feature = "test-support"))]
    pub const NEVER: Self = Self { enabled: false };

    pub fn current() -> Self {
        Self {
            enabled: cfg!(feature = "navy_ledger_changes"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub skipped: u64,
    pub read: u64,
    pub carried: u64,
}

pub fn record(counts: Counts) {
    for (total, value) in NAVIES
        .iter()
        .zip([counts.skipped, counts.read, counts.carried])
    {
        total.fetch_add(value, Ordering::Relaxed);
    }
}
