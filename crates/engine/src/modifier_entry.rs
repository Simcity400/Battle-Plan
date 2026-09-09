//! Inline key/value row in the sorted country modifier accumulator; exact C++ type unproved.

/// Modifier accumulator row stride; old build_profile.rs:100 and compact_contract/tactic_weight.rs:26.
pub const STRIDE: usize = 0x10;
/// Modifier entry definition key u32; old build_profile.rs:101 and D57 binary search.
pub const KEY: usize = 0x00;
/// Modifier entry raw signed i64 fixed5 value; old build_profile.rs:102, D74 and tactic-weight readers.
pub const VALUE: usize = 0x08;
