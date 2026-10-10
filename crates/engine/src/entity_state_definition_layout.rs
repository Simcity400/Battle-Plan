//! Entity-state presentation definition, separate from an entity definition.
//! No capture is enabled.

/// Authored blend time, f32; absence preserves the constructor's positive zero.
pub const BLEND_TIME: usize = 0x208;
/// Constructor's positive-zero blend bits.
pub const DEFAULT_BLEND_TIME_BITS: u32 = 0x0000_0000;
