//! CColor components consumed by rendering.

/// Red, f32 read as bits.
pub const RED: usize = 0x10;
/// Green, f32 read as bits.
pub const GREEN: usize = 0x14;
/// Blue, f32 read as bits.
pub const BLUE: usize = 0x18;
/// Alpha, f32 read as bits.
pub const ALPHA: usize = 0x1c;

/// Color component constructor.
pub const CONSTRUCT_RVA: usize = 0x0223_6fa0;
