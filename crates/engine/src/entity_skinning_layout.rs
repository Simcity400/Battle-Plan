//! Skinning owner reached from a mesh entity at +0x70.
//!
//! Presentation layout only; this module enables no recorded table or hook.

/// Skinning vector-count pointer, dereferenced as u32.
pub const VECTOR_COUNT_POINTER: usize = 0x08;
/// Skinning palette byte pointer.
pub const PALETTE_DATA: usize = 0x18;
/// Palette vector width in bytes; multiply by the live vector count.
pub const VECTOR_STRIDE: usize = 0x10;
