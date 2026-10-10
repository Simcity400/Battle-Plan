//! MIO history presentation row, excluded from hourly recording.

/// MIO history row primary vtable.
pub const VTABLE_RVA: usize = 0x02a8_e230;
/// MIO history row GUI-subobject vtable.
pub const GUI_VTABLE_RVA: usize = 0x02a8_e2a0;
/// MIO history row GUI subobject.
pub const GUI: usize = 0x38;
/// GUI position-method byte offset, returning two i32 coordinates.
pub const POSITION_VTABLE_SLOT: usize = 0x78;
/// GUI extent-method byte offset, returning two i32 dimensions.
pub const EXTENT_VTABLE_SLOT: usize = 0x28;

/// MIO history-row third-subobject vtable.
pub const THIRD_VTABLE_RVA: usize = 0x02a8_e368;
/// MIO history-row third subobject.
pub const THIRD_INTERFACE: usize = 0x68;
/// MIO history-row organisation reference, two u32 words.
pub const ORGANISATION_REFERENCE: usize = 0x70;
/// MIO history-row signed i32 family discriminator.
pub const HISTORY_TYPE: usize = 0x78;
/// Variant-history family discriminator.
pub const VARIANT_TYPE: i32 = 1;
/// Production-history family discriminator.
pub const PRODUCTION_TYPE: i32 = 2;
/// MIO history-row item reference, two u32 words.
pub const ITEM_REFERENCE: usize = 0x7c;
