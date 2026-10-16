//! CDominanceValues primary-pointer layout, inside a country-keyed array entry.
//!
//! The six saved scalars are signed i64 fixed5 values. The daily strategic-region
//! update publishes them into CNavalRegionDominance's sorted snapshot.

/// CDominanceValues primary vtable.
pub const VTABLE_RVA: usize = 0x0296_dfc0;
/// Dominance country u32 key, required to equal the outer array key.
pub const COUNTRY: usize = 0x08;
/// CDominanceValues owning CStrategicRegion pointer.
pub const REGION: usize = 0x10;
/// CDominanceValues current, i64 fixed5.
pub const CURRENT: usize = 0x18;
/// CDominanceValues base_target, i64 fixed5.
pub const BASE_TARGET: usize = 0x20;
/// CDominanceValues target, i64 fixed5.
pub const TARGET: usize = 0x28;
/// CDominanceValues individual_ratio, i64 fixed5.
pub const INDIVIDUAL_RATIO: usize = 0x30;
/// CDominanceValues previous, i64 fixed5.
pub const PREVIOUS: usize = 0x38;
/// CDominanceValues decline_from, i64 fixed5.
pub const DECLINE_FROM: usize = 0x40;
