//! Navy-theatre manager reached through CCountry+0x160.
//!
//! The old selector authenticated its children, not this manager's class;
//! no engine class name or vtable is asserted for this layout.

/// Navy-theatre manager CNavyTheaterGroup pointer array; old offsets.rs:981 and theatre_selector::navy.
pub const THEATER_GROUPS: usize = 0x10;
/// Navy-theatre manager array capacity, u32; old offsets.rs:982 and theatre_selector::navy.
pub const THEATER_GROUP_CAPACITY: usize = 0x18;
/// Navy-theatre manager array count, u32; old offsets.rs:983 and theatre_selector::navy.
pub const THEATER_GROUP_COUNT: usize = 0x1c;
