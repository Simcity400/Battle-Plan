//! Frame encoding, reconstruction and branch continuity, without recording I/O.

pub mod branches;
pub mod cpu;
pub mod envelope;
pub mod frame;
mod fuel_demand;
pub mod rows;

pub(crate) use replay_data::value;
pub use replay_data::{
    BranchId, FORMAT_VERSION, Hour, SessionId, cadence, codec, country_flags, day_night, display,
    event_feed_rows, fixed_point, format, identity, naval_trees, scope, sliced, temperature,
};

pub const KEYFRAME_INTERVAL: u32 = 720;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;
