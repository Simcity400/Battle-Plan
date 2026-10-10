//! Carrier-airbase host handle payload consumed by the oil-capacity walk.
//!
//! The payload is expected to be a CShip, but its class and primary or secondary
//! base adjustment are unverified. Offsets remain relative to the resolved
//! payload until its identity is established.

/// Carrier host equipment-entry array.
pub const EQUIPMENT_ENTRIES: usize = 0x60;
/// Equipment-entry count, i32.
pub const EQUIPMENT_ENTRY_COUNT: usize = 0x6c;
/// Capacity classifier byte.
pub const CAPACITY_CLASSIFIER: usize = 0x628;
/// Classifier bit selecting capacity class three.
pub const CAPACITY_CLASSIFIER_MASK: u8 = 0x80;
/// Cached module-property bool byte.
pub const MODULE_PROPERTY_CACHE: usize = 0x801;
/// Cache-valid bool byte, tested before the cached property.
pub const MODULE_PROPERTY_CACHE_VALID: usize = 0x802;
