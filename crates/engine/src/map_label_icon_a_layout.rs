//! Map-icon label and colour presentation state.

/// Updates existing text and colour receivers.
pub const UPDATE_LABEL_RVA: usize = 0x018b_2e80;
/// Instant-text-box pointer.
pub const TEXT: usize = 0x88;
/// Colour-target pointer used for selector changes.
pub const COLOUR_TARGET: usize = 0x90;
/// State subject pointer, supplying its name and controller.
pub const STATE: usize = 0x98;
/// Alternate-label owner reached through two indirections.
pub const ALTERNATE_LABEL_OWNER: usize = 0xa0;
/// Signed i32 selector choosing the owner branch when equal to one.
pub const LABEL_BRANCH: usize = 0xac;
/// Cached string inline-data/heap-pointer union selected by capacity.
pub const CACHED_LABEL: usize = 0xb8;
/// Cached string u64 length.
pub const CACHED_LABEL_LENGTH: usize = 0xc8;
/// Cached string u64 capacity; values above fifteen select heap storage.
pub const CACHED_LABEL_CAPACITY: usize = 0xd0;
