//! Embedded division acclimatization state; hourly consistency is unverified.

/// Embedded acclimatization object vtable.
pub const VTABLE_RVA: usize = 0x027c_7b30;
/// Category/weight vector data.
pub const DATA: usize = 0x08;
/// Vector capacity, u32.
pub const CAPACITY: usize = 0x10;
/// Vector count, u32.
pub const COUNT: usize = 0x14;
/// Maximum, i64 raw value.
pub const MAXIMUM: usize = 0x38;
