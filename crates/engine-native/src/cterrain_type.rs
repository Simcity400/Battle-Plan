//! CTerrainType inputs consumed by the land combat window.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x029a_a2b0;
/// Terrain null-object vtable.
pub const NULL_VTABLE_RVA: usize = 0x0292_e278;
/// Serialization gate for the combat's stored terrain.
pub const VALID: usize = 0x10;
/// Native definition-name string.
pub const NAME: usize = 0x18;
/// One-based database index; excludes the null slot and selects stat adjusters at index minus one.
pub const DATABASE_INDEX: usize = 0x88;
/// Fixed5 combat width.
pub const COMBAT_WIDTH: usize = 0x68;
