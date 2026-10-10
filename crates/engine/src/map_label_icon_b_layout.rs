//! Map-icon label and value-dependent colour presentation state.

/// Updates existing text and the value-dependent colour selector.
pub const UPDATE_LABEL_RVA: usize = 0x018b_5df0;
/// Instant-text-box pointer.
pub const TEXT: usize = 0x88;
/// Colour-target pointer used for selector changes.
pub const COLOUR_TARGET: usize = 0x90;
/// Subject pointer supplying value and controller fields.
pub const SUBJECT: usize = 0x98;
/// Cached string inline-data/heap-pointer union selected by capacity.
pub const CACHED_LABEL: usize = 0xa0;
/// Cached string u64 length.
pub const CACHED_LABEL_LENGTH: usize = 0xb0;
/// Cached string u64 capacity; values above fifteen select heap storage.
pub const CACHED_LABEL_CAPACITY: usize = 0xb8;
