//! MSVC tree-node specialization for CCharacter's map<string, CAdvisor*>; offsets use the node base.

/// Advisor-map node extent.
pub const SIZE: usize = 0x50;
/// Advisor-map MSVC string key.
pub const KEY: usize = 0x20;
/// Advisor-map unsigned key hash.
pub const KEY_HASH: usize = 0x40;
/// Advisor-map CAdvisor* value.
pub const ADVISOR: usize = 0x48;
