//! Playback calculations over recorded operands, independent of recording I/O.

pub mod air_mission_counts;
pub mod air_mission_sources;
pub mod air_region_derive;
pub mod aircraft_count;
pub mod attack_derive;
pub mod directed_war_derive;
pub mod eta_derive;
pub mod fixed_layout;
pub mod modifier_registry;
pub mod naval_dominance_derive;
pub mod naval_mission_derive;
pub mod naval_repairs;
pub mod power_balance_derive;
pub mod transport_subject_access;
pub mod visibility;

pub use replay_data::{Hour, codec, format, naval_trees, rows};
pub use replay_frame::frame;

pub mod directed_wars;
pub mod naval_math;
pub mod naval_mine_levels;
pub mod naval_region_control;
