//! State presentation holder for map icons, separate from simulation state.
//! Anchor lanes are f32 world x, terrain height and world z (bitmap y).

/// Constructor storing the simulation-state pointer and primary vtable.
pub const CONSTRUCT_RVA: usize = 0x0166_f570;
/// Presentation update checking owner identity and capital state.
pub const UPDATE_RVA: usize = 0x0166_faa0;
/// Simulation-state pointer from the second constructor argument.
pub const STATE: usize = 0x08;
/// Capital icon pointer created, cleared or retained by presentation updates.
pub const CAPITAL_ICON: usize = 0x28;
/// Anchor world x, f32.
pub const ANCHOR_X: usize = 0x70;
/// Anchor terrain height, f32.
pub const ANCHOR_HEIGHT: usize = 0x74;
/// Anchor world z / bitmap y, f32.
pub const ANCHOR_Z: usize = 0x78;
/// Update flag byte initialized to zero and set on update.
pub const UPDATED: usize = 0x7c;
/// Map mode suppressing the capital icon.
pub const CAPITAL_SUPPRESSED_MAP_MODE: u32 = 0x1b;
/// Loaded capital-icon camera-distance ceiling, f32.
pub const CAPITAL_DISTANCE_CEILING_RVA: usize = 0x0331_bdd0;
