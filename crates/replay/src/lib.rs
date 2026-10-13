//! The shared spool and replay recording contract.

pub mod air_region_derive;
pub mod attack_derive;
pub mod branches;
pub mod cadence;
pub mod codec;
pub mod container;
pub mod country_flags;
pub mod day_night;
pub mod directed_war_derive;
pub mod display;
pub mod eta_derive;
pub mod event_feed_rows;
pub mod fixed_layout;
pub mod fixed_point;
pub mod format;
pub mod frame;
pub mod identity;
pub mod modifier_registry;
pub mod naval_mission_derive;
pub mod naval_trees;
mod nuclear_rows;
mod peace_rows;
pub mod power_balance_derive;
#[cfg(windows)]
pub mod publication;
pub mod recorder_settings;
pub mod rows;
pub mod sliced;
pub mod spool;
pub mod temperature;
pub mod transport_subject_access;
mod value;
pub mod witness;

pub type Hour = i32;
pub type SessionId = u64;
pub type BranchId = u64;
/// The recording version this build writes. Reader history lives beside TableId.
pub const FORMAT_VERSION: u16 = 102;
pub const KEYFRAME_INTERVAL: u32 = 720;

#[cfg(test)]
mod acceptance;
