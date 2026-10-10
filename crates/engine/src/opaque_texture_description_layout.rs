//! Description used for opaque render-target allocation; excluded from recorded tables.

/// Pixel-format u32 code.
pub const PIXEL_FORMAT: usize = 0x08;
/// Creation u32 flags; bit one selects a render target.
pub const FLAGS: usize = 0x0c;
/// Requested u32 width.
pub const WIDTH: usize = 0x10;
/// Requested u32 height.
pub const HEIGHT: usize = 0x14;
/// Created-wrapper pointer.
pub const WRAPPER: usize = 0x70;
