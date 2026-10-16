//! The shared spool and replay recording contract.

pub mod air_region_derive;
pub mod attack_derive;
pub mod branches;
pub mod container;
pub mod directed_war_derive;
pub mod eta_derive;
pub mod fixed_layout;
pub mod frame;
mod fuel_demand;
pub mod modifier_registry;
pub mod naval_mission_derive;
pub mod power_balance_derive;
#[cfg(windows)]
pub mod publication;
pub mod recorder_settings;
pub mod rows;
pub mod spool;
pub mod transport_subject_access;
pub mod witness;

pub(crate) use replay_data::value;
pub use replay_data::{BranchId, FORMAT_VERSION, Hour, SessionId};
pub use replay_data::{
    cadence, codec, country_flags, day_night, display, event_feed_rows, fixed_point, format,
    identity, naval_trees, sliced, temperature,
};

pub const KEYFRAME_INTERVAL: u32 = 720;

#[cfg(test)]
mod acceptance;
