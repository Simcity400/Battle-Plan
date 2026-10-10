//! Named-overlap geometry interface. Its position method is distinct from
//! the GUI base's show method at the same byte offset. Receiver identity and
//! derived layout must be validated before invoking it.

/// Position-method byte offset, returning two i32 coordinates.
pub const POSITION_VSLOT_CANDIDATE: usize = 0x78;
/// Extent-method byte offset, returning two i32 dimensions.
pub const EXTENT_VSLOT_CANDIDATE: usize = 0x28;
