//! Ship-class database for sunk-ship records. Missing IDs return the
//! null object at array slot zero.

/// Database singleton pointer.
pub const SINGLETON_RVA: usize = 0x0331_6cc0;
/// Definition-pointer array with eight-byte elements.
pub const DEFINITIONS: usize = 0x40;
/// Definition count, i32; slot zero is required.
pub const DEFINITION_COUNT: usize = 0x4c;
/// Database singleton getter.
pub const GET_RVA: usize = 0x0022_f870;
/// ID lookup comparing definition +8 and falling back to array slot zero.
pub const FIND_RVA: usize = 0x00ab_5ee0;
