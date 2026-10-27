//! The weather carry's diagnostics. The window-following read
//! ([`crate::extended::weather::carry`]) counts its hours and records here,
//! beside the weather writer count ([`super::weather_writes`]).

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub static WINDOW_HOURS: AtomicU64 = AtomicU64::new(0);
pub static FULL_HOURS: AtomicU64 = AtomicU64::new(0);
pub static WINDOW_RECORDS: AtomicU64 = AtomicU64::new(0);
/// Province weather modifier hours that followed the window and that read every
/// province; refused reads.
pub static MODIFIER_WINDOW_HOURS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_FULL_HOURS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_REFUSALS: AtomicU64 = AtomicU64::new(0);
/// QPC ticks spent in the province modifier capture, and the modifier lookups made.
pub static MODIFIER_TICKS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_LOOKUPS: AtomicU64 = AtomicU64::new(0);

/// The status of the window-following read.
pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "configured": cfg!(feature = "weather_changes"),
        "window_hours": WINDOW_HOURS.load(Relaxed),
        "full_hours": FULL_HOURS.load(Relaxed),
        "window_records": WINDOW_RECORDS.load(Relaxed),
        "modifier_window_hours": MODIFIER_WINDOW_HOURS.load(Relaxed),
        "modifier_full_hours": MODIFIER_FULL_HOURS.load(Relaxed),
        "modifier_refusals": MODIFIER_REFUSALS.load(Relaxed),
        "modifier_ticks": MODIFIER_TICKS.load(Relaxed),
        "modifier_lookups": MODIFIER_LOOKUPS.load(Relaxed),
    })
}
