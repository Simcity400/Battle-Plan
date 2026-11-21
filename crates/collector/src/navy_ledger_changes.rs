use std::sync::atomic::{AtomicU64, Ordering};

static NAVIES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static LEADERS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

pub(crate) fn record_leaders(counts: [u64; 2]) {
    for (total, value) in LEADERS.iter().zip(counts) {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Skip {
    pub(crate) enabled: bool,
}

impl Skip {
    #[cfg(test)]
    pub(crate) const NEVER: Self = Self { enabled: false };

    pub(crate) fn current() -> Self {
        Self {
            enabled: cfg!(feature = "navy_ledger_changes"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) skipped: u64,
    pub(crate) read: u64,
    pub(crate) carried: u64,
}

pub(crate) fn record(counts: Counts) {
    for (total, value) in NAVIES
        .iter()
        .zip([counts.skipped, counts.read, counts.carried])
    {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

