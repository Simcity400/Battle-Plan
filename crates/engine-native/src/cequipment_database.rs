//! CEquipmentDatabase, the loaded equipment types, and its archetype list.
//!
//! The archetype list is what the logistics series are indexed by: every
//! `CLogisticsStatus` series record holds one value per archetype, at the
//! archetype's `CEquipmentType+0x538`, which is its position in this list.

/// CEquipmentDatabase singleton pointer global.
pub const SINGLETON_RVA: usize = 0x0331_6af0;
/// CEquipmentDatabase primary vtable at object offset zero.
pub const VTABLE_RVA: usize = 0x0292_19c0;
/// Equipment archetype pointer array: capacity +0x88, count +0x8c. Each
/// archetype's +0x538 is its position here, assigned on insertion and retained
/// on reload. Logistics records use this count and index.
pub const ARCHETYPES: usize = 0x80;
/// Archetype pointer stride.
pub const ARCHETYPE_STRIDE: usize = 8;
