//! MSVC tree-node specialization for CCharacter's map<int, CCountryLeader*>; offsets use the node base.

/// Leader-map node extent; attach_log.rs:81927-81929 documents a 0x30-byte node and copies that extent.
pub const SIZE: usize = 0x30;
/// Leader-map ideology key i32; attach_log.rs:81927 identifies the integer before the role pointer.
pub const IDEOLOGY: usize = 0x20;
/// Leader-map CCountryLeader* value; attach_log.rs:81928 and domain47_shadow_walk_map read this pointer.
pub const LEADER: usize = 0x28;
