//! Presentation settings containing embedded map-rendering options.
//! This module supplies no hourly state walker.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0270_e448;
/// Settings constructor installing settings and embedded map-options identities.
pub const CONSTRUCTOR_RVA: usize = 0x001f_9640;
/// Setting dispatcher parsing force-power-of-two and applying its global setter.
pub const READ_TOKEN_RVA: usize = 0x001f_abf0;
/// Settings singleton pointer used by counter-colour presentation.
pub const SINGLETON_RVA: usize = 0x0331_7038;
/// Counter colour mode, i32, clamped to 0..1: owner comparison at zero and
/// allegiance categories otherwise.
pub const COUNTER_COLOR_MODE: usize = 0x2a8;
/// Counter-colour-mode setting key identifier.
pub const COUNTER_COLOR_MODE_TOKEN: u32 = 0x3486;
/// Embedded map-rendering options.
pub const MAP_RENDERING_OPTIONS: usize = 0x178;
/// Map-options accessor returning the embedded object.
pub const MAP_RENDERING_OPTIONS_GETTER_RVA: usize = 0x001f_aa50;
/// Force-power-of-two texture boolean byte.
pub const FORCE_POW2_TEXTURES: usize = 0x250;
/// Explicit force-power-of-two assignment byte, set after boolean parsing.
pub const FORCE_POW2_TEXTURES_ASSIGNED: usize = 0x251;
/// Force-power-of-two texture key ID.
pub const FORCE_POW2_TEXTURES_TOKEN: u32 = 11029;
