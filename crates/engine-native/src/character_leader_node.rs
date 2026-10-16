//! MSVC tree-node specialization for CCharacter's map<int, CCountryLeader*>; offsets use the node base.

/// Leader-map node extent.
pub const SIZE: usize = 0x30;
/// Leader-map ideology key i32.
pub const IDEOLOGY: usize = 0x20;
/// Leader-map CCountryLeader* value.
pub const LEADER: usize = 0x28;
