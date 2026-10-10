//! Railway-map icon placement.

/// World placement averages endpoint positions and adds the loaded icon shift.
pub const WORLD_POSITION_RVA: usize = 0x018f_4fa0;
/// Railway-map-icon constructor, installing the primary vtable and
/// initializing its map-icon base.
pub const CONSTRUCT_RVA: usize = 0x018f_4b30;
