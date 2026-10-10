//! Display-mode value record.

/// Record stride.
pub const STRIDE: usize = 0x18;
/// Pixel format, u32.
pub const FORMAT: usize = 0;
/// Width, i32 pixels; may be replaced by device caps.
pub const WIDTH: usize = 4;
/// Height, i32 pixels; may be replaced by device caps.
pub const HEIGHT: usize = 8;
/// Refresh rate, i32, from display frequency.
pub const REFRESH_RATE: usize = 0x0c;
/// Driver-data pointer to a copied DEVMODEW.
pub const DRIVER_DATA: usize = 0x10;
