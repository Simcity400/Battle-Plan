//! Air bases whose country-container vector needs no full scan. Every
//! `cair_base::COUNTRY_CONTAINERS` vector has a slot per country (air base construction sizes it to
//! the game state's country count), almost all null; its only run-time writers, create country
//! container and remove country access, keep the dense list (`cair_base::REGISTERED_CONTAINERS`) in
//! step in the same call. AirBaseRegistrations therefore reads, per base, the dense list and each
//! listed container's identity, span, link entry and slot word ([`engine_native::cair_base::dense_plan`]),
//! and no other slot: the listed containers are the occupied slots, in slot order. A list that does
//! not prove the slots leaves the base to the full scan. Containers, pools, wings, capacity and
//! fuel are still read every hour. No hook is involved.
use engine_native::cair_base::DenseRefusal;
use std::sync::atomic::{AtomicU64, Ordering};

/// Fast-path and full-scan bases.
static BASES: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// Bases left to the full scan, by [`DenseRefusal`].
static REFUSALS: [AtomicU64; DenseRefusal::COUNT] =
    [const { AtomicU64::new(0) }; DenseRefusal::COUNT];

/// How one hour's registration acquisition finds each base's occupied slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlotRun {
    /// A base whose dense list proves its slots reads only their slot words.
    pub(crate) dense: bool,
}

impl SlotRun {
    /// The production job's run (tests set their own).
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn current() -> Self {
        Self {
            dense: cfg!(feature = "air_base_changes"),
        }
    }
}

/// One hour's registration acquisition of its bases' slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlotReport {
    /// Bases that read only their listed containers' slot words.
    pub(crate) dense: u64,
    /// Bases that read every chunk.
    pub(crate) full: u64,
    pub(crate) refusals: [u64; DenseRefusal::COUNT],
}

/// Off the game thread, after the hour's acquisition.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn record(report: &SlotReport) {
    for (total, value) in BASES.iter().zip([report.dense, report.full]) {
        total.fetch_add(value, Ordering::Relaxed);
    }
    for (total, value) in REFUSALS.iter().zip(report.refusals) {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

