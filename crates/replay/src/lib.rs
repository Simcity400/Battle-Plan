//! The shared spool and replay recording contract.

pub mod branches;
pub mod cadence;
pub mod codec;
pub mod container;
pub mod eta_derive;
pub mod event_feed_rows;
pub mod fixed_layout;
pub mod fixed_point;
pub mod format;
pub mod frame;
pub mod identity;
pub mod naval_mission_derive;
mod peace_rows;
pub mod rows;
pub mod sliced;
pub mod spool;
mod value;
pub mod witness;

pub type Hour = i32;
pub type SessionId = u64;
pub type BranchId = u64;
/// The recording version this build writes. Reader history lives beside TableId.
pub const FORMAT_VERSION: u16 = rows::compatibility::WIDGET_CAPTURE_FORMAT;
pub const KEYFRAME_INTERVAL: u32 = 720;

#[cfg(test)]
mod acceptance;
