//! Agency-upgrade branch database; the archived reader did not establish its RTTI name.
//!
//! Source: `extractor/native/lanes/diplomacy_intel_operatives_live/src/agency_upgrade_controller.rs`.
//! The database is definition state; its old UI-dependent acquisition is excluded.

pub const SINGLETON_RVA: usize = 0x0331_69f0;
/// Branch database pointer table; confirmed by the old begin reader (source:17,95).
pub const ITEMS: usize = 0x70;
/// Branch database table count, native u32 including reserved index zero; confirmed by the old begin reader (source:18,98).
pub const COUNT: usize = 0x7c;
/// Branch database entries are native 64-bit pointers; confirmed by observe_filtered_branch index arithmetic (source:130).
pub const ITEM_STRIDE: usize = 8;
