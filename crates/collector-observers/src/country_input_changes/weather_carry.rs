//! The weather carry's diagnostics. The window-following read
//! ([`crate::extended::weather::carry`]) counts its hours, records and
//! differences here, beside the weather writer count that explains a
//! difference ([`super::weather_writes`]).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

pub static WINDOW_HOURS: AtomicU64 = AtomicU64::new(0);
pub static FULL_HOURS: AtomicU64 = AtomicU64::new(0);
pub static WINDOW_RECORDS: AtomicU64 = AtomicU64::new(0);
pub static EXPLAINED: AtomicU64 = AtomicU64::new(0);
pub static POISONED: AtomicBool = AtomicBool::new(false);
pub static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "a carried region weather record differs from a fresh read",
    "a carried province weather record differs from a fresh read",
    "a carried region's combat modifier entries differ from a fresh read",
    "a carried province's weather modifier entries differ from a fresh read",
]);
/// Province weather modifier hours that followed the window and that read every
/// province; carried entries that differed from a fresh read; refused reads.
pub static MODIFIER_WINDOW_HOURS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_FULL_HOURS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_DIFFERENCES: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_REFUSALS: AtomicU64 = AtomicU64::new(0);
/// QPC ticks spent in the province modifier capture and in its carried-slot
/// comparison, and the modifier lookups made.
pub static MODIFIER_TICKS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_COMPARE_TICKS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_LOOKUPS: AtomicU64 = AtomicU64::new(0);

/// Whether a comparison reads every carried record (`weather_verify`).
pub const VERIFY: bool = cfg!(feature = "weather_verify");

/// The status of the window-following read.
pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "configured": cfg!(feature = "weather_changes"),
        "verify": VERIFY,
        "window_hours": WINDOW_HOURS.load(Relaxed),
        "full_hours": FULL_HOURS.load(Relaxed),
        "window_records": WINDOW_RECORDS.load(Relaxed),
        "explained_differences": EXPLAINED.load(Relaxed),
        "poisoned": POISONED.load(Relaxed),
        "sampled": SAMPLED.slice(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "modifier_window_hours": MODIFIER_WINDOW_HOURS.load(Relaxed),
        "modifier_full_hours": MODIFIER_FULL_HOURS.load(Relaxed),
        "modifier_differences": MODIFIER_DIFFERENCES.load(Relaxed),
        "modifier_refusals": MODIFIER_REFUSALS.load(Relaxed),
        "modifier_ticks": MODIFIER_TICKS.load(Relaxed),
        "modifier_compare_ticks": MODIFIER_COMPARE_TICKS.load(Relaxed),
        "modifier_lookups": MODIFIER_LOOKUPS.load(Relaxed),
    })
}
