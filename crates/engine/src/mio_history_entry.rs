//! One slot of an organisation's history robin-hood table: slot header,
//! variant handle key and equipment-history value. Serialization includes the
//! date only when set and always includes the completed-unit count.

/// MIO history-entry stride.
pub const STRIDE: usize = 0x40;
/// Signed i8 probe distance. Zero marks an empty slot; a visited slot must
/// not hold the -1 end sentinel.
pub const DISTANCE: usize = 0x04;
/// Variant handle key, with u32 type and u32 ID.
pub const ITEM_REFERENCE: usize = 0x08;
/// Byte marking a set date; only set dates are serialized. Not recorded.
pub const ACTIVE: usize = 0x30;
/// Signed i32 completed units, incremented by production ticks on lines
/// assigned to the organisation.
pub const PRODUCTION_COUNT: usize = 0x38;
