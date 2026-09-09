//! NIndustrialOrganisation::CEquipmentTypeHeaderItem; retained FUN141fa4d70 identifies the RTTI class.
//! Knowledge only: this presentation path is excluded from hourly recording.
//! Sources are preserved in archive/pre-rewrite-2026-09-09 under
//! extractor/native/lanes/dynamic_collection_trace_live/src/.
//! Source: a729_mio_bonus_hook.rs.

/// MIO equipment-type window row primary vtable; archived hook:79 gates this exact a729 identity.
pub const VTABLE_RVA: usize = 0x02a8_e0e0;
/// MIO equipment-type window row GUI-subobject vtable; archived hook:80 gates this exact a729 identity.
pub const GUI_VTABLE_RVA: usize = 0x02a8_e150;
/// MIO equipment-type window row GUI subobject at +0x38; archived hook's row materialization checks its vtable and geometry.
pub const GUI: usize = 0x38;
/// MIO equipment-type window row GUI vtable +0x78 position method returning two i32 coordinates; archived hook invokes this geometry slot.
pub const POSITION_VTABLE_SLOT: usize = 0x78;
/// MIO equipment-type window row GUI vtable +0x28 extent method returning two i32 dimensions; archived hook invokes this geometry slot.
pub const EXTENT_VTABLE_SLOT: usize = 0x28;
