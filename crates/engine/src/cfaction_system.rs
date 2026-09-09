//! CFactionSystem roster geometry used by the old D53 reader.

/// CFactionSystem faction pointer-array data; build_profile.rs:392 and D53 read_faction_rows.
pub const FACTIONS: usize = 0x20;
/// CFactionSystem live count, low u32 of the old packed read; build_profile.rs:393 and D53 read_faction_rows.
pub const FACTION_COUNT: usize = 0x28;
/// CFactionSystem capacity, high u32 of the old +0x28 packed read; D53 read_faction_rows:4665-4670.
pub const FACTION_CAPACITY: usize = 0x2c;
