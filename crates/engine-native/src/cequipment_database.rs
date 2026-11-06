//! CEquipmentDatabase, the loaded equipment types, and its archetype list.
//!
//! The archetype list is what the logistics series are indexed by: every `CLogisticsStatus` series
//! record holds one value per archetype, at the archetype's `cequipment_type::STOCKPILE_INDEX`,
//! which is its position in this list.

/// CEquipmentDatabase singleton pointer global.
pub const SINGLETON_RVA: usize = 0x033d_d300;
/// CEquipmentDatabase primary vtable at object offset zero.
pub const VTABLE_RVA: usize = 0x029b_5f40;
/// Equipment archetype pointer array: capacity and count follow it. Each archetype's
/// `cequipment_type::STOCKPILE_INDEX` is its position here, assigned on insertion and retained on
/// reload. Logistics records use this count and index.
pub const ARCHETYPES: usize = 0x80;
/// Archetype pointer stride.
pub const ARCHETYPE_STRIDE: usize = 8;
