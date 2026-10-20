//! Shared recorded rows, cadence, exact values and their codecs.

pub mod cadence;
mod checkpoint;
pub mod codec;
pub mod country_flags;
pub mod day_night;
pub mod display;
pub mod event_feed_rows;
pub mod fixed_point;
pub mod format;
pub mod identity;
pub mod intel_estimates;
mod map_event_rows;
pub mod military_aid;
pub mod naval_trees;
mod nuclear_rows;
mod peace_rows;
pub mod rows;
pub mod sliced;
pub mod temperature;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;
#[doc(hidden)]
pub mod value;
pub mod witness;

pub type Hour = i32;
pub type SessionId = u64;
pub type BranchId = u64;
/// The recording version this build writes. Reader history lives beside TableId.
pub const FORMAT_VERSION: u16 = 107;

pub mod country_stability;
pub mod war_participation;
