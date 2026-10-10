//! Anonymous CBitmapFont textcolor-table entry. Interior bytes before the RGBA
//! payload have not been individually characterized.

/// Text-color entry stride.
pub const STRIDE: usize = 0x30;
/// Presence byte, checked before reading color.
pub const PRESENT: usize = 0;
/// Red component, f32.
pub const RED: usize = 0x20;
/// Green component, f32.
pub const GREEN: usize = 0x24;
/// Blue component, f32.
pub const BLUE: usize = 0x28;
/// Alpha component, f32.
pub const ALPHA: usize = 0x2c;
