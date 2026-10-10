//! Map-definition parse record, distinct from a replay row.
//! Declared IDs and row positions differ on malformed or modified input.

/// Map-definition record stride.
pub const STRIDE: usize = 0x28;
/// Declared u16 province ID.
pub const ID: usize = 0;
/// Parsed red u8 component.
pub const RED: usize = 2;
/// Parsed green u8 component.
pub const GREEN: usize = 3;
/// Parsed blue u8 component.
pub const BLUE: usize = 4;
/// Terrain-class u8: land 2, sea 4, lake 8, other 1.
pub const TYPE: usize = 5;
/// Parsed continent narrowed to u8.
pub const CONTINENT: usize = 6;
/// Coastal u8 flag set only for a true token.
pub const COASTAL: usize = 7;
/// MSVC-string terrain token.
pub const TERRAIN: usize = 8;
/// Map-definition record parser entry.
pub const PARSE_RVA: usize = 0x0153_0d60;
