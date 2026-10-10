//! MIO separator presentation row, excluded from hourly recording.

/// MIO separator row primary vtable.
pub const VTABLE_RVA: usize = 0x02a8_dbd0;
/// MIO separator row GUI-subobject vtable.
pub const GUI_VTABLE_RVA: usize = 0x02a8_dc40;
/// MIO separator row GUI subobject.
pub const GUI: usize = 0x38;
/// GUI position-method byte offset, returning two i32 coordinates.
pub const POSITION_VTABLE_SLOT: usize = 0x78;
/// GUI extent-method byte offset, returning two i32 dimensions.
pub const EXTENT_VTABLE_SLOT: usize = 0x28;
