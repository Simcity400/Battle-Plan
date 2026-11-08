//! The weather carry's diagnostics. The window-following read
//! ([`crate::extended::weather::carry`]) counts its hours and records here,
//! beside the weather writer count ([`super::weather_writes`]).

use std::sync::atomic::AtomicU64;

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

