//! Ordinary sprite presentation vertex geometry.

/// Packed vertex stride.
pub const STRIDE: usize = 20;
/// Position X, f32.
pub const X: usize = 0;
/// Position Y, f32.
pub const Y: usize = 4;
/// Position Z, f32.
pub const Z: usize = 8;
/// Texture U, f32, after the three position floats.
pub const U: usize = 12;
/// Texture V, f32.
pub const V: usize = 16;
/// Strip vertex count.
pub const VERTICES_PER_STRIP: usize = 4;
