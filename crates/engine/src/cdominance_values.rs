//! CDominanceValues primary-pointer layout, inside a country-keyed array entry.
//!
//! The old constants were relative to the outer entry, eight bytes before
//! this object. Its numeric reader consumed only four bytes from eight-byte
//! spaced fields; native widths and signedness still require engine proof.

/// CDominanceValues primary vtable; naval_missions_status_live:65 and pinned RTTI displacement 0.
pub const VTABLE_RVA: usize = 0x0296_dfc0;
/// CDominanceValues country key, u32; old entry+10 checked equal to the outer key.
pub const COUNTRY: usize = 0x08;
/// CDominanceValues current dominance candidate; old entry+20 read as u32, native width unproved.
pub const CURRENT_CANDIDATE: usize = 0x18;
/// CDominanceValues base-target candidate; old entry+28 read as u32, native width unproved.
pub const BASE_TARGET_CANDIDATE: usize = 0x20;
/// CDominanceValues target candidate; old entry+30 read as u32, native width unproved.
pub const TARGET_CANDIDATE: usize = 0x28;
/// CDominanceValues individual-ratio candidate; old entry+38 read as u32, native width unproved.
pub const INDIVIDUAL_RATIO_CANDIDATE: usize = 0x30;
/// CDominanceValues previous-dominance candidate; old entry+40 read as u32, native width unproved.
pub const PREVIOUS_CANDIDATE: usize = 0x38;
/// CDominanceValues decline-from candidate; old entry+48 read as u32, native width unproved.
pub const DECLINE_FROM_CANDIDATE: usize = 0x40;
