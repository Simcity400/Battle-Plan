//! MSVC tree-node specialization for CCharacter's map<string, CAdvisor*>; offsets use the node base.

/// Advisor-map node extent; attach_log.rs:81930-81932 documents a 0x50-byte node and copies that extent.
pub const SIZE: usize = 0x50;
/// Advisor-map MSVC string key; attach_log.rs:81930 locates it before the role pointer.
pub const KEY: usize = 0x20;
pub const KEY_HASH: usize = 0x40;
/// Advisor-map CAdvisor* value; attach_log.rs:81931 and domain47_shadow_walk_map read this pointer.
pub const ADVISOR: usize = 0x48;
