//! Map-building entity-table owner, separate from per-placement
//! `building_graphic_layout` records.

/// Loads ordinary and destroyed building-entity tables.
pub const LOAD_ENTITY_TABLES_RVA: usize = 0x0166_3170;
/// Destroyed-entity table, falling back to the ordinary entity for a null entry.
pub const DESTROYED_ENTITIES: usize = 0x30;
/// Entity-name prefix bytes read from the installed executable.
pub const ENTITY_PREFIX_RVA: usize = 0x029d_1d28;
/// Length of the authored-name prefix preceding the building token.
pub const ENTITY_PREFIX: &str = "building_";
/// Separator bytes preceding each one-based mesh variant number.
pub const ENTITY_VARIANT_SEPARATOR_RVA: usize = 0x0270_bd64;
/// Length of the mesh variant separator.
pub const ENTITY_VARIANT_SEPARATOR: &str = "_";
