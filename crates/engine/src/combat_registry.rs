//! In-place game-state registry shared by land and naval combats.
//! Entries point to complete combat objects. The land reader advances each
//! entry by `0x10` to its secondary base.

/// Registry pointer array.
pub const DATA: usize = 0x08;
/// Registry u32 entry count.
pub const COUNT: usize = 0x14;
/// Registry holds eight-byte complete-object pointers.
pub const ENTRY_STRIDE: usize = 8;
