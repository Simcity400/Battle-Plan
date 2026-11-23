#[doc(hidden)]
pub mod checkpoint;
pub mod codec;
pub mod day_night;
pub mod display;
pub mod event_feed_rows;
pub mod fixed_point;
mod map_event_rows;
#[doc(hidden)]
pub mod measurement;
pub mod naval_trees;
mod nuclear_rows;
mod peace_rows;
pub mod rows;
#[doc(hidden)]
pub mod sliced {
    pub mod ranges;
}
pub mod temperature;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;
#[doc(hidden)]
pub mod value;

pub type Hour = i32;
pub type SessionId = u64;
pub type BranchId = u64;
pub const FORMAT_VERSION: u16 = rows::compatibility::ENGAGED_REFRESH_WITNESS_FORMAT;
