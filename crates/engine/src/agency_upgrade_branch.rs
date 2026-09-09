//! Agency-upgrade branch definition; the archived reader did not establish its RTTI name.
//!
//! Source: `extractor/native/lanes/diplomacy_intel_operatives_live/src/agency_upgrade_controller.rs`.
//! Its inherited authored identity uses `cgame_item`.

pub const UPGRADES: usize = 0x40;
/// Branch definition upgrade count, native u32; confirmed by observe_filtered_branch (source:20,144).
pub const UPGRADE_COUNT: usize = 0x4c;
/// Branch definition upgrade entries are native 64-bit pointers; confirmed by observe_filtered_branch (source:157).
pub const UPGRADE_STRIDE: usize = 8;
