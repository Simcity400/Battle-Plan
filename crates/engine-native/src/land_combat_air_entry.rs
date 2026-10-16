//! Land combat air-support entry, reached through the land combatant's air support (`cland_combatant::AIR_SUPPORT`).

/// Signed air-support count used by the combat counter.
/// The array contains pointers; it does not contain CAirWing handles.
pub const PLANE_COUNT: usize = 0x14;
