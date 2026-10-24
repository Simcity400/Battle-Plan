//! Garrison equipment-pool headers. Required archetype pools use a different
//! data offset from variant pools; the headers are not interchangeable.
//! Required and allocation pools also have distinct entry layouts.

/// Required archetype-pool primary vtable.
pub const VTABLE_RVA: usize = 0x0278_9c30;

/// Garrison equipment-pool data pointer.
pub const DATA: usize = 0x08;
/// Signed i32 capacity; must cover the count.
pub const CAPACITY: usize = 0x10;
/// Garrison equipment-pool count, i32.
pub const COUNT: usize = 0x14;
