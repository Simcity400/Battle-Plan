//! Trait-tree presentation collection.

/// Item-pointer vector.
pub const ITEMS: usize = 0x28;
/// Item-vector capacity, u32; must cover the count.
pub const ITEM_CAPACITY: usize = 0x30;
/// Item-vector count, u32.
pub const ITEM_COUNT: usize = 0x34;
/// Pointer-vector element width.
pub const ITEM_STRIDE: usize = 8;
/// Item append operation; constructs each appended item.
pub const APPEND_ITEM_RVA: usize = 0x016b_16c0;
