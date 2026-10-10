//! CTerrainDatabase graphics lookup used by the city predicate.
//! Graphics records and simulation terrain types occupy different arrays.

/// CTerrainDatabase singleton pointer.
pub const SINGLETON_RVA: usize = 0x0331_6cd8;
/// CTerrainDatabase CTerrainGraphics pointer array.
pub const GRAPHICS: usize = 0x70;
/// Signed graphics count; an invalid index selects graphics slot zero.
pub const GRAPHICS_COUNT: usize = 0x7c;
/// CTerrainDatabase sampled-byte-to-graphics-index array of i32.
pub const GRAPHICS_INDEX: usize = 0x88;
/// CTerrainDatabase signed graphics-index count.
pub const GRAPHICS_INDEX_COUNT: usize = 0x94;
/// Terrain database constructor; initializes separate graphics and simulation null records.
pub const CONSTRUCT_RVA: usize = 0x00ac_6860;
