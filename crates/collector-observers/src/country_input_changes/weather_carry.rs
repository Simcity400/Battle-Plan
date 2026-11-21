
use std::sync::atomic::AtomicU64;

pub static WINDOW_HOURS: AtomicU64 = AtomicU64::new(0);
pub static FULL_HOURS: AtomicU64 = AtomicU64::new(0);
pub static WINDOW_RECORDS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_WINDOW_HOURS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_FULL_HOURS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_REFUSALS: AtomicU64 = AtomicU64::new(0);
pub static CUSTOM_SEA_REFUSALS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_TICKS: AtomicU64 = AtomicU64::new(0);
pub static MODIFIER_LOOKUPS: AtomicU64 = AtomicU64::new(0);

