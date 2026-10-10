//! Outer entity render row shared in the ordinary and generic Pdxmesh caster walks.
//!
//! Presentation layout only; this module enables no recorded table or hook.

/// Outer row stride.
pub const STRIDE: usize = 0x50;
/// Raw 32-bit group index; signedness is unspecified.
pub const GROUP: usize = 0x00;
/// Entity pointer.
pub const ENTITY: usize = 0x08;
/// World matrix, 16 raw f32 words.
pub const WORLD_TRANSFORM: usize = 0x10;
