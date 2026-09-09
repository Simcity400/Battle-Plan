//! NIndustrialOrganisation::CHistoryItem; retained FUN141fa5390 identifies the RTTI class.
//! Knowledge only: this presentation path is excluded from hourly recording.
//! Sources are preserved in archive/pre-rewrite-2026-09-09 under
//! extractor/native/lanes/dynamic_collection_trace_live/src/.
//! Source: a729_mio_history_hook.rs.

/// MIO history row primary vtable; archived hook:77 gates this exact a729 identity.
pub const VTABLE_RVA: usize = 0x02a8_e230;
/// MIO history row GUI-subobject vtable; archived hook:78 gates this exact a729 identity.
pub const GUI_VTABLE_RVA: usize = 0x02a8_e2a0;
/// MIO history row GUI subobject at +0x38; archived hook's row materialization checks its vtable and geometry.
pub const GUI: usize = 0x38;
/// MIO history row GUI vtable +0x78 position method returning two i32 coordinates; archived hook invokes this geometry slot.
pub const POSITION_VTABLE_SLOT: usize = 0x78;
/// MIO history row GUI vtable +0x28 extent method returning two i32 dimensions; archived hook invokes this geometry slot.
pub const EXTENT_VTABLE_SLOT: usize = 0x28;

/// MIO history row third-subobject vtable; archived hook:79/745 checks identity at row +0x68.
pub const THIRD_VTABLE_RVA: usize = 0x02a8_e368;
/// MIO history row third subobject; archived hook:745 verifies its vtable at +0x68.
pub const THIRD_INTERFACE: usize = 0x68;
/// MIO history row +0x70 organisation reference, pair of u32; archived hook:757 compares the owning organisation.
pub const ORGANISATION_REFERENCE: usize = 0x70;
/// MIO history row +0x78 family discriminator, native i32; archived hook:750 checks history type.
pub const HISTORY_TYPE: usize = 0x78;
/// CHistoryItem variant-family discriminator; a729_mio_history_hook.rs:420 compares this native i32.
pub const VARIANT_TYPE: i32 = 1;
/// CHistoryItem production-family discriminator; a729_mio_history_hook.rs:421 compares this native i32.
pub const PRODUCTION_TYPE: i32 = 2;
/// MIO history row +0x7c item reference, pair of u32; archived hook:766 resolves row identity.
pub const ITEM_REFERENCE: usize = 0x7c;
