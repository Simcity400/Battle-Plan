//! CEquipmentUpgradesInstance, pinned a729; embedded at CEquipmentVariant+68.

/// CEquipmentUpgradesInstance primary vtable; executable RTTI confirms old AIR_VARIANT_UPGRADE_CONTAINER identity.
pub const VTABLE_RVA: usize = 0x027a_b628;
/// CEquipmentUpgradesInstance entry vector data; read_compact_air_variant_upgrade_entries reads variant+70, instance+8.
pub const ENTRIES: usize = 0x08;
/// CEquipmentUpgradesInstance entry-vector u32 capacity; legacy variant+78 normalizes to instance+10.
pub const ENTRIES_CAPACITY: usize = 0x10;
/// CEquipmentUpgradesInstance entry-vector u32 count; legacy variant+7C normalizes to instance+14.
pub const ENTRIES_COUNT: usize = 0x14;
