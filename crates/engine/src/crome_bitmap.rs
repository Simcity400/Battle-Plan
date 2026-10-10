//! Bitmap layout with signed i32 pixel strides.

/// Bitmap allocation size.
pub const SIZE: usize = 0x38;
/// Width, i32, used by bounds checks.
pub const WIDTH: usize = 0x8;
/// Height, i32, used by bounds checks.
pub const HEIGHT: usize = 0xc;
/// X-coordinate byte multiplier, i32.
pub const PIXEL_STRIDE: usize = 0x10;
/// Source bit depth, u32, admitting 1, 8, 16 or 24.
pub const BIT_DEPTH: usize = 0x14;
/// Row byte stride, i32: width times pixel stride with a special one-bit shift.
pub const ROW_STRIDE: usize = 0x1c;
/// 32-bit padded row read size, separate from row stride.
pub const PADDED_ROW_READ_SIZE: usize = 0x20;
/// Allocated pixel-data pointer.
pub const PIXELS: usize = 0x28;
/// Bitmap constructor installing identity before loading.
pub const CONSTRUCT_RVA: usize = 0x0153_3cc0;
/// Format admission and allocation filling depth and stride fields.
pub const READ_HEADER_RVA: usize = 0x0153_3d60;
/// Pixel reader retaining 24-bpp offsets and positive-height rules.
pub const READ_PIXELS_RVA: usize = 0x0153_3ee0;
