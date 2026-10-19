//! CEvent definition identity and presentation fields.
//!
//! Function sites describe operations consuming events. Common dispatch
//! receives a separate owner whose class remains unknown; its location here
//! does not establish that it is an event member function.

/// CEvent combined id, u32.
pub const COMBINED_ID: usize = 0x0c;
/// Packed native reference type and combined identifier words.
pub const REFERENCE: usize = 0x08;
/// Primary event-definition vtable.
pub const VTABLE_RVA: usize = 0x0298_2fb8;
/// Authored event identity, MSVC string; an empty name saves the numeric ID.
pub const TOKEN: usize = 0x20;
/// CEvent option pointer vector.
pub const OPTIONS: usize = 0x368;
/// CEvent option count, u32.
pub const OPTION_COUNT: usize = 0x374;
/// CEvent option vector element width.
pub const OPTION_STRIDE: usize = 8;
/// CEvent description candidate vector.
pub const DESCRIPTION_CANDIDATES: usize = 0x3d0;
/// CEvent fallback picture key, MSVC string.
pub const PICTURE_KEY: usize = 0x3e8;
/// CEvent timeout in days, u32.
pub const TIMEOUT_DAYS: usize = 0x408;
/// CEvent operative presentation flag, u8.
pub const OPERATIVE_CHROME: usize = 0x41d;
/// CEvent unit-leader presentation flag, u8.
pub const UNIT_LEADER_CHROME: usize = 0x41e;
/// CEvent fire-only-once flag, u8.
pub const FIRE_ONLY_ONCE: usize = 0x421;
/// CEvent news presentation flag, u8.
pub const NEWS_CHROME: usize = 0x424;
/// Common event-dispatch entry.
pub const FIRE_RVA: usize = 0x00a0_28a0;
/// Engine-global debug-assert flag used by event dispatch.
pub const DISPATCH_DEBUG_ASSERTS_FLAG_RVA: usize = 0x035c_8bf2;
/// Exact common event-dispatch entry bytes.
pub const FIRE_SENTINEL: [u8; 15] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18,
];
/// Event description-candidate owner entry.
pub const DESCRIPTION_CANDIDATE_OWNER_RVA: usize = 0x0116_b0d0;
/// Event description-text selection entry.
pub const SELECTED_DESCRIPTION_RVA: usize = 0x0116_c180;
/// Exact event-description selection entry bytes.
pub const SELECTED_DESCRIPTION_SENTINEL: [u8; 15] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54,
];
