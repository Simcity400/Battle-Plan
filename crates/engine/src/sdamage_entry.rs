//! SDamageEntry, pinned a729; damage remains native i64.

/// SDamageEntry primary vtable; executable RTTI confirms old AIR_REGION_DAMAGE_ENTRY identity.
pub const VTABLE_RVA: usize = 0x02a0_9990;
/// SDamageEntry array stride; sample_compact_air_region_combat_native advances by 0x18 bytes.
pub const STRIDE: usize = 0x18;
/// SDamageEntry u32 cause type; sample_compact_air_region_combat_native reads entry+8.
pub const CAUSE_TYPE: usize = 0x08;
/// SDamageEntry i64 raw amount; sample_compact_air_region_combat_native reads entry+10 without narrowing.
pub const AMOUNT: usize = 0x10;
