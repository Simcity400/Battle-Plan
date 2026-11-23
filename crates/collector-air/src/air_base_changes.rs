use engine_native_air::cair_base::DenseRefusal;
use std::sync::atomic::{AtomicU64, Ordering};

static BASES: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static REFUSALS: [AtomicU64; DenseRefusal::COUNT] =
    [const { AtomicU64::new(0) }; DenseRefusal::COUNT];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotRun {
    pub dense: bool,
}

impl SlotRun {
    #[cfg_attr(test, allow(dead_code))]
    pub fn current() -> Self {
        Self {
            dense: cfg!(feature = "air_base_changes"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotReport {
    pub dense: u64,
    pub full: u64,
    pub refusals: [u64; DenseRefusal::COUNT],
}

#[cfg_attr(test, allow(dead_code))]
pub fn record(report: &SlotReport) {
    for (total, value) in BASES.iter().zip([report.dense, report.full]) {
        total.fetch_add(value, Ordering::Relaxed);
    }
    for (total, value) in REFUSALS.iter().zip(report.refusals) {
        total.fetch_add(value, Ordering::Relaxed);
    }
}
