//! CReferencedDivisionTemplate identity and retained model preferences.

/// CReferencedDivisionTemplate vtable; old offsets.rs:639, pinned RTTI COL at RVA2CB8218 confirms the actual class name.
pub const VTABLE_RVA: usize = 0x0293_8f70;
/// CReferencedDivisionTemplate reference type; old offsets.rs:640 and template reader.
pub const REFERENCE_TYPE: u32 = 52;
/// CReferencedDivisionTemplate u32 type word; old offsets.rs:641 and template reader.
pub const TYPE: usize = 0x08;
/// CReferencedDivisionTemplate u32 id; old offsets.rs:642 and template reader.
pub const ID: usize = 0x0c;
/// CReferencedDivisionTemplate contained composition storage; old UZ func_140c6f080.decompile.c returns template+18, no RTTI base there.
pub const COMPOSITION: usize = 0x18;
/// CReferencedDivisionTemplate name MSVC string; old recorder_host/native_design_catalog.rs:492 receives registry primary pointer.
pub const NAME: usize = 0x20;
/// CReferencedDivisionTemplate grid dimensions, columns low16/rows high16; old native_design_catalog.rs:493-495.
pub const GRID_DIMENSIONS: usize = 0x60;
/// CReferencedDivisionTemplate regiment CSubUnitDefinition pointer-vector data; old native_design_catalog.rs:500.
pub const REGIMENT_DATA: usize = 0x68;
/// CReferencedDivisionTemplate regiment u32 count; old native_design_catalog.rs:500, row stride8.
pub const REGIMENT_COUNT: usize = 0x74;
/// CReferencedDivisionTemplate support CSubUnitDefinition pointer-vector data; old native_design_catalog.rs:503.
pub const SUPPORT_DATA: usize = 0x88;
/// CReferencedDivisionTemplate support u32 count; old native_design_catalog.rs:501-503, row stride8.
pub const SUPPORT_COUNT: usize = 0x94;
/// CReferencedDivisionTemplate regimental-support definition pointer-vector data; old native_design_catalog.rs:505, primary pointer.
pub const REGIMENTAL_SUPPORT_DATA: usize = 0xa8;
/// CReferencedDivisionTemplate regimental-support u32 count; old native_design_catalog.rs:505, row stride8 not contained composition pairs.
pub const REGIMENTAL_SUPPORT_COUNT: usize = 0xb4;
/// CReferencedDivisionTemplate excluded CEquipmentVariant pointer-vector data; division_presentation_inputs.rs:105-132.
pub const EXCLUDED_VARIANT_DATA: usize = 0x140;
/// CReferencedDivisionTemplate exclusion u32 capacity; division_presentation_inputs.rs:107.
pub const EXCLUDED_VARIANT_CAPACITY: usize = 0x148;
/// CReferencedDivisionTemplate exclusion u32 count; division_presentation_inputs.rs:108.
pub const EXCLUDED_VARIANT_COUNT: usize = 0x14c;
/// CReferencedDivisionTemplate u32 priority; old native_design_catalog.rs:507.
pub const PRIORITY: usize = 0x1a4;
/// CReferencedDivisionTemplate lock-regime snapshot byte; old native_design_catalog.rs:507, no boolean collapse.
pub const LOCK_REGIME: usize = 0x1cc;
/// CReferencedDivisionTemplate token289A country operand i32; old native_design_catalog/native_inputs.rs:37 and template_country_inputs.
pub const COUNTRY_OPERAND_289A: usize = 0x1ec;
/// CReferencedDivisionTemplate token3ABE country operand i32; old native_design_catalog/native_inputs.rs:38 and template_country_inputs.
pub const COUNTRY_OPERAND_3ABE: usize = 0x1f4;
/// CReferencedDivisionTemplate raw model-category u64; division_presentation_inputs.rs:151, meaningful only when presence byte is set.
pub const MODEL_CATEGORY: usize = 0x200;
/// CReferencedDivisionTemplate category-presence byte; division_presentation_inputs.rs:147.
pub const MODEL_CATEGORY_PRESENT: usize = 0x208;
/// CReferencedDivisionTemplate MSVC entity-name string; division_presentation_inputs.rs:144, engine-owned bytes preserved.
pub const MODEL_ENTITY: usize = 0x210;
/// CReferencedDivisionTemplate lock byte; old native_design_catalog.rs:507 preserves its nonzero test.
pub const LOCKED: usize = 0x235;
/// CReferencedDivisionTemplate obsolete flag byte, bit0; old native_design_catalog.rs:507.
pub const OBSOLETE_FLAGS: usize = 0x236;
