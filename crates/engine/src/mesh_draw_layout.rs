//! Mesh draw record selected through a mesh-owner draw group.
//! These presentation layouts do not enable a recorded table or hook.

/// Mesh draw row stride.
pub const STRIDE: usize = 0x108;
/// First draw effect key u32.
pub const EFFECT_KEY: usize = 0x24;
/// Draw skinning pointer selected through the draw record.
pub const SKINNING: usize = 0x40;
/// Draw variant u8.
pub const VARIANT: usize = 0x7d;
