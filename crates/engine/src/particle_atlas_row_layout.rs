//! Per-subsystem particle-atlas row; presentation layouts only.

/// Atlas row stride.
pub const STRIDE: usize = 0x14;
/// Texture runtime u32 index.
pub const TEXTURE_RUNTIME_INDEX: usize = 0x00;
/// Atlas half-pixel f32 width.
pub const HALF_PIXEL_WIDTH: usize = 0x04;
/// Atlas half-pixel f32 height.
pub const HALF_PIXEL_HEIGHT: usize = 0x08;
/// Atlas columns stored as f32.
pub const COLUMNS: usize = 0x0c;
/// Atlas rows stored as f32.
pub const ROWS: usize = 0x10;
