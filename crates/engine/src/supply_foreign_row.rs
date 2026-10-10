//! Per-country row in the supply-system foreign relationship array.

/// Foreign relationship row stride.
pub const STRIDE: usize = 0x78;
/// Related-country u32-handle array data pointer.
pub const RELATED_COUNTRIES: usize = 0x58;
/// Related-country array capacity, u32.
pub const RELATED_COUNTRY_CAPACITY: usize = 0x60;
/// Related-country array count, u32.
pub const RELATED_COUNTRY_COUNT: usize = 0x64;
/// Dirty flag byte at supply-system +0x40 + canonical index * 0x78 +0x70.
pub const FLAGS: usize = 0x70;
/// Bit one, cleared by the first country dirty list.
pub const DIRTY_BIT1: u8 = 1 << 1;
/// Bit two, cleared by the second country dirty list.
pub const DIRTY_BIT2: u8 = 1 << 2;
/// Bit three, cleared by the third country dirty list.
pub const DIRTY_BIT3: u8 = 1 << 3;
