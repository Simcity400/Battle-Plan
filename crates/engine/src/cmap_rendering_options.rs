//! CMapRenderingOptions embedded in settings at +0x178.
//! Fields are stored bytes, not simulation state.

/// CMapRenderingOptions vtable.
pub const VTABLE_RVA: usize = 0x0270_e3a8;
/// Terrain enabled byte.
pub const TERRAIN: usize = 0x08;
/// Water enabled byte.
pub const WATER: usize = 0x09;
/// Borders enabled byte.
pub const BORDERS: usize = 0x0a;
/// Trees enabled byte.
pub const TREES: usize = 0x0b;
/// Rivers enabled byte.
pub const RIVERS: usize = 0x0c;
/// Post-quad enabled byte.
pub const POST_QUAD: usize = 0x0e;
/// Shadows enabled byte.
pub const SHADOWS: usize = 0x13;
/// Airplane diagnostic byte.
pub const AIRPLANE_DIAGNOSTICS: usize = 0x20;
