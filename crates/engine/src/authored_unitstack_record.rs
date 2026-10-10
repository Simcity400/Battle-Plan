//! Authored locator record; its concrete type is unknown.

/// Locator-record stride, covering five floats and a presence flag.
pub const STRIDE: usize = 0x18;
/// World position, f32[3].
pub const POSITION: usize = 0;
/// First additional f32; its downstream meaning is unknown.
pub const FLOAT_A: usize = 0x0c;
/// Second additional f32; its downstream meaning is unknown.
pub const FLOAT_B: usize = 0x10;
/// Authored-presence byte; exporters include only records with it set.
pub const PRESENT: usize = 0x14;
