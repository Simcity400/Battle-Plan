//! CEquipmentType definitions, effective archetypes and null placeholders share the complete
//! layout. The `ARCHETYPE` link reaches another type, including clones. All offsets use the
//! complete primary base.

/// CEquipmentType primary vtable.
pub const VTABLE_RVA: usize = 0x0292_18a0;
/// Null equipment-type vtable. The placeholder keeps the complete layout
/// and self-links its archetype pointer; ordinary types initially point to it.
pub const NULL_VTABLE_RVA: usize = 0x0292_19f0;
/// Equipment-type constructor initializes the primary vtable and self-links.
pub const CONSTRUCTOR_RVA: usize = 0x00c8_2b50;
/// CEquipmentType token u32; variant_definition_body authenticates it against `cequipment_variant::EQUIPMENT_TOKEN`.
pub const TOKEN: usize = 0x08;
/// Build-cost IC stat, signed fixed5; weights required equipment entries.
pub const BUILD_COST_IC: usize = 0x3b0;
/// CEquipmentType real/active flag u8; occupation canonical_equipment_archetype tests it before using its id.
pub const ACTIVE: usize = 0x10;
/// CEquipmentType name MSVC string.
pub const NAME: usize = 0x18;
/// CEquipmentType hull sprite MSVC string.
pub const HULL_SPRITE: usize = 0x118;
/// CEquipmentType copied-slot array pointer.
pub const SLOTS: usize = 0x70;
/// CEquipmentType copied-slot array capacity i32.
pub const SLOTS_CAPACITY: usize = 0x78;
/// CEquipmentType copied-slot array count i32.
pub const SLOTS_COUNT: usize = 0x7c;
/// Copied equipment-slot lookup over the stored slot array.
pub const SLOT_LOOKUP_RVA: usize = 0x00c8_6220;
/// Effective model priority, signed i32. Parent-chain additions are
/// signed and the result has a lower bound of one.
pub const MODEL_PRIORITY: usize = 0x3c8;
/// CEquipmentType visual_tech_level_addition i32.
pub const VISUAL_TECH_LEVEL_ADDITION: usize = 0x3cc;
/// CEquipmentType visual_tech_level_addition default.
pub const DEFAULT_VISUAL_TECH_LEVEL_ADDITION: i32 = 1;
/// CEquipmentType effective model-priority minimum.
pub const MIN_MODEL_PRIORITY: i32 = 1;
/// Reset existing type fields, including the visual-tech addition, without allocating.
pub const RESET_DEFAULTS_RVA: usize = 0x00c8_4830;
/// Finalize inherited values by adding signed visual additions along parent links, then clamping to one.
pub const FINALIZE_INHERITED_VALUES_RVA: usize = 0x00c8_5280;
/// CEquipmentType special-country flag u8; occupation stockpile reconstruction reads it, broader semantics unproved.
pub const SPECIAL_COUNTRY: usize = 0x3eb;
/// CEquipmentType special-pool classifier flags u64; occupation special-stockpile walk reads it on the archetype.
pub const POOL_FLAGS: usize = 0x408;
/// CEquipmentType stable archetype id u32; occupation canonical_equipment_archetype preserves it.
pub const ARCHETYPE_ID: usize = 0x418;
/// Equipment archetype pointer. Ordinary definitions initially use the
/// null placeholder; null-placeholder construction self-links it.
pub const ARCHETYPE: usize = 0x4d8;
/// CEquipmentType parent pointer.
pub const PARENT: usize = 0x4e0;
/// Equipment-type self-pointer; subsequent link semantics remain unknown.
pub const SELF_LINK_4E8: usize = 0x4e8;
/// Signed archetype index into the equipment database archetype array.
/// Non-archetypes retain -1. Every logistics series indexes by this value.
pub const STOCKPILE_INDEX: usize = 0x538;
/// Is-archetype byte, set when the archetype link resolves to this type.
/// Only such types enter the archetype list used by logistics.
pub const IS_ARCHETYPE: usize = 0x555;
/// CEquipmentType primary category flags u64.
pub const PRIMARY_CATEGORY_FLAGS: usize = 0x540;
/// CEquipmentType secondary category flags u64.
pub const SECONDARY_CATEGORY_FLAGS: usize = 0x548;
/// Optional maximum military factories: i32 followed by a has-value byte. Read on the type first,
/// then its archetype.
pub const MAX_MIL_FACTORIES: usize = 0x5a0;
/// Optional maximum naval factories, laid out as [`MAX_MIL_FACTORIES`].
/// Read on the type first, then its archetype.
pub const MAX_NAV_FACTORIES: usize = 0x5a8;
/// Equipment-type plane-family predicate.
pub const PLANE_CLASSIFIER_RVA: usize = 0x00c8_7ce0;
/// CEquipmentType plane-family mask.
pub const PLANE_MASK: u64 = 0x0000_001f_0037_fc00;
/// Equipment-type tank-family predicate.
pub const TANK_CLASSIFIER_RVA: usize = 0x00c8_7eb0;
/// CEquipmentType tank-family mask.
pub const TANK_MASK: u64 = 0x0000_0040_8000_003c;
/// Equipment-type ship-family predicate.
pub const SHIP_CLASSIFIER_RVA: usize = 0x00c8_8440;
/// CEquipmentType ship-family/category-normalization mask.
pub const SHIP_MASK: u64 = 0x0000_0080_0040_03c1;
/// Equipment-type generic-family predicate; examines only the current definition.
pub const GENERIC_CLASSIFIER_RVA: usize = 0x00c8_84b0;
/// CEquipmentType generic-family mask.
pub const GENERIC_MASK: u64 = 0x0000_0000_0008_0000;
/// Equipment category normalization helper.
pub const NORMALIZE_CATEGORY_RVA: usize = 0x00f7_5ea0;
/// Equipment category normalization first selector bit; occupation effective_equipment_mask tests 0x40 before other cases.
pub const NORMALIZE_FIRST_SELECTOR: u64 = 0x40;
/// Equipment category bits removed for selector 0x40; occupation effective_equipment_mask clears 0x181.
pub const NORMALIZE_FIRST_CLEAR: u64 = 0x181;
/// Equipment category low-byte sign bit; occupation effective_equipment_mask tests (mask as i8)<0 after selector 0x40.
pub const NORMALIZE_SECOND_SELECTOR: u64 = 0x80;
/// Equipment category bits removed for low-byte sign selector; occupation effective_equipment_mask clears 0x101.
pub const NORMALIZE_SECOND_CLEAR: u64 = 0x101;
/// Equipment category normalization third selector bit; occupation effective_equipment_mask tests 0x100 last.
pub const NORMALIZE_THIRD_SELECTOR: u64 = 0x100;
/// Equipment category bit cleared by selector 0x100 and used for special stockpile routing.
pub const SPECIAL_POOL_MASK: u64 = 1;
/// Equipment category bit selecting alternate country-runtime subtraction; occupation stockpile_amount tests bit 31.
pub const RUNTIME_SUBTRACTION_MASK: u64 = 1 << 31;
