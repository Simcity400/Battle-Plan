//! MIO equipment-type header presentation row, excluded from hourly recording.

/// MIO equipment-type header row primary vtable.
pub const VTABLE_RVA: usize = 0x02a8_e0e0;
/// MIO equipment-type header row GUI-subobject vtable.
pub const GUI_VTABLE_RVA: usize = 0x02a8_e150;
/// MIO equipment-type header row GUI subobject.
pub const GUI: usize = 0x38;
/// GUI position-method byte offset, returning two i32 coordinates.
pub const POSITION_VTABLE_SLOT: usize = 0x78;
/// GUI extent-method byte offset, returning two i32 dimensions.
pub const EXTENT_VTABLE_SLOT: usize = 0x28;
