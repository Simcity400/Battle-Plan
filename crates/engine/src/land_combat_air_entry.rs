//! Land combat air-support entry, reached through CLandCombatant+120.

/// 1415C94A0 sums the signed native count at entry+14 for the CAS counter.
/// The array contains pointers; it does not contain CAirWing handles.
pub const PLANE_COUNT: usize = 0x14;
