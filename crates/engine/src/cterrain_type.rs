//! CTerrainType inputs consumed by the land combat window.

/// Constructor 1414297F0 installs 1429AA2B0 (pinned a729 disk disassembly).
pub const VTABLE_RVA: usize = 0x029a_a2b0;
/// CTerrainDatabase constructor140AC696A installs the terrain null-object vtable.
pub const NULL_VTABLE_RVA: usize = 0x0292_e278;
/// Serializer1413CEE30 tests this byte before emitting combat's stored terrain.
pub const VALID: usize = 0x10;
/// Native string initialized by 1414297D0; terrain definition name.
pub const NAME: usize = 0x18;
/// One-based database index; 1414114D0 excludes the null slot and 141004CE0 indexes the stat adjuster array with index-1.
pub const DATABASE_INDEX: usize = 0x88;
/// 141298D50 consumes this fixed5 width through province+B8, definition+A8.
pub const COMBAT_WIDTH: usize = 0x68;
/// 141298D50 consumes this fixed5 additional direction width.
pub const ADDITIONAL_COMBAT_WIDTH: usize = 0x70;
