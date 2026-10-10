//! Auxiliary entry in supply-system +0x28.

/// Entry stride.
pub const STRIDE: usize = 0x28;
/// Flag byte at entry +0x24; dirty clearing removes bit zero.
pub const FLAGS: usize = 0x24;
/// Dirty bit mask.
pub const DIRTY: u8 = 1;
