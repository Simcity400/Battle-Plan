//! Raid-definition database used by country-raid selection, distinct from the raid-system object.

/// Raid-definition database global pointer, required to be nonnull.
pub const SINGLETON_RVA: usize = 0x0331_6c30;
/// Raid-definition pointer-array data.
pub const DEFINITIONS: usize = 0xb0;
/// Raid-definition signed i32 count.
pub const DEFINITIONS_COUNT: usize = 0xbc;
/// Raid-type definition primary vtable.
pub const TYPE_VTABLE_RVA: usize = 0x0292_a6c8;
/// Raid type definition `+8`, its lexer token.
pub const TYPE_TOKEN: usize = 0x08;
