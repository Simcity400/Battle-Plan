//! One native province-definition pixel run, read by the centroid builder.

/// Pixel-span stride for three u16 fields.
pub const STRIDE: usize = 6;
/// Pixel-span u16 starting x.
pub const X: usize = 0x00;
/// Pixel-span u16 bitmap y.
pub const Y: usize = 0x02;
/// Pixel-span u16 run length.
pub const LENGTH: usize = 0x04;
