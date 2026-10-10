//! Graphics terrain selected by the city predicate, separate from simulation terrain.
//! The database contains a null graphics record.
//! The two flag meanings below remain unspecified.

/// CTerrainGraphics allocation size.
pub const SIZE: usize = 0x70;
/// Predicate byte; initialized to one and cleared in the null record.
pub const PREDICATE_FLAG_10: usize = 0x10;
/// Flag byte consumed by the city predicate; its semantic name is unspecified.
pub const CITY_PREDICATE_FLAG: usize = 0x69;
/// Graphics-terrain constructor.
pub const CONSTRUCT_RVA: usize = 0x0142_96a0;
